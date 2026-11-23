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
        egui::Panel::bottom("composer-panel").frame(Frame::new().inner_margin(Margin { left: 0, right: 0, top: 6, bottom: 10 })).show(ui, |ui| {
            centered(ui, |ui| self.composer(ui));
        });
        ScrollArea::vertical().id_salt(("messages", self.state.current.clone())).stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            centered(ui, |ui| {
                ui.add_space(12.0);
                let msgs = self.state.messages.clone();
                let last_assistant = msgs.iter().rposition(|m| m.role == Role::Assistant);
                for (i, m) in msgs.iter().enumerate() {
                    match m.role {
                        Role::User => self.user_message(ui, m),
                        // Tool results are shown as chips on the call, not as messages.
                        Role::Tool => continue,
                        _ => self.assistant_message(ui, m, Some(i) == last_assistant),
                    }
                    ui.add_space(14.0);
                }
                self.approval_cards(ui);
                ui.add_space(20.0);
            });
        });
    }

    fn header(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if !self.sidebar_open && icon_button(ui, &t, "☰", "Seitenleiste (Strg+B)").clicked() {
                    self.sidebar_open = true;
                }
                let title = self.state.current.as_ref().and_then(|id| self.state.conversation(id)).map(|c| c.title.clone());
                if let Some(title) = &title {
                    let r = ui.add(egui::Label::new(RichText::new(title).strong()).truncate().sense(egui::Sense::click())).on_hover_text("Doppelklick zum Umbenennen");
                    if r.double_clicked()
                        && let Some(id) = self.state.current.clone()
                    {
                        self.dialog = Some(Dialog::Rename { id, title: title.clone() });
                    }
                }
                if let Some(p) = self.state.selection.project_id.as_ref().and_then(|pid| self.state.projects.iter().find(|p| &p.id == pid)) {
                    ui.label(small_muted(&t, format!("▣ {}", p.name)));
                }
                if let Some(p) = self.state.selection.persona.as_ref().filter(|_| self.state.current.is_some()) {
                    ui.label(small_muted(&t, format!("{}: {}", p.kind.label(), p.name)));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.state.current.is_some() {
                        if self.state.selection.kind == aiscraft_core::ConversationKind::Chat {
                            self.context_meter(ui);
                        }
                        if icon_button(ui, &t, "⬇", "Exportieren (Strg+Umschalt+E)").clicked() {
                            self.export_dialog();
                        }
                        if let Some(url) = self.state.current.as_ref().and_then(|id| {
                            let c = self.state.conversation(id)?;
                            let b = self.state.backend(&c.backend_id).filter(|b| b.config.kind == BackendKind::AisChat)?;
                            let path = if c.kind == aiscraft_core::ConversationKind::Image {
                                format!("/image-generation/d/{}", c.remote_id.as_deref().unwrap_or(&c.id))
                            } else {
                                aiscraft_core::ais_conversation_path(c.persona.as_ref(), &c.id)
                            };
                            Some(format!("{}{path}", b.config.base_url.trim_end_matches('/')))
                        }) && icon_button(ui, &t, "↗", "In AIS.chat öffnen").clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                    }
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
            None => "Backend wählen".into(),
        };
        ui.menu_button(RichText::new(format!("{label}  ⏷")).color(t.muted), |ui| {
            ui.set_min_width(260.0);
            for b in self.state.backends.clone() {
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
                                self.run("chat.select_model", json!({"backend": b.config.id, "model": m.id}));
                                ui.close();
                            }
                        }
                        if !b.image_models.is_empty() {
                            ui.label(small_muted(&t, "Bild erzeugen"));
                            for m in &b.image_models {
                                let active = sel.backend_id == b.config.id && sel.model_id == m.id && sel.kind == aiscraft_core::ConversationKind::Image;
                                let r = ui.add(egui::Button::selectable(active, m.name.as_str())).on_hover_text("Neuer Bild-Chat mit diesem Modell");
                                if r.clicked() {
                                    self.run("chat.new_image", json!({"backend": b.config.id, "model": m.id}));
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
                            self.dialog = Some(Dialog::login(&b.config.id));
                            ui.close();
                        }
                    }
                }
                ui.add_space(4.0);
            }
            ui.separator();
            if ui.button("+ Backend hinzufügen …").clicked() {
                self.dialog = Some(Dialog::add_backend());
                ui.close();
            }
        });
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
                    if ui.add(egui::Button::new(small_muted(&t, &s)).corner_radius(CornerRadius::same(t.radius))).clicked() {
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
        let first = name.split_whitespace().next().filter(|n| !n.is_empty()).map(|n| {
            let mut c = n.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        });
        let any_ready = self.state.backends.iter().any(|b| matches!(b.status, BackendStatus::Ready { .. }));
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
                    ui.label(muted(&t, "Melde dich bei AIS.chat an oder füge ein anderes Backend hinzu (OpenAI, Claude, Ollama, LM Studio …)."));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if let Some(ais) = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat)
                            && accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
                        {
                            self.dialog = Some(Dialog::login(&ais.config.id));
                        }
                        if ui.button("Backend hinzufügen").clicked() {
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
                    &["Stundenentwurf für Klasse 7 zum Thema Photosynthese", "Erkläre Bruchrechnung mit Pizzastücken", "Formuliere eine freundliche Elternmail", "Erstelle ein Quiz mit 5 Fragen"]
                };
                for s in ideas.iter().copied() {
                    if ui.add(egui::Button::new(small_muted(&t, s)).corner_radius(CornerRadius::same(t.radius))).clicked() {
                        self.composer = s.to_string();
                        self.focus_composer = true;
                    }
                }
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
                    ui.label(small_muted(&t, "Alles danach wird verworfen und neu erzeugt."));
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
            let max_w = (ui.available_width() * 0.8).min(600.0);
            let r = Frame::new().fill(t.user_bubble).corner_radius(CornerRadius::same(t.radius.saturating_add(4))).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_max_width(max_w);
                if !m.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for a in &m.attachments {
                            ui.label(small_muted(&t, format!("📎 {}", a.name)));
                        }
                    });
                }
                if !m.content.is_empty() {
                    ui.add(egui::Label::new(&m.content).wrap().selectable(true));
                }
            });
            let hovered = ui.rect_contains_pointer(r.response.rect.expand(24.0));
            ui.horizontal(|ui| {
                ui.set_min_height(18.0);
                if hovered && !self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c)) {
                    if icon_button(ui, &t, "✏", "Bearbeiten").clicked() {
                        self.editing = Some((m.id.clone(), m.content.clone()));
                    }
                    if ui.add(egui::Button::new(small_muted(&t, "Kopieren")).frame(false)).clicked() {
                        ui.ctx().copy_text(m.content.clone());
                    }
                }
            });
        });
    }

    fn assistant_message(&mut self, ui: &mut Ui, m: &Message, is_last: bool) {
        let t = self.tokens.clone();
        let generating = is_last && self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        if let Some(model) = &m.model {
            ui.label(small_muted(&t, model));
        }
        if !m.activity.is_empty() {
            let open_default = generating;
            egui::CollapsingHeader::new(small_muted(&t, format!("Arbeitsschritte ({})", m.activity.len()))).id_salt(("act", &m.id)).default_open(open_default).show(ui, |ui| {
                for (i, s) in m.activity.iter().enumerate() {
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
                    for l in &s.links {
                        ui.hyperlink_to(RichText::new(format!("   ↗ {}", l.title)).small(), &l.url);
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
                        Some(_) => ("×", t.danger),
                        None => ("…", t.muted),
                    };
                    let label = self.tool_label(&tc.name);
                    egui::Frame::new().fill(t.surface_hi).corner_radius(egui::CornerRadius::same(6)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
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
        if m.content.is_empty() && !m.tool_calls.is_empty() {
            return;
        }
        let images: Vec<(usize, aiscraft_core::Attachment)> = m.attachments.iter().filter(|a| a.mime.starts_with("image/")).cloned().enumerate().collect();
        for (i, a) in &images {
            let tex = a.local_path.as_deref().and_then(|p| self.image_texture(ui.ctx(), p));
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
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(small_muted(&t, "Bild speichern …")).frame(false)).clicked()
                    && let Some(path) = rfd::FileDialog::new().set_file_name(&a.name).save_file()
                {
                    self.run("chat.save_image", json!({"message_id": m.id, "index": i, "path": path.display().to_string()}));
                }
                if is_last && icon_button(ui, &t, "⟳", "Neu erzeugen (Strg+R)").clicked() {
                    self.run("chat.regenerate", json!({}));
                }
            });
        }
        if m.content.is_empty() && generating {
            let image_chat = self.state.current.as_ref().and_then(|c| self.state.conversation(c)).is_some_and(|c| c.kind == aiscraft_core::ConversationKind::Image);
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(t.accent));
                ui.label(muted(&t, if image_chat { "Bild wird gezeichnet …" } else { "Denkt nach …" }));
            });
        } else if !m.content.is_empty() || images.is_empty() {
            egui_commonmark::CommonMarkViewer::new().default_width(Some(CONTENT_WIDTH as usize)).show(ui, &mut self.md, &m.content);
        }
        if let Some(err) = &m.error {
            Frame::new().fill(t.danger.gamma_multiply(0.12)).stroke(Stroke::new(1.0, t.danger.gamma_multiply(0.5))).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::same(10)).show(ui, |ui| {
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
                let target = self.state.open_artifact.clone().filter(|id| self.state.artifacts.iter().any(|a| &a.id == id && a.kind == kind));
                ui.horizontal(|ui| match (kind, target) {
                    (aiscraft_core::ArtifactKind::Document, Some(id)) => {
                        if accent_button(ui, &t, "Ins Dokument übernehmen").on_hover_text("Ersetzt den Inhalt; die vorige Fassung bleibt unter Versionen").clicked() {
                            self.run("artifact.save", json!({"id": id, "text": content, "version": "Aus einer Antwort übernommen"}));
                        }
                    }
                    (aiscraft_core::ArtifactKind::Board, Some(id)) => {
                        if accent_button(ui, &t, "Aufs Whiteboard").on_hover_text("Ergänzt das geöffnete Whiteboard um diese Punkte").clicked() {
                            self.run("artifact.board_add", json!({"id": id, "outline": content}));
                        }
                    }
                    (kind, None) => {
                        let label = if kind == aiscraft_core::ArtifactKind::Board { "Als neues Whiteboard öffnen" } else { "Als neues Dokument öffnen" };
                        if ui.button(label).clicked() {
                            self.run("artifact.create", json!({"kind": kind.as_str(), "content": content}));
                        }
                    }
                });
            }
        }
        if !generating && !m.content.is_empty() {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(small_muted(&t, "Kopieren")).frame(false)).clicked() {
                    ui.ctx().copy_text(m.content.clone());
                }
                if ui.add(egui::Button::new(small_muted(&t, "Vorlesen")).frame(false)).on_hover_text("Antwort vorlesen (AIS.chat oder OpenAI-kompatibles Backend)").clicked() {
                    self.run("voice.speak", json!({"message_id": m.id}));
                }
                if ui.add(egui::Button::new(small_muted(&t, "Als Dokument")).frame(false)).on_hover_text("Antwort neben dem Chat als bearbeitbares Dokument öffnen").clicked() {
                    self.run("artifact.create", json!({"kind": "document", "from_message": m.id}));
                }
                if ui.add(egui::Button::new(small_muted(&t, "Als Whiteboard")).frame(false)).on_hover_text("Gliederung der Antwort als Mindmap öffnen").clicked() {
                    self.run("artifact.create", json!({"kind": "board", "from_message": m.id}));
                }
                if is_last && icon_button(ui, &t, "⟳", "Neu erzeugen (Strg+R)").clicked() {
                    self.run("chat.regenerate", json!({}));
                }
                if let Some(u) = m.usage
                    && u.output_tokens > 0
                {
                    ui.label(small_muted(&t, format!("{} ➡ {} Tokens", u.input_tokens, u.output_tokens)));
                }
            });
        }
    }

    /// How full the model's context window is: counted by the server where it reports usage,
    /// estimated from the text otherwise.
    fn context_meter(&self, ui: &mut Ui) {
        let t = &self.tokens;
        let sel = &self.state.selection;
        let window = self.state.backend(&sel.backend_id).and_then(|b| b.models.iter().find(|m| m.id == sel.model_id)).and_then(|m| m.context_window);
        let counted = self.state.messages.iter().rev().find(|m| m.role == Role::Assistant && m.usage.is_some_and(|u| u.input_tokens > 0)).and_then(|m| m.usage).map(|u| u.input_tokens + u.output_tokens);
        let (used, estimated) = match counted {
            Some(n) => (n, false),
            None => {
                let chars: usize = self.state.messages.iter().map(|m| m.content.chars().count() + m.attachments.iter().filter_map(|a| a.text.as_ref()).map(|x| x.chars().count()).sum::<usize>()).sum();
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
            "Kontext geschätzt aus der Textlänge (dieses Backend meldet keine Token-Zahlen)".to_string()
        } else {
            "Kontext laut Server: Eingabe der letzten Antwort plus Antwort, also ungefähr das, was die nächste Nachricht mitschickt".to_string()
        };
        match window {
            Some(w) if w > 0 => {
                let frac = (used as f32 / w as f32).clamp(0.0, 1.0);
                let color = if frac > 0.9 {
                    t.danger
                } else if frac > 0.7 {
                    t.warning
                } else {
                    t.accent
                };
                let r = ui.label(small_muted(t, format!("{prefix}{} / {}", fmt(used), fmt(w))));
                let (rect, bar) = ui.allocate_exact_size(egui::vec2(64.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(3), t.surface_hi);
                let mut fill = rect;
                fill.set_width((rect.width() * frac).max(2.0));
                ui.painter().rect_filled(fill, CornerRadius::same(3), color);
                let tip = format!("{tip}. {:.0} % von {} Tokens belegt.", frac * 100.0, fmt(w));
                r.on_hover_text(&tip);
                bar.on_hover_text(tip);
            }
            _ => {
                ui.label(small_muted(t, format!("Kontext {prefix}{}", fmt(used)))).on_hover_text(format!("{tip}. Die Größe des Kontextfensters meldet dieses Backend nicht."));
            }
        }
    }

    /// What this chat may draw on: tools, memory, project instructions, history, each connector.
    fn sources_menu(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let src = self.state.selection.sources.clone();
        let caps = self.state.backend(&self.state.selection.backend_id).map(|b| b.capabilities).unwrap_or_default();
        let on = [src.tools, src.memory, src.project, src.history].iter().filter(|x| **x).count();
        let label = if on == 4 && src.connectors.is_none() { "Quellen".to_string() } else { format!("Quellen ({on}/4)") };
        // Stays open while switching; closes with its button, the toggle button or Esc.
        let toggle = ui.button(RichText::new(format!("{label}  ⏷")).color(t.muted));
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
            let mut memory = src.memory;
            if ui.checkbox(&mut memory, "Gedächtnis (deine Notizen)").changed() {
                set("memory", memory);
            }
            let mut project = src.project;
            if ui.checkbox(&mut project, "Projektanweisungen").changed() {
                set("project", project);
            }
            let mut tools = src.tools;
            if ui.checkbox(&mut tools, "Werkzeuge (Dokumente, Whiteboards, Zeitpläne)").changed() {
                set("tools", tools);
            }
            let mut history = src.history;
            if ui.add_enabled(src.tools, egui::Checkbox::new(&mut history, "Andere Chats durchsuchen")).changed() {
                set("history", history);
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
                for tool in st {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if tool.enabled { "✔" } else { "×" }).color(if tool.enabled { t.success } else { t.muted }));
                        ui.label(if tool.enabled { RichText::new(&tool.label) } else { RichText::new(&tool.label).color(t.muted) });
                    });
                }
            }
            if !caps.tools || !caps.system_prompt {
                ui.separator();
                let name = self.state.backend(&self.state.selection.backend_id).map(|b| b.config.name.clone()).unwrap_or_default();
                let mut missing = Vec::new();
                if !caps.system_prompt {
                    missing.push("Gedächtnis und Projektanweisungen");
                }
                if !caps.tools {
                    missing.push("Werkzeuge und Konnektoren");
                }
                ui.label(small_muted(&t, format!("{name} nimmt {} nicht an.", missing.join(" und "))));
            }
        });
    }

    /// A readable name for a tool the model called.
    fn tool_label(&self, name: &str) -> String {
        match name {
            "history_search" => "Verlauf durchsucht".into(),
            "document_create" => "Dokument angelegt".into(),
            "document_update" => "Dokument geändert".into(),
            "board_create" => "Whiteboard angelegt".into(),
            "task_start" => "Aufgabe gestartet".into(),
            "schedule_create" => "Nachfrage geplant".into(),
            other => self
                .state
                .connectors
                .iter()
                .find_map(|c| c.tools.iter().find(|t| t.llm_name == other).map(|t| format!("{} · {}", c.name, if t.title.is_empty() { &t.name } else { &t.title })))
                .unwrap_or_else(|| other.to_string()),
        }
    }

    /// Questions from the tool rule: allow this exact call once, or not.
    fn approval_cards(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(conv) = self.state.current.clone() else { return };
        for a in self.state.approvals.iter().filter(|a| a.conversation_id == conv).cloned().collect::<Vec<_>>() {
            Frame::new()
                .fill(t.surface)
                .stroke(Stroke::new(1.5, if a.destructive { t.danger } else { t.accent }))
                .corner_radius(CornerRadius::same(t.radius))
                .inner_margin(Margin::same(14))
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("Darf ich das ausführen? {}", a.label)).strong());
                    if a.destructive {
                        ui.label(RichText::new("Der Konnektor kennzeichnet das als verändernd oder löschend.").color(t.danger).small());
                    }
                    let args = serde_json::to_string_pretty(&a.arguments).unwrap_or_default();
                    Frame::new().fill(t.code_bg).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(8)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(args.chars().take(4000).collect::<String>()).monospace().small()).wrap());
                    });
                    if let Some(tr) = &a.trust {
                        if tr.covered {
                            ui.label(RichText::new(format!("Deine Worte decken das: „{}“", tr.quote)).color(t.success).small());
                        } else {
                            ui.label(RichText::new(format!("Nicht von deinen Worten gedeckt: {}", tr.reason)).color(t.warning).small());
                        }
                    }
                    ui.horizontal(|ui| {
                        if accent_button(ui, &t, "Einmal erlauben").clicked() {
                            self.run("tool.answer", json!({"id": a.id, "allow": true}));
                        }
                        if ui.button("Ablehnen").clicked() {
                            self.run("tool.answer", json!({"id": a.id, "allow": false}));
                        }
                        ui.label(small_muted(&t, "Gilt nur für genau diese Angaben."));
                    });
                });
            ui.add_space(8.0);
        }
    }

    pub(crate) fn composer(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let generating = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        let enter_sends = self.state.settings.enter_sends;
        let image_mode = self.state.current.as_ref().and_then(|c| self.state.conversation(c)).map(|c| c.kind).unwrap_or(self.state.selection.kind) == aiscraft_core::ConversationKind::Image;
        let mut send = false;
        Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(t.radius.saturating_add(6)))
            .inner_margin(Margin::symmetric(14, 10))
            .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(if t.dark { 60 } else { 18 }) })
            .show(ui, |ui| {
                if !self.pending_files.is_empty() {
                    let mut remove = None;
                    ui.horizontal_wrapped(|ui| {
                        for (i, f) in self.pending_files.iter().enumerate() {
                            let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                            Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(6)).inner_margin(Margin::symmetric(8, 3)).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("📎 {name}")).small());
                                    if icon_button(ui, &t, "×", "Entfernen").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                        }
                    });
                    if let Some(i) = remove {
                        self.pending_files.remove(i);
                    }
                }
                if self.attach_artifact
                    && let Some(a) = self.state.open_artifact.as_ref().and_then(|id| self.state.artifacts.iter().find(|a| &a.id == id))
                {
                    ui.label(small_muted(&t, format!("📄 {} geht als Kontext mit", a.title)));
                }
                let has_focus = ui.memory(|m| m.focused()) == Some(egui::Id::unique("composer"));
                if enter_sends && has_focus && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                    send = true;
                }
                if has_focus && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
                    send = true;
                }
                let newline = if enter_sends { KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter) } else { KeyboardShortcut::new(Modifiers::NONE, Key::Enter) };
                let edit = egui::TextEdit::multiline(&mut self.composer)
                    .id(egui::Id::unique("composer"))
                    .hint_text(if image_mode {
                        "Beschreibe das Bild, das entstehen soll …"
                    } else if enter_sends {
                        "Schreib eine Nachricht … (Umschalt+Enter für neue Zeile)"
                    } else {
                        "Schreib eine Nachricht … (Strg+Enter sendet)"
                    })
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(Frame::NONE)
                    .return_key(newline);
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
                ui.horizontal(|ui| {
                    if self.state.recording {
                        let level = self.engine.mic_level();
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 10.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, CornerRadius::same(4), t.surface_hi);
                        let mut fill = rect;
                        fill.set_width((rect.width() * (level * 3.0).clamp(0.05, 1.0)).max(2.0));
                        ui.painter().rect_filled(fill, CornerRadius::same(4), t.danger);
                        if ui.add(egui::Button::new(RichText::new("■ Diktat beenden").color(t.danger))).clicked() {
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
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                    } else if icon_button(ui, &t, "🎤", "Diktieren").clicked() {
                        self.run("voice.record_start", json!({}));
                    }
                    if !image_mode
                        && icon_button(ui, &t, "📎", "Dateien anhängen (oder hineinziehen)").clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        self.pending_files.extend(files);
                    }
                    self.model_picker(ui);
                    if image_mode {
                        let cur = self.state.selection.aspect_ratio.clone();
                        let label = aiscraft_core::ASPECT_RATIOS.iter().find(|(k, _)| *k == cur).map(|(_, l)| *l).unwrap_or("Quadratisch");
                        ui.menu_button(RichText::new(format!("{label}  ⏷")).color(t.muted), |ui| {
                            for (k, l) in aiscraft_core::ASPECT_RATIOS {
                                if ui.add(egui::Button::selectable(k == cur || (cur.is_empty() && k == "quadratic"), l)).clicked() {
                                    self.run("chat.set_aspect", json!({"ratio": k}));
                                    ui.close();
                                }
                            }
                        });
                    } else {
                        self.sources_menu(ui);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if generating {
                            if accent_button(ui, &t, "■").on_hover_text("Anhalten (Esc)").clicked() {
                                self.run("chat.stop", json!({}));
                            }
                        } else {
                            let ok = !self.composer.trim().is_empty() || !self.pending_files.is_empty();
                            let b = ui.add_enabled(ok, egui::Button::new(RichText::new("⬆").strong().color(t.on_accent)).fill(t.accent).corner_radius(CornerRadius::same(t.radius)));
                            if b.on_hover_text("Senden").clicked() {
                                send = true;
                            }
                        }
                    });
                });
            });
        self.context_hint(ui);
        if send && !generating {
            let text = std::mem::take(&mut self.composer);
            let files: Vec<String> = self.pending_files.drain(..).map(|p| p.display().to_string()).collect();
            if !text.trim().is_empty() || !files.is_empty() {
                self.save_if_dirty();
                let artifact = if self.attach_artifact { self.state.open_artifact.clone() } else { None };
                self.run("chat.send", json!({"text": text, "files": files, "artifact": artifact}));
            }
            self.focus_composer = true;
        }
    }

    /// Say when project instructions or notes are not sent, so nobody relies on them silently.
    fn context_hint(&self, ui: &mut Ui) {
        let t = &self.tokens;
        let Some(b) = self.state.backend(&self.state.selection.backend_id) else { return };
        let has_project = self.state.selection.project_id.as_ref().and_then(|p| self.state.projects.iter().find(|x| &x.id == p)).is_some_and(|p| !p.instructions.trim().is_empty());
        let has_memory = self.state.settings.memory && self.state.memory.iter().any(|n| !n.archived);
        let sends = b.config.send_context && b.capabilities.system_prompt;
        ui.add_space(4.0);
        ui.vertical_centered(|ui| {
            if (has_project || has_memory) && !sends {
                ui.label(small_muted(t, format!("Projektanweisungen und Notizen gehen nicht an {}.", b.config.name)));
            } else {
                ui.label(small_muted(t, "KI kann Fehler machen. Prüfe wichtige Angaben."));
            }
        });
    }
}

/// Lay out `add` in a centred column of at most [`CONTENT_WIDTH`].
pub(crate) fn centered<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = avail.min(CONTENT_WIDTH);
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
