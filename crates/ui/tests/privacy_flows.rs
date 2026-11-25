//! The setup of „Namen schützen“ (#73), driven headless through egui's accessibility tree.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::time::Duration;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use common::{click_until, step_until};
use egui::accesskit::Role;
use egui_kittest::kittest::{By, NodeT, Queryable};

const INTRO: &str = "So schützt AIS.ChatCraft Namen";
const LISTS: &str = "Klassenlisten (freiwillig)";
const MODEL: &str = "Namenserkennung (freiwillig)";
const OPTIONS: &str = "Weitere Einstellungen";
const SUMMARY: &str = "Zusammenfassung";

fn harness(engine: &Engine) -> egui_kittest::Harness<'static, AisCraftApp> {
    engine.call("settings.set", serde_json::json!({"key": "look_chosen", "value": true})).unwrap();
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    // A fresh profile gets the welcome first ("Später").
    if let Some(later) = h.query_by_label("Später") {
        later.click();
        h.run_steps(4);
    }
    h
}

fn on_step(h: &egui_kittest::Harness<'static, AisCraftApp>, title: &str) -> bool {
    h.query_by_label(title).is_some()
}

/// Types into the setup's name field: the text field between „Klassenliste einlesen …“ and
/// „Als Klassenliste speichern“ (the composer behind the dialog is a text field too).
fn type_names(h: &mut egui_kittest::Harness<'static, AisCraftApp>, text: &str) {
    let field = |h: &egui_kittest::Harness<'static, AisCraftApp>| {
        let top = h.get_by_label("Klassenliste einlesen …").rect().bottom();
        let save = h.get_by_label("Als Klassenliste speichern").rect();
        h.query_all(By::new().role(Role::MultilineTextInput))
            .find(|n| n.rect().center().y > top && n.rect().center().y < save.top() && (n.rect().left() - save.left()).abs() < 2.0)
            .unwrap_or_else(|| panic!("the name field: {}", common::screen(h)))
            .rect()
    };
    let typed = |h: &egui_kittest::Harness<'static, AisCraftApp>| {
        let r = field(h);
        h.query_all(By::new().role(Role::MultilineTextInput)).find(|n| n.rect() == r).and_then(|n| n.accesskit_node().value().map(|v| v.to_string()))
    };
    // Under load a simulated click is now and then not taken; type again only into an empty
    // field.
    for _ in 0..5 {
        let r = field(h);
        h.query_all(By::new().role(Role::MultilineTextInput)).find(|n| n.rect() == r).unwrap().click();
        h.run_steps(2);
        let r = field(h);
        h.query_all(By::new().role(Role::MultilineTextInput)).find(|n| n.rect() == r).unwrap().type_text(text);
        h.run_steps(2);
        match typed(h).as_deref() {
            Some(t) if t == text => return,
            Some(t) if !t.is_empty() => break,
            _ => {}
        }
    }
    panic!("typing into the name field: {:?}", typed(h));
}

#[test]
fn privacy_setup_opens_the_first_time_and_walks_through() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    assert!(!engine.state().protection_on());
    assert!(!on_step(&h, INTRO), "not before protection is switched on");

    // Switching on the first time: protection is on and the setup explains it.
    click_until(&mut h, "Namen schützen", "the setup opens", |h| on_step(h, INTRO));
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.protection_on()), "switched on");
    assert!(h.query_by_label_contains("Was auf diesem Gerät bleibt").is_some());

    // Forward, back, forward.
    click_until(&mut h, "Weiter", "the list step", |h| on_step(h, LISTS));
    click_until(&mut h, "Zurück", "back to the start", |h| on_step(h, INTRO));
    click_until(&mut h, "Weiter", "the list step again", |h| on_step(h, LISTS));

    // Names typed here become a class list when going on.
    type_names(&mut h, "Lea Müller");
    click_until(&mut h, "Weiter", "the model step", |h| on_step(h, MODEL));
    assert!(
        engine.wait_for(Duration::from_secs(5), |s| s.privacy.lists.len() == 1 && s.privacy.lists[0].count == 1),
        "the class list reached the engine: {:?}",
        engine.state().privacy.lists
    );

    // The model step asks before anything is loaded; skipping it loads nothing.
    assert!(h.query_by_label("Ja, laden").is_none(), "no download without the question");
    click_until(&mut h, "Überspringen", "the options step", |h| on_step(h, OPTIONS));
    let ner = engine.state().privacy.ner.clone();
    assert!(ner.progress.is_none() && !ner.installed, "nothing downloaded: {ner:?}");

    // The chat's switch: the setup sets it, the engine keeps it.
    assert!(h.query_by_label_contains("Was immer gilt").is_some());
    click_until(&mut h, "Automatisch", "automatic", |_| engine.state().protect_choice().is_none());
    click_until(&mut h, "An", "on", |_| engine.state().protect_choice() == Some(true));
    click_until(&mut h, "Weiter", "the summary", |h| on_step(h, SUMMARY));
    assert!(h.query_by_label("1 Liste mit 1 Namen").is_some(), "the summary names the list");
    click_until(&mut h, "Fertig", "the setup closes", |h| !on_step(h, SUMMARY));
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.settings.privacy_onboarded), "remembered in the engine");

    // Off and on again: no setup any more.
    click_until(&mut h, "Namen geschützt", "switched off", |_| !engine.state().protection_on());
    click_until(&mut h, "Namen schützen", "switched on", |_| engine.state().protection_on());
    h.run_steps(10);
    assert!(!on_step(&h, INTRO), "the setup does not come again");
}

#[test]
fn cancelling_the_setup_leaves_protection_as_it_was() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = harness(&engine);
    let before = engine.state().protect_choice();
    click_until(&mut h, "Namen schützen", "the setup opens", |h| on_step(h, INTRO));
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.protection_on()));
    click_until(&mut h, "Überspringen", "the list step", |h| on_step(h, LISTS));
    type_names(&mut h, "Jonas Schmidt");
    click_until(&mut h, "Als Klassenliste speichern", "the list is saved", |_| engine.state().privacy.lists.len() == 1);
    click_until(&mut h, "Einrichtung abbrechen", "the setup closes", |h| !on_step(h, LISTS));
    assert!(
        engine.wait_for(Duration::from_secs(5), |s| s.privacy.lists.is_empty() && s.protect_choice() == before && !s.protection_on()),
        "the switch and the lists as before: {:?}",
        engine.state().privacy.lists
    );
    assert!(!engine.state().settings.privacy_onboarded, "not done: the setup comes again next time");
    click_until(&mut h, "Namen schützen", "the setup opens again", |h| on_step(h, INTRO));
}

/// The app with Einstellungen › Namen schützen open.
fn privacy_settings(engine: &Engine, ready: &str) -> egui_kittest::Harness<'static, AisCraftApp> {
    let (tx, rx) = std::sync::mpsc::channel();
    let e = engine.clone();
    let mut rx = Some(rx);
    engine.call("settings.set", serde_json::json!({"key": "look_chosen", "value": true})).unwrap();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, rx.take(), OmarchyTheme::none()));
    h.run_steps(4);
    if let Some(later) = h.query_by_label("Später") {
        later.click();
        h.run_steps(4);
    }
    let (reply, _r) = std::sync::mpsc::channel();
    tx.send(aiscraft_ui::UiRequest::Show { what: "privacy".into(), reply }).unwrap();
    step_until(&mut h, "the settings tab", |h| h.query_by_label(ready).is_some());
    h
}

/// A first class list saved in the settings switches protection on: the setup follows on its
/// list step, and finishing goes back to the settings.
#[test]
fn first_list_in_the_settings_opens_the_setup() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let mut h = privacy_settings(&engine, "Einrichtung starten");
    type_names(&mut h, "Lea Müller");
    click_until(&mut h, "Als Klassenliste speichern", "the setup on its list step", |h| on_step(h, LISTS));
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.privacy.lists.len() == 1 && s.protection_on()), "the list switched protection on");
    for title in [MODEL, OPTIONS, SUMMARY] {
        click_until(&mut h, "Weiter", title, |h| on_step(h, title));
    }
    click_until(&mut h, "Fertig", "back in the settings", |h| h.query_by_label("Einrichtung erneut durchlaufen").is_some());
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.settings.privacy_onboarded));
    assert_eq!(engine.state().privacy.lists.len(), 1, "the list stays");
}

#[test]
fn privacy_setup_from_the_settings_can_be_run_again() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("privacy.onboarding_done", serde_json::json!({})).unwrap();
    let mut h = privacy_settings(&engine, "Einrichtung erneut durchlaufen");
    click_until(&mut h, "Einrichtung erneut durchlaufen", "the setup opens", |h| on_step(h, INTRO));
    click_until(&mut h, "Einrichtung abbrechen", "back in the settings", |h| h.query_by_label("Einrichtung erneut durchlaufen").is_some());
    assert!(engine.state().settings.privacy_onboarded, "cancelling a repeat keeps it done");
}
