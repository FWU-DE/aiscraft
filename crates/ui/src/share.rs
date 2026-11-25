//! „Teilen“ (#60): a dialog that first asks what the person has in mind. „Gemeinsam bearbeiten“
//! is live collaboration on the one original (the engine's collab plug-in); „Bereitstellen“ hands
//! on a copy as a file (`artifact.export`, the paths from `artifact.export_paths`). Joining with a
//! code has nothing to do with one's own artifact: it lives on the start page, in the sidebar and
//! in the command palette („Mit Code beitreten“).

use aiscraft_core::Artifact;
use egui::{RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::dialogs::{Dialog, SettingsTab};
use crate::icons::Icon;
use crate::theme::Tokens;
use crate::widgets::{accent_button, link_button, muted, small_muted};

/// The two ways of the share dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareWay {
    /// Live, on the one original, with everyone who has the code.
    Together,
    /// A copy as a file; the original stays here.
    Provide,
}

/// The card titles, also the names the tests click.
pub(crate) const TOGETHER: &str = "Gemeinsam bearbeiten";
pub(crate) const PROVIDE: &str = "Bereitstellen";
const TOGETHER_LINE: &str = "Ein Original: Wer den Code hat, schreibt live mit.";
const PROVIDE_LINE: &str = "Eine Kopie als Datei geben. Das Original bleibt bei dir.";

/// The server's name without scheme, for the data note.
fn host(server: &str) -> &str {
    server.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/')
}

impl AisCraftApp {
    /// The "Teilen" button in the artifact header: opens the dialog.
    pub(crate) fn share_button(&mut self, ui: &mut Ui, meta: &Artifact) {
        let t = self.tokens.clone();
        let shared = self.state.share(&meta.id).is_some();
        let text = if shared { "Geteilt" } else { "Teilen" };
        let r = crate::kit::pill(ui, &t, crate::kit::Pill::Primary, Some(Icon::Share), text, 13.5, None);
        if r.on_hover_text("Gemeinsam bearbeiten oder als Datei bereitstellen").clicked() {
            self.save_if_dirty();
            self.dialog = Some(Dialog::Share { id: meta.id.clone(), way: None });
        }
    }

    /// The share dialog; returns whether it stays open.
    pub(crate) fn share_body(&mut self, ui: &mut Ui, id: &str, way: &mut Option<ShareWay>) -> bool {
        let t = self.tokens.clone();
        let Some(meta) = self.state.artifacts.iter().find(|a| a.id == id).cloned() else { return false };
        let share = self.state.share(id).cloned();
        // Shared already: the running share comes first.
        if share.is_some() && way.is_none() {
            *way = Some(ShareWay::Together);
        }
        ui.add(egui::Label::new(crate::theme::bold(format!("„{}“ teilen", meta.title)).size(22.0).color(t.text)).truncate());
        ui.label(muted(&t, "Was hast du vor?"));
        ui.add_space(12.0);
        self.way_cards(ui, way);
        ui.add_space(14.0);
        let mut keep = true;
        match way {
            Some(ShareWay::Together) => self.together_body(ui, &meta, share.as_ref(), &mut keep),
            Some(ShareWay::Provide) => self.provide_body(ui, &meta),
            None => {}
        }
        ui.add_space(10.0);
        ui.separator();
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 32.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Fertig").clicked() {
                keep = false;
            }
        });
        keep
    }

    /// The two ways as large cards side by side, each with one sentence.
    fn way_cards(&mut self, ui: &mut Ui, way: &mut Option<ShareWay>) {
        let t = self.tokens.clone();
        let gap = 12.0;
        let w = ((ui.available_width() - gap) / 2.0).max(120.0);
        let pad = 16.0;
        let wrap = |ui: &Ui, text: &str| {
            let mut job = egui::text::LayoutJob::single_section(
                text.to_string(),
                egui::TextFormat { font_id: crate::kit::font(13.5, ""), color: t.muted, line_height: Some(18.0), ..Default::default() },
            );
            job.wrap.max_width = w - 2.0 * pad;
            job.wrap.max_rows = 3;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let lines = [wrap(ui, TOGETHER_LINE), wrap(ui, PROVIDE_LINE)];
        let text_h = lines.iter().map(|g| g.size().y).fold(0.0, f32::max);
        let h = pad + 36.0 + 10.0 + 20.0 + 4.0 + text_h + pad;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for ((choice, icon, title), line) in
                [(ShareWay::Together, Icon::Users, TOGETHER), (ShareWay::Provide, Icon::Download, PROVIDE)].into_iter().zip(lines)
            {
                let on = *way == Some(choice);
                let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, title));
                let p = ui.painter();
                let hovered = resp.hovered() || resp.has_focus();
                let r = egui::CornerRadius::same(t.r_card);
                let fill = if on { t.opaque(t.accent_soft) } else { t.surface };
                let stroke = if on || hovered {
                    t.accent
                } else if t.glass.is_some() {
                    t.card_stroke
                } else {
                    t.border
                };
                p.rect(rect, r, fill, egui::Stroke::new(if on { 2.0 } else { 1.0 }, stroke), egui::StrokeKind::Inside);
                let tile = egui::Rect::from_min_size(rect.min + vec2(pad, pad), vec2(36.0, 36.0));
                if on {
                    crate::kit::accent_fill(p, tile, egui::CornerRadius::same(11), &t);
                    icon.paint(p, tile.center(), 19.0, t.on_accent);
                } else {
                    crate::kit::icon_tile(p, tile, 11, t.accent_soft, icon, 19.0, t.accent_ink);
                }
                let g = crate::kit::line(ui, title, crate::kit::font(16.0, crate::theme::BOLD), t.text, w - 2.0 * pad);
                p.galley(egui::pos2(rect.left() + pad, tile.bottom() + 10.0), g, t.text);
                p.galley(egui::pos2(rect.left() + pad, tile.bottom() + 34.0), line, t.muted);
                if resp.clicked() {
                    *way = Some(choice);
                }
            }
        });
    }

    /// „Gemeinsam bearbeiten“: the running share, or how to start one.
    fn together_body(&mut self, ui: &mut Ui, meta: &Artifact, share: Option<&aiscraft_engine::CollabView>, keep: &mut bool) {
        let t = self.tokens.clone();
        if let Some(v) = share {
            ui.label(small_muted(&t, "Code zum Beitreten"));
            ui.label(RichText::new(&v.code).monospace().size(22.0).strong());
            ui.horizontal(|ui| {
                if ui.button("Code kopieren").clicked() {
                    ui.ctx().copy_text(v.code.clone());
                }
                if ui.button("Link kopieren").clicked() {
                    ui.ctx().copy_text(v.link.clone());
                }
            });
            ui.add_space(6.0);
            let color = match v.status.as_str() {
                "online" => t.success,
                "ended" => t.danger,
                _ => t.muted,
            };
            ui.horizontal(|ui| {
                crate::widgets::status_dot(ui, color);
                ui.add(egui::Label::new(RichText::new(&v.detail).color(t.text2)).wrap());
            });
            ui.add(
                egui::Label::new(small_muted(&t, format!("Wer den Code hat, kann mitschreiben. Der Inhalt liegt dafür auf dem Server {}.", host(&v.server))))
                    .wrap(),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Nicht mehr teilen").on_hover_text("Hier nicht mehr abgleichen; deine Kopie bleibt").clicked() {
                    self.run("collab.leave", json!({"id": meta.id}));
                }
                if v.owner
                    && ui
                        .button(RichText::new("Raum für alle beenden").color(t.danger))
                        .on_hover_text("Löscht den Raum auf dem Server; niemand kann mehr beitreten")
                        .clicked()
                {
                    self.run("collab.leave", json!({"id": meta.id, "end": true}));
                }
            });
            return;
        }
        if !self.state.collab.available {
            ui.add(egui::Label::new(muted(&t, "Gemeinsam bearbeiten ist in dieser Ausgabe von AIS.ChatCraft nicht enthalten.")).wrap());
            return;
        }
        if self.state.collab.server.is_empty() {
            egui::Frame::new()
                .fill(t.opaque(t.accent_soft))
                .corner_radius(egui::CornerRadius::same(t.control_radius()))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Dafür braucht es einen Server für die Zusammenarbeit, z. B. von der Schule oder dem Träger. Die Adresse trägst du einmal in den Einstellungen ein.",
                            )
                            .color(t.accent_ink),
                        )
                        .wrap(),
                    );
                    if link_button(ui, &t, "Server in den Einstellungen eintragen").clicked() {
                        self.dialog = Some(Dialog::Settings { tab: SettingsTab::Backends, new_note: String::new() });
                        *keep = false;
                    }
                });
            return;
        }
        for line in [
            "Wer den Code hat, kann mitschreiben. Alle arbeiten am selben Original; Änderungen erscheinen sofort bei allen.".to_string(),
            format!("Der Inhalt liegt dafür auf dem Server {}. Räume, die lange niemand öffnet, löscht der Server.", host(&self.state.collab.server)),
        ] {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").color(t.accent));
                ui.add(egui::Label::new(line).wrap());
            });
        }
        ui.add_space(8.0);
        if accent_button(ui, &t, "Teilen starten").clicked() {
            self.save_if_dirty();
            self.run("collab.share", json!({"id": meta.id}));
        }
    }

    /// „Bereitstellen“: the files that fit this kind, each one an `artifact.export`.
    fn provide_body(&mut self, ui: &mut Ui, meta: &Artifact) {
        let t = self.tokens.clone();
        ui.add(
            egui::Label::new(muted(
                &t,
                "Eine Kopie zum Ansehen, Ausdrucken oder für die Lernplattform. Das Original bleibt bei dir; spätere Änderungen gehen nicht mit.",
            ))
            .wrap(),
        );
        ui.add_space(8.0);
        let paths = self.engine.call("artifact.export_paths", json!({"id": meta.id})).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let s = |p: &Value, k: &str| p.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        for p in paths {
            let (label, hint, ext) = (s(&p, "label"), s(&p, "hint"), s(&p, "ext"));
            ui.horizontal(|ui| {
                let clicked = ui.add(egui::Button::new(&label).min_size(vec2(190.0, 30.0))).on_hover_text(&hint).clicked();
                ui.add(egui::Label::new(small_muted(&t, &hint)).wrap());
                if clicked {
                    let name = aiscraft_engine::office::file_name(&format!("{}{}", meta.title, s(&p, "suffix")), &ext);
                    if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                        self.save_if_dirty();
                        let mut params = json!({"id": meta.id, "format": s(&p, "format"), "path": path.display().to_string()});
                        if let (Some(v), Some(o)) = (p.get("variant").and_then(Value::as_str), params.as_object_mut()) {
                            o.insert("variant".into(), json!(v));
                        }
                        self.run("artifact.export", params);
                    }
                }
            });
        }
    }

    /// „Mit Code beitreten“; returns whether the dialog stays open.
    pub(crate) fn join_body(&mut self, ui: &mut Ui, code: &mut String) -> bool {
        let t = self.tokens.clone();
        ui.label(crate::theme::bold("Mit Code beitreten").size(22.0).color(t.text));
        ui.add(
            egui::Label::new(muted(
                &t,
                "Hat dir jemand einen Code oder Link zum gemeinsamen Bearbeiten gegeben? Hier einfügen: Das Dokument öffnet sich, und du schreibst live mit.",
            ))
            .wrap(),
        );
        ui.add_space(10.0);
        let r = ui.add(egui::TextEdit::singleline(code).hint_text("XXXXX-XXXXX oder Link").desired_width(f32::INFINITY));
        r.request_focus();
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let given = code.trim().to_string();
        if self.state.collab.server.is_empty() && !given.contains("/join/") {
            ui.add(
                egui::Label::new(small_muted(
                    &t,
                    "Für einen Code braucht es den Server unter Einstellungen › Dienste › Zusammenarbeit. Ein ganzer Link geht auch ohne.",
                ))
                .wrap(),
            );
        }
        ui.add_space(10.0);
        let mut keep = true;
        ui.horizontal(|ui| {
            if (accent_button(ui, &t, "Beitreten").clicked() || enter) && !given.is_empty() {
                let key = if given.contains("/join/") { "link" } else { "code" };
                self.save_if_dirty();
                self.run("collab.join", json!({ key: given }));
                keep = false;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        keep
    }

    /// Settings › Dienste: the server for working together.
    pub(crate) fn collab_settings(&mut self, ui: &mut Ui) {
        if !self.state.collab.available {
            return;
        }
        let t: Tokens = self.tokens.clone();
        ui.add_space(14.0);
        ui.label(crate::theme::semibold("Zusammenarbeit"));
        ui.add(
            egui::Label::new(small_muted(
                &t,
                "Ein Server, über den du Dokumente, Whiteboards, Arbeitsblätter und Lern-Apps live mit anderen bearbeitest (aiscraft-collab-server, z. B. von der Schule oder dem Träger). Ohne Server bleibt alles auf diesem Rechner.",
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let server = self.state.collab.server.clone();
        if !server.is_empty() {
            crate::widgets::field_then_buttons(
                ui,
                |ui| {
                    if ui.button("Server entfernen").on_hover_text("Geteilte Dokumente bleiben verbunden; neue lassen sich nicht mehr teilen").clicked() {
                        self.run("collab.set_server", json!({"url": "", "key": ""}));
                    }
                },
                |ui, w| ui.add_sized(vec2(w, 20.0), egui::Label::new(format!("Server: {}", host(&server))).truncate()),
            );
            return;
        }
        let mut url = std::mem::take(&mut self.collab_url);
        let (_, save) = crate::widgets::field_then_buttons(
            ui,
            |ui| ui.button("Server speichern").clicked(),
            |ui, w| ui.add(egui::TextEdit::singleline(&mut url).hint_text("https://collab.example.org").desired_width(w)),
        );
        if save && !url.trim().is_empty() {
            let key = std::mem::take(&mut self.collab_key);
            self.run("collab.set_server", json!({"url": url.trim(), "key": key}));
            url.clear();
        }
        self.collab_url = url;
        let w = ui.available_width();
        ui.add(egui::TextEdit::singleline(&mut self.collab_key).password(true).hint_text("Schlüssel (falls der Server einen verlangt)").desired_width(w));
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

#[cfg(test)]
mod tests {
    #[test]
    fn the_server_is_named_without_scheme() {
        assert_eq!(super::host("https://collab.schule.de/"), "collab.schule.de");
        assert_eq!(super::host("http://127.0.0.1:8080"), "127.0.0.1:8080");
    }
}
