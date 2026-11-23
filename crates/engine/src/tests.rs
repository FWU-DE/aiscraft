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
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
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
        // chat.send hits the network, backend.remove drops the test backend, and the browser
        // login must never be started by a test (in-memory engines refuse it anyway).
        if cmd.id == "chat.send" || cmd.id == "backend.remove" || cmd.id == "backend.browser_login" || cmd.id.starts_with("voice.") {
            continue; // covered elsewhere; would hit the network or drop the test backend
        }
        for p in &hostile {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.call(cmd.id, p.clone())));
            assert!(r.is_ok(), "{} panicked with {p}", cmd.id);
        }
    }
    // chat.send with hostile params, no network involved: bad files fail before sending.
    for p in &hostile {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.call("chat.send", json!({"text": "", "files": p.get("files").cloned().unwrap_or(json!([]))}))
        }));
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

    // Vault placeholders: filled in before the call, always asked, taken out of the result.
    engine
        .call("vault.add", json!({"name": "Schulportal", "username": "frau.ada", "password": "geheim-123", "totp": "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"}))
        .unwrap();
    assert_eq!(engine.state().vault[0].name, "schulportal");
    assert!(serde_json::to_string(&*engine.state()).unwrap().find("geheim-123").is_none(), "secrets never in state");
    let ctx2 = ctx.clone();
    let c3 = call(&echo.llm_name, json!({"text": "Login {{vault:schulportal.username}} / {{vault:schulportal.password}}"}));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn({
        let engine = engine.clone();
        move || {
            let _ = tx.send(engine.block_on(crate::tools::run(&ctx2, "conv", &c3, "", true)));
        }
    });
    assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()), "a read-only tool still asks when a secret goes out");
    let a = engine.state().approvals[0].clone();
    assert!(a.arguments.to_string().contains("{{vault:schulportal.password}}"), "the approval shows the placeholder, not the secret");
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && !text.contains("geheim-123") && !text.contains("frau.ada") && text.contains("[Tresor]"), "{text}");
    let bad = call(&echo.llm_name, json!({"text": "{{vault:gibtsnicht.password}}"}));
    let (t, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &bad, "", true));
    assert!(err && t.contains("gibtsnicht"));
    let code = engine.call("vault.code", json!({"id": engine.state().vault[0].id})).unwrap();
    assert_eq!(code["code"].as_str().unwrap().len(), 6);
    let csv = std::env::temp_dir().join(format!("bw-{}.csv", std::process::id()));
    std::fs::write(&csv, "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n,,login,Moodle,,,,https://moodle,ada,pw\"x,\n,,login,Schulportal,,,,,dup,dup,\n").unwrap();
    assert_eq!(engine.call("vault.import_bitwarden", json!({"path": csv.display().to_string()})).unwrap()["added"], 1, "duplicates are skipped");
    let _ = std::fs::remove_file(csv);
    let vid = engine.state().vault.iter().find(|v| v.name == "moodle").unwrap().id.clone();
    engine.call("vault.remove", json!({"id": vid})).unwrap();

    // Switch off, on, reconnect, remove.
    let id = engine.state().connectors[0].id.clone();
    engine.call("connector.toggle", json!({"id": id, "enabled": false})).unwrap();
    assert_eq!(engine.state().connectors[0].status, "off");
    assert!(
        engine.call("tools.list", json!({})).unwrap().as_array().unwrap().iter().all(|t| !t["name"].as_str().unwrap().ends_with("__echo")),
        "tools of an off connector are not offered"
    );
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
                            let len = s[..h]
                                .lines()
                                .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    seen.lock().unwrap().push(req.clone());
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m","context_length":1000}]}"#;
                        let _ = sock
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
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
    assert!(!last.contains("aiscraft-dokument"), "backends with tools get no change-block instruction");
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
                    let (status, ctype, body) = if req.starts_with("GET /login") {
                        ("200 OK\r\nset-cookie: __Secure-ais-chat-app.authjs.session-token=good; Path=/; Secure; HttpOnly", "text/html", "<p>angemeldet</p>".to_string())
                    } else if !authed {
                        ("307 Temporary Redirect\r\nlocation: /login", "text/plain", String::new())
                    } else if req.starts_with("GET /api/auth/session") {
                        ("200 OK", "application/json", r#"{"user":{"firstName":"Ada","lastName":"Lehrerin","school":{"name":"Gesamtschule"}},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("200 OK", "application/json", r#"{"conversations":[{"id":"r1","name":"Alter Chat","updatedAt":"2026-11-01"}]}"#.to_string())
                    } else if req.starts_with("GET /d/r1") {
                        let row = r#"5:{"messages":[{"id":"u1","role":"user","content":"Hallo AIS"},{"id":"a1","role":"assistant","content":"Hallo zurück"}]}"#;
                        ("200 OK", "text/html", format!("<script>self.__next_f.push([1,{}])</script>", serde_json::to_string(&format!("{row}\n")).unwrap()))
                    } else if req.starts_with("GET / ") {
                        let row = r#"3:{"usedBudget":250,"maxBudget":1000,"models":[{"id":"m-uuid","name":"gpt","displayName":"GPT","priceMetadata":{"type":"text"}}]}"#;
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
    // Engines in memory never start a browser: the login refuses instead of opening a window.
    // (The login itself is tested against a fake DevTools endpoint in aiscraft-backends, and
    // live, headless, with `cargo test -p aiscraft-backends -- --ignored real_browser_login`.)
    let err = engine.call("backend.browser_login", json!({"id": "ais-chat"})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Unsupported);
    let account = engine.call("backend.login", json!({"id": "ais-chat", "secret": "good"})).unwrap();
    assert_eq!(account["display_name"], "Ada Lehrerin");
    assert_eq!(engine.state().backend("ais-chat").unwrap().models[0].id, "m-uuid");
    let q = engine.state().backend("ais-chat").unwrap().quota.unwrap();
    assert_eq!((q.used, q.max), (250.0, 1000.0));
    assert!((q.left() - 0.75).abs() < 1e-9);
    let list = engine.call("remote.refresh", json!({"backend": "ais-chat"})).unwrap();
    assert_eq!(list[0]["title"], "Alter Chat");
    engine.call("remote.import", json!({"backend": "ais-chat", "id": "r1"})).unwrap();
    let s = engine.state();
    assert_eq!(s.current.as_deref(), Some("r1"), "the local id is the server's id");
    assert_eq!(s.conversation("r1").unwrap().title, "Alter Chat");
    assert_eq!(s.messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Hallo AIS", "Hallo zurück"]);
}

/// Read one HTTP request, head and body (by content-length).
async fn read_http(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = sock.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let s = String::from_utf8_lossy(&buf);
        if let Some(head_end) = s.find("\r\n\r\n") {
            let len = s[..head_end]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() >= head_end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// A scripted AIS.chat for assistants, characters and scenarios (and later features): pages
/// ship their own JS chunks with server action ids; POSTs to "/" are answered by action id.
/// Every POST is recorded.
fn scripted_ais(engine: &Engine) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let posted = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let rec = posted.clone();
    let base = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let rec = rec.clone();
                tokio::spawn(async move {
                    let req = read_http(&mut sock).await;
                    let id = |n: u8| format!("{n:02x}").repeat(21);
                    let page = |chunk: &str, row: &str| {
                        let payload = serde_json::to_string(&format!("{row}\n")).unwrap();
                        format!(r#"<script src="/_next/static/chunks/{chunk}.js"></script><script>self.__next_f.push([1,{payload}])</script>"#)
                    };
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    // Static chunks need no session, everything else does.
                    let authed = req.contains("ais-chat-app.authjs.session-token=good") || req.starts_with("GET /_next/") || req.starts_with("GET /avatars/");
                    let (status, ctype, body) = if !authed {
                        ("307 Temporary Redirect\r\nlocation: /login", "text/plain", String::new())
                    } else if req.starts_with("GET /api/auth/session") {
                        ("200 OK", "application/json", r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin"},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("200 OK", "application/json", r#"{"conversations":[{"id":"r2","name":"Mit Goethe","characterId":"c1","type":"chat"}]}"#.to_string())
                    } else if req.starts_with("GET /d/r2") || req.starts_with("GET /characters/d/c1/r2") {
                        ("200 OK", "text/html", page("none", r#"5:{"messages":[{"id":"u9","role":"user","content":"Hallo Goethe"},{"id":"a9","role":"assistant","content":"Seid gegrüßt!"}]}"#))
                    } else if req.starts_with("GET /assistants/editor/a1 ") {
                        ("200 OK", "text/html", page("editor", "1:{}"))
                    } else if req.starts_with("GET /_next/static/chunks/editor.js") {
                        ("200 OK", "application/javascript", reference(0x55, "ingestWebContentAction"))
                    } else if req.starts_with("GET /image-generation ") {
                        ("200 OK", "text/html", page("images", r#"4:{"models":[{"id":"m-img","name":"img","displayName":"Bildmodell","priceMetadata":{"type":"image"}}]}"#))
                    } else if req.starts_with("GET /_next/static/chunks/images.js") {
                        ("200 OK", "application/javascript", reference(0x44, "generateImageAction"))
                    } else if req.starts_with("GET /avatars/") && req.to_ascii_lowercase().contains("\r\ncookie:") {
                        // Signed storage URLs never get the session cookie.
                        ("403 Forbidden", "text/plain", String::new())
                    } else if req.starts_with("GET /avatars/goethe.png?X-Amz-Signature=") {
                        ("200 OK", "image/gif", "GIF89aGoethe".to_string())
                    } else if req.starts_with("GET /avatars/coach.png") {
                        ("200 OK", "text/html", "<html>kein Bild</html>".to_string())
                    } else if req.starts_with("GET /avatars/riesig.png") {
                        ("200 OK", "image/gif", format!("GIF89a{}", "x".repeat(3 * 1024 * 1024)))
                    } else if req.starts_with("GET /files/bild.png") {
                        ("200 OK", "image/png", "\u{89}PNG-bytes".to_string())
                    } else if req.starts_with("GET /assistants ") {
                        ("200 OK", "text/html", page("assistants", "1:{}"))
                    } else if req.starts_with("GET /characters ") {
                        ("200 OK", "text/html", page("characters", "1:{}"))
                    } else if req.starts_with("GET /learning-scenarios ") {
                        ("200 OK", "text/html", page("scenarios", "1:{}"))
                    } else if req.starts_with("GET / ") {
                        ("200 OK", "text/html", page("app", r#"3:{"usedBudget":10,"maxBudget":100,"models":[{"id":"m-uuid","name":"gpt","displayName":"GPT","priceMetadata":{"type":"text"}},{"id":"m-other","name":"other","displayName":"Anderes","priceMetadata":{"type":"text"}}]}"#))
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        ("200 OK", "application/javascript", reference(0x7f, "sendChatMessageAction"))
                    } else if req.starts_with("GET /_next/static/chunks/assistants.js") {
                        ("200 OK", "application/javascript", reference(0x11, "getAssistantsByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/characters.js") {
                        ("200 OK", "application/javascript", reference(0x22, "getCharactersByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/scenarios.js") {
                        ("200 OK", "application/javascript", reference(0x33, "getLearningScenariosByFilterAction"))
                    } else if req.starts_with("POST / ") {
                        rec.lock().unwrap().push(req.clone());
                        let body = if action == id(0x11) {
                            flight(r#"[{"id":"a1","name":"Mathe-Coach","userId":"me","description":"Übt Bruchrechnung","promptSuggestions":["Erkläre 3/4"],"maybeSignedPictureUrl":"https://s3.example/avatars/coach.png"}]"#)
                        } else if action == id(0x22) {
                            flight(r#"[{"id":"c1","name":"Goethe","userId":"x","description":"Dichter aus Weimar","initialMessage":"Seid gegrüßt!","modelId":"m-other","isFavorite":true,"maybeSignedPictureUrl":"https://s3.example/avatars/goethe.png?X-Amz-Signature=abc"}]"#)
                        } else if action == id(0x33) {
                            flight(r#"[{"id":"l1","name":"Photosynthese erforschen","userId":"me","studentExercise":"Finde heraus, was Pflanzen brauchen.","maybeSignedPictureUrl":"https://s3.example/avatars/riesig.png"}]"#)
                        } else if action == id(0x55) {
                            if req.contains("kaputt") {
                                flight(r#"{"success":true,"value":{"processedUrls":[],"errorUrls":["https://kaputt.example/"]}}"#)
                            } else {
                                flight(r#"{"success":true,"value":{"processedUrls":["https://schule.example/plan"],"errorUrls":[]}}"#)
                            }
                        } else if action == id(0x44) {
                            if req.contains("Fehler bitte") {
                                flight(r#"{"success":false,"error":{"name":"TokenPointsExceededError","message":"leer"}}"#)
                            } else {
                                flight(r#"{"success":true,"value":{"imageUrl":"/files/bild.png","conversationId":"srv-img","fileId":"file_1","assistantMessageId":"am1","userMessageId":"um1"}}"#)
                            }
                        } else if action == id(0x7f) {
                            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{{\"stream\":\"$2\",\"messageId\":\"srv-{n}\"}}\n2:R\n2:\"Ich bin Goethe.\"\n2:C\n")
                        } else {
                            String::new()
                        };
                        if body.is_empty() { ("404 Not Found", "text/plain", body) } else { ("200 OK", "text/x-component", body) }
                    } else {
                        ("404 Not Found", "text/plain", String::new())
                    };
                    let _ = sock.write_all(format!("HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    });
    (base, posted)
}

fn ais_engine() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (base, posted) = scripted_ais(&engine);
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).ok();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "good"})).unwrap();
    engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m-uuid"})).unwrap();
    (engine, posted)
}

/// Assistants, characters and learning scenarios: list, start a chat with one (the greeting
/// comes first, the send names it), reopen it, import one from the server history.
#[test]
fn ais_chat_personas() {
    let (engine, posted) = ais_engine();
    let list = engine.call("persona.refresh", json!({"backend": "ais-chat"})).unwrap();
    let names: Vec<_> = list.as_array().unwrap().iter().map(|p| (p["kind"].as_str().unwrap().to_string(), p["name"].as_str().unwrap().to_string())).collect();
    assert_eq!(names.len(), 3, "{names:?}");
    assert!(names.contains(&("learning_scenario".into(), "Photosynthese erforschen".into())));
    assert_eq!(engine.state().backend("ais-chat").unwrap().personas.as_ref().unwrap().len(), 3);

    let info = engine.call("persona.start", json!({"kind": "character", "id": "c1"})).unwrap();
    assert_eq!(info["initial_message"], "Seid gegrüßt!");
    let s = engine.state();
    assert_eq!(s.selection.persona.as_ref().map(|p| p.name.as_str()), Some("Goethe"));
    assert_eq!(s.selection.model_id, "m-other", "the character's own model");
    assert!(s.current.is_none());

    engine.call("chat.send", json!({"text": "Wer bist du?"})).unwrap();
    let s = engine.state();
    let conv = s.conversation(s.current.as_deref().unwrap()).unwrap().clone();
    assert_eq!(conv.persona.as_ref().map(|p| p.id.as_str()), Some("c1"));
    assert_eq!(s.messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["Seid gegrüßt!", "Wer bist du?", "Ich bin Goethe."]);
    let sent = posted.lock().unwrap().last().cloned().unwrap();
    assert!(sent.contains(r#""characterId":"c1""#), "{sent}");
    assert!(!sent.contains("assistantId"), "{sent}");

    // A plain new chat forgets the persona; reopening the chat brings it back.
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().selection.persona.is_none());
    engine.call("chat.open", json!({"id": conv.id})).unwrap();
    assert_eq!(engine.state().selection.persona, conv.persona);

    // An assistant chat sends assistantId.
    engine.call("persona.start", json!({"kind": "assistant", "id": "a1"})).unwrap();
    engine.call("chat.send", json!({"text": "3/4?"})).unwrap();
    assert!(posted.lock().unwrap().last().unwrap().contains(r#""assistantId":"a1""#));

    // From the server history: the character chat keeps its character.
    engine.call("remote.refresh", json!({"backend": "ais-chat"})).unwrap();
    engine.call("remote.import", json!({"backend": "ais-chat", "id": "r2"})).unwrap();
    let p = engine.state().conversation("r2").unwrap().persona.clone().unwrap();
    assert_eq!((p.kind, p.id.as_str(), p.name.as_str()), (aiscraft_core::PersonaKind::Character, "c1", "Goethe"));

    let err = engine.call("persona.start", json!({"kind": "character", "id": "weg"})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::NotFound);
    assert!(engine.call("persona.start", json!({"kind": "robot", "id": "c1"})).is_err());
    // Backends without personas say so.
    let (other, id) = engine_with_backend();
    assert_eq!(other.call("persona.refresh", json!({"backend": id})).unwrap_err().kind, aiscraft_core::ErrorKind::Unsupported);
}

/// Pictures: fetched from the signed URL (through the local origin, without the session
/// cookie), cached by id, refused when too large or not an image; no network without an origin.
#[test]
fn ais_chat_persona_pictures() {
    let (engine, _) = ais_engine();
    let base = engine.state().backend("ais-chat").unwrap().config.base_url.clone();
    engine.call("persona.refresh", json!({"backend": "ais-chat"})).unwrap();
    let goethe = engine.state().backend("ais-chat").unwrap().personas.as_ref().unwrap().iter().find(|p| p.id == "c1").cloned().unwrap();
    assert!(goethe.favorite);
    // An engine in memory has no origin yet: it stays offline.
    let err = engine.call("persona.picture", json!({"kind": "character", "id": "c1"})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Unsupported);
    assert!(engine.set_picture_origin(Some("https://elsewhere.example")).is_err(), "only a local server");
    engine.set_picture_origin(Some(&base)).unwrap();

    let first = engine.call("persona.picture", json!({"backend": "ais-chat", "kind": "character", "id": "c1"})).unwrap();
    assert_eq!(first["cached"], false, "{first}");
    assert_eq!(first["content_type"], "image/gif");
    let path = first["path"].as_str().unwrap().to_string();
    assert!(path.starts_with(&engine.pictures_dir().display().to_string()));
    assert_eq!(std::fs::read(&path).unwrap(), b"GIF89aGoethe");
    let key = crate::picture_key("ais-chat", aiscraft_core::PersonaKind::Character, "c1");
    assert_eq!(engine.state().persona_pictures.get(&key).cloned().flatten().as_deref(), Some(path.as_str()));
    assert_eq!(engine.call("persona.picture", json!({"kind": "character", "id": "c1"})).unwrap()["cached"], true, "second time from the cache");
    assert_eq!(engine.call("persona.picture", json!({"kind": "character", "id": "c1", "fresh": true})).unwrap()["cached"], false);

    // Not an image, too large: errors, nothing cached.
    let err = engine.call("persona.picture", json!({"kind": "assistant", "id": "a1"})).unwrap_err();
    assert!(err.message.contains("Kein Bild"), "{}", err.message);
    let err = engine.call("persona.picture", json!({"kind": "learning_scenario", "id": "l1"})).unwrap_err();
    assert!(err.message.contains("2 MB"), "{}", err.message);
    let s = engine.state();
    assert_eq!(s.persona_pictures.get(&crate::picture_key("ais-chat", aiscraft_core::PersonaKind::LearningScenario, "l1")), Some(&None));
    // Unknown ones and bad kinds.
    assert_eq!(engine.call("persona.picture", json!({"kind": "character", "id": "weg"})).unwrap_err().kind, aiscraft_core::ErrorKind::NotFound);
    assert!(engine.call("persona.picture", json!({"kind": "robot", "id": "c1"})).is_err());
    let _ = std::fs::remove_file(&path);
}

/// A suggestion click: start the persona chat and send the text right away.
#[test]
fn persona_start_with_text_sends() {
    let (engine, posted) = ais_engine();
    engine.call("persona.start", json!({"kind": "character", "id": "c1", "text": "Wer bist du?"})).unwrap();
    let s = engine.state();
    assert!(s.current.is_some(), "the chat exists");
    assert_eq!(s.messages.last().map(|m| m.content.as_str()), Some("Ich bin Goethe."));
    assert!(posted.lock().unwrap().last().unwrap().contains(r#""characterId":"c1""#));
}

/// Image chats: the AIS.chat image models appear, a prompt becomes an image attachment that can
/// be saved, the next prompt continues the server's image chat, errors land on the answer.
#[test]
fn ais_chat_image_generation() {
    let (engine, posted) = ais_engine();
    let s = engine.state();
    let b = s.backend("ais-chat").unwrap();
    assert_eq!(b.image_models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m-img"]);
    assert!(!b.models.iter().any(|m| m.id == "m-img"), "image models are not text models");

    engine.call("chat.new_image", json!({})).unwrap();
    assert_eq!(engine.state().selection.kind, aiscraft_core::ConversationKind::Image);
    assert_eq!(engine.state().selection.model_id, "m-img");
    assert!(engine.call("chat.set_aspect", json!({"ratio": "rund"})).is_err());
    engine.call("chat.set_aspect", json!({"ratio": "landscape"})).unwrap();
    let out = engine.call("chat.send", json!({"text": "Ein Fuchs im Schnee"})).unwrap();
    assert_eq!(out["images"], 1);
    let s = engine.state();
    let conv = s.conversation(s.current.as_deref().unwrap()).unwrap().clone();
    assert_eq!(conv.kind, aiscraft_core::ConversationKind::Image);
    assert_eq!(conv.remote_id.as_deref(), Some("srv-img"), "the server picked its own id");
    let answer = s.messages.last().unwrap();
    let img = &answer.attachments[0];
    assert_eq!(img.mime, "image/png");
    assert_eq!(std::fs::read(img.local_path.as_ref().unwrap()).unwrap(), "\u{89}PNG-bytes".as_bytes());
    let first = posted.lock().unwrap().last().cloned().unwrap();
    assert!(first.contains(r#""aspectRatio":"landscape""#) && first.contains(r#""modelId":"m-img""#) && !first.contains("conversationId"), "{first}");

    let target = std::env::temp_dir().join(format!("aiscraft-bild-{}.png", std::process::id()));
    engine.call("chat.save_image", json!({"message_id": answer.id, "path": target.display().to_string()})).unwrap();
    assert_eq!(std::fs::read(&target).unwrap().len(), img.size as usize);
    let _ = std::fs::remove_file(&target);
    assert!(engine.call("chat.save_image", json!({"message_id": answer.id, "index": 3, "path": "/tmp/x.png"})).is_err());

    // The next prompt continues the server's image chat; regenerate draws again.
    engine.call("chat.send", json!({"text": "jetzt mit Mond", "aspect_ratio": "portrait"})).unwrap();
    let second = posted.lock().unwrap().last().cloned().unwrap();
    assert!(second.contains(r#""conversationId":"srv-img""#) && second.contains("portrait"), "{second}");
    engine.call("chat.regenerate", json!({})).unwrap();
    assert_eq!(engine.state().messages.len(), 4);
    assert!(engine.call("chat.send", json!({"text": "x", "files": ["/etc/hostname"]})).is_err(), "no input images yet");

    // A refusal from the server is shown on the answer.
    assert!(engine.call("chat.send", json!({"text": "Fehler bitte"})).is_err());
    assert!(engine.state().messages.last().unwrap().error.as_deref().unwrap_or("").contains("Kontingent"));

    // Text models stay out of image chats and the other way round.
    assert!(engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m-uuid"})).is_err());
    engine.call("chat.new", json!({})).unwrap();
    assert_eq!(engine.state().selection.kind, aiscraft_core::ConversationKind::Chat);
    engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m-img"})).unwrap();
    assert_eq!(engine.state().selection.kind, aiscraft_core::ConversationKind::Image, "picking an image model in a new chat makes it an image chat");
    // Backends without image models refuse image chats.
    let (other, id) = engine_with_backend();
    assert_eq!(other.call("chat.new_image", json!({"backend": id})).unwrap_err().kind, aiscraft_core::ErrorKind::Unsupported);
    assert!(other.call("chat.new", json!({"kind": "video"})).is_err());
}

/// Web links on a message: AIS.chat reads the page first where it can (ingestWebContentAction,
/// shipped with the editor pages of the person's own assistants), the link goes along in the
/// text for its web scraper; other backends get it as text too.
#[test]
fn links_on_messages() {
    let (engine, posted) = ais_engine();
    // Not checkable yet (no own assistant known): the link goes along unchecked.
    engine.call("chat.send", json!({"text": "Fasse zusammen", "links": ["https://schule.example/plan"]})).unwrap();
    let sent = posted.lock().unwrap().last().cloned().unwrap();
    assert!(sent.contains(r#""content":"Fasse zusammen\n\nhttps://schule.example/plan""#), "{sent}");
    let user = engine.state().messages.iter().find(|m| m.role == aiscraft_core::Role::User).cloned().unwrap();
    assert!(user.attachments[0].is_link() && user.attachments[0].name == "https://schule.example/plan");

    // With an own assistant, the editor page carries the ingest action: a page AIS.chat cannot
    // read is refused before anything is sent.
    engine.call("persona.refresh", json!({})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    let before = posted.lock().unwrap().len();
    let err = engine.call("chat.send", json!({"text": "Lies das", "links": ["https://kaputt.example/"]})).unwrap_err();
    assert!(err.message.contains("nicht lesen"), "{}", err.message);
    assert!(engine.state().current.is_none(), "nothing was saved");
    engine.call("chat.send", json!({"text": "Lies das", "links": ["https://schule.example/plan"]})).unwrap();
    let posts = posted.lock().unwrap()[before..].to_vec();
    assert!(posts.iter().any(|p| p.contains(r#"[{"url":"https://schule.example/plan"}]"#)), "ingest was asked: {posts:?}");

    for bad in [json!(["ftp://x.example"]), json!(["https://"]), json!(["https://a b"]), json!(vec!["https://x.example"; 6])] {
        assert_eq!(engine.call("chat.send", json!({"text": "x", "links": bad})).unwrap_err().kind, aiscraft_core::ErrorKind::Invalid, "{bad}");
    }

    // Other backends read the address in the text.
    let (other, seen) = engine_with_recording("Ok.");
    other.call("chat.send", json!({"text": "", "links": ["https://schule.example/plan"]})).unwrap();
    assert!(seen.lock().unwrap().iter().any(|r| r.contains("Link: https://schule.example/plan")));
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

/// Schedules: create, run, watch without change, change, delete; check-ins from the model.
#[test]
fn schedules_run_watch_and_check_in() {
    let (engine, seen) = engine_with_recording("Heute: Mathe, Deutsch.");
    let created = engine.call("schedule.create", json!({"title": "Morgenplan", "prompt": "Was steht heute an?", "cron": "0 7 * * 1-5"})).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert!(created["next_run"].as_i64().is_some());
    assert!(engine.call("schedule.create", json!({"title": "x", "prompt": "y", "cron": "99 * * * *"})).is_err());
    assert!(engine.call("schedule.create", json!({"title": "x", "prompt": "y"})).is_err(), "cron or at required");
    assert!(engine.call("schedule.create", json!({"title": "x", "prompt": "y", "at": "irgendwann"})).is_err());
    let out = engine.call("schedule.run", json!({"id": id})).unwrap();
    assert_eq!(out["text"], "Heute: Mathe, Deutsch.");
    let s = engine.state();
    let sched = s.schedules.iter().find(|x| x.id == id).unwrap().clone();
    let conv = sched.conversation_id.clone().unwrap();
    assert!(s.conversation(&conv).unwrap().title.contains("Morgenplan"));
    assert_eq!(engine.ctx().store(|st| st.messages(&conv)).unwrap().len(), 2);
    assert!(seen.lock().unwrap().last().unwrap().contains("Was steht heute an?"));
    engine.call("schedule.update", json!({"id": id, "enabled": false})).unwrap();
    assert!(!engine.state().schedules[0].enabled);
    engine.call("schedule.delete", json!({"id": id})).unwrap();
    assert!(engine.state().schedules.is_empty());

    // A watch whose answer is "no change" leaves no trace in its chat.
    let (engine2, _) = engine_with_recording(crate::schedules::NO_CHANGE);
    let w =
        engine2.call("schedule.create", json!({"title": "Vertretungsplan", "prompt": "Prüfe den Plan", "cron": "*/30 * * * *", "watch": true})).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
    engine2.call("schedule.run", json!({"id": w})).unwrap();
    let conv2 = engine2.state().schedules[0].conversation_id.clone().unwrap();
    assert!(engine2.ctx().store(|st| st.messages(&conv2)).unwrap().is_empty(), "unchanged watch runs are removed");
}

#[test]
fn check_in_tool_answers_in_the_same_chat() {
    let (engine, _) = engine_with_recording("ok");
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    let call = aiscraft_core::ToolCall {
        id: "t".into(),
        name: "schedule_create".into(),
        arguments: json!({"title": "Nachfragen", "prompt": "Hat Frau Yilmaz geantwortet?", "at": "in 2 Tagen"}),
        outcome: None,
    };
    let (text, err, _) = engine.block_on(crate::tools::run(engine.ctx(), &conv, &call, "", true));
    assert!(!err && text.starts_with("Geplant für"), "{text}");
    let s = engine.state();
    assert_eq!(s.schedules[0].conversation_id.as_deref(), Some(conv.as_str()));
    assert!(s.schedules[0].run_at.is_some());
}

/// The usage ledger and the monthly limit.
#[test]
fn usage_report_and_monthly_limit() {
    let (engine, _) = engine_with_recording("ok");
    let mut m = engine.call("chat.send", json!({"text": "eins"})).unwrap();
    // The recording server sends no usage; give the answer some, as a real server would.
    let mid = m["message_id"].take();
    let conv = engine.state().current.clone().unwrap();
    let mut msgs = engine.ctx().store(|st| st.messages(&conv)).unwrap();
    let a = msgs.iter_mut().find(|x| json!(x.id) == mid).unwrap();
    a.usage = Some(aiscraft_core::Usage { input_tokens: 900, output_tokens: 200 });
    engine.ctx().store(|st| st.save_message(a)).unwrap();
    let r = engine.call("usage.report", json!({})).unwrap();
    let row = &r["rows"][0];
    assert_eq!((row["input_tokens"].as_i64(), row["output_tokens"].as_i64()), (Some(900), Some(200)));
    assert_eq!(row["month"], r["this_month"]);
    engine.call("settings.set", json!({"key": "monthly_token_limit", "value": 1000})).unwrap();
    let err = engine.call("chat.send", json!({"text": "zwei"})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Refused);
    assert!(engine.state().notices.iter().any(|n| n.action.as_ref().is_some_and(|a| a.params["force"] == true)), "offers to send anyway");
    engine.call("chat.send", json!({"text": "zwei", "force": true})).unwrap();
}

/// Per-chat sources: memory, project and tools follow the chat's switches.
#[test]
fn per_chat_sources() {
    let (engine, seen) = engine_with_recording("ok");
    let pid = engine.call("project.create", json!({"name": "P", "instructions": "PROJEKTREGEL"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("memory.add", json!({"text": "MERKNOTIZ"})).unwrap();
    engine.call("chat.new", json!({"project": pid})).unwrap();
    // Switched before the first message: the new chat takes them over.
    engine.call("chat.set_sources", json!({"memory": false, "history": false})).unwrap();
    engine.call("chat.send", json!({"text": "eins"})).unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("PROJEKTREGEL") && !last.contains("MERKNOTIZ"), "memory off, project on");
    assert!(last.contains("document_create") && !last.contains("history_search"), "history search off, other tools on");
    let conv = engine.state().current.clone().unwrap();
    assert!(!engine.state().conversation(&conv).unwrap().sources.memory);
    engine.call("chat.set_sources", json!({"tools": false, "project": false, "memory": true})).unwrap();
    engine.call("chat.send", json!({"text": "zwei"})).unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(!last.contains("\"tools\"") && !last.contains("PROJEKTREGEL") && last.contains("MERKNOTIZ"));
    // A model calling history_search anyway is refused.
    let call = aiscraft_core::ToolCall { id: "h".into(), name: "history_search".into(), arguments: json!({"query": "x"}), outcome: None };
    let (_, err, _) = engine.block_on(crate::tools::run(engine.ctx(), &conv, &call, "", true));
    assert!(err);
    // A new chat starts with everything on again; reopening brings the chat's own switches back.
    engine.call("chat.new", json!({})).unwrap();
    assert_eq!(engine.state().selection.sources, aiscraft_core::Sources::default());
    engine.call("chat.open", json!({"id": conv})).unwrap();
    assert!(!engine.state().selection.sources.tools);
    assert!(engine.call("chat.set_sources", json!({"connectors": "alle"})).is_err());
}

/// Skills: created by hand, fitting ones reach the context; learned from tool-heavy chats.
#[test]
fn skills_recall_and_learning() {
    let (engine, seen) = engine_with_recording(
        r#"{"title": "Elternbrief schreiben", "when": "Wenn ein Elternbrief gebraucht wird", "steps": "1. Anlass klären\n2. Entwurf als Dokument"}"#,
    );
    let id = engine
        .call("skill.create", json!({"title": "Arbeitsblatt erstellen", "when": "Arbeitsblatt für eine Klasse", "steps": "1. Ziel\n2. Aufgaben\n3. Lösungen"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("chat.send", json!({"text": "Erstelle ein Arbeitsblatt zur Photosynthese"})).unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("Gelernte Vorgehen") && last.contains("Arbeitsblatt erstellen"), "fitting skill in context");
    assert_eq!(engine.state().skills[0].uses, 1);
    // Recall: the next chat sees the earlier one.
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Was war das mit der Photosynthese?"})).unwrap();
    let last = seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.contains("früheren Chats") && last.contains("Photosynthese"), "recall excerpt");
    engine.call("settings.set", json!({"key": "recall", "value": false})).unwrap();
    engine.call("chat.send", json!({"text": "Und das Arbeitsblatt zur Photosynthese?"})).unwrap();
    assert!(!seen.lock().unwrap().last().unwrap().contains("Gelernte Vorgehen"));
    engine.call("skill.update", json!({"id": id, "when": "immer"})).unwrap();
    assert_eq!(engine.state().skills.iter().find(|s| s.id == id).unwrap().when, "immer");
    engine.call("skill.delete", json!({"id": id})).unwrap();
    assert!(engine.state().skills.iter().all(|s| s.id != id));

    // Learning needs two successful tool steps in the chat.
    let conv = engine.state().current.clone().unwrap();
    assert_eq!(engine.block_on(crate::skills::learn_from(engine.ctx(), &conv)).unwrap(), None);
    let mut m = engine.ctx().store(|st| st.messages(&conv)).unwrap().into_iter().find(|m| m.role == aiscraft_core::Role::Assistant).unwrap();
    for i in 0..2 {
        m.tool_calls.push(aiscraft_core::ToolCall { id: format!("t{i}"), name: "document_create".into(), arguments: json!({}), outcome: Some("ok".into()) });
    }
    engine.ctx().store(|st| st.save_message(&m)).unwrap();
    let learned = engine.block_on(crate::skills::learn_from(engine.ctx(), &conv)).unwrap();
    assert!(learned.is_some());
    assert!(engine.state().skills.iter().any(|s| s.title == "Elternbrief schreiben"));
}

#[test]
fn memory_tidy_archives_duplicates() {
    let (engine, _) = engine_with_recording("ok");
    engine.call("memory.add", json!({"text": "Ich unterrichte Mathe."})).unwrap();
    engine.call("memory.add", json!({"text": "ich  unterrichte mathe"})).unwrap();
    engine.call("memory.add", json!({"text": "Ich mag Tee"})).unwrap();
    assert_eq!(engine.call("memory.tidy", json!({})).unwrap()["archived"], 1);
    assert_eq!(engine.state().memory.iter().filter(|n| !n.archived).count(), 2);
}

/// Background tasks: run in their own chat; the result comes back to the origin chat.
#[test]
fn background_task_reports_back() {
    let (engine, _) = engine_with_recording("Drei Entwürfe sind fertig.");
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let origin = engine.state().current.clone().unwrap();
    let id = engine.call("task.start", json!({"title": "Entwürfe", "instructions": "Schreibe drei Entwürfe"})).unwrap()["id"].as_str().unwrap().to_string();
    let oc = origin.clone();
    assert!(
        engine.wait_for(Duration::from_secs(30), |s| s.messages.iter().any(|m| m.conversation_id == oc && m.content.contains("Aufgabe „Entwürfe“ ist fertig")))
    );
    assert!(engine.state().conversation(&id).unwrap().title.starts_with("▶ "));
    assert_eq!(engine.ctx().store(|st| st.messages(&id)).unwrap().len(), 2);
    // No task inside a task.
    let call = aiscraft_core::ToolCall { id: "x".into(), name: "task_start".into(), arguments: json!({"title": "a", "instructions": "b"}), outcome: None };
    let (_, err, _) = engine.block_on(crate::tools::run(engine.ctx(), &id, &call, "", true));
    assert!(err);
}

/// The tool bridge: token-bound, the chat's tools only, calls run in this engine.
#[test]
fn tool_bridge_serves_the_chats_tools() {
    let (engine, _) = engine_with_recording("ok");
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    let ctx = engine.ctx().clone();
    let (b, guard) = engine.block_on(crate::bridge::open(&ctx, &conv)).unwrap();
    let post = |token: &str, body: serde_json::Value| -> (u16, serde_json::Value) {
        let url = b.url.clone();
        engine.block_on(async move {
            let addr = url.trim_start_matches("http://").trim_end_matches("/mcp").to_string();
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let body = body.to_string();
            let req = format!(
                "POST /mcp HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {token}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                body.len()
            );
            s.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            let status = out.split_whitespace().nth(1).unwrap().parse().unwrap();
            let json = out.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(serde_json::Value::Null);
            (status, json)
        })
    };
    assert_eq!(post("falsch", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).0, 401);
    let (st, init) = post(&b.token, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}));
    assert_eq!((st, init["result"]["serverInfo"]["name"].as_str()), (200, Some("aischatcraft")));
    assert_eq!(post(&b.token, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).0, 202);
    let (_, list) = post(&b.token, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"document_create") && names.contains(&"board_add"));
    let (_, call) = post(
        &b.token,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "document_create", "arguments": {"title": "Über die Brücke", "markdown": "# Über die Brücke"}}}),
    );
    assert_eq!(call["result"]["isError"], false);
    assert!(engine.state().artifacts.iter().any(|a| a.title == "Über die Brücke"));
    assert!(
        engine.state().messages.iter().any(|m| m.tool_calls.iter().any(|t| t.name == "document_create" && t.outcome.as_deref() == Some("ok"))),
        "shown as a chip"
    );
    // Sources apply: no tools in this chat, none over the bridge.
    engine.call("chat.set_sources", json!({"tools": false})).unwrap();
    let (_, list) = post(&b.token, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}));
    assert!(list["result"]["tools"].as_array().unwrap().is_empty());
    drop(guard);
    assert_eq!(post(&b.token, json!({"jsonrpc": "2.0", "id": 5, "method": "tools/list"})).0, 401, "the token ends with the answer");
}

#[test]
fn board_tools_read_and_add() {
    let (engine, _) = engine_with_recording("ok");
    let id = engine.call("artifact.create", json!({"kind": "board", "content": "# Wald\n- Bäume"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("artifact.board_add", json!({"id": id, "outline": "# Tiere\n- Fuchs\n- Reh"})).unwrap();
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("Bäume") && text.contains("Fuchs") && text.contains("Reh"), "{text}");
    assert!(engine.call("artifact.board_add", json!({"id": id, "outline": "  "})).is_err());
    // The tools use the open board when no id is given.
    let read = aiscraft_core::ToolCall { id: "r".into(), name: "board_read".into(), arguments: json!({}), outcome: None };
    let (t, err, _) = engine.block_on(crate::tools::run(engine.ctx(), "c", &read, "", true));
    assert!(!err && t.contains("Fuchs"), "{t}");
}

/// Live: Claude through the subscription uses the app's tools over the bridge.
/// `cargo test -p aiscraft-engine -- --ignored claude_subscription_uses_tools`
#[test]
#[ignore]
fn claude_subscription_uses_tools() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let id = engine.call("backend.add", json!({"kind": "claude_cli"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "claude-haiku-5-5"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Lege mit dem Werkzeug document_create ein Dokument mit dem Titel „Brückentest“ und einem Satz Inhalt an. Antworte danach nur mit „erledigt“."})).unwrap();
    assert!(
        engine.state().artifacts.iter().any(|a| a.title.contains("Brückentest")),
        "document created through the bridge: {:?}",
        engine.state().messages.last()
    );
}

/// Voice: engines in memory never touch devices; provider choice prefers AIS.chat, then OpenAI.
#[test]
fn voice_refuses_devices_in_tests_and_picks_providers() {
    let (engine, _) = engine_with_recording("Hallo **Welt**");
    engine.call("chat.send", json!({"text": "x"})).unwrap();
    let mid = engine.state().messages[1].id.clone();
    for (cmd, p) in
        [("voice.speak", json!({"message_id": mid})), ("voice.stop", json!({})), ("voice.record_start", json!({})), ("voice.record_stop", json!({}))]
    {
        let e = engine.call(cmd, p).unwrap_err();
        assert!(matches!(e.kind, aiscraft_core::ErrorKind::Unsupported | aiscraft_core::ErrorKind::Invalid), "{cmd}: {e:?}");
    }
    assert!(!engine.state().recording);
    assert!(crate::voice::tts_provider(engine.ctx()).is_err(), "no key yet");
    let id = engine.state().selection.backend_id.clone();
    engine.call("backend.login", json!({"id": id, "secret": "sk-x"})).unwrap();
    assert_eq!(crate::voice::tts_provider(engine.ctx()).unwrap().0, aiscraft_core::BackendKind::OpenAi);
    assert_eq!(engine.mic_level(), 0.0);
}

/// Hints: one concrete need gives one hint, never twice; "nothing" gives none.
#[test]
fn hints_once_per_need() {
    let (engine, _) = engine_with_recording(r#"{"need": true, "text": "Du wolltest Frau Yilmaz bis Freitag den Plan schicken.", "conversation_id": "x"}"#);
    engine.call("chat.send", json!({"text": "Erinnere mich: Plan bis Freitag an Frau Yilmaz"})).unwrap();
    let first = engine.call("hints.check", json!({})).unwrap();
    assert!(first["hint"].as_str().unwrap().contains("Yilmaz"));
    assert!(engine.state().notices.iter().any(|n| n.text.starts_with("Hinweis:")));
    assert!(engine.call("hints.check", json!({})).unwrap()["hint"].is_null(), "the same hint never comes twice");
    let (engine2, _) = engine_with_recording(r#"{"need": false}"#);
    engine2.call("chat.send", json!({"text": "Hallo"})).unwrap();
    assert!(engine2.call("hints.check", json!({})).unwrap()["hint"].is_null());
    // Off by default: the hourly check does nothing.
    assert!(engine2.block_on(crate::proactive::check(engine2.ctx(), false)).unwrap().is_none());
}

/// AIS.chat takes no system prompt: memory and project instructions go along in the message only
/// after the person agreed (`inline`), once, and again only when they change.
#[test]
fn inline_context_for_backends_without_system_prompt() {
    let (engine, _) = engine_with_recording("ok");
    let pid = engine.call("project.create", json!({"name": "P", "instructions": "PROJEKTREGEL"})).unwrap()["id"].as_str().unwrap().to_string();
    let nid = engine.call("memory.add", json!({"text": "MERKNOTIZ"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("chat.new", json!({"project": pid})).unwrap();
    engine.call("chat.send", json!({"text": "eins"})).unwrap();
    let conv = || engine.state().conversation(engine.state().current.as_deref().unwrap()).cloned().unwrap();
    // Older chats and new ones: off until switched on.
    assert!(!conv().sources.inline);
    assert!(crate::chat::inline_context(engine.ctx(), &conv(), false).is_none());
    assert!(!conv().sources.memory_for(false) && conv().sources.memory_for(true));
    engine.call("chat.set_sources", json!({"memory": true, "project": false, "inline": true})).unwrap();
    let (block, print) = crate::chat::inline_context(engine.ctx(), &conv(), false).unwrap();
    assert!(block.contains("MERKNOTIZ") && !block.contains("PROJEKTREGEL"), "only what is switched on");
    // Backends with a system prompt never get it inline.
    assert!(crate::chat::inline_context(engine.ctx(), &conv(), true).is_none());
    // Once sent, not again …
    let mut sent = conv();
    sent.sources.inline_sent = Some(print.clone());
    assert!(crate::chat::inline_context(engine.ctx(), &sent, false).is_none());
    // … until the notes change.
    engine.call("memory.update", json!({"id": nid, "text": "NEUE NOTIZ"})).unwrap();
    let (block, print2) = crate::chat::inline_context(engine.ctx(), &sent, false).unwrap();
    assert!(block.contains("NEUE NOTIZ") && print2 != print);
}

/// The collab plug-in without a relay: present, but nothing is shared and nothing connects.
#[cfg(feature = "collab")]
#[test]
fn collab_without_server_stays_local() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let s = engine.call("collab.status", json!({})).unwrap();
    assert_eq!(s["available"], true);
    assert_eq!(s["server"], "");
    let id = engine.call("artifact.create", json!({"kind": "document", "content": "# Lokal"})).unwrap()["id"].as_str().unwrap().to_string();
    let e = engine.call("collab.share", json!({"id": id})).unwrap_err();
    assert!(e.message.contains("Kein Server"), "{}", e.message);
    assert!(engine.call("collab.join", json!({"code": "ABCDE-FGHJK"})).unwrap_err().message.contains("Kein Server"));
    assert!(engine.call("collab.join", json!({"code": "kein code"})).is_err());
    assert!(engine.call("collab.leave", json!({"id": id})).is_err(), "not shared");
    assert!(engine.call("collab.set_server", json!({"url": "ftp://x"})).is_err());
    assert_eq!(engine.call("collab.status", json!({"id": id})).unwrap()["shared"], false);
    // Saving works exactly as without the plug-in.
    engine.call("artifact.save", json!({"id": id, "text": "# Lokal\n\nText"})).unwrap();
    assert_eq!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"], "# Lokal\n\nText");
}

/// Poll until `f` holds (10 s); for checks that need engine calls.
#[cfg(feature = "collab")]
fn eventually(f: impl Fn() -> bool) -> bool {
    for _ in 0..400 {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[cfg(feature = "collab")]
fn text_of(engine: &Engine, id: &str) -> String {
    engine.call("artifact.get", json!({"id": id})).map(|v| v["text"].as_str().unwrap_or("").to_string()).unwrap_or_default()
}

/// Two engines share a document and a whiteboard through an in-process relay and converge.
#[cfg(feature = "collab")]
#[test]
fn collab_two_engines_converge() {
    let a = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let b = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let dir = std::env::temp_dir().join(format!("aiscraft-engine-collab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let relay = a.block_on(async {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        aiscraft_collab_server::start(l, aiscraft_collab_server::Config::new(dir.clone())).await.unwrap()
    });
    let url = format!("http://{}", relay.addr);
    assert_eq!(a.call("collab.set_server", json!({"url": url, "key": ""})).unwrap()["server"], url.as_str());

    // A shares a document; B joins by link without any server set.
    let doc = a.call("artifact.create", json!({"kind": "document", "content": "# Plan\n\nEins\n"})).unwrap()["id"].as_str().unwrap().to_string();
    let shared = a.call("collab.share", json!({"id": doc})).unwrap();
    let link = shared["link"].as_str().unwrap().to_string();
    assert!(link.starts_with(&url) && link.contains("/join/"));
    assert_eq!(a.call("collab.share", json!({"id": doc})).unwrap()["code"], shared["code"], "sharing again shows the same code");
    let joined = b.call("collab.join", json!({"link": link})).unwrap();
    assert_eq!(joined["id"], doc.as_str(), "the room is the artifact id");
    assert_eq!(joined["synced"], true);
    assert_eq!(text_of(&b, &doc), "# Plan\n\nEins\n");
    assert_eq!(b.state().open_artifact.as_deref(), Some(doc.as_str()));
    assert!(a.wait_for(Duration::from_secs(10), |s| s.share(&doc).is_some_and(|v| v.status == "online" && v.peers == 2)));
    let st = b.call("collab.status", json!({"id": doc})).unwrap();
    assert_eq!(st["share"]["owner"], false);

    // B edits, A sees it.
    b.call("artifact.save", json!({"id": doc, "text": "# Plan\n\nEins\nZwei\n"})).unwrap();
    assert!(eventually(|| text_of(&a, &doc) == "# Plan\n\nEins\nZwei\n"));

    // Both edit at the same time, each from the state it had: both edits survive, everywhere.
    let base = a.call("artifact.get", json!({"id": doc})).unwrap()["state_b64"].as_str().unwrap().to_string();
    use base64::Engine as _;
    let base_bytes = base64::engine::general_purpose::STANDARD.decode(&base).unwrap();
    let mine = crate::doc_replace(&base_bytes, "# Plan!\n\nEins\nZwei\n").unwrap();
    b.call("artifact.save", json!({"id": doc, "text": "# Plan\n\nEins\nZwei\nDrei\n"})).unwrap();
    a.call("artifact.save", json!({"id": doc, "state_b64": base64::engine::general_purpose::STANDARD.encode(mine)})).unwrap();
    let want = "# Plan!\n\nEins\nZwei\nDrei\n";
    assert!(eventually(|| text_of(&a, &doc) == want), "a: {:?}", text_of(&a, &doc));
    assert!(eventually(|| text_of(&b, &doc) == want), "b: {:?}", text_of(&b, &doc));

    // A whiteboard, joined by code with the server set.
    let board = a.call("artifact.create", json!({"kind": "board", "content": "# Wald\n- Bäume"})).unwrap()["id"].as_str().unwrap().to_string();
    let code = a.call("collab.share", json!({"id": board})).unwrap()["code"].as_str().unwrap().to_string();
    b.call("collab.set_server", json!({"url": url})).unwrap();
    b.call("collab.join", json!({"code": code.to_lowercase()})).unwrap();
    assert!(text_of(&b, &board).contains("Bäume"));
    b.call("artifact.board_add", json!({"id": board, "outline": "# Tiere\n- Fuchs"})).unwrap();
    assert!(eventually(|| text_of(&a, &board).contains("Fuchs")));

    // B leaves (keeps its copy); A ends the room for everyone.
    b.call("collab.leave", json!({"id": board})).unwrap();
    assert!(b.state().share(&board).is_none());
    assert!(text_of(&b, &board).contains("Fuchs"));
    assert!(b.call("collab.leave", json!({"id": doc, "end": true})).is_err(), "only the owner ends a room");
    a.call("collab.leave", json!({"id": board, "end": true})).unwrap();
    assert!(b.call("collab.join", json!({"code": code})).is_err(), "the room is gone");
    // Deleting a shared artifact stops sharing it.
    a.call("artifact.delete", json!({"id": doc})).unwrap();
    assert!(a.state().collab.shares.is_empty());
    a.block_on(relay.shutdown());
    let _ = std::fs::remove_dir_all(&dir);
}
