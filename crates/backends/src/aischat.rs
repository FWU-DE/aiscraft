//! AIS.chat through the person's signed-in web session.
//!
//! AIS.chat (FWU-DE/ais-chat) is a Next.js app with Auth.js sessions in an encrypted cookie. A
//! native client that sends that cookie is treated like the browser. Plain JSON routes cover the
//! conversation list, uploads and the session; sending a message is the server action
//! `sendChatMessageAction`, whose id changes with each deploy and is read from the shipped JS
//! chunks. The answer is a React Flight response carrying a `ReadableStream<string>` of text
//! deltas and `\x1e`-prefixed activity events.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use aiscraft_core::{
    AccountInfo, ActivityLink, ActivityStep, BackendConfig, Capabilities, ChatRequest, Error, GeneratedDocument, ModelInfo, RemoteConversation, Result,
    Role, StreamEvent,
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
    action_id: tokio::sync::Mutex<Option<String>>,
}

impl AisChat {
    pub fn new(config: BackendConfig, base: String, secret: Option<String>, on_secret: SecretSink) -> Result<Self> {
        let jar = secret.as_deref().and_then(normalize_session_cookie).map(|c| parse_cookie_pairs(&c)).unwrap_or_default();
        Ok(Self { config, base, http: client(false)?, jar: Mutex::new(jar), on_secret, action_id: tokio::sync::Mutex::new(None) })
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

    async fn page(&self) -> Result<String> {
        let resp = self.send(self.request(Method::GET, "/")?.header("Accept", "text/html")).await?;
        if !resp.status().is_success() {
            return Err(status_error(resp, "AIS.chat-Startseite").await);
        }
        read_capped(resp, PAGE_CAP).await
    }

    async fn discover_action(&self) -> Result<String> {
        let html = self.page().await?;
        let paths = scrape::chunk_paths(&html);
        let fetched: Vec<Option<String>> = stream::iter(paths.into_iter().take(MAX_CHUNKS))
            .map(|p| async move {
                let resp = self.http.get(format!("{}{p}", self.base)).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                read_capped(resp, PAGE_CAP).await.ok()
            })
            .buffer_unordered(8)
            .collect()
            .await;
        for js in fetched.into_iter().flatten() {
            if let Some(id) = scrape::action_ids(&js).remove(SEND_ACTION) {
                return Ok(id);
            }
        }
        Err(Error::protocol("Die Chat-Funktion von AIS.chat wurde in der Web-App nicht gefunden. Vermutlich hat sich AIS.chat geändert."))
    }

    async fn action_id(&self, refresh: bool) -> Result<String> {
        let mut guard = self.action_id.lock().await;
        if !refresh && let Some(id) = guard.as_ref() {
            return Ok(id.clone());
        }
        let id = self.discover_action().await?;
        *guard = Some(id.clone());
        Ok(id)
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
}

/// The action's single argument.
pub fn action_payload(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .filter(|m| m.role == Role::User || (m.role == Role::Assistant && !m.content.is_empty()))
        .map(|m| json!({"id": m.id, "role": m.role.as_str(), "content": m.content}))
        .collect();
    let file_ids: Vec<&str> =
        req.messages.iter().rev().find(|m| m.role == Role::User).map(|m| m.attachments.iter().filter_map(|a| a.remote_id.as_deref()).collect()).unwrap_or_default();
    let mut p = json!({"conversationId": req.conversation_id, "messages": messages, "modelId": req.model});
    if !file_ids.is_empty()
        && let Some(o) = p.as_object_mut()
    {
        o.insert("fileIds".into(), json!(file_ids));
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

fn tool_label(tool: &str) -> &'static str {
    let t = tool.to_ascii_lowercase();
    if t.contains("websearch") || t.contains("web_search") {
        "Websuche"
    } else if t.contains("scrap") {
        "Webseite lesen"
    } else if t.contains("chunk") || t.contains("file") {
        "Datei durchsuchen"
    } else if t.contains("mundo") {
        "Suche in Mundo"
    } else if t.contains("math") || t.contains("calc") {
        "Rechnen"
    } else if t.contains("plot") || t.contains("graph") {
        "Diagramm"
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
                        .map(|a| a.iter().filter_map(|l| Some(ActivityLink { title: s(l, "title")?, url: s(l, "url")? })).collect())
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
        Capabilities { remote_history: true, uploads: true, system_prompt: false, vision: true, tools: false, tools_via_mcp: false }
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
            Ok(AccountInfo { display_name: name, detail: (!detail.is_empty()).then_some(detail), expires: v.get("expires").and_then(Value::as_str).map(str::to_string) })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let payload = scrape::rsc_payload(&self.page().await?);
            let initial = scrape::initial_model_name(&payload);
            let mut models: Vec<ModelInfo> = scrape::page_models(&payload)
                .into_iter()
                .filter(|m| m.kind == "text")
                .map(|m| ModelInfo { id: m.id, name: m.display_name, description: (!m.description.is_empty()).then_some(m.description), vision: m.vision, context_window: None })
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
            let body = encode_action_args(&[action_payload(&req)]);
            let mut id = self.action_id(false).await?;
            let mut resp = self.post_action(&id, body.clone()).await?;
            let stale = resp.status() == StatusCode::NOT_FOUND || resp.headers().contains_key("x-nextjs-action-not-found");
            if stale {
                id = self.action_id(true).await?;
                resp = self.post_action(&id, body).await?;
            }
            if !resp.status().is_success() {
                return Err(status_error(resp, "AIS.chat").await);
            }
            let ctype = resp.headers().get("content-type").and_then(|c| c.to_str().ok()).unwrap_or("").to_string();
            if !ctype.contains("text/x-component") {
                return Err(Error::protocol(format!("AIS.chat hat nicht als Chat geantwortet (Inhaltstyp {ctype:?}).")));
            }
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
                            Some(RemoteConversation {
                                id: s("id")?,
                                title: s("name").or_else(|| s("title")).unwrap_or_else(|| "Ohne Titel".into()),
                                updated_at: s("updatedAt").or_else(|| s("createdAt")),
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

    fn web_url(&self, conversation_id: &str) -> Option<String> {
        Some(format!("{}/d/{conversation_id}", self.base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiscraft_core::{Attachment, Message};

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
            usage: None, tool_calls: vec![], tool_result: None,
        };
        let a = Message { id: "a0".into(), role: Role::Assistant, content: String::new(), attachments: vec![], ..u.clone() };
        u.id = "u1".into();
        let req = ChatRequest { conversation_id: "c1".into(), model: "uuid-m".into(), system: Some("ignored".into()), messages: vec![u, a], tools: vec![], tool_bridge: None };
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
        let err = map_action_event(ActionEvent::Result(json!({"stream":"$2","messageId":"m","error":{"name":"TokenPointsExceededError","message":"x"}}))).unwrap_err();
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
                        ("application/javascript", String::new(), format!(r#"(0,a.createServerReference)("{action}",a.callServer,void 0,a.findSourceMapURL,"sendChatMessageAction")"#))
                    } else if req.starts_with("POST / ") && req.contains(&format!("next-action: {action}")) && req.contains("cookie: __Secure-ais-chat-app") {
                        (
                            "text/x-component",
                            format!("set-cookie: {SESSION_COOKIE}=renewed; Path=/; Secure\r\n"),
                            "0:{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}\n1:{\"stream\":\"$2\",\"messageId\":\"srv-1\"}\n2:R\n2:T6,Hallo 2:\"Welt\"\n2:C\n".to_string(),
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
            usage: None, tool_calls: vec![], tool_result: None,
        };
        let mut s = backend.chat(ChatRequest { conversation_id: "c".into(), model: "m".into(), system: None, messages: vec![msg], tools: vec![], tool_bridge: None }).await.unwrap();
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
}
