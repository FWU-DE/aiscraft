//! Background work at a glance (#69, Nod 0.9.3): running tasks, schedules and page watches in one
//! line above the composer instead of progress in the chat, and a task's result as a compact card
//! with „Details“.

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

impl AisCraftApp {
    /// „2 Aufgaben laufen · Elternbrief prüfen …“; a click opens the first one's chat.
    pub(crate) fn status_line(&mut self, ui: &mut Ui) {
        let Some(text) = aiscraft_engine::running_summary(&self.state.running) else { return };
        let t = self.tokens.clone();
        let first = self.state.running.first().map(|j| j.id.clone());
        let all: Vec<String> = self.state.running.iter().map(|j| j.title.clone()).collect();
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
            let r = ui.add(egui::Button::new(small_muted(&t, &text)).frame(false).truncate()).on_hover_text(all.join("\n"));
            if r.clicked()
                && let Some(id) = first
            {
                self.run("chat.open", json!({"id": id}));
            }
        });
    }

    /// A task's result: title, one line, „Details“ opens the task's chat.
    pub(crate) fn task_card(&mut self, ui: &mut Ui, card: &aiscraft_engine::TaskCard) {
        let t = self.tokens.clone();
        Frame::new().fill(t.surface_hi).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::same(10)).show(
            ui,
            |ui| {
                let (word, color) = if card.ok { ("ist fertig", t.text) } else { ("ist gescheitert", t.danger) };
                ui.add(egui::Label::new(RichText::new(format!("Aufgabe „{}“ {word}", card.title)).strong().color(color)).wrap());
                if !card.line.is_empty() {
                    ui.add(egui::Label::new(small_muted(&t, &card.line)).wrap());
                }
                if ui.button("Details").clicked() {
                    self.run("chat.open", json!({"id": card.task_id}));
                }
            },
        );
    }
}
