//! „Entdecken“ › „Filter und Sortierung“: AIS.chat's tabs, sort orders and filter panel in the
//! UI, against a scripted AIS.chat. Every click goes through the `explore.*` commands.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_core::{ExploreSort, ExploreSource};
use aiscraft_engine::{Engine, Options};
use aiscraft_ui::theme::OmarchyTheme;
use common::{Harness, step_until};

use egui_kittest::kittest::Queryable;
use serde_json::json;

fn x(h: &Harness, label: &str) -> f32 {
    h.get_by_label(label).rect().min.x
}

/// Tabs, sort order with a popularity tie, OR within and AND across filter groups, the chips
/// with ×, „N Treffer“, „Zurücksetzen“ and the search, as a person clicks them.
#[test]
fn explore_filters_and_sorting_in_the_ui() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let base = common::scripted_ais_lists(&engine, common::filter_catalog());
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).unwrap();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "demo"})).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, Some(rx), OmarchyTheme::none()));
    h.run_steps(4);
    for close in ["Später", "Fertig"] {
        if let Some(b) = h.query_by_label(close) {
            b.click();
            h.run_steps(4);
        }
    }
    let (reply, _answer) = std::sync::mpsc::channel();
    tx.send(aiscraft_ui::UiRequest::Show { what: "personas".into(), reply }).unwrap();
    step_until(&mut h, "the cards", |h| h.query_by_label("Mathe-Coach").is_some() && h.query_by_label("Elternbrief-Helfer").is_some());

    // Newest first, like AIS.chat.
    assert!(x(&h, "Mathe-Coach") < x(&h, "Korrekturhilfe Deutsch") && x(&h, "Korrekturhilfe Deutsch") < x(&h, "Elternbrief-Helfer"));

    // The section opens; „Schulintern“ is there because something is shared within the school.
    h.query_by_label_contains("Filter und Sortierung").unwrap().click();
    step_until(&mut h, "the section opens", |h| h.query_by_label_contains("Schulintern").is_some() && h.query_by_label("Schulform").is_some());
    assert!(h.query_by_label("Sprache").is_some() && h.query_by_label("Bundesland").is_none(), "only groups some entry has values for");

    // Tabs.
    h.query_by_label_contains("Offiziell").unwrap().click();
    step_until(&mut h, "only official ones", |h| {
        engine.state().explore.source == ExploreSource::Official
            && h.query_by_label("Mathe-Coach").is_none()
            && h.query_by_label("Elternbrief-Helfer").is_some()
    });
    assert!(h.query_by_label("Marie Curie").is_none(), "community, not official");
    h.query_by_label_contains("Schulintern").unwrap().click();
    step_until(&mut h, "only those shared within the school", |h| {
        h.query_by_label("Korrekturhilfe Deutsch").is_some() && h.query_by_label("Elternbrief-Helfer").is_none() && h.query_by_label("Marie Curie").is_none()
    });
    engine.call("explore.filter", json!({"source": "all"})).unwrap();
    step_until(&mut h, "all again", |h| h.query_by_label("Mathe-Coach").is_some());

    // Sort order: popularity; Mathe-Coach and Korrekturhilfe tie at 30, the newer first.
    h.get_by_label("Sortierung").click();
    step_until(&mut h, "the sort menu", |h| h.query_by_label("Beliebteste zuerst").is_some());
    h.get_by_label("Beliebteste zuerst").click();
    step_until(&mut h, "sorted by popularity", |h| engine.state().explore.sort == ExploreSort::Popularity && x(h, "Elternbrief-Helfer") < x(h, "Mathe-Coach"));
    assert!(x(&h, "Mathe-Coach") < x(&h, "Korrekturhilfe Deutsch"), "a tie goes to the newest");

    // Filter values: one school type …
    h.get_by_label("Schulform").click();
    step_until(&mut h, "the school types", |h| h.query_by_label("Gymnasium (2)").is_some());
    h.get_by_label("Gymnasium (2)").click();
    step_until(&mut h, "filtered by school type", |h| engine.state().explore.active_count() == 1 && h.query_by_label("2 Treffer").is_some());
    assert!(h.query_by_label("Elternbrief-Helfer").is_none() && h.query_by_label("Mathe-Coach").is_some());
    // … OR another one of the same group …
    h.get_by_label("Realschule (1)").click();
    step_until(&mut h, "two school types", |_| engine.state().explore.active_count() == 2);
    assert!(h.query_by_label("2 Treffer").is_some(), "either school type is enough");
    // … AND a subject.
    h.get_by_label("Assistenten, Dialogpartner & Lernszenarien").click();
    step_until(&mut h, "the menu closes", |h| h.query_by_label("Gymnasium (2)").is_none());
    h.get_by_label("Fach").click();
    step_until(&mut h, "the subjects", |h| h.query_by_label("Deutsch (1)").is_some());
    h.get_by_label("Deutsch (1)").click();
    step_until(&mut h, "both groups must match", |h| h.query_by_label("1 Treffer").is_some());
    assert!(h.query_by_label("Mathe-Coach").is_none() && h.query_by_label("Korrekturhilfe Deutsch").is_some());
    h.get_by_label("Assistenten, Dialogpartner & Lernszenarien").click();
    step_until(&mut h, "the menu closes", |h| h.query_by_label("Deutsch (1)").is_none());

    // The chips: × takes one value away, „Zurücksetzen“ all.
    h.get_by_label("Deutsch").click();
    step_until(&mut h, "the subject is gone", |h| h.query_by_label("2 Treffer").is_some());
    h.get_by_label("Zurücksetzen").click();
    step_until(&mut h, "no filter values", |h| engine.state().explore.active_count() == 0 && h.query_by_label("Elternbrief-Helfer").is_some());
    assert!(h.query_by_label_contains(" Treffer").is_none());

    // The search goes to the engine too.
    h.get_by_label("Strg+F").click();
    h.event(egui::Event::Text("weimar".into()));
    step_until(&mut h, "only Goethe", |h| engine.state().explore.query == "weimar" && h.query_by_label("Mathe-Coach").is_none());
    assert!(h.query_by_label("Marie Curie").is_none() && h.query_by_label("Elternbrief-Helfer").is_none());
}
