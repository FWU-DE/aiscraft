//! Connector packages and plug-ins (#43, #28) in Einstellungen › Konnektoren, and the install
//! dialog: what a package may do and its settings, before anything runs.

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::dialogs::{Dialog, SettingsTab};
use crate::widgets::{accent_button, card, muted, small_muted};

impl AisCraftApp {
    /// Opens the install dialog once `extension.inspect` has read a package.
    pub(crate) fn sync_install_dialog(&mut self) {
        let Some(offer) = self.state.extensions.offer.as_ref() else { return };
        let open = matches!(&self.dialog, Some(Dialog::Install { id, .. }) if *id == offer.id);
        if !open {
            let values = offer
                .fields
                .iter()
                .map(|f| {
                    let v = match &f.default {
                        Value::String(s) => s.clone(),
                        Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    (f.key.clone(), v)
                })
                .collect();
            self.dialog = Some(Dialog::Install { id: offer.id.clone(), values });
        }
    }

    pub(crate) fn extensions_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Pakete und Plug-ins").strong());
        ui.label(small_muted(
            &t,
            "Konnektoren mit einem Klick: MCP-Pakete (.mcpb) und Plug-ins im Format Agent Plugins (plugin.json, wie bei Nod, Claude und Codex). Vor der Installation siehst du, welches Programm startet und worauf es zugreift. Ihre Werkzeuge folgen denselben Freigaben wie alle Konnektoren.",
        ));
        for x in self.state.extensions.installed.clone() {
            card(&t).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !x.found && ui.small_button("Entfernen").clicked() {
                            self.dialog = Some(Dialog::Confirm {
                                text: format!("{} entfernen? Seine Dateien, Einstellungen und Zugangsdaten werden gelöscht.", x.display_name),
                                command: "extension.remove".into(),
                                params: json!({"name": x.name}),
                            });
                        }
                        let mut on = x.enabled;
                        if ui.checkbox(&mut on, if x.found { "einschalten" } else { "aktiv" }).changed() {
                            self.run("extension.toggle", json!({"name": x.name, "enabled": on}));
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let kind = if x.kind == "bundle" { "MCP-Paket" } else { "Plug-in" };
                            let line = format!("{} {} · {kind}", x.display_name, x.version);
                            ui.add(egui::Label::new(RichText::new(line).strong()).truncate());
                        });
                    });
                });
                if x.found {
                    ui.label(small_muted(&t, "Im Plug-in-Ordner gefunden, noch nie gestartet. Einschalten prüft es und startet seine Server."));
                } else {
                    let mut parts = Vec::new();
                    if !x.connectors.is_empty() {
                        parts.push(format!("{} Konnektor(en)", x.connectors.len()));
                    }
                    if !x.skills.is_empty() {
                        parts.push(format!("Fähigkeiten: {}", x.skills.join(", ")));
                    }
                    if !x.author.is_empty() {
                        parts.push(format!("von {}", x.author));
                    }
                    ui.label(small_muted(&t, parts.join(" · ")));
                }
            });
            ui.add_space(4.0);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Paket installieren …").clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("MCP-Paket oder Plug-in", &["mcpb", "dxt", "zip", "plugin"]).pick_file()
            {
                self.run("extension.inspect", json!({"path": path.display().to_string()}));
            }
            if ui.button("Plug-in-Ordner …").clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                self.run("extension.inspect", json!({"path": dir.display().to_string()}));
            }
        });
        if !self.state.extensions.folder.is_empty() {
            ui.label(small_muted(
                &t,
                format!("Plug-ins lassen sich auch in {} ablegen; sie erscheinen dann hier zum Einschalten.", self.state.extensions.folder),
            ));
        }
    }

    /// The install dialog; returns whether it stays open.
    pub(crate) fn install_body(&mut self, ui: &mut Ui, id: &str, values: &mut BTreeMap<String, String>) -> bool {
        let t = self.tokens.clone();
        let Some(offer) = self.state.extensions.offer.clone().filter(|o| o.id == id) else {
            // Installed or cancelled: back to the connectors.
            self.dialog = Some(Dialog::Settings { tab: SettingsTab::Connectors, new_note: String::new() });
            return false;
        };
        let kind = if offer.kind == "bundle" { "MCP-Paket" } else { "Plug-in" };
        ui.heading(if offer.update { format!("{} aktualisieren", offer.display_name) } else { format!("{} installieren", offer.display_name) });
        let mut meta = vec![kind.to_string()];
        if !offer.version.is_empty() {
            meta.push(format!("Version {}", offer.version));
        }
        if !offer.author.is_empty() {
            meta.push(format!("von {}", offer.author));
        }
        ui.label(small_muted(&t, meta.join(" · ")));
        let mut keep = true;
        let max_h = (ui.ctx().content_rect().height() - 260.0).max(160.0);
        egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            if !offer.description.is_empty() {
                ui.add_space(4.0);
                ui.label(muted(&t, &offer.description));
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Das darf es").strong());
            for p in &offer.permissions {
                ui.label(format!("– {p}"));
            }
            ui.label(small_muted(&t, "Werkzeuge, die nur lesen, laufen direkt; alles andere fragt vorher, wie bei jedem Konnektor."));
            if !offer.tools.is_empty() {
                ui.label(small_muted(&t, format!("Werkzeuge laut Paket: {}", offer.tools.join(", "))));
            }
            if !offer.skills.is_empty() {
                ui.label(small_muted(&t, format!("Fähigkeiten: {}", offer.skills.join(", "))));
            }
            let forbids: Vec<&str> = offer.rules.iter().filter(|r| r.kind == "forbid").map(|r| r.text.as_str()).collect();
            if !forbids.is_empty() {
                ui.label(small_muted(&t, format!("Verbote, die übernommen werden: {}", forbids.join(" · "))));
            }
            for w in &offer.warnings {
                ui.label(RichText::new(w).small().color(t.warning));
            }
            if !offer.fields.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Einstellungen").strong());
                egui::Grid::new("ext-fields").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    for f in &offer.fields {
                        let title = if f.required { format!("{} *", f.title) } else { f.title.clone() };
                        ui.label(title).on_hover_text(&f.description);
                        let v = values.entry(f.key.clone()).or_default();
                        match f.kind.as_str() {
                            "boolean" => {
                                let mut b = v == "true";
                                if ui.checkbox(&mut b, "").changed() {
                                    *v = b.to_string();
                                }
                            }
                            "directory" | "file" => {
                                ui.vertical(|ui| {
                                    let edit = if f.multiple { egui::TextEdit::multiline(v).desired_rows(2) } else { egui::TextEdit::singleline(v) };
                                    ui.add(edit.desired_width(300.0).hint_text(if f.multiple { "ein Pfad pro Zeile" } else { "Pfad" }));
                                    if ui.small_button("Wählen …").clicked() {
                                        let picked =
                                            if f.kind == "directory" { rfd::FileDialog::new().pick_folder() } else { rfd::FileDialog::new().pick_file() };
                                        if let Some(p) = picked {
                                            let p = p.display().to_string();
                                            if f.multiple && !v.trim().is_empty() {
                                                v.push('\n');
                                                v.push_str(&p);
                                            } else {
                                                *v = p;
                                            }
                                        }
                                    }
                                });
                            }
                            _ => {
                                ui.add(egui::TextEdit::singleline(v).password(f.sensitive).desired_width(300.0));
                            }
                        }
                        ui.end_row();
                    }
                });
            }
        });
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let missing = offer.fields.iter().any(|f| f.required && values.get(&f.key).is_none_or(|v| v.trim().is_empty()));
            let label = if offer.update { "Aktualisieren und starten" } else { "Installieren und starten" };
            let clicked = if missing {
                ui.add_enabled(false, egui::Button::new(label));
                false
            } else {
                accent_button(ui, &t, label).clicked()
            };
            if clicked {
                let config: serde_json::Map<String, Value> = offer
                    .fields
                    .iter()
                    .filter_map(|f| {
                        let v = values.get(&f.key)?.clone();
                        let v = match f.kind.as_str() {
                            "boolean" => json!(v == "true"),
                            "directory" | "file" if f.multiple => json!(v.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>()),
                            _ => json!(v),
                        };
                        Some((f.key.clone(), v))
                    })
                    .collect();
                self.run("extension.install", json!({"id": offer.id, "config": config}));
            }
            if ui.button("Abbrechen").clicked() {
                self.run("extension.cancel", json!({}));
                keep = false;
            }
        });
        keep
    }
}
