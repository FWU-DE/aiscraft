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
    h.get_by_label_contains("Quellen").click();
    h.run_steps(4);
    h.get_by_label("Gedächtnis (deine Notizen)").click();
    h.run_steps(6);
    assert!(h.query_by_label("Gedächtnis (deine Notizen)").is_some(), "the popup stays open after a switch");
    assert!(engine.wait_for(std::time::Duration::from_secs(5), |s| !s.selection.sources.memory), "the switch reached the engine");
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
