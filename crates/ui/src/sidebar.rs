use aiscraft_core::BackendKind;
use aiscraft_engine::BackendStatus;
use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Sense, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{age_group, icon_button, muted, small_muted, status_dot};

impl AisCraftApp {
    pub(crate) fn sidebar(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        egui::Panel::bottom("sidebar-account").frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 2 })).show(ui, |ui| self.account_row(ui));
        ui.horizontal(|ui| {
            ui.label(RichText::new("AISCraft").strong().size(17.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, "☰", "Seitenleiste ausblenden (Strg+B)").clicked() {
                    self.sidebar_open = false;
                }
                if icon_button(ui, &t, "⌘", "Befehle (Strg+K)").clicked() {
                    self.palette = Some(Default::default());
                }
            });
        });
        ui.add_space(8.0);

        let new_btn = egui::Button::new(RichText::new("+  Neuer Chat").strong()).min_size(egui::vec2(ui.available_width(), 32.0)).corner_radius(CornerRadius::same(t.radius));
        if ui.add(new_btn).on_hover_text("Strg+N").clicked() {
            self.new_chat();
        }
        ui.add_space(6.0);

        let search = ui.add(egui::TextEdit::singleline(&mut self.search_text).hint_text("🔍  Chats durchsuchen").desired_width(f32::INFINITY));
        if self.focus_search {
            search.request_focus();
            self.focus_search = false;
        }
        if search.changed() {
            if self.search_text.trim().is_empty() {
                self.run("search.clear", json!({}));
            } else {
                self.run("search", json!({"query": self.search_text}));
            }
        }
        ui.add_space(8.0);

        ScrollArea::vertical().id_salt("sidebar-list").auto_shrink([false, false]).show(ui, |ui| {
            if self.state.search.is_some() && !self.search_text.trim().is_empty() {
                self.search_results(ui);
            } else {
                self.projects_section(ui);
                ui.add_space(4.0);
                self.chat_list(ui);
                self.remote_section(ui);
            }
        });
    }

    fn search_results(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let hits = self.state.search.as_ref().map(|s| s.hits.clone()).unwrap_or_default();
        if hits.is_empty() {
            ui.label(muted(&t, "Nichts gefunden."));
            return;
        }
        ui.label(small_muted(&t, format!("{} Treffer", hits.len())));
        for h in hits {
            let r = ui.add(egui::Button::new(RichText::new(if h.title.is_empty() { "Ohne Titel" } else { &h.title }).strong()).frame(false).wrap());
            let snippet = h.snippet.replace(['«', '»'], "");
            ui.add(egui::Label::new(small_muted(&t, snippet)).wrap());
            ui.add_space(4.0);
            if r.clicked() {
                self.run("chat.open", json!({"id": h.conversation_id}));
            }
        }
    }

    fn projects_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.horizontal(|ui| {
            ui.label(small_muted(&t, "PROJEKTE"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, &t, "+", "Neues Projekt").clicked() {
                    self.dialog = Some(Dialog::project(None));
                }
            });
        });
        let projects = self.state.projects.clone();
        if projects.is_empty() {
            ui.label(small_muted(&t, "Projekte bündeln Chats mit eigenen Anweisungen."));
        }
        for p in projects {
            let active = self.project_filter.as_deref() == Some(&p.id);
            let r = ui.add(egui::Button::selectable(active, format!("▣  {}", p.name)).min_size(egui::vec2(ui.available_width(), 24.0)));
            if r.clicked() {
                if active {
                    self.project_filter = None;
                    self.run("chat.new", json!({"project": null}));
                } else {
                    self.project_filter = Some(p.id.clone());
                    self.run("chat.new", json!({"project": p.id}));
                }
            }
            r.context_menu(|ui| {
                if ui.button("Bearbeiten").clicked() {
                    self.dialog = Some(Dialog::project(Some(&p)));
                    ui.close();
                }
                if ui.button("Neuer Chat im Projekt").clicked() {
                    self.run("chat.new", json!({"project": p.id}));
                    ui.close();
                }
                if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.dialog = Some(Dialog::Confirm {
                        text: format!("Projekt „{}“ löschen? Die Chats bleiben erhalten.", p.name),
                        command: "project.delete".into(),
                        params: json!({"id": p.id}),
                    });
                    ui.close();
                }
            });
        }
    }

    fn chat_list(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let convs: Vec<_> = self.state.conversations.iter().filter(|c| self.project_filter.as_ref().is_none_or(|p| c.project_id.as_ref() == Some(p))).cloned().collect();
        if convs.is_empty() {
            ui.add_space(8.0);
            ui.label(small_muted(&t, if self.project_filter.is_some() { "Noch keine Chats in diesem Projekt." } else { "Noch keine Chats." }));
            return;
        }
        let mut last_group = "";
        for c in convs {
            let group = if c.pinned { "Angeheftet" } else { age_group(c.updated_at) };
            if group != last_group {
                ui.add_space(6.0);
                ui.label(small_muted(&t, group.to_uppercase()));
                last_group = group;
            }
            let selected = self.state.current.as_deref() == Some(&c.id);
            let generating = self.state.is_generating(&c.id);
            let w = ui.available_width();
            let resp = ui.add(egui::Button::selectable(selected, RichText::new(&c.title)).truncate().min_size(egui::vec2(w, 26.0)));
            if generating {
                let r = resp.rect;
                let center = egui::pos2(r.right() - 12.0, r.center().y);
                let tt = ui.input(|i| i.time) as f32;
                for k in 0..3 {
                    let a = (tt * 3.0 - k as f32 * 0.6).sin() * 0.5 + 0.5;
                    ui.painter().circle_filled(center + egui::vec2(-(k as f32) * 7.0, 0.0), 2.2, t.accent.gamma_multiply(0.35 + 0.65 * a));
                }
            }
            if resp.clicked() {
                self.run("chat.open", json!({"id": c.id}));
            }
            resp.context_menu(|ui| {
                if ui.button("Umbenennen").clicked() {
                    self.dialog = Some(Dialog::Rename { id: c.id.clone(), title: c.title.clone() });
                    ui.close();
                }
                if ui.button(if c.pinned { "Lösen" } else { "Anheften" }).clicked() {
                    self.run("chat.pin", json!({"id": c.id, "pinned": !c.pinned}));
                    ui.close();
                }
                ui.menu_button("In Projekt", |ui| {
                    if ui.button("Kein Projekt").clicked() {
                        self.run("chat.move", json!({"id": c.id, "project": null}));
                        ui.close();
                    }
                    for p in self.state.projects.clone() {
                        if ui.button(&p.name).clicked() {
                            self.run("chat.move", json!({"id": c.id, "project": p.id}));
                            ui.close();
                        }
                    }
                });
                if let Some(url) = self.state.backend(&c.backend_id).filter(|b| b.config.kind == BackendKind::AisChat).map(|b| format!("{}/d/{}", b.config.base_url.trim_end_matches('/'), c.id))
                    && ui.button("In AIS.chat öffnen").clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.dialog = Some(Dialog::Confirm { text: format!("„{}“ löschen?", c.title), command: "chat.delete".into(), params: json!({"id": c.id}) });
                    ui.close();
                }
            });
        }
    }

    /// Conversations on the AIS.chat server that are not here yet.
    fn remote_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let ais: Vec<_> = self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. })).cloned().collect();
        for b in ais {
            ui.add_space(10.0);
            let header = egui::CollapsingHeader::new(small_muted(&t, format!("{} – VERLAUF", b.config.name.to_uppercase()))).id_salt(("remote", &b.config.id)).default_open(false).show(ui, |ui| {
                let Some(list) = self.state.remote.get(&b.config.id).cloned() else {
                    ui.add(egui::Spinner::new());
                    return;
                };
                let local: std::collections::HashSet<&str> = self.state.conversations.iter().map(|c| c.id.as_str()).collect();
                let missing: Vec<_> = list.iter().filter(|r| !local.contains(r.id.as_str())).collect();
                if missing.is_empty() {
                    ui.label(small_muted(&t, "Alles schon hier."));
                }
                for r in missing.into_iter().take(60) {
                    if ui.add(egui::Button::new(RichText::new(&r.title).color(t.muted)).frame(false).truncate()).on_hover_text("Hierher übernehmen und weiterschreiben").clicked() {
                        self.run("remote.import", json!({"backend": b.config.id, "id": r.id, "title": r.title}));
                    }
                }
            });
            if header.fully_open() && !self.state.remote.contains_key(&b.config.id) && !self.remote_open {
                self.remote_open = true;
                self.run("remote.refresh", json!({"backend": b.config.id}));
            }
            if !header.fully_open() {
                self.remote_open = false;
            }
        }
    }

    fn account_row(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let sel = self.state.selection.backend_id.clone();
        let view = self.state.backend(&sel).cloned();
        let (color, line1, line2) = match view.as_ref().map(|b| &b.status) {
            Some(BackendStatus::Ready { account }) => {
                (t.success, account.display_name.clone().unwrap_or_else(|| view.as_ref().map(|b| b.config.name.clone()).unwrap_or_default()), account.detail.clone().unwrap_or_default())
            }
            Some(BackendStatus::Checking) => (t.warning, "Verbinde …".into(), String::new()),
            Some(BackendStatus::SignedOut { .. }) | Some(BackendStatus::Unknown) => (t.muted, view.as_ref().map(|b| b.config.name.clone()).unwrap_or_default(), "Nicht angemeldet".into()),
            Some(BackendStatus::Failed { message }) => (t.danger, view.as_ref().map(|b| b.config.name.clone()).unwrap_or_default(), message.clone()),
            None => (t.muted, "Kein Backend".into(), String::new()),
        };
        let r = ui
            .horizontal(|ui| {
                status_dot(ui, color);
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(line1).strong()).truncate());
                    if !line2.is_empty() {
                        ui.add(egui::Label::new(small_muted(&t, line2)).truncate());
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, &t, "⚙", "Einstellungen (Strg+,)").clicked() {
                        self.dialog = Some(Dialog::settings());
                    }
                    if icon_button(ui, &t, "📌", "Gedächtnis").clicked() {
                        self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Memory, new_note: String::new() });
                    }
                });
            })
            .response
            .interact(Sense::click());
        if r.clicked() {
            if let Some(b) = view.filter(|b| !matches!(b.status, BackendStatus::Ready { .. })) {
                self.dialog = Some(Dialog::login(&b.config.id));
            } else {
                self.dialog = Some(Dialog::settings());
            }
        }
    }
}
