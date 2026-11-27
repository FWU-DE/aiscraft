//! Local files (#25), in Einstellungen › Konnektoren: on or off, and folders kept out.

use crate::files::Then;
use craft_dialogs::FileDialog;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

impl AisCraftApp {
    pub(crate) fn local_files_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Lokale Dateien").strong());
        ui.label(small_muted(
            &t,
            "Modelle mit Werkzeugen können Dateien in deinem Persönlichen Ordner und auf externen Laufwerken auflisten, lesen und speichern. Ist das aus, fragt der Chat beim ersten Mal. Versteckte Ordner (.ssh, Browserprofile, Schlüsselbund) und Dateien mit Geheimnissen (.env, Schlüssel, Passwort-Datenbanken) bleiben immer gesperrt; Lesen und Speichern fragen wie andere Aktionen; Aufgaben und Zeitpläne greifen nie auf Dateien zu.",
        ));
        let st = self.state.local_files.clone();
        let mut on = st.enabled;
        if ui.checkbox(&mut on, "Lokale Dateien erlauben").changed() {
            self.run("files.local_set", json!({"enabled": on}));
        }
        for path in &st.keep_out {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Freigeben").clicked() {
                        self.run("files.keep_out_remove", json!({"path": path}));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("Gesperrt: {path}")).small()).truncate());
                    });
                });
            });
        }
        if ui.button("Ordner sperren …").clicked() {
            self.files.ask(FileDialog::folder(), Then::Run { cmd: "files.keep_out_add", params: json!({}) });
        }
    }
}
