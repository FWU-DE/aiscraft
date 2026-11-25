//! Under an answer: „Speichern als …“ (Word, PDF, Excel, Markdown, Kalenderdatei; #46, #51),
//! „Lesbarkeit“ (#48) and „Mit Kennzeichnung kopieren“ (#57). Each item dispatches a command.

use aiscraft_core::Message;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::theme::Tokens;

impl AisCraftApp {
    /// The two pills in the row of actions under an answer.
    pub(crate) fn answer_tools(&mut self, ui: &mut Ui, t: &Tokens, m: &Message) {
        use crate::icons::Icon;
        use crate::kit::{Pill, pill};
        let r = pill(ui, t, Pill::Ghost, Some(Icon::Download), "Speichern als", 13.0, Some(Icon::ChevronDown))
            .on_hover_text("Antwort als Word, PDF, Excel (Tabellen), Markdown oder Kalenderdatei (Termine) speichern");
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(220.0);
            // Looked at only while the menu is open.
            let tables = !aiscraft_engine::office::message::markdown_tables(&m.content).is_empty();
            let events = aiscraft_engine::learn::calendar::events_in(&m.content).0.len();
            let base = aiscraft_engine::office::message::title_of(&m.content);
            for (label, format, ext, enabled, tip) in [
                ("Word (.docx)", "docx", "docx", true, "Zum Weiterbearbeiten in Word, LibreOffice oder Pages"),
                ("PDF", "pdf", "pdf", true, "Zum Drucken und Verschicken"),
                ("Excel (.xlsx)", "xlsx", "xlsx", tables, "Die Tabellen der Antwort, eine pro Blatt"),
                ("Markdown (.md)", "markdown", "md", true, "Der Text mit Formatierung als einfache Textdatei"),
            ] {
                let b = ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(tip).on_disabled_hover_text("Die Antwort enthält keine Tabelle");
                if b.clicked() {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new().set_file_name(aiscraft_engine::office::file_name(&base, ext)).save_file() {
                        self.run("message.export", json!({"message_id": m.id, "format": format, "path": path.display().to_string()}));
                    }
                }
            }
            let cal = ui
                .add_enabled(events > 0, egui::Button::new(if events > 0 { format!("Kalenderdatei (.ics), {events} Termine") } else { "Kalenderdatei (.ics)".into() }))
                .on_hover_text("Die Termine der Antwort für Outlook, Google, Apple oder Thunderbird")
                .on_disabled_hover_text("Die Antwort enthält keine Termine (ics-Block oder Tabelle mit Datum)");
            if cal.clicked() {
                ui.close();
                if let Some(path) = rfd::FileDialog::new().set_file_name(aiscraft_engine::office::file_name(&base, "ics")).save_file() {
                    self.run("calendar.save", json!({"from_message": m.id, "title": base, "path": path.display().to_string()}));
                }
            }
        });
        let shown = self.state.readability.contains_key(&m.id);
        if pill(ui, t, if shown { Pill::Soft } else { Pill::Ghost }, None, "Lesbarkeit", 13.0, None)
            .on_hover_text("Wortzahl, Lesbarkeit (Flesch, LIX), etwa welche Klassenstufe, lange Sätze. Wird auf diesem Rechner berechnet.")
            .clicked()
        {
            self.run("message.readability", json!({"message_id": m.id, "show": !shown}));
        }
    }

    /// The readability line under an answer, when it was asked for.
    pub(crate) fn readability_line(&mut self, ui: &mut Ui, t: &Tokens, m: &Message) {
        let Some(r) = self.state.readability.get(&m.id) else { return };
        let mut hover = String::from(
            "Flesch: 0 sehr schwer bis 100 sehr leicht (Leichte Sprache etwa ab 70). LIX: unter 40 leicht, über 50 schwer. Die Klassenstufe ist eine Schätzung (Wiener Sachtextformel).",
        );
        if !r.long_examples.is_empty() {
            hover.push_str("\n\nLange Sätze:");
            for e in &r.long_examples {
                hover.push_str("\n• ");
                hover.push_str(e);
            }
        }
        ui.add(egui::Label::new(RichText::new(r.summary()).small().color(t.muted)).wrap()).on_hover_text(hover);
    }

    /// Context menu of „Kopieren“: with the AI label (#57).
    pub(crate) fn copy_menu(&mut self, r: &egui::Response, m: &Message) {
        r.context_menu(|ui| {
            if ui.button("Kopieren").clicked() {
                ui.ctx().copy_text(m.content.clone());
                ui.close();
            }
            if ui.button("Mit Kennzeichnung kopieren").on_hover_text("Mit „Erstellt mit KI …“ und, wenn eingestellt, dem Prompt").clicked() {
                self.run("message.copy_labeled", json!({"message_id": m.id}));
                ui.close();
            }
        });
    }
}
