use aiscraft_core::{BackendKind, MemoryArea, Project};
use aiscraft_engine::BackendStatus;
use egui::{Id, RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, card, muted, small_muted, status_dot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Backends,
    Connectors,
    Schedules,
    Usage,
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
    AddSchedule { title: String, prompt: String, when: String, recurring: bool, watch: bool },
    AddConnector { name: String, command: String, env: String, url: String, headers: String, http: bool },
    Project { id: Option<String>, name: String, instructions: String },
    Rename { id: String, title: String },
    RenameArtifact { id: String, title: String },
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
                Dialog::Settings { .. } => 780.0,
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
                ui.heading("Willkommen bei AIS.ChatCraft");
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
            Dialog::AddConnector { name, command, env, url, headers, http } => {
                ui.heading("Konnektor hinzufügen");
                ui.add_space(4.0);
                let presets = aiscraft_engine::connector_presets();
                if !presets.is_empty() {
                    ui.label(small_muted(&t, "Vorlagen"));
                    ui.horizontal_wrapped(|ui| {
                        for (label, pname, cmdline) in &presets {
                            if ui.button(label).on_hover_text(cmdline).clicked() {
                                *http = false;
                                *name = pname.clone();
                                *command = cmdline.clone();
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.selectable_value(http, false, "Programm (lokal)");
                    ui.selectable_value(http, true, "Adresse (entfernt)");
                });
                egui::Grid::new("add-conn").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(name).hint_text("z. B. Dateien").desired_width(320.0));
                    ui.end_row();
                    if *http {
                        ui.label("URL");
                        ui.add(egui::TextEdit::singleline(url).hint_text("https://…/mcp").desired_width(320.0));
                        ui.end_row();
                        ui.label("Header");
                        ui.add(egui::TextEdit::multiline(headers).hint_text("Authorization: Bearer …").desired_rows(2).desired_width(320.0));
                        ui.end_row();
                    } else {
                        ui.label("Befehl");
                        ui.add(egui::TextEdit::singleline(command).hint_text("npx -y @modelcontextprotocol/server-filesystem ~/Dokumente").desired_width(320.0));
                        ui.end_row();
                        ui.label("Umgebung");
                        ui.add(egui::TextEdit::multiline(env).hint_text("SCHLÜSSEL=Wert (eine pro Zeile)").desired_rows(2).desired_width(320.0));
                        ui.end_row();
                    }
                });
                ui.label(small_muted(&t, "Lesende Werkzeuge laufen direkt, alles andere fragt vorher. Die Vorlagen brauchen Node (npx) bzw. uv (uvx). Zugangsdaten liegen im Schlüsselbund."));
                ui.add_space(8.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    let ok = !name.trim().is_empty() && if *http { !url.trim().is_empty() } else { !command.trim().is_empty() };
                    if ui.add_enabled(ok, egui::Button::new("Verbinden")).clicked() {
                        if *http {
                            self.run("connector.add", json!({"name": name, "url": url, "headers": headers}));
                        } else {
                            self.run("connector.add", json!({"name": name, "command": command, "env": env}));
                        }
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::AddSchedule { title, prompt, when, recurring, watch } => {
                ui.heading("Zeitplan anlegen");
                ui.horizontal(|ui| {
                    if ui.selectable_label(*recurring, "Wiederkehrend").clicked() && !*recurring {
                        *recurring = true;
                        *when = "0 7 * * 1-5".into();
                    }
                    if ui.selectable_label(!*recurring, "Einmalig").clicked() && *recurring {
                        *recurring = false;
                        *when = "morgen 7:30".into();
                    }
                });
                egui::Grid::new("add-sched").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Titel");
                    ui.add(egui::TextEdit::singleline(title).hint_text("z. B. Morgenüberblick").desired_width(320.0));
                    ui.end_row();
                    ui.label("Auftrag");
                    ui.add(egui::TextEdit::multiline(prompt).hint_text("Was soll dann passieren?").desired_rows(3).desired_width(320.0));
                    ui.end_row();
                    ui.label(if *recurring { "Regel (cron)" } else { "Wann" });
                    ui.add(egui::TextEdit::singleline(when).desired_width(320.0));
                    ui.end_row();
                });
                ui.label(small_muted(&t, if *recurring { "Minute Stunde Tag Monat Wochentag, Ortszeit. „0 7 * * 1-5“ = werktags 7:00, „*/30 * * * *“ = alle 30 Minuten." } else { "„in 30 min“, „in 2 Tagen“, „morgen 7:30“ oder „2026-12-04 08:00“." }));
                ui.checkbox(watch, "Beobachten: nur melden, wenn sich etwas geändert hat");
                ui.add_space(8.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!title.trim().is_empty() && !prompt.trim().is_empty(), egui::Button::new("Anlegen")).clicked() {
                        let p = if *recurring { json!({"title": title, "prompt": prompt, "cron": when, "watch": watch}) } else { json!({"title": title, "prompt": prompt, "at": when, "watch": watch}) };
                        self.run("schedule.create", p);
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::RenameArtifact { id, title } => {
                ui.heading("Umbenennen");
                let r = ui.add(egui::TextEdit::singleline(title).desired_width(f32::INFINITY));
                r.request_focus();
                let mut keep = true;
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.horizontal(|ui| {
                    if (ui.button("Speichern").clicked() || enter) && !title.trim().is_empty() {
                        self.run("artifact.rename", json!({"id": id, "title": title}));
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
        let mut manual = view.config.kind != BackendKind::AisChat;
        if view.config.kind == BackendKind::AisChat {
            ui.label(muted(&t, "Die Anmeldung (VIDIS, Schulkonto oder Microsoft 365) läuft in einem eigenen Browserfenster. AIS.ChatCraft übernimmt danach die Sitzung und schließt das Fenster."));
            ui.add_space(8.0);
            let checking = matches!(view.status, BackendStatus::Checking);
            ui.horizontal(|ui| {
                if ui.add_enabled(!checking, egui::Button::new(RichText::new("Im Browser anmelden").color(t.on_accent).strong()).fill(t.accent)).clicked() {
                    self.run("backend.browser_login", json!({"id": backend_id}));
                    *sent = true;
                }
                if checking {
                    ui.add(egui::Spinner::new());
                    ui.label(small_muted(&t, "Warte auf die Anmeldung im Browserfenster …"));
                }
            });
            ui.label(small_muted(&t, "Braucht Chrome, Chromium, Edge, Brave oder Vivaldi. Das Profil bleibt, damit die nächste Anmeldung meist ohne Fenster klappt."));
            ui.add_space(6.0);
            let open = egui::CollapsingHeader::new(small_muted(&t, "Oder den Sitzungs-Cookie von Hand einfügen")).id_salt("manual-cookie").show(ui, |ui| {
                card(&t).show(ui, |ui| {
                    ui.label(RichText::new("1.  AIS.chat im Browser öffnen und anmelden").strong());
                    if ui.button("AIS.chat öffnen ↗").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&view.config.base_url));
                    }
                    ui.label(RichText::new("2.  F12 › Anwendung/Web-Speicher › Cookies, diesen Wert kopieren:").strong());
                    ui.horizontal(|ui| {
                        ui.code(aiscraft_backends_cookie_name());
                        if ui.small_button("Kopieren").on_hover_text("Namen kopieren").clicked() {
                            ui.ctx().copy_text(aiscraft_backends_cookie_name().to_string());
                        }
                    });
                });
            });
            manual = open.fully_open() || !secret.is_empty();
        } else {
            ui.label(muted(&t, "API-Schlüssel eingeben. Er liegt danach im Schlüsselbund des Systems."));
        }
        if !manual {
            match &view.status {
                BackendStatus::SignedOut { message } | BackendStatus::Failed { message } if *sent => {
                    ui.label(RichText::new(message).color(t.danger));
                }
                _ => {}
            }
            ui.add_space(6.0);
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
            return keep;
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
            ui.label(small_muted(&t, "Die Sitzung gilt 8 Stunden und verlängert sich, solange du AIS.ChatCraft nutzt. Gespeichert im Schlüsselbund des Systems."));
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
        ui.horizontal_wrapped(|ui| {
            for (k, label) in [(SettingsTab::Backends, "Backends"), (SettingsTab::Connectors, "Konnektoren"), (SettingsTab::Schedules, "Zeitpläne"), (SettingsTab::Usage, "Nutzung"), (SettingsTab::Memory, "Gedächtnis"), (SettingsTab::Appearance, "Darstellung"), (SettingsTab::About, "Über")] {
                ui.selectable_value(tab, k, label);
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().max_height((ui.ctx().content_rect().height() * 0.8 - 140.0).max(300.0)).show(ui, |ui| match tab {
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
            SettingsTab::Connectors => {
                ui.label(muted(&t, "Konnektoren sind MCP-Server. Ihre Werkzeuge können Modelle aufrufen, die das unterstützen (OpenAI-kompatibel, Claude über API). AIS.chat über die Anmeldung und das Claude-Abo nutzen keine eigenen Werkzeuge."));
                ui.add_space(6.0);
                let st = self.state.settings.clone();
                let mut tools = st.tools;
                if ui.checkbox(&mut tools, "Werkzeuge anbieten (Verlauf durchsuchen, Dokumente, Konnektoren)").changed() {
                    self.run("settings.set", json!({"key": "tools", "value": tools}));
                }
                let mut trust = st.trust_check;
                if ui.checkbox(&mut trust, "Vertrauensprüfung: vor jeder Frage prüfen, ob deine Worte die Aktion decken").changed() {
                    self.run("settings.set", json!({"key": "trust_check", "value": trust}));
                }
                let mut auto = st.auto_allow_covered;
                if ui.add_enabled(trust, egui::Checkbox::new(&mut auto, "Ohne Nachfrage ausführen, wenn deine Worte sie wörtlich decken (nie bei löschenden Werkzeugen)")).changed() {
                    self.run("settings.set", json!({"key": "auto_allow_covered", "value": auto}));
                }
                ui.add_space(8.0);
                for k in self.state.connectors.clone() {
                    card(&t).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = match k.status.as_str() {
                                "connected" => t.success,
                                "connecting" => t.warning,
                                "error" => t.danger,
                                _ => t.muted,
                            };
                            status_dot(ui, color);
                            ui.label(RichText::new(&k.name).strong());
                            ui.label(small_muted(&t, format!("{} Werkzeuge", k.tools.len())));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Entfernen").clicked() {
                                    self.dialog = Some(Dialog::Confirm { text: format!("Konnektor {} entfernen?", k.name), command: "connector.remove".into(), params: json!({"id": k.id}) });
                                }
                                if ui.small_button("Neu verbinden").clicked() {
                                    self.run("connector.refresh", json!({"id": k.id}));
                                }
                                let mut en = k.enabled;
                                if ui.checkbox(&mut en, "aktiv").changed() {
                                    self.run("connector.toggle", json!({"id": k.id, "enabled": en}));
                                }
                            });
                        });
                        ui.label(small_muted(&t, &k.target));
                        if let Some(e) = &k.error {
                            ui.label(RichText::new(e).small().color(t.danger));
                        }
                        if !k.tools.is_empty() {
                            egui::CollapsingHeader::new(small_muted(&t, "Werkzeuge")).id_salt(("ktools", &k.id)).show(ui, |ui| {
                                for tool in &k.tools {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&tool.name).monospace().small());
                                        ui.label(small_muted(&t, if tool.read_only { "nur lesen" } else { "fragt vorher" }));
                                    });
                                }
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
                if ui.button("+ Konnektor hinzufügen").clicked() {
                    self.dialog = Some(Dialog::AddConnector { name: String::new(), command: String::new(), env: String::new(), url: String::new(), headers: String::new(), http: false });
                }
            }
            SettingsTab::Schedules => {
                ui.label(muted(&t, "Zeitpläne schicken einen Auftrag wiederkehrend oder einmal ab; die Antwort landet in einem eigenen Chat. Sie laufen, solange AIS.ChatCraft läuft, und Werkzeuge, die eine Freigabe brauchen, werden dabei abgelehnt."));
                ui.add_space(6.0);
                let now = aiscraft_engine::now_ms();
                for sch in self.state.schedules.clone() {
                    card(&t).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            status_dot(ui, if sch.enabled { t.success } else { t.muted });
                            ui.label(RichText::new(&sch.title).strong());
                            let rule = sch.cron.clone().map(|c| format!("cron {c}")).unwrap_or_else(|| "einmalig".into());
                            ui.label(small_muted(&t, if sch.watch { format!("{rule} · beobachtet") } else { rule }));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Löschen").clicked() {
                                    self.run("schedule.delete", json!({"id": sch.id}));
                                }
                                if ui.small_button("Jetzt ausführen").clicked() {
                                    self.run("schedule.run", json!({"id": sch.id}));
                                }
                                let mut en = sch.enabled;
                                if ui.checkbox(&mut en, "aktiv").changed() {
                                    self.run("schedule.update", json!({"id": sch.id, "enabled": en}));
                                }
                            });
                        });
                        ui.label(small_muted(&t, sch.prompt.chars().take(160).collect::<String>()));
                        let next = aiscraft_engine::schedule_next_run(&sch, now).map(fmt_time).unwrap_or_else(|| "–".into());
                        ui.label(small_muted(&t, format!("Nächster Lauf: {next}{}", sch.last_run.map(|l| format!(" · zuletzt {}", fmt_time(l))).unwrap_or_default())));
                        if let Some(conv) = &sch.conversation_id
                            && ui.small_button("Chat öffnen").clicked()
                        {
                            self.run("chat.open", json!({"id": conv}));
                        }
                    });
                    ui.add_space(6.0);
                }
                if ui.button("+ Zeitplan anlegen").clicked() {
                    self.dialog = Some(Dialog::AddSchedule { title: String::new(), prompt: String::new(), when: "0 7 * * 1-5".into(), recurring: true, watch: false });
                }
            }
            SettingsTab::Usage => {
                match self.engine.call("usage.report", json!({})) {
                    Ok(r) => {
                        let rows = r["rows"].as_array().cloned().unwrap_or_default();
                        if rows.is_empty() {
                            ui.label(muted(&t, "Noch keine Token-Zahlen. AIS.chat über die Anmeldung meldet keine."));
                        }
                        egui::Grid::new("usage").striped(true).num_columns(5).spacing([14.0, 6.0]).show(ui, |ui| {
                            for h in ["Monat", "Backend", "Modell", "Tokens (ein/aus)", "Kosten"] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for row in rows {
                                ui.label(row["month"].as_str().unwrap_or(""));
                                ui.label(row["backend"].as_str().unwrap_or(""));
                                ui.label(row["model"].as_str().unwrap_or(""));
                                ui.label(format!("{} / {}", row["input_tokens"].as_i64().unwrap_or(0), row["output_tokens"].as_i64().unwrap_or(0)));
                                ui.label(row["usd"].as_f64().map(|u| format!("≈ {u:.2} $")).unwrap_or_else(|| "–".into()));
                                ui.end_row();
                            }
                        });
                    }
                    Err(e) => {
                        ui.label(RichText::new(e.message).color(t.danger));
                    }
                }
                ui.add_space(10.0);
                let mut limit = self.state.settings.monthly_token_limit;
                ui.horizontal(|ui| {
                    ui.label("Monatsgrenze (Tokens, 0 = keine):");
                    if ui.add(egui::DragValue::new(&mut limit).speed(10_000.0).range(0..=1_000_000_000u64)).changed() {
                        self.run("settings.set", json!({"key": "monthly_token_limit", "value": limit}));
                    }
                });
                ui.label(small_muted(&t, "Ab der Grenze fragt AIS.ChatCraft vor dem Senden nach. Kosten nur für Claude über API (Preise je Million Tokens); Abo und AIS.chat rechnen nicht pro Token ab."));
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
                ui.label(RichText::new(format!("AIS.ChatCraft {}", env!("CARGO_PKG_VERSION"))).strong());
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

fn fmt_time(ms: i64) -> String {
    aiscraft_engine::format_local(ms)
}
