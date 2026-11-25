//! Helpers shared by the UI tests: wait for conditions, not for a fixed number of frames (under
//! heavy load a frame count is sometimes not enough, issue #71).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_ui::AisCraftApp;
use egui_kittest::kittest::{NodeT, Queryable};

pub type Harness = egui_kittest::Harness<'static, AisCraftApp>;

/// The app as the tests open it: no Omarchy theme, whatever the desktop has, unless a test gives
/// one. Each app has its own theme source, so tests running side by side can't change each
/// other's screen (#71).
pub fn app(
    cc: &eframe::CreationContext<'_>,
    engine: &aiscraft_engine::Engine,
    control: Option<std::sync::mpsc::Receiver<aiscraft_ui::UiRequest>>,
    omarchy: aiscraft_ui::theme::OmarchyTheme,
) -> AisCraftApp {
    AisCraftApp::new(cc, engine.clone(), control).with_omarchy_theme(omarchy)
}

/// Whether the control with this label is the chosen one (selected or toggled on).
pub fn chosen(h: &egui_kittest::Harness<'static, AisCraftApp>, label: &str) -> bool {
    h.query_all(
        egui_kittest::kittest::By::new().label(label).predicate(|n| n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)),
    )
    .next()
    .is_some()
}

/// Draws frames until `done` holds (at most about 10 s). Engine commands run in the background,
/// so under load a fixed number of frames is sometimes not enough.
pub fn step_until(h: &mut egui_kittest::Harness<'static, AisCraftApp>, what: &str, done: impl Fn(&egui_kittest::Harness<'static, AisCraftApp>) -> bool) {
    for _ in 0..500 {
        if done(h) {
            return;
        }
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for: {what}: {}", screen(h));
}

/// Draws frames until a widget with this label is there and has stopped moving, so a click hits
/// it and not where it was a frame earlier.
pub fn settled(h: &mut egui_kittest::Harness<'static, AisCraftApp>, label: &str) {
    let mut last = None;
    for _ in 0..500 {
        let now = h.query_by_label(label).map(|n| n.rect());
        if now.is_some() && now == last {
            return;
        }
        last = now;
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("„{label}“ did not settle: {}", screen(h));
}

/// Clicks `label` until `done` holds. Under heavy load (many test processes at once) a simulated
/// click is now and then not taken; what the click must achieve is still checked.
pub fn click_until(
    h: &mut egui_kittest::Harness<'static, AisCraftApp>,
    label: &str,
    what: &str,
    done: impl Fn(&egui_kittest::Harness<'static, AisCraftApp>) -> bool,
) {
    for _ in 0..5 {
        settled(h, label);
        h.get_by_label(label).click();
        for _ in 0..100 {
            h.step();
            if done(h) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    panic!("clicking „{label}“ did not lead to: {what}: {}", screen(h));
}

/// Every labelled node with its rect, for failure messages: what was on screen instead.
pub fn screen(h: &egui_kittest::Harness<'static, AisCraftApp>) -> String {
    let labels: Vec<String> = h
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.label().is_some()))
        .map(|n| format!("{:?}@{:?}", n.accesskit_node().label().unwrap_or_default(), n.rect()))
        .collect();
    labels.join(" | ")
}
