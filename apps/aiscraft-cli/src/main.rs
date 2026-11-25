//! `aiscraft-cli`: the same engine and data as the desktop app, on the command line.

use std::io::{BufRead, Read, Write};
use std::time::{Duration, Instant};

use aiscraft_engine::{BackendStatus, Engine, Options, commands};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "aiscraft-cli", version, about = "AIS.ChatCraft auf der Kommandozeile")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Bei einem Backend anmelden. Ohne --secret wird der Wert von der Standardeingabe gelesen.
    Login {
        #[arg(long, default_value = "ais-chat")]
        backend: String,
        #[arg(long)]
        secret: Option<String>,
    },
    /// Backends und ihr Anmeldestatus.
    Status,
    /// Modelle eines Backends.
    Models {
        #[arg(long)]
        backend: Option<String>,
    },
    /// Eine Nachricht senden und die Antwort ausgeben (im zuletzt gewählten oder neuen Chat).
    Chat {
        text: Vec<String>,
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// In einem neuen Chat statt im zuletzt geöffneten.
        #[arg(long)]
        new: bool,
        /// Im bestehenden Chat mit dieser ID weiterschreiben.
        #[arg(long)]
        conversation: Option<String>,
        #[arg(long = "file")]
        files: Vec<String>,
        /// Nur die fertige Antwort als JSON.
        #[arg(long)]
        json: bool,
    },
    /// Chats auflisten.
    List {
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// Alle Chats durchsuchen.
    Search { query: Vec<String> },
    /// Einen Chat als Markdown (oder JSON) ausgeben.
    Export {
        id: String,
        #[arg(long, default_value = "markdown")]
        format: String,
    },
    /// Einen beliebigen Engine-Befehl aufrufen: `aiscraft-cli call chat.rename '{"id":"…","title":"…"}'`.
    Call { command: String, params: Option<String> },
    /// Alle Befehle mit Parametern.
    Commands,
    /// MCP-Server über stdio: jeder Befehl wird ein Werkzeug.
    Mcp,
    /// Das Handbuch ausgeben (Markdown).
    Manual,
}

/// The same handbook as in the app (F1).
const MANUAL: &str = include_str!("../../../crates/ui/manual/de.md");

fn main() {
    let cli = Cli::parse();
    if matches!(cli.cmd, Cmd::Manual) {
        print!("{MANUAL}");
        return;
    }
    let engine = match Engine::open(Options::default_dirs()) {
        Ok(e) => e,
        Err(e) => fail(&e.message),
    };
    let result = match cli.cmd {
        Cmd::Mcp => {
            mcp(&engine);
            Ok(())
        }
        other => run(&engine, other),
    };
    if let Err(e) = result {
        fail(&e);
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("Fehler: {msg}");
    std::process::exit(1);
}

fn call(engine: &Engine, cmd: &str, params: Value) -> Result<Value, String> {
    engine.call(cmd, params).map_err(|e| e.message)
}

fn wait_backend(engine: &Engine, id: &str) {
    use aiscraft_core::BackendKind;
    engine.wait_for(Duration::from_secs(20), |s| {
        s.backend(id).is_none_or(|b| {
            let needs_secret = matches!(b.config.kind, BackendKind::AisChat | BackendKind::Anthropic | BackendKind::Vertex);
            !matches!(b.status, BackendStatus::Checking | BackendStatus::Unknown) || (needs_secret && !b.has_secret)
        })
    });
}

fn run(engine: &Engine, cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Login { backend, secret } => {
            let secret = match secret {
                Some(s) => s,
                None => {
                    eprintln!("Wert einfügen (Sitzungs-Cookie oder API-Schlüssel), dann Enter:");
                    let mut s = String::new();
                    std::io::stdin().lock().read_line(&mut s).map_err(|e| e.to_string())?;
                    s
                }
            };
            let account = call(engine, "backend.login", json!({"id": backend, "secret": secret.trim()}))?;
            println!(
                "Angemeldet: {}",
                [account.get("display_name").and_then(serde_json::Value::as_str), account.get("detail").and_then(serde_json::Value::as_str)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ")
            );
        }
        Cmd::Status => {
            for b in &engine.state().backends {
                wait_backend(engine, &b.config.id);
            }
            for b in &engine.state().backends {
                let status = match &b.status {
                    BackendStatus::Ready { account } => format!(
                        "bereit  {}",
                        [account.display_name.clone(), account.detail.clone(), account.expires.clone().map(|e| format!("bis {e}"))]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ")
                    ),
                    BackendStatus::SignedOut { message } => format!("abgemeldet  {message}"),
                    BackendStatus::Failed { message } => format!("Fehler  {message}"),
                    BackendStatus::Checking => "prüft …".into(),
                    BackendStatus::Unknown => {
                        if b.has_secret {
                            "unbekannt".into()
                        } else {
                            "nicht angemeldet".into()
                        }
                    }
                };
                println!("{:<24} {:<22} {status}", b.config.id, b.config.kind.label());
            }
        }
        Cmd::Models { backend } => {
            let id = backend.unwrap_or_else(|| engine.state().selection.backend_id.clone());
            call(engine, "backend.check", json!({"id": id}))?;
            let s = engine.state();
            for m in s.backend(&id).map(|b| b.models.clone()).unwrap_or_default() {
                println!("{:<40} {}", m.id, m.name);
            }
        }
        Cmd::Chat { text, backend, model, new, conversation, files, json: as_json } => {
            let mut text = text.join(" ");
            if text.is_empty() && files.is_empty() {
                std::io::stdin().read_to_string(&mut text).map_err(|e| e.to_string())?;
            }
            if let Some(id) = conversation {
                call(engine, "chat.open", json!({"id": id}))?;
            } else if new || engine.state().current.is_none() {
                call(engine, "chat.new", json!({"backend": backend}))?;
            }
            let backend_id = backend.unwrap_or_else(|| engine.state().selection.backend_id.clone());
            wait_backend(engine, &backend_id);
            if let Some(m) = model {
                call(engine, "chat.select_model", json!({"backend": backend_id, "model": m}))?;
            } else if engine.state().selection.backend_id != backend_id {
                let first = engine.state().backend(&backend_id).and_then(|b| b.models.first().map(|m| m.id.clone())).unwrap_or_default();
                call(engine, "chat.select_model", json!({"backend": backend_id, "model": first}))?;
            }
            if as_json {
                let out = call(engine, "chat.send", json!({"text": text, "files": files}))?;
                println!("{out}");
                return Ok(());
            }
            stream_answer(engine, text, files)?;
        }
        Cmd::List { limit } => {
            // The chats in use, as the sidebar lists them (not archived, not temporary; #77).
            for c in engine.state().listed_conversations().take(limit) {
                println!("{}  {}{}", c.id, if c.pinned { "📌 " } else { "" }, c.title);
            }
        }
        Cmd::Search { query } => {
            let hits = call(engine, "search", json!({"query": query.join(" ")}))?;
            for h in hits.as_array().cloned().unwrap_or_default() {
                println!(
                    "{}  {}\n    {}",
                    h.get("conversation_id").and_then(serde_json::Value::as_str).unwrap_or(""),
                    h.get("title").and_then(serde_json::Value::as_str).unwrap_or(""),
                    h.get("snippet").and_then(serde_json::Value::as_str).unwrap_or("").replace('\n', " ")
                );
            }
        }
        Cmd::Export { id, format } => {
            let out = call(engine, "chat.export", json!({"id": id, "format": format}))?;
            print!("{}", out.get("text").and_then(serde_json::Value::as_str).unwrap_or(""));
        }
        Cmd::Call { command, params } => {
            let params: Value = match params {
                Some(p) => serde_json::from_str(&p).map_err(|e| format!("Parameter sind kein JSON: {e}"))?,
                None => json!({}),
            };
            let out = call(engine, &command, params)?;
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        }
        Cmd::Commands => {
            for c in commands() {
                println!("{:<22} {:<42} {}", c.id, c.label, c.params);
            }
        }
        Cmd::Mcp | Cmd::Manual => {}
    }
    Ok(())
}

/// Send and print the answer as it streams in.
fn stream_answer(engine: &Engine, text: String, files: Vec<String>) -> Result<(), String> {
    let before = engine.state().messages.len();
    engine.dispatch("chat.send", json!({"text": text, "files": files}));
    let started = Instant::now();
    let mut printed = 0usize;
    let mut out = std::io::stdout();
    loop {
        let s = engine.state();
        let generating = s.current.as_ref().is_some_and(|c| s.is_generating(c));
        if let Some(m) = s.messages.get(before + 1) {
            if let Some(rest) = m.content.get(printed..) {
                let _ = out.write_all(rest.as_bytes());
                let _ = out.flush();
                printed = m.content.len();
            }
            if !generating {
                println!();
                if let Some(e) = &m.error {
                    return Err(e.clone());
                }
                return Ok(());
            }
        } else if !generating && started.elapsed() > Duration::from_secs(3) {
            // The send failed before an answer existed; the error is in the notices.
            let err = s.notices.iter().rev().find(|n| n.level == aiscraft_engine::NoticeLevel::Error).map(|n| n.text.clone());
            return Err(err.unwrap_or_else(|| "Keine Antwort".into()));
        }
        engine.wait_for(Duration::from_millis(200), |n| n.revision != s.revision);
    }
}

/// The MCP protocol versions the server answers with; an unknown one gets the first.
const MCP_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// Longest request line the MCP server reads. Attachments travel as paths, not bytes.
const MAX_MCP_MESSAGE: usize = 16 * 1024 * 1024;

/// The MCP server over stdio. Every engine command is a tool.
fn mcp(engine: &Engine) {
    serve_mcp(engine, std::io::stdin().lock(), std::io::stdout());
}

/// Newline-delimited JSON-RPC 2.0 until `input` closes. Framing, batches and the MCP lifecycle
/// come from craft-jsonrpc and craft-mcp; the tools are the engine's commands.
fn serve_mcp(engine: &Engine, mut input: impl BufRead, mut output: impl Write) {
    let version = env!("CARGO_PKG_VERSION");
    let mut session = craft_mcp::Session::new("aiscraft", version, craft_mcp::Policy::Compatible)
        .with_legacy_initialize(MCP_VERSIONS, json!({"name": "aiscraft", "title": "AIS.ChatCraft", "version": version}));
    session.set_instructions("AIS.ChatCraft: chat with AIS.chat and other backends, search and export chats. Call app_commands for the full list.".into());
    let session = std::cell::RefCell::new(session);
    let limits = craft_jsonrpc::Limits { max_message_bytes: MAX_MCP_MESSAGE, integer_ids_only: true, allow_null_ids: false, ..Default::default() };
    loop {
        let (reply, close) = match craft_jsonrpc::read_line(&mut input, MAX_MCP_MESSAGE) {
            Ok(None) | Err(craft_jsonrpc::LineError::Io(_)) => return,
            // The rest of the line is still unread: answer, then hang up.
            Err(craft_jsonrpc::LineError::TooLong) => {
                (Some(craft_jsonrpc::error_response(Value::Null, craft_jsonrpc::RpcError::new(-32600, "Anfrage zu groß"))), true)
            }
            Ok(Some(line)) if line.trim_ascii().is_empty() => continue,
            Ok(Some(line)) => {
                let reply = match craft_jsonrpc::parse_json(&line, limits) {
                    Err(_) => Some(craft_jsonrpc::parse_error_response()),
                    Ok(msg) => {
                        let in_batch = msg.is_array();
                        craft_jsonrpc::process(&msg, limits, |r| mcp_request(engine, &session, r, in_batch), |n| session.borrow_mut().notification(n))
                            .unwrap_or_else(|e| Some(craft_jsonrpc::error_response(Value::Null, craft_jsonrpc::RpcError::new(-32600, e.to_string()))))
                    }
                };
                (reply, false)
            }
        };
        if let Some(reply) = reply
            && (writeln!(output, "{reply}").is_err() || output.flush().is_err())
        {
            return;
        }
        if close {
            return;
        }
    }
}

fn mcp_request(
    engine: &Engine,
    session: &std::cell::RefCell<craft_mcp::Session>,
    r: craft_jsonrpc::Request<'_>,
    in_batch: bool,
) -> Result<Value, craft_jsonrpc::RpcError> {
    if let Some(answer) = session.borrow_mut().request(r, in_batch) {
        return answer;
    }
    let tool_name = |id: &str| id.replace('.', "_");
    match r.method {
        // Only commands an outside program may run (CommandSpec::external): an MCP client can be
        // steered by a prompt injection, so installs, switches, trust, secrets and deletions
        // stay inside the app.
        "tools/list" => Ok(craft_mcp::tool_list(json!(
            commands()
                .iter()
                .filter(|c| c.external())
                .map(|c| json!({
                    "name": tool_name(c.id),
                    "description": format!("{} ({}). Params: {}", c.label, c.group, if c.params.is_empty() { "none" } else { c.params }),
                    "inputSchema": {"type": "object", "additionalProperties": true}
                }))
                .collect::<Vec<_>>()
        ))),
        "tools/call" => {
            let (name, args) = craft_mcp::tool_call(r.params.unwrap_or(&Value::Null))?;
            let c = commands()
                .iter()
                .find(|c| tool_name(c.id) == name)
                .ok_or_else(|| craft_jsonrpc::RpcError::new(-32602, format!("Unbekanntes Werkzeug {name}")))?;
            if !c.external() {
                return Ok(craft_mcp::text_tool_result(&mcp_refusal(c.id), true));
            }
            Ok(match engine.call(c.id, args) {
                Ok(v) => craft_mcp::text_tool_result(&serde_json::to_string_pretty(&v).unwrap_or_default(), false),
                Err(e) => craft_mcp::text_tool_result(&e.message, true),
            })
        }
        other => Err(craft_jsonrpc::RpcError::new(-32601, format!("Methode {other} gibt es nicht"))),
    }
}

/// Why the MCP server does not run a command that stays inside the app.
fn mcp_refusal(id: &str) -> String {
    format!(
        "„{id}“ ist über den MCP-Server gesperrt: Installieren, Einschalten von Erweiterungen, Konnektoren und Werkzeugen, \
         Einstellungen, Vertrauensregeln, Freigaben, Geheimnisse und Löschen gehen nur in AIS.ChatCraft selbst."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(input: &str) -> Vec<Value> {
        let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
        let mut out = Vec::new();
        serve_mcp(&engine, input.as_bytes(), &mut out);
        String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn mcp_server_lists_and_calls_engine_commands() {
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"app_commands","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"chat_open","arguments":{"id":"gibt-es-nicht"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nichts"}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"resources/list"}"#,
        ]
        .join("\n");
        let replies = exchange(&input);
        assert_eq!(replies.len(), 7, "{replies:?}");
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(replies[0]["result"]["serverInfo"]["title"], "AIS.ChatCraft");
        assert!(replies[0]["result"]["instructions"].as_str().unwrap().contains("app_commands"));
        let tools = replies[1]["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), commands().iter().filter(|c| c.external()).count());
        assert!(tools.iter().any(|t| t["name"] == "chat_send"));
        assert_eq!(replies[2]["result"]["isError"], false);
        assert!(replies[2]["result"]["content"][0]["text"].as_str().unwrap().contains("chat.send"));
        assert_eq!(replies[3]["result"]["isError"], true);
        assert_eq!(replies[4]["error"]["code"], -32602);
        assert_eq!(replies[5]["result"], json!({}));
        assert_eq!(replies[6]["error"]["code"], -32601);
    }

    #[test]
    fn mcp_server_neither_lists_nor_runs_installs_switches_or_deletions() {
        let blocked = [
            ("extension_install", json!({"path": "x"})),
            ("h5p_player_install", json!({"yes": true})),
            ("privacy_ner_install", json!({"yes": true})),
            ("connector_add", json!({"name": "x", "command": "x"})),
            ("tool_answer", json!({"id": "x", "allow": true})),
            ("trust_rule_add", json!({"text": "x"})),
            ("settings_set", json!({"trust_check": false})),
            ("vault_add", json!({"name": "x", "value": "y"})),
            ("chat_set_sources", json!({"tools": true})),
            ("chat_purge", json!({"id": "x"})),
            ("sandbox_install", json!({"yes": true})),
        ];
        let mut input = vec![r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_string()];
        for (i, (name, args)) in blocked.iter().enumerate() {
            input.push(json!({"jsonrpc": "2.0", "id": i + 2, "method": "tools/call", "params": {"name": name, "arguments": args}}).to_string());
        }
        input.push(r#"{"jsonrpc":"2.0","id":100,"method":"tools/call","params":{"name":"app_state","arguments":{}}}"#.into());
        input.push(r#"{"jsonrpc":"2.0","id":101,"method":"tools/call","params":{"name":"chat_new","arguments":{}}}"#.into());
        let replies = exchange(&input.join("\n"));
        assert_eq!(replies.len(), blocked.len() + 3, "{replies:?}");
        let listed: Vec<&str> = replies[0]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for (i, (name, _)) in blocked.iter().enumerate() {
            assert!(!listed.contains(name), "{name} is listed");
            let reply = &replies[i + 1]["result"];
            assert_eq!(reply["isError"], true, "{name}: {reply}");
            assert!(reply["content"][0]["text"].as_str().unwrap().contains("über den MCP-Server gesperrt"), "{name}: {reply}");
        }
        assert!(listed.contains(&"chat_send") && listed.contains(&"search") && listed.contains(&"app_state"));
        let state = &replies[blocked.len() + 1]["result"];
        assert_eq!(state["isError"], false, "{state}");
        let new_chat = &replies[blocked.len() + 2]["result"];
        assert_eq!(new_chat["isError"], false, "{new_chat}");
    }

    #[test]
    fn mcp_server_answers_hostile_input_without_dispatching() {
        let replies = exchange(
            "kein json\n[]\n{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":7}\n\n",
        );
        assert_eq!(replies.len(), 4, "{replies:?}");
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["error"]["code"], -32600);
        assert_eq!(replies[2]["error"]["code"], -32600);
        assert_eq!(replies[3]["error"]["code"], -32602);
        let long = format!("{}\n{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}}\n", "x".repeat(MAX_MCP_MESSAGE + 1));
        let replies = exchange(&long);
        assert_eq!(replies.len(), 1, "an oversized line closes the connection");
        assert_eq!(replies[0]["error"]["code"], -32600);
    }

    #[test]
    fn mcp_server_handles_batches_and_unknown_versions() {
        let replies = exchange(
            r#"[{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":"b","method":"ping"}]"#,
        );
        let batch = replies[0].as_array().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0]["result"]["protocolVersion"], MCP_VERSIONS[0]);
        assert_eq!(batch[1]["id"], "b");
    }
}
