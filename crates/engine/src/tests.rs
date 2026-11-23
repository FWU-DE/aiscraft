use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

/// A tiny OpenAI-compatible server: `/models` lists one model; `/chat/completions` streams
/// "Hallo Welt" (or, if the prompt contains "langsam", a slow stream that can be stopped).
fn scripted_openai(engine: &Engine) -> String {
    engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 16];
                    let mut n = 0;
                    // Read until the body is complete enough to route.
                    for _ in 0..50 {
                        let k = sock.read(&mut buf[n..]).await.unwrap_or(0);
                        n += k;
                        let s = String::from_utf8_lossy(&buf[..n]);
                        if k == 0 || s.starts_with("GET") && s.contains("\r\n\r\n") || s.contains("\"stream\"") {
                            break;
                        }
                    }
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    if req.starts_with("GET /v1/models") {
                        let body = r#"{"data":[{"id":"test-model"}]}"#;
                        let _ = sock
                            .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes())
                            .await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let slow = req.contains("langsam");
                    for word in ["Hallo", " Welt"] {
                        let chunk = json!({"choices":[{"delta":{"content":word}}]});
                        let _ = sock.write_all(format!("data: {chunk}\n\n").as_bytes()).await;
                        if slow {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                        }
                    }
                    let _ = sock.write_all(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

fn engine_with_backend() -> (Engine, String) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = scripted_openai(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    // Check synchronously instead of waiting for the background refresh: deterministic under load.
    engine.call("backend.check", json!({"id": id})).unwrap();
    assert!(!engine.state().backend(&id).unwrap().models.is_empty(), "models load");
    engine.call("chat.select_model", json!({"backend": id, "model": "test-model"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, id)
}

#[test]
fn default_ais_backend_exists_and_is_signed_out() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let s = engine.state();
    assert_eq!(s.backends[0].config.id, "ais-chat");
    assert!(!s.backends[0].has_secret);
    let err = engine.call("backend.login", json!({"id": "ais-chat", "secret": "foo=bar"})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Invalid);
}

#[test]
fn send_streams_saves_searches_and_regenerates() {
    let (engine, _) = engine_with_backend();
    let out = engine.call("chat.send", json!({"text": "Merke dir: ich unterrichte Biologie"})).unwrap();
    assert_eq!(out["text"], "Hallo Welt");
    let s = engine.state();
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[1].content, "Hallo Welt");
    assert_eq!(s.messages[1].usage.unwrap().output_tokens, 2);
    assert_eq!(s.conversations[0].title, "Merke dir: ich unterrichte Biologie");
    assert_eq!(s.memory.len(), 1, "explicit memory from the owner's words");
    assert_eq!(s.memory[0].text, "Ich unterrichte Biologie");
    assert_eq!(s.memory[0].source_message_id.as_deref(), Some(s.messages[0].id.as_str()));
    assert!(s.generating.is_empty());

    let hits = engine.call("search", json!({"query": "biolog"})).unwrap();
    assert_eq!(hits.as_array().unwrap().len(), 1);

    engine.call("chat.regenerate", json!({})).unwrap();
    assert_eq!(engine.state().messages.len(), 2);

    let md = engine.call("chat.export", json!({})).unwrap();
    assert!(md["text"].as_str().unwrap().contains("## Antwort\n\nHallo Welt"));

    // Reopen from the store.
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().messages.is_empty());
    engine.call("chat.open", json!({"id": conv})).unwrap();
    assert_eq!(engine.state().messages.len(), 2);
}

#[test]
fn stop_keeps_partial_answer() {
    let (engine, _) = engine_with_backend();
    engine.dispatch("chat.send", json!({"text": "bitte langsam"}));
    assert!(engine.wait_for(Duration::from_secs(30), |s| s.messages.get(1).is_some_and(|m| m.content == "Hallo")));
    engine.call("chat.stop", json!({})).unwrap();
    assert!(engine.wait_for(Duration::from_secs(30), |s| s.generating.is_empty()));
    assert_eq!(engine.state().messages[1].content, "Hallo");
}

#[test]
fn project_instructions_and_memory_reach_the_system_prompt() {
    let (engine, _) = engine_with_backend();
    let pid = engine.call("project.create", json!({"name": "Bio 7", "instructions": "Antworte für Klasse 7."})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("chat.new", json!({"project": pid})).unwrap();
    engine.call("memory.add", json!({"text": "Ich bin Lehrerin", "area": "about"})).unwrap();
    let conv = {
        engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
        engine.state().conversation(engine.state().current.as_deref().unwrap()).cloned().unwrap()
    };
    let backend = engine.ctx().backend(&conv.backend_id).unwrap();
    let sys = crate::chat_system_prompt_for_test(engine.ctx(), &conv, backend.config().send_context, backend.capabilities().system_prompt).unwrap();
    assert!(sys.contains("Antworte für Klasse 7."));
    assert!(sys.contains("Ich bin Lehrerin"));
}

/// Rule "never crash": every command, any params, returns Ok or Err but never panics.
#[test]
fn panic_hunt_every_command_with_hostile_params() {
    let (engine, _) = engine_with_backend();
    let hostile: Vec<Value> = vec![
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"id": "", "text": "", "query": "\"*(", "name": "", "kind": "nope", "key": "zoom", "value": "NaN"}),
        json!({"id": 42, "text": 7, "files": "notalist", "step": -1, "message_id": "", "path": "/nonexistent/dir/x"}),
        json!({"id": "ä".repeat(5000), "text": "\u{0}".repeat(100), "query": "ü".repeat(10_000), "backend": "nope", "model": ""}),
        json!({"key": "zoom", "value": 1e308}),
        json!({"files": ["/dev/null", "/etc", "/nonexistent"]}),
    ];
    for cmd in crate::commands() {
        if cmd.id == "chat.send" || cmd.id == "backend.remove" {
            continue; // covered elsewhere; would hit the network or drop the test backend
        }
        for p in &hostile {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.call(cmd.id, p.clone())));
            assert!(r.is_ok(), "{} panicked with {p}", cmd.id);
        }
    }
    // chat.send with hostile params, no network involved: bad files fail before sending.
    for p in &hostile {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.call("chat.send", json!({"text": "", "files": p.get("files").cloned().unwrap_or(json!([]))}))));
        assert!(r.is_ok());
    }
    assert!(engine.call("does.not.exist", json!({})).is_err());
}


/// The tool loop: the model asks for history_search, gets the result, then answers.
#[test]
fn tool_loop_with_builtin_history_search() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 17];
                    let mut n = 0;
                    loop {
                        let k = sock.read(&mut buf[n..]).await.unwrap_or(0);
                        n += k;
                        let s = String::from_utf8_lossy(&buf[..n]).to_string();
                        if k == 0 || (s.starts_with("GET") && s.contains("\r\n\r\n")) || s.contains("\"stream_options\"") {
                            break;
                        }
                    }
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    if req.contains("\"role\":\"tool\"") {
                        // Second round: the tool result is in the request; answer with it.
                        let found = req.contains("Fotosynthese");
                        let text = if found { "Ja, du hattest nach Fotosynthese gefragt." } else { "Nichts gefunden." };
                        let c = json!({"choices":[{"delta":{"content": text}}]});
                        let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                    } else if req.contains("history_search") && req.contains("Was hatten wir") {
                        let c1 = json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"history_search","arguments":"{\"query\":\"Fotosynthese\"}"}}]}}]});
                        let c2 = json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]});
                        let _ = sock.write_all(format!("data: {c1}\n\ndata: {c2}\n\ndata: [DONE]\n\n").as_bytes()).await;
                    } else {
                        let c = json!({"choices":[{"delta":{"content":"Okay."}}]});
                        let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                    }
                });
            }
        });
        format!("http://{addr}/v1")
    });
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    assert!(!engine.state().backend(&id).unwrap().models.is_empty());
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Erkläre Fotosynthese"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    let out = engine.call("chat.send", json!({"text": "Was hatten wir neulich besprochen?"})).unwrap();
    assert_eq!(out["text"], "Ja, du hattest nach Fotosynthese gefragt.");
    let s = engine.state();
    let roles: Vec<_> = s.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
    assert_eq!(s.messages[1].tool_calls[0].outcome.as_deref(), Some("ok"));
    assert!(s.messages[2].tool_result.as_ref().is_some_and(|r| !r.is_error));
}

/// Connectors and the approval rule, against the MCP crate's scripted server.
#[test]
fn connector_tools_need_approval_unless_read_only() {
    let bin = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/examples/fake_server");
    if !bin.exists() {
        eprintln!("skipped: build it with `cargo build -p aiscraft-mcp --example fake_server`");
        return;
    }
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("settings.set", json!({"key": "trust_check", "value": false})).unwrap();
    engine.call("connector.add", json!({"name": "Fake", "command": bin.display().to_string()})).unwrap();
    let s = engine.state();
    let conn = &s.connectors[0];
    assert_eq!(conn.status, "connected", "{:?}", conn.error);
    let echo = conn.tools.iter().find(|t| t.name == "echo").unwrap().clone();
    let notify = conn.tools.iter().find(|t| t.name == "notify").unwrap().clone();
    assert!(echo.read_only && !notify.read_only);
    let ctx = engine.ctx().clone();
    let call = |name: &str, args: serde_json::Value| aiscraft_core::ToolCall { id: "c".into(), name: name.into(), arguments: args, outcome: None };

    // Read-only: runs without asking.
    let (text, err, outcome) = engine.block_on(crate::tools::run(&ctx, "conv", &call(&echo.llm_name, json!({"text": "hallo"})), "", true));
    assert!(!err && outcome == "ok" && text.contains("hallo") && text.contains("untrusted_tool_result"), "{text}");

    // Not read-only, nobody there: refused, no question asked.
    let (_, err, outcome) = engine.block_on(crate::tools::run(&ctx, "conv", &call(&notify.llm_name, json!({})), "", false));
    assert!(err && outcome == "denied");
    assert!(engine.state().approvals.is_empty());

    // Not read-only, person there: asks; "no" means it does not run.
    for allow in [false, true] {
        let ctx2 = ctx.clone();
        let c2 = call(&notify.llm_name, json!({"x": 1}));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn({
            let engine = engine.clone();
            move || {
                let r = engine.block_on(crate::tools::run(&ctx2, "conv", &c2, "", true));
                let _ = tx.send(r);
            }
        });
        assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()));
        let a = engine.state().approvals[0].clone();
        assert_eq!(a.arguments, json!({"x": 1}));
        assert!(a.label.starts_with("Fake"));
        engine.call("tool.answer", json!({"id": a.id, "allow": allow})).unwrap();
        let (_, err, outcome) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
        if allow {
            assert_eq!(outcome, "ok", "allowed call runs");
            assert!(!err);
        } else {
            assert_eq!(outcome, "denied");
        }
        assert!(engine.state().approvals.is_empty());
    }
    // An answer for a question that is gone is an error, not a crash.
    assert!(engine.call("tool.answer", json!({"id": "nope", "allow": true})).is_err());
}
