//! H5P content in the artifact panel (#59): the tasks as a list to edit (the worksheet editor,
//! limited to what H5P has), "Ausprobieren" with the worksheet's checking, and "Als H5P
//! speichern". Imported packages AIS.ChatCraft cannot edit show what is in them and are saved
//! back unchanged. No H5P player here: the main app has no webview.

use std::time::Instant;

use aiscraft_core::h5p;
use egui::{CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, muted, small_muted};
use crate::worksheet::{Mode, edit_sheet, page, work_sheet};

impl AisCraftApp {
    pub(crate) fn h5p_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(v) = self.artifact.as_mut() else { return };
        v.sheet.sync_h5p(&v.doc);
        if v.sheet.mode == Mode::Preview {
            v.sheet.mode = Mode::Edit;
        }
        let id = v.id.clone();
        let content = v.sheet.h5p.clone();
        let editable = content.as_ref().is_some_and(|h| h.package.as_ref().is_none_or(|p| p.editable));
        let built = content.as_ref().filter(|_| editable).map(h5p::build);
        let mut mode = v.sheet.mode;
        let (mut check, mut reset, mut save) = (false, false, false);
        Frame::new().inner_margin(Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if editable {
                    if t.designed() {
                        crate::widgets::segmented(ui, &t, &mut mode, [(Mode::Edit, "Bearbeiten"), (Mode::Work, "Ausprobieren")]);
                    } else {
                        ui.selectable_value(&mut mode, Mode::Edit, "Bearbeiten");
                        ui.selectable_value(&mut mode, Mode::Work, "Ausprobieren").on_hover_text("Die Aufgaben so lösen, wie Schülerinnen und Schüler es tun");
                    }
                    ui.separator();
                    if mode == Mode::Work {
                        check = accent_button(ui, &t, "Prüfen").clicked();
                        reset = ui.button("Zurücksetzen").clicked();
                        ui.separator();
                    }
                }
                save = accent_button(ui, &t, "Als H5P speichern …")
                    .on_hover_text("Als .h5p-Datei speichern, zum Hochladen in Moodle, Mebis, ByCS oder eine andere Lernplattform mit H5P")
                    .clicked();
                let what = match &built {
                    Some(Ok(b)) => format!("{} · {} Aufgaben", b.main.machine, content.as_ref().map(|h| h.tasks.len()).unwrap_or(0)),
                    _ => content.as_ref().and_then(|h| h.package.as_ref()).map(|p| p.main_library.clone()).unwrap_or_default(),
                };
                ui.label(small_muted(&t, what));
            });
        });
        if let Some(v) = self.artifact.as_mut() {
            v.sheet.mode = mode;
            if reset {
                v.sheet.reset();
            }
            if check {
                v.sheet.check();
            }
        }
        if save {
            self.save_if_dirty();
            let title = self.state.artifacts.iter().find(|a| a.id == id).map(|a| a.title.clone()).unwrap_or_default();
            if let Some(path) = rfd::FileDialog::new().add_filter("H5P", &["h5p"]).set_file_name(aiscraft_engine::office::file_name(&title, "h5p")).save_file()
            {
                self.run("h5p.export", json!({"id": id, "path": path.display().to_string()}));
            }
        }
        if editable {
            self.revise_bar(ui, "Mit KI überarbeiten, z. B. „Zwei Fragen mehr, leichter“");
        }
        let Some(v) = self.artifact.as_mut() else { return };
        let mut changed = false;
        ScrollArea::vertical().id_salt(("h5p", id.clone())).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
                page(ui, |ui| {
                    if let Some(err) = &v.sheet.error {
                        ui.label(RichText::new(format!("Der H5P-Inhalt lässt sich nicht lesen: {err}")).color(t.danger));
                        ui.label(small_muted(&t, "Eine frühere Fassung liegt unter Versionen (⟳ oben)."));
                        return;
                    }
                    if let Some(p) = content.as_ref().and_then(|h| h.package.as_ref()) {
                        package_card(ui, &t, p);
                        ui.add_space(8.0);
                    }
                    if !editable {
                        return;
                    }
                    let sheet = &mut v.sheet;
                    match sheet.mode {
                        Mode::Work => work_sheet(ui, &t, sheet),
                        _ => {
                            if let Some(ws) = sheet.ws.as_mut() {
                                changed = edit_sheet(ui, &t, ws, true);
                            }
                            for p in sheet.problems.iter().take(6) {
                                ui.label(RichText::new(p).small().color(t.warning));
                            }
                            match &built {
                                Some(Ok(b)) if !b.notes.is_empty() => {
                                    ui.add_space(4.0);
                                    ui.label(small_muted(&t, "Beim Speichern als H5P:"));
                                    for n in b.notes.iter().take(8) {
                                        ui.label(RichText::new(format!("• {n}")).small().color(t.muted));
                                    }
                                }
                                Some(Err(e)) => {
                                    ui.label(RichText::new(e).small().color(t.warning));
                                }
                                _ => {}
                            }
                        }
                    }
                });
            });
        });
        if changed && let (Some(ws), Some(h)) = (v.sheet.ws.as_mut(), v.sheet.h5p.as_mut()) {
            aiscraft_core::worksheet::normalize_ids(ws);
            h5p::apply_worksheet(h, ws);
            let json = h5p::to_json(h);
            v.sheet.problems = h5p::validate(h);
            v.sheet.src = json.clone();
            v.doc = json;
            v.dirty_since = Some(Instant::now());
        }
        self.autosave(ui);
    }
}

/// What an imported package is: its content type, libraries, files, and what did not come across.
fn package_card(ui: &mut Ui, t: &crate::theme::Tokens, p: &h5p::PackageInfo) {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(t.radius.saturating_add(2)))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let kb = p.bytes.div_ceil(1024);
            ui.label(RichText::new(format!("Aus einer H5P-Datei · {}", p.main_library)).strong());
            ui.label(small_muted(t, format!("{} Dateien, {kb} KB · {} Bibliotheken", p.files, p.libraries.len())));
            if p.editable {
                ui.label(muted(t, "Die Aufgaben lassen sich hier bearbeiten. Bilder und andere Dateien des Pakets bleiben beim Speichern erhalten."));
            } else {
                ui.label(muted(
                    t,
                    "Diesen Inhaltstyp kann AIS.ChatCraft nicht bearbeiten. „Als H5P speichern“ speichert das Paket unverändert, z. B. zum Hochladen in eine Lernplattform.",
                ));
            }
            // "H5P.X kann AIS.ChatCraft nicht bearbeiten" says what the paragraph above says;
            // a quiz with one unknown question ("Frage 3: …") is worth naming.
            for n in p.notes.iter().filter(|n| p.editable || n.starts_with("Frage")).take(6) {
                ui.label(RichText::new(n).small().color(t.warning));
            }
            if !p.libraries.is_empty() {
                egui::CollapsingHeader::new(small_muted(t, "Bibliotheken")).id_salt("h5p-libs").default_open(false).show(ui, |ui| {
                    for l in p.libraries.iter().take(60) {
                        ui.label(small_muted(t, l));
                    }
                });
            }
        });
}
