//! Chat features after Claude's apps, driven headless (no window, no focus): thinking (#30),
//! answer styles (#31), temporary chats (#34), citations (#39), finding in the chat (#41).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{click_until, settled, step_until};
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

fn harness(engine: &Engine) -> egui_kittest::Harness<'static, AisCraftApp> {
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    if let Some(later) = h.query_by_label("Später") {
        later.click();
        h.run_steps(4);
    } else if let Some(done) = h.query_by_label("Fertig") {
        done.click();
        h.run_steps(4);
    }
    h
}

/// A local OpenAI-compatible server streaming the chunks `answer` makes from the request.
fn backend(engine: &Engine, answer: fn(&str) -> Vec<Value>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
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
    })
}

fn engine_with(answer: fn(&str) -> Vec<Value>) -> Engine {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = backend(&engine, answer);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine
}

/// #30: the thinking sits folded above the answer as „Nachgedacht“ and opens with a click.
#[test]
fn thinking_is_folded_above_the_answer() {
    fn thinks(_: &str) -> Vec<Value> {
        vec![
            json!({"choices":[{"delta":{"reasoning_content":"Erst die Einheiten prüfen."}}]}),
            json!({"choices":[{"delta":{"content":"Es sind zwölf Meter."}}]}),
        ]
    }
    let engine = engine_with(thinks);
    engine.call("chat.send", json!({"text": "Wie lang?"})).unwrap();
    let mut h = harness(&engine);
    step_until(&mut h, "the answer shows", |h| h.query_by_label_contains("zwölf Meter").is_some());
    assert!(h.query_by_label("Nachgedacht").is_some(), "the thinking has its fold");
    assert!(h.query_by_label_contains("Einheiten prüfen").is_none(), "folded at first");
    click_until(&mut h, "Nachgedacht", "the thinking opens", |h| h.query_by_label_contains("Einheiten prüfen").is_some());
    settled(&mut h, "Nachgedacht");
}

/// #31: the style is chosen in the composer; „Knapp“ reaches the chat.
#[test]
fn style_is_chosen_in_the_composer() {
    let engine = engine_with(|_| vec![json!({"choices":[{"delta":{"content":"Ok."}}]})]);
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let mut h = harness(&engine);
    step_until(&mut h, "the chat shows", |h| h.query_by_label("Stil").is_some());
    click_until(&mut h, "Stil", "the style menu opens", |h| h.query_by_label("Knapp").is_some());
    let e = engine.clone();
    click_until(&mut h, "Knapp", "the style reaches the engine", move |_| e.state().selection.sources.style.as_deref() == Some("concise"));
    step_until(&mut h, "the composer names the style", |h| h.query_by_label("Stil: Knapp").is_some());
}

/// #34: „Temporärer Chat“ on the start page starts one; it is not listed in the sidebar and
/// says that it is not kept.
#[test]
fn temporary_chat_from_the_start_page() {
    let engine = engine_with(|_| vec![json!({"choices":[{"delta":{"content":"Ok."}}]})]);
    let mut h = harness(&engine);
    click_until(&mut h, "Temporärer Chat", "the next chat is temporary", |h| h.query_by_label("Temporär: an").is_some());
    assert!(engine.state().selection.temporary);
    engine.call("chat.send", json!({"text": "Streng geheimer Plan"})).unwrap();
    step_until(&mut h, "the chat shows", |h| h.query_by_label_contains("wird nicht gespeichert").is_some());
    assert!(h.query_all_by_label_contains("Streng geheimer Plan").count() <= 1, "only the message, not a sidebar entry");
}

/// #41: Strg+F opens the search in the chat; Enter jumps to the next hit, Esc closes it.
#[test]
fn ctrl_f_finds_in_the_chat() {
    let engine = engine_with(|_| vec![json!({"choices":[{"delta":{"content":"Der Igel schläft. Igel fressen Käfer."}}]})]);
    engine.call("chat.send", json!({"text": "Erzähl vom Igel"})).unwrap();
    let mut h = harness(&engine);
    step_until(&mut h, "the answer shows", |h| h.query_by_label_contains("fressen Käfer").is_some());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    step_until(&mut h, "the search field opens", |h| h.query_by_label("Im Chat suchen").is_some());
    h.run_steps(2);
    h.event(egui::Event::Text("igel".into()));
    step_until(&mut h, "three hits", |h| h.query_by_label("1 von 3").is_some());
    h.key_press(egui::Key::Enter);
    step_until(&mut h, "the next hit", |h| h.query_by_label("2 von 3").is_some());
    assert_eq!(engine.state().find.as_ref().map(|f| f.current), Some(1));
    click_until(&mut h, "‹", "back to the first hit", |h| h.query_by_label("1 von 3").is_some());
    h.key_press(egui::Key::Escape);
    step_until(&mut h, "Esc closes the search", |h| h.query_by_label("Im Chat suchen").is_none());
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.find.is_none()));
    assert!(engine.state().current.is_some(), "Esc closed only the search, not the chat");
}

/// #39: under a Claude answer the passages that back it; a click shows the passage marked in
/// the attached file.
#[test]
fn citations_show_under_the_answer_and_open_at_the_passage() {
    fn cites(req: &str) -> Vec<Value> {
        if !req.contains("\"type\":\"document\"") {
            return vec![json!({"type": "message_stop"})];
        }
        vec![
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "citations_delta", "citation": {
                "type": "char_location", "cited_text": "Im Winter halten sie Winterschlaf.", "document_index": 0,
                "document_title": "igel.txt", "start_char_index": 23, "end_char_index": 57}}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Igel schlafen im Winter."}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "message_stop"}),
        ]
    }
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = backend(&engine, cites);
    let id = engine
        .call("backend.add", json!({"kind": "anthropic", "base_url": url.trim_end_matches("/v1"), "name": "Claude", "secret": "sk-ant-test"}))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("chat.select_model", json!({"backend": id, "model": "claude-opus-5-5"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    let dir = std::env::temp_dir().join(format!("aiscraft-ui-cite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("igel.txt");
    std::fs::write(&file, "Igel fressen Insekten. Im Winter halten sie Winterschlaf. Sie sind nachtaktiv.").unwrap();
    engine.call("chat.send", json!({"text": "Was machen Igel im Winter?", "files": [file.display().to_string()]})).unwrap();
    let mut h = harness(&engine);
    step_until(&mut h, "the citation shows under the answer", |h| h.query_by_label("Belege:").is_some());
    let e = engine.clone();
    click_until(&mut h, "[1] igel.txt", "the citation opens", move |_| e.state().citation.is_some());
    step_until(&mut h, "the passage shows in its window", |h| {
        h.query_by_label("Beleg").is_some() && h.query_by_label_contains("Igel fressen Insekten. Im Winter halten sie Winterschlaf.").is_some()
    });
    assert_eq!(engine.state().citation.as_ref().map(|c| c.passage.as_str()), Some("Im Winter halten sie Winterschlaf."));
    engine.call("message.citation", json!({"close": true})).unwrap();
    step_until(&mut h, "the window closes", |h| h.query_by_label("Beleg").is_none());
    let _ = std::fs::remove_dir_all(dir);
}
