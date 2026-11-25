use aiscraft_core::{BackendKind, Message, Role};
use aiscraft_engine::BackendStatus;
use egui::{Align, CornerRadius, Frame, Key, KeyboardShortcut, Layout, Margin, Modifiers, RichText, ScrollArea, Stroke, Ui};
use serde_json::json;

use crate::app::{AisCraftApp, CONTENT_WIDTH};
use crate::dialogs::Dialog;
use crate::widgets::{accent_button, card, icon_button, muted, small_muted};

impl AisCraftApp {
    pub(crate) fn conversation(&mut self, ui: &mut Ui) {
        self.header(ui);
        let empty = self.state.current.is_none() && self.state.messages.is_empty();
        if empty {
            self.empty_state(ui);
            return;
        }
        if self.find.is_some() && self.state.current.is_some() {
            egui::Panel::top("find-bar").show_separator_line(false).frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 6 })).show(
                ui,
                |ui| {
                    centered(ui, |ui| self.find_bar(ui));
                },
            );
        } else if self.find.is_some() {
            self.find = None;
        }
        egui::Panel::bottom("composer-panel")
            .show_separator_line(false)
            .frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 10 }))
            .show(ui, |ui| {
                let w = crate::app::CONTENT_WIDTH.max(self.composer_need);
                centered_w(ui, w, |ui| self.composer(ui));
            });
        let main = if self.tokens.glass.is_some() { self.tokens.main.gamma_multiply(0.6) } else { self.tokens.main };
        let out = ScrollArea::vertical().id_salt(("messages", self.state.current.clone())).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            centered(ui, |ui| {
                ui.add_space(12.0);
                // Placeholders of „Namen schützen“ shown, copied and edited as the names (#55).
                let msgs = self.engine.shown_messages(&self.state);
                let last_assistant = msgs.iter().rposition(|m| m.role == Role::Assistant);
                // The message with the current hit of Strg+F (#41): marked, and scrolled to once per step.
                let hit =
                    self.find.as_ref().and(self.state.find.as_ref()).and_then(|f| f.current_hit().map(|h| (f.query.clone(), f.current, h.message_id.clone())));
                for (i, m) in msgs.iter().enumerate() {
                    let top = ui.cursor().min.y;
                    match m.role {
                        Role::User => self.user_message(ui, m),
                        // Tool results are shown as chips on the call, not as messages; a result
                        // with a connector's view gets its card (#74).
                        Role::Tool if m.tool_result.as_ref().is_some_and(|r| r.view.is_some()) => self.view_card(ui, m),
                        // What an open view put into the chat: its own entry, not the person's words.
                        Role::Tool if m.tool_result.as_ref().is_some_and(|r| r.from_view.is_some()) => self.view_entry(ui, m),
                        Role::Tool => continue,
                        _ => self.assistant_message(ui, m, Some(i) == last_assistant),
                    }
                    if let Some((q, n, id)) = &hit
                        && *id == m.id
                    {
                        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().min.y).expand2(egui::vec2(6.0, 4.0));
                        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(2.0, self.tokens.accent), egui::StrokeKind::Outside);
                        let key = (q.clone(), *n);
                        if self.find_scrolled.as_ref() != Some(&key) {
                            ui.scroll_to_rect(rect, Some(Align::Center));
                            self.find_scrolled = Some(key);
                        }
                    }
                    ui.add_space(18.0);
                }
                self.approval_cards(ui);
                self.browser_takeover_panel(ui);
                ui.add_space(20.0);
            });
        });
        // Lines scrolled to the top and bottom edge fade out instead of being cut, so nothing
        // sharp sits right under the header or above the composer.
        let r = out.inner_rect;
        let clear = egui::Color32::TRANSPARENT;
        let top = egui::Rect::from_min_size(r.min, egui::vec2(r.width() - 12.0, 26.0));
        crate::backdrop::vertical_gradient(ui.painter(), top, egui::CornerRadius::ZERO, main, clear);
        let bottom = egui::Rect::from_min_max(egui::pos2(r.left(), r.bottom() - 26.0), egui::pos2(r.right() - 12.0, r.bottom()));
        crate::backdrop::vertical_gradient(ui.painter(), bottom, egui::CornerRadius::ZERO, clear, main);
    }

    /// The link to this chat in AIS.chat, for chats there.
    fn ais_link(&self) -> Option<String> {
        let id = self.state.current.as_ref()?;
        let c = self.state.conversation(id)?;
        let b = self.state.backend(&c.backend_id).filter(|b| b.config.kind == BackendKind::AisChat)?;
        let path = if c.kind == aiscraft_core::ConversationKind::Image {
            format!("/image-generation/d/{}", c.remote_id.as_deref().unwrap_or(&c.id))
        } else {
            aiscraft_core::ais_conversation_path(c.persona.as_ref(), &c.id)
        };
        Some(format!("{}{path}", b.config.base_url.trim_end_matches('/')))
    }

    /// Strg+F (#41): the search field, „2 von 5“, previous and next hit, close.
    fn find_bar(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let found = self.state.find.clone().unwrap_or_default();
        let mut step = None;
        let mut close = false;
        let mut changed = None;
        Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let Some(query) = self.find.as_mut() else { return };
                let (r, ()) = crate::widgets::field_then_buttons(
                    ui,
                    |ui| {
                        if ui.small_button("Schließen").clicked() {
                            close = true;
                        }
                        let any = !found.hits.is_empty();
                        if ui.add_enabled(any, egui::Button::new("›").small()).on_hover_text("Nächster Treffer (Enter)").clicked() {
                            step = Some("next");
                        }
                        if ui.add_enabled(any, egui::Button::new("‹").small()).on_hover_text("Voriger Treffer (Umschalt+Enter)").clicked() {
                            step = Some("prev");
                        }
                        let count = if found.query.trim().is_empty() {
                            String::new()
                        } else if found.hits.is_empty() {
                            "keine Treffer".to_string()
                        } else {
                            format!("{} von {}", found.current + 1, found.hits.len())
                        };
                        ui.label(small_muted(&t, count));
                    },
                    |ui, w| ui.add(egui::TextEdit::singleline(query).id(egui::Id::unique("find-field")).hint_text("Im Chat suchen").desired_width(w)),
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Im Chat suchen"));
                if r.changed() {
                    changed = Some(query.clone());
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    step = Some(if ui.input(|i| i.modifiers.shift) { "prev" } else { "next" });
                    r.request_focus();
                }
                if self.focus_find {
                    r.request_focus();
                    self.focus_find = false;
                }
            });
        });
        if close {
            self.close_find();
        } else if let Some(q) = changed {
            self.run("chat.find", json!({"query": q}));
        } else if let Some(s) = step {
            let q = self.find.clone().unwrap_or_default();
            self.run("chat.find", json!({"query": q, "step": s}));
        }
    }

    /// The title in Bold with project chip and model · time underneath; on the
    /// right the context pill, export, notifications and a menu with the rarer actions.
    fn header(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let conv = self.state.current.as_ref().and_then(|id| self.state.conversation(id)).cloned();
        Frame::new().inner_margin(Margin { left: 28, right: 22, top: 16, bottom: 10 }).show(ui, |ui| {
            let h = if conv.is_some() { 44.0 } else { 34.0 };
            let row = egui::vec2(ui.available_width(), h);
            ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                ui.set_max_width(row.x);
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(c) = &conv {
                    let more = crate::kit::icon_button(ui, &t, crate::icons::Icon::More, "Weitere Aktionen", t.text2, 19.0, 36.0);
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(220.0);
                        if ui.button("Umbenennen …").clicked() {
                            self.dialog = Some(Dialog::Rename { id: c.id.clone(), title: c.title.clone() });
                            ui.close();
                        }
                        if let Some(url) = self.ais_link()
                            && ui.button("In AIS.chat öffnen").clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            ui.close();
                        }
                        if ui.button("Chat schließen, zur Startseite (Esc)").clicked() {
                            self.close_chat();
                            ui.close();
                        }
                    });
                }
                self.inbox_button(ui);
                if conv.is_some() {
                    if crate::kit::icon_button(ui, &t, crate::icons::Icon::Share, "Chat als Datei speichern (Strg+Umschalt+E)", t.text2, 19.0, 36.0).clicked() {
                        self.export_dialog();
                    }
                    // Next to an open artifact the header is narrow: the title gets the room.
                    if self.state.selection.kind == aiscraft_core::ConversationKind::Chat && self.state.open_artifact.is_none() {
                        ui.add_space(6.0);
                        self.context_meter(ui);
                    }
                } else {
                    // A temporary chat (#34): switched on before the first message.
                    // Not with AIS.chat: it keeps every chat in its own history.
                    let temp = self.state.selection.temporary;
                    let (kind, label) = if temp { (crate::kit::Pill::Accent2, "Temporär: an") } else { (crate::kit::Pill::Ghost, "Temporärer Chat") };
                    if (temp || self.state.temporary_possible(&self.state.selection.backend_id))
                        && crate::kit::pill(ui, &t, kind, None, label, 12.5, None)
                            .on_hover_text("Wird nicht gespeichert, nicht durchsucht und legt keine Notizen an; beim Schließen ist er weg.")
                            .clicked()
                    {
                        self.run("chat.temporary", json!({"on": !temp}));
                    }
                    let unread = self.state.inbox.iter().filter(|i| !i.read).count();
                    if unread > 0 {
                        let label = if unread == 1 { "1 neue Benachrichtigung".to_string() } else { format!("{unread} neue Benachrichtigungen") };
                        if crate::kit::pill(ui, &t, crate::kit::Pill::Accent2, Some(crate::icons::Icon::Sparkles), &label, 12.5, None).clicked() {
                            self.run("notice.read", json!({}));
                        }
                    }
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    if !self.sidebar_open && crate::kit::icon_button(ui, &t, crate::icons::Icon::Panel, "Seitenleiste (Strg+B)", t.text2, 19.0, 36.0).clicked()
                    {
                        self.sidebar_open = true;
                    }
                    let Some(c) = &conv else { return };
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 3.0;
                        let r = ui
                            .add(
                                egui::Label::new(crate::theme::bold(self.engine.reveal(&c.id, &c.title)).size(20.0).color(t.text))
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Doppelklick zum Umbenennen");
                        if r.double_clicked() {
                            self.dialog = Some(Dialog::Rename { id: c.id.clone(), title: c.title.clone() });
                        }
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if let Some(p) = self.state.selection.project_id.as_ref().and_then(|pid| self.state.projects.iter().find(|p| &p.id == pid)) {
                                project_chip(ui, &t, &p.name);
                            }
                            let mut meta = Vec::new();
                            if let Some(p) = self.state.selection.persona.as_ref() {
                                meta.push(format!("{}: {}", p.kind.label(), p.name));
                            } else if !self.state.selection.model_id.is_empty() {
                                meta.push(self.state.model_name(&self.state.selection.backend_id, &self.state.selection.model_id));
                            }
                            if c.temporary {
                                meta.push("Temporär, wird nicht gespeichert".to_string());
                            } else {
                                meta.push(aiscraft_engine::friendly_time(c.updated_at, aiscraft_engine::now_ms()));
                            }
                            // An archived chat opened from the archive or the search (#77).
                            if c.archived {
                                meta.push("Archiviert".to_string());
                            }
                            ui.add(egui::Label::new(RichText::new(meta.join(" · ")).size(13.0).color(t.muted)).truncate());
                        });
                    });
                });
            });
        });
    }

    /// Backend and model picker: one menu, grouped by backend.
    pub(crate) fn model_picker(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let sel = self.state.selection.clone();
        let label = match self.state.backend(&sel.backend_id) {
            Some(b) if sel.model_id.is_empty() => format!("{} · kein Modell", b.config.name),
            Some(b) => format!("{} · {}", b.config.name, self.state.model_name(&sel.backend_id, &sel.model_id)),
            None => "Dienst wählen".into(),
        };
        let body = |app: &mut Self, ui: &mut Ui| {
            ui.set_min_width(260.0);
            for b in app.state.backends.clone() {
                ui.label(small_muted(&t, b.config.name.to_uppercase()));
                match &b.status {
                    BackendStatus::Ready { .. } if b.models.is_empty() => {
                        ui.label(small_muted(&t, "keine Modelle"));
                    }
                    BackendStatus::Ready { .. } => {
                        for m in &b.models {
                            let active = sel.backend_id == b.config.id && sel.model_id == m.id && sel.kind == aiscraft_core::ConversationKind::Chat;
                            let r = ui.add(egui::Button::selectable(active, &m.name));
                            let r = match &m.description {
                                Some(d) => r.on_hover_text(d),
                                None => r,
                            };
                            if r.clicked() {
                                app.run("chat.select_model", json!({"backend": b.config.id, "model": m.id}));
                                ui.close();
                            }
                        }
                        if !b.image_models.is_empty() {
                            ui.label(small_muted(&t, "Bild erzeugen"));
                            for m in &b.image_models {
                                let active = sel.backend_id == b.config.id && sel.model_id == m.id && sel.kind == aiscraft_core::ConversationKind::Image;
                                let r = ui.add(egui::Button::selectable(active, m.name.as_str())).on_hover_text("Neuer Bild-Chat mit diesem Modell");
                                if r.clicked() {
                                    app.run("chat.new_image", json!({"backend": b.config.id, "model": m.id}));
                                    ui.close();
                                }
                            }
                        }
                    }
                    BackendStatus::Checking => {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(12.0));
                            ui.label(small_muted(&t, "verbinde …"));
                        });
                    }
                    _ => {
                        if ui.button("Anmelden …").clicked() {
                            app.dialog = Some(Dialog::login(&b.config.id));
                            ui.close();
                        }
                    }
                }
                ui.add_space(4.0);
            }
            ui.separator();
            if ui.button("+ Dienst hinzufügen …").clicked() {
                app.dialog = Some(Dialog::add_backend());
                ui.close();
            }
        };
        let model = if sel.model_id.is_empty() { label.clone() } else { self.state.model_name(&sel.backend_id, &sel.model_id) };
        let r = model_chip(ui, &t, &model).on_hover_text(label);
        egui::Popup::menu(&r).show(|ui| body(self, ui));
    }

    /// A new chat with an assistant, character or learning scenario: who it is, the task, the
    /// greeting and its suggestions.
    fn persona_empty_state(&mut self, ui: &mut Ui, p: aiscraft_core::Persona) {
        let t = self.tokens.clone();
        let info = self.state.persona_info(&self.state.selection.backend_id, &p).cloned();
        ui.add_space((ui.available_height() * 0.16).max(20.0));
        centered(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(small_muted(&t, p.kind.label().to_uppercase()));
                ui.label(RichText::new(&p.name).size(26.0));
                if let Some(d) = info.as_ref().and_then(|i| i.description.clone()) {
                    ui.add(egui::Label::new(muted(&t, d)).wrap());
                }
                if let Some(i) = &info {
                    let (label, hint) =
                        if i.favorite { ("★ Favorit", "Favorit entfernen") } else { ("☆ Als Favorit markieren", "Erscheint dann in der Seitenleiste") };
                    if ui.add(egui::Button::new(small_muted(&t, label)).frame(false)).on_hover_text(hint).clicked() {
                        self.toggle_favorite(&self.state.selection.backend_id, i.kind, &i.id, !i.favorite);
                    }
                }
            });
            ui.add_space(12.0);
            if let Some(ex) = info.as_ref().and_then(|i| i.exercise.clone()) {
                card(&t).show(ui, |ui| {
                    ui.label(RichText::new("Aufgabe").strong());
                    ui.add(egui::Label::new(ex).wrap());
                });
                ui.add_space(8.0);
            }
            if let Some(g) = info.as_ref().and_then(|i| i.initial_message.clone()) {
                Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.add(egui::Label::new(g).wrap());
                });
                ui.add_space(8.0);
            }
            self.composer(ui);
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                for s in info.as_ref().map(|i| i.prompt_suggestions.clone()).unwrap_or_default() {
                    if ui.add(suggestion_chip(&t, &s)).clicked() {
                        self.composer = s;
                        self.focus_composer = true;
                    }
                }
                if ui.add(egui::Button::new(small_muted(&t, format!("Ohne {} chatten", p.kind.label()))).frame(false)).clicked() {
                    self.new_chat();
                }
            });
        });
    }

    fn empty_state(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        if let Some(p) = self.state.selection.persona.clone() {
            self.persona_empty_state(ui, p);
            return;
        }
        // A name from a sign-in (AIS.chat via VIDIS), else the person logged in to this computer.
        let sso = self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat).find_map(|b| match &b.status {
            BackendStatus::Ready { account } => account.display_name.clone().filter(|n| !n.contains('@')),
            _ => None,
        });
        let name = sso.unwrap_or_else(aiscraft_engine::local_user_name);
        let first = name.split_whitespace().next().filter(|n| looks_like_a_name(n)).map(|n| {
            let mut c = n.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        });
        let any_ready = self.state.backends.iter().any(|b| matches!(b.status, BackendStatus::Ready { .. }));
        // The start page: below the composer come the cards (crate::home), so the whole page scrolls.
        let home = self.state.selection.kind == aiscraft_core::ConversationKind::Chat;
        if home {
            ScrollArea::vertical().id_salt("start-page").auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(18.0);
                let w = 820.0_f32.max(self.composer_need);
                centered_w(ui, w, |ui| self.home_page(ui, first));
            });
            return;
        }
        ScrollArea::vertical().id_salt("start-page").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.22).max(20.0));
            centered(ui, |ui| {
                ui.vertical_centered(|ui| {
                    crate::widgets::ais_logo(ui, 46.0, t.accent);
                    ui.add_space(6.0);
                    let image_mode = self.state.selection.kind == aiscraft_core::ConversationKind::Image;
                    let greeting = match first {
                        _ if image_mode => "Was soll das Bild zeigen?".to_string(),
                        Some(n) => format!("Hallo, {n}. Woran arbeiten wir?"),
                        None => "Woran arbeiten wir?".into(),
                    };
                    ui.label(RichText::new(greeting).size(26.0));
                    if let Some(p) = self.state.selection.project_id.as_ref().and_then(|pid| self.state.projects.iter().find(|p| &p.id == pid)) {
                        ui.label(muted(&t, format!("Neuer Chat im Projekt {}", p.name)));
                    }
                });
                ui.add_space(18.0);
                if !any_ready {
                    card(&t).show(ui, |ui| {
                        ui.label(RichText::new("Noch nicht verbunden").strong());
                        ui.label(muted(&t, "Melde dich bei AIS.chat an oder füge einen anderen Dienst hinzu (OpenAI, Claude, Ollama, LM Studio …)."));
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if let Some(ais) = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat)
                                && accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
                            {
                                self.dialog = Some(Dialog::login(&ais.config.id));
                            }
                            if ui.button("Dienst hinzufügen").clicked() {
                                self.dialog = Some(Dialog::add_backend());
                            }
                        });
                    });
                    ui.add_space(12.0);
                }
                self.composer(ui);
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    let ideas: &[&str] = if self.state.selection.kind == aiscraft_core::ConversationKind::Image {
                        &["Ein Fuchs im verschneiten Wald, Aquarell", "Schaubild einer Pflanzenzelle für Klasse 7", "Comic: drei Kinder bauen ein Hochbeet"]
                    } else {
                        &[
                            "Stundenentwurf für Klasse 7 zum Thema Photosynthese",
                            "Erkläre Bruchrechnung mit Pizzastücken",
                            "Formuliere eine freundliche Elternmail",
                            "Erstelle ein Quiz mit 5 Fragen",
                        ]
                    };
                    for s in ideas.iter().copied() {
                        if ui.add(suggestion_chip(&t, s)).clicked() {
                            self.composer = s.to_string();
                            self.focus_composer = true;
                        }
                    }
                });
            });
        });
    }

    fn user_message(&mut self, ui: &mut Ui, m: &Message) {
        let t = self.tokens.clone();
        if let Some((id, text)) = self.editing.as_mut()
            && *id == m.id
        {
            let mut send = None;
            let mut cancel = false;
            card(&t).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(text).desired_width(f32::INFINITY).desired_rows(3));
                ui.horizontal(|ui| {
                    if accent_button(ui, &t, "Senden").clicked() {
                        send = Some((id.clone(), text.clone()));
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                    ui.label(small_muted(&t, "Die bisherige Fassung bleibt erhalten; mit ‹ › wechselst du zwischen den Fassungen."));
                });
            });
            if let Some((mid, text)) = send {
                self.run("chat.edit", json!({"message_id": mid, "text": text}));
                self.editing = None;
            } else if cancel {
                self.editing = None;
            }
            return;
        }
        ui.with_layout(Layout::top_down(Align::Max), |ui| {
            let max_w = (ui.available_width() * 0.8).min(470.0 - 36.0);
            let (radius, margin) = (CornerRadius { nw: t.r_bubble, ne: 6, sw: t.r_bubble, se: t.r_bubble }, Margin::symmetric(18, 12));
            let r = Frame::new().fill(t.user_bubble).corner_radius(radius).inner_margin(margin).show(ui, |ui| {
                ui.visuals_mut().override_text_color = Some(t.user_text);
                ui.set_max_width(max_w);
                if !m.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for a in &m.attachments {
                            if a.is_link() {
                                ui.hyperlink_to(RichText::new(format!("🔗 {}", short_url(&a.name))).small(), &a.name);
                            } else {
                                ui.label(small_muted(&t, format!("📎 {}", a.name)));
                            }
                        }
                    });
                }
                // A format request (Erstellen → Arbeitsblatt, Lern-App, …): only the teacher's
                // words; the format spec that went along is folded away (#56).
                if let Some(r) = aiscraft_engine::learn::formats::split_request(&m.content) {
                    ui.add(egui::Label::new(r.topic).wrap().selectable(true));
                    ui.add_space(2.0);
                    egui::CollapsingHeader::new(small_muted(&t, format!("Formatvorgabe: {}", r.format.label)))
                        .id_salt(("format-spec", &m.id))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(r.spec).small()).wrap().selectable(true));
                        })
                        .header_response
                        .on_hover_text(
                            "Diese Beschreibung des Formats hat AIS.ChatCraft mit deiner Nachricht geschickt, damit das Modell weiß, wie es antworten soll.",
                        );
                } else if !m.content.is_empty() {
                    ui.add(egui::Label::new(&m.content).wrap().selectable(true));
                }
            });
            // What AIS.ChatCraft put in front of the message for a backend without a system prompt.
            if let Some(along) = m.activity.iter().find(|s| s.label == aiscraft_engine::SENT_ALONG).and_then(|s| s.detail.as_deref()) {
                ui.add(egui::Label::new(small_muted(&t, format!("Mitgeschickt: {along}"))).wrap())
                    .on_hover_text("Stand oben in deiner Nachricht; der Server behält es im Verlauf, darum geht es in diesem Chat nur einmal mit.");
            }
            if let Some(done) = m.activity.iter().find(|s| s.label == aiscraft_engine::privacy::PROTECTED).and_then(|s| s.detail.as_deref()) {
                let sent = self.state.messages.iter().find(|x| x.id == m.id).map(|x| x.content.clone()).unwrap_or_default();
                ui.add(egui::Label::new(small_muted(&t, format!("Namen geschützt: {done}"))).wrap())
                    .on_hover_text(format!("So ging die Nachricht raus:\n{}", sent.chars().take(1500).collect::<String>()));
            }
            if let Some(left) = m.activity.iter().find(|s| s.label == aiscraft_engine::LEFT_OUT).and_then(|s| s.detail.as_deref()) {
                ui.add(egui::Label::new(small_muted(&t, format!("Nicht mitgeschickt (anderer Dienst): {left}"))).wrap()).on_hover_text(
                    "Passt, stammt aber aus einem Chat mit einem anderen Dienst. Unter „Quellen“ kannst du erlauben, dass solche Ausschnitte mitgehen.",
                );
            }
            let hovered = ui.rect_contains_pointer(r.response.rect.expand(24.0));
            ui.horizontal_wrapped(|ui| {
                ui.set_min_height(18.0);
                self.variant_switcher(ui, &m.id);
                if hovered && !self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c)) {
                    if icon_button(ui, &t, "✏", "Bearbeiten").clicked() {
                        self.editing = Some((m.id.clone(), m.content.clone()));
                    }
                    if message_action(ui, &t, "🗐", "Kopieren").clicked() {
                        ui.ctx().copy_text(m.content.clone());
                    }
                }
            });
        });
    }

    fn assistant_message(&mut self, ui: &mut Ui, m: &Message, is_last: bool) {
        // A task's result is a compact card (#69).
        if let Some(card) = aiscraft_engine::task_card(&self.state, m) {
            self.task_card(ui, &card);
            return;
        }
        let t = self.tokens.clone();
        let generating = is_last && self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        // Token counts are for the curious: on the model name, not in the row of actions.
        let tokens = m
            .usage
            .filter(|u| u.output_tokens > 0)
            .map(|u| format!("Tokens: {} Eingabe, {} Antwort", crate::home::thousands(u.input_tokens), crate::home::thousands(u.output_tokens)));
        // Sender row: the logo tile, the service in Bold, the model.
        let service =
            self.state.current.as_ref().and_then(|c| self.state.conversation(c)).and_then(|c| self.state.backend(&c.backend_id)).map(|b| b.config.name.clone());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 9.0;
            let (tile, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
            let p = ui.painter();
            p.rect(
                tile,
                CornerRadius::same(8),
                egui::Color32::WHITE,
                Stroke::new(1.0, if t.dark { egui::Color32::TRANSPARENT } else { t.border }),
                egui::StrokeKind::Inside,
            );
            crate::kit::paint_logo(p, egui::Rect::from_center_size(tile.center(), egui::vec2(15.0, 12.8)), egui::Color32::from_rgb(0x10, 0x7f, 0xad));
            ui.label(crate::theme::bold(service.unwrap_or_else(|| "AIS.ChatCraft".into())).size(14.0).color(t.text));
            if let Some(model) = &m.model {
                let r = ui.label(RichText::new(model).size(13.5).color(t.muted));
                if let Some(tk) = &tokens {
                    r.on_hover_text(tk);
                }
            }
            self.variant_switcher(ui, &m.id);
        });
        // What the model thought (#30): Claude's summarised thinking, the reasoning of
        // OpenAI-compatible servers, or AIS.chat's analysis steps. Folded; open while it thinks.
        let thinking = thinking_text(m);
        if !thinking.is_empty() {
            let still = generating && m.content.is_empty();
            let title = if still { "Denkt nach …" } else { "Nachgedacht" };
            egui::CollapsingHeader::new(small_muted(&t, title)).id_salt(("think", &m.id)).default_open(false).show(ui, |ui| {
                Frame::new().stroke(Stroke::new(1.0, t.border)).inner_margin(Margin { left: 10, right: 4, top: 2, bottom: 2 }).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(thinking.as_str()).size(13.5).color(t.text2)).wrap());
                });
            });
        }
        let shown_steps: Vec<(usize, &aiscraft_core::ActivityStep)> =
            m.activity.iter().enumerate().filter(|(_, s)| !(is_analysis(s) && m.thinking.trim().is_empty())).collect();
        if !shown_steps.is_empty() {
            let open_default = generating;
            egui::CollapsingHeader::new(small_muted(&t, format!("Arbeitsschritte ({})", shown_steps.len())))
                .id_salt(("act", &m.id))
                .default_open(open_default)
                .show(ui, |ui| {
                    for (i, s) in shown_steps {
                        ui.horizontal_wrapped(|ui| {
                            let mark = if s.failed {
                                RichText::new("×").color(t.danger)
                            } else if s.done {
                                RichText::new("✔").color(t.success)
                            } else {
                                RichText::new("…").color(t.muted)
                            };
                            ui.label(mark);
                            ui.label(RichText::new(&s.label).strong());
                            if let Some(d) = &s.detail {
                                ui.add(egui::Label::new(small_muted(&t, d.chars().take(300).collect::<String>())).wrap());
                            }
                        });
                        let dead_note = if s.label == aiscraft_engine::LINK_CHECK { "nicht erreichbar" } else { "nicht lesbar" };
                        for l in &s.links {
                            if l.failed {
                                ui.horizontal(|ui| {
                                    ui.hyperlink_to(RichText::new(format!("   ↗ {}", l.title)).small().color(t.muted).strikethrough(), &l.url);
                                    ui.label(RichText::new(dead_note).small().color(t.danger));
                                });
                            } else {
                                ui.hyperlink_to(RichText::new(format!("   ↗ {}", l.title)).small(), &l.url);
                            }
                        }
                        if let Some(doc) = &s.document
                            && ui.button(format!("💾  {} speichern", doc.file_name)).clicked()
                            && let Some(path) = rfd::FileDialog::new().set_file_name(&doc.file_name).save_file()
                        {
                            self.run("chat.save_document", json!({"message_id": m.id, "step": i, "path": path.display().to_string()}));
                        }
                    }
                });
        }
        if !m.tool_calls.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for tc in &m.tool_calls {
                    let (mark, color) = match tc.outcome.as_deref() {
                        Some("ok") => ("✔", t.success),
                        Some("denied") => ("×", t.warning),
                        // Not asked while another call of the step waited for a yes (#63).
                        Some("skipped") => ("–", t.muted),
                        // It may or may not have run (#24).
                        Some("uncertain") => ("?", t.warning),
                        Some(_) => ("×", t.danger),
                        None => ("…", t.muted),
                    };
                    let label = self.tool_label(&tc.name);
                    egui::Frame::new()
                        .fill(t.surface_hi)
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(mark).color(color));
                                ui.label(RichText::new(label).small());
                            });
                        })
                        .response
                        .on_hover_text(serde_json::to_string_pretty(&tc.arguments).unwrap_or_default());
                }
            });
        }
        self.download_cards(ui, m);
        if m.content.is_empty() && !m.tool_calls.is_empty() {
            return;
        }
        let images: Vec<(usize, aiscraft_core::Attachment)> = m.attachments.iter().filter(|a| a.mime.starts_with("image/")).cloned().enumerate().collect();
        for (i, a) in &images {
            // Images in an answer are generated: „KI-Kennzeichnung“ as in AIS.chat, on by default.
            let badge = self.image_badge(&m.id, *i);
            let tex = a.local_path.as_deref().and_then(|p| self.image_texture(ui.ctx(), p, badge));
            match tex {
                Some(tex) => {
                    let size = tex.size_vec2();
                    let w = size.x.min(ui.available_width()).min(512.0);
                    let h = if size.x > 0.0 { w * size.y / size.x } else { w };
                    ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(w, h)).corner_radius(CornerRadius::same(t.radius)));
                }
                None => {
                    ui.label(small_muted(&t, format!("Bild {} lässt sich nicht anzeigen", a.name)));
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui.add(egui::Button::new(small_muted(&t, "Bild speichern …")).frame(false)).clicked()
                    && let Some(path) = rfd::FileDialog::new().set_file_name(badged_name(&a.name, badge)).save_file()
                {
                    self.run("chat.save_image", self.image_save_params(&m.id, *i, &path.display().to_string()));
                }
                if ui.add(egui::Button::new(small_muted(&t, "Bild kopieren")).frame(false)).clicked()
                    && let Some(img) = a.local_path.as_deref().and_then(|p| crate::app::image_pixels(p, badge))
                {
                    ui.ctx().copy_image(img);
                }
                if ui.add(egui::Button::new(small_muted(&t, "Prompt kopieren")).frame(false)).clicked() {
                    let shown = self.engine.shown_messages(&self.state);
                    let at = shown.iter().position(|x| x.id == m.id).unwrap_or(shown.len());
                    if let Some(prompt) = shown.get(..at).and_then(|before| before.iter().rev().find(|x| x.role == aiscraft_core::Role::User)) {
                        ui.ctx().copy_text(prompt.content.clone());
                    }
                }
                let mut on = badge;
                if ui
                    .checkbox(&mut on, small_muted(&t, "KI-Kennzeichnung"))
                    .on_hover_text("Zeigt, kopiert und speichert das Bild mit dem Zeichen „AI“ unten rechts, wie AIS.chat")
                    .changed()
                {
                    let key = (m.id.clone(), *i);
                    if on {
                        self.badge_off.remove(&key);
                    } else {
                        self.badge_off.insert(key);
                    }
                }
                if is_last && icon_button(ui, &t, "⟳", "Neu erzeugen (Strg+R)").clicked() {
                    self.run("chat.regenerate", json!({}));
                }
            });
        }
        if m.content.is_empty() && generating {
            let image_chat =
                self.state.current.as_ref().and_then(|c| self.state.conversation(c)).is_some_and(|c| c.kind == aiscraft_core::ConversationKind::Image);
            crate::waiting::show(ui, &t, &self.state.settings.wait_animation, &m.id, image_chat);
        } else if !m.content.is_empty() || images.is_empty() {
            // Diagrams from AIS.chat's create_plot tool come as ```jsxgraph-json blocks; formulas
            // and Mermaid diagrams are drawn by crate::markup. Cited statements get [n] markers
            // (#39) that open the citation like the entries under the answer.
            let content = crate::citations::with_markers(&m.content, &m.citations);
            for (k, seg) in crate::markup::segments(&content).into_iter().enumerate() {
                match seg {
                    crate::markup::Segment::Plot(src) => self.plot_block(ui, &m.id, k, src),
                    crate::markup::Segment::App(src) => self.learn_card(ui, aiscraft_core::ArtifactKind::App, src),
                    crate::markup::Segment::Worksheet(src) => self.learn_card(ui, aiscraft_core::ArtifactKind::Worksheet, src),
                    crate::markup::Segment::H5p(src) => self.learn_card(ui, aiscraft_core::ArtifactKind::H5p, src),
                    crate::markup::Segment::Board(src) => self.learn_card(ui, aiscraft_core::ArtifactKind::Board, src),
                    seg => {
                        if let Some(crate::markup::Action::Board(outline)) = crate::markup::show(ui, &mut self.md, &t, &seg, (&m.id, k)) {
                            self.run("artifact.create", json!({"kind": "board", "content": outline}));
                        }
                    }
                }
            }
            // Such addresses never reach the browser, also when a model writes one itself.
            if let Some(i) = crate::citations::take_marker_click(ui.ctx())
                && i < m.citations.len()
            {
                self.run("message.citation", json!({"message_id": m.id, "index": i}));
            }
        }
        sources_row(ui, &t, m);
        // #66: which backup model answered, and why.
        if let Some(note) = m.activity.iter().find(|s| s.label == aiscraft_engine::FALLBACK).and_then(|s| s.detail.as_deref()) {
            ui.add(egui::Label::new(small_muted(&t, note)).wrap());
        }
        self.citations_row(ui, m);
        dead_links_row(ui, &t, m);
        if let Some(err) = &m.error {
            Frame::new()
                .fill(t.danger.gamma_multiply(0.12))
                .stroke(Stroke::new(1.0, t.danger.gamma_multiply(0.5)))
                .corner_radius(CornerRadius::same(t.radius))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.label(RichText::new(err).color(t.danger));
                    ui.horizontal(|ui| {
                        if is_last && ui.button("Erneut versuchen").clicked() {
                            self.run("chat.regenerate", json!({}));
                        }
                        let backend = self.state.current.as_ref().and_then(|c| self.state.conversation(c)).map(|c| c.backend_id.clone());
                        if let Some(b) = backend
                            && self.state.backend(&b).is_some_and(|v| !matches!(v.status, BackendStatus::Ready { .. }))
                            && ui.button("Anmelden").clicked()
                        {
                            self.dialog = Some(Dialog::login(&b));
                        }
                    });
                });
        }
        // Changes an assistant without tools (AIS.chat) proposed as marked blocks: apply on click.
        if !generating {
            let blocks = aiscraft_engine::change_blocks(&m.content);
            for (kind, content) in blocks {
                // The panel's content, or the one going along from its own window (#75).
                let target = [self.panel_artifact(), self.attach_artifact.clone()]
                    .into_iter()
                    .flatten()
                    .find(|id| self.state.artifacts.iter().any(|a| &a.id == id && a.kind == kind));
                ui.horizontal(|ui| match (kind, target) {
                    (aiscraft_core::ArtifactKind::Document, Some(id)) => {
                        if accent_button(ui, &t, "Ins Dokument übernehmen")
                            .on_hover_text("Ersetzt den Inhalt; die vorige Fassung bleibt unter Versionen")
                            .clicked()
                        {
                            self.run("artifact.save", json!({"id": id, "text": content, "version": "Aus einer Antwort übernommen"}));
                        }
                    }
                    (aiscraft_core::ArtifactKind::Board, Some(id)) => {
                        if accent_button(ui, &t, "Aufs Whiteboard").on_hover_text("Ergänzt das geöffnete Whiteboard um diese Punkte").clicked() {
                            self.run("artifact.board_add", json!({"id": id, "outline": content}));
                        }
                    }
                    // Learning apps and worksheets have their own cards (crate::learn_panel).
                    (aiscraft_core::ArtifactKind::App | aiscraft_core::ArtifactKind::Worksheet | aiscraft_core::ArtifactKind::H5p, Some(_)) => {}
                    (kind, None) => {
                        let label = if kind == aiscraft_core::ArtifactKind::Board { "Als neues Whiteboard öffnen" } else { "Als neues Dokument öffnen" };
                        if ui.button(label).clicked() {
                            self.run("artifact.create", json!({"kind": kind.as_str(), "content": content}));
                        }
                    }
                });
            }
        }
        // „Vorlesen“ only where a service is set up that can speak (the engine decides).
        let can_speak = self.state.speech_available();
        if !generating && !m.content.is_empty() {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                use crate::icons::Icon;
                use crate::kit::{Pill, pill};
                let copy = pill(ui, &t, Pill::Ghost, Some(Icon::Copy), "Kopieren", 13.0, None)
                    .on_hover_text("Antwort in die Zwischenablage; Rechtsklick: mit Kennzeichnung");
                self.copy_menu(&copy, m);
                if copy.clicked() {
                    ui.ctx().copy_text(m.content.clone());
                }
                if can_speak
                    && pill(ui, &t, Pill::Ghost, Some(Icon::Speaker), "Vorlesen", 13.0, None)
                        .on_hover_text("Antwort vorlesen (AIS.chat oder OpenAI-kompatibler Dienst)")
                        .clicked()
                {
                    self.run("voice.speak", json!({"message_id": m.id}));
                }
                if pill(ui, &t, Pill::Ghost, Some(Icon::File), "Als Dokument", 13.0, None)
                    .on_hover_text("Antwort neben dem Chat als bearbeitbares Dokument öffnen")
                    .clicked()
                {
                    self.run("artifact.create", json!({"kind": "document", "from_message": m.id}));
                }
                if pill(ui, &t, Pill::Ghost, Some(Icon::Note), "Als Whiteboard", 13.0, None)
                    .on_hover_text("Gliederung der Antwort als Mindmap öffnen")
                    .clicked()
                {
                    self.run("artifact.create", json!({"kind": "board", "from_message": m.id}));
                }
                if aiscraft_engine::has_links(&m.content)
                    && pill(ui, &t, Pill::Ghost, Some(Icon::Link), "Links prüfen", 13.0, None)
                        .on_hover_text("Fragt jede Adresse in der Antwort einmal an; tote Links werden durchgestrichen")
                        .clicked()
                {
                    self.run("message.check_links", json!({"message_id": m.id}));
                }
                self.answer_tools(ui, &t, m);
                if is_last && crate::kit::icon_button(ui, &t, Icon::Refresh, "Neu erzeugen (Strg+R)", t.muted, 17.0, 32.0).clicked() {
                    self.run("chat.regenerate", json!({}));
                }
            });
            self.readability_line(ui, &t, m);
        } else if !generating && !m.content.is_empty() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if message_action(ui, &t, "🗐", "Kopieren").on_hover_text("Antwort in die Zwischenablage").clicked() {
                    ui.ctx().copy_text(m.content.clone());
                }
                if can_speak && message_action(ui, &t, "🔊", "Vorlesen").on_hover_text("Antwort vorlesen (AIS.chat oder OpenAI-kompatibler Dienst)").clicked()
                {
                    self.run("voice.speak", json!({"message_id": m.id}));
                }
                if message_action(ui, &t, "📄", "Als Dokument").on_hover_text("Antwort neben dem Chat als bearbeitbares Dokument öffnen").clicked() {
                    self.run("artifact.create", json!({"kind": "document", "from_message": m.id}));
                }
                if message_action(ui, &t, "▣", "Als Whiteboard").on_hover_text("Gliederung der Antwort als Mindmap öffnen").clicked() {
                    self.run("artifact.create", json!({"kind": "board", "from_message": m.id}));
                }
                if aiscraft_engine::has_links(&m.content)
                    && ui
                        .add(egui::Button::new(small_muted(&t, "Links prüfen")).frame(false))
                        .on_hover_text("Fragt jede Adresse in der Antwort einmal an; tote Links werden durchgestrichen")
                        .clicked()
                {
                    self.run("message.check_links", json!({"message_id": m.id}));
                }
                if is_last && icon_button(ui, &t, "⟳", "Neu erzeugen (Strg+R)").clicked() {
                    self.run("chat.regenerate", json!({}));
                }
            });
        }
    }

    /// ‹ 1/2 › for a message with several versions (after edit or regenerate).
    fn variant_switcher(&mut self, ui: &mut Ui, message_id: &str) {
        let Some(v) = self.state.variants.get(message_id).copied() else { return };
        let t = self.tokens.clone();
        let busy = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        let gap = ui.spacing().item_spacing.x;
        ui.spacing_mut().item_spacing.x = 4.0;
        let prev = ui.add_enabled(!busy && v.index > 1, egui::Button::new(RichText::new("‹").size(17.0)).frame(false)).on_hover_text("Vorige Fassung");
        if prev.clicked() {
            self.run("chat.variant", json!({"message_id": message_id, "index": v.index - 1}));
        }
        ui.label(small_muted(&t, format!("{}/{}", v.index, v.count)));
        let next = ui.add_enabled(!busy && v.index < v.count, egui::Button::new(RichText::new("›").size(17.0)).frame(false)).on_hover_text("Nächste Fassung");
        if next.clicked() {
            self.run("chat.variant", json!({"message_id": message_id, "index": v.index + 1}));
        }
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_space(gap);
    }

    /// A diagram from a ```jsxgraph-json block: drawn where possible, the data one click away.
    fn plot_block(&mut self, ui: &mut Ui, message_id: &str, k: usize, src: &str) {
        let t = self.tokens.clone();
        Frame::new().stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::same(10)).show(ui, |ui| {
            match crate::plot::parse_plot(src) {
                Ok(p) => {
                    crate::plot::show(ui, &p, t.text, t.muted, t.code_bg);
                    if !p.sliders.is_empty() {
                        let s: Vec<String> = p.sliders.iter().map(|(n, v)| format!("{n} = {}", format!("{v}").replace('.', ","))).collect();
                        ui.label(small_muted(&t, format!("Schieberegler (Startwert): {}", s.join(", "))));
                    }
                    if !p.skipped.is_empty() {
                        ui.label(small_muted(&t, format!("Nicht dargestellt: {} (in AIS.chat interaktiv)", p.skipped.join(", "))));
                    }
                }
                Err(e) => {
                    ui.label(RichText::new(format!("Diagramm: {e}")).color(t.warning).small());
                }
            }
            egui::CollapsingHeader::new(small_muted(&t, "Diagrammdaten")).id_salt(("plot", message_id, k)).default_open(false).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(src.chars().take(6000).collect::<String>()).monospace().small()).wrap());
                if ui.small_button("Kopieren").clicked() {
                    ui.ctx().copy_text(src.to_string());
                }
            });
        });
    }

    /// How full the model's context window is: counted by the server where it reports usage,
    /// estimated from the text otherwise.
    fn context_meter(&self, ui: &mut Ui) {
        let t = &self.tokens;
        let sel = &self.state.selection;
        let window = self.state.backend(&sel.backend_id).and_then(|b| b.models.iter().find(|m| m.id == sel.model_id)).and_then(|m| m.context_window);
        let counted = self
            .state
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant && m.usage.is_some_and(|u| u.input_tokens > 0))
            .and_then(|m| m.usage)
            .map(|u| u.input_tokens + u.output_tokens);
        let (used, estimated) = match counted {
            Some(n) => (n, false),
            None => {
                let chars: usize = self
                    .state
                    .messages
                    .iter()
                    .map(|m| m.content.chars().count() + m.attachments.iter().filter_map(|a| a.text.as_ref()).map(|x| x.chars().count()).sum::<usize>())
                    .sum();
                ((chars / 4) as u64, true)
            }
        };
        if used == 0 {
            return;
        }
        let fmt = |n: u64| -> String {
            if n >= 1_000_000 {
                format!("{:.1}M", n as f64 / 1e6).replace(".0M", "M").replace('.', ",")
            } else if n >= 1000 {
                format!("{:.1}k", n as f64 / 1e3).replace(".0k", "k").replace('.', ",")
            } else {
                n.to_string()
            }
        };
        let prefix = if estimated { "≈ " } else { "" };
        let tip = if estimated {
            "Kontext geschätzt aus der Textlänge (dieser Dienst meldet keine Token-Zahlen)".to_string()
        } else {
            "Kontext laut Server: Eingabe der letzten Antwort plus Antwort, also ungefähr das, was die nächste Nachricht mitschickt".to_string()
        };
        // The window: as the service reports it, else what this model family is known to take,
        // else an assumption (said so in the tooltip and with „≈“).
        let (w, window_note) = match window.filter(|w| *w > 0) {
            Some(w) => (w, String::new()),
            None => match known_context_window(&sel.model_id) {
                Some(w) => (w, " Größe des Kontextfensters: übliche Größe dieses Modells, der Dienst meldet sie nicht.".to_string()),
                None => (ASSUMED_CONTEXT_WINDOW, " Der Dienst meldet die Größe des Kontextfensters nicht; angenommen sind 128k Tokens.".to_string()),
            },
        };
        let prefix = if window_note.is_empty() { prefix } else { "≈ " };
        let frac = (used as f32 / w as f32).clamp(0.0, 1.0);
        let color = if frac > 0.9 {
            t.danger
        } else if frac > 0.7 {
            t.warning
        } else {
            t.accent
        };
        // One control: label, ring and percentage share a tooltip (the row is right to left).
        let pct = format!("{prefix}{:.0} %", (frac * 100.0).max(1.0));
        let g1 = crate::kit::line(ui, "Kontext", crate::kit::font(12.5, crate::theme::SEMIBOLD), t.muted, 100.0);
        let g2 = crate::kit::line(ui, &pct, crate::kit::font(12.5, crate::theme::SEMIBOLD), t.text2, 100.0);
        const RING: f32 = 16.0;
        let wdt = 12.0 + g1.size().x + 8.0 + RING + 6.0 + g2.size().x + 12.0;
        let (rect, r) = ui.allocate_exact_size(egui::vec2(wdt, 30.0), egui::Sense::hover());
        let p = ui.painter();
        let stroke = if t.glass.is_some() { t.card_stroke } else { t.border };
        p.rect(rect, CornerRadius::same(15), t.surface, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        let mut x = rect.left() + 12.0;
        let g1w = g1.size().x;
        p.galley(egui::pos2(x, rect.center().y - g1.size().y / 2.0), g1, t.muted);
        x += g1w + 8.0;
        let center = egui::pos2(x + RING / 2.0, rect.center().y);
        context_ring(p, center, RING / 2.0 - 1.5, frac, t.border, color);
        p.galley(egui::pos2(x + RING + 6.0, rect.center().y - g2.size().y / 2.0), g2, t.text2);
        r.on_hover_text(format!("Kontext {prefix}{} von {} Tokens ({:.0} %). {tip}.{window_note}", fmt(used), fmt(w), frac * 100.0));
    }
}

/// Assumed context window when neither the service nor the model name tells (most current models
/// take at least this much).
const ASSUMED_CONTEXT_WINDOW: u64 = 128_000;

/// The usual context window of well-known model families, by the model id (lower case match).
pub(crate) fn known_context_window(model_id: &str) -> Option<u64> {
    let id = model_id.to_ascii_lowercase();
    let table: [(&str, u64); 12] = [
        ("claude", 200_000),
        ("gemini", 1_000_000),
        ("gpt-4.1", 1_000_000),
        ("gpt-5", 400_000),
        ("gpt-4o", 128_000),
        ("o3", 200_000),
        ("o4", 200_000),
        ("mistral-large", 128_000),
        ("llama-3", 128_000),
        ("llama3", 128_000),
        ("qwen", 32_000),
        ("deepseek", 128_000),
    ];
    table.iter().find(|(k, _)| id.contains(k)).map(|(_, w)| *w)
}

/// A ring that fills clockwise from the top with `frac` of a full turn.
fn context_ring(p: &egui::Painter, center: egui::Pos2, radius: f32, frac: f32, track: egui::Color32, fill: egui::Color32) {
    p.circle_stroke(center, radius, Stroke::new(3.0, track));
    let frac = frac.clamp(0.0, 1.0).max(0.02);
    let steps = ((48.0 * frac).ceil() as usize).max(2);
    let points: Vec<egui::Pos2> = (0..=steps)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * (i as f32 / steps as f32);
            center + radius * egui::vec2(a.cos(), a.sin())
        })
        .collect();
    p.add(egui::Shape::line(points, Stroke::new(3.0, fill)));
}

impl AisCraftApp {
    /// What this chat may draw on: tools, memory, project instructions, history, each connector.
    fn sources_menu(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let src = self.state.selection.sources.clone();
        let caps = self
            .state
            .backend(&self.state.selection.backend_id)
            .map(|b| b.capabilities.for_model(b.config.kind, &self.state.selection.model_id))
            .unwrap_or_default();
        let takes_label = self.state.backend(&self.state.selection.backend_id).is_none_or(|b| b.capabilities.system_prompt);
        let on = [src.tools, src.memory_for(takes_label), src.project_for(takes_label), src.history].iter().filter(|x| **x).count();
        let label = if on == 4 && src.connectors.is_none() { "Quellen".to_string() } else { format!("Quellen ({on}/4)") };
        // Stays open while switching; closes with its button, the toggle button or Esc.
        let toggle = crate::kit::pill(ui, &t, crate::kit::Pill::Chip, None, &label, 13.5, Some(crate::icons::Icon::ChevronDown));
        let popup_id = egui::Popup::default_response_id(&toggle);
        egui::Popup::from_toggle_button_response(&toggle).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            ui.set_min_width(280.0);
            ui.horizontal(|ui| {
                ui.label(small_muted(&t, "Was dieser Chat nutzen darf"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Schließen").clicked() {
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                });
            });
            let set = |key: &str, value: bool| self.engine.dispatch("chat.set_sources", json!({key: value}));
            // Without a system prompt (AIS.chat) notes and instructions go along in the message and stay in the
            // backend's history: off until switched on here, and switching one on does not switch on the other.
            // No backend chosen yet: nothing goes anywhere, show the switches as they are.
            let takes = self.state.backend(&self.state.selection.backend_id).is_none_or(|b| b.capabilities.system_prompt);
            let switch_inline = |key: &str, value: bool| {
                if takes || !value {
                    set(key, value);
                } else {
                    let other = if key == "memory" { "project" } else { "memory" };
                    let other_on = if key == "memory" { src.project_for(false) } else { src.memory_for(false) };
                    self.engine.dispatch("chat.set_sources", json!({key: true, other: other_on, "inline": true}));
                }
            };
            let mut memory = src.memory_for(takes);
            if ui.checkbox(&mut memory, "Gedächtnis (deine Notizen)").changed() {
                switch_inline("memory", memory);
            }
            // #20: shown once there is a style note.
            if self.state.memory.iter().any(|n| n.area == aiscraft_core::MemoryArea::Style && !n.archived) {
                let mut own = src.own_style;
                if ui.add_enabled(memory, egui::Checkbox::new(&mut own, "Mein Schreibstil (bei Entwürfen)")).changed() {
                    set("own_style", own);
                }
            }
            // Instructions belong to a project: outside one there are none to send.
            let in_project = self.state.selection.project_id.is_some();
            let mut project = in_project && src.project_for(takes);
            if ui
                .add_enabled(in_project, egui::Checkbox::new(&mut project, "Projektanweisungen"))
                .on_disabled_hover_text("Nur in einem Projekt: Verschiebe den Chat in ein Projekt oder starte ihn dort.")
                .changed()
            {
                switch_inline("project", project);
            }
            if !takes {
                let name = self.state.backend(&self.state.selection.backend_id).map(|b| b.config.name.clone()).unwrap_or_default();
                ui.label(small_muted(
                    &t,
                    format!("Gehen am Anfang deiner nächsten Nachricht mit und bleiben im Verlauf bei {name}; danach nur, wenn sie sich ändern."),
                ));
            }
            if self.state.selection.project_id.as_ref().is_some_and(|p| self.state.project_files.iter().any(|f| &f.project_id == p)) {
                // Without a system prompt (AIS.chat) the files need the same consent as notes and
                // instructions; switching them on leaves those two as they are.
                let mut knowledge = src.knowledge && (takes || src.inline);
                if ui.checkbox(&mut knowledge, "Projektwissen (Dateien des Projekts)").changed() {
                    if takes || !knowledge {
                        set("knowledge", knowledge);
                    } else {
                        self.engine.dispatch(
                            "chat.set_sources",
                            json!({"knowledge": true, "memory": src.memory_for(false), "project": src.project_for(false), "inline": true}),
                        );
                    }
                }
            }
            let mut tools = src.tools;
            if ui.checkbox(&mut tools, "Werkzeuge (Dokumente, Whiteboards, Zeitpläne)").changed() {
                set("tools", tools);
            }
            let mut history = src.history;
            if ui.add_enabled(src.tools, egui::Checkbox::new(&mut history, "Andere Chats durchsuchen")).changed() {
                set("history", history);
            }
            // Recall stays within the chat's service unless the person agrees: chats with Claude do not
            // reach AIS.chat unasked, nor the other way round.
            let mut others = src.other_services;
            let service = self.state.backend(&self.state.selection.backend_id).map(|b| b.config.name.clone()).unwrap_or_else(|| "diesen Dienst".to_string());
            let others_label = format!("Auch Chats mit anderen Diensten an {service} geben");
            if ui
                .add_enabled(src.tools && src.history, egui::Checkbox::new(&mut others, others_label))
                .on_hover_text("Passende Ausschnitte aus Chats mit anderen Diensten (etwa Claude oder AIS.chat) gehen sonst nicht mit.")
                .changed()
            {
                set("other_services", others);
            }
            if caps.web_search || caps.tools {
                let mut web = src.web && (caps.web_search || src.tools);
                let label = if caps.web_search { "Websuche (sucht und liest im Netz)" } else { "Webseiten lesen (nur lesen)" };
                if ui.add_enabled(caps.web_search || src.tools, egui::Checkbox::new(&mut web, label)).changed() {
                    set("web", web);
                }
            }
            // How much Claude thinks before it answers (#30): per chat.
            if self
                .state
                .backend(&self.state.selection.backend_id)
                .is_some_and(|b| aiscraft_core::speaks_anthropic(b.config.kind, &self.state.selection.model_id))
            {
                ui.separator();
                ui.label(small_muted(&t, "Nachdenken vor der Antwort"));
                ui.horizontal_wrapped(|ui| {
                    if ui.add(egui::Button::selectable(src.effort.is_none(), "Standard")).clicked() {
                        self.engine.dispatch("chat.set_sources", json!({"effort": null}));
                    }
                    for (id, label) in aiscraft_core::EFFORTS {
                        if ui.add(egui::Button::selectable(src.effort.as_deref() == Some(id), label)).clicked() {
                            self.engine.dispatch("chat.set_sources", json!({"effort": id}));
                        }
                    }
                });
                ui.label(small_muted(&t, "Gründlicher dauert länger und kostet mehr."));
            }
            let conns = self.state.connectors.clone();
            if !conns.is_empty() {
                ui.separator();
                ui.label(small_muted(&t, "Konnektoren"));
                for k in conns {
                    let mut allowed = src.allows_connector(&k.id) || (!src.tools && src.connectors.as_ref().is_none_or(|c| c.contains(&k.id)));
                    let text = if k.enabled { k.name.clone() } else { format!("{} (aus)", k.name) };
                    if ui.add_enabled(src.tools && k.enabled, egui::Checkbox::new(&mut allowed, text)).changed() {
                        let mut list: Vec<String> = src.connectors.clone().unwrap_or_else(|| self.state.connectors.iter().map(|c| c.id.clone()).collect());
                        if allowed {
                            if !list.contains(&k.id) {
                                list.push(k.id.clone());
                            }
                        } else {
                            list.retain(|x| x != &k.id);
                        }
                        self.engine.dispatch("chat.set_sources", json!({"connectors": list}));
                    }
                }
            }
            let server_tools = self.state.backend(&self.state.selection.backend_id).and_then(|b| match &b.status {
                aiscraft_engine::BackendStatus::Ready { account } => Some(account.server_tools.clone()),
                _ => None,
            });
            if let Some(st) = server_tools.filter(|s| !s.is_empty()) {
                ui.separator();
                ui.label(small_muted(&t, "Auf dem AIS.chat-Server (vom Land eingestellt)"));
                // An assistant, character or scenario can switch web search off in its AIS.chat settings.
                let persona = self
                    .state
                    .selection
                    .persona
                    .as_ref()
                    .and_then(|p| self.state.persona_info(&self.state.selection.backend_id, p).map(|i| (p.kind, i.web_search)));
                for tool in st {
                    let off_by_persona = tool.id == "web_search" && tool.enabled && matches!(persona, Some((_, Some(false))));
                    let enabled = tool.enabled && !off_by_persona;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if enabled { "✔" } else { "×" }).color(if enabled { t.success } else { t.muted }));
                        ui.label(if enabled { RichText::new(&tool.label) } else { RichText::new(&tool.label).color(t.muted) });
                        if let (true, Some((kind, _))) = (off_by_persona, persona) {
                            ui.label(small_muted(&t, format!("aus ({})", kind.label())));
                        }
                    });
                }
            }
            if !caps.tools {
                ui.separator();
                let name = self.state.backend(&self.state.selection.backend_id).map(|b| b.config.name.clone()).unwrap_or_default();
                let mut missing = Vec::new();
                if !caps.tools {
                    missing.push("Werkzeuge und Konnektoren");
                }
                ui.label(small_muted(&t, format!("{name} nimmt {} nicht an.", missing.join(" und "))));
            }
        });
    }

    /// How answers are written (#31): normal, concise, explanatory, formal, or after an example.
    fn style_menu(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let src = self.state.selection.sources.clone();
        let current = src.style.as_deref().and_then(|s| aiscraft_core::STYLES.iter().find(|(id, _)| *id == s)).map(|(_, l)| *l);
        let label = match current {
            Some(l) => format!("Stil: {l}"),
            None => "Stil".to_string(),
        };
        let toggle = crate::kit::pill(ui, &t, crate::kit::Pill::Chip, None, &label, 13.5, Some(crate::icons::Icon::ChevronDown))
            .on_hover_text("Wie die Antworten geschrieben sind");
        let popup_id = egui::Popup::default_response_id(&toggle);
        egui::Popup::from_toggle_button_response(&toggle).close_behavior(egui::PopupCloseBehavior::IgnoreClicks).show(|ui| {
            ui.set_min_width(260.0);
            ui.set_max_width(320.0);
            ui.horizontal(|ui| {
                ui.label(small_muted(&t, "Wie dieser Chat antwortet"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Schließen").clicked() {
                        self.style_draft = None;
                        egui::Popup::close_id(ui.ctx(), popup_id);
                    }
                });
            });
            if ui.add(egui::Button::selectable(src.style.is_none(), "Normal")).clicked() {
                self.run("chat.set_style", json!({"style": "normal"}));
            }
            for (id, label) in aiscraft_core::STYLES {
                if id == "custom" {
                    continue;
                }
                if ui.add(egui::Button::selectable(src.style.as_deref() == Some(id), label)).clicked() {
                    self.run("chat.set_style", json!({"style": id}));
                }
            }
            let custom = src.style.as_deref() == Some("custom");
            if ui.add(egui::Button::selectable(custom, "Eigener Stil …")).clicked() && self.style_draft.is_none() {
                self.style_draft = Some(src.style_sample.clone().unwrap_or_default());
            }
            if let Some(draft) = self.style_draft.as_mut() {
                ui.label(small_muted(&t, "Füge einen Text ein, dessen Stil die Antworten übernehmen sollen (etwa einen eigenen Elternbrief)."));
                ui.add(egui::TextEdit::multiline(draft).desired_rows(4).desired_width(f32::INFINITY).hint_text("Beispieltext"));
                let mut apply = false;
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(!draft.trim().is_empty(), egui::Button::new("Stil übernehmen")).clicked();
                });
                if apply {
                    let sample = draft.clone();
                    self.style_draft = None;
                    self.run("chat.set_style", json!({"style": "custom", "sample": sample}));
                }
            } else if custom {
                ui.label(small_muted(&t, "Antwortet im Stil deines Beispieltexts."));
            }
            let takes = self.state.backend(&self.state.selection.backend_id).is_none_or(|b| b.capabilities.system_prompt);
            if !takes && src.style.is_some() {
                let name = self.state.backend(&self.state.selection.backend_id).map(|b| b.config.name.clone()).unwrap_or_default();
                ui.label(small_muted(&t, format!("Geht als Hinweis in deiner nächsten Nachricht an {name} mit.")));
            }
        });
    }

    /// A readable name for a tool the model called.
    fn tool_label(&self, name: &str) -> String {
        match name {
            "history_search" => "Verlauf durchsucht".into(),
            "web_fetch" => "Webseite gelesen".into(),
            "document_create" => "Dokument angelegt".into(),
            "document_update" => "Dokument geändert".into(),
            "board_create" => "Whiteboard angelegt".into(),
            "task_start" => "Aufgabe gestartet".into(),
            "schedule_create" => "Nachfrage geplant".into(),
            "spreadsheet_create" => "Tabelle erstellt".into(),
            "slides_create" => "Folien erstellt".into(),
            "document_file_create" => "Datei erstellt".into(),
            other => self
                .state
                .connectors
                .iter()
                .find_map(|c| {
                    c.tools.iter().find(|t| t.llm_name == other).map(|t| format!("{} · {}", c.name, if t.title.is_empty() { &t.name } else { &t.title }))
                })
                .unwrap_or_else(|| other.to_string()),
        }
    }

    /// Questions from the tool rule: allow this exact call once, or not (#67).
    fn approval_cards(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(conv) = self.state.current.clone() else { return };
        for a in self.state.approvals.iter().filter(|a| a.conversation_id == conv).cloned().collect::<Vec<_>>() {
            if let Some(allow) = crate::approval::card(ui, &t, &a) {
                self.run("tool.answer", json!({"id": a.id, "allow": allow}));
            }
            ui.add_space(8.0);
        }
    }

    /// The files in the composer, each with what will be sent (#45, #47, #50): the converted
    /// name, "Standortdaten entfernt", an error, and for PDFs, tables and slides a field to send
    /// only some pages ("Nur Seiten …"). The engine prepares each file once per backend.
    /// Returns the index of a file to remove.
    fn attachment_chips(&mut self, ui: &mut Ui) -> Option<usize> {
        let t = self.tokens.clone();
        let mut remove = None;
        let mut requests = Vec::new();
        ui.vertical(|ui| {
            for (i, f) in self.pending_files.iter().enumerate() {
                let path = f.display().to_string();
                let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let preview = self.state.attachments.get(&path);
                // Ask once; the flag goes when the answer is there, so a file added again later
                // (after sending) is prepared again.
                let asked = egui::Id::unique(("attachment-asked", &path));
                if preview.is_some() {
                    ui.data_mut(|d| d.remove::<bool>(asked));
                } else if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
                    ui.data_mut(|d| {
                        d.insert_temp(asked, true);
                        d.remove::<String>(egui::Id::unique(("attachment-range", &path)));
                    });
                    requests.push(("attachment.prepare", json!({"path": path, "quiet": true})));
                }
                Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("📎 {name}")).small());
                        if let Some(p) = preview {
                            if let Some(err) = &p.error {
                                ui.label(RichText::new(err).small().color(t.danger));
                            } else if let Some(note) = &p.note {
                                ui.label(RichText::new(note).small().color(t.muted));
                            }
                            if let Some(kind) = p.range_kind {
                                let (label, hint) = match kind {
                                    aiscraft_engine::RangeKind::Pages => {
                                        ("Nur Seiten", if p.total > 0 { format!("1-{}", p.total) } else { "z. B. 3-7".into() })
                                    }
                                    aiscraft_engine::RangeKind::Slides => {
                                        ("Nur Folien", if p.total > 0 { format!("1-{}", p.total) } else { "z. B. 2-5".into() })
                                    }
                                    aiscraft_engine::RangeKind::Sheet => ("Nur Blatt", p.sheets.first().cloned().unwrap_or_else(|| "Name oder Nummer".into())),
                                };
                                let id = egui::Id::unique(("attachment-range", &path));
                                // While not typing in it, the field shows the range the engine uses.
                                let focused = ui.memory(|m| m.has_focus(id));
                                let mut text =
                                    if focused { ui.data_mut(|d| d.get_temp_mut_or_insert_with(id, || p.range.clone()).clone()) } else { p.range.clone() };
                                ui.label(RichText::new(label).small().color(t.muted));
                                let r = ui.add(egui::TextEdit::singleline(&mut text).id(id).hint_text(hint).desired_width(90.0).font(egui::TextStyle::Small));
                                if r.changed() {
                                    ui.data_mut(|d| d.insert_temp(id, text.clone()));
                                }
                                if r.lost_focus() && text.trim() != p.range {
                                    requests.push(("attachment.set_range", json!({"path": path, "range": text.trim(), "quiet": true})));
                                }
                                if kind == aiscraft_engine::RangeKind::Sheet && p.sheets.len() > 1 {
                                    r.on_hover_text(format!("Blätter: {}", p.sheets.join(", ")));
                                }
                            }
                        }
                        if icon_button(ui, &t, "×", "Entfernen").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
        for (cmd, params) in requests {
            self.run(cmd, params);
        }
        remove
    }

    pub(crate) fn composer(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let generating = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        let enter_sends = self.state.settings.enter_sends;
        let image_mode = self.state.current.as_ref().and_then(|c| self.state.conversation(c)).map(|c| c.kind).unwrap_or(self.state.selection.kind)
            == aiscraft_core::ConversationKind::Image;
        let mut send = false;
        let big = std::mem::take(&mut self.composer_big);
        // The text starts on the start page's keyline (`home::KEYLINE`), in the chat too: the field
        // itself has no margin of its own on the left.
        let k = crate::home::KEYLINE_I8;
        let m = if big { Margin { left: k, right: 16, top: 20, bottom: 14 } } else { Margin { left: k, right: 14, top: 16, bottom: 12 } };
        let frame = Frame::new()
            .fill(t.composer)
            .stroke(Stroke::new(1.0, t.composer_stroke))
            .corner_radius(CornerRadius::same(t.r_composer))
            .inner_margin(m)
            .shadow(t.composer_shadow);
        frame.show(ui, |ui| {
            self.status_line(ui);
            if self.state.selection.temporary {
                ui.label(small_muted(&t, "Temporärer Chat: wird nicht gespeichert und ist beim Schließen weg."));
            }
            if !self.pending_files.is_empty() {
                let remove = self.attachment_chips(ui);
                if let Some(i) = remove {
                    let gone = self.pending_files.remove(i);
                    self.run("attachment.remove", json!({"path": gone.display().to_string()}));
                }
            }
            if !self.pending_links.is_empty() {
                let mut remove = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, l) in self.pending_links.iter().enumerate() {
                        Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("🔗 {}", short_url(l))).small()).on_hover_text(l);
                                if icon_button(ui, &t, "×", "Entfernen").clicked() {
                                    remove = Some(i);
                                }
                            });
                        });
                    }
                });
                if let Some(i) = remove {
                    self.pending_links.remove(i);
                }
            }
            if let Some(input) = self.link_input.as_mut() {
                let mut done = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    let (r, cancelled) = crate::widgets::field_then_buttons(
                        ui,
                        |ui| ui.small_button("Abbrechen").clicked(),
                        |ui, w| ui.add(egui::TextEdit::singleline(input).hint_text("https://… (Enter fügt hinzu)").desired_width(w)),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        done = true;
                    }
                    if !r.has_focus() && input.is_empty() && !r.gained_focus() {
                        r.request_focus();
                    }
                    cancel = cancelled;
                });
                if done {
                    let url = input.trim().to_string();
                    if url.starts_with("http://") || url.starts_with("https://") {
                        if !self.pending_links.contains(&url) && self.pending_links.len() < 5 {
                            self.pending_links.push(url);
                        }
                        self.link_input = None;
                        self.focus_composer = true;
                    } else if !url.is_empty() {
                        self.engine.ctx().notice(aiscraft_engine::NoticeLevel::Warning, "Bitte eine Webadresse mit https:// eingeben.");
                    }
                } else if cancel {
                    self.link_input = None;
                }
            }
            if let Some(a) = self.attach_artifact.as_ref().and_then(|id| self.state.artifacts.iter().find(|a| &a.id == id)) {
                ui.label(small_muted(&t, format!("📄 {} geht als Kontext mit", a.title)));
            }
            self.composer_bars(ui);
            if !image_mode {
                self.privacy_preview(ui);
            }
            let has_focus = ui.memory(|m| m.focused()) == Some(egui::Id::unique("composer"));
            if enter_sends && has_focus && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                send = true;
            }
            if has_focus && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                send = true;
            }
            let newline = if enter_sends { KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter) } else { KeyboardShortcut::new(Modifiers::NONE, Key::Enter) };
            // „Erstellen“ chosen with nothing typed: the next text is its topic.
            let create_hint = self.create_pending.map(|(_, what)| format!("Thema für {what}, z. B. Brüche vergleichen, Klasse 6"));
            if let Some((_, what)) = self.create_pending {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{what} erstellen: Thema eingeben, dann Enter")).color(t.accent).strong());
                    if ui.small_button("Abbrechen").on_hover_text("Wieder eine normale Nachricht schreiben").clicked() {
                        self.create_pending = None;
                    }
                });
            }
            let hint = if let Some(h) = create_hint.as_deref() {
                h
            } else if image_mode {
                "Beschreibe das Bild, das entstehen soll …"
            } else if big {
                "Frag etwas oder beschreib, was du brauchst …"
            } else {
                "Schreib eine Nachricht …"
            };
            let edit = egui::TextEdit::multiline(&mut self.composer)
                .id(egui::Id::unique("composer"))
                .hint_text(RichText::new(hint).color(t.muted))
                .desired_rows(if big { 3 } else { 2 })
                .desired_width(f32::INFINITY)
                .frame(Frame::NONE)
                .return_key(newline)
                .margin(Margin::symmetric(0, 2))
                .font(egui::FontId::proportional(if big { 17.5 } else { 16.0 }));
            let r = ScrollArea::vertical().id_salt("composer-scroll").max_height(220.0).show(ui, |ui| ui.add(edit)).inner;
            if self.focus_composer && self.dialog.is_none() && self.palette.is_none() {
                r.request_focus();
                self.focus_composer = false;
            }
            if let Some(text) = self.dictated.lock().ok().and_then(|mut d| d.take()) {
                if !self.composer.is_empty() && !self.composer.ends_with(' ') {
                    self.composer.push(' ');
                }
                self.composer.push_str(text.trim());
                self.focus_composer = true;
            }
            ui.add_space(if big { 10.0 } else { 4.0 });
            self.composer_tools(ui, generating, image_mode, &mut send);
        });
        if !big {
            self.context_hint(ui);
        }
        // The topic for a pending „Erstellen“ goes to its command, not into the chat as a message.
        if send
            && !generating
            && let Some((cmd, _)) = self.create_pending
        {
            if crate::learn_panel::topic_usable(&self.composer) {
                let topic = std::mem::take(&mut self.composer).trim().to_string();
                self.create_pending = None;
                self.run(cmd, json!({"topic": topic}));
            }
            self.focus_composer = true;
            send = false;
        }
        if send && !generating {
            let text = std::mem::take(&mut self.composer);
            let files: Vec<String> = self.pending_files.drain(..).map(|p| p.display().to_string()).collect();
            let links = std::mem::take(&mut self.pending_links);
            self.link_input = None;
            if !text.trim().is_empty() || !files.is_empty() || !links.is_empty() {
                self.save_if_dirty();
                // In its own window the content saves there; send what is typed there too (#75).
                if let Some(id) = self.attach_artifact.clone()
                    && self.state.detached.iter().any(|d| d.id == id)
                {
                    self.save_window_if_dirty(&id);
                }
                let artifact = self.attach_artifact.clone();
                let privacy = self.privacy.params();
                self.privacy.clear();
                self.run("chat.send", json!({"text": text, "files": files, "links": links, "artifact": artifact, "privacy": privacy}));
            }
            self.focus_composer = true;
        }
    }

    /// Attach and link icons, the model, "Quellen" and "Erstellen" as filled
    /// chips, the microphone and the round send button.
    fn composer_tools(&mut self, ui: &mut Ui, generating: bool, image_mode: bool, send: &mut bool) {
        let t = self.tokens.clone();
        let mut tools_w = 0.0_f32;
        // A narrow composer (next to a panel, small window): "Erstellen" becomes the sparkle icon.
        let compact = ui.available_width() < 560.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let ok = !self.composer.trim().is_empty() || !self.pending_files.is_empty() || !self.pending_links.is_empty();
                let (rect, b) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
                let name = if generating { "Anhalten (Esc)" } else { "Senden" };
                b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, generating || ok, name));
                {
                    let p = ui.painter();
                    let round = CornerRadius::same(20);
                    p.add(
                        egui::epaint::Shadow { offset: [0, 4], blur: 12, spread: 0, color: t.accent.gamma_multiply(if ok || generating { 0.35 } else { 0.0 }) }
                            .as_shape(rect, round),
                    );
                    // Always the solid accent, as designed; with nothing to send the arrow is quieter.
                    crate::kit::accent_fill(p, rect, round, &t);
                    if (ok || generating) && b.hovered() {
                        p.rect_filled(rect, round, egui::Color32::from_white_alpha(28));
                    }
                    let icon = if generating { crate::icons::Icon::Stop } else { crate::icons::Icon::ArrowUp };
                    let fg = if ok || generating { t.on_accent } else { t.on_accent.gamma_multiply(0.7) };
                    icon.paint(p, rect.center(), if generating { 16.0 } else { 20.0 }, fg);
                }
                if b.on_hover_text(name).clicked() {
                    if generating {
                        self.run("chat.stop", json!({}));
                    } else if ok {
                        *send = true;
                    }
                }
                if !image_mode && !self.state.recording {
                    self.voice_mode_button(ui);
                }
                if self.state.recording {
                    if crate::kit::pill(ui, &t, crate::kit::Pill::Accent2, Some(crate::icons::Icon::Stop), "Diktat beenden", 13.5, None).clicked() {
                        let engine = self.engine.clone();
                        let out = self.dictated.clone();
                        std::thread::spawn(move || match engine.call("voice.record_stop", json!({})) {
                            Ok(v) => {
                                if let (Some(t), Ok(mut slot)) = (v.get("text").and_then(|x| x.as_str()), out.lock()) {
                                    *slot = Some(t.to_string());
                                }
                            }
                            Err(e) => {
                                engine.ctx().notice(aiscraft_engine::NoticeLevel::Error, e.message);
                            }
                        });
                    }
                    let level = self.engine.mic_level();
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 8.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(4), t.chip);
                    let mut fill = rect;
                    fill.set_width((rect.width() * (level * 3.0).clamp(0.05, 1.0)).max(2.0));
                    ui.painter().rect_filled(fill, CornerRadius::same(4), t.danger);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                } else if crate::kit::icon_button(ui, &t, crate::icons::Icon::Mic, "Diktieren", t.text2, 19.0, 36.0).clicked() {
                    self.run("voice.record_start", json!({}));
                }
                // Everything right of here (send, voice, microphone) is drawn already.
                let right_used = ui.max_rect().width() - ui.available_width();
                ui.with_layout(Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    let start = ui.cursor().min;
                    // The width all buttons take in one row: each step adds what the cursor moved
                    // (after a wrap: from the row start).
                    let (mut acc, mut last) = (0.0_f32, start);
                    fn step(ui: &Ui, acc: &mut f32, last: &mut egui::Pos2, start: egui::Pos2) {
                        let now = ui.cursor().min;
                        *acc += if (now.y - last.y).abs() < 1.0 { now.x - last.x } else { now.x - start.x };
                        *last = now;
                    }
                    if compact && !image_mode {
                        self.create_menu_icon(ui);
                    }
                    step(ui, &mut acc, &mut last, start);
                    if !image_mode
                        && crate::kit::icon_button(ui, &t, crate::icons::Icon::Clip, "Dateien anhängen (oder hineinziehen)", t.text2, 19.0, 34.0).clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        self.take_files(files);
                    }
                    step(ui, &mut acc, &mut last, start);
                    if !image_mode
                        && self.link_input.is_none()
                        && crate::kit::icon_button(ui, &t, crate::icons::Icon::Globe, "Webseite anhängen (bis zu 5 Links)", t.text2, 19.0, 34.0).clicked()
                    {
                        self.link_input = Some(String::new());
                    }
                    step(ui, &mut acc, &mut last, start);
                    ui.add_space(2.0);
                    self.model_picker(ui);
                    step(ui, &mut acc, &mut last, start);
                    if image_mode {
                        let cur = self.state.selection.aspect_ratio.clone();
                        let label = aiscraft_core::ASPECT_RATIOS.iter().find(|(k, _)| *k == cur).map(|(_, l)| *l).unwrap_or("Quadratisch");
                        let r = crate::kit::pill(ui, &t, crate::kit::Pill::Chip, None, label, 13.5, Some(crate::icons::Icon::ChevronDown));
                        egui::Popup::menu(&r).show(|ui| {
                            for (k, l) in aiscraft_core::ASPECT_RATIOS {
                                if ui.add(egui::Button::selectable(k == cur || (cur.is_empty() && k == "quadratic"), l)).clicked() {
                                    self.run("chat.set_aspect", json!({"ratio": k}));
                                    ui.close();
                                }
                            }
                        });
                    } else {
                        self.sources_menu(ui);
                        step(ui, &mut acc, &mut last, start);
                        self.style_menu(ui);
                        step(ui, &mut acc, &mut last, start);
                        self.privacy_switch(ui);
                        step(ui, &mut acc, &mut last, start);
                        if !compact {
                            self.create_menu(ui);
                        }
                    }
                    step(ui, &mut acc, &mut last, start);
                    tools_w = acc + right_used;
                });
            });
        });
        // Frame margins (keyline left, 16 right), the stroke and a little air. Capped, so a long
        // model name can't make the column endless.
        if tools_w > 0.0 {
            self.composer_need = (tools_w + f32::from(crate::home::KEYLINE_I8) + 16.0 + 8.0).min(1100.0);
        }
    }

    /// Say when project instructions or notes are not sent, so nobody relies on them silently.
    fn context_hint(&self, ui: &mut Ui) {
        let t = &self.tokens;
        let Some(b) = self.state.backend(&self.state.selection.backend_id) else { return };
        let has_project = self
            .state
            .selection
            .project_id
            .as_ref()
            .and_then(|p| self.state.projects.iter().find(|x| &x.id == p))
            .is_some_and(|p| !p.instructions.trim().is_empty());
        let has_memory = self.state.settings.memory && self.state.memory.iter().any(|n| !n.archived);
        let src = &self.state.selection.sources;
        let sends = (b.config.send_context && b.capabilities.system_prompt) || (!b.capabilities.system_prompt && src.inline && (src.memory || src.project));
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            let typing = !self.composer.is_empty() && ui.memory(|m| m.focused()) == Some(egui::Id::unique("composer"));
            if (has_project || has_memory) && !sends {
                ui.label(small_muted(t, format!("Projektanweisungen und Notizen gehen nicht an {}.", b.config.name)));
            } else if typing {
                ui.label(small_muted(
                    t,
                    if self.state.settings.enter_sends {
                        "Enter sendet, Umschalt+Enter beginnt eine neue Zeile."
                    } else {
                        "Strg+Enter sendet, Enter beginnt eine neue Zeile."
                    },
                ));
            } else {
                ui.label(small_muted(t, "KI kann Fehler machen. Prüfe wichtige Angaben."));
            }
        });
    }
}

/// Lay out `add` in a centred column of at most [`CONTENT_WIDTH`].
pub(crate) fn centered<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    centered_w(ui, CONTENT_WIDTH, add)
}

/// Lay out `add` in a centred column of at most `max` points.
pub(crate) fn centered_w<R>(ui: &mut Ui, max: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = avail.min(max);
    let pad = ((avail - w) / 2.0).max(12.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w - if pad > 12.0 { 0.0 } else { 24.0 });
            add(ui)
        })
        .inner
    })
    .inner
}

/// The pages an answer cites or read, as links under it.
fn sources_row(ui: &mut Ui, t: &crate::theme::Tokens, m: &Message) {
    let mut seen = std::collections::HashSet::new();
    let links: Vec<_> = m.activity.iter().flat_map(|s| &s.links).filter(|l| l.cited && !l.failed && seen.insert(l.url.as_str())).take(20).collect();
    if links.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(small_muted(t, "Quellen:"));
        for (i, l) in links.iter().enumerate() {
            ui.hyperlink_to(RichText::new(format!("[{}] {}", i + 1, l.title.chars().take(60).collect::<String>())).small(), &l.url).on_hover_text(&l.url);
        }
    });
}

/// Links of the answer the link check found dead, struck through right under it.
fn dead_links_row(ui: &mut Ui, t: &crate::theme::Tokens, m: &Message) {
    let Some(step) = m.activity.iter().find(|s| s.label == aiscraft_engine::LINK_CHECK) else { return };
    let dead: Vec<_> = step.links.iter().filter(|l| l.failed).collect();
    if dead.is_empty() {
        ui.label(small_muted(t, format!("Links geprüft: {}", step.detail.as_deref().unwrap_or(""))));
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Nicht erreichbar:").small().color(t.danger));
        for l in dead {
            ui.hyperlink_to(RichText::new(short_url(&l.url)).small().color(t.muted).strikethrough(), &l.url).on_hover_text(&l.title);
        }
    });
}

/// A link as people read it: host and path, without the scheme, shortened.
/// AIS.chat's analysis steps („Analyse“, „Zusammenfassung“): its counterpart to thinking.
fn is_analysis(s: &aiscraft_core::ActivityStep) -> bool {
    (s.label == "Analyse" || s.label == "Zusammenfassung") && s.detail.as_deref().is_some_and(|d| !d.trim().is_empty())
}

/// The thinking to show above an answer: the backend's own, else AIS.chat's analysis steps.
fn thinking_text(m: &Message) -> String {
    if !m.thinking.trim().is_empty() {
        return m.thinking.trim().to_string();
    }
    m.activity.iter().filter(|s| is_analysis(s)).filter_map(|s| s.detail.as_deref()).map(str::trim).collect::<Vec<_>>().join("\n\n")
}

fn short_url(url: &str) -> String {
    let s = url.split_once("://").map(|(_, r)| r).unwrap_or(url).trim_end_matches('/');
    if s.chars().count() > 48 { format!("{}…", s.chars().take(47).collect::<String>()) } else { s.to_string() }
}

/// A login handle ("uds2027", "j.renz") is no name to greet someone with.
fn looks_like_a_name(word: &str) -> bool {
    let mut letters = 0;
    for c in word.chars() {
        if c.is_alphabetic() {
            letters += 1;
        } else if c != '-' && c != '\'' {
            return false;
        }
    }
    letters >= 2
}

/// The project of a chat as a small tinted chip with a folder icon.
fn project_chip(ui: &mut Ui, t: &crate::theme::Tokens, name: &str) {
    let g = crate::kit::line(ui, name, crate::kit::font(12.5, crate::theme::SEMIBOLD), t.accent_ink, 200.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(g.size().x + 7.0 + 13.0 + 5.0 + 9.0, 20.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(10), t.accent_soft);
    crate::icons::Icon::Folder.paint(p, egui::pos2(rect.left() + 7.0 + 6.5, rect.center().y), 13.0, t.accent_ink);
    p.galley(egui::pos2(rect.left() + 25.0, rect.center().y - g.size().y / 2.0), g, t.accent_ink);
}

/// The model picker's chip: the AIS.chat logo in a white dot, the model's name, a chevron.
fn model_chip(ui: &mut Ui, t: &crate::theme::Tokens, model: &str) -> egui::Response {
    let g = crate::kit::line(ui, model, crate::kit::font(13.5, crate::theme::SEMIBOLD), t.chip_text, 220.0);
    let w = 8.0 + 20.0 + 7.0 + g.size().x + 7.0 + 15.0 + 10.0;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 32.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, model));
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(16), t.chip);
    if r.hovered() || r.has_focus() {
        p.rect_filled(rect, CornerRadius::same(16), t.text.gamma_multiply(0.06));
    }
    let dot = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), egui::vec2(20.0, 20.0));
    p.circle_filled(dot.center(), 10.0, egui::Color32::WHITE);
    crate::kit::paint_logo(p, egui::Rect::from_center_size(dot.center(), egui::vec2(10.5, 9.0)), egui::Color32::from_rgb(0x10, 0x7f, 0xad));
    let gw = g.size().x;
    p.galley(egui::pos2(dot.right() + 7.0, rect.center().y - g.size().y / 2.0), g, t.chip_text);
    crate::icons::Icon::ChevronDown.paint(p, egui::pos2(dot.right() + 7.0 + gw + 7.0 + 7.5, rect.center().y), 15.0, t.chip_text);
    r
}

/// A suggestion under the composer: readable text on a quiet, outlined pill.
fn suggestion_chip<'a>(t: &crate::theme::Tokens, text: &str) -> egui::Button<'a> {
    egui::Button::new(RichText::new(text).size(13.0).color(t.text))
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(t.radius))
        .min_size(egui::vec2(0.0, 30.0))
}

/// The file name to suggest when saving: with the badge the picture is a PNG, unless it was a
/// JPEG (see `ai_badge::bake`).
fn badged_name(name: &str, badge: bool) -> String {
    let lower = name.to_lowercase();
    if !badge || lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        return name.to_string();
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    format!("{stem}.png")
}

/// An action under a message: symbol and word, framed only on hover.
fn message_action(ui: &mut Ui, t: &crate::theme::Tokens, icon: &str, label: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(format!("{icon} {label}")).small().color(t.muted)).frame_when_inactive(false).min_size(egui::vec2(0.0, 24.0)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn greets_names_not_login_handles() {
        assert!(super::looks_like_a_name("Jan"));
        assert!(super::looks_like_a_name("Anne-Marie"));
        assert!(super::looks_like_a_name("Jürgen"));
        assert!(!super::looks_like_a_name("uds2027"));
        assert!(!super::looks_like_a_name("j.renz"));
        assert!(!super::looks_like_a_name("x"));
    }
}

#[cfg(test)]
mod context_window_tests {
    use super::known_context_window;

    #[test]
    fn known_model_families_have_a_window() {
        assert_eq!(known_context_window("claude-sonnet-5-5"), Some(200_000));
        assert_eq!(known_context_window("GPT-4.1-mini"), Some(1_000_000));
        assert_eq!(known_context_window("gpt-4o"), Some(128_000));
        assert_eq!(known_context_window("models/gemini-2.5-pro"), Some(1_000_000));
        assert_eq!(known_context_window("etwas-ganz-anderes"), None);
        assert_eq!(known_context_window(""), None);
    }
}
