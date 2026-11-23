//! The local Claude Code CLI as a chat backend, so a claude.ai subscription can be used.
//!
//! Each answer is one `claude -p` run with every tool and MCP server switched off, no saved
//! session and an empty working directory: the CLI only talks, it cannot touch files. The whole
//! conversation goes in each time (as text plus image/PDF blocks via `--input-format
//! stream-json`), so AIS.ChatCraft stays the only place the history lives. `ANTHROPIC_API_KEY` is
//! removed from the environment so the subscription, not an API key, pays.

use std::process::Stdio;

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, ChatRequest, Error, Message, ModelInfo, Result, Role, StreamEvent, Usage};
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
    private.atomic_write(path, bytes).map_err(|e| Error::storage(format!("MCP-Konfiguration: {e}")))?;
    Ok(())
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

/// Map one line of `--output-format stream-json` to events.
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
                } else if lower.contains("limit") {
                    Error::refused(msg.to_string())
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
        Capabilities { remote_history: false, uploads: false, system_prompt: true, vision: true, tools: false, tools_via_mcp: true, web_search: false }
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
            std::fs::create_dir_all(&dir).map_err(|e| Error::storage(format!("Arbeitsordner: {e}")))?;
            let mut c = self.command();
            c.current_dir(&dir).args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--tools",
                "",
                "--strict-mcp-config",
                "--no-session-persistence",
                "--setting-sources",
                "",
                "--model",
                &req.model,
            ]);
            if let Some(sys) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
                c.args(["--system-prompt", sys]);
            }
            // The app's tools, through its MCP bridge: only that server, every other tool stays off.
            let mcp_file = match &req.tool_bridge {
                Some(b) => {
                    let path = dir.join(format!("mcp-{}.json", std::process::id() as u64 ^ req.conversation_id.len() as u64 ^ b.token.len() as u64));
                    let cfg =
                        json!({"mcpServers": {"aischatcraft": {"type": "http", "url": b.url, "headers": {"Authorization": format!("Bearer {}", b.token)}}}});
                    write_private(&path, cfg.to_string().as_bytes())?;
                    c.args(["--mcp-config", &path.display().to_string(), "--allowedTools", "mcp__aischatcraft"]);
                    Some(path)
                }
                None => None,
            };
            let mut child = c.spawn().map_err(|e| Error::not_found(format!("Claude-CLI „{}“ lässt sich nicht starten: {e}", self.program)))?;
            let mut stdin = child.stdin.take().ok_or_else(|| Error::protocol("Claude-CLI: keine Eingabe"))?;
            let stdout = child.stdout.take().ok_or_else(|| Error::protocol("Claude-CLI: keine Ausgabe"))?;
            let mut stderr = child.stderr.take();
            let line = format!("{}\n", input_message(&req));
            stdin.write_all(line.as_bytes()).await.map_err(|e| Error::protocol(format!("Claude-CLI: {e}")))?;
            drop(stdin); // end of input: one turn

            struct St {
                _mcp: Option<TempFile>,
                child: tokio::process::Child,
                lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
                stderr: Option<tokio::process::ChildStderr>,
                queue: std::collections::VecDeque<Result<StreamEvent>>,
                finished: bool,
            }
            let st = St {
                _mcp: mcp_file.map(TempFile),
                child,
                lines: BufReader::new(stdout).lines(),
                stderr: stderr.take(),
                queue: Default::default(),
                finished: false,
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
                        Ok(Some(l)) => match line_events(&l) {
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
                            st.queue.push_back(if status.is_some_and(|s| s.success()) && err.is_empty() {
                                Ok(StreamEvent::Done)
                            } else {
                                Err(Error::protocol(if err.is_empty() {
                                    "Claude-CLI wurde unerwartet beendet".to_string()
                                } else {
                                    format!("Claude-CLI: {err}")
                                }))
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
