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
    let bid = id.clone();
    assert!(engine.wait_for(Duration::from_secs(30), |s| s.backend(&bid).is_some_and(|b| !b.models.is_empty())), "models load");
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
