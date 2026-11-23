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
                        _ => self.assistant_message(ui, m, Some(i) == last_assistant),
                    }
                    ui.add_space(14.0);
                }
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
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.state.current.is_some() {
                        if icon_button(ui, &t, "⬇", "Exportieren (Strg+Umschalt+E)").clicked() {
                            self.export_dialog();
                        }
                        if let Some(url) = self.state.current.as_ref().and_then(|id| {
                            let c = self.state.conversation(id)?;
                            let b = self.state.backend(&c.backend_id).filter(|b| b.config.kind == BackendKind::AisChat)?;
                            Some(format!("{}/d/{}", b.config.base_url.trim_end_matches('/'), c.id))
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
                            let active = sel.backend_id == b.config.id && sel.model_id == m.id;
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

    fn empty_state(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let name = self.state.backends.iter().find_map(|b| match &b.status {
            BackendStatus::Ready { account } => account.display_name.clone(),
            _ => None,
        });
        let first = name.as_deref().and_then(|n| n.split_whitespace().next()).map(str::to_string);
        let any_ready = self.state.backends.iter().any(|b| matches!(b.status, BackendStatus::Ready { .. }));
        ui.add_space((ui.available_height() * 0.22).max(20.0));
        centered(ui, |ui| {
            ui.vertical_centered(|ui| {
                crate::widgets::ais_logo(ui, 46.0, t.accent);
                ui.add_space(6.0);
                let greeting = match first {
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
                for s in ["Stundenentwurf für Klasse 7 zum Thema Photosynthese", "Erkläre Bruchrechnung mit Pizzastücken", "Formuliere eine freundliche Elternmail", "Erstelle ein Quiz mit 5 Fragen"] {
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
        if m.content.is_empty() && generating {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(t.accent));
                ui.label(muted(&t, "Denkt nach …"));
            });
        } else {
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
        if !generating && !m.content.is_empty() {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(small_muted(&t, "Kopieren")).frame(false)).clicked() {
                    ui.ctx().copy_text(m.content.clone());
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

    pub(crate) fn composer(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let generating = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        let enter_sends = self.state.settings.enter_sends;
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
                    .hint_text(if enter_sends { "Schreib eine Nachricht … (Umschalt+Enter für neue Zeile)" } else { "Schreib eine Nachricht … (Strg+Enter sendet)" })
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(Frame::NONE)
                    .return_key(newline);
                let r = ScrollArea::vertical().id_salt("composer-scroll").max_height(220.0).show(ui, |ui| ui.add(edit)).inner;
                if self.focus_composer && self.dialog.is_none() && self.palette.is_none() {
                    r.request_focus();
                    self.focus_composer = false;
                }
                ui.horizontal(|ui| {
                    if icon_button(ui, &t, "📎", "Dateien anhängen (oder hineinziehen)").clicked()
                        && let Some(files) = rfd::FileDialog::new().pick_files()
                    {
                        self.pending_files.extend(files);
                    }
                    self.model_picker(ui);
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
                self.run("chat.send", json!({"text": text, "files": files}));
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
