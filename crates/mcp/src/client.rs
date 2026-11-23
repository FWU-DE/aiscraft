use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use aiscraft_core::{Error, ErrorKind, Result};
use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};

use crate::config::{ServerConfig, Transport};
use crate::types::{ServerInfo, Tool, ToolResult};
use crate::{MAX_MESSAGE_BYTES, PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS, http, stdio};

/// Pages of `tools/list` we follow before assuming the server loops.
const MAX_TOOL_PAGES: usize = 100;
const MAX_TOOLS: usize = 10_000;
/// Messages in one JSON-RPC batch we look at.
const MAX_BATCH: usize = 1000;

/// A poisoned lock only means another thread panicked while holding it; the data is still usable.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

type Reply = oneshot::Sender<Result<Value>>;

pub(crate) enum Conn {
    Stdio(stdio::Stdio),
    Http(http::Http),
}

/// Shared by the client and its I/O tasks (which only hold a `Weak`).
pub(crate) struct Inner {
    pub(crate) name: String,
    pub(crate) conn: Conn,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Reply>>,
    closed: Mutex<Option<Error>>,
    tools_changed: watch::Sender<u64>,
    tools_changed_flag: AtomicBool,
}

/// What happened to a message after the transport took it.
pub(crate) enum Sent {
    /// Written; the answer arrives later through [`Inner::dispatch`].
    Queued,
    /// Everything the server will say about it has been dispatched (HTTP).
    Done,
}

impl Inner {
    fn new(name: String, conn: Conn) -> Self {
        Self {
            name,
            conn,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            closed: Mutex::new(None),
            tools_changed: watch::Sender::new(0),
            tools_changed_flag: AtomicBool::new(false),
        }
    }

    pub(crate) fn closed_error(&self) -> Option<Error> {
        lock(&self.closed).clone()
    }

    pub(crate) fn is_pending(&self, id: u64) -> bool {
        lock(&self.pending).contains_key(&id)
    }

    /// The connection is gone: remember why and fail every waiting request with it.
    pub(crate) fn fail_all(&self, err: Error) {
        let err = {
            let mut closed = lock(&self.closed);
            closed.get_or_insert(err).clone()
        };
        let waiting: Vec<Reply> = lock(&self.pending).drain().map(|(_, r)| r).collect();
        for reply in waiting {
            let _ = reply.send(Err(err.clone()));
        }
    }

    async fn send(self: &Arc<Self>, msg: &Value, wait_for: Option<u64>) -> Result<Sent> {
        let bytes = serde_json::to_vec(msg)?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(Error::invalid("Die Nachricht an den Konnektor ist zu groß (mehr als 16 MiB)."));
        }
        match &self.conn {
            Conn::Stdio(s) => s.send(bytes).await.map(|()| Sent::Queued),
            Conn::Http(h) => {
                let is_init = msg.get("method").and_then(Value::as_str) == Some("initialize");
                h.send(self, bytes, is_init, wait_for).await.map(|()| Sent::Done)
            }
        }
    }

    /// Send without waiting (replies, notifications, cancellations). Failures are only logged.
    fn send_detached(self: &Arc<Self>, msg: Value) {
        match &self.conn {
            Conn::Stdio(s) => match serde_json::to_vec(&msg) {
                Ok(bytes) => s.try_send(bytes),
                Err(e) => log::warn!("MCP: Nachricht nicht serialisierbar: {e}"),
            },
            Conn::Http(_) => match tokio::runtime::Handle::try_current() {
                Ok(rt) => {
                    let inner = Arc::clone(self);
                    rt.spawn(async move {
                        if let Err(e) = inner.send(&msg, None).await {
                            log::debug!("MCP „{}“: Nachricht nicht zugestellt: {e}", inner.name);
                        }
                    });
                }
                Err(_) => log::debug!("MCP: keine Tokio-Laufzeit, Nachricht verworfen"),
            },
        }
    }

    /// Route one message (or batch) from the server.
    pub(crate) fn dispatch(self: &Arc<Self>, msg: Value) {
        match msg {
            Value::Array(items) => {
                for item in items.into_iter().take(MAX_BATCH) {
                    self.dispatch_one(item);
                }
            }
            other => self.dispatch_one(other),
        }
    }

    fn dispatch_one(self: &Arc<Self>, msg: Value) {
        let Value::Object(mut obj) = msg else {
            log::warn!("MCP „{}“: Nachricht ist kein JSON-Objekt, verworfen", self.name);
            return;
        };
        let method = obj.get("method").and_then(Value::as_str).map(str::to_string);
        let id = obj.remove("id").filter(|v| !v.is_null());
        match (method, id) {
            (Some(method), Some(id)) => self.answer_server_request(&method, id),
            (Some(method), None) => self.notification(&method, obj.get("params")),
            (None, Some(id)) => self.response(&id, obj),
            (None, None) => {
                let detail = obj.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).map(|m| preview(m, 200)).unwrap_or_default();
                log::warn!("MCP „{}“: Nachricht ohne id und Methode verworfen {detail}", self.name);
            }
        }
    }

    fn response(&self, id: &Value, mut obj: serde_json::Map<String, Value>) {
        let Some(id) = id.as_u64().or_else(|| id.as_str().and_then(|s| s.parse().ok())) else {
            log::warn!("MCP „{}“: Antwort mit unbekannter id verworfen", self.name);
            return;
        };
        let Some(reply) = lock(&self.pending).remove(&id) else {
            log::debug!("MCP „{}“: späte oder unbekannte Antwort {id} verworfen", self.name);
            return;
        };
        let outcome = if let Some(err) = obj.remove("error").filter(|e| !e.is_null()) {
            Err(rpc_error(&self.name, &err))
        } else if let Some(result) = obj.remove("result") {
            Ok(result)
        } else {
            Err(Error::protocol(format!("Der Konnektor „{}“ hat eine Antwort ohne Ergebnis geschickt.", self.name)))
        };
        let _ = reply.send(outcome);
    }

    fn notification(&self, method: &str, params: Option<&Value>) {
        match method {
            "notifications/tools/list_changed" => {
                self.tools_changed_flag.store(true, Ordering::SeqCst);
                self.tools_changed.send_modify(|n| *n = n.wrapping_add(1));
            }
            "notifications/message" => {
                let data = params.and_then(|p| p.get("data")).map(|d| d.as_str().map(str::to_string).unwrap_or_else(|| d.to_string())).unwrap_or_default();
                log::info!("MCP „{}“: {}", self.name, preview(&data, 1000));
            }
            other => log::debug!("MCP „{}“: Benachrichtigung {} ignoriert", self.name, preview(other, 100)),
        }
    }

    /// We declare no client capabilities, so apart from `ping` every server request gets
    /// "method not found" right away instead of leaving the server waiting.
    fn answer_server_request(self: &Arc<Self>, method: &str, id: Value) {
        let reply = if method == "ping" {
            json!({ "jsonrpc": "2.0", "id": id, "result": {} })
        } else {
            log::debug!("MCP „{}“: Anfrage {} des Servers nicht unterstützt", self.name, preview(method, 100));
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("Method not found: {}", preview(method, 100)) } })
        };
        self.send_detached(reply);
    }
}

/// Removes the pending entry when a request ends in any way. If it ended without an answer
/// (timeout, or the caller dropped the future), the server is told with `notifications/cancelled`.
struct PendingGuard<'a> {
    inner: &'a Arc<Inner>,
    id: u64,
    cancel: bool,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let unanswered = lock(&self.inner.pending).remove(&self.id).is_some();
        if unanswered && self.cancel && self.inner.closed_error().is_none() {
            self.inner.send_detached(json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": self.id, "reason": "Abgebrochen oder Zeitüberschreitung" }
            }));
        }
    }
}

async fn request(inner: &Arc<Inner>, method: &str, params: Value, timeout: Duration, cancellable: bool) -> Result<Value> {
    if let Some(e) = inner.closed_error() {
        return Err(e);
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let id = inner.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    lock(&inner.pending).insert(id, tx);
    let mut guard = PendingGuard { inner, id, cancel: cancellable };
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });

    let sent = match tokio::time::timeout_at(deadline, inner.send(&msg, Some(id))).await {
        Ok(Ok(sent)) => sent,
        Ok(Err(e)) => {
            guard.cancel = false;
            return Err(e);
        }
        Err(_) => return Err(timeout_error(inner, method, timeout)),
    };
    if let Sent::Done = sent {
        // Anything still pending now will never be answered; dropping the sender says so.
        lock(&inner.pending).remove(&id);
    }
    match tokio::time::timeout_at(deadline, rx).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) => {
            Err(inner.closed_error().unwrap_or_else(|| Error::protocol(format!("Der Konnektor „{}“ hat auf „{method}“ nicht geantwortet.", inner.name))))
        }
        Err(_) => Err(timeout_error(inner, method, timeout)),
    }
}

fn timeout_error(inner: &Inner, method: &str, timeout: Duration) -> Error {
    let secs = if timeout.as_millis() < 1000 { format!("{} ms", timeout.as_millis()) } else { format!("{} s", timeout.as_secs()) };
    let mut msg = format!("Der Konnektor „{}“ hat nicht innerhalb von {secs} auf „{method}“ geantwortet.", inner.name);
    if method == "initialize" {
        let tail = stderr_tail_of(inner, 600);
        if !tail.is_empty() {
            msg.push_str(&format!(" Letzte Ausgabe: {tail}"));
        }
    }
    Error::network(msg)
}

pub(crate) fn stderr_tail_of(inner: &Inner, max_chars: usize) -> String {
    match &inner.conn {
        Conn::Stdio(s) => s.stderr_tail(max_chars),
        Conn::Http(_) => String::new(),
    }
}

fn rpc_error(name: &str, err: &Value) -> Error {
    let code = err.get("code").and_then(Value::as_i64).unwrap_or(0);
    let message = err.get("message").and_then(Value::as_str).map(|m| preview(m, 500)).unwrap_or_default();
    let kind = match code {
        -32601 => ErrorKind::Unsupported,
        -32602 => ErrorKind::Invalid,
        -32002 => ErrorKind::NotFound,
        _ => ErrorKind::Protocol,
    };
    Error::new(kind, format!("Der Konnektor „{name}“ meldet einen Fehler ({code}): {message}"))
}

/// At most `max` characters of untrusted text, for logs and messages.
pub(crate) fn preview(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).filter(|c| !c.is_control() || *c == '\n').collect();
    if s.chars().nth(max).is_some() {
        out.push('…');
    }
    out
}

/// A connection to one MCP server. `Send + Sync`; share it behind an `Arc`.
///
/// Dropping the client ends the connection (a stdio server process is killed).
pub struct McpClient {
    inner: Arc<Inner>,
    id: String,
    info: ServerInfo,
    instructions: Option<String>,
    timeout: Duration,
}

impl fmt::Debug for McpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpClient").field("id", &self.id).field("name", &self.inner.name).field("server", &self.info.name).finish()
    }
}

impl McpClient {
    /// Start or reach the server and run the MCP handshake (`initialize`, then
    /// `notifications/initialized`).
    pub async fn connect(config: &ServerConfig) -> Result<Self> {
        let name = if config.name.trim().is_empty() { config.id.clone() } else { config.name.clone() };
        if !config.enabled {
            return Err(Error::invalid(format!("Der Konnektor „{name}“ ist ausgeschaltet.")));
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return Err(Error::unsupported("Der MCP-Client braucht eine Tokio-Laufzeit."));
        };
        let timeout = config.request_timeout();
        let inner = match &config.transport {
            Transport::Stdio { command, args, env, cwd } => {
                let (conn, pipes) = stdio::spawn(command, args, env, cwd.as_deref())?;
                let inner = Arc::new(Inner::new(name, Conn::Stdio(conn)));
                stdio::start(&rt, &inner, pipes);
                inner
            }
            Transport::Http { url, headers } => Arc::new(Inner::new(name, Conn::Http(http::Http::new(url, headers)?))),
        };
        match Self::handshake(&inner, timeout).await {
            Ok((info, instructions)) => {
                if let Conn::Http(h) = &inner.conn
                    && info.tools_list_changed()
                {
                    h.start_listener(&rt, &inner);
                }
                Ok(Self { inner, id: config.id.clone(), info, instructions, timeout })
            }
            Err(e) => {
                close_inner(&inner).await;
                Err(e)
            }
        }
    }

    async fn handshake(inner: &Arc<Inner>, timeout: Duration) -> Result<(ServerInfo, Option<String>)> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "AISCraft", "version": env!("CARGO_PKG_VERSION") }
        });
        // The spec forbids cancelling `initialize`.
        let result = request(inner, "initialize", params, timeout, false).await?;
        if !result.is_object() {
            return Err(Error::protocol(format!("Der Konnektor „{}“ hat auf „initialize“ keine gültige Antwort geschickt.", inner.name)));
        }
        let info = ServerInfo::parse(&result);
        if info.protocol_version.is_empty() {
            return Err(Error::protocol(format!("Der Konnektor „{}“ hat keine Protokollversion genannt.", inner.name)));
        }
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&info.protocol_version.as_str()) {
            log::warn!("MCP „{}“: unbekannte Protokollversion {}, versuche es trotzdem", inner.name, preview(&info.protocol_version, 40));
        }
        if let Conn::Http(h) = &inner.conn {
            h.set_protocol_version(&info.protocol_version);
        }
        let instructions = result.get("instructions").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(str::to_string);
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        tokio::time::timeout(timeout, inner.send(&note, None))
            .await
            .map_err(|_| Error::network(format!("Der Konnektor „{}“ reagiert nicht.", inner.name)))??;
        Ok((info, instructions))
    }

    /// The connector id from the config.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The connector name from the config.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// The server's usage hints for the model, if it sent any. Untrusted text.
    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn request_timeout(&self) -> Duration {
        self.timeout
    }

    /// All tools, following `nextCursor` across pages.
    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_TOOL_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = request(&self.inner, "tools/list", params, self.timeout, true).await?;
            let Some(items) = result.get("tools").and_then(Value::as_array) else {
                return Err(Error::protocol(format!("Der Konnektor „{}“ hat keine Werkzeugliste geschickt.", self.inner.name)));
            };
            for item in items {
                match Tool::parse(item) {
                    Some(t) if tools.len() < MAX_TOOLS => tools.push(t),
                    Some(_) => {}
                    None => log::warn!("MCP „{}“: Werkzeug ohne Namen übersprungen", self.inner.name),
                }
            }
            cursor = result.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_string);
            match &cursor {
                None => return Ok(tools),
                Some(c) if !seen.insert(c.clone()) => {
                    log::warn!("MCP „{}“: Seitenzeiger wiederholt sich, Werkzeugliste abgeschnitten", self.inner.name);
                    return Ok(tools);
                }
                Some(_) => {}
            }
        }
        log::warn!("MCP „{}“: mehr als {MAX_TOOL_PAGES} Seiten Werkzeuge, Liste abgeschnitten", self.inner.name);
        Ok(tools)
    }

    /// Call a tool. `args` must be a JSON object (or null for none). A tool that ran and
    /// failed is `Ok` with [`ToolResult::is_error`]; `Err` means the call itself failed.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult> {
        self.call_tool_with_timeout(name, args, self.timeout).await
    }

    /// Like [`Self::call_tool`] with its own timeout. Dropping the future cancels the call.
    pub async fn call_tool_with_timeout(&self, name: &str, args: Value, timeout: Duration) -> Result<ToolResult> {
        let args = match args {
            Value::Null => json!({}),
            v @ Value::Object(_) => v,
            _ => return Err(Error::invalid(format!("Die Argumente für „{}“ müssen ein JSON-Objekt sein.", preview(name, 100)))),
        };
        let result = request(&self.inner, "tools/call", json!({ "name": name, "arguments": args }), timeout, true).await?;
        if !result.is_object() {
            return Err(Error::protocol(format!("Der Konnektor „{}“ hat ein ungültiges Werkzeug-Ergebnis geschickt.", self.inner.name)));
        }
        Ok(ToolResult::parse(&result))
    }

    pub async fn ping(&self) -> Result<()> {
        request(&self.inner, "ping", json!({}), self.timeout, true).await.map(|_| ())
    }

    /// End the connection: stdio servers get their stdin closed and are killed if they do
    /// not exit; HTTP sessions are deleted. Later requests fail.
    pub async fn close(&self) {
        close_inner(&self.inner).await;
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed_error().is_some()
    }

    /// Counts `notifications/tools/list_changed`; wait on `changed()` to refresh the tools.
    pub fn tools_changed(&self) -> watch::Receiver<u64> {
        self.inner.tools_changed.subscribe()
    }

    /// True once after the server announced a changed tool list.
    pub fn take_tools_changed(&self) -> bool {
        self.inner.tools_changed_flag.swap(false, Ordering::SeqCst)
    }

    /// Recent stderr lines of a stdio server, for diagnostics (empty for HTTP).
    pub fn stderr_tail(&self) -> String {
        stderr_tail_of(&self.inner, 64 * 1024)
    }
}

async fn close_inner(inner: &Arc<Inner>) {
    inner.fail_all(Error::new(ErrorKind::Cancelled, format!("Die Verbindung zum Konnektor „{}“ wurde geschlossen.", inner.name)));
    match &inner.conn {
        Conn::Stdio(s) => s.close().await,
        Conn::Http(h) => h.close().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<McpClient>();
        check::<Arc<McpClient>>();
    }

    #[test]
    fn preview_cuts_and_strips_control_chars() {
        assert_eq!(preview("ab\u{1b}[31mcd", 3), "ab…");
        assert_eq!(preview("äöü", 5), "äöü");
    }
}
