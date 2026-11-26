//! Google Vertex AI with a service account key (JSON). One sign-in for every model the project
//! may use: Claude through Anthropic's Messages API (`publishers/anthropic/…:streamRawPredict`),
//! Gemini through Vertex' OpenAI-compatible endpoint (`endpoints/openapi/chat/completions`).
//!
//! Which models the project may use is nowhere listed: the catalog (`publishers.models.list`)
//! names what Vertex offers, not what is enabled. So each candidate is tried with a token count,
//! which costs nothing and answers 403/404 for a model the project has not enabled.
//!
//! Sign-in: a JWT signed with the key (RS256) is exchanged at the key's `token_uri` for an access
//! token that lives an hour; it is renewed five minutes before it ends. RSA signing uses `ring`,
//! which rustls already brings, so the service account needs no new dependency.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, ChatRequest, Error, ErrorKind, ModelCheck, ModelInfo, Result};
use aiscraft_core::{WebAccess, WebRead, WebSearch};
use base64::Engine as _;
use futures::StreamExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::anthropic::{ReqParts, stream_messages, vertex_body};
use crate::http::{client, json as read_json, net_err, read_capped, status_error};
use crate::{Backend, EventStream};

/// The longest key file accepted (real ones are about 2.4 KB).
pub const MAX_KEY_BYTES: usize = 64 * 1024;
pub(crate) const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const DEFAULT_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
/// An access token is renewed this long before it ends.
const RENEW_BEFORE: Duration = Duration::from_secs(5 * 60);
/// Models tried at most per check, so a large catalog cannot run up a long list of requests.
const MAX_CANDIDATES: usize = 40;
/// Token counts running at once while checking.
const PARALLEL_PROBES: usize = 6;

/// Models tried even when the catalog cannot be read (Google's and Anthropic's docs,
/// 2026-11-25). Claude models before 4.6 need the version after `@`.
const KNOWN_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-sonnet-4-6",
    "claude-opus-4-5@20251101",
    "claude-haiku-4-5@20251001",
    "google/gemini-3.8-flash",
    "google/gemini-3.5-flash",
    "google/gemini-3.1-pro-preview",
    "google/gemini-2.5-pro",
    "google/gemini-2.5-flash",
];

/// The fields of a service account key that signing in needs.
#[derive(Debug, Clone)]
pub struct ServiceAccount {
    pub client_email: String,
    pub project_id: String,
    pub private_key_id: Option<String>,
    private_key: String,
    token_uri: String,
}

/// Read a service account key (the JSON file from the Google Cloud console) and check that it
/// can sign. Errors name what is wrong in words a person can act on.
pub fn parse_service_account(text: &str) -> Result<ServiceAccount> {
    if text.len() > MAX_KEY_BYTES {
        return Err(Error::invalid("Die Schlüsseldatei ist zu groß; gesucht ist die JSON-Datei eines Dienstkontos (etwa 2 KB)"));
    }
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| Error::invalid("Das ist keine JSON-Schlüsseldatei. Gesucht ist der Schlüssel eines Dienstkontos aus der Google Cloud Console."))?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    if s("type").as_deref() != Some("service_account") {
        return Err(Error::invalid(
            "Die Datei ist kein Dienstkonto-Schlüssel (\"type\": \"service_account\" fehlt). In der Google Cloud Console: IAM › Dienstkonten › Schlüssel › Schlüssel hinzufügen › JSON.",
        ));
    }
    let missing = |k: &str| Error::invalid(format!("In der Schlüsseldatei fehlt \"{k}\""));
    let client_email = s("client_email").ok_or_else(|| missing("client_email"))?;
    let project_id = s("project_id").ok_or_else(|| missing("project_id"))?;
    if !project_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':')) || project_id.len() > 100 {
        return Err(Error::invalid(format!("Ungültige Projekt-ID in der Schlüsseldatei: {project_id}")));
    }
    let token_uri = s("token_uri").unwrap_or_else(|| DEFAULT_TOKEN_URI.to_string());
    check_token_uri(&token_uri)?;
    let sa = ServiceAccount {
        client_email,
        project_id,
        private_key_id: s("private_key_id"),
        private_key: s("private_key").ok_or_else(|| missing("private_key"))?,
        token_uri,
    };
    signer(&sa)?;
    Ok(sa)
}

/// The key as it is kept in the keychain: checked, compact, only the fields signing in needs.
pub fn normalize_key(text: &str) -> Result<String> {
    let sa = parse_service_account(text)?;
    Ok(json!({
        "type": "service_account",
        "project_id": sa.project_id,
        "client_email": sa.client_email,
        "private_key_id": sa.private_key_id,
        "private_key": sa.private_key,
        "token_uri": sa.token_uri,
    })
    .to_string())
}

/// The signed JWT goes only to Google, or to a server on this computer (tests).
fn check_token_uri(uri: &str) -> Result<()> {
    let url = reqwest::Url::parse(uri).map_err(|_| Error::invalid(format!("Ungültige token_uri in der Schlüsseldatei: {uri}")))?;
    let host = url.host_str().unwrap_or_default();
    let ok = match url.scheme() {
        "https" => host == "oauth2.googleapis.com" || host.ends_with(".googleapis.com") || aiscraft_core::is_loopback_host(host),
        "http" => aiscraft_core::is_loopback_host(host),
        _ => false,
    };
    if ok { Ok(()) } else { Err(Error::invalid(format!("Die token_uri der Schlüsseldatei zeigt nicht zu Google: {uri}"))) }
}

pub(crate) fn signer(sa: &ServiceAccount) -> Result<ring::rsa::KeyPair> {
    let bad = || Error::invalid("Der private Schlüssel in der Datei ist unlesbar (erwartet: RSA im PEM-Format, wie Google ihn ausgibt)");
    let pem = sa.private_key.replace("\\n", "\n");
    let pkcs1 = pem.contains("BEGIN RSA PRIVATE KEY");
    let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).flat_map(|l| l.chars()).filter(|c| !c.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|_| bad())?;
    let key = if pkcs1 { ring::rsa::KeyPair::from_der(&der) } else { ring::rsa::KeyPair::from_pkcs8(&der) };
    key.map_err(|_| bad())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The assertion for the token endpoint: RS256, `iss` the account, `aud` the token endpoint, one
/// hour at most.
pub fn assertion(sa: &ServiceAccount, now: u64) -> Result<String> {
    let mut header = json!({"alg": "RS256", "typ": "JWT"});
    if let (Some(kid), Some(o)) = (&sa.private_key_id, header.as_object_mut()) {
        o.insert("kid".into(), json!(kid));
    }
    let claims = json!({"iss": sa.client_email, "scope": SCOPE, "aud": sa.token_uri, "iat": now, "exp": now + 3600});
    let input = format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
    let key = signer(sa)?;
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig)
        .map_err(|_| Error::invalid("Mit diesem Schlüssel lässt sich nicht signieren"))?;
    Ok(format!("{input}.{}", b64url(&sig)))
}

/// Where the API is: a region gives Google's host for it; a full address (tests, private
/// endpoints) is used as it is, with region `global`.
fn api_root(base: &str) -> Result<(String, String)> {
    let base = base.trim().trim_end_matches('/');
    if base.starts_with("https://") || base.starts_with("http://") {
        return Ok((base.to_string(), "global".into()));
    }
    let region = if base.is_empty() { "global".to_string() } else { base.to_ascii_lowercase() };
    if region.len() > 40 || !region.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(Error::invalid(format!("Ungültige Region für Vertex AI: {base} (z. B. global, eu, europe-west3)")));
    }
    let root = match region.as_str() {
        "global" => "https://aiplatform.googleapis.com".to_string(),
        "us" | "eu" => format!("https://aiplatform.{region}.rep.googleapis.com"),
        r => format!("https://{r}-aiplatform.googleapis.com"),
    };
    Ok((root, region))
}

/// Model ids go into URL paths: only letters, digits and `-._@`, and `/` between publisher and
/// model for the OpenAI-compatible endpoint.
fn safe_model(id: &str) -> bool {
    !id.is_empty() && id.len() <= 120 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '@' | '/')) && !id.contains("..")
}

/// Claude goes through Anthropic's Messages API; everything else through the OpenAI-compatible endpoint.
fn is_claude(model: &str) -> bool {
    model.starts_with("claude-")
}

/// "claude-opus-5-5" → "Claude Opus 5.5", "google/gemini-2.5-flash" → "Gemini 2.5 Flash".
pub fn display_name(id: &str) -> String {
    let base = id.rsplit('/').next().unwrap_or(id);
    let base = base.split('@').next().unwrap_or(base);
    let mut words: Vec<String> = Vec::new();
    let mut digits: Vec<&str> = Vec::new();
    for part in base.split('-') {
        if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()) && part.len() <= 2 {
            digits.push(part);
            continue;
        }
        if !digits.is_empty() {
            words.push(digits.join("."));
            digits.clear();
        }
        let mut cs = part.chars();
        words.push(cs.next().map(|f| f.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default());
    }
    if !digits.is_empty() {
        words.push(digits.join("."));
    }
    words.join(" ")
}

/// Chat models in a catalog page: Claude and Gemini text models; embeddings, images, speech and
/// live models are left out.
fn catalog_ids(publisher: &str, v: &Value) -> Vec<String> {
    let skip = ["embedding", "image", "imagen", "tts", "audio", "live", "veo", "native", "computer-use"];
    v.get("publisherModels")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let name = m.get("name")?.as_str()?.rsplit('/').next()?.to_string();
                    let version = m.get("versionId").and_then(Value::as_str).unwrap_or_default();
                    match publisher {
                        "anthropic" if name.starts_with("claude-") && !name.contains('@') => {
                            // Older Claude models are addressed with their version date.
                            Some(if version.len() == 8 && version.chars().all(|c| c.is_ascii_digit()) { format!("{name}@{version}") } else { name })
                        }
                        "google" if name.starts_with("gemini-") => Some(format!("google/{name}")),
                        _ => None,
                    }
                })
                .filter(|id| safe_model(id) && !skip.iter().any(|s| id.contains(s)))
                .collect()
        })
        .unwrap_or_default()
}

struct Token {
    value: String,
    until: Instant,
}

pub struct Vertex {
    config: BackendConfig,
    root: String,
    region: String,
    account: std::result::Result<ServiceAccount, String>,
    http: reqwest::Client,
    token: Mutex<Option<Token>>,
    /// Models tried, from this run or remembered from the last; `None`: not tried yet.
    checks: Mutex<Option<Vec<ModelCheck>>>,
}

impl Vertex {
    pub fn new(config: BackendConfig, secret: Option<String>) -> Result<Self> {
        let (root, region) = api_root(&config.base_url)?;
        let account = match secret.filter(|s| !s.trim().is_empty()) {
            None => Err("Keine Schlüsseldatei hinterlegt. Unter Einstellungen › Dienste die JSON-Datei des Dienstkontos wählen.".to_string()),
            Some(s) => parse_service_account(&s).map_err(|e| e.message),
        };
        Ok(Self { config, root, region, account, http: client(true)?, token: Mutex::new(None), checks: Mutex::new(None) })
    }

    fn account(&self) -> Result<&ServiceAccount> {
        self.account.as_ref().map_err(|m| Error::auth(m.clone()))
    }

    fn project_path(&self) -> Result<String> {
        Ok(format!("{}/v1/projects/{}/locations/{}", self.root, self.account()?.project_id, self.region))
    }

    /// A valid access token: the kept one, or a fresh one from the token endpoint.
    async fn token(&self) -> Result<String> {
        if let Some(t) = self.token.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
            && t.until > Instant::now() + RENEW_BEFORE
        {
            return Ok(t.value.clone());
        }
        let sa = self.account()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let jwt = assertion(sa, now)?;
        let body = format!("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={jwt}");
        let resp = self
            .http
            .post(&sa.token_uri)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(net_err)?;
        let status = resp.status();
        let text = read_capped(resp, 64 * 1024).await?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let why = v.get("error_description").or_else(|| v.get("error")).and_then(Value::as_str).unwrap_or("").chars().take(300).collect::<String>();
            let msg = format!(
                "Google nimmt den Dienstkonto-Schlüssel nicht an ({}{}). Ist der Schlüssel gelöscht oder das Dienstkonto deaktiviert? Dann in der Google Cloud Console einen neuen JSON-Schlüssel anlegen.",
                status.as_u16(),
                if why.is_empty() { String::new() } else { format!(": {why}") }
            );
            return Err(if status.is_client_error() { Error::auth(msg) } else { Error::network(msg) });
        }
        let value = v
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty() && t.len() < 8192)
            .ok_or_else(|| Error::protocol("Google hat kein Zugriffstoken geschickt"))?
            .to_string();
        let secs = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600).clamp(60, 24 * 3600);
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = Some(Token { value: value.clone(), until: Instant::now() + Duration::from_secs(secs) });
        Ok(value)
    }

    /// Ids worth trying: the known ones, then what the catalog adds.
    async fn candidates(&self, token: &str) -> Vec<String> {
        let mut ids: Vec<String> = KNOWN_MODELS.iter().map(|s| s.to_string()).collect();
        let project = self.account().map(|a| a.project_id.clone()).unwrap_or_default();
        for publisher in ["anthropic", "google"] {
            let url = format!("{}/v1beta1/publishers/{publisher}/models?pageSize=100", self.root);
            let resp = self.http.get(&url).bearer_auth(token).header("x-goog-user-project", &project).send().await;
            // The catalog is a help, never needed: without it the known models are tried.
            let Ok(resp) = resp else { continue };
            if !resp.status().is_success() {
                log::info!("Vertex-Katalog {publisher}: HTTP {}", resp.status());
                continue;
            }
            if let Ok(v) = read_json(resp, "Modellkatalog").await {
                for id in catalog_ids(publisher, &v) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
        }
        ids.truncate(MAX_CANDIDATES);
        ids
    }

    /// Try one model with a token count: free, and 403/404 when the project may not use it.
    async fn probe(&self, token: &str, id: &str) -> ModelCheck {
        let check = |usable: bool, reason: Option<String>| ModelCheck { id: id.to_string(), name: display_name(id), usable, reason };
        let Ok(base) = self.project_path() else { return check(false, Some("Kein Schlüssel".into())) };
        let (url, body) = if is_claude(id) {
            (format!("{base}/publishers/anthropic/models/count-tokens:rawPredict"), json!({"model": id,"messages": [{"role": "user", "content": "Hallo"}]}))
        } else {
            let (publisher, model) = id.split_once('/').unwrap_or(("google", id));
            (format!("{base}/publishers/{publisher}/models/{model}:countTokens"), json!({"contents": [{"role": "user", "parts": [{"text": "Hallo"}]}]}))
        };
        match self.http.post(&url).bearer_auth(token).json(&body).send().await {
            Ok(r) if r.status().is_success() => check(true, None),
            Ok(r) => {
                let status = r.status().as_u16();
                let e = status_error(r, "Probe").await;
                let detail = e.message.trim_start_matches("Probe: ").chars().take(200).collect::<String>();
                let reason = match status {
                    401 => "Anmeldung abgelaufen".to_string(),
                    403 => {
                        "Im Projekt nicht freigeschaltet (Model Garden › Modell aktivieren) oder dem Dienstkonto fehlt die Rolle „Vertex AI User“".to_string()
                    }
                    404 => format!("In Projekt oder Region {} nicht verfügbar", self.region),
                    429 => "Anfragegrenze erreicht, später neu prüfen".to_string(),
                    _ => format!("Fehler {status}: {detail}"),
                };
                check(false, Some(reason))
            }
            Err(e) => check(false, Some(net_err(e).message)),
        }
    }

    async fn discover(&self) -> Result<Vec<ModelCheck>> {
        let token = self.token().await?;
        let ids = self.candidates(&token).await;
        let probes: Vec<BoxFuture<'_, (usize, ModelCheck)>> = ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| {
                let token = token.clone();
                Box::pin(async move { (i, self.probe(&token, &id).await) }) as BoxFuture<'_, (usize, ModelCheck)>
            })
            .collect();
        let mut found: Vec<(usize, ModelCheck)> = futures::stream::iter(probes).buffer_unordered(PARALLEL_PROBES).collect().await;
        found.sort_by_key(|(i, _)| *i);
        let checks: Vec<ModelCheck> = found.into_iter().map(|(_, c)| c).collect();
        if checks.iter().any(|c| c.reason.as_deref() == Some("Anmeldung abgelaufen")) && !checks.iter().any(|c| c.usable) {
            return Err(Error::auth("Google hat die Anmeldung abgelehnt. Bitte die Schlüsseldatei neu wählen."));
        }
        *self.checks.lock().unwrap_or_else(PoisonError::into_inner) = Some(checks.clone());
        Ok(checks)
    }
}

fn model_info(c: &ModelCheck) -> ModelInfo {
    ModelInfo { id: c.id.clone(), name: c.name.clone(), description: None, vision: true, context_window: None }
}

impl Backend for Vertex {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    /// Per backend, not per model: web search is Claude's (on Vertex only `web_search_20250305`,
    /// no web fetch, no Files API); Gemini answers without it.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            remote_history: false,
            uploads: false,
            system_prompt: true,
            vision: true,
            tools: true,
            tools_via_mcp: false,
            web: WebAccess::new(WebSearch::ServerTool, WebRead::ServerTool),
        }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            self.token().await?;
            let sa = self.account()?;
            Ok(AccountInfo {
                display_name: Some(sa.client_email.clone()),
                detail: Some(format!("Projekt {} · Region {}", sa.project_id, self.region)),
                expires: None,
                server_tools: vec![],
            })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let known = self.checks.lock().unwrap_or_else(PoisonError::into_inner).clone();
            let checks = match known {
                Some(c) => c,
                None => self.discover().await?,
            };
            Ok(checks.iter().filter(|c| c.usable).map(model_info).collect())
        })
    }

    fn model_checks(&self) -> Vec<ModelCheck> {
        self.checks.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_default()
    }

    fn seed_model_checks(&self, checks: Vec<ModelCheck>) {
        let checks: Vec<ModelCheck> = checks.into_iter().filter(|c| safe_model(&c.id)).take(MAX_CANDIDATES).collect();
        *self.checks.lock().unwrap_or_else(PoisonError::into_inner) = (!checks.is_empty()).then_some(checks);
    }

    fn discover_models(&self) -> BoxFuture<'_, Result<Vec<ModelCheck>>> {
        Box::pin(self.discover())
    }

    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>> {
        Box::pin(async move {
            if !safe_model(&req.model) {
                return Err(Error::invalid(format!("Unbekanntes Modell: {}", req.model.chars().take(120).collect::<String>())));
            }
            let token = self.token().await?;
            let base = self.project_path()?;
            if is_claude(&req.model) {
                let parts = ReqParts {
                    url: format!("{base}/publishers/anthropic/models/{}:streamRawPredict", req.model),
                    headers: vec![("authorization", format!("Bearer {token}"))],
                };
                return stream_messages(self.http.clone(), parts, vertex_body(&req)).await.map_err(|e| not_enabled(e, &req.model));
            }
            let openai = crate::openai::OpenAi::with_client(self.config.clone(), format!("{base}/endpoints/openapi"), Some(token), self.http.clone());
            openai.chat(req.clone()).await.map_err(|e| not_enabled(e, &req.model))
        })
    }
}

/// A 403/404 while chatting most often means the model is not enabled in the project.
fn not_enabled(e: Error, model: &str) -> Error {
    match e.kind {
        ErrorKind::NotFound => {
            Error::not_found(format!("{model} ist im Google-Projekt nicht verfügbar ({}). Unter Einstellungen › Dienste „Modelle prüfen“.", e.message))
        }
        ErrorKind::Auth if !e.message.contains("Dienstkonto") => Error::auth(format!(
            "{model}: kein Zugriff ({}). Ist das Modell im Model Garden freigeschaltet und hat das Dienstkonto die Rolle „Vertex AI User“?",
            e.message
        )),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vertex_fake::{fake_google, fake_key, key_json, test_pem};
    use aiscraft_core::{Message, Role, StreamEvent};

    #[test]

    fn keys_are_read_and_bad_ones_named() {
        let sa = parse_service_account(&key_json("https://oauth2.googleapis.com/token", false)).unwrap();
        assert_eq!(sa.project_id, "schule-test");
        assert!(parse_service_account(&key_json("https://oauth2.googleapis.com/token", true)).is_ok(), "PKCS#1 keys work too");
        assert!(parse_service_account("{kaputt").unwrap_err().message.contains("keine JSON"));
        assert!(parse_service_account(r#"{"type":"authorized_user"}"#).unwrap_err().message.contains("kein Dienstkonto"));
        let evil = key_json("https://evil.example/token", false);
        assert!(parse_service_account(&evil).unwrap_err().message.contains("nicht zu Google"));
        let mut broken: Value = serde_json::from_str(&key_json(DEFAULT_TOKEN_URI, false)).unwrap();
        broken["private_key"] = json!("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----");
        assert!(parse_service_account(&broken.to_string()).unwrap_err().message.contains("unlesbar"));
        assert!(parse_service_account(&"x".repeat(MAX_KEY_BYTES + 1)).is_err());
        // Stored compact, with what signing in needs.
        let n: Value = serde_json::from_str(&normalize_key(&key_json(DEFAULT_TOKEN_URI, false)).unwrap()).unwrap();
        assert_eq!(n["client_email"], "chatcraft@schule-test.iam.gserviceaccount.com");
    }

    #[test]
    fn regions_names_and_catalog() {
        assert_eq!(api_root("global").unwrap().0, "https://aiplatform.googleapis.com");
        assert_eq!(api_root("eu").unwrap().0, "https://aiplatform.eu.rep.googleapis.com");
        assert_eq!(api_root("europe-west3").unwrap(), ("https://europe-west3-aiplatform.googleapis.com".into(), "europe-west3".into()));
        assert!(api_root("evil.com/x").is_err());
        assert_eq!(display_name("claude-opus-5-5"), "Claude Opus 5.5");
        assert_eq!(display_name("claude-haiku-4-5@20251001"), "Claude Haiku 4.5");
        assert_eq!(display_name("google/gemini-2.5-flash"), "Gemini 2.5 Flash");
        assert!(!safe_model("claude/../../x") && !safe_model("a b") && safe_model("claude-opus-4-5@20251101"));
        let page = json!({"publisherModels": [
            {"name": "publishers/anthropic/models/claude-mythos-5"},
            {"name": "publishers/anthropic/models/claude-3-haiku", "versionId": "20240307"},
            {"name": "publishers/anthropic/models/../x"},
        ]});
        assert_eq!(catalog_ids("anthropic", &page), ["claude-mythos-5", "claude-3-haiku@20240307"]);
        let page = json!({"publisherModels": [{"name": "publishers/google/models/gemini-9-pro"}, {"name": "publishers/google/models/gemini-embedding-001"}, {"name": "publishers/google/models/imagen-4"}]});
        assert_eq!(catalog_ids("google", &page), ["google/gemini-9-pro"]);
    }

    #[test]
    fn claude_body_for_vertex() {
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: "claude-opus-5-5".into(),
            system: None,
            messages: vec![],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: true,
            effort: None,
        };
        let b = vertex_body(&req);
        assert!(b.get("model").is_none() && b.get("fallbacks").is_none());
        assert_eq!(b["anthropic_version"], "2026-10-21");
        let tools = b["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1, "no web fetch on Vertex: {tools:?}");
        assert_eq!(tools[0]["type"], "web_search_20250305");
        // Per model: Claude searches the web and takes documents, Gemini does neither.
        let v = aiscraft_core::BackendKind::Vertex;
        let caps = Vertex::new(config("global"), None).unwrap().capabilities();
        assert!(caps.for_model(v, "claude-opus-5-5").web.own_search() && !caps.for_model(v, "google/gemini-2.5-flash").web.own_search());
        assert!(aiscraft_core::speaks_anthropic(v, "claude-opus-5-5") && !aiscraft_core::speaks_anthropic(v, "google/gemini-2.5-flash"));
    }

    fn config(addr: &str) -> BackendConfig {
        BackendConfig {
            id: "v".into(),
            name: "Vertex".into(),
            kind: aiscraft_core::BackendKind::Vertex,
            base_url: addr.into(),
            default_model: None,
            send_context: true,
        }
    }

    fn ask(model: &str) -> ChatRequest {
        let user = Message {
            id: "u".into(),
            conversation_id: "c".into(),
            role: Role::User,
            content: "Hallo".into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
            thinking: String::new(),
            citations: Vec::new(),
        };
        ChatRequest {
            conversation_id: "c".into(),
            model: model.into(),
            system: None,
            messages: vec![user],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
            effort: None,
        }
    }

    async fn text_of(s: EventStream) -> String {
        let events: Vec<StreamEvent> = s.map(|e| e.unwrap()).collect().await;
        events.iter().filter_map(|e| if let StreamEvent::Delta { text } = e { Some(text.as_str()) } else { None }).collect()
    }

    /// One sign-in, then the models are found by trying them, and both kinds answer.
    #[tokio::test]
    async fn sign_in_find_models_and_chat() {
        let (addr, seen) = fake_google().await;
        let v = Vertex::new(config(&addr), Some(fake_key(&addr))).unwrap();
        let account = v.check().await.unwrap();
        assert_eq!(account.display_name.as_deref(), Some("chatcraft@schule-test.iam.gserviceaccount.com"));
        assert!(account.detail.unwrap().contains("schule-test"));
        let models: Vec<String> = v.models().await.unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(models, ["claude-opus-5-5", "google/gemini-2.5-flash"]);
        let checks = v.model_checks();
        let sonnet = checks.iter().find(|c| c.id == "claude-sonnet-5-5").unwrap();
        assert!(!sonnet.usable && sonnet.reason.as_deref().unwrap().contains("nicht freigeschaltet"), "{sonnet:?}");
        assert!(checks.iter().any(|c| c.id == "claude-mythos-5"), "the catalog adds models: {checks:?}");
        // Known checks answer without asking again.
        let before = seen.lock().unwrap().len();
        assert_eq!(v.models().await.unwrap().len(), 2);
        assert_eq!(seen.lock().unwrap().len(), before);

        assert_eq!(text_of(v.chat(ask("claude-opus-5-5")).await.unwrap()).await, "Hallo von Claude");
        assert_eq!(text_of(v.chat(ask("google/gemini-2.5-flash")).await.unwrap()).await, "Hallo von Gemini");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.iter().filter(|r| r.starts_with("POST /token")).count(), 1, "one token for every call");
        let claude = seen.iter().find(|r| r.contains(":streamRawPredict")).unwrap();
        let body: Value = serde_json::from_str(claude.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body.get("model").is_none());
        assert_eq!(body["anthropic_version"], "2026-10-21");
        let gemini = seen.iter().find(|r| r.contains("/chat/completions")).unwrap();
        assert!(gemini.contains("\"model\":\"google/gemini-2.5-flash\""));
    }

    #[tokio::test]
    async fn a_rejected_key_or_model_is_named() {
        let (addr, _) = fake_google().await;
        // The same key in PKCS#1 form signs the same way: accepted.
        let mut pkcs1: Value = serde_json::from_str(&fake_key(&addr)).unwrap();
        pkcs1["private_key"] = json!(test_pem(true));
        assert!(Vertex::new(config(&addr), Some(pkcs1.to_string())).unwrap().check().await.is_ok());
        // An account Google does not know (deleted key, disabled account): named, as a sign-in problem.
        let mut gone: Value = serde_json::from_str(&fake_key(&addr)).unwrap();
        gone["client_email"] = json!("geloescht@schule-test.iam.gserviceaccount.com");
        let e = Vertex::new(config(&addr), Some(gone.to_string())).unwrap().check().await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Auth);
        assert!(e.message.contains("Invalid JWT") && e.message.contains("neuen JSON-Schlüssel"), "{e:?}");
        let none = Vertex::new(config(&addr), None).unwrap();
        let e = none.check().await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Auth);
        assert!(e.message.contains("Schlüsseldatei"));
        let v = Vertex::new(config(&addr), Some(fake_key(&addr))).unwrap();
        let e = v.chat(ask("claude-sonnet-5-5")).await.err().unwrap();
        assert!(e.message.contains("claude-sonnet-5-5"), "{e:?}");
        assert!(v.chat(ask("../evil")).await.is_err());
    }
}
