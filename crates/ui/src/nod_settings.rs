//! Settings sections for features taken from Nod (AGENTS.md §5), kept out of `dialogs.rs`: the
//! writing style (#20).

use aiscraft_core::MemoryArea;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

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
}
