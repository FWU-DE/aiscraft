//! The approval card („Darf ich das ausführen?“, #67): the action and the few values to check in
//! bold, the trust check's verdict, the full arguments folded under „Alle Angaben“. The yes is
//! bound to exactly these arguments, as before.

use aiscraft_engine::Approval;
use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};

use crate::theme::Tokens;
use crate::widgets::{accent_button, small_muted};

/// The sentence of the card: „Kalender · Termin verschieben: Titel **Elternabend 7b**, Beginn
/// **Sa, 12. Dez. 2026, 19:00**“, one label so it wraps as text.
pub(crate) fn sentence(ui: &Ui, t: &Tokens, a: &Approval) -> egui::text::LayoutJob {
    let style = ui.style().clone();
    let mut job = egui::text::LayoutJob::default();
    let mut add = |text: RichText| text.append_to(&mut job, &style, egui::FontSelection::Default, egui::Align::Center);
    add(crate::theme::semibold(a.label.as_str()).color(t.text));
    for (i, h) in a.highlights.iter().enumerate() {
        add(RichText::new(if i == 0 { ": " } else { ", " }).color(t.muted));
        add(RichText::new(format!("{} ", h.label)).color(t.muted));
        add(crate::theme::semibold(h.value.as_str()).color(t.text));
    }
    job
}

/// One card. Returns `Some(true)` for „Einmal erlauben“, `Some(false)` for „Ablehnen“.
pub(crate) fn card(ui: &mut Ui, t: &Tokens, a: &Approval) -> Option<bool> {
    let mut answer = None;
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.5, if a.destructive { t.danger } else { t.accent }))
        .corner_radius(CornerRadius::same(t.radius))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.label(RichText::new("Darf ich das ausführen?").strong());
            let job = sentence(ui, t, a);
            ui.add(egui::Label::new(job).wrap());
            if a.destructive {
                ui.label(RichText::new("Der Konnektor kennzeichnet das als verändernd oder löschend.").color(t.danger).small());
            }
            if let Some(tr) = &a.trust {
                if tr.covered {
                    ui.label(RichText::new(format!("Deine Bitte deckt das: „{}“", tr.quote)).color(t.success).small());
                } else {
                    ui.label(RichText::new(format!("Nicht von deinen Worten gedeckt: {}", tr.reason)).color(t.warning).small());
                }
            }
            egui::CollapsingHeader::new(small_muted(t, "Alle Angaben")).id_salt(("approval-args", &a.id)).default_open(a.highlights.is_empty()).show(
                ui,
                |ui| {
                    let args = serde_json::to_string_pretty(&a.arguments).unwrap_or_default();
                    Frame::new().fill(t.code_bg).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(args.chars().take(4000).collect::<String>()).monospace().small()).wrap());
                    });
                },
            );
            ui.horizontal_wrapped(|ui| {
                if accent_button(ui, t, "Einmal erlauben").clicked() {
                    answer = Some(true);
                }
                if ui.button("Ablehnen").clicked() {
                    answer = Some(false);
                }
                ui.label(small_muted(t, "Gilt nur für genau diese Angaben."));
            });
        });
    answer
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use aiscraft_engine::{Approval, Highlight};
    use egui_kittest::kittest::Queryable as _;
    use serde_json::json;

    fn approval(args: serde_json::Value) -> Approval {
        let schema = json!({});
        Approval {
            id: "a1".into(),
            conversation_id: "c".into(),
            tool: "k__send".into(),
            label: "Schulmail · Mail senden".into(),
            highlights: aiscraft_engine::highlights::highlights(&args, &schema),
            arguments: args,
            destructive: false,
            trust: None,
        }
    }

    /// The values to check are in the sentence; the JSON is folded until „Alle Angaben“ opens.
    #[test]
    fn card_shows_the_values_to_check_and_folds_the_json() {
        let a = approval(json!({"an": "kollegium@schule.de", "betreff": "Konferenz am 4. Dezember", "text": "Liebe Kolleginnen und Kollegen, GEHEIMER TEXT"}));
        assert_eq!(
            a.highlights,
            [
                Highlight { label: "An".into(), value: "kollegium@schule.de".into() },
                Highlight { label: "Betreff".into(), value: "Konferenz am 4. Dezember".into() }
            ]
        );
        let t = crate::theme::Tokens::modern_light();
        let answer = std::cell::Cell::new(None);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 400.0)).build_ui(|ui| {
            // The app's fonts (with „semibold“) apply from the next frame on.
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            if let Some(x) = super::card(ui, &t, &a) {
                answer.set(Some(x));
            }
        });
        h.run();
        // One label, so it wraps as text; the values are in it.
        let _ = h.get_by_label_contains("Schulmail · Mail senden: An kollegium@schule.de, Betreff Konferenz am 4. Dezember");
        assert!(h.query_by_label_contains("GEHEIMER TEXT").is_none(), "the JSON is folded");
        h.get_by_label("Alle Angaben").click();
        h.run();
        assert!(h.query_by_label_contains("GEHEIMER TEXT").is_some(), "„Alle Angaben“ shows the full arguments");
        h.get_by_label("Einmal erlauben").click();
        h.run();
        drop(h);
        assert_eq!(answer.get(), Some(true));
    }

    /// Without values worth highlighting, the arguments are open from the start.
    #[test]
    fn card_without_highlights_shows_the_arguments() {
        let a = approval(json!({"x": "nur das"}));
        assert!(a.highlights.is_empty());
        let t = crate::theme::Tokens::modern_light();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(700.0, 400.0)).build_ui(|ui| {
            // The app's fonts (with „semibold“) apply from the next frame on.
            if ui.ctx().cumulative_pass_nr() == 0 {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            let _ = super::card(ui, &t, &a);
        });
        h.run();
        assert!(h.query_by_label_contains("nur das").is_some());
    }
}
