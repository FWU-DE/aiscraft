//! "Chats übernehmen" (#42): the data export of claude.ai or ChatGPT becomes local chats
//! (`chat.import`). The dialog says how to get the export and that names protection does not
//! apply to what was written there.

use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, small_muted};

impl AisCraftApp {
    /// Returns whether the dialog stays open.
    pub(crate) fn import_chats_dialog(&mut self, ui: &mut Ui) -> bool {
        let t = self.tokens.clone();
        let mut keep = true;
        ui.label(crate::theme::bold("Chats aus claude.ai oder ChatGPT übernehmen").size(20.0));
        ui.add_space(6.0);
        for line in [
            "1. Datenexport anfordern: bei claude.ai unter Einstellungen → Datenschutz → Daten exportieren, bei ChatGPT unter Einstellungen → Datenkontrollen → Daten exportieren.",
            "2. Den Link aus der E-Mail öffnen und die ZIP-Datei speichern.",
            "3. Hier die ZIP-Datei wählen. Die Chats erscheinen in der Seitenleiste und lassen sich durchsuchen und weiterführen.",
        ] {
            ui.add(egui::Label::new(RichText::new(line).color(t.text2)).wrap());
        }
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(small_muted(
                &t,
                "„Namen schützen“ wirkt nicht rückwirkend: Übernommene Chats enthalten Namen so, wie sie bei claude.ai oder ChatGPT standen. Sie bleiben auf diesem Gerät; wer so einen Chat weiterführt, schickt den Verlauf an den gewählten Dienst.",
            ))
            .wrap(),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if accent_button(ui, &t, "Exportdatei wählen …").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Datenexport", &["zip", "json"]).pick_file() {
                    let project = self.state.selection.project_id.clone();
                    self.run("chat.import", json!({"path": path.display().to_string(), "project": project}));
                }
                keep = false;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        keep
    }
}
