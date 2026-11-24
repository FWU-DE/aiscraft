//! UI behaviour, driven headless through egui's accessibility tree (no window, no focus).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use egui_kittest::kittest::Queryable;

fn harness(engine: &Engine) -> egui_kittest::Harness<'static, AisCraftApp> {
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| AisCraftApp::new(cc, e.clone(), None));
    h.run_steps(4);
    // Close the welcome dialog a fresh profile shows.
    if let Some(later) = h.query_by_label("Später") {
        later.click();
        h.run_steps(4);
    }
    h
}

#[test]
fn sources_popup_stays_open_until_closed() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    let before = engine.state().selection.sources.clone();
    h.get_by_label_contains("Quellen").click();
    h.run_steps(4);
    h.get_by_label("Gedächtnis (deine Notizen)").click();
    h.run_steps(6);
    assert!(h.query_by_label("Gedächtnis (deine Notizen)").is_some(), "the popup stays open after a switch");
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.selection.sources != before), "the switch reached the engine");
    h.get_by_label("Schließen").click();
    h.run_steps(6);
    assert!(h.query_by_label("Gedächtnis (deine Notizen)").is_none(), "Schließen closes it");
}

#[test]
fn sidebar_collapse_button_collapses() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    assert!(h.query_by_label("Neuer Chat").is_some() || h.query_by_label_contains("Neuer Chat").is_some());
    // The ☰ button in the sidebar header hides the sidebar (regression: the click was lost).
    h.get_all_by_label("☰").next().unwrap().click();
    h.run_steps(30);
    assert!(h.query_by_label_contains("Neuer Chat").is_none(), "sidebar hidden");
}

#[test]
fn handbook_opens_closes_and_follows_settings_links() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    h.key_press(egui::Key::F1);
    h.run_steps(6);
    assert!(h.query_by_label("Erste Schritte").is_some(), "F1 opens the handbook with its chapters");
    h.get_by_label("Tresor").click();
    h.run_steps(6);
    assert!(h.query_by_label_contains("{{vault:").is_some(), "a chapter opens");
    h.key_press(egui::Key::F1);
    h.run_steps(6);
    assert!(h.query_by_label("Erste Schritte").is_none(), "F1 closes it again");
    // From the settings: the link opens the chapter of the tab.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(6);
    h.get_by_role_and_label(egui::accesskit::Role::Link, "Anmelden").click();
    h.run_steps(6);
    assert!(h.query_by_label("Erste Schritte").is_some(), "settings link opens the handbook");
    assert!(h.query_by_label_contains("Im Browser anmelden").is_some(), "at the sign-in chapter");
}

/// The "Entdecken" page opens from the palette, says what is missing when nobody is signed in to
/// AIS.chat, and Esc brings the chat back.
#[test]
fn explore_page_opens_and_esc_returns() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::K);
    h.run_steps(4);
    h.event(egui::Event::Text("entdecken".into()));
    h.run_steps(4);
    h.get_by_label("Assistenten, Figuren & Lernszenarien entdecken").click();
    h.run_steps(6);
    assert!(h.query_by_label("Nicht bei AIS.chat angemeldet").is_some(), "the page explains the missing sign-in");
    assert!(h.query_by_label_contains("Zurück zum Chat").is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(6);
    assert!(h.query_by_label("Nicht bei AIS.chat angemeldet").is_none(), "Esc closes the page");
}

/// A local OpenAI-compatible server whose answers count up: "Antwort 1", "Antwort 2", …
fn counting_backend(engine: &Engine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let n = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let n = n.clone();
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
                    if buf.starts_with(b"GET") {
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let i = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    let c = serde_json::json!({"choices":[{"delta":{"content": format!("Antwort {i}")}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// After "Neu erzeugen" the answer shows ‹ 2/2 ›; ‹ brings the first answer back.
#[test]
fn variant_switcher_shows_the_old_answer() {
    use serde_json::json;
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = counting_backend(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    engine.call("chat.regenerate", json!({})).unwrap();
    assert_eq!(engine.state().messages[1].content, "Antwort 2");
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label("2/2").is_some(), "the switcher shows the version");
    h.get_by_label("‹").click();
    h.run_steps(6);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.messages.get(1).is_some_and(|m| m.content == "Antwort 1")), "the old answer is back");
    h.run_steps(6);
    assert!(h.query_by_label("1/2").is_some());
    assert!(h.query_by_label_contains("Antwort 1").is_some());
}

/// The × in the header and Esc close the chat; the start page shows its cards, and its quick
/// start opens a new document.
#[test]
fn close_chat_shows_the_start_page() {
    use serde_json::json;
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = counting_backend(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label("Neu starten").is_none(), "a chat is open, no start page");
    h.get_by_label("×").click();
    h.run_steps(8);
    assert!(engine.state().current.is_none(), "× closed the chat");
    assert!(h.query_by_label("Neu starten").is_some() && h.query_by_label("Zeitpläne").is_some(), "the start page shows");
    assert!(h.get_all_by_label_contains("Hallo").count() >= 2, "greeting, and recent chats list it");

    // Back into the chat, then Esc with an empty composer closes it again.
    engine.call("chat.open", json!({"id": engine.state().conversations[0].id})).unwrap();
    h.run_steps(8);
    assert!(engine.state().current.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(8);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.current.is_none()), "Esc closed the chat");

    // A quick start on the start page.
    h.get_by_label("📄  Dokument").click();
    h.run_steps(8);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.open_artifact.is_some()), "a new document opens");
}

/// Notifications: the bell shows the unread count and opens the list; "Alle gelesen" marks them.
#[test]
fn bell_lists_notifications() {
    use serde_json::json;
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = counting_backend(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    let sid = engine.call("schedule.create", json!({"title": "Morgenplan", "prompt": "Was steht an?", "cron": "0 7 * * 1-5"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("schedule.run", json!({"id": sid})).unwrap();
    engine.call("chat.close", json!({})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label("Benachrichtigungen (1 neu)").is_some(), "the start page card");
    h.get_by_label("🔔 1").click();
    h.run_steps(6);
    assert!(h.query_by_label("Leeren").is_some(), "the popover is open");
    h.get_all_by_label("Alle gelesen").next().unwrap().click();
    h.run_steps(6);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.inbox.iter().all(|i| i.read)), "marked read");
    h.run_steps(6);
    assert!(h.query_by_label("🔔").is_some(), "no count any more");
}
