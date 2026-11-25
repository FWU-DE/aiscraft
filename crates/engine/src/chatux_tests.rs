//! Chat features after Claude's apps: thinking (#30), answer styles (#31), temporary chats (#34),
//! quick entry (#35), sources and citations (#39), finding in the open chat (#41).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

type Seen = Arc<Mutex<Vec<String>>>;

/// An OpenAI-compatible server that records every request and streams the chunks `answer`
/// makes from the request body.
fn scripted(engine: &Engine, answer: fn(&str) -> Vec<Value>) -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
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
                        let body = r#"{"data":[{"id":"m"},{"id":"claude-opus-5-5"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    for c in answer(&req) {
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

fn engine_with(answer: fn(&str) -> Vec<Value>) -> (Engine, Seen) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, answer);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, seen)
}

fn plain(_: &str) -> Vec<Value> {
    vec![json!({"choices":[{"delta":{"content":"Antwort"}}]})]
}

fn last_chat_request(seen: &Seen, marker: &str) -> String {
    seen.lock().unwrap().iter().rev().find(|r| r.contains(marker)).cloned().unwrap_or_default()
}

/// #30: the model's thinking is kept with the answer, apart from its text, and survives a reload.
#[test]
fn thinking_is_kept_beside_the_answer() {
    fn thinks(_: &str) -> Vec<Value> {
        vec![
            json!({"choices":[{"delta":{"reasoning_content":"Erst die Frage verstehen. "}}]}),
            json!({"choices":[{"delta":{"reasoning_content":"Dann knapp antworten."}}]}),
            json!({"choices":[{"delta":{"content":"Zwölf."}}]}),
        ]
    }
    let (engine, seen) = engine_with(thinks);
    engine.call("chat.send", json!({"text": "Wie viel ist 3 mal 4?"})).unwrap();
    assert!(!last_chat_request(&seen, "3 mal 4").contains("Erst die Frage"), "the thinking is not sent back as text");
    let answer = engine.state().messages.last().cloned().unwrap();
    assert_eq!(answer.content, "Zwölf.");
    assert_eq!(answer.thinking, "Erst die Frage verstehen. Dann knapp antworten.");
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.close", json!({})).unwrap();
    engine.call("chat.open", json!({"id": conv})).unwrap();
    assert_eq!(engine.state().messages.last().unwrap().thinking, "Erst die Frage verstehen. Dann knapp antworten.", "kept in the store");
}

/// #30: the thinking depth is a per-chat switch; unknown levels are refused.
#[test]
fn effort_is_a_per_chat_switch() {
    let (engine, _) = engine_with(plain);
    engine.call("chat.set_sources", json!({"effort": "high"})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    assert_eq!(engine.state().conversation(&conv).unwrap().sources.effort.as_deref(), Some("high"));
    assert!(engine.call("chat.set_sources", json!({"effort": "ganz viel"})).is_err());
    engine.call("chat.set_sources", json!({"effort": null})).unwrap();
    assert!(engine.state().conversation(&conv).unwrap().sources.effort.is_none());
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().selection.sources.effort.is_none(), "a new chat starts with the model's default");
}
