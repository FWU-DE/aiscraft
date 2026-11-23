use aiscraft_core::{BackendKind, MemoryArea, Project};
use aiscraft_engine::BackendStatus;
use egui::{Id, RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, card, muted, small_muted, status_dot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Backends,
    Memory,
    Appearance,
    About,
}

#[derive(Debug, Clone)]
pub enum Dialog {
    Welcome,
    Login { backend_id: String, secret: String, sent: bool },
    AddBackend { preset: usize, name: String, base_url: String, secret: String },
    Settings { tab: SettingsTab, new_note: String },
    Project { id: Option<String>, name: String, instructions: String },
    Rename { id: String, title: String },
    Confirm { text: String, command: String, params: Value },
}

/// Ready-made backends: (label, kind, base URL, needs a key, hint).
pub const PRESETS: &[(&str, BackendKind, &str, bool, &str)] = &[
    ("Claude (Abo)", BackendKind::ClaudeCli, "claude", false, "Nutzt dein claude.ai-Abo über die lokale Claude-CLI (vorher `claude auth login`). Ohne Werkzeuge, nur Chat."),
    ("Claude (API)", BackendKind::Anthropic, "https://api.anthropic.com", true, "API-Schlüssel von console.anthropic.com, Abrechnung nach Verbrauch"),
    ("OpenAI", BackendKind::OpenAi, "https://api.openai.com/v1", true, "API-Schlüssel von platform.openai.com"),
    ("AIS.chat API", BackendKind::OpenAi, "https://api.ais-chat.schule/v1", true, "API-Schlüssel aus dem AIS.chat-Admin; eigener Verlauf, getrennt von der Web-App"),
    ("Ollama (lokal)", BackendKind::OpenAi, "http://127.0.0.1:11434/v1", false, "Läuft auf diesem Rechner, kein Schlüssel nötig"),
    ("LM Studio (lokal)", BackendKind::OpenAi, "http://127.0.0.1:1234/v1", false, "Läuft auf diesem Rechner, kein Schlüssel nötig"),
    ("OpenRouter", BackendKind::OpenAi, "https://openrouter.ai/api/v1", true, "Ein Schlüssel für viele Modelle"),
    ("AIS.chat (weitere Instanz)", BackendKind::AisChat, "https://app.ais-chat.schule", true, "Anmeldung über die Web-Sitzung"),
    ("Eigener OpenAI-kompatibler Server", BackendKind::OpenAi, "https://", false, "vLLM, LiteLLM, LocalAI …"),
];

fn kind_str(k: BackendKind) -> &'static str {
    match k {
        BackendKind::AisChat => "ais_chat",
        BackendKind::OpenAi => "openai",
        BackendKind::Anthropic => "anthropic",
        BackendKind::ClaudeCli => "claude_cli",
    }
}

impl Dialog {
    pub fn settings() -> Self {
        Dialog::Settings { tab: SettingsTab::Backends, new_note: String::new() }
    }
    pub fn login(id: &str) -> Self {
        Dialog::Login { backend_id: id.to_string(), secret: String::new(), sent: false }
    }
    pub fn add_backend() -> Self {
        let (label, _, url, _, _) = PRESETS[0];
        Dialog::AddBackend { preset: 0, name: label.into(), base_url: url.into(), secret: String::new() }
    }
    pub fn project(p: Option<&Project>) -> Self {
        Dialog::Project { id: p.map(|p| p.id.clone()), name: p.map(|p| p.name.clone()).unwrap_or_default(), instructions: p.map(|p| p.instructions.clone()).unwrap_or_default() }
    }
}

impl AisCraftApp {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let modal = egui::Modal::new(Id::unique("dialog")).show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::Settings { .. } => 620.0,
                Dialog::Login { .. } | Dialog::Welcome => 520.0,
                _ => 460.0,
            });
            keep = self.dialog_body(ui, &mut dialog);
        });
        if modal.should_close() {
            keep = false;
        }
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    /// Returns whether the dialog stays open.
    fn dialog_body(&mut self, ui: &mut Ui, dialog: &mut Dialog) -> bool {
        let t = self.tokens.clone();
        match dialog {
            Dialog::Welcome => {
                ui.heading("Willkommen bei AISCraft");
                ui.add_space(4.0);
                ui.label(muted(&t, "Ein Chat-Programm für AIS.chat und andere KI-Dienste. Deine Chats liegen auf diesem Rechner und sind durchsuchbar; Projekte bündeln Chats mit eigenen Anweisungen; mit „Merke dir: …“ legst du Notizen an, die nur aus deinen eigenen Worten stammen."));
                ui.add_space(12.0);
                let ais = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat).map(|b| b.config.id.clone());
                let mut next = None;
                let mut keep = true;
                ui.horizontal(|ui| {
                    if let Some(id) = &ais
                        && accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
                    {
                        next = Some(Dialog::login(id));
                    }
                    if ui.button("Anderes Backend").clicked() {
                        next = Some(Dialog::add_backend());
                    }
                    if ui.button("Später").clicked() {
                        keep = false;
                    }
                });
                if let Some(n) = next {
                    *dialog = n;
                }
                keep
            }
            Dialog::Login { backend_id, secret, sent } => self.login_body(ui, backend_id, secret, sent),
            Dialog::AddBackend { preset, name, base_url, secret } => {
                ui.heading("Backend hinzufügen");
                ui.add_space(6.0);
                egui::ComboBox::from_id_salt("preset").width(ui.available_width()).selected_text(PRESETS.get(*preset).map(|p| p.0).unwrap_or("")).show_ui(ui, |ui| {
                    for (i, p) in PRESETS.iter().enumerate() {
                        if ui.selectable_label(*preset == i, p.0).clicked() {
                            *preset = i;
                            *name = p.0.trim_end_matches(" (lokal)").to_string();
                            *base_url = p.2.to_string();
                        }
                    }
                });
                let Some(&(_, kind, _, needs_key, hint)) = PRESETS.get(*preset) else { return false };
                ui.label(small_muted(&t, hint));
                ui.add_space(6.0);
                egui::Grid::new("add-backend").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(name).desired_width(300.0));
                    ui.end_row();
                    ui.label(if kind == BackendKind::ClaudeCli { "Programm" } else { "Adresse" });
                    ui.add(egui::TextEdit::singleline(base_url).desired_width(300.0));
                    ui.end_row();
                    if !matches!(kind, BackendKind::AisChat | BackendKind::ClaudeCli) {
                        ui.label(if needs_key { "API-Schlüssel" } else { "Schlüssel (optional)" });
                        ui.add(egui::TextEdit::singleline(secret).password(true).desired_width(300.0));
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    let ok = !base_url.trim().is_empty() && (!needs_key || !secret.trim().is_empty() || kind == BackendKind::AisChat);
                    if ui.add_enabled(ok, egui::Button::new("Hinzufügen")).clicked() {
                        self.run("backend.add", json!({"kind": kind_str(kind), "name": name, "base_url": base_url, "secret": secret}));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                ui.label(small_muted(&t, "Schlüssel liegen im Schlüsselbund des Systems, nie in der Datenbank."));
                keep
            }
            Dialog::Settings { tab, new_note } => self.settings_body(ui, tab, new_note),
            Dialog::Project { id, name, instructions } => {
                ui.heading(if id.is_some() { "Projekt bearbeiten" } else { "Neues Projekt" });
                ui.add_space(6.0);
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(name).hint_text("z. B. Biologie 7b").desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label("Anweisungen für jeden Chat im Projekt");
                ui.add(
                    egui::TextEdit::multiline(instructions)
                        .hint_text("z. B. Antworte für Siebtklässler, kurz und mit Beispielen aus dem Alltag.")
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
                ui.label(small_muted(&t, "Gehen an Backends, die Kontext erlauben (nicht an AIS.chat über die Anmeldung)."));
                ui.add_space(8.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Speichern")).clicked() {
                        match id {
                            Some(id) => self.run("project.update", json!({"id": id, "name": name, "instructions": instructions})),
                            None => self.run("project.create", json!({"name": name, "instructions": instructions})),
                        }
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::Rename { id, title } => {
                ui.heading("Chat umbenennen");
                let r = ui.add(egui::TextEdit::singleline(title).desired_width(f32::INFINITY));
                r.request_focus();
                let mut keep = true;
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if (ui.button("Speichern").clicked() || enter) && !title.trim().is_empty() {
                        self.run("chat.rename", json!({"id": id, "title": title}));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::Confirm { text, command, params } => {
                ui.label(RichText::new(text.as_str()).strong());
                ui.add_space(8.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Löschen").color(t.on_accent)).fill(t.danger)).clicked() {
                        self.run(command, params.clone());
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
        }
    }

    fn login_body(&mut self, ui: &mut Ui, backend_id: &mut String, secret: &mut String, sent: &mut bool) -> bool {
        let t = self.tokens.clone();
        let Some(view) = self.state.backend(backend_id).cloned() else { return false };
        if *sent && matches!(view.status, BackendStatus::Ready { .. }) {
            return false;
        }
        ui.heading(format!("Bei {} anmelden", view.config.name));
        ui.add_space(4.0);
        let mut keep = true;
        if view.config.kind == BackendKind::AisChat {
            ui.label(muted(&t, "AISCraft nutzt deine AIS.chat-Sitzung aus dem Browser. Die Anmeldung selbst (VIDIS, Schulkonto oder Microsoft 365) bleibt im Browser."));
            ui.add_space(8.0);
            card(&t).show(ui, |ui| {
                ui.label(RichText::new("1.  AIS.chat im Browser öffnen und anmelden").strong());
                if ui.button("AIS.chat öffnen ↗").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&view.config.base_url));
                }
                ui.add_space(6.0);
                ui.label(RichText::new("2.  Entwicklerwerkzeuge öffnen (F12)").strong());
                ui.label(small_muted(&t, "Chrome/Edge: Anwendung › Cookies. Firefox: Web-Speicher › Cookies."));
                ui.add_space(6.0);
                ui.label(RichText::new("3.  Diesen Cookie-Wert kopieren:").strong());
                ui.horizontal(|ui| {
                    ui.code(aiscraft_backends_cookie_name());
                    if ui.small_button("Kopieren").on_hover_text("Namen kopieren").clicked() {
                        ui.ctx().copy_text(aiscraft_backends_cookie_name().to_string());
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("4.  Hier einfügen").strong());
            });
            ui.add_space(6.0);
        } else {
            ui.label(muted(&t, "API-Schlüssel eingeben. Er liegt danach im Schlüsselbund des Systems."));
        }
        let r = ui.add(egui::TextEdit::singleline(secret).password(true).hint_text("Wert einfügen").desired_width(f32::INFINITY));
        if !*sent {
            r.request_focus();
        }
        match &view.status {
            BackendStatus::Checking => {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label("Prüfe …");
                });
            }
            BackendStatus::SignedOut { message } | BackendStatus::Failed { message } if *sent => {
                ui.label(RichText::new(message).color(t.danger));
            }
            _ => {}
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if accent_button(ui, &t, "Anmelden").clicked() && !secret.trim().is_empty() {
                self.run("backend.login", json!({"id": backend_id, "secret": secret.trim()}));
                *sent = true;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        if view.config.kind == BackendKind::AisChat {
            ui.label(small_muted(&t, "Die Sitzung gilt 8 Stunden und verlängert sich, solange du AISCraft nutzt. Gespeichert im Schlüsselbund des Systems."));
        }
        keep
    }

    fn settings_body(&mut self, ui: &mut Ui, tab: &mut SettingsTab, new_note: &mut String) -> bool {
        let t = self.tokens.clone();
        let mut keep = true;
        ui.horizontal(|ui| {
            ui.heading("Einstellungen");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Schließen").clicked() {
                    keep = false;
                }
            });
        });
        ui.horizontal(|ui| {
            for (k, label) in [(SettingsTab::Backends, "Backends"), (SettingsTab::Memory, "Gedächtnis"), (SettingsTab::Appearance, "Darstellung"), (SettingsTab::About, "Über")] {
                ui.selectable_value(tab, k, label);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| match tab {
            SettingsTab::Backends => {
                for b in self.state.backends.clone() {
                    card(&t).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = match b.status {
                                BackendStatus::Ready { .. } => t.success,
                                BackendStatus::Checking => t.warning,
                                BackendStatus::Failed { .. } => t.danger,
                                _ => t.muted,
                            };
                            status_dot(ui, color);
                            ui.label(RichText::new(&b.config.name).strong());
                            ui.label(small_muted(&t, b.config.kind.label()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Entfernen").clicked() {
                                    self.dialog = Some(Dialog::Confirm { text: format!("{} entfernen?", b.config.name), command: "backend.remove".into(), params: json!({"id": b.config.id}) });
                                }
                                let is_ais = b.config.kind == BackendKind::AisChat;
                                if b.config.kind == BackendKind::ClaudeCli {
                                    // Signed in through `claude auth login`; nothing to store here.
                                } else if b.has_secret {
                                    if ui.small_button(if is_ais { "Abmelden" } else { "Schlüssel entfernen" }).clicked() {
                                        self.run("backend.logout", json!({"id": b.config.id}));
                                    }
                                } else if ui.small_button(if is_ais { "Anmelden" } else { "Schlüssel …" }).clicked() {
                                    self.dialog = Some(Dialog::login(&b.config.id));
                                }
                                if ui.small_button("Prüfen").clicked() {
                                    self.run("backend.check", json!({"id": b.config.id}));
                                }
                            });
                        });
                        ui.label(small_muted(&t, &b.config.base_url));
                        match &b.status {
                            BackendStatus::Ready { account } => {
                                let who = [account.display_name.clone(), account.detail.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
                                ui.label(small_muted(&t, if who.is_empty() { format!("{} Modelle", b.models.len()) } else { who }));
                            }
                            BackendStatus::Failed { message } | BackendStatus::SignedOut { message } => {
                                ui.label(RichText::new(message).small().color(t.danger));
                            }
                            _ => {}
                        }
                        let mut ctx_on = b.config.send_context;
                        if b.capabilities.system_prompt && ui.checkbox(&mut ctx_on, "Projektanweisungen und Notizen mitsenden").changed() {
                            self.run("backend.update", json!({"id": b.config.id, "send_context": ctx_on}));
                        }
                    });
                    ui.add_space(6.0);
                }
                if ui.button("+ Backend hinzufügen").clicked() {
                    self.dialog = Some(Dialog::add_backend());
                }
            }
            SettingsTab::Memory => {
                let mut on = self.state.settings.memory;
                if ui.checkbox(&mut on, "Gedächtnis verwenden").changed() {
                    self.run("settings.set", json!({"key": "memory", "value": on}));
                }
                ui.label(small_muted(
                    &t,
                    "Notizen entstehen nur aus deinen eigenen Worten: „Merke dir: …“ im Chat oder hier. Antworten, Webseiten und Dateien können keine Notizen anlegen. Jede Notiz zeigt, woher sie stammt.",
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(new_note).hint_text("Neue Notiz, z. B. Ich unterrichte Deutsch und Geschichte").desired_width(440.0));
                    if ui.add_enabled(!new_note.trim().is_empty(), egui::Button::new("Merken")).clicked() {
                        self.run("memory.add", json!({"text": new_note.trim()}));
                        new_note.clear();
                    }
                });
                ui.add_space(6.0);
                for area in MemoryArea::ALL {
                    let notes: Vec<_> = self.state.memory.iter().filter(|n| n.area == area && !n.archived).cloned().collect();
                    if notes.is_empty() {
                        continue;
                    }
                    ui.label(RichText::new(area.label()).strong());
                    for n in notes {
                        ui.horizontal(|ui| {
                            ui.label(format!("•  {}", n.text)).on_hover_text(format!("Aus deinen Worten: „{}“", n.source_quote));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Vergessen").clicked() {
                                    self.run("memory.archive", json!({"id": n.id}));
                                }
                                ui.menu_button("Bereich", |ui| {
                                    for a in MemoryArea::ALL {
                                        if ui.button(a.label()).clicked() {
                                            self.run("memory.update", json!({"id": n.id, "area": a.as_str()}));
                                            ui.close();
                                        }
                                    }
                                });
                            });
                        });
                    }
                    ui.add_space(4.0);
                }
                let archived: Vec<_> = self.state.memory.iter().filter(|n| n.archived).cloned().collect();
                if !archived.is_empty() {
                    egui::CollapsingHeader::new(small_muted(&t, format!("Vergessen ({})", archived.len()))).show(ui, |ui| {
                        for n in archived {
                            ui.horizontal(|ui| {
                                ui.label(muted(&t, &n.text));
                                if ui.small_button("Wiederherstellen").clicked() {
                                    self.run("memory.restore", json!({"id": n.id}));
                                }
                            });
                        }
                    });
                }
            }
            SettingsTab::Appearance => {
                ui.label("Farben");
                ui.horizontal(|ui| {
                    for (k, label) in [("system", "AIS (wie System)"), ("light", "AIS hell"), ("dark", "AIS dunkel"), ("omarchy", "Wie Omarchy")] {
                        if ui.selectable_label(self.state.settings.theme == k, label).clicked() {
                            self.run("settings.set", json!({"key": "theme", "value": k}));
                        }
                    }
                });
                ui.add_space(8.0);
                let mut zoom = self.state.settings.zoom;
                if ui.add(egui::Slider::new(&mut zoom, 0.7..=2.0).text("Zoom").step_by(0.05)).drag_stopped() {
                    self.run("settings.set", json!({"key": "zoom", "value": zoom}));
                }
                let mut enter = self.state.settings.enter_sends;
                if ui.checkbox(&mut enter, "Enter sendet (Umschalt+Enter für neue Zeile)").changed() {
                    self.run("settings.set", json!({"key": "enter_sends", "value": enter}));
                }
            }
            SettingsTab::About => {
                ui.label(RichText::new(format!("AISCraft {}", env!("CARGO_PKG_VERSION"))).strong());
                ui.label(muted(&t, "Nativer Client für AIS.chat und andere KI-Dienste. Rust und egui, ohne Webview."));
                ui.add_space(6.0);
                ui.label(small_muted(&t, "Tastenkürzel: Strg+K Befehle · Strg+N neuer Chat · Strg+B Seitenleiste · Strg+Umschalt+F Suche · Strg+R neu erzeugen · Esc anhalten"));
                ui.label(small_muted(&t, "Für Agenten: aiscraft --control startet den JSON-Steuerkanal; aiscraft-cli spricht dieselben Befehle."));
            }
        });
        keep
    }
}
fn aiscraft_backends_cookie_name() -> &'static str {
    aiscraft_engine::SESSION_COOKIE
}
