//! `aiscraft-cli`: the same engine and data as the desktop app, on the command line.

use std::io::{BufRead, Read, Write};
use std::time::{Duration, Instant};

use aiscraft_engine::{BackendStatus, Engine, Options, commands};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "aiscraft-cli", version, about = "AISCraft auf der Kommandozeile")]
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
}

fn main() {
    let cli = Cli::parse();
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
            let needs_secret = matches!(b.config.kind, BackendKind::AisChat | BackendKind::Anthropic);
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
            println!("Angemeldet: {}", [account["display_name"].as_str(), account["detail"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" · "));
        }
        Cmd::Status => {
            for b in &engine.state().backends {
                wait_backend(engine, &b.config.id);
            }
            for b in &engine.state().backends {
                let status = match &b.status {
                    BackendStatus::Ready { account } => format!("bereit  {}", [account.display_name.clone(), account.detail.clone(), account.expires.clone().map(|e| format!("bis {e}"))].into_iter().flatten().collect::<Vec<_>>().join(" · ")),
                    BackendStatus::SignedOut { message } => format!("abgemeldet  {message}"),
                    BackendStatus::Failed { message } => format!("Fehler  {message}"),
                    BackendStatus::Checking => "prüft …".into(),
                    BackendStatus::Unknown => if b.has_secret { "unbekannt".into() } else { "nicht angemeldet".into() },
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
            for c in engine.state().conversations.iter().take(limit) {
                println!("{}  {}{}", c.id, if c.pinned { "📌 " } else { "" }, c.title);
            }
        }
        Cmd::Search { query } => {
            let hits = call(engine, "search", json!({"query": query.join(" ")}))?;
            for h in hits.as_array().cloned().unwrap_or_default() {
                println!("{}  {}\n    {}", h["conversation_id"].as_str().unwrap_or(""), h["title"].as_str().unwrap_or(""), h["snippet"].as_str().unwrap_or("").replace('\n', " "));
            }
        }
        Cmd::Export { id, format } => {
            let out = call(engine, "chat.export", json!({"id": id, "format": format}))?;
            print!("{}", out["text"].as_str().unwrap_or(""));
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
        Cmd::Mcp => {}
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

/// A minimal MCP server (JSON-RPC 2.0 over stdio). Every engine command is a tool.
fn mcp(engine: &Engine) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(req) = serde_json::from_str::<Value>(&line) else { continue };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let result: Option<Result<Value, (i64, String)>> = match method {
            "initialize" => Some(Ok(json!({
                "protocolVersion": req.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "aiscraft", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "AISCraft: chat with AIS.chat and other backends, search and export chats. Call app_commands for the full list."
            }))),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(Ok(json!({}))),
            "tools/list" => Some(Ok(json!({"tools": commands().iter().map(|c| json!({
                "name": c.id.replace('.', "_"),
                "description": format!("{} ({}). Params: {}", c.label, c.group, if c.params.is_empty() { "none" } else { c.params }),
                "inputSchema": {"type": "object", "additionalProperties": true}
            })).collect::<Vec<_>>()}))),
            "tools/call" => {
                let name = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                match commands().iter().find(|c| c.id.replace('.', "_") == name) {
                    None => Some(Err((-32602, format!("Unbekanntes Werkzeug {name}")))),
                    Some(c) => Some(Ok(match engine.call(c.id, args) {
                        Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}]}),
                        Err(e) => json!({"content": [{"type": "text", "text": e.message}], "isError": true}),
                    })),
                }
            }
            _ if id.is_some() => Some(Err((-32601, format!("Methode {method} gibt es nicht")))),
            _ => None,
        };
        let (Some(id), Some(result)) = (id, result) else { continue };
        let resp = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        };
        if writeln!(stdout, "{resp}").is_err() || stdout.flush().is_err() {
            return;
        }
    }
}
