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
        egui::Panel::bottom("sidebar-account")
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 2 }))
            .show(ui, |ui| self.account_row(ui));
        ui.horizontal(|ui| {
            crate::widgets::ais_logo(ui, 18.0, t.accent);
            ui.label(RichText::new("AIS.ChatCraft").strong().size(17.0));
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

        let new_btn = egui::Button::new(RichText::new("+  Neuer Chat").strong())
            .min_size(egui::vec2(ui.available_width(), 32.0))
            .corner_radius(CornerRadius::same(t.radius));
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
            // Leave room for the scroll bar, so it never covers the "+" buttons on the right.
            ui.set_max_width((ui.available_width() - 12.0).max(80.0));
            if self.state.search.is_some() && !self.search_text.trim().is_empty() {
                self.search_results(ui);
            } else {
                self.projects_section(ui);
                ui.add_space(4.0);
                self.artifacts_section(ui);
                ui.add_space(4.0);
                self.personas_section(ui);
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

    fn artifacts_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.horizontal(|ui| {
            ui.label(small_muted(&t, "DOKUMENTE & BOARDS"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(RichText::new("+").color(t.muted), |ui| {
                    if ui.button("📄  Neues Dokument").clicked() {
                        self.run("artifact.create", json!({"kind": "document"}));
                        ui.close();
                    }
                    if ui.button("▣  Neues Whiteboard").clicked() {
                        self.run("artifact.create", json!({"kind": "board"}));
                        ui.close();
                    }
                    if self.state.collab.available {
                        ui.separator();
                        self.collab_join_row(ui);
                    }
                });
            });
        });
        let items: Vec<_> =
            self.state.artifacts.iter().filter(|a| self.project_filter.as_ref().is_none_or(|p| a.project_id.as_ref() == Some(p))).take(12).cloned().collect();
        if items.is_empty() {
            ui.label(small_muted(&t, "„Als Dokument“ an einer Antwort legt eins an."));
        }
        for a in items {
            let open = self.state.open_artifact.as_deref() == Some(&a.id);
            let icon = crate::learn_panel::kind_icon(a.kind);
            let r = ui.add(egui::Button::selectable(open, format!("{icon}  {}", a.title)).truncate().min_size(egui::vec2(ui.available_width(), 24.0)));
            if r.clicked() {
                if open {
                    self.save_if_dirty();
                    self.run("artifact.close", json!({}));
                } else {
                    self.save_if_dirty();
                    self.run("artifact.open", json!({"id": a.id}));
                }
            }
            r.context_menu(|ui| {
                if ui.button("Umbenennen").clicked() {
                    self.dialog = Some(Dialog::RenameArtifact { id: a.id.clone(), title: a.title.clone() });
                    ui.close();
                }
                if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.dialog =
                        Some(Dialog::Confirm { text: format!("„{}“ löschen?", a.title), command: "artifact.delete".into(), params: json!({"id": a.id}) });
                    ui.close();
                }
            });
        }
    }

    fn chat_list(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let convs: Vec<_> =
            self.state.conversations.iter().filter(|c| self.project_filter.as_ref().is_none_or(|p| c.project_id.as_ref() == Some(p))).cloned().collect();
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
            let is_ais = self.state.backend(&c.backend_id).is_some_and(|b| b.config.kind == BackendKind::AisChat);
            // AIS.chat chats get a small AIS.chat logo; the spaces make room for it.
            let title = if is_ais { format!("      {}", c.title) } else { c.title.clone() };
            let resp = ui.add(egui::Button::selectable(selected, RichText::new(title)).truncate().min_size(egui::vec2(w, 26.0)));
            if is_ais {
                let r = resp.rect;
                let h = 10.0;
                let logo = egui::Rect::from_min_size(egui::pos2(r.left() + 8.0, r.center().y - h / 2.0), egui::vec2(h * 269.8 / 230.1, h));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(logo));
                crate::widgets::ais_logo(&mut child, h, if selected { t.on_accent } else { t.accent }).on_hover_text("AIS.chat");
            }
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
                if let Some(url) = self
                    .state
                    .backend(&c.backend_id)
                    .filter(|b| b.config.kind == BackendKind::AisChat)
                    .map(|b| format!("{}{}", b.config.base_url.trim_end_matches('/'), aiscraft_core::ais_conversation_path(c.persona.as_ref(), &c.id)))
                    && ui.button("In AIS.chat öffnen").clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.dialog =
                        Some(Dialog::Confirm { text: format!("„{}“ löschen?", c.title), command: "chat.delete".into(), params: json!({"id": c.id}) });
                    ui.close();
                }
            });
        }
    }

    /// Assistants, characters and learning scenarios on AIS.chat: start a chat with one.
    /// One entry for the "Entdecken" page, and up to five favourites as quick links.
    fn personas_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let ais: Vec<_> =
            self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. })).cloned().collect();
        if ais.is_empty() {
            return;
        }
        self.prefetch_personas(false);
        let count: usize = ais.iter().filter_map(|b| b.personas.as_ref()).map(Vec::len).sum();
        let loaded = ais.iter().any(|b| b.personas.is_some());
        ui.add_space(4.0);
        let r = ui.add(
            egui::Button::selectable(self.explore.is_some(), RichText::new("👥  Assistenten & Figuren entdecken"))
                .right_text(small_muted(&t, if loaded { count.to_string() } else { String::new() }))
                .truncate()
                .min_size(egui::vec2(ui.available_width(), 28.0)),
        );
        if r.on_hover_text("Assistenten, Figuren und Lernszenarien aus AIS.chat durchsuchen").clicked() {
            if self.explore.is_some() {
                self.explore = None;
            } else {
                self.open_explore();
            }
        }
        let favorites: Vec<(String, aiscraft_core::PersonaInfo)> =
            ais.iter().flat_map(|b| b.personas.iter().flatten().filter(|p| p.favorite).map(|p| (b.config.id.clone(), p.clone()))).take(5).collect();
        for (backend, p) in favorites {
            let active = self.explore.is_none()
                && self.state.current.is_none()
                && self.state.selection.persona.as_ref().is_some_and(|x| x.kind == p.kind && x.id == p.id);
            let r =
                ui.add(egui::Button::selectable(active, RichText::new(format!("★  {}", p.name))).truncate().min_size(egui::vec2(ui.available_width(), 22.0)));
            let r = r.on_hover_text(format!("Neuer Chat: {}", p.kind.label()));
            if r.clicked() {
                self.start_persona(&backend, p.kind, &p.id, None);
            }
        }
        ui.add_space(4.0);
    }

    /// Conversations on the AIS.chat server that are not here yet.
    fn remote_section(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let ais: Vec<_> =
            self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. })).cloned().collect();
        for b in ais {
            ui.add_space(10.0);
            let header = egui::CollapsingHeader::new(small_muted(&t, format!("{} – VERLAUF", b.config.name.to_uppercase())))
                .id_salt(("remote", &b.config.id))
                .default_open(false)
                .show(ui, |ui| {
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
                        if ui
                            .add(egui::Button::new(RichText::new(&r.title).color(t.muted)).frame(false).truncate())
                            .on_hover_text("Hierher übernehmen und weiterschreiben")
                            .clicked()
                        {
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
            Some(BackendStatus::Ready { account }) => (
                t.success,
                account.display_name.clone().filter(|n| !n.contains('@')).unwrap_or_else(|| view.as_ref().map(|b| b.config.name.clone()).unwrap_or_default()),
                account.detail.clone().unwrap_or_default(),
            ),
            Some(BackendStatus::Checking) => (t.warning, "Verbinde …".into(), String::new()),
            Some(BackendStatus::SignedOut { .. }) | Some(BackendStatus::Unknown) => {
                (t.muted, view.as_ref().map(|b| b.config.name.clone()).unwrap_or_default(), "Nicht angemeldet".into())
            }
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
                    if ui.add(egui::Button::new(RichText::new("?").color(t.muted).size(15.0)).frame(false)).on_hover_text("Handbuch (F1)").clicked() {
                        self.manual = Some(Default::default());
                    }
                    if icon_button(ui, &t, "⚙", "Einstellungen (Strg+,)").clicked() {
                        self.dialog = Some(Dialog::settings());
                    }
                });
            })
            .response
            .interact(Sense::click());
        if let Some(q) = self
            .state
            .backends
            .iter()
            .filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. }))
            .find_map(|b| b.quota)
        {
            let left = q.left() as f32;
            ui.horizontal(|ui| {
                ui.label(small_muted(&t, format!("AIS.chat-Kontingent: {:.0} % übrig", left * 100.0)));
            });
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 5.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(3), t.surface_hi);
            let mut fill = rect;
            fill.set_width((rect.width() * left).max(2.0));
            ui.painter().rect_filled(
                fill,
                CornerRadius::same(3),
                if left < 0.1 {
                    t.danger
                } else if left < 0.3 {
                    t.warning
                } else {
                    t.accent
                },
            );
            resp.on_hover_text(format!("{:.0} von {:.0} Token-Punkten verbraucht (aus AIS.chat, nach jeder Antwort aktualisiert)", q.used, q.max));
        }
        if r.clicked() {
            if let Some(b) = view.filter(|b| !matches!(b.status, BackendStatus::Ready { .. })) {
                self.dialog = Some(Dialog::login(&b.config.id));
            } else {
                self.dialog = Some(Dialog::settings());
            }
        }
    }
}
