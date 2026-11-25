//! Settings sections for features taken from Nod (AGENTS.md §5), kept out of `dialogs.rs`: the
//! writing style (#20), briefing templates for schedules (#27).

use aiscraft_core::MemoryArea;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{card, small_muted};

impl AisCraftApp {
    /// Einstellungen › Gedächtnis: the writing style learned from the owner's own messages (#20).
    pub(crate) fn style_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new(MemoryArea::Style.label()).strong());
        ui.label(small_muted(
            &t,
            "AIS.ChatCraft lernt aus deinen eigenen Nachrichten, wie du schreibst, damit Entwürfe (Mails, Elternbriefe, Antworten) nach dir klingen. Namen und Zahlen werden vorher entfernt; Antworten, Dateien und Werkzeug-Ergebnisse zählen nicht. Der Stil geht nur bei Schreibaufträgen mit und lässt sich pro Chat unter „Quellen“ abschalten.",
        ));
        let note = self.state.memory.iter().rev().find(|n| n.area == MemoryArea::Style && !n.archived).cloned();
        if let Some(n) = &note {
            ui.add(egui::Label::new(RichText::new(&n.text)).wrap()).on_hover_text(&n.source_quote);
        }
        ui.horizontal_wrapped(|ui| {
            let label = if note.is_some() { "Schreibstil neu lernen" } else { "Schreibstil aus meinen Nachrichten lernen" };
            if ui.button(label).clicked() {
                self.run("style.learn", json!({}));
            }
            if let Some(n) = &note
                && ui.small_button("Vergessen").clicked()
            {
                self.run("memory.archive", json!({"id": n.id}));
            }
        });
    }

    /// Einstellungen › Zeitpläne: briefing templates (#27), each with what it still needs.
    pub(crate) fn schedule_templates_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Vorlagen").strong());
        ui.label(small_muted(&t, "Mit einem Klick als Zeitplan anlegen; danach lässt er sich wie jeder andere ändern."));
        for tpl in aiscraft_engine::template_views(&self.state) {
            card(&t).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tpl.title).strong());
                    ui.label(small_muted(&t, tpl.rhythm));
                });
                ui.add(egui::Label::new(small_muted(&t, tpl.prompt.replace("{url}", "…"))).wrap());
                let url_id = egui::Id::new(("schedule-template-url", tpl.id));
                let mut url: String = ui.data_mut(|d| d.get_temp::<String>(url_id)).unwrap_or_default();
                let ready = tpl.missing.is_empty() && (!tpl.needs_url || !url.trim().is_empty());
                let mut create = false;
                if tpl.needs_url {
                    crate::widgets::field_then_buttons(
                        ui,
                        |ui| create = ui.add_enabled(ready, egui::Button::new("Anlegen")).clicked(),
                        |ui, w| ui.add(egui::TextEdit::singleline(&mut url).hint_text("Adresse der Seite, https://…").desired_width(w)),
                    );
                    ui.data_mut(|d| d.insert_temp(url_id, url.clone()));
                } else {
                    create = ui.add_enabled(ready, egui::Button::new("Anlegen")).clicked();
                }
                for m in &tpl.missing {
                    ui.add(egui::Label::new(RichText::new(format!("Fehlt – {m}")).small().color(t.warning)).wrap());
                }
                if create {
                    let mut p = json!({"template": tpl.id});
                    if tpl.needs_url {
                        p["url"] = json!(url.trim());
                    }
                    self.run("schedule.from_template", p);
                    ui.data_mut(|d| d.remove::<String>(url_id));
                }
            });
            ui.add_space(4.0);
        }
    }
}
