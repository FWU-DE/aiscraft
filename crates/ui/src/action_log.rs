//! The action log under Einstellungen › Nutzung (#23): every model call and tool action, newest
//! first, with why it was allowed, and a CSV export.

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

/// How many entries the settings show (the export has all).
const SHOWN: u64 = 60;

pub(crate) fn section(app: &mut AisCraftApp, ui: &mut Ui) {
    let t = app.tokens.clone();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Als CSV exportieren …").clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("aktionsprotokoll.csv").save_file()
            {
                app.run("log.export", json!({"path": path.display().to_string()}));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(RichText::new("Aktionsprotokoll").strong());
            });
        });
    });
    ui.label(small_muted(
        &t,
        "Was getan wurde: jeder Modellaufruf und jede Werkzeugaktion, mit Angaben und warum sie erlaubt war. Zugangsdaten stehen nie darin.",
    ));
    let entries = match app.engine.call("log.list", json!({"limit": SHOWN})) {
        Ok(v) => v.get("entries").and_then(Value::as_array).cloned().unwrap_or_default(),
        Err(e) => {
            ui.label(RichText::new(e.message).color(t.danger));
            return;
        }
    };
    if entries.is_empty() {
        ui.label(small_muted(&t, "Noch nichts protokolliert."));
        return;
    }
    let s = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    // One entry per block: time and what in one line, the details and the reason under it
    // (wrapped, so nothing widens the dialog).
    for e in &entries {
        let at = e.get("at").and_then(Value::as_i64).unwrap_or(0);
        let outcome = s(e, "outcome");
        let color = match outcome.as_str() {
            "ok" => t.text,
            "uncertain" | "denied" | "skipped" => t.warning,
            _ => t.danger,
        };
        let detail = if s(e, "kind") == "model" {
            let n = |k: &str| e.get(k).and_then(Value::as_i64).map(|v| v.to_string()).unwrap_or_else(|| "–".into());
            let usd = e.get("usd").and_then(Value::as_f64).map(|u| format!(" · ≈ {u:.4} $")).unwrap_or_default();
            if e.get("input_tokens").and_then(Value::as_i64).is_none() {
                s(e, "detail")
            } else {
                format!("{} · {} / {} Tokens{usd}", s(e, "detail"), n("input_tokens"), n("output_tokens"))
            }
        } else {
            s(e, "detail")
        };
        let style = ui.style().clone();
        let mut job = egui::text::LayoutJob::default();
        let mut add = |text: RichText| text.append_to(&mut job, &style, egui::FontSelection::Default, egui::Align::Center);
        add(RichText::new(format!("{}  ", aiscraft_engine::format_local(at))).small().color(t.muted));
        add(RichText::new(s(e, "name")).small().strong().color(color));
        if outcome != "ok" {
            add(RichText::new(format!("  ({outcome})")).small().color(color));
        }
        ui.add(egui::Label::new(job).wrap());
        let reason = s(e, "reason");
        let line = match (detail.is_empty(), reason.is_empty()) {
            (false, false) => format!("{detail} — {reason}"),
            (false, true) => detail,
            (true, _) => reason,
        };
        if !line.is_empty() {
            ui.add(egui::Label::new(small_muted(&t, line.chars().take(300).collect::<String>())).wrap());
        }
        ui.add_space(4.0);
    }
}
