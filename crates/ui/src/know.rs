//! Project knowledge (#32), research (#37), voice mode (#40), connector results for AIS.chat
//! (#53) and templates (#54): the dialogs, bars and sections. The engine does the work; this
//! file only draws and dispatches commands.

use egui::{Id, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::icons::Icon;
use crate::widgets::small_muted;

/// The composer command that opens the connector dialog (#53).
pub const CONNECTOR_COMMAND: &str = "/konnektor";

#[derive(Debug, Clone)]
pub enum KnowDialog {
    /// Fill a template's fields.
    UseTemplate {
        id: String,
        values: Vec<(String, String)>,
    },
    EditTemplate {
        id: Option<String>,
        title: String,
        text: String,
        persona: Option<(String, Value)>,
    },
    /// A read-only connector tool with the person's own arguments.
    Connector {
        tool: Option<String>,
        args: Vec<(String, String)>,
    },
}

impl KnowDialog {
    pub fn use_template(t: &aiscraft_engine::templates::Template) -> Self {
        let values = t
            .fields()
            .into_iter()
            .map(|f| {
                let v = t.last.iter().find(|(k, _)| k.eq_ignore_ascii_case(&f.name)).map(|(_, v)| v.clone()).unwrap_or(f.default);
                (f.name, v)
            })
            .collect();
        KnowDialog::UseTemplate { id: t.id.clone(), values }
    }
    pub fn edit_template(t: Option<&aiscraft_engine::templates::Template>) -> Self {
        KnowDialog::EditTemplate {
            id: t.map(|t| t.id.clone()),
            title: t.map(|t| t.title.clone()).unwrap_or_default(),
            text: t.map(|t| t.text.clone()).unwrap_or_default(),
            persona: t
                .and_then(|t| t.persona.as_ref().map(|p| (p.name.clone(), json!({"kind": p.kind.as_str(), "id": p.id, "name": p.name, "backend": t.backend})))),
        }
    }
}

/// The fields of a connector tool's input schema: (name, required, description).
fn schema_fields(schema: &Value) -> Vec<(String, bool, String)> {
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .take(20)
                .map(|(k, v)| {
                    (k.clone(), required.contains(&k.as_str()), v.get("description").and_then(Value::as_str).unwrap_or("").chars().take(120).collect())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A typed value: numbers, true/false and JSON stay what they are, the rest is text.
fn typed(v: &str, kind: Option<&str>) -> Value {
    match kind {
        Some("number") | Some("integer") => v.trim().parse::<f64>().map(|n| json!(n)).unwrap_or_else(|_| json!(v)),
        Some("boolean") => json!(matches!(v.trim(), "true" | "ja" | "1")),
        Some("array") | Some("object") => serde_json::from_str(v).unwrap_or_else(|_| json!(v)),
        _ => json!(v),
    }
}

impl AisCraftApp {
    /// Text the engine put into the composer (#53, #54), and `/konnektor` typed there.
    pub(crate) fn know_frame(&mut self, ctx: &egui::Context) {
        if let Some(fill) = self.state.composer_fill.clone() {
            let id = Id::unique("composer-fill-seen");
            let seen = ctx.data(|d| d.get_temp::<u64>(id)).unwrap_or(0);
            if fill.seq != seen {
                ctx.data_mut(|d| d.insert_temp(id, fill.seq));
                if fill.replace || self.composer.trim().is_empty() {
                    self.composer = fill.text;
                } else {
                    if !self.composer.ends_with('\n') {
                        self.composer.push('\n');
                    }
                    self.composer.push_str(&fill.text);
                }
                self.focus_composer = true;
            }
        }
        if self.composer.trim() == CONNECTOR_COMMAND && self.dialog.is_none() {
            self.composer.clear();
            self.dialog = Some(crate::dialogs::Dialog::Know(KnowDialog::Connector { tool: None, args: Vec::new() }));
        }
    }

    /// The dialogs of this file; `false` closes.
    pub(crate) fn know_dialog(&mut self, ui: &mut Ui, d: &mut KnowDialog) -> bool {
        let t = self.tokens.clone();
        let mut keep = true;
        match d {
            KnowDialog::UseTemplate { id, values } => {
                let Some(tpl) = self.state.templates.iter().find(|x| &x.id == id).cloned() else { return false };
                ui.heading(&tpl.title);
                if let Some(p) = &tpl.persona {
                    ui.label(small_muted(&t, format!("Geht an: {} ({})", p.name, p.kind.label())));
                }
                ui.add_space(6.0);
                egui::Grid::new("template-fields").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    for (name, value) in values.iter_mut() {
                        ui.label(name.as_str());
                        let long = name.to_lowercase().contains("text") || value.len() > 60;
                        if long {
                            ui.add(egui::TextEdit::multiline(value).desired_rows(3).desired_width(300.0));
                        } else {
                            ui.add(egui::TextEdit::singleline(value).desired_width(300.0));
                        }
                        ui.end_row();
                    }
                });
                if values.is_empty() {
                    ui.label(small_muted(&t, "Diese Vorlage hat keine Felder."));
                }
                ui.add_space(8.0);
                let filled = values.iter().all(|(_, v)| !v.trim().is_empty());
                let params = |send: bool| {
                    let map: serde_json::Map<String, Value> = values.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
                    json!({"id": id, "values": map, "send": send})
                };
                let mut edit = false;
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(filled, egui::Button::new("Senden")).clicked() {
                        self.run("template.use", params(true));
                        keep = false;
                    }
                    if ui.add_enabled(filled, egui::Button::new("Ins Eingabefeld")).clicked() {
                        self.run("template.use", params(false));
                        keep = false;
                    }
                    if ui.button("Bearbeiten").clicked() {
                        edit = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                if !filled {
                    ui.label(small_muted(&t, "Fülle alle Felder aus."));
                }
                if edit {
                    *d = KnowDialog::edit_template(Some(&tpl));
                }
            }
            KnowDialog::EditTemplate { id, title, text, persona } => {
                ui.heading(if id.is_some() { "Vorlage bearbeiten" } else { "Neue Vorlage" });
                ui.add_space(6.0);
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(title).hint_text("z. B. Elternbrief").desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label("Text");
                ui.add(
                    egui::TextEdit::multiline(text)
                        .hint_text("z. B. Schreibe einen Elternbrief für die Klasse {{Klasse}} zum Thema {{Thema}}.")
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
                ui.label(small_muted(&t, "Felder in doppelten geschweiften Klammern: {{Klasse}}, mit Vorgabe {{Niveau=mittel}}."));
                let fields = aiscraft_engine::templates::fields(text);
                if !fields.is_empty() {
                    ui.label(small_muted(&t, format!("Felder: {}", fields.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(", "))));
                }
                // An AIS.chat assistant, character or scenario as the target.
                let options: Vec<(String, Value)> = self
                    .state
                    .backends
                    .iter()
                    .flat_map(|b| {
                        b.personas.iter().flatten().map(move |p| {
                            (format!("{} ({})", p.name, p.kind.label()), json!({"kind": p.kind.as_str(), "id": p.id, "name": p.name, "backend": b.config.id}))
                        })
                    })
                    .take(300)
                    .collect();
                if !options.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label("Ziel");
                        let current = persona.as_ref().map(|(n, _)| n.clone()).unwrap_or_else(|| "Neuer Chat".to_string());
                        egui::ComboBox::from_id_salt("template-target").selected_text(current).width(260.0).show_ui(ui, |ui| {
                            if ui.selectable_label(persona.is_none(), "Neuer Chat").clicked() {
                                *persona = None;
                            }
                            for (label, v) in &options {
                                let on = persona.as_ref().is_some_and(|(_, pv)| pv.get("id") == v.get("id"));
                                if ui.selectable_label(on, label).clicked() {
                                    *persona = Some((label.clone(), v.clone()));
                                }
                            }
                        });
                    });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = !title.trim().is_empty() && !text.trim().is_empty();
                    if ui.add_enabled(ok, egui::Button::new("Speichern")).clicked() {
                        let mut p = serde_json::Map::new();
                        p.insert("title".into(), json!(title));
                        p.insert("text".into(), json!(text));
                        if let Some(id) = id {
                            p.insert("id".into(), json!(id));
                        }
                        if let Some((_, v)) = persona {
                            p.insert("persona".into(), v.clone());
                            p.insert("backend".into(), v.get("backend").cloned().unwrap_or(Value::Null));
                        }
                        self.run("template.save", Value::Object(p));
                        keep = false;
                    }
                    if let Some(id) = id
                        && ui.button("Löschen").clicked()
                    {
                        self.run("template.delete", json!({"id": id}));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
            }
            KnowDialog::Connector { tool, args } => {
                ui.heading("Konnektor-Ergebnis einfügen");
                ui.label(small_muted(
                    &t,
                    "Ein Werkzeug, das nur liest, mit deinen Angaben. Das Ergebnis kommt ins Eingabefeld; du prüfst es und schickst es selbst ab.",
                ));
                ui.add_space(6.0);
                let tools: Vec<(String, String, Value)> = self
                    .state
                    .connectors
                    .iter()
                    .filter(|k| k.enabled)
                    .flat_map(|k| {
                        k.tools.iter().filter(|x| x.read_only).map(move |x| {
                            (x.llm_name.clone(), format!("{} · {}", k.name, if x.title.is_empty() { &x.name } else { &x.title }), x.input_schema.clone())
                        })
                    })
                    .collect();
                if tools.is_empty() {
                    ui.label("Kein verbundener Konnektor hat Werkzeuge, die nur lesen. Konnektoren richtest du in den Einstellungen ein.");
                } else {
                    let current = tool
                        .as_ref()
                        .and_then(|x| tools.iter().find(|(n, _, _)| n == x))
                        .map(|(_, l, _)| l.clone())
                        .unwrap_or_else(|| "Werkzeug wählen".into());
                    egui::ComboBox::from_id_salt("connector-tool").selected_text(current).width(340.0).show_ui(ui, |ui| {
                        for (name, label, schema) in &tools {
                            if ui.selectable_label(tool.as_deref() == Some(name.as_str()), label).clicked() {
                                *tool = Some(name.clone());
                                *args = schema_fields(schema).into_iter().map(|(k, _, _)| (k, String::new())).collect();
                            }
                        }
                    });
                    let schema = tool.as_ref().and_then(|x| tools.iter().find(|(n, _, _)| n == x)).map(|(_, _, s)| s.clone()).unwrap_or(Value::Null);
                    let fields = schema_fields(&schema);
                    if tool.is_some() {
                        ui.add_space(6.0);
                        egui::Grid::new("connector-args").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                            for (name, value) in args.iter_mut() {
                                let (req, desc) = fields.iter().find(|(k, _, _)| k == name).map(|(_, r, d)| (*r, d.clone())).unwrap_or((false, String::new()));
                                ui.label(if req { format!("{name} *") } else { name.clone() });
                                ui.add(egui::TextEdit::singleline(value).hint_text(desc).desired_width(280.0));
                                ui.end_row();
                            }
                        });
                    }
                    ui.add_space(8.0);
                    let missing = fields.iter().any(|(k, r, _)| *r && args.iter().find(|(n, _)| n == k).is_none_or(|(_, v)| v.trim().is_empty()));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(tool.is_some() && !missing, egui::Button::new("Einfügen")).clicked() {
                            let props = schema.get("properties").cloned().unwrap_or(Value::Null);
                            let map: serde_json::Map<String, Value> = args
                                .iter()
                                .filter(|(_, v)| !v.trim().is_empty())
                                .map(|(k, v)| (k.clone(), typed(v, props.get(k).and_then(|p| p.get("type")).and_then(Value::as_str))))
                                .collect();
                            self.run("connector.insert", json!({"tool": tool, "arguments": map}));
                            keep = false;
                        }
                        if ui.button("Abbrechen").clicked() {
                            keep = false;
                        }
                    });
                }
                if tools.is_empty() && ui.button("Schließen").clicked() {
                    keep = false;
                }
            }
        }
        keep
    }

    /// Templates on the start page (#54).
    pub(crate) fn templates_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
            ui.label(small_muted(&t, "Vorlagen"));
            let list = self.state.templates.clone();
            for tpl in list.iter().take(12) {
                if crate::kit::pill(ui, &t, crate::kit::Pill::Chip, None, &tpl.title, 13.0, None).on_hover_text("Felder ausfüllen und senden").clicked() {
                    self.dialog = Some(crate::dialogs::Dialog::Know(KnowDialog::use_template(tpl)));
                }
            }
            if crate::kit::pill(ui, &t, crate::kit::Pill::Chip, Some(Icon::Plus), "Neue Vorlage", 13.0, None).clicked() {
                self.dialog = Some(crate::dialogs::Dialog::Know(KnowDialog::edit_template(None)));
            }
        });
    }

    /// `ui.show` targets of this file; `false` when `what` is none of them.
    pub(crate) fn show_know(&mut self, what: &str) -> bool {
        use crate::dialogs::Dialog;
        let d = match what {
            "connector_insert" => KnowDialog::Connector { tool: None, args: Vec::new() },
            "template_new" => KnowDialog::edit_template(None),
            w if w.starts_with("template:") => match self.state.templates.iter().find(|t| Some(t.id.as_str()) == w.strip_prefix("template:")) {
                Some(t) => KnowDialog::use_template(t),
                None => return false,
            },
            _ => return false,
        };
        self.dialog = Some(Dialog::Know(d));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connector_fields_come_from_the_schema() {
        let s = json!({"properties": {"query": {"type": "string", "description": "Suchwörter"}, "max": {"type": "integer"}}, "required": ["query"]});
        let f = schema_fields(&s);
        assert!(f.contains(&("query".into(), true, "Suchwörter".into())));
        assert_eq!(typed("5", Some("integer")), json!(5.0));
        assert_eq!(typed("ja", Some("boolean")), json!(true));
        assert_eq!(typed("x", None), json!("x"));
        assert!(schema_fields(&Value::Null).is_empty());
    }
}
