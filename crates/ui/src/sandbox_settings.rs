//! „Eigener Rechner (Sandbox)“ (#94), in Einstellungen › Fähigkeiten: what it can do and what it
//! cannot see, size, source and licence; the download starts only after the second click
//! („Ja, laden und einrichten“). Once set up: on or off, the network switch, stop, remove.

use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{muted, small_muted};

impl AisCraftApp {
    pub(crate) fn sandbox_section(&mut self, ui: &mut Ui) {
        use aiscraft_engine::sandbox::{CAN, CANNOT, NOT_WITH_AIS, image};
        let t = self.tokens.clone();
        let s = self.state.sandbox.clone();
        ui.label(RichText::new("Eigener Rechner (Sandbox)").strong());
        ui.label(muted(&t, CAN));
        // Ask the system once per run whether WSL2 is there (no computer is started by that).
        let probed = egui::Id::unique("sandbox-probed");
        if !ui.data(|d| d.get_temp::<bool>(probed)).unwrap_or(false) {
            ui.data_mut(|d| d.insert_temp(probed, true));
            self.run("sandbox.status", json!({}));
        }
        ui.add_space(4.0);
        let ask_id = egui::Id::unique("sandbox-install-ask");
        if let Some(step) = &s.step {
            match s.progress {
                Some((done, total)) if total > 0 => {
                    let share = done as f32 / total as f32;
                    ui.add(egui::ProgressBar::new(share).text(format!("{step}: {} von {} MB", done / 1_000_000, total.div_ceil(1_000_000))));
                }
                _ => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(small_muted(&t, step));
                    });
                }
            }
        } else if s.installed {
            let mut on = s.enabled;
            if ui.checkbox(&mut on, format!("Eigenen Rechner nutzen ({})", s.machine)).changed() {
                self.run("sandbox.set", json!({"enabled": on}));
            }
            let mut net = s.network;
            if ui.checkbox(&mut net, "Netz im Arbeitsbereich erlauben").changed() {
                self.run("sandbox.set", json!({"network": net}));
            }
            ui.label(small_muted(
                &t,
                if s.network {
                    "Netz an: Befehle können Seiten im Internet erreichen und Daten dorthin schicken. Nur einschalten, solange du es brauchst."
                } else {
                    "Kein Netz: Befehle erreichen weder das Internet noch dein Schulnetz."
                },
            ));
            ui.label(small_muted(&t, CANNOT));
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Anhalten").on_hover_text("Beendet alles, was darin läuft; die Dateien bleiben").clicked() {
                    self.run("sandbox.stop", json!({}));
                }
                if ui.small_button("Entfernen").clicked() {
                    self.dialog = Some(Dialog::Confirm {
                        text: format!("Eigenen Rechner {} entfernen? Er wird mit allen Arbeitsordnern gelöscht.", s.machine),
                        command: "sandbox.remove".into(),
                        params: json!({}),
                    });
                }
            });
        } else {
            ui.label(small_muted(&t, CANNOT));
            ui.label(small_muted(&t, format!("Größe: {} MB Download, etwa {} MB auf der Platte (im Datenordner)", image::APPROX_MB, image::DISK_MB)));
            ui.label(small_muted(&t, format!("Quelle: {}", image::SOURCE_TEXT)));
            ui.label(small_muted(&t, format!("Lizenzen: {}", image::LICENSE)));
            ui.label(small_muted(&t, "Grenzen: 2 GB Arbeitsspeicher, 2 Prozessorkerne, 1024 Prozesse, höchstens 10 Minuten je Befehl. Das Abbild wird mit seiner SHA-256-Prüfsumme geprüft; stimmt sie nicht, bleibt nichts davon."));
            ui.label(small_muted(&t, NOT_WITH_AIS));
            ui.add_space(4.0);
            let blocked = !s.supported;
            let asking = ui.data(|d| d.get_temp::<bool>(ask_id)).unwrap_or(false);
            if !asking {
                if ui.add_enabled(!blocked, egui::Button::new("Einrichten …")).clicked() {
                    ui.data_mut(|d| d.insert_temp(ask_id, true));
                }
            } else {
                ui.label(RichText::new(format!("{} MB von github.com laden und den eigenen Rechner einrichten?", image::APPROX_MB)).strong());
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Ja, laden und einrichten").color(t.on_accent)).fill(t.accent)).clicked() {
                        ui.data_mut(|d| d.insert_temp(ask_id, false));
                        self.run("sandbox.install", json!({"yes": true}));
                    }
                    if ui.button("Abbrechen").clicked() {
                        ui.data_mut(|d| d.insert_temp(ask_id, false));
                    }
                });
            }
        }
        if let Some(p) = &s.problem {
            ui.label(RichText::new(p).small().color(t.danger));
        }
    }
}
