//! „Apps verbinden“ (#26) in Einstellungen › Konnektoren: the catalog with categories and a
//! search, and the setup of one app (its fields, then the sign-in in the browser).

use std::collections::BTreeMap;

use egui::{RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{card, field_then_buttons, small_muted};

impl AisCraftApp {
    pub(crate) fn apps_catalog_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Apps verbinden").strong());
        ui.label(small_muted(
            &t,
            "Mail, Kalender und Dateien mit einer Anmeldung im Browser statt einer MCP-Konfiguration. Verbundene Apps lesen nur; Schreiben (Entwürfe, Termine) gibt es nur, wenn du es beim Verbinden ankreuzt, und es fragt jedes Mal vorher.",
        ));
        if let Some(name) = &self.state.app_sign_in {
            ui.label(RichText::new(format!("Anmeldung bei {name} läuft im Browser …")).small().color(t.warning));
        }
        ui.horizontal_wrapped(|ui| {
            let mut cats = vec![("", "Alle")];
            cats.extend(aiscraft_engine::catalog_categories().iter().map(|c| (*c, *c)));
            for (key, label) in cats {
                if ui.selectable_label(self.catalog_category == key, label).clicked() {
                    self.catalog_category = key.to_string();
                }
            }
        });
        let mut query = std::mem::take(&mut self.catalog_query);
        let has_text = !query.is_empty();
        let (_, clear) = field_then_buttons(
            ui,
            |ui| has_text && ui.small_button("Leeren").clicked(),
            |ui, w| ui.add(egui::TextEdit::singleline(&mut query).hint_text("Apps suchen, z. B. Outlook, Nextcloud").desired_width(w)),
        );
        if clear {
            query.clear();
        }
        self.catalog_query = query;
        let entries = aiscraft_engine::connector_catalog(&self.catalog_query, &self.catalog_category);
        if entries.is_empty() {
            ui.label(small_muted(&t, "Keine App gefunden."));
        }
        for e in entries {
            card(&t).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let clicked = field_then_buttons(
                    ui,
                    |ui| ui.small_button("Verbinden").clicked(),
                    |ui, w| {
                        ui.add_sized([w, 18.0], egui::Label::new(RichText::new(e.name).strong()).truncate());
                    },
                )
                .1;
                if clicked {
                    self.open_connect_app(e.id);
                }
                ui.label(small_muted(&t, format!("{} · {}", e.categories.join(", "), e.runs)));
                ui.add(egui::Label::new(RichText::new(e.description).small()).wrap());
            });
            ui.add_space(4.0);
        }
    }

    /// Open the setup of a catalog entry; false when there is none with this id.
    pub(crate) fn open_connect_app(&mut self, entry: &str) -> bool {
        if !aiscraft_engine::connector_catalog("", "").iter().any(|e| e.id == entry) {
            return false;
        }
        self.dialog = Some(Dialog::ConnectApp { entry: entry.to_string(), values: BTreeMap::new(), write: false });
        true
    }

    pub(crate) fn connect_app_body(&mut self, ui: &mut Ui, entry: &str, values: &mut BTreeMap<String, String>, write: &mut bool) -> bool {
        let t = self.tokens.clone();
        let Some(e) = aiscraft_engine::connector_catalog("", "").into_iter().find(|e| e.id == entry) else {
            return false;
        };
        ui.heading(e.name);
        ui.add(egui::Label::new(e.description).wrap());
        ui.add_space(4.0);
        ui.label(small_muted(&t, format!("Liest: {}", e.reads.join(", "))));
        ui.label(small_muted(&t, format!("Läuft: {}", e.runs)));
        if !e.writes.is_empty() {
            ui.checkbox(write, format!("Auch schreiben: {} (fragt jedes Mal vorher)", e.writes.join(", ")));
        }
        ui.add_space(6.0);
        egui::Grid::new("app-fields").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            for f in e.fields {
                ui.label(if f.required { format!("{} *", f.label) } else { f.label.to_string() });
                let v = values.entry(f.key.to_string()).or_default();
                ui.add(egui::TextEdit::singleline(v).password(f.secret).hint_text(f.hint).desired_width(300.0));
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.add(egui::Label::new(small_muted(&t, e.setup_hint)).wrap());
        if e.auth == "oauth" {
            ui.label(small_muted(&t, "Zugangsdaten und Anmeldung liegen im Schlüsselbund deines Systems."));
        }
        ui.add_space(8.0);
        let mut keep = true;
        ui.horizontal(|ui| {
            let missing = e.fields.iter().any(|f| f.required && values.get(f.key).is_none_or(|v| v.trim().is_empty()));
            let label = match e.auth {
                "oauth" => "Im Browser anmelden",
                "own" => "Verbinden (startet den MCP-Server)",
                _ => "Verbinden",
            };
            if ui.add_enabled(!missing, egui::Button::new(label)).clicked() {
                let mut p = serde_json::Map::new();
                p.insert("entry".into(), json!(e.id));
                p.insert("write".into(), json!(*write));
                for (k, v) in values.iter().filter(|(_, v)| !v.trim().is_empty()) {
                    p.insert(k.clone(), Value::String(v.trim().to_string()));
                }
                self.run("connector.connect", Value::Object(p));
                keep = false;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        keep
    }
}
