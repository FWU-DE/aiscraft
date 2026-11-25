//! UI behaviour of features taken from Nod (AGENTS.md §5), driven headless through egui's
//! accessibility tree (no window, no focus).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{click_until, step_until};
use egui_kittest::kittest::Queryable;
use serde_json::json;

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

/// A local OpenAI-compatible server: answers to the task's prompt („Prüfe den Elternbrief“) wait for `release`, the rest come
/// at once.
fn gated_backend(engine: &Engine, release: Arc<AtomicBool>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let release = release.clone();
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
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let text = if req.contains("Prüfe den Elternbrief") {
                        while !release.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        }
                        "Der Elternbrief ist geprüft, zwei Tippfehler sind korrigiert."
                    } else {
                        "Gern."
                    };
                    let c = json!({"choices":[{"delta":{"content": text}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// #69: a started task shows in the status line above the composer (a click opens its chat) and
/// leaves it when it ends; its result is a compact card whose „Details“ opens the task's chat.
#[test]
fn running_task_in_the_status_line_and_result_card() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let url = gated_backend(&engine, release.clone());
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let origin = engine.state().current.clone().unwrap();
    let mut h = harness(&engine);

    let task =
        engine.call("task.start", json!({"title": "Elternbrief prüfen", "instructions": "Prüfe den Elternbrief"})).unwrap()["id"].as_str().unwrap().to_string();
    let line = "1 Aufgabe läuft · Elternbrief prüfen …";
    step_until(&mut h, "the task shows in the status line", |h| h.query_by_label(line).is_some());
    let t = task.clone();
    click_until(&mut h, line, "the status line opens the task's chat", |_| engine.state().current.as_deref() == Some(t.as_str()));
    engine.call("chat.open", json!({"id": origin})).unwrap();

    release.store(true, Ordering::SeqCst);
    step_until(&mut h, "the task leaves the status line", |h| h.query_by_label(line).is_none() && engine.state().running.is_empty());
    step_until(&mut h, "the result card", |h| h.query_by_label("Aufgabe „Elternbrief prüfen“ ist fertig").is_some() && h.query_by_label("Details").is_some());
    assert!(h.query_by_label_contains("zwei Tippfehler").is_some(), "one line of the result");
    let t = task.clone();
    click_until(&mut h, "Details", "Details opens the task's chat", |_| engine.state().current.as_deref() == Some(t.as_str()));
}
