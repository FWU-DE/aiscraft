//! The local Claude Code CLI as a chat backend, so a claude.ai subscription can be used.
//!
//! Each answer is one `claude -p` run with every tool and MCP server switched off (but the app's
//! tool bridge, and the CLI's web search while the chat may use the web, #122), no saved
//! session and an empty working directory: the CLI only talks, it cannot touch files. The whole
//! conversation goes in each time (as text plus image/PDF blocks via `--input-format
//! stream-json`), so AIS.ChatCraft stays the only place the history lives. `ANTHROPIC_API_KEY` is
//! removed from the environment so the subscription, not an API key, pays.

use std::process::Stdio;

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, ChatRequest, Error, Message, ModelInfo, Result, Role, StreamEvent, Usage};
use aiscraft_core::{ActivityLink, ActivityStep, WebAccess, WebRead, WebSearch};
use futures::future::BoxFuture;
use futures::{StreamExt, stream};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{Backend, EventStream};

const MODELS: &[(&str, &str, &str)] = &[
    ("claude-opus-5-5", "Claude Opus 5.5", "Das stärkste Allround-Modell"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5", "Schnell und stark"),
    ("claude-haiku-5-5", "Claude Haiku 5.5", "Am schnellsten"),
    ("claude-fable-5-2", "Claude Fable 5.2", "Für die schwierigsten Aufgaben, langsamer"),
];
/// A line of output larger than this is not a CLI event we understand.
const MAX_LINE: usize = 16 * 1024 * 1024;
/// Env vars that would make the CLI use an API key or think it runs nested inside Claude Code.
const STRIP_ENV: &[&str] = &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT"];

pub struct ClaudeCli {
    config: BackendConfig,
    program: String,
}

impl ClaudeCli {
    pub fn new(config: BackendConfig, program: String) -> Result<Self> {
        let program = if program.trim().is_empty() || program.starts_with("http") { "claude".to_string() } else { program.trim().to_string() };
        Ok(Self { config, program })
    }

    fn command(&self) -> tokio::process::Command {
        let mut c = tokio::process::Command::new(&self.program);
        for k in STRIP_ENV {
            c.env_remove(k);
        }
        c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        c
    }
}

/// A file removed when dropped (the MCP config holds the bridge token).
struct TempFile(std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    // Mode 0600 from the first byte: the file holds the bridge token.
    let private = craft_atomic::WriteOptions::default().with_new_file_permissions(craft_atomic::NewFilePermissions::Private);
    private.atomic_write(path, bytes).map_err(|e| Error::io("MCP-Konfiguration", &e))?;
    Ok(())
}

/// The Claude CLI ended in the middle of an answer: which service stopped and what to do, in the
/// style of the connector messages, with the CLI's last output.
pub fn cli_ended(code: Option<i32>, last_output: &str) -> Error {
    let code = code.map(|c| format!(" (Exit-Code {c})")).unwrap_or_default();
    let mut msg = format!(
        "Die Claude-CLI wurde unerwartet beendet{code}. Bitte die Nachricht noch einmal senden; wenn das wieder passiert, im Terminal `claude` starten und prüfen, ob sie läuft, dann AIS.ChatCraft neu starten."
    );
    let tail: String = last_output.trim().chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect();
    if !tail.is_empty() {
        msg.push_str(" Letzte Ausgabe: ");
        msg.push_str(&tail);
    }
    Error::network(msg)
}

/// The conversation so far as one prompt: earlier turns as a transcript, the last user message
/// as the request, attachments as content blocks.
pub fn input_message(req: &ChatRequest) -> Value {
    let msgs: Vec<&Message> = req.messages.iter().filter(|m| m.role == Role::User || (m.role == Role::Assistant && !m.content.is_empty())).collect();
    let (last, earlier) = match msgs.split_last() {
        Some((l, e)) if l.role == Role::User => (Some(*l), e),
        _ => (None, msgs.as_slice()),
    };
    let mut text = String::new();
    if !earlier.is_empty() {
        text.push_str("<bisheriger_verlauf>\n");
        for m in earlier {
            let who = if m.role == Role::User { "Person" } else { "Assistent" };
            text.push_str(&format!("<{who}>\n{}\n</{who}>\n", crate::anthropic::text_with_files(m)));
        }
        text.push_str("</bisheriger_verlauf>\n\nAntworte jetzt auf die folgende Nachricht der Person.\n\n");
    }
    let mut blocks = Vec::new();
    if let Some(l) = last {
        text.push_str(&crate::anthropic::text_with_files(l));
        for a in &l.attachments {
            if let Some(d) = &a.data_base64 {
                let kind = if a.mime == "application/pdf" { "document" } else { "image" };
                blocks.push(json!({"type": kind, "source": {"type": "base64", "media_type": a.mime, "data": d}}));
            }
        }
    }
    blocks.push(json!({"type": "text", "text": text}));
    json!({"type": "user", "message": {"role": "user", "content": blocks}})
}

/// The CLI's own tool for the web search (#122). Its `WebFetch` stays off: pages go through the
/// app's reader over the bridge, which refuses private addresses and logs every call.
const WEB_SEARCH_TOOL: &str = "WebSearch";
const MAX_STEPS: usize = 20;
const MAX_LINKS: usize = 20;
const MAX_TEXT: usize = 2 * 1024 * 1024;

/// The command line of one answer: every tool off, but the app's bridge (when there is one)
/// and the CLI's web search (when the chat may go on the web).
pub fn cli_args(req: &ChatRequest, mcp_config: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--tools",
        if req.web { WEB_SEARCH_TOOL } else { "" },
        "--strict-mcp-config",
        "--no-session-persistence",
        "--setting-sources",
        "",
        "--model",
        &req.model,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(sys) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        a.extend(["--system-prompt".to_string(), sys.to_string()]);
    }
    let mut allowed: Vec<&str> = Vec::new();
    if let Some(path) = mcp_config {
        a.extend(["--mcp-config".to_string(), path.to_string()]);
        allowed.push("mcp__aischatcraft");
    }
    if req.web {
        allowed.push(WEB_SEARCH_TOOL);
    }
    if !allowed.is_empty() {
        a.extend(["--allowedTools".to_string(), allowed.join(",")]);
    }
    a
}

/// One answer's `stream-json` output, line by line. It remembers the text (to see which pages
/// the answer cites) and the steps of the CLI's web search, shown as „Websuche“ with its links.
#[derive(Default)]
pub struct Lines {
    text: String,
    steps: Vec<(String, ActivityStep)>,
}

fn clean(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect::<String>().trim().to_string()
}

/// The pages one search found: the CLI's structured result, else the `Links: [...]` in its text.
fn search_links(line: &Value, item: &Value) -> Vec<ActivityLink> {
    let mut raw: Vec<Value> = line
        .pointer("/tool_use_result/results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("content").and_then(Value::as_array))
        .flatten()
        .cloned()
        .collect();
    if raw.is_empty() {
        let text = match item.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        };
        if let Some((_, rest)) = text.split_once("Links: ")
            && let Some(Ok(Value::Array(list))) = serde_json::Deserializer::from_str(rest).into_iter::<Value>().next()
        {
            raw = list;
        }
    }
    let mut out: Vec<ActivityLink> = Vec::new();
    for r in raw {
        let url = r.get("url").and_then(Value::as_str).unwrap_or("").trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 || url.chars().any(char::is_control) {
            continue;
        }
        if out.len() >= MAX_LINKS || out.iter().any(|l| l.url == url) {
            continue;
        }
        let title = clean(r.get("title").and_then(Value::as_str).unwrap_or(""), 200);
        out.push(ActivityLink { title: if title.is_empty() { url.to_string() } else { title }, url: url.to_string(), failed: false, cited: false });
    }
    out
}

impl Lines {
    fn activity(&self) -> StreamEvent {
        StreamEvent::Activity { steps: self.steps.iter().map(|(_, s)| s.clone()).collect() }
    }

    /// Map one line of `--output-format stream-json` to events.
    pub fn line(&mut self, line: &str) -> Result<Vec<StreamEvent>> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return Ok(vec![]) };
        let content = |v: &Value| v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default();
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            // The model asks for a search: a step with the query.
            "assistant" => {
                let mut changed = false;
                for item in content(&v) {
                    let id = item.get("id").and_then(Value::as_str).unwrap_or("");
                    if item.get("type").and_then(Value::as_str) == Some("tool_use")
                        && item.get("name").and_then(Value::as_str) == Some(WEB_SEARCH_TOOL)
                        && !id.is_empty()
                        && self.steps.len() < MAX_STEPS
                        && !self.steps.iter().any(|(i, _)| i == id)
                    {
                        let query = clean(item.pointer("/input/query").and_then(Value::as_str).unwrap_or(""), 200);
                        let step = ActivityStep { label: "Websuche".into(), detail: (!query.is_empty()).then_some(query), ..Default::default() };
                        self.steps.push((id.to_string(), step));
                        changed = true;
                    }
                }
                Ok(if changed { vec![self.activity()] } else { vec![] })
            }
            // The search's result: the step is done, with the pages it found.
            "user" => {
                let mut changed = false;
                for item in content(&v) {
                    let id = item.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                    if item.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let links = search_links(&v, &item);
                    if let Some((_, step)) = self.steps.iter_mut().find(|(i, _)| i == id) {
                        step.done = true;
                        step.failed = item.get("is_error").and_then(Value::as_bool) == Some(true);
                        step.links = links;
                        changed = true;
                    }
                }
                Ok(if changed { vec![self.activity()] } else { vec![] })
            }
            "result" if v.get("is_error").and_then(Value::as_bool) != Some(true) => {
                let mut out = line_events(line)?;
                if !self.steps.is_empty() {
                    // The pages the answer links to are its sources, shown under it.
                    for (_, step) in self.steps.iter_mut() {
                        step.done = true;
                        for l in step.links.iter_mut() {
                            l.cited = self.text.contains(&l.url);
                        }
                    }
                    out.insert(0, self.activity());
                }
                Ok(out)
            }
            _ => {
                let out = line_events(line)?;
                for e in &out {
                    if let StreamEvent::Delta { text } = e
                        && self.text.len() + text.len() <= MAX_TEXT
                    {
                        self.text.push_str(text);
                    }
                }
                Ok(out)
            }
        }
    }
}

/// Map one line of `--output-format stream-json` to events (text, refusal, result).
pub fn line_events(line: &str) -> Result<Vec<StreamEvent>> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Ok(vec![]) };
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    Ok(match ty {
        "stream_event" => {
            let ev = v.get("event").cloned().unwrap_or(Value::Null);
            match ev.get("type").and_then(Value::as_str) {
                Some("content_block_delta") if ev.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") => {
                    ev.pointer("/delta/text").and_then(Value::as_str).map(|t| vec![StreamEvent::Delta { text: t.to_string() }]).unwrap_or_default()
                }
                Some("message_delta") if ev.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("refusal") => {
                    vec![StreamEvent::Delta { text: "\n\n_(Das Modell hat diese Anfrage abgelehnt.)_".into() }]
                }
                _ => vec![],
            }
        }
        "result" => {
            if v.get("is_error").and_then(Value::as_bool) == Some(true) {
                let msg = v.get("result").and_then(Value::as_str).unwrap_or("Claude-CLI meldet einen Fehler");
                let lower = msg.to_lowercase();
                return Err(if lower.contains("login") || lower.contains("auth") {
                    Error::auth(format!("{msg} – im Terminal `claude auth login` ausführen"))
                } else if lower.contains("limit") || lower.contains("overloaded") {
                    Error::busy(msg.to_string())
                } else {
                    Error::protocol(msg.to_string())
                });
            }
            let input = n("/usage/input_tokens") + n("/usage/cache_read_input_tokens") + n("/usage/cache_creation_input_tokens");
            vec![StreamEvent::Usage { usage: Usage { input_tokens: input, output_tokens: n("/usage/output_tokens") } }, StreamEvent::Done]
        }
        _ => vec![],
    })
}

impl Backend for ClaudeCli {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            remote_history: false,
            uploads: false,
            system_prompt: true,
            vision: true,
            tools: false,
            tools_via_mcp: true,
            web: WebAccess::new(WebSearch::CliTool, WebRead::AppReader),
        }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let mut c = self.command();
            c.args(["auth", "status"]).stdin(Stdio::null());
            let out = tokio::time::timeout(std::time::Duration::from_secs(20), c.output())
                .await
                .map_err(|_| Error::network("Claude-CLI antwortet nicht"))?
                .map_err(|e| Error::not_found(format!("Claude-CLI „{}“ nicht gefunden: {e}", self.program)))?;
            let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
            if v.get("loggedIn").and_then(Value::as_bool) != Some(true) {
                return Err(Error::auth("Claude-CLI ist nicht angemeldet. Im Terminal `claude auth login` ausführen."));
            }
            let method = v.get("authMethod").and_then(Value::as_str).unwrap_or("");
            let mut detail = match method {
                "claude.ai" => "claude.ai-Abo".to_string(),
                "" => "angemeldet".to_string(),
                other => format!("angemeldet über {other} (nicht das Abo)"),
            };
            if let Some(email) = v.get("email").and_then(Value::as_str) {
                detail = format!("{detail} · {email}");
            }
            // No person's name: the account is an email address, and a greeting should not use it.
            Ok(AccountInfo {
                display_name: None,
                detail: Some(detail),
                expires: v.get("loginExpiresAt").and_then(Value::as_str).map(str::to_string),
                server_tools: vec![],
            })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            Ok(MODELS
                .iter()
                .map(|(id, name, d)| ModelInfo {
                    id: id.to_string(),
                    name: name.to_string(),
                    description: Some(d.to_string()),
                    vision: true,
                    context_window: Some(1_000_000),
                })
                .collect())
        })
    }

    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>> {
        Box::pin(async move {
            let dir = std::env::temp_dir().join(format!("aiscraft-claude-{}", std::process::id()));
            std::fs::create_dir_all(&dir).map_err(|e| Error::io("Arbeitsordner", &e))?;
            let mut c = self.command();
            // The app's tools, through its MCP bridge: only that server, every other tool stays off.
            let mcp_file = match &req.tool_bridge {
                Some(b) => {
                    let path = dir.join(format!("mcp-{}.json", std::process::id() as u64 ^ req.conversation_id.len() as u64 ^ b.token.len() as u64));
                    let cfg =
                        json!({"mcpServers": {"aischatcraft": {"type": "http", "url": b.url, "headers": {"Authorization": format!("Bearer {}", b.token)}}}});
                    write_private(&path, cfg.to_string().as_bytes())?;
                    Some(path)
                }
                None => None,
            };
            let mcp_path = mcp_file.as_ref().map(|p| p.display().to_string());
            c.current_dir(&dir).args(cli_args(&req, mcp_path.as_deref()));
            let mut child = c.spawn().map_err(|e| Error::not_found(format!("Claude-CLI „{}“ lässt sich nicht starten: {e}", self.program)))?;
            let mut stdin = child.stdin.take().ok_or_else(|| Error::protocol("Claude-CLI: keine Eingabe"))?;
            let stdout = child.stdout.take().ok_or_else(|| Error::protocol("Claude-CLI: keine Ausgabe"))?;
            let mut stderr = child.stderr.take();
            let line = format!("{}\n", input_message(&req));
            // A CLI that dies at start closes its input before we write; the read side then says
            // why it ended (exit code, last output), which tells more than "broken pipe".
            if let Err(e) = stdin.write_all(line.as_bytes()).await {
                log::debug!("Claude-CLI: Eingabe nicht geschrieben: {e}");
            }
            drop(stdin); // end of input: one turn

            struct St {
                _mcp: Option<TempFile>,
                child: tokio::process::Child,
                lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
                stderr: Option<tokio::process::ChildStderr>,
                queue: std::collections::VecDeque<Result<StreamEvent>>,
                finished: bool,
                parse: Lines,
            }
            let st = St {
                _mcp: mcp_file.map(TempFile),
                child,
                lines: BufReader::new(stdout).lines(),
                stderr: stderr.take(),
                queue: Default::default(),
                finished: false,
                parse: Lines::default(),
            };
            Ok(stream::unfold(st, |mut st| async move {
                loop {
                    if let Some(item) = st.queue.pop_front() {
                        if matches!(item, Ok(StreamEvent::Done) | Err(_)) {
                            st.finished = true;
                            st.queue.clear();
                        }
                        return Some((item, st));
                    }
                    if st.finished {
                        let _ = st.child.start_kill();
                        return None;
                    }
                    match st.lines.next_line().await {
                        Ok(Some(l)) if l.len() > MAX_LINE => st.queue.push_back(Err(Error::protocol("Claude-CLI: Zeile zu lang"))),
                        Ok(Some(l)) => match st.parse.line(&l) {
                            Ok(evs) => st.queue.extend(evs.into_iter().map(Ok)),
                            Err(e) => st.queue.push_back(Err(e)),
                        },
                        Ok(None) => {
                            // Ended without a result line: report what the CLI said on stderr.
                            let mut err = String::new();
                            if let Some(mut e) = st.stderr.take() {
                                let mut buf = Vec::new();
                                let _ = (&mut e).take(8192).read_to_end(&mut buf).await;
                                err = String::from_utf8_lossy(&buf).trim().to_string();
                            }
                            let status = st.child.wait().await.ok();
                            let success = status.is_some_and(|s| s.success());
                            st.queue.push_back(if success && err.is_empty() {
                                Ok(StreamEvent::Done)
                            } else if success {
                                Err(Error::protocol(format!("Claude-CLI: {err}")))
                            } else {
                                Err(cli_ended(status.and_then(|s| s.code()), &err))
                            });
                        }
                        Err(e) => st.queue.push_back(Err(Error::protocol(format!("Claude-CLI: {e}")))),
                    }
                }
            })
            .boxed())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: Role, c: &str) -> Message {
        Message {
            id: "x".into(),
            conversation_id: "c".into(),
            role,
            content: c.into(),
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
        }
    }

    #[test]
    fn transcript_and_last_message() {
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: "claude-haiku-5-5".into(),
            system: None,
            messages: vec![m(Role::User, "Hallo"), m(Role::Assistant, "Hi!"), m(Role::User, "Und jetzt?")],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
            effort: None,
        };
        let v = input_message(&req);
        let text = v.pointer("/message/content/0/text").and_then(Value::as_str).unwrap();
        assert!(text.contains("<Person>\nHallo\n</Person>"));
        assert!(text.contains("<Assistent>\nHi!\n</Assistent>"));
        assert!(text.ends_with("Und jetzt?"));
    }

    #[test]
    fn stream_lines() {
        let d = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hallo"}}}"#;
        assert_eq!(line_events(d).unwrap(), vec![StreamEvent::Delta { text: "Hallo".into() }]);
        let r = r#"{"type":"result","subtype":"success","is_error":false,"usage":{"input_tokens":2,"cache_read_input_tokens":10,"output_tokens":5}}"#;
        let evs = line_events(r).unwrap();
        assert_eq!(evs.last(), Some(&StreamEvent::Done));
        assert!(
            line_events(r#"{"type":"result","is_error":true,"result":"Not logged in · Please run /login"}"#).unwrap_err().kind
                == aiscraft_core::ErrorKind::Auth
        );
        assert!(line_events("garbage").unwrap().is_empty());
    }
    /// With „Web“ on, the CLI may use its web search (and only that besides the bridge); with
    /// „Web“ off every tool stays off as before (#122). Its WebFetch is never switched on.
    #[test]
    fn web_search_only_while_the_chat_may_use_the_web() {
        let req = |web: bool| ChatRequest {
            conversation_id: "c".into(),
            model: "claude-opus-5-5".into(),
            system: None,
            messages: vec![],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web,
            effort: None,
        };
        let after = |a: &[String], flag: &str| a.iter().position(|x| x == flag).and_then(|i| a.get(i + 1)).cloned();
        let off = cli_args(&req(false), None);
        assert_eq!(after(&off, "--tools").as_deref(), Some(""));
        assert!(!off.contains(&"--allowedTools".to_string()), "{off:?}");
        let off_bridge = cli_args(&req(false), Some("/tmp/mcp.json"));
        assert_eq!(after(&off_bridge, "--tools").as_deref(), Some(""));
        assert_eq!(after(&off_bridge, "--allowedTools").as_deref(), Some("mcp__aischatcraft"));
        let on = cli_args(&req(true), Some("/tmp/mcp.json"));
        assert_eq!(after(&on, "--tools").as_deref(), Some("WebSearch"));
        assert_eq!(after(&on, "--allowedTools").as_deref(), Some("mcp__aischatcraft,WebSearch"));
        assert_eq!(after(&cli_args(&req(true), None), "--allowedTools").as_deref(), Some("WebSearch"));
        for a in [&off, &off_bridge, &on] {
            assert!(!a.iter().any(|x| x.contains("WebFetch")), "{a:?}");
        }
    }

    /// The CLI's web search shows as „Websuche“ with its pages; the pages the answer links to
    /// are its sources under it. Hostile results (no http, control characters) are dropped.
    #[test]
    fn web_search_steps_and_sources() {
        let mut p = Lines::default();
        let ask = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s1","name":"WebSearch","input":{"query":"Ferien Bayern 2027"}}]}}"#;
        let out = p.line(ask).unwrap();
        let [StreamEvent::Activity { steps }] = out.as_slice() else { panic!("{out:?}") };
        assert_eq!((steps[0].label.as_str(), steps[0].detail.as_deref(), steps[0].done), ("Websuche", Some("Ferien Bayern 2027"), false));
        let result = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "s1", "content": "Web search results"}]},
            "tool_use_result": {"query": "Ferien Bayern 2027", "results": [{"tool_use_id": "x", "content": [
                {"title": "Ferientermine", "url": "https://www.km.bayern.de/ferien"},
                {"title": "Böse", "url": "javascript:alert(1)"},
                {"title": "Andere", "url": "https://example.org/ferien"}]}, "Zusammenfassung"]}});
        let out = p.line(&result.to_string()).unwrap();
        let [StreamEvent::Activity { steps }] = out.as_slice() else { panic!("{out:?}") };
        assert!(steps[0].done && steps[0].links.len() == 2, "{steps:?}");
        // Text form, as older CLIs write it.
        let ask2 = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"s2","name":"WebSearch","input":{"query":"zweite"}}]}}"#;
        p.line(ask2).unwrap();
        let text = "Web search results for query: \"zweite\"\n\nLinks: [{\"title\":\"Drei\",\"url\":\"https://drei.example/\"}]\n\nMehr";
        let r2 = json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "s2", "content": [{"type": "text", "text": text}]}]}});
        let out = p.line(&r2.to_string()).unwrap();
        let [StreamEvent::Activity { steps }] = out.as_slice() else { panic!("{out:?}") };
        assert_eq!(steps[1].links[0].url, "https://drei.example/");
        // Other tools (the bridge) make no step.
        assert!(
            p.line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"b","name":"mcp__aischatcraft__web_fetch","input":{}}]}}"#)
                .unwrap()
                .is_empty()
        );
        let d = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Laut [KM](https://www.km.bayern.de/ferien) …"}}}"#;
        p.line(d).unwrap();
        let evs = p.line(r#"{"type":"result","is_error":false,"usage":{"input_tokens":1,"output_tokens":1}}"#).unwrap();
        let StreamEvent::Activity { steps } = &evs[0] else { panic!("{evs:?}") };
        let cited: Vec<&str> = steps.iter().flat_map(|s| &s.links).filter(|l| l.cited).map(|l| l.url.as_str()).collect();
        assert_eq!(cited, vec!["https://www.km.bayern.de/ferien"]);
        assert_eq!(evs.last(), Some(&StreamEvent::Done));
        // Without a search, a result is as before: usage and done.
        assert_eq!(Lines::default().line(r#"{"type":"result","is_error":false}"#).unwrap().len(), 2);
    }

    /// A CLI that ends in the middle of an answer: the message names the Claude CLI, the exit
    /// code and the next step, and keeps its last output. A shell script stands in for `claude`.
    #[cfg(unix)]
    #[tokio::test]
    async fn claude_cli_that_ends_mid_answer_says_so() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("aiscraft-fake-claude-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("claude");
        std::fs::write(&script, "#!/bin/sh\ncat >/dev/null\necho '{\"type\":\"system\"}'\necho 'Speicherzugriffsfehler' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cfg = BackendConfig {
            id: "cc".into(),
            name: "Claude".into(),
            kind: aiscraft_core::BackendKind::ClaudeCli,
            base_url: "claude".into(),
            default_model: None,
            send_context: true,
        };
        let b = ClaudeCli::new(cfg, script.display().to_string()).unwrap();
        let req = || ChatRequest {
            conversation_id: "c".into(),
            model: "claude-haiku-5-5".into(),
            system: None,
            messages: vec![m(Role::User, "Hallo")],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
            effort: None,
        };
        // Another test thread may still hold the fresh script open for writing (ETXTBSY).
        let mut stream = None;
        for _ in 0..20 {
            match b.chat(req()).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            }
        }
        let mut stream = stream.expect("fake CLI starts");
        let mut err = None;
        while let Some(ev) = stream.next().await {
            if let Err(e) = ev {
                err = Some(e);
            }
        }
        let e = err.expect("an error at the end");
        assert_eq!(e.kind, aiscraft_core::ErrorKind::Network);
        assert!(e.message.starts_with("Die Claude-CLI wurde unerwartet beendet (Exit-Code 3)."), "{}", e.message);
        assert!(e.message.contains("noch einmal senden"), "{}", e.message);
        assert!(e.message.ends_with("Letzte Ausgabe: Speicherzugriffsfehler"), "{}", e.message);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cli_ended_without_code_or_output() {
        let e = cli_ended(None, "  ");
        assert!(e.message.starts_with("Die Claude-CLI wurde unerwartet beendet. Bitte"), "{}", e.message);
        assert!(!e.message.contains("Letzte Ausgabe"));
        assert!(cli_ended(Some(1), &"x".repeat(5000)).message.chars().count() < 900);
    }

    /// Talks to the real CLI with the subscription. `cargo test -p aiscraft-backends -- --ignored claude_cli_live`
    #[tokio::test]
    #[ignore]
    async fn claude_cli_live() {
        let cfg = BackendConfig {
            id: "cc".into(),
            name: "Claude".into(),
            kind: aiscraft_core::BackendKind::ClaudeCli,
            base_url: "claude".into(),
            default_model: None,
            send_context: true,
        };
        let b = ClaudeCli::new(cfg, "claude".into()).unwrap();
        b.check().await.unwrap();
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: "claude-haiku-5-5".into(),
            system: Some("Antworte mit genau einem Wort.".into()),
            messages: vec![m(Role::User, "Sag Hallo")],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
            effort: None,
        };
        let mut s = b.chat(req).await.unwrap();
        let mut text = String::new();
        while let Some(e) = s.next().await {
            if let StreamEvent::Delta { text: t } = e.unwrap() {
                text.push_str(&t);
            }
        }
        assert!(!text.trim().is_empty());
    }
}
