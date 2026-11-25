//! Own rules for the trust check (#22), in Einstellungen › Konnektoren: what runs without asking
//! and what never runs. Prohibitions win.

use aiscraft_engine::RuleKind;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

const DRAFT: &str = "trust-rule-draft";
const DRAFT_KIND: &str = "trust-rule-kind";
const DRAFT_CONNECTOR: &str = "trust-rule-connector";

impl AisCraftApp {
    pub(crate) fn trust_rules_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Eigene Regeln").strong());
        ui.label(small_muted(
            &t,
            "Die Vertrauensprüfung hält jede Aktion auch an deine Regeln. „Erlauben“ lässt Passendes ohne Nachfrage laufen (nie Löschendes oder Tresor-Einträge), „Verbieten“ lehnt es ab, auch wenn du darum bittest. Verbote gewinnen. Aufgaben und Zeitpläne bleiben bei Freigaben gesperrt.",
        ));
        let rules = self.state.settings.trust_rules.clone();
        for r in &rules {
            // Right to left: the button first, the rule takes the rest (and is cut, never wider).
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Entfernen").clicked() {
                        self.run("trust.rule_remove", json!({"id": r.id}));
                    }
                    let (word, color) = match r.kind {
                        RuleKind::Allow => ("Erlaubt", t.success),
                        RuleKind::Forbid => ("Verboten", t.danger),
                    };
                    let style = ui.style().clone();
                    let mut job = egui::text::LayoutJob::default();
                    let mut add = |text: RichText| text.append_to(&mut job, &style, egui::FontSelection::Default, egui::Align::Center);
                    add(RichText::new(format!("{word}  ")).small().strong().color(color));
                    if let Some(k) = self.state.connectors.iter().find(|k| k.id == r.connector) {
                        add(RichText::new(format!("{} · ", k.name)).color(t.muted));
                    }
                    add(RichText::new(r.text.as_str()).color(t.text));
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(job).truncate());
                    });
                });
            });
        }
        // The new rule: kind, connector, text, „Hinzufügen“ (buttons first, the field takes the rest).
        let (draft_id, kind_id, conn_id) = (egui::Id::unique(DRAFT), egui::Id::unique(DRAFT_KIND), egui::Id::unique(DRAFT_CONNECTOR));
        let mut text: String = ui.data_mut(|d| d.get_temp(draft_id).unwrap_or_default());
        let mut forbid: bool = ui.data_mut(|d| d.get_temp(kind_id).unwrap_or(false));
        let mut connector: String = ui.data_mut(|d| d.get_temp(conn_id).unwrap_or_default());
        ui.horizontal(|ui| {
            ui.radio_value(&mut forbid, false, "Erlauben");
            ui.radio_value(&mut forbid, true, "Verbieten");
            let shown = self.state.connectors.iter().find(|k| k.id == connector).map(|k| k.name.clone()).unwrap_or_else(|| "alle Konnektoren".into());
            egui::ComboBox::from_id_salt("trust-rule-connector-box").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut connector, String::new(), "alle Konnektoren");
                for k in &self.state.connectors {
                    ui.selectable_value(&mut connector, k.id.clone(), &k.name);
                }
            });
        });
        let mut add = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                add = ui.add_enabled(!text.trim().is_empty(), egui::Button::new("Hinzufügen")).clicked();
                let w = ui.available_width().max(40.0);
                let hint = if forbid { "z. B. nie Mails an Eltern senden" } else { "z. B. Lesen in Obsidian immer" };
                let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(w));
                add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty();
            });
        });
        if add {
            self.run("trust.rule_add", json!({"kind": if forbid { "forbid" } else { "allow" }, "text": text.trim(), "connector": connector}));
            text.clear();
        }
        ui.data_mut(|d| {
            d.insert_temp(draft_id, text);
            d.insert_temp(kind_id, forbid);
            d.insert_temp(conn_id, connector);
        });
    }
}
