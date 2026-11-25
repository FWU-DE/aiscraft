//! MCP Apps (#74, SEP-1865): a connector tool can come with a small interactive view, an HTML
//! resource at a `ui://` address. Under such a tool's result the chat shows a card „Ansicht
//! öffnen“; the view opens only in the separate `aiscraft-preview` process (never a web engine
//! in the main app), with a Content-Security-Policy that allows no network except the domains
//! the resource declares (none by default). View and app talk JSON-RPC over the preview's
//! stdin/stdout: `ui/initialize` with the look's colours, tool calls that go through the same
//! approval rules as the model's (`tools.rs`), the tool's input and result.
//!
//! Everything from the connector is hostile: the resource is checked for type and size, its
//! declared domains for shape (no `*`, no `http:`, no schemes or keywords), its permission
//! requests are not granted. Without the preview the app shows the result's text and a note.

use std::collections::BTreeMap;

use aiscraft_core::{Error, Result, ToolView};
use base64::Engine as _;
use serde_json::{Value, json};

use crate::commands::req_str;
use crate::{ConnectorTool, ConnectorView, Ctx, NoticeAction, NoticeLevel};

/// The resource type an MCP App view has.
pub(crate) const MIME: &str = "text/html;profile=mcp-app";
/// Largest view loaded.
pub(crate) const MAX_VIEW_BYTES: usize = 2 * 1024 * 1024;
/// Most domains a view may declare per kind.
const MAX_DOMAINS: usize = 20;
/// Longest message a view may send.
pub(crate) const MAX_VIEW_MESSAGE: usize = 1024 * 1024;
/// The protocol version of the app bridge.
const BRIDGE_VERSION: &str = "2026-01-26";

/// A view, loaded and checked.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CheckedView {
    pub html: String,
    pub connect: Vec<String>,
    pub resources: Vec<String>,
    pub csp: String,
    /// Permissions the view asked for (camera, microphone …); never granted.
    pub refused_permissions: Vec<String>,
}

/// A declared domain: `https://host[:port]`, `https://*.host` (and `wss://` for connections).
/// Never `*`, `http:`, other schemes, paths, quotes or keywords.
pub(crate) fn domain_ok(d: &str, connect: bool) -> bool {
    let rest = match d.split_once("://") {
        Some(("https", r)) => r,
        Some(("wss", r)) if connect => r,
        _ => return false,
    };
    let host = rest.strip_prefix("*.").unwrap_or(rest);
    let (host, port) = match host.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host, None),
    };
    let port_ok = port.is_none_or(|p| !p.is_empty() && p.len() <= 5 && p.chars().all(|c| c.is_ascii_digit()));
    let labels: Vec<&str> = host.split('.').collect();
    port_ok
        && d.len() <= 253
        && labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.len() <= 63 && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !l.starts_with('-') && !l.ends_with('-'))
}

/// The policy the preview enforces: inline code only, no network but the declared domains.
pub(crate) fn csp(connect: &[String], resources: &[String]) -> String {
    let res = if resources.is_empty() { String::new() } else { format!(" {}", resources.join(" ")) };
    let conn = if connect.is_empty() { "'none'".to_string() } else { connect.join(" ") };
    format!(
        "default-src 'none'; script-src 'unsafe-inline'{res}; style-src 'unsafe-inline'{res}; img-src data: blob:{res}; media-src data: blob:{res}; font-src data:{res}; connect-src {conn}; frame-src 'none'; object-src 'none'; form-action 'none'; base-uri 'none'"
    )
}

/// Check a `resources/read` result for the view at `uri`.
pub(crate) fn check(uri: &str, read: &aiscraft_mcp::ResourceContents) -> Result<CheckedView> {
    let mime = read.mime_type.as_deref().unwrap_or("").to_ascii_lowercase().replace(' ', "");
    if mime != MIME {
        return Err(Error::refused(format!("Die Ansicht hat den falschen Typ ({}); erlaubt ist nur {MIME}", if mime.is_empty() { "keiner" } else { &mime })));
    }
    let bytes = match (&read.text, &read.blob) {
        (Some(t), _) => {
            if t.len() > MAX_VIEW_BYTES {
                return Err(too_large());
            }
            t.clone().into_bytes()
        }
        (None, Some(b)) => {
            if b.len() > MAX_VIEW_BYTES * 4 / 3 + 4 {
                return Err(too_large());
            }
            base64::engine::general_purpose::STANDARD.decode(b.trim()).map_err(|_| Error::refused("Die Ansicht ist nicht lesbar (base64)"))?
        }
        _ => return Err(Error::refused("Die Ansicht ist leer")),
    };
    if bytes.len() > MAX_VIEW_BYTES {
        return Err(too_large());
    }
    let html = String::from_utf8(bytes).map_err(|_| Error::refused("Die Ansicht ist kein UTF-8-Text"))?;
    let meta = read.meta.get("ui").cloned().unwrap_or(Value::Null);
    let list = |key: &str, connect: bool| -> Result<Vec<String>> {
        let Some(v) = meta.pointer(&format!("/csp/{key}")) else { return Ok(Vec::new()) };
        let arr = v.as_array().ok_or_else(|| Error::refused(format!("Die Ansicht gibt {key} nicht als Liste an")))?;
        if arr.len() > MAX_DOMAINS {
            return Err(Error::refused(format!("Die Ansicht verlangt zu viele Netzfreigaben ({key})")));
        }
        arr.iter()
            .map(|d| {
                let d = d.as_str().unwrap_or("");
                if domain_ok(d, connect) {
                    Ok(d.to_string())
                } else {
                    Err(Error::refused(format!(
                        "Die Ansicht verlangt eine unzulässige Netzfreigabe: {}",
                        d.chars().filter(|c| !c.is_control()).take(80).collect::<String>()
                    )))
                }
            })
            .collect()
    };
    let connect = list("connectDomains", true)?;
    let resources = list("resourceDomains", false)?;
    for key in ["frameDomains", "baseUriDomains"] {
        if meta.pointer(&format!("/csp/{key}")).and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
            return Err(Error::refused(format!("Die Ansicht verlangt eingebettete fremde Seiten ({key}); das erlaubt AIS.ChatCraft nicht")));
        }
    }
    let refused_permissions = meta.get("permissions").and_then(Value::as_object).map(|o| o.keys().take(10).cloned().collect()).unwrap_or_default();
    let _ = uri;
    Ok(CheckedView { csp: csp(&connect, &resources), html, connect, resources, refused_permissions })
}

fn too_large() -> Error {
    Error::refused(format!("Die Ansicht ist größer als {} MB", MAX_VIEW_BYTES / (1024 * 1024)))
}

/// The view of a tool message, and the connector and tool it belongs to.
fn view_of_message(c: &Ctx, message: &str) -> Result<(String, ToolView, ConnectorView, ConnectorTool)> {
    let s = c.state();
    let m = s.messages.iter().find(|m| m.id == message).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    let view = m.tool_result.as_ref().and_then(|r| r.view.clone()).ok_or_else(|| Error::not_found("Diese Nachricht hat keine Ansicht"))?;
    let conn =
        s.connectors.iter().find(|k| k.id == view.connector).cloned().ok_or_else(|| Error::not_found("Der Konnektor dieser Ansicht ist nicht mehr da"))?;
    let tool = conn.tools.iter().find(|t| t.name == view.tool).cloned().ok_or_else(|| Error::not_found("Das Werkzeug dieser Ansicht gibt es nicht mehr"))?;
    Ok((m.conversation_id.clone(), view, conn, tool))
}

async fn load(c: &Ctx, message: &str) -> Result<(String, ToolView, ConnectorView, ConnectorTool, CheckedView)> {
    let (conv, view, conn, tool) = view_of_message(c, message)?;
    let client = crate::tools::client(c, &conn.id).await?;
    let read = client.read_resource(&view.resource).await?;
    let checked = check(&view.resource, &read)?;
    Ok((conv, view, conn, tool, checked))
}

/// `view.check`: load the view of a tool result and check it; opens nothing.
pub(crate) async fn check_cmd(c: Ctx, p: Value) -> Result<Value> {
    let message = req_str(&p, "message")?;
    let (_, view, _, _, checked) = load(&c, &message).await?;
    Ok(json!({
        "resource": view.resource,
        "bytes": checked.html.len(),
        "csp": checked.csp,
        "connect": checked.connect,
        "resources": checked.resources,
        "refused_permissions": checked.refused_permissions,
    }))
}

/// Look colours the UI passes on to the view as CSS variables; anything else is dropped.
pub(crate) fn styles(v: Option<&Value>) -> BTreeMap<String, String> {
    let Some(o) = v.and_then(Value::as_object) else { return BTreeMap::new() };
    o.iter()
        .take(60)
        .filter_map(|(k, v)| {
            let v = v.as_str()?;
            let key_ok = k.starts_with("--") && k.len() <= 64 && k.chars().skip(2).all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            let val_ok = v.len() <= 120 && v.chars().all(|c| c.is_ascii_alphanumeric() || " #(),.%-'\"".contains(c));
            (key_ok && val_ok).then(|| (k.clone(), v.to_string()))
        })
        .collect()
}

/// One open view: what it may call and what it shows.
#[derive(Debug, Clone)]
pub(crate) struct Session {
    pub conversation: String,
    pub connector: String,
    pub tool: ConnectorTool,
    pub arguments: Value,
    /// The result shown first (`ui/notifications/tool-result`).
    pub result: Value,
    pub theme: String,
    pub styles: BTreeMap<String, String>,
}

async fn session_for(c: &Ctx, p: &Value) -> Result<(Session, CheckedView, String)> {
    let message = req_str(p, "message")?;
    let (conv, view, conn, tool, checked) = load(c, &message).await?;
    let s = c.state();
    let call_id = s.messages.iter().find(|m| m.id == message).and_then(|m| m.tool_result.as_ref()).map(|r| r.call_id.clone()).unwrap_or_default();
    let arguments = s.messages.iter().flat_map(|m| m.tool_calls.iter()).find(|tc| tc.id == call_id).map(|tc| tc.arguments.clone()).unwrap_or(json!({}));
    let text = s.messages.iter().find(|m| m.id == message).map(|m| crate::tools::unwrap_untrusted(&m.content)).unwrap_or_default();
    let mut result = json!({"content": [{"type": "text", "text": text}]});
    if let (Some(sc), Some(o)) = (view.structured.clone(), result.as_object_mut()) {
        o.insert("structuredContent".into(), sc);
    }
    let theme = match p.get("theme").and_then(Value::as_str) {
        Some("dark") => "dark",
        _ => "light",
    };
    let title = if tool.title.is_empty() { tool.name.clone() } else { tool.title.clone() };
    Ok((
        Session { conversation: conv, connector: conn.id.clone(), tool, arguments, result, theme: theme.into(), styles: styles(p.get("styles")) },
        checked,
        format!("{} · {title}", conn.name),
    ))
}

/// `view.open`: open the view in the preview window.
pub(crate) async fn open(c: Ctx, p: Value) -> Result<Value> {
    let (session, checked, title) = session_for(&c, &p).await?;
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keine Fenster (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    let preview = c.0.preview.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().filter(|b| b.is_file());
    let Some(bin) = preview else {
        // Never the system browser: it would not keep the view's network locked.
        c.notice(
            NoticeLevel::Info,
            "Diese Ansicht braucht die Vorschau (aiscraft-preview), die hier nicht installiert ist. Der Text des Ergebnisses steht im Chat.",
        );
        return Ok(json!({"shown_in": "none"}));
    };
    let dir = c.0.open_dir.clone();
    crate::disk::create_dir(&dir)?;
    let path = dir.join(format!("ansicht-{}.html", crate::new_id()));
    crate::disk::write(&path, checked.html.as_bytes())?;
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.arg("--file").arg(&path).arg("--title").arg(title.chars().take(80).collect::<String>()).arg("--bridge");
    for d in &checked.connect {
        cmd.arg("--connect").arg(d);
    }
    for d in &checked.resources {
        cmd.arg("--resource").arg(d);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| Error::unsupported(format!("Die Vorschau (aiscraft-preview) startet nicht: {e}")))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(Error::unsupported("Die Vorschau (aiscraft-preview) hat keine Verbindung"));
    };
    let ctx = c.clone();
    c.spawn(async move {
        run_bridge(&ctx, session, stdin, stdout).await;
        let _ = child.wait().await;
        let _ = std::fs::remove_file(&path);
    });
    Ok(json!({"shown_in": "preview"}))
}

async fn run_bridge(c: &Ctx, session: Session, mut stdin: tokio::process::ChildStdin, stdout: tokio::process::ChildStdout) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let mut lines = tokio::io::BufReader::new(stdout.take(u64::MAX)).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > MAX_VIEW_MESSAGE {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        for out in handle(c, &session, msg).await {
            let mut bytes = out.to_string().into_bytes();
            bytes.push(b'\n');
            if stdin.write_all(&bytes).await.is_err() {
                return;
            }
        }
    }
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// One message from a view; what goes back (replies and, after `initialized`, the tool's input
/// and result). Hostile input never panics and never reaches past this view's connector.
pub(crate) async fn handle(c: &Ctx, s: &Session, msg: Value) -> Vec<Value> {
    let Some(obj) = msg.as_object() else { return Vec::new() };
    let method = obj.get("method").and_then(Value::as_str).unwrap_or("");
    let id = obj.get("id").cloned().filter(|i| i.is_string() || i.is_number());
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = id else {
        // Notifications.
        return match method {
            "ui/notifications/initialized" => vec![
                json!({"jsonrpc": "2.0", "method": "ui/notifications/tool-input", "params": {"arguments": s.arguments}}),
                json!({"jsonrpc": "2.0", "method": "ui/notifications/tool-result", "params": s.result}),
            ],
            _ => Vec::new(),
        };
    };
    match method {
        "ui/initialize" => {
            let variables: serde_json::Map<String, Value> = s.styles.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
            vec![reply(
                &id,
                json!({
                    "protocolVersion": BRIDGE_VERSION,
                    "hostInfo": {"name": "AIS.ChatCraft", "version": env!("CARGO_PKG_VERSION")},
                    "hostCapabilities": {"serverTools": {}, "serverResources": {}, "openLinks": {}, "logging": {}},
                    "hostContext": {
                        "theme": s.theme,
                        "styles": {"variables": variables},
                        "displayMode": "fullscreen",
                        "availableDisplayModes": ["fullscreen"],
                        "locale": "de-DE",
                        "platform": "desktop",
                        "toolInfo": {"tool": {"name": s.tool.name, "title": s.tool.title}},
                    }
                }),
            )]
        }
        "ping" => vec![reply(&id, json!({}))],
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !args.is_object() {
                return vec![error(&id, -32602, "arguments muss ein Objekt sein")];
            }
            let tool = c.state().connectors.iter().find(|k| k.id == s.connector).and_then(|k| k.tools.iter().find(|t| t.name == name).cloned());
            let Some(tool) = tool.filter(|t| t.app_callable) else {
                return vec![error(&id, -32601, "Dieses Werkzeug steht der Ansicht nicht zur Verfügung")];
            };
            // The same rules as the model's calls: reads run, the rest asks the person.
            let (text, is_error, outcome) = crate::tools::run_from_view(c, &s.conversation, &tool.llm_name, args).await;
            let mut result = json!({"content": [{"type": "text", "text": crate::tools::unwrap_untrusted(&text)}], "isError": is_error});
            if let (Some(sc), Some(o)) = (crate::tools::take_structured(c, &tool.llm_name), result.as_object_mut()) {
                o.insert("structuredContent".into(), sc);
            }
            let _ = outcome;
            vec![reply(&id, result)]
        }
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            if !uri.starts_with("ui://") || uri.len() > 2048 {
                return vec![error(&id, -32602, "Nur ui://-Ressourcen dieses Konnektors")];
            }
            let read = async { crate::tools::client(c, &s.connector).await?.read_resource(uri).await }.await;
            match read.and_then(|r| check(uri, &r).map(|v| (r, v))) {
                Ok((r, v)) => vec![reply(&id, json!({"contents": [{"uri": uri, "mimeType": r.mime_type, "text": v.html}]}))],
                Err(e) => vec![error(&id, -32000, &e.message)],
            }
        }
        "ui/open-link" => {
            let url = params.get("url").and_then(Value::as_str).unwrap_or("");
            if !url.starts_with("https://") || url.len() > 2000 || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return vec![error(&id, -32000, "Nur https-Adressen")];
            }
            // The person decides: a notice with the address and a button.
            c.notice_with(
                NoticeLevel::Info,
                format!("Die Ansicht möchte {url} öffnen."),
                Some(NoticeAction { label: "Im Browser öffnen".into(), command: "view.open_link".into(), params: json!({"url": url}) }),
            );
            vec![reply(&id, json!({}))]
        }
        "ui/message" | "ui/update-model-context" => vec![error(&id, -32601, "Rückmeldungen einer Ansicht an den Chat unterstützt AIS.ChatCraft noch nicht")],
        "ui/request-display-mode" => vec![reply(&id, json!({"mode": "fullscreen"}))],
        _ => vec![error(&id, -32601, "Methode nicht unterstützt")],
    }
}

/// `view.open_link`: the person clicked the link a view offered.
pub(crate) async fn open_link(c: Ctx, p: Value) -> Result<Value> {
    let url = req_str(&p, "url")?;
    if !url.starts_with("https://") || url.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        return Err(Error::invalid("Nur https-Adressen"));
    }
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keinen Browser (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    webbrowser::open(&url).map_err(|e| Error::unsupported(format!("Browser ließ sich nicht öffnen: {e}")))?;
    Ok(json!({}))
}

pub(crate) static COMMANDS: &[crate::CommandSpec] = &[
    crate::CommandSpec {
        id: "view.check",
        label: "Ansicht prüfen",
        group: "Konnektoren",
        shortcut: None,
        params: "message: string (Werkzeug-Nachricht mit Ansicht). Lädt die ui://-Ressource und prüft Typ, Größe und Netzfreigaben; öffnet nichts",
        palette: false,
        run: |c, p| Box::pin(check_cmd(c, p)),
    },
    crate::CommandSpec {
        id: "view.open",
        label: "Ansicht öffnen",
        group: "Konnektoren",
        shortcut: None,
        params: "message: string, theme?: light|dark, styles?: {\"--color-…\": \"#rrggbb\"}. Öffnet die Ansicht in der Vorschau (aiscraft-preview)",
        palette: false,
        run: |c, p| Box::pin(open(c, p)),
    },
    crate::CommandSpec {
        id: "view.open_link",
        label: "Link einer Ansicht öffnen",
        group: "Konnektoren",
        shortcut: None,
        params: "url: string (https)",
        palette: false,
        run: |c, p| Box::pin(open_link(c, p)),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_domains() {
        for ok in ["https://api.example.org", "https://*.example.org", "https://cdn.example.org:8443"] {
            assert!(domain_ok(ok, false), "{ok}");
        }
        assert!(domain_ok("wss://live.example.org", true) && !domain_ok("wss://live.example.org", false));
        for bad in [
            "*",
            "https://*",
            "http://example.org",
            "data:",
            "https://example.org/path",
            "'unsafe-eval'",
            "https://exa mple.org",
            "https://example.org;",
            "example.org",
            "https://localhost",
        ] {
            assert!(!domain_ok(bad, true), "{bad}");
        }
        assert!(csp(&[], &[]).contains("connect-src 'none'"));
        assert!(csp(&["https://api.example.org".into()], &[]).contains("connect-src https://api.example.org;"));
    }
}
