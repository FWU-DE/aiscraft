//! Archive, trash and multi-select in the sidebar (#77), driven headless through egui's
//! accessibility tree (no window, no focus).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{click_until, step_until};
use egui_kittest::kittest::Queryable;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, AisCraftApp>;

fn harness(engine: &Engine) -> Harness {
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

/// A local OpenAI-compatible server that answers every request with „Antwort“.
fn backend(engine: &Engine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async {
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
                    if buf.starts_with(b"GET") {
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let c = json!({"choices":[{"delta":{"content": "Antwort"}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// An engine with one chat per title (the first one the oldest), on the start page.
fn engine_with_chats(titles: &[&str]) -> (Engine, Vec<String>) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = backend(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    let mut ids = Vec::new();
    for t in titles {
        engine.call("chat.new", json!({})).unwrap();
        engine.call("chat.send", json!({"text": t})).unwrap();
        ids.push(engine.state().current.clone().unwrap());
        // Distinct times, so the list's order is the order made.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    engine.call("chat.close", json!({})).unwrap();
    (engine, ids)
}

/// The sidebar's entry with this label (the start page may list the chat too, further right).
fn side<'a>(h: &'a Harness, label: &'a str) -> Option<egui_kittest::Node<'a>> {
    h.query_all_by_label(label).filter(|n| n.rect().left() < 300.0).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
}

fn in_sidebar(h: &Harness, label: &str) -> bool {
    side(h, label).is_some()
}

#[test]
fn right_click_archives_and_the_archive_restores() {
    let (engine, ids) = engine_with_chats(&["Kastanienfest planen", "Elternabend vorbereiten"]);
    let mut h = harness(&engine);
    step_until(&mut h, "both chats in the sidebar", |h| in_sidebar(h, "Kastanienfest planen") && in_sidebar(h, "Elternabend vorbereiten"));
    assert!(h.query_by_label_contains("Archiv (").is_none(), "no archive entry while nothing is archived");

    for _ in 0..5 {
        side(&h, "Kastanienfest planen").unwrap().click_secondary();
        h.run_steps(4);
        if h.query_by_label("Archivieren").is_some() {
            break;
        }
    }
    click_until(&mut h, "Archivieren", "the chat is archived", |_| engine.state().conversation(&ids[0]).is_some_and(|c| c.archived));
    step_until(&mut h, "the chat leaves the sidebar and „Zuletzt bearbeitet“", |h| h.query_all_by_label("Kastanienfest planen").next().is_none());
    assert!(in_sidebar(&h, "Elternabend vorbereiten"), "the other chat stays");

    // „Archiv (1)“ at the bottom of the sidebar opens the archive; „Wiederherstellen“ brings it back.
    click_until(&mut h, "Archiv (1)", "the archive page", |h| h.query_by_label("Nach Titel suchen").is_some() || h.query_by_label("Öffnen").is_some());
    step_until(&mut h, "the archived chat is listed", |h| {
        h.query_all_by_label("Kastanienfest planen").next().is_some() && h.query_by_label("Öffnen").is_some()
    });
    click_until(&mut h, "Wiederherstellen", "the chat is back", |_| engine.state().conversation(&ids[0]).is_some_and(|c| !c.archived));
    step_until(&mut h, "back in the sidebar", |h| in_sidebar(h, "Kastanienfest planen"));
}

#[test]
fn undo_after_delete_brings_the_chat_back() {
    let (engine, ids) = engine_with_chats(&["Wandertag organisieren"]);
    let mut h = harness(&engine);
    step_until(&mut h, "the chat in the sidebar", |h| in_sidebar(h, "Wandertag organisieren"));
    for _ in 0..5 {
        side(&h, "Wandertag organisieren").unwrap().click_secondary();
        h.run_steps(4);
        if h.query_all_by_label("Löschen").next().is_some() {
            break;
        }
    }
    click_until(&mut h, "Löschen", "the dialog asks", |h| h.query_all_by_label_contains("Papierkorb lässt es sich").next().is_some());
    // The menu has closed: the dialog's button is the only „Löschen“.
    step_until(&mut h, "the context menu closes", |h| h.query_all_by_label("Löschen").count() == 1);
    click_until(&mut h, "Löschen", "the chat is in the trash", |_| engine.state().trash.iter().any(|c| c.id == ids[0]));
    step_until(&mut h, "the toast offers „Rückgängig“", |h| {
        h.query_by_label("Rückgängig").is_some() && h.query_all_by_label_contains("Gelöscht").next().is_some()
    });
    assert!(!in_sidebar(&h, "Wandertag organisieren"), "gone from the sidebar");
    assert!(in_sidebar(&h, "Papierkorb (1)"), "the trash shows at the bottom");
    click_until(&mut h, "Rückgängig", "the chat is back", |_| engine.state().conversation(&ids[0]).is_some() && engine.state().trash.is_empty());
    step_until(&mut h, "back in the sidebar", |h| in_sidebar(h, "Wandertag organisieren"));
}

#[test]
fn multi_select_archives_several_chats_at_once() {
    let (engine, ids) = engine_with_chats(&["Erster Chat", "Zweiter Chat", "Dritter Chat"]);
    let mut h = harness(&engine);
    step_until(&mut h, "all chats in the sidebar", |h| ["Erster Chat", "Zweiter Chat", "Dritter Chat"].iter().all(|l| in_sidebar(h, l)));
    // A plain click opens; Umschalt+Klick picks the range up to there.
    side(&h, "Dritter Chat").unwrap().click();
    step_until(&mut h, "the chat opens", |_| engine.state().current.as_deref() == Some(ids[2].as_str()));
    h.run_steps(4);
    side(&h, "Erster Chat").unwrap().click_modifiers(egui::Modifiers::SHIFT);
    step_until(&mut h, "three picked", |h| h.query_by_label("3 ausgewählt").is_some());
    // Strg+Klick takes one out again.
    side(&h, "Zweiter Chat").unwrap().click_modifiers(egui::Modifiers::COMMAND);
    step_until(&mut h, "two picked", |h| h.query_by_label("2 ausgewählt").is_some());
    assert_eq!(engine.state().current.as_deref(), Some(ids[2].as_str()), "picking opens nothing");
    click_until(&mut h, "Archivieren", "both picked chats are archived", |_| {
        let s = engine.state();
        [&ids[0], &ids[2]].iter().all(|id| s.conversation(id).is_some_and(|c| c.archived))
    });
    let s = engine.state();
    assert!(!s.conversation(&ids[1]).unwrap().archived, "the one taken out stays");
    step_until(&mut h, "the selection ends", |h| h.query_by_label_contains("ausgewählt").is_none());
    assert!(in_sidebar(&h, "Zweiter Chat") && !in_sidebar(&h, "Erster Chat") && in_sidebar(&h, "Archiv (2)"));
}

/// Every entry a chat's menu can have, sidebar and header (#123).
const MENU_ENTRIES: [&str; 10] = [
    "Umbenennen …",
    "Anheften",
    "Lösen",
    "In Projekt",
    "Duplizieren",
    "Als Datei speichern",
    "In AIS.chat öffnen",
    "Archivieren",
    "Löschen",
    "Chat schließen (Esc)",
];

/// The entries of the menu on screen, in [`MENU_ENTRIES`] order.
fn menu_entries(h: &Harness) -> Vec<&'static str> {
    MENU_ENTRIES.iter().copied().filter(|l| h.query_by_label(l).is_some()).collect()
}

/// #123: the sidebar's „…“ (on the open chat, and on another while the pointer is over it) and
/// the header's „…“ open the same menu; the header adds „Chat schließen (Esc)“. Archiving from
/// the open chat goes back to the start page with „Rückgängig“; F2 and Entf on a focused row.
#[test]
fn the_sidebar_and_the_header_share_one_chat_menu() {
    let (engine, ids) = engine_with_chats(&["Kastanienfest planen", "Elternabend vorbereiten"]);
    engine.call("chat.open", json!({"id": ids[0]})).unwrap();
    let mut h = harness(&engine);
    step_until(&mut h, "the open chat's row shows its „…“", |h| {
        h.query_by_label("Chat-Aktionen").is_some() && h.query_by_label("Weitere Aktionen").is_some()
    });
    assert_eq!(h.query_all_by_label("Chat-Aktionen").count(), 1, "only the open chat's row, while nothing is hovered");
    // The row's „…“ lies inside the row, 20 px and more from the sidebar's right edge, right of the title.
    let more = h.get_by_label("Chat-Aktionen").rect();
    let rows: Vec<egui::Rect> = h.query_all_by_label("Kastanienfest planen").map(|n| n.rect()).collect();
    assert!(rows.iter().any(|row| row.contains_rect(more) && row.right() - more.right() >= 4.0), "the button {more:?} lies in its row {rows:?}");
    let sidebar = h.state().panel_layout().sidebar.unwrap();
    assert!(sidebar.right() - more.right() >= 20.0, "the button {more:?} keeps 20 px from the sidebar's edge {sidebar:?}");

    click_until(&mut h, "Chat-Aktionen", "the sidebar's menu opens", |h| h.query_by_label("Duplizieren").is_some());
    let sidebar_menu = menu_entries(&h);
    click_until(&mut h, "Chat-Aktionen", "the sidebar's menu closes", |h| h.query_by_label("Duplizieren").is_none());
    click_until(&mut h, "Weitere Aktionen", "the header's menu opens", |h| h.query_by_label("Duplizieren").is_some());
    let header_menu = menu_entries(&h);
    assert_eq!(sidebar_menu, ["Umbenennen …", "Anheften", "In Projekt", "Duplizieren", "Als Datei speichern", "Archivieren", "Löschen"], "the sidebar's menu");
    let mut expected = sidebar_menu.clone();
    expected.push("Chat schließen (Esc)");
    assert_eq!(header_menu, expected, "the header's menu is the sidebar's plus „Chat schließen“");

    // Archiving the open chat from its header: back to the start page, with „Rückgängig“.
    click_until(&mut h, "Archivieren", "the chat is archived", |_| engine.state().conversation(&ids[0]).is_some_and(|c| c.archived));
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| s.current.is_none()), "back to the start page");
    step_until(&mut h, "the toast offers „Rückgängig“", |h| h.query_by_label("Rückgängig").is_some());
    click_until(&mut h, "Rückgängig", "the chat is back", |_| engine.state().conversation(&ids[0]).is_some_and(|c| !c.archived));

    // Another chat's „…“ while the pointer is over its row.
    step_until(&mut h, "the chats in the sidebar", |h| in_sidebar(h, "Elternabend vorbereiten") && in_sidebar(h, "Kastanienfest planen"));
    side(&h, "Elternabend vorbereiten").unwrap().hover();
    step_until(&mut h, "the hovered row shows its „…“", |h| h.query_by_label("Chat-Aktionen").is_some());

    // F2 and Entf on the row under the pointer (no text field has the keys).
    h.key_press(egui::Key::F2);
    step_until(&mut h, "F2 renames", |h| h.query_by_label("Chat umbenennen").is_some());
    click_until(&mut h, "Abbrechen", "the dialog closes", |h| h.query_by_label("Chat umbenennen").is_none());
    side(&h, "Elternabend vorbereiten").unwrap().hover();
    step_until(&mut h, "the hovered row shows its „…“ again", |h| h.query_by_label("Chat-Aktionen").is_some());
    h.key_press(egui::Key::Delete);
    step_until(&mut h, "Entf asks to delete", |h| h.query_all_by_label_contains("Papierkorb lässt es sich").next().is_some());
}
