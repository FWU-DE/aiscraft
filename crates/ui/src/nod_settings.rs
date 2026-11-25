//! Settings sections for features taken from Nod (AGENTS.md §5), kept out of `dialogs.rs`: the
//! writing style (#20), briefing templates for schedules (#27), schedule ideas and the model per
//! schedule (#68), backup models (#66).

use aiscraft_core::MemoryArea;
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{card, small_muted};

impl AisCraftApp {
    /// Einstellungen › Gedächtnis: the writing style learned from the owner's own messages (#20).
    pub(crate) fn style_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new(MemoryArea::Style.label()).strong());
        ui.label(small_muted(
            &t,
            "AIS.ChatCraft lernt aus deinen eigenen Nachrichten, wie du schreibst, damit Entwürfe (Mails, Elternbriefe, Antworten) nach dir klingen. Namen und Zahlen werden vorher entfernt; Antworten, Dateien und Werkzeug-Ergebnisse zählen nicht. Der Stil geht nur bei Schreibaufträgen mit und lässt sich pro Chat unter „Quellen“ abschalten.",
        ));
        let note = self.state.memory.iter().rev().find(|n| n.area == MemoryArea::Style && !n.archived).cloned();
        if let Some(n) = &note {
            ui.add(egui::Label::new(RichText::new(&n.text)).wrap()).on_hover_text(&n.source_quote);
        }
        ui.horizontal_wrapped(|ui| {
            let label = if note.is_some() { "Schreibstil neu lernen" } else { "Schreibstil aus meinen Nachrichten lernen" };
            if ui.button(label).clicked() {
                self.run("style.learn", json!({}));
            }
            if let Some(n) = &note
                && ui.small_button("Vergessen").clicked()
            {
                self.run("memory.archive", json!({"id": n.id}));
            }
        });
    }

    /// Einstellungen › Zeitpläne: briefing templates (#27), each with what it still needs.
    pub(crate) fn schedule_templates_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Vorlagen").strong());
        ui.label(small_muted(&t, "Mit einem Klick als Zeitplan anlegen; danach lässt er sich wie jeder andere ändern."));
        for tpl in aiscraft_engine::template_views(&self.state) {
            card(&t).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tpl.title).strong());
                    ui.label(small_muted(&t, tpl.rhythm));
                });
                ui.add(egui::Label::new(small_muted(&t, tpl.prompt.replace("{url}", "…"))).wrap());
                let url_id = egui::Id::new(("schedule-template-url", tpl.id));
                let mut url: String = ui.data_mut(|d| d.get_temp::<String>(url_id)).unwrap_or_default();
                let ready = tpl.missing.is_empty() && (!tpl.needs_url || !url.trim().is_empty());
                let mut create = false;
                if tpl.needs_url {
                    crate::widgets::field_then_buttons(
                        ui,
                        |ui| create = ui.add_enabled(ready, egui::Button::new("Anlegen")).clicked(),
                        |ui, w| ui.add(egui::TextEdit::singleline(&mut url).hint_text("Adresse der Seite, https://…").desired_width(w)),
                    );
                    ui.data_mut(|d| d.insert_temp(url_id, url.clone()));
                } else {
                    create = ui.add_enabled(ready, egui::Button::new("Anlegen")).clicked();
                }
                for m in &tpl.missing {
                    ui.add(egui::Label::new(RichText::new(format!("Fehlt – {m}")).small().color(t.warning)).wrap());
                }
                if create {
                    let mut p = json!({"template": tpl.id});
                    if tpl.needs_url {
                        p["url"] = json!(url.trim());
                    }
                    self.run("schedule.from_template", p);
                    ui.data_mut(|d| d.remove::<String>(url_id));
                }
            });
            ui.add_space(4.0);
        }
    }

    /// Einstellungen › Zeitpläne: schedules suggested from requests the owner keeps making (#68).
    pub(crate) fn schedule_ideas_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Ideen aus deinen Bitten").strong());
        let ideas = self.state.schedule_ideas.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(small_muted(
                &t,
                if ideas.is_empty() {
                    "Bittest du an mindestens drei Tagen um dasselbe, zur gleichen Zeit oder am gleichen Wochentag, schlägt AIS.ChatCraft hier einen Zeitplan vor. Gesucht wird nur auf diesem Gerät, in deinen eigenen Nachrichten."
                } else {
                    "Diese Bitten stellst du immer wieder. Gesucht wurde nur auf diesem Gerät, in deinen eigenen Nachrichten."
                },
            ));
            if ui.small_button("Jetzt suchen").clicked() {
                self.run("schedule.ideas", json!({}));
            }
        });
        for idea in ideas {
            card(&t).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&idea.title).strong());
                    ui.label(small_muted(&t, format!("{} · {}", idea.rhythm, self.state.model_name(&idea.backend_id, &idea.model_id))));
                });
                ui.add(egui::Label::new(small_muted(&t, format!("„{}“, an {} Tagen", idea.prompt.chars().take(200).collect::<String>(), idea.days))).wrap());
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Als Zeitplan anlegen").clicked() {
                        self.run("schedule.idea_accept", json!({"id": idea.id}));
                    }
                    if ui.button("Anpassen …").clicked() {
                        self.dialog = Some(crate::dialogs::Dialog::AddSchedule {
                            title: idea.title.clone(),
                            prompt: idea.prompt.clone(),
                            when: idea.cron.clone(),
                            recurring: true,
                            watch: false,
                        });
                    }
                    if ui.small_button("Nein danke").clicked() {
                        self.run("schedule.idea_dismiss", json!({"id": idea.id}));
                    }
                });
            });
            ui.add_space(4.0);
        }
    }

    /// The model a schedule runs with (#68), chosen in its card.
    pub(crate) fn schedule_model_menu(&mut self, ui: &mut Ui, sch: &aiscraft_engine::Schedule) {
        let t = self.tokens.clone();
        ui.horizontal(|ui| {
            ui.label(small_muted(&t, "Modell"));
            let current = self.state.model_name(&sch.backend_id, &sch.model_id);
            let mut picked = None;
            egui::ComboBox::from_id_salt(("schedule-model", &sch.id)).selected_text(current).show_ui(ui, |ui| {
                for b in &self.state.backends {
                    for m in &b.models {
                        let on = b.config.id == sch.backend_id && m.id == sch.model_id;
                        if ui.selectable_label(on, format!("{} · {}", m.name, b.config.name)).clicked() {
                            picked = Some((b.config.id.clone(), m.id.clone()));
                        }
                    }
                }
            });
            if let Some((backend, model)) = picked {
                self.run("schedule.update", json!({"id": sch.id, "backend": backend, "model": model}));
            }
        });
    }

    /// Einstellungen › Dienste: backup models in order (#66).
    pub(crate) fn fallback_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(RichText::new("Ersatzmodelle").strong());
        ui.label(small_muted(
            &t,
            "Scheitert eine Anfrage am Modell (überlastet, Serverfehler, Zeitüberschreitung, Kontingent aufgebraucht), antwortet das erste passende Ersatzmodell. Bei Fehlern am Inhalt, etwa einem Inhaltsfilter, wird nicht gewechselt. Unter der Antwort steht, welches Modell eingesprungen ist.",
        ));
        let list = self.state.settings.fallback_models.clone();
        let as_param = |l: &[aiscraft_engine::FallbackModel]| l.iter().map(|f| json!({"backend": f.backend_id, "model": f.model_id})).collect::<Vec<_>>();
        let mut change: Option<Vec<aiscraft_engine::FallbackModel>> = None;
        for (i, f) in list.iter().enumerate() {
            let service = self.state.backend(&f.backend_id).map(|b| b.config.name.clone()).unwrap_or_default();
            let name = format!("{}. {} · {service}", i + 1, self.state.model_name(&f.backend_id, &f.model_id));
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Entfernen").on_hover_text(name.clone()).clicked() {
                        let mut l = list.clone();
                        l.remove(i);
                        change = Some(l);
                    }
                    if i > 0 && ui.small_button("Nach oben").on_hover_text(name.clone()).clicked() {
                        let mut l = list.clone();
                        l.swap(i, i - 1);
                        change = Some(l);
                    }
                    ui.add(egui::Label::new(name.clone()).truncate());
                });
            });
        }
        if list.len() < 5 {
            let mut add = None;
            egui::ComboBox::from_id_salt("fallback-add").selected_text("Ersatzmodell hinzufügen").show_ui(ui, |ui| {
                for b in &self.state.backends {
                    for m in &b.models {
                        let f = aiscraft_engine::FallbackModel { backend_id: b.config.id.clone(), model_id: m.id.clone() };
                        if !list.contains(&f) && ui.selectable_label(false, format!("{} · {}", m.name, b.config.name)).clicked() {
                            add = Some(f);
                        }
                    }
                }
            });
            if let Some(f) = add {
                let mut l = list.clone();
                l.push(f);
                change = Some(l);
            }
        }
        if let Some(l) = change {
            self.run("fallback.set", json!({"models": as_param(&l)}));
        }
        let mut others = self.state.settings.fallback_other_services;
        if ui.checkbox(&mut others, "Auch Ersatzmodelle bei anderen Diensten nutzen").changed() {
            self.run("fallback.set", json!({"other_services": others}));
        }
        ui.label(small_muted(
            &t,
            "Dann bekommt der andere Dienst den Chat. Nie für Chats, deren Verlauf beim Dienst liegt (AIS.chat), und nie für Assistenten. Aus: nur Ersatzmodelle beim selben Dienst.",
        ));
    }
}
