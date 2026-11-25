//! The card under a connector tool's result that has an interactive view (MCP Apps, #74). The
//! view opens in the separate preview window, never in this app. What an open view puts into
//! the chat shows as an entry of its own, marked as coming from the view, not from the person.

use aiscraft_core::Message;
use egui::{Color32, RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::theme::Tokens;
use crate::widgets::{card, small_muted};

fn hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

/// The look as a view gets it: light or dark, and the colours as CSS variables.
pub(crate) fn look(t: &Tokens) -> (&'static str, Value) {
    let styles = json!({
        "--color-background-primary": hex(t.bg),
        "--color-background-secondary": hex(t.surface),
        "--color-text-primary": hex(t.text),
        "--color-text-secondary": hex(t.muted),
        "--color-border-primary": hex(t.border),
        "--color-accent": hex(t.accent),
    });
    (if t.dark { "dark" } else { "light" }, styles)
}

/// Longest text of a view's entry shown in the chat.
const MAX_SHOWN: usize = 2000;

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
                        let (theme, styles) = look(&t);
                        self.run("view.open", json!({"message": m.id, "theme": theme, "styles": styles}));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("Ansicht von {source}")).strong()).truncate());
                    });
                });
            });
            if let Some(reason) = &view.cancelled {
                ui.label(small_muted(&t, format!("Der Aufruf lief nicht ({reason}); die Ansicht zeigt das.")));
            }
            ui.label(small_muted(
                &t,
                "Öffnet in einem eigenen Fenster, ohne Netz außer den Adressen, die der Konnektor nennt. Aktionen daraus fragen wie sonst auch.",
            ));
        });
    }

    /// What an open view put into the chat (`ui/message`, `ui/update-model-context`).
    pub(crate) fn view_entry(&mut self, ui: &mut Ui, m: &Message) {
        let Some(r) = m.tool_result.as_ref() else { return };
        let t = self.tokens.clone();
        let what = if r.from_view.as_deref() == Some("context") { "Kontext" } else { "Nachricht" };
        let mut text: String = m.content.chars().take(MAX_SHOWN).collect();
        if m.content.chars().count() > MAX_SHOWN {
            text.push('…');
        }
        card(&t).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(format!("{what} der Ansicht {}", r.name)).strong()).truncate());
            ui.add(egui::Label::new(text).wrap());
            ui.label(small_muted(&t, "Kommt von der Ansicht, nicht von dir. Das Modell sieht es als fremden Inhalt; es erlaubt keine Aktion."));
        });
    }
}
