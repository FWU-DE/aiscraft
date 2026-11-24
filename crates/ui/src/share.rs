//! "Teilen": live collaboration on the open document or whiteboard (the engine's collab plug-in).

use aiscraft_core::Artifact;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, small_muted};

impl AisCraftApp {
    /// Code or link, and "Beitreten".
    pub(crate) fn collab_join_row(&mut self, ui: &mut Ui) {
        ui.label(small_muted(&self.tokens, "Beitreten mit Code oder Link"));
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.collab_join).hint_text("XXXXX-XXXXX").desired_width(170.0));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Beitreten").clicked() || enter) && !self.collab_join.trim().is_empty() {
                let given = self.collab_join.trim().to_string();
                let key = if given.contains("/join/") { "link" } else { "code" };
                self.save_if_dirty();
                self.run("collab.join", json!({ key: given }));
                self.collab_join.clear();
                ui.close();
            }
        });
    }

    /// The "Teilen" button in the artifact header.
    pub(crate) fn share_menu(&mut self, ui: &mut Ui, meta: &Artifact) {
        let t = self.tokens.clone();
        let share = self.state.share(&meta.id).cloned();
        let label = if share.is_some() { RichText::new("Geteilt").color(t.accent) } else { RichText::new("Teilen").color(t.muted) };
        let body = |app: &mut Self, ui: &mut Ui| {
            ui.set_min_width(280.0);
            if let Some(v) = &share {
                ui.label(small_muted(&t, "Code zum Beitreten"));
                ui.label(RichText::new(&v.code).monospace().size(20.0).strong());
                ui.horizontal(|ui| {
                    if ui.button("Code kopieren").clicked() {
                        ui.ctx().copy_text(v.code.clone());
                    }
                    if ui.button("Link kopieren").clicked() {
                        ui.ctx().copy_text(v.link.clone());
                    }
                });
                ui.label(small_muted(&t, &v.detail));
                ui.separator();
                if ui.button("Nicht mehr teilen").on_hover_text("Hier nicht mehr abgleichen; die Kopie bleibt").clicked() {
                    app.run("collab.leave", json!({"id": meta.id}));
                    ui.close();
                }
                if v.owner && ui.button(RichText::new("Raum für alle beenden").color(t.danger)).clicked() {
                    app.run("collab.leave", json!({"id": meta.id, "end": true}));
                    ui.close();
                }
                return;
            }
            if app.state.collab.server.is_empty() {
                ui.label(small_muted(&t, "Für gemeinsames Bearbeiten braucht es einen Server."));
                ui.add(egui::TextEdit::singleline(&mut app.collab_url).hint_text("https://collab.example.org").desired_width(260.0));
                ui.add(
                    egui::TextEdit::singleline(&mut app.collab_key).password(true).hint_text("Schlüssel (falls der Server einen will)").desired_width(260.0),
                );
                if accent_button(ui, &t, "Speichern").clicked() && !app.collab_url.trim().is_empty() {
                    let key = std::mem::take(&mut app.collab_key);
                    app.run("collab.set_server", json!({"url": app.collab_url.trim(), "key": key}));
                }
            } else {
                ui.label(small_muted(&t, format!("Server: {}", app.state.collab.server)));
                if accent_button(ui, &t, "Teilen starten").clicked() {
                    app.save_if_dirty();
                    app.run("collab.share", json!({"id": meta.id}));
                    ui.close();
                }
            }
            ui.separator();
            app.collab_join_row(ui);
        };
        if t.designed() {
            let text = if share.is_some() { "Geteilt" } else { "Teilen" };
            let r = crate::kit::pill(ui, &t, crate::kit::Pill::Primary, Some(crate::icons::Icon::Share), text, 13.5, None);
            egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| body(self, ui));
        } else {
            ui.menu_button(label, |ui| body(self, ui));
        }
    }

    /// Code and connection under the header of a shared artifact.
    pub(crate) fn share_strip(&mut self, ui: &mut Ui, id: &str) {
        let t = self.tokens.clone();
        let Some(v) = self.state.share(id).cloned() else { return };
        let color = match v.status.as_str() {
            "online" => t.accent,
            "ended" => t.danger,
            _ => t.muted,
        };
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("Geteilt · {}", v.code)).color(color).strong());
                ui.label(small_muted(&t, &v.detail));
            });
        });
        ui.separator();
    }
}
