use aiscraft_core::BackendKind;
use aiscraft_engine::BackendStatus;
use egui::{Align, CornerRadius, Layout, RichText, ScrollArea, Sense, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::icons::Icon;
use crate::widgets::{age_group, muted, small_muted};

impl AisCraftApp {
    pub(crate) fn sidebar(&mut self, ui: &mut Ui) {
        // Inside the sidebar the sidebar's own colours apply (restored at the end).
        let outer = self.tokens.clone();
        self.tokens = outer.for_sidebar();
        let v = ui.visuals_mut();
        v.override_text_color = Some(outer.side_text2);
        v.widgets.inactive.weak_bg_fill = outer.opaque(outer.side_search);
        v.widgets.inactive.bg_fill = outer.opaque(outer.side_search);
        v.widgets.hovered.weak_bg_fill = outer.opaque(outer.side_active);
        v.widgets.hovered.bg_fill = outer.opaque(outer.side_active);
        v.widgets.noninteractive.fg_stroke.color = outer.side_text2;
        v.widgets.inactive.fg_stroke.color = outer.side_text2;
        v.widgets.hovered.fg_stroke.color = outer.side_text;
        v.hyperlink_color = outer.side_text;
        self.sidebar_top(ui, &outer);
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
                self.archive_links(ui);
                self.remote_section(ui);
            }
        });
        self.tokens = outer;
    }

    /// A section heading with an optional "+" (its tooltip); returns the plus button.
    fn heading(&self, ui: &mut Ui, label: &str, plus: Option<&str>) -> Option<egui::Response> {
        crate::kit::section(ui, &self.tokens, label, plus)
    }

    /// One entry of a list: its icon and label.
    fn entry(&self, ui: &mut Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
        crate::kit::side_item(ui, &self.tokens, icon, label, active)
    }

    /// Logo tile and name, the "Neuer Chat" pill, the search pill; the account
    /// at the bottom.
    fn sidebar_top(&mut self, ui: &mut Ui, t: &crate::theme::Tokens) {
        let t = t.clone();
        if let Some(g) = &t.glass {
            let full = ui.max_rect() + egui::Margin { left: 14, right: 14, top: 18, bottom: 14 };
            crate::backdrop::glass_edge(ui.painter(), full, f32::from(t.r_panel), g.edge);
        }
        if t.flush_modern() {
            let full = ui.max_rect().expand2(egui::vec2(14.0, 0.0)) + egui::Margin { left: 0, right: 0, top: 18, bottom: 14 };
            crate::backdrop::vertical_gradient(ui.painter(), full, CornerRadius::ZERO, t.side_top, t.side_bottom);
        }
        egui::Panel::bottom("sidebar-account")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                self.update_banner(ui);
                self.account_row(ui);
            });
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let tile = crate::kit::logo_tile(ui, 34.0);
            ui.add_space(2.0);
            let name = ui.add(egui::Label::new(crate::theme::bold("AIS.ChatCraft").size(19.0).color(t.side_text)).selectable(false));
            let home = ui
                .interact(tile.rect.union(name.rect), ui.scope_id().with("home"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Startseite");
            if home.clicked() && (self.state.current.is_some() || self.explore.is_some()) {
                self.close_chat();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if crate::kit::icon_button(ui, &t, Icon::Panel, "Seitenleiste ausblenden (Strg+B)", t.side_muted, 19.0, 32.0).clicked() {
                    self.sidebar_open = false;
                }
            });
        });
        ui.add_space(12.0);
        // "Neuer Chat": a full-width pill in the accent.
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Neuer Chat"));
        {
            let p = ui.painter();
            p.add(
                egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: egui::Color32::from_black_alpha(38) }.as_shape(rect, CornerRadius::same(20)),
            );
            if t.glass.is_some() {
                crate::kit::accent_fill(p, rect, CornerRadius::same(20), &t);
            } else {
                p.rect_filled(rect, CornerRadius::same(20), t.side_new);
            }
            if r.hovered() {
                p.rect_filled(rect, CornerRadius::same(20), egui::Color32::from_white_alpha(24));
            }
            let g = crate::kit::line(ui, "Neuer Chat", crate::kit::font(15.0, crate::theme::SEMIBOLD), t.side_new_text, 200.0);
            let w = 18.0 + 8.0 + g.size().x;
            let x = rect.center().x - w / 2.0;
            Icon::Plus.paint(p, egui::pos2(x + 9.0, rect.center().y), 18.0, t.side_new_text);
            p.galley(egui::pos2(x + 26.0, rect.center().y - g.size().y / 2.0), g, t.side_new_text);
        }
        if r.on_hover_text("Strg+N").clicked() {
            self.new_chat();
        }
        ui.add_space(8.0);
        // Search: a pill with the icon, the field and the key for the command palette.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::hover());
        ui.painter().rect(
            rect,
            CornerRadius::same(18),
            t.side_search,
            egui::Stroke::new(1.0, if t.glass.is_some() { t.side_line } else { egui::Color32::TRANSPARENT }),
            egui::StrokeKind::Inside,
        );
        Icon::Search.paint(ui.painter(), egui::pos2(rect.left() + 22.0, rect.center().y), 17.0, t.side_search_text);
        let key_bg = if t.glass.is_none() {
            t.side_line
        } else if t.dark {
            egui::Color32::from_white_alpha(20)
        } else {
            egui::Color32::from_white_alpha(180)
        };
        let key = crate::kit::kbd(ui, ui.painter(), egui::pos2(rect.right() - 8.0, rect.center().y), "Strg K", t.side_search_text, key_bg);
        let key_r = ui.interact(key, ui.scope_id().with("palette-key"), Sense::click()).on_hover_text("Befehle (Strg+K)");
        key_r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Befehle (Strg+K)"));
        if key_r.clicked() {
            self.palette = Some(Default::default());
        }
        let field = egui::Rect::from_min_max(egui::pos2(rect.left() + 36.0, rect.center().y - 9.0), egui::pos2(key.left() - 6.0, rect.center().y + 9.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(Layout::left_to_right(Align::Center)));
        let search = child.add(
            egui::TextEdit::singleline(&mut self.search_text)
                .hint_text(RichText::new("Chats durchsuchen").color(t.side_search_text))
                .text_color(t.side_text)
                .frame(egui::Frame::NONE)
                .font(egui::FontId::proportional(14.5))
                .desired_width(field.width()),
        );
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
        ui.add_space(10.0);
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
            // An archived chat is found too, with a note (#77).
            if h.archived {
                ui.label(small_muted(&t, "archiviert"));
            }
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
        if self.heading(ui, "Projekte", Some("Neues Projekt")).is_some_and(|r| r.clicked()) {
            self.dialog = Some(Dialog::project(None));
        }
        let projects = self.state.projects.clone();
        if projects.is_empty() {
            empty_hint(ui, &t, "Noch keine Projekte. Ein Projekt bündelt Chats mit eigenen Anweisungen.");
        }
        for p in projects {
            let active = self.project_filter.as_deref() == Some(&p.id);
            let r = self.entry(ui, Icon::Folder, &p.name, active).on_hover_text(format!("Neuer Chat im Projekt {}", p.name));
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
                // The project as a whole into the archive, and back (#77).
                let (live, archived) = self
                    .state
                    .conversations
                    .iter()
                    .filter(|c| c.project_id.as_deref() == Some(&p.id) && !c.temporary)
                    .fold((0, 0), |(l, a), c| if c.archived { (l, a + 1) } else { (l + 1, a) });
                if live > 0 && ui.button("Chats archivieren").clicked() {
                    self.run("chat.archive", json!({"project": p.id}));
                    ui.close();
                }
                if archived > 0 && ui.button(format!("Archivierte Chats zurückholen ({archived})")).clicked() {
                    self.run("chat.unarchive", json!({"project": p.id}));
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
        if let Some(plus) = self.heading(ui, "Dokumente & Whiteboards", Some("Neues Dokument oder Whiteboard")) {
            egui::Popup::menu(&plus).show(|ui| {
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
        }
        let items: Vec<_> =
            self.state.artifacts.iter().filter(|a| self.project_filter.as_ref().is_none_or(|p| a.project_id.as_ref() == Some(p))).take(12).cloned().collect();
        if items.is_empty() {
            empty_hint(ui, &t, "Noch keine Dokumente. Mit „Als Dokument“ unter einer Antwort legst du eins an.");
        }
        for a in items {
            let open = self.state.open_artifact.as_deref() == Some(&a.id);
            let r = self.entry(ui, crate::learn_panel::kind_lucide(a.kind), &a.title, open).on_hover_text(&a.title);
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
        // A temporary chat (#34) is not listed: it is gone once closed; an archived one (#77)
        // lies in the archive.
        let convs: Vec<_> =
            self.state.listed_conversations().filter(|c| self.project_filter.as_ref().is_none_or(|p| c.project_id.as_ref() == Some(p))).cloned().collect();
        if convs.is_empty() {
            ui.add_space(8.0);
            empty_hint(ui, &t, if self.project_filter.is_some() { "Noch keine Chats in diesem Projekt." } else { "Noch keine Chats." });
            return;
        }
        self.selection_bar(ui);
        let order: Vec<String> = convs.iter().map(|c| c.id.clone()).collect();
        let mut last_group = "";
        for c in convs {
            let group = if c.pinned { "Angeheftet" } else { age_group(c.updated_at) };
            if group != last_group {
                ui.add_space(8.0);
                self.heading(ui, group, None);
                last_group = group;
            }
            let picked = self.selected_chats.contains(&c.id);
            let selected = if self.selected_chats.is_empty() { self.state.current.as_deref() == Some(&c.id) } else { picked };
            let generating = self.state.is_generating(&c.id);
            // „Namen schützen“: the title keeps placeholders, shown with the names (#55).
            let shown = self.engine.reveal(&c.id, &c.title);
            let resp = crate::kit::side_item(ui, &t, Icon::Chat, &shown, selected).on_hover_text(&shown);
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
                // Strg or Umschalt picks chats for the actions above the list (#77); a plain
                // click opens the chat and ends the selection.
                if !self.select_click(&c.id, click_modifiers(ui), &order) {
                    self.selected_chats.clear();
                    self.select_anchor = Some(c.id.clone());
                    self.run("chat.open", json!({"id": c.id}));
                }
            }
            // On a picked chat the menu acts on all picked ones.
            let many = picked && self.selected_chats.len() > 1;
            if many {
                let ids = self.selected_chats.clone();
                resp.context_menu(|ui| {
                    ui.label(small_muted(&t, format!("{} Chats ausgewählt", ids.len())));
                    if ui.button("Archivieren").clicked() {
                        self.run("chat.archive", json!({"ids": ids}));
                        self.selected_chats.clear();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                        self.ask_delete(ids.clone());
                        ui.close();
                    }
                });
                continue;
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
                if ui.button("Archivieren").on_hover_text("Aus der Seitenleiste ins Archiv; bleibt ganz erhalten").clicked() {
                    self.run("chat.archive", json!({"id": c.id}));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.ask_delete(vec![c.id.clone()]);
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
        let r = crate::kit::side_item(
            ui,
            &t,
            Icon::Explore,
            &if loaded { format!("Entdecken ({count})") } else { "Entdecken".to_string() },
            self.explore.is_some(),
        );
        if r.on_hover_text("Assistenten, Dialogpartner und Lernszenarien aus AIS.chat").clicked() {
            if self.explore.is_some() {
                self.explore = None;
            } else {
                self.open_explore();
            }
        }
        // Image generation, where AIS.chat offers it (it hides it, too, when the state switched
        // it off).
        if let Some(b) = ais.iter().find(|b| !b.image_models.is_empty()) {
            let active = self.explore.is_none() && self.state.current.is_none() && self.state.selection.kind == aiscraft_core::ConversationKind::Image;
            let r = self.entry(ui, Icon::Image, "Bild erzeugen", active);
            if r.on_hover_text("Neuer Bild-Chat: beschreibe, was das Bild zeigen soll").clicked() {
                self.explore = None;
                self.run("chat.new_image", json!({"backend": b.config.id}));
                self.focus_composer = true;
            }
        }
        let favorites: Vec<(String, aiscraft_core::PersonaInfo)> =
            ais.iter().flat_map(|b| b.personas.iter().flatten().filter(|p| p.favorite).map(|p| (b.config.id.clone(), p.clone()))).take(5).collect();
        if favorites.is_empty() && loaded && count > 0 {
            empty_hint(ui, &t, "Mit dem Stern ☆ in „Entdecken“ legst du Favoriten an.");
        }
        for (backend, p) in favorites {
            let active = self.explore.is_none()
                && self.state.current.is_none()
                && self.state.selection.persona.as_ref().is_some_and(|x| x.kind == p.kind && x.id == p.id);
            let r = self.entry(ui, Icon::Star, &p.name, active);
            let r = r.on_hover_text(format!("Neuer Chat: {} {}", p.kind.label(), p.name));
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
            let header = egui::CollapsingHeader::new(RichText::new(format!("Verlauf in {}", b.config.name)).size(12.5).color(t.muted).strong())
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

    /// The selected service's status: dot colour, first and second line, and the service.
    fn account_status(&self) -> (egui::Color32, String, String, Option<aiscraft_engine::BackendView>) {
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
            None => (t.muted, "Kein Dienst".into(), String::new()),
        };
        (color, line1, line2, view)
    }
}

impl AisCraftApp {
    /// AIS.chat's token points left, as a bar under the account.
    fn quota_bar(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
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
    }

    /// Avatar with initials, name, status, and the settings and handbook buttons.
    fn account_row(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let (color, line1, line2, view) = self.account_status();
        ui.add_space(4.0);
        let top = ui.cursor().top();
        ui.painter().line_segment([egui::pos2(ui.max_rect().left(), top), egui::pos2(ui.max_rect().right(), top)], egui::Stroke::new(1.0, t.side_line));
        ui.add_space(10.0);
        let r = ui
            .horizontal(|ui| {
                ui.add_space(6.0);
                let (av, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::hover());
                crate::kit::avatar(ui.painter(), av.center(), 36.0, &crate::kit::initials(&line1));
                ui.add_space(2.0);
                // The settings button first, from the right; name and status truncate into the rest.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if crate::kit::icon_button(ui, &t, Icon::Sliders, "Einstellungen (Strg+,)", t.text, 19.0, 34.0).clicked() {
                        self.dialog = Some(Dialog::settings());
                    }
                    ui.add_space(8.0);
                    let rest = egui::vec2(ui.available_width().max(40.0), 36.0);
                    ui.allocate_ui_with_layout(rest, Layout::top_down(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.set_width(rest.x);
                        ui.add(egui::Label::new(crate::theme::semibold(&line1).size(14.5).color(t.text)).truncate().selectable(false));
                        if !line2.is_empty() {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;
                                let (d, _) = ui.allocate_exact_size(egui::vec2(7.0, 14.0), Sense::hover());
                                ui.painter().circle_filled(d.center(), 3.5, color);
                                ui.add(egui::Label::new(RichText::new(&line2).size(12.5).color(t.muted)).truncate().selectable(false));
                            });
                        }
                    });
                });
            })
            .response
            .interact(Sense::click());
        self.quota_bar(ui);
        ui.add_space(2.0);
        if r.clicked() {
            if let Some(b) = view.filter(|b| !matches!(b.status, BackendStatus::Ready { .. })) {
                self.dialog = Some(Dialog::login(&b.config.id));
            } else {
                self.dialog = Some(Dialog::settings());
            }
        }
    }
}

/// The modifiers of the click just made: from the pointer event (what the platform reported with
/// it), else the keyboard state.
fn click_modifiers(ui: &Ui) -> egui::Modifiers {
    ui.input(|i| {
        let ev = i
            .events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let k = i.modifiers;
        egui::Modifiers {
            alt: ev.alt || k.alt,
            ctrl: ev.ctrl || k.ctrl,
            shift: ev.shift || k.shift,
            mac_cmd: ev.mac_cmd || k.mac_cmd,
            command: ev.command || k.command,
        }
    })
}

/// What an empty section says, with a little room above and the entries' indent.
fn empty_hint(ui: &mut Ui, t: &crate::theme::Tokens, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().button_padding.x);
        ui.add(egui::Label::new(small_muted(t, text)).wrap());
    });
    ui.add_space(4.0);
}
