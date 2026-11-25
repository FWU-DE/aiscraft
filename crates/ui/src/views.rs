//! The card under a connector tool's result that has an interactive view (MCP Apps, #74). The
//! view opens in the separate preview window, never in this app.

use aiscraft_core::Message;
use egui::{Color32, RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{card, small_muted};

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

impl AisCraftApp {
    pub(crate) fn view_card(&mut self, ui: &mut Ui, m: &Message) {
        let Some(view) = m.tool_result.as_ref().and_then(|r| r.view.as_ref()) else { return };
        let t = self.tokens.clone();
        let source = self.state.connectors.iter().find(|k| k.id == view.connector).map(|k| k.name.clone()).unwrap_or_else(|| "Konnektor".into());
        card(&t).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Ansicht öffnen").clicked() {
                        // The look's colours go along, so the view fits in.
                        let styles = json!({
                            "--color-background-primary": hex(t.bg),
                            "--color-background-secondary": hex(t.surface),
                            "--color-text-primary": hex(t.text),
                            "--color-text-secondary": hex(t.muted),
                            "--color-border-primary": hex(t.border),
                            "--color-accent": hex(t.accent),
                        });
                        self.run("view.open", json!({"message": m.id, "theme": if t.dark { "dark" } else { "light" }, "styles": styles}));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("Ansicht von {source}")).strong()).truncate());
                    });
                });
            });
            ui.label(small_muted(
                &t,
                "Öffnet in einem eigenen Fenster, ohne Netz außer den Adressen, die der Konnektor nennt. Aktionen daraus fragen wie sonst auch.",
            ));
        });
    }
}
