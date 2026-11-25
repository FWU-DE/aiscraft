//! Templates (#54) and connector results (#53) in the UI, driven headless (no window, no focus).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{click_until, step_until};
use egui::accesskit::Role;
use egui_kittest::kittest::{By, NodeT, Queryable};
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

/// What the composer holds.
fn composer(h: &egui_kittest::Harness<'static, AisCraftApp>) -> String {
    h.query_all(By::new().role(Role::MultilineTextInput)).filter_map(|n| n.accesskit_node().value().map(|v| v.to_string())).collect::<Vec<_>>().join("\n")
}

/// A filled template lands in the composer; the start page's template opens its form; text the
/// engine puts there is taken once; „/konnektor“ in the composer opens the connector dialog.
#[test]
fn templates_fill_the_composer_and_konnektor_opens_its_dialog() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("settings.set", json!({"key": "look_chosen", "value": true})).unwrap();
    let mut h = harness(&engine);
    let id = engine.state().templates.iter().find(|t| t.title == "Brief an die Eltern").unwrap().id.clone();
    engine.call("template.use", json!({"id": id, "values": {"Klasse": "7b", "Anlass": "Wandertag", "Angaben": "Montag, 8 Uhr"}})).unwrap();
    step_until(&mut h, "the filled text is in the composer", |h| composer(h).contains("Klasse 7b zum Anlass: Wandertag"));

    click_until(&mut h, "Brief an die Eltern", "the form opens", |h| h.query_by_label("Ins Eingabefeld").is_some());
    assert!(h.query_by_label("Klasse").is_some() && h.query_by_label("Anlass").is_some(), "one field per placeholder");
    click_until(&mut h, "Abbrechen", "the form closes", |h| h.query_by_label("Ins Eingabefeld").is_none());

    let k = engine.call("template.save", json!({"title": "Konnektor", "text": "/konnektor"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("template.use", json!({"id": k})).unwrap();
    step_until(&mut h, "the connector dialog opens", |h| h.query_by_label("Konnektor-Ergebnis einfügen").is_some());
    assert!(!composer(&h).contains("/konnektor"), "the command leaves the composer");
}
