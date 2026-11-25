//! Tests for project knowledge (#32), research (#37), voice mode (#40), connector results for
//! AIS.chat (#53) and templates (#54). In the list `every_command_has_a_functional_test` reads.

use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

/// One request a scripted server saw: the first line and the body.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub line: String,
    pub body: String,
}

/// A local HTTP server: `route(first line, body)` gives (content type, body). Bodies of
/// `text/event-stream` are sent as they are. Every request is recorded.
pub(crate) fn scripted(engine: &Engine, route: impl Fn(&str, &str) -> (String, Vec<u8>) + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::<Seen>::new()));
    let seen2 = seen.clone();
    let route = Arc::new(route);
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                let route = route.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    let mut head_end = None;
                    loop {
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..k]);
                        if head_end.is_none() {
                            head_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
                        }
                        if let Some(h) = head_end {
                            let head = String::from_utf8_lossy(&buf[..h]).to_lowercase();
                            let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let h = head_end.unwrap_or(buf.len());
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let line = text.lines().next().unwrap_or("").to_string();
                    let body = String::from_utf8_lossy(buf.get(h + 4..).unwrap_or(&[])).to_string();
                    seen.lock().unwrap().push(Seen { line: line.clone(), body: body.clone() });
                    let (ctype, out) = route(&line, &body);
                    let head = if ctype == "text/event-stream" {
                        format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\n\r\n")
                    } else {
                        format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", out.len())
                    };
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&out).await;
                });
            }
        });
        format!("http://{addr}")
    });
    (url, seen)
}

/// An OpenAI-compatible answer stream with `text`.
pub(crate) fn sse(text: &str) -> (String, Vec<u8>) {
    let c = json!({"choices":[{"delta":{"content": text}}]});
    ("text/event-stream".into(), format!("data: {c}\n\ndata: [DONE]\n\n").into_bytes())
}

pub(crate) fn models() -> (String, Vec<u8>) {
    ("application/json".into(), br#"{"data":[{"id":"m","context_length":100000}]}"#.to_vec())
}

/// An engine with an OpenAI-compatible backend at `url` (with a key) and a new chat.
pub(crate) fn engine_at(engine: &Engine, url: &str) -> String {
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": format!("{url}/v1"), "name": "Test", "secret": "sk-test"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    id
}

fn posts(seen: &Arc<Mutex<Vec<Seen>>>, path: &str) -> Vec<String> {
    seen.lock().unwrap().iter().filter(|s| s.line.starts_with("POST") && s.line.contains(path)).map(|s| s.body.clone()).collect()
}

/// #54: templates with fields: starters on a fresh profile, save with fields, fill into the
/// composer, missing fields named, send, values remembered, delete.
#[test]
fn templates_fill_fields_and_send() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, |line, _| if line.starts_with("GET") { models() } else { sse("Liebe Eltern …") });
    engine_at(&engine, &url);
    let starters: Vec<String> = engine.state().templates.iter().map(|t| t.title.clone()).collect();
    assert!(starters.contains(&"Brief an die Eltern".to_string()) && starters.len() == 3, "{starters:?}");

    let out =
        engine.call("template.save", json!({"title": "Wandertag", "text": "Brief an {{Klasse}} zum Wandertag am {{Datum}}, Ton {{Ton=locker}}."})).unwrap();
    let id = out["id"].as_str().unwrap().to_string();
    let fields: Vec<&str> = out["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(fields, ["Klasse", "Datum", "Ton"]);
    assert!(engine.call("template.save", json!({"title": " ", "text": "x"})).is_err());

    let err = engine.call("template.use", json!({"id": id, "values": {"Klasse": "7b"}})).unwrap_err();
    assert!(err.message.contains("Datum"), "{}", err.message);
    let out = engine.call("template.use", json!({"id": id, "values": {"Klasse": "7b", "Datum": "3. Mai"}})).unwrap();
    assert_eq!(out["text"], "Brief an 7b zum Wandertag am 3. Mai, Ton locker.");
    let fill = engine.state().composer_fill.clone().unwrap();
    assert!(fill.replace && fill.text == out["text"].as_str().unwrap(), "goes into the composer");
    assert!(posts(&seen, "/chat/completions").is_empty(), "nothing sent yet");
    let t = engine.state().templates.iter().find(|t| t.id == id).cloned().unwrap();
    assert_eq!(t.last.get("Klasse").map(String::as_str), Some("7b"), "values are offered next time");

    let out = engine.call("template.use", json!({"id": id, "values": {"Klasse": "8a", "Datum": "4. Mai", "Ton": "förmlich"}, "send": true})).unwrap();
    assert_eq!(out["sent"], true);
    let sent = posts(&seen, "/chat/completions");
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Brief an 8a zum Wandertag am 4. Mai, Ton förmlich."));

    engine.call("template.delete", json!({"id": id})).unwrap();
    assert!(engine.state().templates.iter().all(|t| t.id != id));
    assert!(engine.call("template.delete", json!({"id": id})).is_err());
    assert!(engine.call("template.use", json!({"id": "nope"})).is_err());
}
