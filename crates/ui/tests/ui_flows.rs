//! UI behaviour, driven headless through egui's accessibility tree (no window, no focus).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{chosen, click_until, settled, step_until};
use egui_kittest::kittest::Queryable;

fn harness(engine: &Engine) -> egui_kittest::Harness<'static, AisCraftApp> {
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    // Close the welcome dialog a fresh profile shows ("Später"), or the look question a profile
    // that is set up already shows once ("Fertig").
    if let Some(later) = h.query_by_label("Später") {
        later.click();
        h.run_steps(4);
    } else if let Some(done) = h.query_by_label("Fertig") {
        done.click();
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
    settled(&mut h, "Gedächtnis (deine Notizen)");
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
    h.get_by_label("Seitenleiste ausblenden (Strg+B)").click();
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
    // The table of contents scrolls on small windows.
    h.get_by_label("Tresor").scroll_to_me();
    h.run_steps(4);
    h.get_by_label("Tresor").click();
    step_until(&mut h, "a chapter opens", |h| h.query_by_label_contains("{{vault:").is_some());
    h.key_press(egui::Key::F1);
    h.run_steps(6);
    assert!(h.query_by_label("Erste Schritte").is_none(), "F1 closes it again");
    // From the settings: the link opens the chapter of the tab.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(6);
    h.get_by_label("Handbuch: Dienste").click();
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
    h.get_by_label("Assistenten, Dialogpartner & Lernszenarien entdecken").click();
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

/// A local OpenAI-compatible server that always answers `answer`; the engine is set up to use it.
fn answering_backend(engine: &Engine, answer: &str) {
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let answer = answer.to_string();
    let url = engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let answer = answer.clone();
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
                    let c = json!({"choices":[{"delta":{"content": answer}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
}

/// An answer with an `aiscraft-board` block (#56) shows a card instead of the JSON; „Whiteboard
/// öffnen“ lays it out next to the chat.
#[test]
fn whiteboard_block_becomes_a_card() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    answering_backend(
        &engine,
        "Hier:\n```aiscraft-board\n{\"title\":\"Wasserkreislauf\",\"layout\":\"cluster\",\"nodes\":[\"Verdunstung\",\"Regen\",\"Grundwasser\"]}\n```",
    );
    engine.call("chat.send", serde_json::json!({"text": "Whiteboard bitte"})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label_contains("Whiteboard · 3 Karten, Cluster").is_some(), "the card names the board");
    assert!(h.query_by_label_contains("\"nodes\"").is_none(), "no raw JSON in the chat");
    click_until(&mut h, "Whiteboard öffnen", "a board opens", |_| engine.state().artifacts.iter().any(|a| a.kind == aiscraft_core::ArtifactKind::Board));
    assert_eq!(engine.state().artifacts[0].title, "Wasserkreislauf");
}

/// „Lesbarkeit“ under an answer (#48) shows one discreet line and hides it again.
#[test]
fn readability_line_under_an_answer() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    answering_backend(&engine, "Der Hund ist klein. Er spielt im Garten. Die Sonne scheint.");
    engine.call("chat.send", serde_json::json!({"text": "Leicht bitte"})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    click_until(&mut h, "Lesbarkeit", "the line shows", |h| h.query_by_label_contains("11 Wörter · Flesch").is_some());
    click_until(&mut h, "Lesbarkeit", "the line hides", |h| h.query_by_label_contains("11 Wörter · Flesch").is_none());
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

/// A format request (#56) shows the teacher's topic; the format spec sits folded under
/// „Formatvorgabe“ and opens with a click.
#[test]
fn format_spec_is_folded_under_the_request() {
    use serde_json::json;
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = counting_backend(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    let prompt = aiscraft_engine::learn::worksheet_prompt("Photosynthese (Klasse: 7)", 8, false, true);
    engine.call("chat.send", json!({"text": prompt})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label("Photosynthese (Klasse: 7)").is_some(), "the topic stands alone");
    assert!(h.query_by_label_contains("Etwa 8 Aufgaben").is_none(), "the spec is folded away");
    h.get_by_label_contains("Formatvorgabe: Arbeitsblatt").click();
    h.run_steps(6);
    assert!(h.query_by_label_contains("Etwa 8 Aufgaben").is_some(), "a click shows the spec");
}

/// "Chat schließen" in the header's menu and Esc close the chat; the start page shows its cards,
/// a prompt starter fills the composer, and the sidebar's "+" opens a new document.
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
    assert!(h.query_by_label("Woran arbeiten wir heute?").is_none(), "a chat is open, no start page");
    h.get_by_label("Weitere Aktionen").click();
    h.run_steps(4);
    h.get_by_label("Chat schließen, zur Startseite (Esc)").click();
    h.run_steps(8);
    // chat.close runs on the engine's runtime; on a slow runner it lands after these frames.
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.current.is_none()), "× closed the chat");
    h.run_steps(4);
    assert!(h.query_by_label("Woran arbeiten wir heute?").is_some() && h.query_by_label("Zeitpläne").is_some(), "the start page shows");
    assert!(
        h.query_by_label("Zuletzt bearbeitet").is_some() && h.get_all_by_label_contains("Hallo").count() >= 2,
        "recent work lists the chat (and the sidebar)"
    );

    // Back into the chat, then Esc with an empty composer closes it again.
    engine.call("chat.open", json!({"id": engine.state().conversations[0].id})).unwrap();
    h.run_steps(8);
    assert!(engine.state().current.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(8);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.current.is_none()), "Esc closed the chat");

    // A prompt starter fills the composer; a new document from the sidebar's "+".
    h.get_by_label("Stundenentwurf").click();
    h.run_steps(4);
    assert!(
        h.query_all(egui_kittest::kittest::By::new().predicate(|n| n.value().is_some_and(|v| v.contains("Stundenentwurf für Klasse 7")))).next().is_some(),
        "the starter's prompt is in the composer"
    );
    h.get_by_label("Neues Dokument oder Whiteboard").click();
    h.run_steps(4);
    h.get_by_label("📄  Neues Dokument").click();
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
    click_until(&mut h, "Benachrichtigungen: 1 neu", "the popover is open", |h| h.query_by_label("Leeren").is_some());
    h.get_all_by_label("Alle gelesen").next().unwrap().click();
    h.run_steps(6);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.inbox.iter().all(|i| i.read)), "marked read");
    h.run_steps(6);
    assert!(h.query_by_label("Benachrichtigungen: 1 neu").is_none(), "no count any more");
}

/// A worksheet: edit mode shows the tasks, work mode checks an answer and counts points; a
/// learning app shows its source and the open button.
#[test]
fn worksheet_work_mode_checks_answers_and_app_panel_shows_source() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let sheet = serde_json::json!({"title": "Brüche", "tasks": [
        {"id": "a1", "type": "choice", "prompt": "Welcher Bruch ist am größten?", "options": [{"text": "ein Halb"}, {"text": "drei Viertel", "correct": true}]},
        {"id": "a2", "type": "true_false", "prompt": "Zwei Viertel sind ein Halb.", "answer": true}
    ]});
    engine.call("worksheet.create", serde_json::json!({"worksheet": sheet})).unwrap();
    let mut h = harness(&engine);
    h.run_steps(6);
    assert!(h.query_by_label("+ Aufgabe hinzufügen  ⏷").is_some(), "edit mode first");
    h.get_by_label("Lösen").click();
    settled(&mut h, "drei Viertel");
    h.get_by_label("drei Viertel").click();
    h.run_steps(4);
    h.get_by_label("Prüfen").click();
    h.run_steps(6);
    assert!(h.query_by_label_contains("1 von 2 Punkte").is_some(), "score shown");
    assert!(h.query_by_label("✔ Richtig").is_some());
    h.get_by_label("Zurücksetzen").click();
    h.run_steps(4);
    assert!(h.query_by_label("✔ Richtig").is_none());
    // The print preview shows the rendered pages, for the worksheet and the solution sheet.
    h.get_by_label("Druckvorschau").click();
    h.run_steps(6);
    assert!(h.query_by_label("Seite 1 von 1").is_some(), "preview page shown");
    h.get_by_label("Lösungsblatt").click();
    h.run_steps(6);
    assert!(h.query_by_label("Seite 1 von 1").is_some(), "solution sheet preview");

    engine.call("learning_app.create", serde_json::json!({"html": "<!doctype html><html><head><title>Quiz</title></head><body>Hallo</body></html>"})).unwrap();
    h.run_steps(8);
    assert!(h.query_by_label("▶ Öffnen").is_some());
    assert!(h.query_by_label("Speichern …").is_some());
}

/// A worksheet without points (#72): the switch „Punkte vergeben“ in the editor hides the points
/// fields, the sheet is saved without points, and „Prüfen“ counts right tasks.
#[test]
fn worksheet_points_switch_hides_points() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let sheet = serde_json::json!({"title": "Brüche", "tasks": [
        {"id": "a1", "type": "choice", "prompt": "Welcher Bruch ist am größten?", "points": 2, "options": [{"text": "ein Halb"}, {"text": "drei Viertel", "correct": true}]},
        {"id": "a2", "type": "true_false", "prompt": "Zwei Viertel sind ein Halb.", "answer": true}
    ]});
    let id = engine.call("worksheet.create", serde_json::json!({"worksheet": sheet})).unwrap()["id"].as_str().unwrap().to_string();
    let mut h = harness(&engine);
    step_until(&mut h, "the editor", |h| h.query_by_label("Punkte vergeben").is_some());
    assert!(chosen(&h, "Punkte vergeben"), "on for a sheet with points");
    assert_eq!(h.query_all_by_label("Punkte").count(), 2, "a points field per task");
    click_until(&mut h, "Punkte vergeben", "the points fields hidden", |h| h.query_all_by_label("Punkte").count() == 0);
    assert!(!chosen(&h, "Punkte vergeben"));
    let text = |e: &Engine| e.call("artifact.get", serde_json::json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    step_until(&mut h, "saved without points", |_| text(&engine).contains("\"points\": false"));
    assert!(text(&engine).contains("\"points\": 2"), "task points kept");
    // Lösen: no points next to the task, and „Prüfen“ counts right tasks.
    h.get_by_label("Lösen").click();
    settled(&mut h, "drei Viertel");
    assert!(h.query_by_label("(2 Punkte)").is_none());
    h.get_by_label("drei Viertel").click();
    h.run_steps(4);
    h.get_by_label("Prüfen").click();
    step_until(&mut h, "the count", |h| h.query_by_label_contains("1 von 2 richtig").is_some());
    assert!(h.query_by_label_contains("Punkte").is_none(), "no points anywhere");
    // Switched back on: the fields are back with their points.
    h.get_by_label("Bearbeiten").click();
    click_until(&mut h, "Punkte vergeben", "the points fields back", |h| h.query_all_by_label("Punkte").count() == 2);
}

/// The look question: a fresh profile gets the whole welcome (look, then sign-in), a profile that
/// is set up already gets only the look step, once; with the flag set nothing opens. Picking a
/// look applies it at once. Both say that AIS.ChatCraft is a community experiment.
#[test]
fn welcome_asks_for_the_look_once() {
    use serde_json::json;
    let notice = "kein offizielles Werkzeug des AIS.chat-Teams";
    // Fresh: the full welcome, starting with the look.
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    assert!(h.query_by_label("Willkommen bei AIS.ChatCraft").is_some(), "full welcome");
    assert_eq!(engine.state().settings.theme, "modern", "Modern, like the system, is preselected");
    assert!(chosen(&h, "Wie das System"), "scheme: like the system");
    assert!(h.query_by_label_contains(notice).is_some(), "community notice");
    assert!(h.query_by_label("Weiter").is_some(), "the sign-in step follows");
    let e2 = engine.clone();
    click_until(&mut h, "Glas", "the choice applies at once", move |_| e2.state().settings.theme == "glas");
    click_until(&mut h, "Weiter", "sign-in step", |h| h.query_by_label("Mit einem Dienst verbinden").is_some());
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.settings.look_chosen), "asked once");

    // Set up already (two services, one signed in counts as set up), no flag: only the look.
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("backend.add", json!({"kind": "openai", "name": "Lokal", "base_url": "http://127.0.0.1:9/v1"})).unwrap();
    engine.call("settings.set", json!({"key": "theme", "value": "dark"})).unwrap();
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    assert!(h.query_by_label("AIS.ChatCraft sieht jetzt neu aus").is_some(), "look step only");
    assert!(h.query_by_label("Willkommen bei AIS.ChatCraft").is_none());
    assert!(h.query_by_label_contains(notice).is_some(), "community notice");
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.settings.theme == "modern"), "starts on Modern, like the system");
    h.run_steps(4);
    assert!(chosen(&h, "Wie das System"), "scheme: like the system");
    assert!(h.query_by_label("Klassisch").is_none(), "Klassisch is gone");
    h.get_by_label("Glas").click();
    h.run_steps(4);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.settings.theme == "glas"), "a look is one click away");
    h.get_by_label("Fertig").click();
    h.run_steps(4);
    assert!(h.query_by_label("AIS.ChatCraft sieht jetzt neu aus").is_none(), "closed");
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.settings.look_chosen));

    // Flag set: nothing opens.
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    assert!(h.query_by_label("AIS.ChatCraft sieht jetzt neu aus").is_none() && h.query_by_label("Willkommen bei AIS.ChatCraft").is_none());
}

/// Every look renders the main views without trouble, and the look switch in Einstellungen
/// applies live.
#[test]
fn every_look_renders_and_switches_live() {
    use serde_json::json;
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    for theme in aiscraft_engine::THEMES {
        engine.call("settings.set", json!({"key": "theme", "value": theme})).unwrap();
        h.run_steps(3);
        engine.call("artifact.create", json!({"kind": "board"})).unwrap();
        h.run_steps(3);
        engine.call("artifact.close", json!({})).unwrap();
    }
    engine.call("settings.set", json!({"key": "transparency", "value": "off"})).unwrap();
    engine.call("settings.set", json!({"key": "theme", "value": "glas-dark"})).unwrap();
    h.run_steps(3);
    let (tx, _rx) = std::sync::mpsc::channel::<aiscraft_ui::UiRequest>();
    drop(tx);
    engine.call("settings.set", json!({"key": "theme", "value": "modern"})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Neuer Chat").is_some(), "Modern's sidebar");
}

/// The Omarchy card shows where an Omarchy theme is there, and choosing it hides the scheme
/// switch (Omarchy follows the desktop). Uses a fake theme directory, never the real one.
#[test]
fn omarchy_card_only_with_an_omarchy_theme() {
    let dir = std::env::temp_dir().join(format!("aiscraft-omarchy-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("colors.toml"), "background = \"#1e1e2e\"\nforeground = \"#cdd6f4\"\naccent = \"#89b4fa\"\n").unwrap();
    let open = |engine: &Engine, omarchy: OmarchyTheme| {
        let e = engine.clone();
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .allow_missing_glyphs()
            .build_eframe(move |cc| common::app(cc, &e, None, omarchy.clone()));
        h.run_steps(4);
        h
    };
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = open(&engine, OmarchyTheme::at(&dir));
    assert!(h.query_by_label("Omarchy").is_some(), "card with an Omarchy theme");
    assert_eq!(engine.state().settings.theme, "modern", "Modern stays the default");
    h.get_by_label("Omarchy").click();
    h.run_steps(4);
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.settings.theme == "omarchy"));
    h.run_steps(4);
    assert!(h.query_by_label("Wie das System").is_none(), "no scheme switch for Omarchy");
    assert!(h.query_by_label_contains("bestimmt das Omarchy-Theme").is_some(), "a hint instead");

    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let h = open(&engine, OmarchyTheme::at(dir.join("fehlt")));
    assert!(h.query_by_label("Omarchy").is_none(), "no card without an Omarchy theme");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two apps side by side, one with an Omarchy theme and one without: each shows the welcome for
/// its own theme source, also while the other runs. A process-wide theme directory once let one
/// test reshape the welcome dialog under another test's click (#71).
#[test]
fn each_app_reads_its_own_omarchy_theme() {
    let dir = std::env::temp_dir().join(format!("aiscraft-omarchy-own-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("colors.toml"), "background = \"#1e1e2e\"\nforeground = \"#cdd6f4\"\naccent = \"#89b4fa\"\n").unwrap();
    let open = |omarchy: OmarchyTheme| {
        let e = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .allow_missing_glyphs()
            .build_eframe(move |cc| common::app(cc, &e, None, omarchy.clone()))
    };
    let mut with = open(OmarchyTheme::at(&dir));
    let mut without = open(OmarchyTheme::none());
    for _ in 0..4 {
        with.step();
        without.step();
        assert!(with.query_by_label("Omarchy").is_some(), "the card where the app has a theme");
        assert!(without.query_by_label("Omarchy").is_none(), "no card where it has none");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
