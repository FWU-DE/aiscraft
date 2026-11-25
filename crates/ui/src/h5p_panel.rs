//! H5P content in the artifact panel (#59): the tasks as a list to edit (the worksheet editor,
//! limited to what H5P has), "Ausprobieren" with the worksheet's checking, and "Als H5P
//! speichern". Imported packages AIS.ChatCraft cannot edit show what is in them and are saved
//! back unchanged. The main app has no webview: "Im Player ansehen" (#62) shows the content in
//! the real H5P player in the preview process, once the optional player is downloaded.

use std::time::Instant;

use aiscraft_core::h5p;
use egui::{CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{accent_button, card, link_button, muted, small_muted};
use crate::worksheet::{Mode, edit_sheet, page, work_sheet};

impl AisCraftApp {
    /// The H5P player dialog (#62): what the player is, and loading it only on an explicit yes
    /// that names size, source and licence. Returns whether the dialog stays open.
    pub(crate) fn h5p_player_body(&mut self, ui: &mut Ui, artifact: &str) -> bool {
        let t = self.tokens.clone();
        let p = self.state.h5p_player.clone();
        let mut keep = true;
        ui.heading("H5P-Player");
        ui.add_space(4.0);
        ui.label(muted(
            &t,
            "„Im Player ansehen“ zeigt H5P-Inhalte mit dem echten H5P-Player, so wie in Moodle, Mebis oder ByCS: in einem eigenen Vorschaufenster, ohne Internetzugriff.",
        ));
        ui.add_space(10.0);
        if !p.preview {
            ui.label(
                RichText::new(
                    "Zu dieser Installation gehört keine Vorschau (aiscraft-preview), die den Player zeigen kann. „Ausprobieren“ zeigt die Aufgaben trotzdem.",
                )
                .color(t.warning),
            );
            ui.add_space(8.0);
        }
        if p.installed {
            ui.label(format!("Der H5P-Player ist installiert ({} Bibliotheken, {} KB).", p.libraries, p.bytes.div_ceil(1024)));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if p.preview && accent_button(ui, &t, "Im Player ansehen").clicked() {
                    self.save_if_dirty();
                    self.run("h5p.play", json!({"id": artifact}));
                    keep = false;
                }
                if ui.button("H5P-Player entfernen").clicked() {
                    self.run("h5p.player_remove", json!({}));
                }
            });
        } else {
            card(&t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Der Player gehört nicht zu AIS.ChatCraft und wird nur auf deinen Wunsch geladen.").strong());
                ui.add_space(4.0);
                ui.label(muted(&t, format!("Größe: ca. {} MB", p.approx_mb)));
                ui.horizontal_wrapped(|ui| {
                    ui.label(muted(&t, "Quelle:"));
                    ui.hyperlink_to(RichText::new("Veröffentlichung auf GitHub").color(t.link), &p.source).on_hover_text(&p.source);
                });
                ui.label(muted(&t, format!("Lizenz: {}", p.license)));
                ui.label(small_muted(
                    &t,
                    "Der H5P-Kern steht unter der GPL-3.0, deshalb liegt er nicht im App-Paket. Der Download wird mit seiner SHA-256-Prüfsumme geprüft.",
                ));
            });
            ui.add_space(10.0);
            if p.installing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(muted(&t, "H5P-Player wird geladen …"));
                });
            } else if !p.can_download {
                ui.label(small_muted(&t, "In dieser Umgebung lädt AIS.ChatCraft nichts herunter."));
            } else if accent_button(ui, &t, &format!("H5P-Player laden (GPL-3.0, ca. {} MB)", p.approx_mb)).clicked() {
                self.run("h5p.player_install", json!({"yes": true}));
            }
        }
        if let Some(m) = &p.message {
            ui.add_space(6.0);
            ui.label(small_muted(&t, m));
        }
        ui.add_space(12.0);
        if link_button(ui, &t, "Schließen").clicked() {
            keep = false;
        }
        keep
    }

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
        // Some(true): show in the player; Some(false): manage the player.
        let mut play: Option<bool> = None;
        Frame::new().inner_margin(Margin::symmetric(12, 4)).show(ui, |ui| {
            // Wraps in a narrow panel: one long row made the panel wider than its default.
            ui.horizontal_wrapped(|ui| {
                if editable {
                    crate::widgets::segmented(ui, &t, &mut mode, [(Mode::Edit, "Bearbeiten"), (Mode::Work, "Ausprobieren")]);
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
                let r = ui
                    .button("Im Player ansehen")
                    .on_hover_text("Mit dem echten H5P-Player ansehen, so wie in der Lernplattform (optionaler Download). Rechtsklick: H5P-Player verwalten");
                if r.clicked() {
                    play = Some(true);
                } else if r.secondary_clicked() {
                    play = Some(false);
                }
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
        if let Some(show) = play {
            let p = &self.state.h5p_player;
            if show && p.installed && p.preview {
                self.save_if_dirty();
                self.run("h5p.play", json!({"id": id}));
            } else {
                self.dialog = Some(Dialog::H5pPlayer { artifact: id.clone() });
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
