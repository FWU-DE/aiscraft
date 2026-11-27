//! Download cards for files the model created (#38): Excel, PowerPoint, Word, PDF. The file is
//! in the app's attachments folder; "Speichern …" asks where to put it (the save dialog), then
//! `chat.save_file` copies it there.

use crate::files::Then;
use aiscraft_core::Message;
use craft_dialogs::FileDialog;
use egui::{CornerRadius, Frame, Margin, RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

/// What kind of file, for the badge on the card.
fn kind_of(mime: &str) -> &'static str {
    match mime {
        m if m.contains("spreadsheetml") => "Excel",
        m if m.contains("presentationml") => "PowerPoint",
        m if m.contains("wordprocessingml") => "Word",
        "application/pdf" => "PDF",
        _ => "Datei",
    }
}

fn size_label(bytes: u64) -> String {
    if bytes < 1024 * 1024 { format!("{} KB", bytes.div_ceil(1024)) } else { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }
}

impl AisCraftApp {
    /// The created files of an assistant message, one card each.
    pub(crate) fn download_cards(&mut self, ui: &mut Ui, m: &Message) {
        let t = self.tokens.clone();
        for (i, a) in m.attachments.iter().enumerate() {
            if a.mime.starts_with("image/") || a.is_link() || a.local_path.is_none() {
                continue;
            }
            Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    Frame::new().fill(t.accent).corner_radius(CornerRadius::same(4)).inner_margin(Margin::symmetric(6, 2)).show(ui, |ui| {
                        ui.label(RichText::new(kind_of(&a.mime)).small().strong().color(egui::Color32::WHITE));
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&a.name).strong());
                        ui.label(small_muted(&t, size_label(a.size)));
                    });
                    ui.add_space(12.0);
                    if ui.button("Speichern …").clicked() {
                        self.files
                            .ask(FileDialog::save().file_name(&a.name), Then::Run { cmd: "chat.save_file", params: json!({"message_id": m.id, "index": i}) });
                    }
                });
            });
            ui.add_space(4.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_and_sizes() {
        assert_eq!(kind_of("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"), "Excel");
        assert_eq!(kind_of("application/pdf"), "PDF");
        assert_eq!(size_label(1), "1 KB");
        assert_eq!(size_label(3 * 1024 * 1024), "3.0 MB");
    }
}
