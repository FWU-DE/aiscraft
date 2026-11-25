//! Computer use (#97), in Einstellungen › Fähigkeiten: on or off, what it can and never does, and
//! the person's own list „Hier nie“.

use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

impl AisCraftApp {
    pub(crate) fn computer_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let st = self.state.computer.clone();
        ui.label(RichText::new("Programme auf diesem Rechner bedienen (experimentell)").strong());
        if !st.supported {
            ui.label(small_muted(&t, "Gibt es zuerst für Windows; für macOS und Linux ist es noch nicht verfügbar."));
            return;
        }
        let mut on = st.enabled;
        if ui.checkbox(&mut on, "Programme auf diesem Rechner bedienen").changed() {
            self.run("computer.set", json!({"enabled": on}));
        }
        ui.label(small_muted(
            &t,
            "Kann: Programme auflisten und öffnen (Word, Excel, Editor, Outlook …), Fenster lesen wie ein Bildschirmleser, Schaltflächen und Menüpunkte bedienen, Text in Felder schreiben, Tastenkürzel an dieses Programm senden. Ohne Mauszeiger; ein Fenster kommt nur nach vorn, wenn ein Schritt es braucht. Ein Programm öffnen fragt, wenn du es nicht genannt hast, und gilt nur für diese Anfrage. Jeder Schritt, der etwas ändert, geht durch die Vertrauensprüfung mit dem aktuellen Fensterinhalt; hat sich das Fenster seitdem geändert, passiert nichts.",
        ));
        ui.label(small_muted(
            &t,
            "Nie: Eingabeaufforderung, PowerShell und Terminals, Einstellungen, Systemsteuerung, Registrierungs-Editor, Task-Manager und andere Programme des Windows-Ordners, Explorer, Passwortmanager, Krypto-Wallets, AIS.ChatCraft selbst, Programme mit Administratorrechten, Passwortfelder. Kein Bild vom Bildschirm. Aufgaben und Zeitpläne bedienen nie Programme. Mit AIS.chat nicht verfügbar (keine Werkzeuge).",
        ));
        ui.add_space(4.0);
        ui.label(RichText::new("Hier nie").small().strong());
        for name in &st.never {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Freigeben").clicked() {
                        self.run("computer.never_remove", json!({"name": name}));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("Gesperrt: {name}")).small()).truncate());
                    });
                });
            });
        }
        let draft = &mut self.computer_never_draft;
        let (enter, clicked) = crate::widgets::field_then_buttons(
            ui,
            |ui| ui.button("Sperren").clicked(),
            |ui, width| {
                let r = ui.add(egui::TextEdit::singleline(draft).desired_width(width).hint_text("Programm, z. B. Outlook oder outlook.exe"));
                r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
            },
        );
        if (enter || clicked) && !self.computer_never_draft.trim().is_empty() {
            let name = std::mem::take(&mut self.computer_never_draft);
            self.run("computer.never_add", json!({"name": name.trim()}));
        }
    }
}
