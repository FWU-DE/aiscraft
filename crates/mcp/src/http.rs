//! Streamable HTTP transport (MCP 2025-06-18): every message is a POST; the answer comes back
//! as a JSON body or as an SSE stream. An optional GET stream carries server notifications.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde_json::Value;
use tokio::sync::watch;

use crate::MAX_MESSAGE_BYTES;
use crate::client::{Conn, Inner, lock, preview};
use crate::sse::SseParser;

const USER_AGENT: &str = concat!("AIS.ChatCraft/", env!("CARGO_PKG_VERSION"));
const SESSION_HEADER: &str = "mcp-session-id";
const VERSION_HEADER: &str = "mcp-protocol-version";

pub(crate) struct Http {
    client: reqwest::Client,
    url: reqwest::Url,
    headers: HeaderMap,
    session: Mutex<Option<HeaderValue>>,
    protocol: Mutex<Option<HeaderValue>>,
    /// Dropped with the client (or set on close): ends the GET listener.
    shutdown: watch::Sender<bool>,
}

impl Http {
    pub(crate) fn new(url: &str, headers: &BTreeMap<String, String>) -> Result<Self> {
        let parsed = reqwest::Url::parse(url.trim()).map_err(|_| Error::invalid(format!("„{}“ ist keine gültige Adresse.", preview(url, 200))))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::invalid(format!("Die Adresse des Konnektors muss mit https:// beginnen: {}", preview(url, 200))));
        }
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            let name =
                HeaderName::from_bytes(k.trim().as_bytes()).map_err(|_| Error::invalid(format!("„{}“ ist kein gültiger Kopfzeilen-Name.", preview(k, 100))))?;
            let mut value =
                HeaderValue::from_str(v.trim()).map_err(|_| Error::invalid(format!("Der Wert der Kopfzeile „{name}“ enthält ungültige Zeichen.")))?;
            value.set_sensitive(true);
            map.insert(name, value);
        }
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| Error::network(format!("HTTP-Client: {e}")))?;
        Ok(Self { client, url: parsed, headers: map, session: Mutex::new(None), protocol: Mutex::new(None), shutdown: watch::Sender::new(false) })
    }

    pub(crate) fn set_protocol_version(&self, version: &str) {
        *lock(&self.protocol) = HeaderValue::from_str(version).ok();
    }

    fn request(&self, method: Method) -> RequestBuilder {
        let mut rb = self.client.request(method, self.url.clone()).headers(self.headers.clone());
        if let Some(s) = lock(&self.session).clone() {
            rb = rb.header(SESSION_HEADER, s);
        }
        if let Some(p) = lock(&self.protocol).clone() {
            rb = rb.header(VERSION_HEADER, p);
        }
        rb
    }

    /// POST one message and dispatch whatever comes back. Returns when the answer to
    /// `wait_for` has been dispatched or the response is over.
    pub(crate) async fn send(&self, inner: &Arc<Inner>, body: Vec<u8>, is_init: bool, wait_for: Option<u64>) -> Result<()> {
        let resp = self
            .request(Method::POST)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .body(body)
            .send()
            .await
            .map_err(|e| net_err(&inner.name, &e))?;
        if let Some(sid) = resp.headers().get(SESSION_HEADER)
            && is_init
        {
            // Visible ASCII only (spec); HeaderValue already rules out control characters.
            if sid.to_str().is_ok_and(|s| !s.is_empty() && s.len() <= 1024 && s.bytes().all(|b| (0x21..=0x7e).contains(&b))) {
                *lock(&self.session) = Some(sid.clone());
            } else {
                log::warn!("MCP „{}“: ungültige Sitzungskennung ignoriert", inner.name);
            }
        }
        let status = resp.status();
        if !status.is_success() {
            return Err(self.status_error(&inner.name, resp, is_init).await);
        }
        if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
            return Ok(());
        }
        let content_type = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if content_type.starts_with("text/event-stream") {
            read_sse(inner, resp, wait_for).await
        } else if content_type.starts_with("application/json") || content_type.is_empty() {
            let text = read_capped(&inner.name, resp, MAX_MESSAGE_BYTES).await?;
            if text.trim().is_empty() {
                return Ok(());
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(msg) => {
                    inner.dispatch(msg);
                    Ok(())
                }
                Err(_) => Err(Error::protocol(format!("Der Konnektor „{}“ hat keine gültige JSON-Antwort geschickt.", inner.name))),
            }
        } else {
            Err(Error::protocol(format!("Der Konnektor „{}“ hat mit unerwartetem Inhaltstyp „{}“ geantwortet.", inner.name, preview(&content_type, 80))))
        }
    }

    async fn status_error(&self, name: &str, resp: Response, is_init: bool) -> Error {
        let status = resp.status();
        let body = read_capped(name, resp, 16 * 1024).await.unwrap_or_default();
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).or_else(|| v.get("message")).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| body.trim().to_string());
        let detail = if detail.is_empty() { String::new() } else { format!(" Server: {}", preview(&detail, 300)) };
        match status {
            StatusCode::UNAUTHORIZED => Error::auth(format!(
                "Der Konnektor „{name}“ verlangt eine Anmeldung (HTTP 401). Die Anmeldung per OAuth wird noch nicht unterstützt; \
                 hinterlege ein Zugangstoken als Kopfzeile „Authorization: Bearer …“ oder prüfe das vorhandene Token."
            )),
            StatusCode::FORBIDDEN => {
                Error::auth(format!("Der Konnektor „{name}“ verweigert den Zugriff (HTTP 403). Prüfe das Zugangstoken und seine Rechte.{detail}"))
            }
            StatusCode::NOT_FOUND if lock(&self.session).is_some() && !is_init => {
                Error::not_found(format!("Die Sitzung beim Konnektor „{name}“ ist abgelaufen. Bitte die Verbindung neu aufbauen."))
            }
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED if is_init => Error::unsupported(format!(
                "Unter dieser Adresse antwortet kein MCP-Server mit Streamable HTTP (HTTP {}). Prüfe die Adresse; \
                 ältere Server, die nur HTTP+SSE sprechen (Adresse endet oft auf /sse), werden nicht unterstützt.",
                status.as_u16()
            )),
            StatusCode::TOO_MANY_REQUESTS => {
                Error::refused(format!("Der Konnektor „{name}“ ist überlastet (HTTP 429). Bitte später erneut versuchen.{detail}"))
            }
            s if s.is_client_error() => Error::invalid(format!("Der Konnektor „{name}“ hat die Anfrage abgelehnt (HTTP {}).{detail}", s.as_u16())),
            s => Error::network(format!("Der Konnektor „{name}“ hat einen Fehler gemeldet (HTTP {}).{detail}", s.as_u16())),
        }
    }

    /// Listen on the GET stream for server-initiated messages (e.g. `tools/list_changed`).
    pub(crate) fn start_listener(&self, rt: &tokio::runtime::Handle, inner: &Arc<Inner>) {
        rt.spawn(listen(Arc::downgrade(inner), self.shutdown.subscribe()));
    }

    pub(crate) async fn close(&self) {
        let _ = self.shutdown.send(true);
        if lock(&self.session).is_some() {
            let _ = self.request(Method::DELETE).timeout(Duration::from_secs(5)).send().await;
        }
    }
}

async fn read_sse(inner: &Arc<Inner>, resp: Response, wait_for: Option<u64>) -> Result<()> {
    let mut body = resp.bytes_stream();
    let mut parser = SseParser::new(MAX_MESSAGE_BYTES);
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| net_err(&inner.name, &e))?;
        for data in parser.feed(&chunk)? {
            dispatch_text(inner, &data);
        }
        if wait_for.is_some_and(|id| !inner.is_pending(id)) {
            return Ok(());
        }
    }
    if let Some(data) = parser.finish() {
        dispatch_text(inner, &data);
    }
    Ok(())
}

fn dispatch_text(inner: &Arc<Inner>, data: &str) {
    if data.trim().is_empty() {
        return;
    }
    match serde_json::from_str::<Value>(data) {
        Ok(msg) => inner.dispatch(msg),
        Err(_) => log::warn!("MCP „{}“: Ereignis ist kein JSON: {}", inner.name, preview(data, 200)),
    }
}

async fn listen(weak: Weak<Inner>, mut shutdown: watch::Receiver<bool>) {
    let mut failures = 0u32;
    while failures < 5 {
        let req = {
            let Some(inner) = weak.upgrade() else { return };
            let Conn::Http(h) = &inner.conn else { return };
            if inner.closed_error().is_some() {
                return;
            }
            h.request(Method::GET).header(ACCEPT, "text/event-stream")
        };
        let resp = tokio::select! {
            r = req.send() => r,
            _ = shutdown.changed() => return,
        };
        match resp {
            Ok(r) if r.status().is_success() => {
                let mut body = r.bytes_stream();
                let mut parser = SseParser::new(MAX_MESSAGE_BYTES);
                let mut got_any = false;
                loop {
                    let chunk = tokio::select! {
                        c = body.next() => c,
                        _ = shutdown.changed() => return,
                    };
                    let Some(Ok(chunk)) = chunk else { break };
                    let Ok(events) = parser.feed(&chunk) else { break };
                    let Some(inner) = weak.upgrade() else { return };
                    for data in events {
                        got_any = true;
                        dispatch_text(&inner, &data);
                    }
                }
                failures = if got_any { 0 } else { failures + 1 };
            }
            // 405: the server offers no GET stream, which is allowed.
            Ok(r) => {
                log::debug!("MCP: kein Ereignis-Stream (HTTP {})", r.status());
                return;
            }
            Err(_) => failures += 1,
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(u64::from(failures).max(1) * 2)) => {}
            _ = shutdown.changed() => return,
        }
    }
}

fn net_err(name: &str, e: &reqwest::Error) -> Error {
    if e.is_timeout() {
        Error::network(format!("Zeitüberschreitung beim Konnektor „{name}“."))
    } else if e.is_connect() {
        Error::network(format!("Der Konnektor „{name}“ ist nicht erreichbar."))
    } else {
        Error::network(format!("Verbindung zum Konnektor „{name}“ unterbrochen: {e}"))
    }
}

/// Read a body, refusing more than `cap` bytes.
async fn read_capped(name: &str, resp: Response, cap: usize) -> Result<String> {
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| net_err(name, &e))?;
        if out.len() + chunk.len() > cap {
            return Err(Error::protocol(format!("Der Konnektor „{name}“ hat eine zu große Antwort geschickt (mehr als {} MiB).", cap / (1024 * 1024))));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}
