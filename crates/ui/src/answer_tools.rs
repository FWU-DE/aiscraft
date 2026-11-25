//! Under an answer: „Lesbarkeit“ (#48). Each item dispatches a command.

use aiscraft_core::Message;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::theme::Tokens;

impl AisCraftApp {
    /// The pills in the row of actions under an answer.
    pub(crate) fn answer_tools(&mut self, ui: &mut Ui, t: &Tokens, m: &Message) {
        use crate::kit::{Pill, pill};
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
}
