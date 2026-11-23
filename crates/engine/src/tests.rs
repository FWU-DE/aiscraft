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
                            .write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes())
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
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
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

    // Switch off, on, reconnect, remove.
    let id = engine.state().connectors[0].id.clone();
    engine.call("connector.toggle", json!({"id": id, "enabled": false})).unwrap();
    assert_eq!(engine.state().connectors[0].status, "off");
    assert!(engine.call("tools.list", json!({})).unwrap().as_array().unwrap().iter().all(|t| !t["name"].as_str().unwrap().ends_with("__echo")), "tools of an off connector are not offered");
    engine.call("connector.toggle", json!({"id": id, "enabled": true})).unwrap();
    assert_eq!(engine.state().connectors[0].status, "connected");
    engine.call("connector.refresh", json!({"id": id})).unwrap();
    assert_eq!(engine.state().connectors[0].status, "connected");
    engine.call("connector.remove", json!({"id": id})).unwrap();
    assert!(engine.state().connectors.is_empty());
}

/// A server that records every request body and answers like an OpenAI-compatible server.
fn recording_openai(engine: &Engine, answer: &'static str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    // Read headers, then exactly content-length bytes of body.
                    loop {
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..k]);
                        let s = String::from_utf8_lossy(&buf).to_string();
                        if let Some(h) = s.find("\r\n\r\n") {
                            let len = s[..h].lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    seen.lock().unwrap().push(req.clone());
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m","context_length":1000}]}"#;
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let c = json!({"choices":[{"delta":{"content": answer}}]});
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

fn engine_with_recording(answer: &'static str) -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = recording_openai(&engine, answer);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, seen)
}

/// Regression: an attached image must reach the model (it was dropped when the history was
/// reloaded from the database, which does not keep image bytes).
#[test]
fn attached_image_reaches_the_model() {
    let (engine, seen) = engine_with_recording("Ein Diagramm.");
    let dir = std::env::temp_dir().join(format!("aiscraft-img-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("tafel.png");
    std::fs::write(&img, [0x89u8, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4]).unwrap();
    let txt = dir.join("notizen.txt");
    std::fs::write(&txt, "Stundenziel: Brüche addieren").unwrap();
    engine.call("chat.send", json!({"text": "Was zeigt das Bild?", "files": [img.display().to_string(), txt.display().to_string()]})).unwrap();
    // The original may vanish; the copy keeps it available for the next turn.
    std::fs::remove_file(&img).unwrap();
    engine.call("chat.send", json!({"text": "Und nochmal?"})).unwrap();
    let reqs: Vec<String> = seen.lock().unwrap().iter().filter(|r| r.starts_with("POST")).cloned().collect();
    assert_eq!(reqs.len(), 2);
    for r in &reqs {
        assert!(r.contains("data:image/png;base64,iVBORw0KGgoBAgME"), "image missing in request: {}", &r[..r.len().min(400)]);
        assert!(r.contains("Stundenziel: Brüche addieren"), "text file missing");
    }
    let s = engine.state();
    assert_eq!(s.messages[0].attachments.len(), 2);
    assert!(s.messages[0].attachments[0].data_base64.is_none(), "bytes are not kept in state/db");
    let _ = std::fs::remove_dir_all(dir);
}

/// Documents: create from an answer, save, revise with AI, versions, restore, export.
#[test]
fn document_flow_end_to_end() {
    let (engine, seen) = engine_with_recording("# Plan\n\nKürzer.");
    engine.call("chat.send", json!({"text": "Mach einen Plan"})).unwrap();
    let mid = engine.state().messages[1].id.clone();
    let id = engine.call("artifact.create", json!({"kind": "document", "from_message": mid})).unwrap()["id"].as_str().unwrap().to_string();
    let s = engine.state();
    assert_eq!(s.open_artifact.as_deref(), Some(id.as_str()));
    assert_eq!(s.artifacts[0].title, "Plan");
    engine.call("artifact.save", json!({"id": id, "text": "# Plan\n\nLang und ausführlich."})).unwrap();
    assert_eq!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"], "# Plan\n\nLang und ausführlich.");
    let out = engine.call("artifact.revise", json!({"id": id, "instruction": "kürzer"})).unwrap();
    assert_eq!(out["text"], "# Plan\n\nKürzer.");
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("Lang und ausführlich") && last.contains("kürzer"), "revise sends the document and the instruction");
    let versions = engine.call("artifact.versions", json!({"id": id})).unwrap();
    let notes: Vec<&str> = versions.as_array().unwrap().iter().map(|v| v["note"].as_str().unwrap()).collect();
    assert!(notes.contains(&"Vor KI-Überarbeitung") && notes.iter().any(|n| n.starts_with("KI:")), "{notes:?}");
    let before = versions.as_array().unwrap().iter().find(|v| v["note"] == "Vor KI-Überarbeitung").unwrap()["version"].as_i64().unwrap();
    engine.call("artifact.restore", json!({"id": id, "version": before})).unwrap();
    assert_eq!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"], "# Plan\n\nLang und ausführlich.");
    let html = engine.call("artifact.export", json!({"id": id, "format": "html"})).unwrap();
    assert!(html["text"].as_str().unwrap().contains("<h1>Plan</h1>"));
    // As context for the next message.
    engine.call("chat.send", json!({"text": "Fasse zusammen", "artifact": id})).unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("Lang und ausführlich"), "open document goes along as context");
    // Boards from an outline, readable as text.
    let b = engine.call("artifact.create", json!({"kind": "board", "content": "# Wald\n- Bäume\n- Tiere"})).unwrap()["id"].as_str().unwrap().to_string();
    let text = engine.call("artifact.get", json!({"id": b})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("Bäume") && text.contains("Tiere"), "{text}");
    engine.call("artifact.delete", json!({"id": b})).unwrap();
    assert!(engine.call("artifact.get", json!({"id": b})).is_err());
}

/// Everything around chats, projects, memory and artifacts that is not sending.
#[test]
fn management_commands() {
    let (engine, seen) = engine_with_recording("Antwort");
    engine.call("chat.send", json!({"text": "Erste Frage"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.rename", json!({"id": conv, "title": "Umbenannt"})).unwrap();
    assert!(engine.call("chat.rename", json!({"id": conv, "title": "  "})).is_err());
    engine.call("chat.pin", json!({"id": conv, "pinned": true})).unwrap();
    let pid = engine.call("project.create", json!({"name": "Mathe"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("chat.move", json!({"id": conv, "project": pid})).unwrap();
    assert!(engine.call("chat.move", json!({"id": conv, "project": "nope"})).is_err());
    let s = engine.state();
    let c = s.conversation(&conv).unwrap();
    assert_eq!((c.title.as_str(), c.pinned, c.project_id.as_deref()), ("Umbenannt", true, Some(pid.as_str())));

    // Edit the first message: everything after it is replaced.
    let first = engine.state().messages[0].id.clone();
    engine.call("chat.edit", json!({"message_id": first, "text": "Geänderte Frage"})).unwrap();
    let s = engine.state();
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].content, "Geänderte Frage");
    assert!(seen.lock().unwrap().last().unwrap().contains("Geänderte Frage"));

    // Projects.
    engine.call("project.update", json!({"id": pid, "name": "Mathe 7", "instructions": "Kurz."})).unwrap();
    assert_eq!(engine.state().projects[0].instructions, "Kurz.");
    engine.call("project.delete", json!({"id": pid})).unwrap();
    assert!(engine.state().projects.is_empty());
    assert_eq!(engine.state().conversation(&conv).unwrap().project_id, None, "chats leave a deleted project");

    // Memory.
    let nid = engine.call("memory.add", json!({"text": "Ich mag Tabellen"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("memory.update", json!({"id": nid, "text": "Ich mag kurze Tabellen", "area": "preferences"})).unwrap();
    engine.call("memory.archive", json!({"id": nid})).unwrap();
    assert!(engine.state().memory[0].archived);
    engine.call("memory.restore", json!({"id": nid})).unwrap();
    let n = &engine.state().memory[0];
    assert!(!n.archived && n.text == "Ich mag kurze Tabellen" && n.area == aiscraft_core::MemoryArea::Preferences);

    // Artifacts: open, close, rename.
    let a = engine.call("artifact.create", json!({"kind": "document", "content": "# Alt"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("artifact.close", json!({})).unwrap();
    assert_eq!(engine.state().open_artifact, None);
    engine.call("artifact.open", json!({"id": a})).unwrap();
    assert_eq!(engine.state().open_artifact.as_deref(), Some(a.as_str()));
    engine.call("artifact.rename", json!({"id": a, "title": "Neu"})).unwrap();
    assert_eq!(engine.state().artifacts[0].title, "Neu");
    assert!(engine.call("artifact.open", json!({"id": "nope"})).is_err());

    // Generated documents from server activity can be saved.
    let mid = engine.state().messages[1].id.clone();
    engine.ctx().update(|s| {
        if let Some(m) = s.messages.iter_mut().find(|m| m.id == mid) {
            m.activity.push(aiscraft_core::ActivityStep {
                label: "Dokument erstellen".into(),
                document: Some(aiscraft_core::GeneratedDocument { file_name: "a.md".into(), format: "md".into(), content: "# Hallo".into() }),
                ..Default::default()
            });
        }
    });
    let path = std::env::temp_dir().join(format!("aiscraft-doc-{}.md", std::process::id()));
    engine.call("chat.save_document", json!({"message_id": mid, "step": 0, "path": path.display().to_string()})).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Hallo");
    let _ = std::fs::remove_file(path);

    // Search, notices, state, commands, tools.
    engine.call("search", json!({"query": "Geänderte"})).unwrap();
    assert!(engine.state().search.is_some());
    engine.call("search.clear", json!({})).unwrap();
    assert!(engine.state().search.is_none());
    let notice = engine.ctx().notice(crate::NoticeLevel::Error, "Test");
    engine.call("notice.dismiss", json!({"id": notice})).unwrap();
    assert!(engine.state().notices.iter().all(|n| n.id != notice));
    assert!(engine.call("app.state", json!({})).unwrap().get("conversations").is_some());
    assert!(engine.call("app.commands", json!({})).unwrap().as_array().unwrap().iter().any(|c| c["id"] == "chat.send"));
    let tools = engine.call("tools.list", json!({})).unwrap();
    assert!(tools.as_array().unwrap().iter().any(|t| t["name"] == "history_search"));

    // Delete the chat.
    engine.call("chat.delete", json!({"id": conv})).unwrap();
    assert!(engine.state().conversation(&conv).is_none() && engine.state().current.is_none());
}

/// Backend settings and sign-out.
#[test]
fn backend_update_and_logout() {
    let (engine, _) = engine_with_recording("x");
    let id = engine.state().selection.backend_id.clone();
    engine.call("backend.update", json!({"id": id, "name": "Umbenannt", "send_context": false, "default_model": "m"})).unwrap();
    let b = engine.state().backend(&id).unwrap().config.clone();
    assert_eq!((b.name.as_str(), b.send_context, b.default_model.as_deref()), ("Umbenannt", false, Some("m")));
    assert!(engine.call("backend.update", json!({"id": id, "base_url": "ftp://x"})).is_err(), "bad address rejected before saving");
    engine.call("backend.login", json!({"id": id, "secret": "sk-test"})).unwrap();
    assert!(engine.state().backend(&id).unwrap().has_secret);
    engine.call("backend.logout", json!({"id": id})).unwrap();
    assert!(!engine.state().backend(&id).unwrap().has_secret);
    engine.call("backend.remove", json!({"id": id})).unwrap();
    let s = engine.state();
    assert!(s.backend(&id).is_none());
    assert_eq!(s.selection.backend_id, "ais-chat", "selection falls back to a remaining backend");
}

/// AIS.chat through the engine: sign in with the session cookie, list and import the history.
#[test]
fn ais_chat_login_history_and_import() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let base = engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 16];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let authed = req.contains("ais-chat-app.authjs.session-token=good");
                    let (status, ctype, body) = if !authed {
                        ("307 Temporary Redirect\r\nlocation: /login", "text/plain", String::new())
                    } else if req.starts_with("GET /api/auth/session") {
                        ("200 OK", "application/json", r#"{"user":{"firstName":"Ada","lastName":"Lehrerin","school":{"name":"Gesamtschule"}},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("200 OK", "application/json", r#"{"conversations":[{"id":"r1","name":"Alter Chat","updatedAt":"2026-11-01"}]}"#.to_string())
                    } else if req.starts_with("GET /d/r1") {
                        let row = r#"5:{"messages":[{"id":"u1","role":"user","content":"Hallo AIS"},{"id":"a1","role":"assistant","content":"Hallo zurück"}]}"#;
                        ("200 OK", "text/html", format!("<script>self.__next_f.push([1,{}])</script>", serde_json::to_string(&format!("{row}\n")).unwrap()))
                    } else if req.starts_with("GET / ") {
                        let row = r#"3:{"models":[{"id":"m-uuid","name":"gpt","displayName":"GPT","priceMetadata":{"type":"text"}}]}"#;
                        ("200 OK", "text/html", format!("<script>self.__next_f.push([1,{}])</script>", serde_json::to_string(&format!("{row}\n")).unwrap()))
                    } else {
                        ("404 Not Found", "text/plain", String::new())
                    };
                    let _ = sock.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    });
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).ok();
    assert!(engine.call("backend.login", json!({"id": "ais-chat", "secret": "bad"})).is_err(), "a wrong cookie is an auth error");
    assert!(matches!(engine.state().backend("ais-chat").unwrap().status, crate::BackendStatus::SignedOut { .. }));
    let account = engine.call("backend.login", json!({"id": "ais-chat", "secret": "good"})).unwrap();
    assert_eq!(account["display_name"], "Ada Lehrerin");
    assert_eq!(engine.state().backend("ais-chat").unwrap().models[0].id, "m-uuid");
    let list = engine.call("remote.refresh", json!({"backend": "ais-chat"})).unwrap();
    assert_eq!(list[0]["title"], "Alter Chat");
    engine.call("remote.import", json!({"backend": "ais-chat", "id": "r1"})).unwrap();
    let s = engine.state();
    assert_eq!(s.current.as_deref(), Some("r1"), "the local id is the server's id");
    assert_eq!(s.conversation("r1").unwrap().title, "Alter Chat");
    assert_eq!(s.messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hallo AIS", "Hallo zurück"]);
}

/// Rule: every command has a functional test (beyond the panic hunt). A new command without one
/// fails here.
#[test]
fn every_command_has_a_functional_test() {
    let src = include_str!("tests.rs");
    // The panic hunt touches every command; count only calls outside it.
    let hunt_start = src.find("fn panic_hunt_every_command_with_hostile_params").unwrap_or(0);
    let hunt_end = src[hunt_start..].find("\n}\n").map(|e| hunt_start + e).unwrap_or(hunt_start);
    let outside = format!("{}{}", &src[..hunt_start], &src[hunt_end..]);
    let missing: Vec<&str> = crate::commands().iter().map(|c| c.id).filter(|id| !outside.contains(&format!("\"{id}\""))).collect();
    assert!(missing.is_empty(), "commands without a functional test: {missing:?}");
}
