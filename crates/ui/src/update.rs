//! The update hint above the account row and the update part of the "Über" tab. The engine
//! checks (`app.check_update`); this only shows what it found.

use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, card, small_muted, status_dot};

impl AisCraftApp {
    /// A calm card when a newer published release exists; nothing otherwise.
    pub(crate) fn update_banner(&mut self, ui: &mut Ui) {
        let Some(a) = self.state.update.available.clone() else { return };
        let t = self.tokens.clone();
        card(&t).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, t.accent);
                ui.label(RichText::new(format!("Neue Version {}", a.version)).strong());
            });
            ui.label(small_muted(&t, format!("Du nutzt AIS.ChatCraft {}.", self.state.update.current)));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if accent_button(ui, &t, "Herunterladen").on_hover_text("Öffnet den Download im Browser").clicked() {
                    self.run("app.update_open", json!({"what": "download"}));
                }
                if ui.link("Was ist neu").on_hover_text("Öffnet die Release-Seite im Browser").clicked() {
                    self.run("app.update_open", json!({"what": "notes"}));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.add(egui::Button::new(small_muted(&t, "Diese Version überspringen")).frame(false)).clicked() {
                    self.run("app.update_skip", json!({"version": a.version}));
                }
                ui.label(small_muted(&t, "·"));
                if ui.add(egui::Button::new(small_muted(&t, "Später")).frame(false)).on_hover_text("Bis zum nächsten Start ausblenden").clicked() {
                    self.run("app.update_dismiss", json!({}));
                }
            });
        });
        ui.add_space(6.0);
    }

    /// Version, the switch and "Jetzt prüfen" in the "Über" tab.
    pub(crate) fn update_settings(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let u = self.state.update.clone();
        ui.label(RichText::new("Updates").strong());
        let mut on = self.state.settings.update_check;
        if ui.checkbox(&mut on, "Einmal am Tag nach einer neuen Version suchen").changed() {
            self.run("settings.set", json!({"key": "update_check", "value": on}));
        }
        ui.label(small_muted(
            &t,
            "Dabei geht eine anonyme Anfrage an api.github.com (Releases von FWU-DE/aiscraft). Es wird nichts heruntergeladen oder installiert.",
        ));
        ui.horizontal(|ui| {
            let button = ui.add_enabled(!u.checking, egui::Button::new(if u.checking { "Prüfe …" } else { "Jetzt prüfen" }));
            if button.clicked() {
                self.run_quiet("app.check_update", json!({}));
            }
            if u.last_check > 0 {
                ui.label(small_muted(&t, format!("Zuletzt geprüft: {}", aiscraft_engine::format_local(u.last_check))));
            }
        });
        if let Some(m) = &u.message {
            ui.label(small_muted(&t, m));
        }
        if let Some(a) = &u.available {
            ui.horizontal_wrapped(|ui| {
                if accent_button(ui, &t, &format!("Version {} herunterladen", a.version)).clicked() {
                    self.run("app.update_open", json!({"what": "download"}));
                }
                if ui.link("Was ist neu").clicked() {
                    self.run("app.update_open", json!({"what": "notes"}));
                }
            });
        } else if let Some(s) = &u.skipped {
            ui.label(small_muted(&t, format!("Übersprungen: Version {s}")));
        }
    }

    /// A command whose error the settings show themselves (no extra notice).
    fn run_quiet(&self, cmd: &str, params: serde_json::Value) {
        self.engine.dispatch_quiet(cmd, params);
    }
}
