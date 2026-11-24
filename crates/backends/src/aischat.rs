//! AIS.chat through the person's signed-in web session.
//!
//! AIS.chat (FWU-DE/ais-chat) is a Next.js app with Auth.js sessions in an encrypted cookie. A
//! native client that sends that cookie is treated like the browser. Plain JSON routes cover the
//! conversation list, uploads and the session; sending a message is the server action
//! `sendChatMessageAction`, whose id changes with each deploy and is read from the shipped JS
//! chunks. The answer is a React Flight response carrying a `ReadableStream<string>` of text
//! deltas and `\x1e`-prefixed activity events.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, PoisonError};

use aiscraft_core::{
    AccountInfo, ActivityLink, ActivityStep, BackendConfig, Capabilities, ChatRequest, ConversationKind, Error, GeneratedDocument, ImageRequest, ImageResult,
    ModelInfo, Persona, PersonaInfo, PersonaKind, RemoteConversation, Result, Role, StreamEvent,
};
use aiscraft_rsc::flight::{ActionDecoder, ActionEvent};
use aiscraft_rsc::{encode_action_args, scrape};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use reqwest::header::{HeaderMap, LOCATION, SET_COOKIE};
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

use crate::http::{PAGE_CAP, client, json as read_json, net_err, read_capped, status_error};
use crate::{Backend, EventStream, SecretSink};

pub const SESSION_COOKIE: &str = "__Secure-ais-chat-app.authjs.session-token";
const SEND_ACTION: &str = "sendChatMessageAction";
const MAX_CHUNKS: usize = 80;

/// Accept what a person pastes (a bare token, `name=value`, or a whole `Cookie:` header) and keep
/// only the session cookie, including Auth.js's chunked `.0`, `.1` parts.
pub fn normalize_session_cookie(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.strip_prefix("Cookie:").or_else(|| s.strip_prefix("cookie:")).unwrap_or(s).trim();
    if s.is_empty() {
        return None;
    }
    if !s.contains('=') {
        return (!s.contains(char::is_whitespace)).then(|| format!("{SESSION_COOKIE}={s}"));
    }
    let jar = parse_cookie_pairs(s);
    let kept: BTreeMap<_, _> = jar.into_iter().filter(|(k, v)| k.starts_with(SESSION_COOKIE) && !v.is_empty()).collect();
    (!kept.is_empty()).then(|| header_of(&kept))
}

fn parse_cookie_pairs(s: &str) -> BTreeMap<String, String> {
    s.split(';').filter_map(|p| p.split_once('=')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect()
}

fn header_of(jar: &BTreeMap<String, String>) -> String {
    jar.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
}

/// Apply `Set-Cookie` headers to the session jar. Returns whether the jar changed.
fn absorb_set_cookie(jar: &mut BTreeMap<String, String>, headers: &HeaderMap) -> bool {
    let mut changed = false;
    for h in headers.get_all(SET_COOKIE) {
        let Ok(h) = h.to_str() else { continue };
        let mut parts = h.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else { continue };
        let (name, value) = (name.trim(), value.trim());
        if !name.starts_with(SESSION_COOKIE) {
            continue;
        }
        let expired = parts.any(|a| {
            let a = a.trim().to_ascii_lowercase();
            a == "max-age=0" || a.starts_with("expires=thu, 01 jan 1970")
        });
        if expired || value.is_empty() {
            changed |= jar.remove(name).is_some();
        } else if jar.get(name).map(String::as_str) != Some(value) {
            jar.insert(name.to_string(), value.to_string());
            changed = true;
        }
    }
    changed
}

pub struct AisChat {
    config: BackendConfig,
    base: String,
    http: reqwest::Client,
    jar: Mutex<BTreeMap<String, String>>,
    on_secret: SecretSink,
    /// Server action ids by name, as far as found in the shipped JS so far.
    actions: tokio::sync::Mutex<BTreeMap<String, String>>,
    /// Editor pages of the person's own assistants, characters and scenarios: the only pages
    /// whose JS carries `ingestWebContentAction`.
    editor_pages: Mutex<Vec<String>>,
    /// JS chunks already searched for action ids: the overview pages share most of theirs, so
    /// each is fetched once, not once per page.
    scanned: Mutex<HashSet<String>>,
}

/// Pages whose JS carries an action, in the order to look. Every page also loads the shared
/// chunks, so the start page comes first for actions used everywhere.
fn action_pages(name: &str) -> &'static [&'static str] {
    match name {
        "getAssistantsByFilterAction" => &["/assistants"],
        "getCharactersByFilterAction" => &["/characters"],
        "getLearningScenariosByFilterAction" => &["/learning-scenarios"],
        "generateImageAction" => &["/image-generation"],
        // Ships with the overview pages (the favourite button on cards).
        FAVORITE_ACTION => &["/assistants", "/characters", "/learning-scenarios"],
        _ => &["/"],
    }
}

/// A person-readable name for a missing action, for the error message.
fn action_label(name: &str) -> &'static str {
    match name {
        SEND_ACTION => "Die Chat-Funktion",
        "getAssistantsByFilterAction" => "Die Liste der Assistenten",
        "getCharactersByFilterAction" => "Die Liste der Dialogpartner",
        "getLearningScenariosByFilterAction" => "Die Liste der Lernszenarien",
        "generateImageAction" => "Die Bilderzeugung",
        FAVORITE_ACTION => "Das Markieren von Favoriten",
        INGEST_ACTION => "Das Einlesen von Webseiten",
        _ => "Diese Funktion",
    }
}

const INGEST_ACTION: &str = "ingestWebContentAction";
const FAVORITE_ACTION: &str = "setFavoriteAction";
const IMAGE_CAP: usize = 25 * 1024 * 1024;

impl AisChat {
    pub fn new(config: BackendConfig, base: String, secret: Option<String>, on_secret: SecretSink) -> Result<Self> {
        let jar = secret.as_deref().and_then(normalize_session_cookie).map(|c| parse_cookie_pairs(&c)).unwrap_or_default();
        Ok(Self {
            config,
            base,
            http: client(false)?,
            jar: Mutex::new(jar),
            on_secret,
            actions: tokio::sync::Mutex::new(BTreeMap::new()),
            editor_pages: Mutex::new(Vec::new()),
            scanned: Mutex::new(HashSet::new()),
        })
    }

    fn cookie(&self) -> Result<String> {
        let jar = self.jar.lock().unwrap_or_else(PoisonError::into_inner);
        if jar.is_empty() {
            return Err(Error::auth("Nicht bei AIS.chat angemeldet: Sitzungs-Cookie fehlt"));
        }
        Ok(header_of(&jar))
    }

    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        Ok(self.http.request(method, format!("{}{path}", self.base)).header("Cookie", self.cookie()?))
    }

    /// Send, keep a renewed session cookie, and turn a redirect to the login page into `Auth`.
    async fn send(&self, rb: reqwest::RequestBuilder) -> Result<Response> {
        let resp = rb.send().await.map_err(net_err)?;
        let changed = {
            let mut jar = self.jar.lock().unwrap_or_else(PoisonError::into_inner);
            absorb_set_cookie(&mut jar, resp.headers()).then(|| header_of(&jar))
        };
        if let Some(secret) = changed {
            (self.on_secret)(&self.config.id, secret);
        }
        if resp.status().is_redirection() {
            let loc = resp.headers().get(LOCATION).and_then(|l| l.to_str().ok()).unwrap_or("").to_string();
            if loc.contains("/login") || loc.contains("/api/auth") {
                return Err(Error::auth("AIS.chat-Sitzung abgelaufen. Bitte neu anmelden."));
            }
            return Err(Error::protocol(format!("AIS.chat leitet weiter nach {loc}. Bitte einmal im Browser öffnen (z. B. Nutzungsbedingungen bestätigen).")));
        }
        Ok(resp)
    }

    async fn page_at(&self, path: &str) -> Result<String> {
        let resp = self.send(self.request(Method::GET, path)?.header("Accept", "text/html")).await?;
        if !resp.status().is_success() {
            return Err(status_error(resp, "AIS.chat-Seite").await);
        }
        read_capped(resp, PAGE_CAP).await
    }

    async fn page(&self) -> Result<String> {
        self.page_at("/").await
    }

    /// Read the JS chunks of `pages` until the action `name` turns up; keeps every id found.
    async fn discover_action(&self, name: &str, pages: &[String], found: &mut BTreeMap<String, String>) -> Result<String> {
        let mut first_err = None;
        let mut loaded_any = false;
        for page in pages {
            let html = match self.page_at(page).await {
                Ok(h) => {
                    loaded_any = true;
                    h
                }
                Err(e) if e.kind == aiscraft_core::ErrorKind::Auth => return Err(e),
                Err(e) => {
                    first_err.get_or_insert(e);
                    continue;
                }
            };
            let paths: Vec<String> = {
                let mut scanned = self.scanned.lock().unwrap_or_else(PoisonError::into_inner);
                scrape::chunk_paths(&html).into_iter().take(MAX_CHUNKS).filter(|p| scanned.insert(p.clone())).collect()
            };
            let fetched: Vec<(String, Option<String>)> = stream::iter(paths)
                .map(|p| async move {
                    let js = async {
                        let resp = self.http.get(format!("{}{p}", self.base)).send().await.ok()?;
                        if !resp.status().is_success() {
                            return None;
                        }
                        read_capped(resp, PAGE_CAP).await.ok()
                    }
                    .await;
                    (p, js)
                })
                .buffer_unordered(8)
                .collect()
                .await;
            for (path, js) in fetched {
                match js {
                    Some(js) => found.extend(scrape::action_ids(&js)),
                    // Not read: try it again next time.
                    None => {
                        self.scanned.lock().unwrap_or_else(PoisonError::into_inner).remove(&path);
                    }
                }
            }
            if let Some(id) = found.get(name) {
                return Ok(id.clone());
            }
        }
        match first_err {
            Some(e) if !loaded_any => Err(e),
            _ => {
                Err(Error::protocol(format!("{} von AIS.chat wurde in der Web-App nicht gefunden. Vermutlich hat sich AIS.chat geändert.", action_label(name))))
            }
        }
    }

    async fn action_id_for(&self, name: &str, pages: &[String], refresh: bool) -> Result<String> {
        let mut guard = self.actions.lock().await;
        if refresh {
            guard.clear();
            self.scanned.lock().unwrap_or_else(PoisonError::into_inner).clear();
        } else if let Some(id) = guard.get(name) {
            return Ok(id.clone());
        }
        self.discover_action(name, pages, &mut guard).await
    }

    async fn post_action(&self, id: &str, body: String) -> Result<Response> {
        let rb = self
            .request(Method::POST, "/")?
            .header("Next-Action", id)
            .header("Accept", "text/x-component")
            .header("Content-Type", "text/plain;charset=UTF-8")
            .header("Origin", self.base.clone())
            .header("Referer", format!("{}/", self.base))
            .body(body);
        self.send(rb).await
    }

    /// Post a server action, finding its id first (and again once, when the deploy changed it).
    /// The response must be a React Flight answer.
    async fn post_named(&self, name: &str, pages: &[String], args: &[Value]) -> Result<Response> {
        let body = encode_action_args(args);
        let mut id = self.action_id_for(name, pages, false).await?;
        let mut resp = self.post_action(&id, body.clone()).await?;
        let stale = resp.status() == StatusCode::NOT_FOUND || resp.headers().contains_key("x-nextjs-action-not-found");
        if stale {
            id = self.action_id_for(name, pages, true).await?;
            resp = self.post_action(&id, body).await?;
        }
        if !resp.status().is_success() {
            return Err(status_error(resp, "AIS.chat").await);
        }
        let ctype = resp.headers().get("content-type").and_then(|c| c.to_str().ok()).unwrap_or("").to_string();
        if !ctype.contains("text/x-component") {
            return Err(Error::protocol(format!("AIS.chat hat nicht wie erwartet geantwortet (Inhaltstyp {ctype:?}).")));
        }
        Ok(resp)
    }

    /// Call a server action that returns a plain value (no stream) and return that value.
    async fn call_action(&self, name: &str, pages: &[String], args: &[Value]) -> Result<Value> {
        let resp = self.post_named(name, pages, args).await?;
        let body = crate::http::read_bytes_capped(resp, PAGE_CAP, "Die Antwort von AIS.chat").await?;
        decode_action_value(&body)
    }

    fn pages_for(&self, name: &str) -> Vec<String> {
        let mut pages: Vec<String> = action_pages(name).iter().map(|p| p.to_string()).collect();
        if name == INGEST_ACTION {
            pages.extend(self.editor_pages.lock().unwrap_or_else(PoisonError::into_inner).iter().cloned());
        }
        pages
    }

    async fn session_user_id(&self) -> Option<String> {
        let resp = self.send(self.request(Method::GET, "/api/auth/session").ok()?).await.ok()?;
        let v = read_json(resp, "Sitzung").await.ok()?;
        v.pointer("/user/id").and_then(Value::as_str).map(str::to_string)
    }

    /// Fetch a generated image: a path on AIS.chat (with the session), a signed storage URL
    /// (without it), or a `data:` URL.
    async fn fetch_image(&self, url: &str) -> Result<(Vec<u8>, String)> {
        use base64::Engine as _;
        if let Some(rest) = url.strip_prefix("data:") {
            let (meta, data) = rest.split_once(',').ok_or_else(|| Error::protocol("Bild: ungültige data-URL"))?;
            let mime = meta.split(';').next().filter(|m| m.starts_with("image/")).unwrap_or("image/png").to_string();
            let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| Error::protocol("Bild: ungültige Base64-Daten"))?;
            if bytes.len() > IMAGE_CAP {
                return Err(Error::protocol("Das Bild ist größer als 25 MB"));
            }
            return Ok((bytes, mime));
        }
        let rb = if url.starts_with('/') && !url.starts_with("//") {
            self.request(Method::GET, url)?
        } else if url.starts_with("https://") || url.starts_with("http://") {
            if url.starts_with(&format!("{}/", self.base)) { self.http.get(url).header("Cookie", self.cookie()?) } else { self.http.get(url) }
        } else {
            return Err(Error::protocol("AIS.chat hat keine gültige Bildadresse geliefert"));
        };
        let resp = rb.send().await.map_err(net_err)?;
        if !resp.status().is_success() {
            return Err(status_error(resp, "Bild laden").await);
        }
        let mime = resp
            .headers()
            .get("content-type")
            .and_then(|c| c.to_str().ok())
            .map(|c| c.split(';').next().unwrap_or("").trim().to_string())
            .filter(|c| c.starts_with("image/"))
            .unwrap_or_else(|| "image/png".into());
        let bytes = crate::http::read_bytes_capped(resp, IMAGE_CAP, "Das Bild").await?;
        Ok((bytes, mime))
    }
}

/// The value of a plain (non-streaming) server action response.
pub fn decode_action_value(body: &[u8]) -> Result<Value> {
    let mut d = ActionDecoder::new("stream");
    let mut evs = d.feed(body).map_err(|e| Error::protocol(format!("AIS.chat-Antwort nicht lesbar: {}", e.0)))?;
    evs.extend(d.finish());
    for ev in evs {
        match ev {
            ActionEvent::Result(v) => return Ok(v),
            ActionEvent::Error(msg) => return Err(ais_error("", &msg)),
            _ => {}
        }
    }
    Err(Error::protocol("AIS.chat hat kein Ergebnis geliefert"))
}

/// Unwrap AIS.chat's `runServerAction` result `{success, value | error}`; other values pass.
pub fn server_result(v: Value) -> Result<Value> {
    match v.get("success").and_then(Value::as_bool) {
        Some(true) => Ok(v.get("value").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let err = v.get("error");
            let name = err.and_then(|e| e.get("name")).and_then(Value::as_str).unwrap_or("");
            let message = err.and_then(|e| e.get("message")).and_then(Value::as_str).unwrap_or("");
            Err(ais_error(name, if message.is_empty() && name.is_empty() { "Unerwarteter Fehler auf dem Server" } else { message }))
        }
        None => Ok(v),
    }
}

/// Entities from `get…ByFilterAction`: own ones first, then by name. Unnamed or deleted ones are
/// left out, at most 500.
pub fn parse_personas(kind: PersonaKind, v: &Value, me: Option<&str>) -> Vec<PersonaInfo> {
    let list = v.as_array().or_else(|| v.get("value").and_then(Value::as_array));
    let Some(list) = list else { return Vec::new() };
    let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string);
    let mut out: Vec<PersonaInfo> = list
        .iter()
        .filter(|o| o.get("isDeleted").and_then(Value::as_bool) != Some(true) && o.get("suspended").and_then(Value::as_bool) != Some(true))
        .filter_map(|o| {
            let id = s(o, "id")?;
            if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || id.len() > 64 {
                return None;
            }
            Some(PersonaInfo {
                kind,
                id,
                name: s(o, "name")?.chars().take(200).collect(),
                description: s(o, "description").map(|d| d.chars().take(2000).collect()),
                own: me.is_some_and(|me| o.get("userId").and_then(Value::as_str) == Some(me)),
                initial_message: s(o, "initialMessage"),
                exercise: s(o, "studentExercise"),
                prompt_suggestions: o
                    .get("promptSuggestions")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter().filter_map(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).take(8).map(|x| x.chars().take(500).collect()).collect()
                    })
                    .unwrap_or_default(),
                model_id: s(o, "modelId"),
                picture_url: s(o, "maybeSignedPictureUrl").filter(|u| picture_url_ok(u)),
                favorite: o.get("isFavorite").and_then(Value::as_bool) == Some(true),
                web_search: o.get("isWebSearchEnabled").and_then(Value::as_bool),
            })
        })
        .take(500)
        .collect();
    out.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// A picture URL worth fetching: https, a host, at most 4096 characters, no spaces or controls.
fn picture_url_ok(u: &str) -> bool {
    u.len() <= 4096
        && u.strip_prefix("https://").is_some_and(|rest| rest.split(['/', '?', '#']).next().is_some_and(|host| !host.is_empty() && !host.contains('@')))
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The action's single argument.
pub fn action_payload(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| m.role == Role::User || (m.role == Role::Assistant && !m.content.is_empty()))
        .map(|m| {
            // Links go along as text: AIS.chat's web scraper reads the pages a message names.
            let links: Vec<&str> = m.attachments.iter().filter(|a| a.is_link()).map(|a| a.name.as_str()).collect();
            let content = if links.is_empty() { m.content.clone() } else { format!("{}\n\n{}", m.content, links.join("\n")).trim().to_string() };
            json!({"id": m.id, "role": m.role.as_str(), "content": content})
        })
        .collect();
    let file_ids: Vec<&str> = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| m.attachments.iter().filter_map(|a| a.remote_id.as_deref()).collect())
        .unwrap_or_default();
    let mut p = json!({"conversationId": req.conversation_id, "messages": messages, "modelId": req.model});
    if let Some(o) = p.as_object_mut() {
        if !file_ids.is_empty() {
            o.insert("fileIds".into(), json!(file_ids));
        }
        if let Some(persona) = &req.persona {
            let key = match persona.kind {
                PersonaKind::Assistant => "assistantId",
                PersonaKind::Character => "characterId",
                PersonaKind::LearningScenario => "learningScenarioId",
            };
            o.insert(key.into(), json!(persona.id));
        }
    }
    p
}

fn ais_error(name: &str, message: &str) -> Error {
    match name {
        "TokenPointsExceededError" => Error::refused("Dein Kontingent bei AIS.chat ist für diesen Zeitraum aufgebraucht."),
        "ResponsibleAIError" => Error::refused("AIS.chat hat die Anfrage wegen unangemessener Inhalte blockiert."),
        "SharedChatExpiredError" => Error::refused("Dieser geteilte Chat ist abgelaufen."),
        "NotFoundError" => Error::not_found(if message.is_empty() { "Nicht gefunden" } else { message }),
        _ if message.contains("training") || message.contains("Training") => Error::refused(format!("AIS.chat: {message} (Schulung im Browser abschließen)")),
        _ => Error::refused(if message.is_empty() { name.to_string() } else { format!("AIS.chat: {message}") }),
    }
}

/// The tools AIS.chat runs for this person, from the state's feature toggles (see
/// `apps/chat-bot/src/app/api/chat/build-tools.ts`). Documents and file search are always on.
pub fn server_tools(toggles: Option<&Value>) -> Vec<aiscraft_core::ServerTool> {
    let on = |k: &str| toggles.and_then(|t| t.get(k)).and_then(Value::as_bool).unwrap_or(false);
    let t = |id: &str, label: &str, enabled: bool| aiscraft_core::ServerTool { id: id.into(), label: label.into(), enabled };
    if toggles.is_none() {
        return Vec::new();
    }
    vec![
        t("web_search", "Websuche und Webseiten lesen", on("isWebSearchEnabled")),
        t("documents", "Dokumente erzeugen", true),
        t("files", "Angehängte Dateien durchsuchen", true),
        t("plots", "Diagramme (JSXGraph)", on("isJsxGraphEnabled")),
        t("calculator", "Rechner", on("isCalculatorEnabled")),
        t("images", "Bilder erzeugen", on("isImageGenerationEnabled")),
        t("speech", "Sprachausgabe", on("isSpeechModelEnabled")),
        t("agentic", "Mehrschrittiges Arbeiten (Agent)", on("isAgenticChatEnabled")),
        t("characters", "Dialogpartner", on("isCharacterEnabled")),
        t("custom", "Eigene Assistenten", on("isCustomGptEnabled")),
    ]
}

fn tool_label(tool: &str) -> &'static str {
    let t = tool.to_ascii_lowercase();
    if t.contains("websearch") || t.contains("web_search") {
        "Websuche"
    } else if t.contains("scrap") {
        "Webseite lesen"
    } else if t.contains("entire") {
        "Datei lesen"
    } else if t.contains("chunk") || t.contains("file") {
        "Datei durchsuchen"
    } else if t.contains("mundo") {
        "Suche in Mundo"
    } else if t.contains("math") || t.contains("calc") {
        "Rechnen"
    } else if t.contains("plot") || t.contains("graph") {
        "Diagramm erstellen"
    } else if t.contains("document") {
        "Dokument erstellen"
    } else {
        "Werkzeug"
    }
}

/// Map AIS.chat's `aiActivityStepSchema` steps.
pub fn activity_steps(v: &Value) -> Vec<ActivityStep> {
    let Some(steps) = v.get("steps").and_then(Value::as_array) else { return Vec::new() };
    let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
    steps
        .iter()
        .filter_map(|st| {
            let kind = st.get("kind").and_then(Value::as_str)?;
            Some(match kind {
                "analysis" => ActivityStep { label: "Analyse".into(), detail: s(st, "summary"), ..Default::default() },
                "analysis-summary" => ActivityStep { label: "Zusammenfassung".into(), detail: s(st, "content"), done: true, ..Default::default() },
                "done" => ActivityStep { label: "Fertig".into(), done: true, ..Default::default() },
                "tool" => ActivityStep {
                    label: tool_label(st.get("tool").and_then(Value::as_str).unwrap_or("")).into(),
                    detail: s(st, "detail").or_else(|| s(st, "result")),
                    done: st.get("result").is_some() || st.get("document").is_some(),
                    failed: st.get("failed").and_then(Value::as_bool).unwrap_or(false),
                    links: st
                        .get("links")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|l| {
                                    let url = s(l, "url")?;
                                    let title = s(l, "title").filter(|t| !t.trim().is_empty()).unwrap_or_else(|| url.clone());
                                    Some(ActivityLink { title, url, failed: l.get("failed").and_then(Value::as_bool).unwrap_or(false), cited: false })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    document: st.get("document").and_then(|d| {
                        Some(GeneratedDocument { file_name: s(d, "fileName")?, format: s(d, "format").unwrap_or_default(), content: s(d, "content")? })
                    }),
                },
                _ => return None,
            })
        })
        .collect()
}

/// Map decoded action events to stream events.
pub fn map_action_event(ev: ActionEvent) -> Result<Vec<StreamEvent>> {
    Ok(match ev {
        ActionEvent::Result(v) => {
            if let Some(err) = v.get("error").filter(|e| e.is_object()) {
                let name = err.get("name").and_then(Value::as_str).unwrap_or("");
                let message = err.get("message").and_then(Value::as_str).unwrap_or("");
                return Err(ais_error(name, message));
            }
            v.get("messageId").and_then(Value::as_str).map(|id| vec![StreamEvent::MessageId { id: id.to_string() }]).unwrap_or_default()
        }
        ActionEvent::Chunk(text) => {
            if let Some(rest) = text.strip_prefix('\u{1e}') {
                match serde_json::from_str::<Value>(rest) {
                    Ok(v) if v.get("type").and_then(Value::as_str) == Some("ai_activity") => vec![StreamEvent::Activity { steps: activity_steps(&v) }],
                    _ => vec![],
                }
            } else if text.is_empty() {
                vec![]
            } else {
                vec![StreamEvent::Delta { text }]
            }
        }
        ActionEvent::Closed => vec![StreamEvent::Done],
        ActionEvent::Error(msg) => return Err(ais_error("", &msg)),
    })
}

fn action_stream(resp: Response) -> EventStream {
    struct St {
        body: futures::stream::BoxStream<'static, reqwest::Result<bytes::Bytes>>,
        decoder: ActionDecoder,
        queue: std::collections::VecDeque<Result<StreamEvent>>,
        finished: bool,
    }
    let st = St { body: resp.bytes_stream().boxed(), decoder: ActionDecoder::new("stream"), queue: Default::default(), finished: false };
    let push = |st: &mut St, evs: Vec<ActionEvent>| {
        for ev in evs {
            match map_action_event(ev) {
                Ok(out) => st.queue.extend(out.into_iter().map(Ok)),
                Err(e) => st.queue.push_back(Err(e)),
            }
        }
    };
    stream::unfold(st, move |mut st| async move {
        loop {
            if let Some(item) = st.queue.pop_front() {
                if matches!(item, Ok(StreamEvent::Done) | Err(_)) {
                    st.finished = true;
                    st.queue.clear();
                }
                return Some((item, st));
            }
            if st.finished {
                return None;
            }
            match st.body.next().await {
                Some(Ok(bytes)) => match st.decoder.feed(&bytes) {
                    Ok(evs) => push(&mut st, evs),
                    Err(e) => st.queue.push_back(Err(Error::protocol(format!("AIS.chat-Antwort nicht lesbar: {}", e.0)))),
                },
                Some(Err(e)) => st.queue.push_back(Err(net_err(e))),
                None => {
                    let evs = st.decoder.finish();
                    if evs.is_empty() {
                        st.queue.push_back(Ok(StreamEvent::Done));
                    } else {
                        push(&mut st, evs);
                    }
                }
            }
        }
    })
    .boxed()
}

impl Backend for AisChat {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { remote_history: true, uploads: true, system_prompt: false, vision: true, tools: false, tools_via_mcp: false, web_search: false }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let resp = self.send(self.request(Method::GET, "/api/auth/session")?).await?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Sitzung").await);
            }
            let v = read_json(resp, "Sitzung").await?;
            let Some(user) = v.get("user").filter(|u| u.is_object()) else {
                return Err(Error::auth("AIS.chat-Sitzung abgelaufen. Bitte neu anmelden."));
            };
            let s = |p: &str| user.pointer(p).and_then(Value::as_str).map(str::to_string);
            let name = match (s("/firstName"), s("/lastName")) {
                (Some(f), Some(l)) => Some(format!("{f} {l}")),
                (Some(f), None) => Some(f),
                _ => s("/name").or_else(|| s("/email")),
            };
            let detail = [s("/school/name"), s("/federalState/name").or_else(|| s("/federalState/id")), s("/userRole").or_else(|| s("/role"))]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            Ok(AccountInfo {
                display_name: name,
                detail: (!detail.is_empty()).then_some(detail),
                expires: v.get("expires").and_then(Value::as_str).map(str::to_string),
                server_tools: server_tools(user.pointer("/federalState/featureToggles")),
            })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let payload = scrape::rsc_payload(&self.page().await?);
            let initial = scrape::initial_model_name(&payload);
            let mut models: Vec<ModelInfo> = scrape::page_models(&payload)
                .into_iter()
                .filter(|m| m.kind == "text")
                .map(|m| ModelInfo {
                    id: m.id,
                    name: m.display_name,
                    description: (!m.description.is_empty()).then_some(m.description),
                    vision: m.vision,
                    context_window: None,
                })
                .collect();
            if models.is_empty() {
                return Err(Error::protocol("AIS.chat hat keine Modelle geliefert. Ist die Schulung abgeschlossen?"));
            }
            if let Some(name) = initial
                && let Some(i) = scrape::page_models(&payload).iter().filter(|m| m.kind == "text").position(|m| m.name == name)
                && i < models.len()
            {
                let m = models.remove(i);
                models.insert(0, m);
            }
            Ok(models)
        })
    }

    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>> {
        Box::pin(async move {
            let resp = self.post_named(SEND_ACTION, &self.pages_for(SEND_ACTION), &[action_payload(&req)]).await?;
            Ok(action_stream(resp))
        })
    }

    fn upload(&self, name: String, mime: String, bytes: Vec<u8>) -> BoxFuture<'_, Result<String>> {
        Box::pin(async move {
            let part = reqwest::multipart::Part::bytes(bytes).file_name(name).mime_str(&mime).map_err(|e| Error::invalid(format!("Dateityp: {e}")))?;
            let form = reqwest::multipart::Form::new().part("file", part);
            let resp = self.send(self.request(Method::POST, "/api/v1/files")?.multipart(form)).await?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Hochladen").await);
            }
            let v = read_json(resp, "Hochladen").await?;
            // The route answers {"body": "{\"file_id\":…}", "status": 200}.
            let inner = v.get("body").and_then(Value::as_str).and_then(|b| serde_json::from_str::<Value>(b).ok()).unwrap_or(v);
            inner.get("file_id").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::protocol("Hochladen: keine Datei-ID erhalten"))
        })
    }

    fn remote_conversations(&self) -> BoxFuture<'_, Result<Vec<RemoteConversation>>> {
        Box::pin(async move {
            let resp = self.send(self.request(Method::GET, "/api/conversations")?).await?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Verlauf").await);
            }
            let v = read_json(resp, "Verlauf").await?;
            Ok(v.get("conversations")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            let s = |k: &str| c.get(k).and_then(Value::as_str).map(str::to_string);
                            let persona = [
                                ("assistantId", PersonaKind::Assistant),
                                ("characterId", PersonaKind::Character),
                                ("learningScenarioId", PersonaKind::LearningScenario),
                            ]
                            .into_iter()
                            .find_map(|(k, kind)| s(k).filter(|v| !v.is_empty()).map(|id| Persona { kind, id, name: String::new() }));
                            Some(RemoteConversation {
                                id: s("id")?,
                                title: s("name").or_else(|| s("title")).unwrap_or_else(|| "Ohne Titel".into()),
                                updated_at: s("updatedAt").or_else(|| s("createdAt")),
                                persona,
                                kind: s("type").and_then(|t| ConversationKind::parse(&t)).unwrap_or_default(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn quota(&self) -> BoxFuture<'_, Result<Option<aiscraft_core::Quota>>> {
        Box::pin(async move {
            let payload = scrape::rsc_payload(&self.page().await?);
            Ok(scrape::page_budget(&payload).map(|(used, max)| aiscraft_core::Quota { used, max }))
        })
    }

    fn remote_messages(&self, id: String) -> BoxFuture<'_, Result<Vec<(String, Role, String)>>> {
        Box::pin(async move {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(Error::invalid("Ungültige Chat-ID"));
            }
            let resp = self.send(self.request(Method::GET, &format!("/d/{id}"))?.header("Accept", "text/html")).await?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Chat laden").await);
            }
            let payload = scrape::rsc_payload(&read_capped(resp, PAGE_CAP).await?);
            let msgs: Vec<_> = scrape::page_messages(&payload).into_iter().filter_map(|(id, role, content)| Some((id, Role::parse(&role)?, content))).collect();
            if msgs.is_empty() {
                return Err(Error::protocol("In diesem AIS.chat-Verlauf wurden keine Nachrichten gefunden"));
            }
            Ok(msgs)
        })
    }

    fn known_actions(&self) -> BTreeMap<String, String> {
        self.actions.try_lock().map(|g| g.clone()).unwrap_or_default()
    }

    fn seed_actions(&self, ids: BTreeMap<String, String>) {
        if let Ok(mut g) = self.actions.try_lock() {
            for (name, id) in ids {
                g.entry(name).or_insert(id);
            }
        }
    }

    fn personas(&self) -> BoxFuture<'_, Result<Vec<PersonaInfo>>> {
        Box::pin(async move {
            let kinds = [
                (PersonaKind::Assistant, "getAssistantsByFilterAction"),
                (PersonaKind::Character, "getCharactersByFilterAction"),
                (PersonaKind::LearningScenario, "getLearningScenariosByFilterAction"),
            ];
            // The three lists and the session at the same time: AIS.chat takes a second or two
            // for each list.
            // "all": own, school, community and official ones (`OverviewFilter`).
            let lists = futures::future::join_all(
                kinds.iter().map(|(_, action)| async move { self.call_action(action, &self.pages_for(action), &[json!("all")]).await.and_then(server_result) }),
            );
            let (me, results) = futures::join!(self.session_user_id(), lists);
            let mut out = Vec::new();
            let mut errors = Vec::new();
            for ((kind, _), result) in kinds.iter().zip(results) {
                match result {
                    Ok(v) => out.extend(parse_personas(*kind, &v, me.as_deref())),
                    Err(e) if e.kind == aiscraft_core::ErrorKind::Auth => return Err(e),
                    // A kind the state switched off (its page redirects) is simply absent.
                    Err(e) => errors.push(e),
                }
            }
            if out.is_empty()
                && errors.len() == 3
                && let Some(e) = errors.into_iter().next()
            {
                return Err(e);
            }
            let pages: Vec<String> = PersonaKind::ALL
                .iter()
                .filter_map(|k| out.iter().find(|p| p.own && p.kind == *k))
                .map(|p| {
                    let base = match p.kind {
                        PersonaKind::Assistant => "/assistants",
                        PersonaKind::Character => "/characters",
                        PersonaKind::LearningScenario => "/learning-scenarios",
                    };
                    format!("{base}/editor/{}", p.id)
                })
                .collect();
            *self.editor_pages.lock().unwrap_or_else(PoisonError::into_inner) = pages;
            Ok(out)
        })
    }

    fn set_favorite(&self, kind: PersonaKind, id: String, favorite: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            // AIS.chat wants a UUID; refuse anything that could not be an id before asking it.
            if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(Error::invalid("Ungültige ID"));
            }
            let entity_type = match kind {
                PersonaKind::Assistant => "assistant",
                PersonaKind::Character => "character",
                PersonaKind::LearningScenario => "learningScenario",
            };
            let arg = json!({"entityType": entity_type, "entityId": id, "isFavorite": favorite});
            server_result(self.call_action(FAVORITE_ACTION, &self.pages_for(FAVORITE_ACTION), &[arg]).await?).map(|_| ())
        })
    }

    fn image_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            // The page redirects to "/" when the state has image generation switched off.
            let html = match self.page_at("/image-generation").await {
                Ok(h) => h,
                Err(e) if e.kind == aiscraft_core::ErrorKind::Auth => return Err(e),
                Err(_) => return Ok(Vec::new()),
            };
            Ok(scrape::page_models(&scrape::rsc_payload(&html))
                .into_iter()
                .filter(|m| m.kind == "image")
                .map(|m| ModelInfo {
                    id: m.id,
                    name: m.display_name,
                    description: (!m.description.is_empty()).then_some(m.description),
                    vision: m.vision,
                    context_window: None,
                })
                .collect())
        })
    }

    fn generate_image(&self, req: ImageRequest) -> BoxFuture<'_, Result<ImageResult>> {
        Box::pin(async move {
            if req.prompt.trim().is_empty() {
                return Err(Error::invalid("Beschreibe, was das Bild zeigen soll"));
            }
            let aspect = if aiscraft_core::ASPECT_RATIOS.iter().any(|(k, _)| *k == req.aspect_ratio) { req.aspect_ratio.as_str() } else { "quadratic" };
            let mut arg = json!({"prompt": req.prompt.trim(), "modelId": req.model, "options": {"aspectRatio": aspect}, "inputFileIds": []});
            if let (Some(id), Some(o)) = (&req.conversation_id, arg.as_object_mut()) {
                o.insert("conversationId".into(), json!(id));
            }
            let v = server_result(self.call_action("generateImageAction", &self.pages_for("generateImageAction"), &[arg]).await?)?;
            let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
            let url = s("imageUrl").filter(|u| !u.is_empty()).ok_or_else(|| Error::protocol("AIS.chat hat kein Bild geliefert"))?;
            let (bytes, mime) = self.fetch_image(&url).await?;
            if bytes.is_empty() {
                return Err(Error::protocol("AIS.chat hat ein leeres Bild geliefert"));
            }
            Ok(ImageResult { bytes, mime, conversation_id: s("conversationId"), message_id: s("assistantMessageId") })
        })
    }

    fn check_link(&self, url: String) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move {
            let v = match self.call_action(INGEST_ACTION, &self.pages_for(INGEST_ACTION), &[json!({"url": url})]).await {
                Ok(v) => server_result(v)?,
                // The action ships only with editor pages; without one, the server cannot be asked.
                Err(e) if e.kind == aiscraft_core::ErrorKind::Protocol && e.message.contains("nicht gefunden") => return Ok(false),
                Err(e) => return Err(e),
            };
            let failed = v.get("errorUrls").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
            if failed {
                return Err(Error::invalid(format!("AIS.chat kann die Seite {url} nicht lesen. Ist die Adresse richtig und öffentlich erreichbar?")));
            }
            Ok(true)
        })
    }

    fn web_url(&self, conversation_id: &str) -> Option<String> {
        Some(format!("{}/d/{conversation_id}", self.base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiscraft_core::{Attachment, Message};

    #[test]
    fn server_tools_from_toggles() {
        let t = json!({"isWebSearchEnabled": true, "isCalculatorEnabled": false, "isSpeechModelEnabled": false, "isJsxGraphEnabled": true});
        let tools = server_tools(Some(&t));
        let get = |id: &str| tools.iter().find(|x| x.id == id).unwrap().enabled;
        assert!(get("web_search") && get("plots") && get("documents"));
        assert!(!get("calculator") && !get("speech"));
        assert!(server_tools(None).is_empty());
    }

    #[test]
    fn cookie_input_forms() {
        assert_eq!(normalize_session_cookie("abc.def"), Some(format!("{SESSION_COOKIE}=abc.def")));
        let pasted = format!("Cookie: other=1; {SESSION_COOKIE}.1=bb; {SESSION_COOKIE}.0=aa; __Host-x=2");
        assert_eq!(normalize_session_cookie(&pasted), Some(format!("{SESSION_COOKIE}.0=aa; {SESSION_COOKIE}.1=bb")));
        assert_eq!(normalize_session_cookie("foo=bar"), None);
        assert_eq!(normalize_session_cookie("  "), None);
    }

    #[test]
    fn set_cookie_renews_and_expires() {
        let mut jar = BTreeMap::from([(SESSION_COOKIE.to_string(), "old".to_string())]);
        let mut h = HeaderMap::new();
        h.append(SET_COOKIE, format!("{SESSION_COOKIE}=new; Path=/; Secure; HttpOnly").parse().unwrap());
        h.append(SET_COOKIE, "unrelated=1; Path=/".parse().unwrap());
        assert!(absorb_set_cookie(&mut jar, &h));
        assert_eq!(jar.get(SESSION_COOKIE).map(String::as_str), Some("new"));
        let mut h = HeaderMap::new();
        h.append(SET_COOKIE, format!("{SESSION_COOKIE}=; Max-Age=0").parse().unwrap());
        assert!(absorb_set_cookie(&mut jar, &h));
        assert!(jar.is_empty());
    }

    #[test]
    fn payload_shape() {
        let mut u = Message {
            id: "u1".into(),
            conversation_id: "c1".into(),
            role: Role::User,
            content: "$5?".into(),
            created_at: 0,
            model: None,
            attachments: vec![Attachment { remote_id: Some("f1".into()), ..Default::default() }],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        let a = Message { id: "a0".into(), role: Role::Assistant, content: String::new(), attachments: vec![], ..u.clone() };
        u.id = "u1".into();
        let req = ChatRequest {
            conversation_id: "c1".into(),
            model: "uuid-m".into(),
            system: Some("ignored".into()),
            messages: vec![u, a],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
        };
        let body = encode_action_args(&[action_payload(&req)]);
        assert_eq!(body, r#"[{"conversationId":"c1","fileIds":["f1"],"messages":[{"content":"$$5?","id":"u1","role":"user"}],"modelId":"uuid-m"}]"#);
    }

    #[test]
    fn maps_activity_and_errors() {
        let ev = ActionEvent::Chunk("\u{1e}{\"type\":\"ai_activity\",\"steps\":[{\"kind\":\"tool\",\"id\":\"1\",\"tool\":\"webSearch\",\"detail\":\"Wetter\",\"links\":[{\"title\":\"DWD\",\"url\":\"https://dwd.de\"}]},{\"kind\":\"tool\",\"id\":\"2\",\"tool\":\"generateDocument\",\"document\":{\"fileName\":\"a.docx\",\"format\":\"docx\",\"content\":\"x\"}}]}".into());
        let out = map_action_event(ev).unwrap();
        let StreamEvent::Activity { steps } = &out[0] else { panic!() };
        assert_eq!(steps[0].label, "Websuche");
        assert_eq!(steps[0].links[0].url, "https://dwd.de");
        assert_eq!(steps[1].document.as_ref().unwrap().file_name, "a.docx");
        // Tool names as AIS.chat sends them (TOOL_NAMES), a plot description, a page it could not read.
        let ev = ActionEvent::Chunk("\u{1e}{\"type\":\"ai_activity\",\"steps\":[{\"kind\":\"tool\",\"id\":\"1\",\"tool\":\"create_plot\",\"detail\":\"Parabel y=x^2\",\"detailPlacement\":\"below\"},{\"kind\":\"tool\",\"id\":\"2\",\"tool\":\"web_scraper\",\"links\":[{\"title\":\"\",\"url\":\"https://a.example\",\"failed\":true}],\"failed\":true},{\"kind\":\"tool\",\"id\":\"3\",\"tool\":\"retrieve_entire_file\"}]}".into());
        let StreamEvent::Activity { steps } = &map_action_event(ev).unwrap()[0] else { panic!() };
        assert_eq!(steps.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Diagramm erstellen", "Webseite lesen", "Datei lesen"]);
        assert_eq!(steps[0].detail.as_deref(), Some("Parabel y=x^2"));
        assert!(steps[1].failed && steps[1].links[0].failed && steps[1].links[0].title == "https://a.example");
        let err = map_action_event(ActionEvent::Result(json!({"stream":"$2","messageId":"m","error":{"name":"TokenPointsExceededError","message":"x"}})))
            .unwrap_err();
        assert_eq!(err.kind, aiscraft_core::ErrorKind::Refused);
        assert_eq!(map_action_event(ActionEvent::Result(json!({"messageId":"m9"}))).unwrap(), vec![StreamEvent::MessageId { id: "m9".into() }]);
    }

    /// The whole send path against a scripted local server that speaks like AIS.chat.
    #[tokio::test]
    async fn chat_against_scripted_server() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let action = "7f".repeat(21);
        let srv_action = action.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let action = srv_action.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let (ctype, extra, body) = if req.starts_with("GET / ") {
                        ("text/html", String::new(), r#"<script src="/_next/static/chunks/app.js"></script>"#.to_string())
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        (
                            "application/javascript",
                            String::new(),
                            format!(r#"(0,a.createServerReference)("{action}",a.callServer,void 0,a.findSourceMapURL,"sendChatMessageAction")"#),
                        )
                    } else if req.starts_with("POST / ") && req.contains(&format!("next-action: {action}")) && req.contains("cookie: __Secure-ais-chat-app") {
                        (
                            "text/x-component",
                            format!("set-cookie: {SESSION_COOKIE}=renewed; Path=/; Secure\r\n"),
                            "0:{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}\n1:{\"stream\":\"$2\",\"messageId\":\"srv-1\"}\n2:R\n2:T6,Hallo 2:\"Welt\"\n2:C\n"
                                .to_string(),
                        )
                    } else {
                        ("text/plain", String::new(), "nope".to_string())
                    };
                    let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        let renewed = std::sync::Arc::new(Mutex::new(None::<String>));
        let r2 = renewed.clone();
        let cfg = BackendConfig {
            id: "ais".into(),
            name: "AIS".into(),
            kind: aiscraft_core::BackendKind::AisChat,
            base_url: format!("http://{addr}"),
            default_model: None,
            send_context: false,
        };
        let backend = crate::build(cfg, Some("tok".into()), std::sync::Arc::new(move |_, s| *r2.lock().unwrap() = Some(s))).unwrap();
        let msg = Message {
            id: "u".into(),
            conversation_id: "c".into(),
            role: Role::User,
            content: "Hi".into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        let mut s = backend
            .chat(ChatRequest {
                conversation_id: "c".into(),
                model: "m".into(),
                system: None,
                messages: vec![msg],
                tools: vec![],
                tool_bridge: None,
                persona: None,
                web: false,
            })
            .await
            .unwrap();
        let mut evs = Vec::new();
        while let Some(e) = s.next().await {
            evs.push(e.unwrap());
        }
        assert_eq!(
            evs,
            vec![
                StreamEvent::MessageId { id: "srv-1".into() },
                StreamEvent::Delta { text: "Hallo ".into() },
                StreamEvent::Delta { text: "Welt".into() },
                StreamEvent::Done
            ]
        );
        assert_eq!(renewed.lock().unwrap().as_deref(), Some(format!("{SESSION_COOKIE}=renewed").as_str()));
    }

    #[test]
    fn personas_from_filter_results() {
        let v = json!([
            {"id": "c2", "name": "Goethe", "description": "Dichter", "userId": "other", "initialMessage": "Seid gegrüßt!", "modelId": "m1"},
            {"id": "c1", "name": "Ada Lovelace", "userId": "me", "promptSuggestions": ["Was ist ein Algorithmus?", "", 5]},
            {"id": "c3", "name": "  ", "userId": "me"},
            {"id": "bad id!", "name": "X"},
            {"id": "c4", "name": "Weg", "isDeleted": true}
        ]);
        let list = parse_personas(PersonaKind::Character, &v, Some("me"));
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Ada Lovelace", "Goethe"], "own first, unnamed/deleted/odd ids dropped");
        assert!(list[0].own && !list[1].own);
        assert_eq!(list[0].prompt_suggestions, ["Was ist ein Algorithmus?"]);
        assert_eq!(list[1].initial_message.as_deref(), Some("Seid gegrüßt!"));
        assert!(!list[0].favorite && list[0].picture_url.is_none());

        let long = format!("https://s3.example/{}", "a".repeat(4100));
        let v = json!([
            {"id": "p1", "name": "Mit Bild", "maybeSignedPictureUrl": "https://s3.example/avatar_1?X-Amz-Signature=abc", "isFavorite": true, "isWebSearchEnabled": true},
            {"id": "p2", "name": "Http", "maybeSignedPictureUrl": "http://s3.example/a.png", "isFavorite": "ja", "isWebSearchEnabled": false},
            {"id": "p7", "name": "Websuche kaputt", "isWebSearchEnabled": "ja"},
            {"id": "p3", "name": "Lang", "maybeSignedPictureUrl": long},
            {"id": "p4", "name": "Leer", "maybeSignedPictureUrl": "https:///x"},
            {"id": "p5", "name": "Leerzeichen", "maybeSignedPictureUrl": "https://s3.example/a b.png"},
            {"id": "p6", "name": "Nutzer", "maybeSignedPictureUrl": "https://ich@s3.example/a.png"}
        ]);
        let list = parse_personas(PersonaKind::LearningScenario, &v, None);
        let by = |n: &str| list.iter().find(|p| p.name == n).unwrap();
        assert_eq!(by("Mit Bild").picture_url.as_deref(), Some("https://s3.example/avatar_1?X-Amz-Signature=abc"));
        assert!(by("Mit Bild").favorite);
        assert!(!by("Http").favorite, "only a real true counts");
        assert_eq!(by("Mit Bild").web_search, Some(true));
        assert_eq!(by("Http").web_search, Some(false));
        assert_eq!(by("Leer").web_search, None, "absent: unknown");
        assert_eq!(by("Websuche kaputt").web_search, None, "not a bool: unknown");
        for n in ["Http", "Lang", "Leer", "Leerzeichen", "Nutzer"] {
            assert!(by(n).picture_url.is_none(), "{n}: only plain https URLs up to 4096 characters");
        }
        assert!(parse_personas(PersonaKind::Assistant, &json!({"success": false}), None).is_empty());
        let err = server_result(json!({"success": false, "error": {"name": "NotFoundError", "message": "weg"}})).unwrap_err();
        assert_eq!(err.kind, aiscraft_core::ErrorKind::NotFound);
        assert_eq!(server_result(json!({"success": true, "value": [1]})).unwrap(), json!([1]));
    }

    #[test]
    fn payload_names_the_persona() {
        let msg = Message {
            id: "u".into(),
            conversation_id: "c".into(),
            role: Role::User,
            content: "Hi".into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        for (kind, key) in
            [(PersonaKind::Assistant, "assistantId"), (PersonaKind::Character, "characterId"), (PersonaKind::LearningScenario, "learningScenarioId")]
        {
            let req = ChatRequest {
                conversation_id: "c".into(),
                model: "m".into(),
                system: None,
                messages: vec![msg.clone()],
                tools: vec![],
                tool_bridge: None,
                persona: Some(Persona { kind, id: "p-1".into(), name: "X".into() }),
                web: false,
            };
            let p = action_payload(&req);
            assert_eq!(p[key], "p-1");
            assert_eq!(p.as_object().unwrap().len(), 4, "only the one id field is added: {p}");
        }
    }

    /// Reads one HTTP request (head and body, by content-length).
    async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let s = String::from_utf8_lossy(&buf);
            if let Some(head_end) = s.find("\r\n\r\n") {
                let len = s[..head_end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if buf.len() >= head_end + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// A scripted AIS.chat: action ids live in page-specific chunks, as in the real app.
    async fn scripted_ais(posted: std::sync::Arc<Mutex<Vec<String>>>) -> String {
        scripted_ais_logged(posted, std::sync::Arc::new(Mutex::new(Vec::new()))).await
    }

    /// The scripted AIS.chat that also records the path of every GET.
    async fn scripted_ais_logged(posted: std::sync::Arc<Mutex<Vec<String>>>, gets: std::sync::Arc<Mutex<Vec<String>>>) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let posted = posted.clone();
                let gets = gets.clone();
                tokio::spawn(async move {
                    let req = read_request(&mut sock).await;
                    if let Some(path) = req.strip_prefix("GET ").and_then(|r| r.split(' ').next()) {
                        gets.lock().unwrap().push(path.to_string());
                    }
                    let id = |n: u8| format!("{:02x}", n).repeat(21);
                    let chunk = |path: &str| format!(r#"<script src="/_next/static/chunks/{path}.js"></script>"#);
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", r#"{"user":{"id":"me","firstName":"Ada"}}"#.to_string())
                    } else if req.starts_with("GET /assistants ") {
                        ("text/html", chunk("shared") + &chunk("assistants"))
                    } else if req.starts_with("GET /characters ") {
                        ("text/html", chunk("shared") + &chunk("characters"))
                    } else if req.starts_with("GET /_next/static/chunks/shared.js") {
                        ("application/javascript", reference(0x44, "someOtherAction"))
                    } else if req.starts_with("GET /learning-scenarios ") {
                        // Switched off for this person: the page redirects, the kind is absent.
                        ("text/html", "<p>nichts</p>".to_string())
                    } else if req.starts_with("GET / ") {
                        ("text/html", chunk("app"))
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        ("application/javascript", reference(0x7f, "sendChatMessageAction"))
                    } else if req.starts_with("GET /image-generation ") {
                        // Shape of 2026-11-24: `defaultImageModel` repeats the model with
                        // back-references and sorts before `models`.
                        let payload = concat!(
                            r#"1f:[["$","$L3e","k",{"models":[{"id":"img-1","name":"gpt-image-1.5","displayName":"GPT-Image-1.5","priceMetadata":{"type":"image"},"isDeleted":false},{"id":"txt-1","name":"gpt-5","displayName":"GPT-5","priceMetadata":{"type":"text"}}],"#,
                            r#""defaultImageModel":{"id":"img-1","name":"gpt-image-1.5","displayName":"GPT-Image-1.5","priceMetadata":"$1f:0:props:models:0:priceMetadata"}}]]"#,
                            "\n"
                        );
                        ("text/html", format!("<script>self.__next_f.push([1,{}])</script>", serde_json::to_string(payload).unwrap()))
                    } else if req.starts_with("GET /_next/static/chunks/assistants.js") {
                        ("application/javascript", reference(0x11, "getAssistantsByFilterAction") + &reference(0x33, "setFavoriteAction"))
                    } else if req.starts_with("GET /_next/static/chunks/characters.js") {
                        ("application/javascript", reference(0x22, "getCharactersByFilterAction"))
                    } else if req.starts_with("POST / ") {
                        posted.lock().unwrap().push(req.clone());
                        if action == id(0x11) {
                            assert!(req.ends_with(r#"["all"]"#), "{req}");
                            (
                                "text/x-component",
                                flight(
                                    r#"[{"id":"a1","name":"Mathe-Coach","userId":"me","description":"Übt Bruchrechnung","updatedAt":"$D2026-11-01T00:00:00.000Z"},{"id":"a2","name":"Schulassistent","userId":"x"}]"#,
                                ),
                            )
                        } else if action == id(0x22) {
                            ("text/x-component", flight(r#"[{"id":"c1","name":"Goethe","userId":"x","initialMessage":"Seid gegrüßt!"}]"#))
                        } else if action == id(0x33) {
                            assert!(req.ends_with(r#"[{"entityId":"0a1b-2c","entityType":"learningScenario","isFavorite":true}]"#), "{req}");
                            ("text/x-component", flight(r#"{"success":true}"#))
                        } else if action == id(0x7f) {
                            (
                                "text/x-component",
                                "0:{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}\n1:{\"stream\":\"$2\",\"messageId\":\"srv-1\"}\n2:R\n2:\"Gern.\"\n2:C\n".to_string(),
                            )
                        } else {
                            ("text/plain", "unbekannte Aktion".to_string())
                        }
                    } else {
                        ("text/plain", "nope".to_string())
                    };
                    let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    fn ais_config(base: String) -> BackendConfig {
        BackendConfig {
            id: "ais".into(),
            name: "AIS".into(),
            kind: aiscraft_core::BackendKind::AisChat,
            base_url: base,
            default_model: None,
            send_context: false,
        }
    }

    /// Quality gate (start-up speed): finding the action ids loads each JS chunk once, even when
    /// several pages share it, and a backend seeded with remembered ids loads no page or chunk.
    #[tokio::test]
    async fn action_discovery_loads_each_chunk_once_and_remembered_ids_load_none() {
        let gets = std::sync::Arc::new(Mutex::new(Vec::new()));
        let base = scripted_ais_logged(std::sync::Arc::new(Mutex::new(Vec::new())), gets.clone()).await;
        let first = crate::build(ais_config(base.clone()), Some("tok".into()), std::sync::Arc::new(|_, _| {})).unwrap();
        assert_eq!(first.personas().await.unwrap().len(), 3);
        let count = |path: &str| gets.lock().unwrap().iter().filter(|g| g.as_str() == path).count();
        assert_eq!(count("/_next/static/chunks/shared.js"), 1, "{:?}", gets.lock().unwrap());
        assert_eq!(count("/_next/static/chunks/assistants.js"), 1);
        assert_eq!(count("/_next/static/chunks/characters.js"), 1);

        gets.lock().unwrap().clear();
        let second = crate::build(ais_config(base), Some("tok".into()), std::sync::Arc::new(|_, _| {})).unwrap();
        second.seed_actions(first.known_actions());
        assert_eq!(second.personas().await.unwrap().len(), 3);
        let loaded = gets.lock().unwrap().clone();
        // Only the session; the switched-off learning scenarios still look once for their action.
        assert!(loaded.iter().all(|g| !g.starts_with("/_next/") || g.contains("learning")), "{loaded:?}");
        assert!(!loaded.iter().any(|g| g == "/assistants" || g == "/characters"), "{loaded:?}");
    }

    /// Listing assistants and characters, then chatting as one, against a scripted server.
    #[tokio::test]
    async fn personas_and_persona_chat_against_scripted_server() {
        let posted = std::sync::Arc::new(Mutex::new(Vec::new()));
        let base = scripted_ais(posted.clone()).await;
        let backend = crate::build(ais_config(base), Some("tok".into()), std::sync::Arc::new(|_, _| {})).unwrap();
        let list = backend.personas().await.unwrap();
        let names: Vec<_> = list.iter().map(|p| (p.kind, p.name.as_str(), p.own)).collect();
        assert_eq!(
            names,
            [(PersonaKind::Assistant, "Mathe-Coach", true), (PersonaKind::Assistant, "Schulassistent", false), (PersonaKind::Character, "Goethe", false)]
        );
        let goethe = list.iter().find(|p| p.id == "c1").unwrap();
        let msgs = vec![
            Message {
                id: "g".into(),
                conversation_id: "c".into(),
                role: Role::Assistant,
                content: "Seid gegrüßt!".into(),
                created_at: 0,
                model: None,
                attachments: vec![],
                activity: vec![],
                error: None,
                usage: None,
                tool_calls: vec![],
                tool_result: None,
            },
            Message {
                id: "u".into(),
                conversation_id: "c".into(),
                role: Role::User,
                content: "Wer bist du?".into(),
                created_at: 1,
                model: None,
                attachments: vec![],
                activity: vec![],
                error: None,
                usage: None,
                tool_calls: vec![],
                tool_result: None,
            },
        ];
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: "m".into(),
            system: None,
            messages: msgs,
            tools: vec![],
            tool_bridge: None,
            persona: Some(goethe.persona()),
            web: false,
        };
        let mut s = backend.chat(req).await.unwrap();
        let mut text = String::new();
        while let Some(e) = s.next().await {
            if let StreamEvent::Delta { text: t } = e.unwrap() {
                text.push_str(&t);
            }
        }
        assert_eq!(text, "Gern.");
        let last = posted.lock().unwrap().last().cloned().unwrap();
        assert!(last.contains(r#""characterId":"c1""#) && last.contains("Seid gegrüßt!"), "{last}");
    }

    /// Image models from the image page (current payload shape) and toggling a favourite.
    #[tokio::test]
    async fn image_models_and_favorites_against_scripted_server() {
        let posted = std::sync::Arc::new(Mutex::new(Vec::new()));
        let base = scripted_ais(posted.clone()).await;
        let backend = crate::build(ais_config(base), Some("tok".into()), std::sync::Arc::new(|_, _| {})).unwrap();
        let models = backend.image_models().await.unwrap();
        assert_eq!(models.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect::<Vec<_>>(), [("img-1", "GPT-Image-1.5")]);
        backend.set_favorite(PersonaKind::LearningScenario, "0a1b-2c".into(), true).await.unwrap();
        assert_eq!(posted.lock().unwrap().len(), 1);
        assert!(backend.set_favorite(PersonaKind::Assistant, "../x".into(), true).await.is_err());
        assert_eq!(posted.lock().unwrap().len(), 1, "bad ids are refused before asking the server");
    }

    #[tokio::test]
    async fn image_addresses_are_checked() {
        let ais =
            AisChat::new(ais_config("http://127.0.0.1:9".into()), "http://127.0.0.1:9".into(), Some("tok".into()), std::sync::Arc::new(|_, _| {})).unwrap();
        let (bytes, mime) = ais.fetch_image("data:image/webp;base64,AAEC").await.unwrap();
        assert_eq!((bytes, mime.as_str()), (vec![0, 1, 2], "image/webp"));
        assert!(ais.fetch_image("data:image/png;base64,***").await.is_err());
        assert!(ais.fetch_image("file:///etc/passwd").await.is_err(), "only http(s), paths and data URLs");
        assert!(ais.fetch_image("//evil.example/x.png").await.is_err());
    }
}
