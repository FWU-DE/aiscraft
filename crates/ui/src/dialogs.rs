use aiscraft_core::{BackendKind, MemoryArea, Project};
use aiscraft_engine::BackendStatus;
use egui::{Id, RichText, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, card, dialog_frame, link_button, muted, small_muted, status_dot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Backends,
    Connectors,
    Schedules,
    Skills,
    Vault,
    Usage,
    Memory,
    Appearance,
    About,
}

#[derive(Debug, Clone)]
pub enum Dialog {
    /// First the look; `look_only` for people who are set up already (shown once after the
    /// update that brought the new looks), else the sign-in follows.
    Welcome {
        look_only: bool,
    },
    /// The welcome's second step: sign in to AIS.chat or add another service.
    WelcomeSignIn,
    Login {
        backend_id: String,
        secret: String,
        sent: bool,
    },
    AddBackend {
        preset: usize,
        name: String,
        base_url: String,
        secret: String,
    },
    Settings {
        tab: SettingsTab,
        new_note: String,
    },
    AddSchedule {
        title: String,
        prompt: String,
        when: String,
        recurring: bool,
        watch: bool,
    },
    EditSkill {
        id: Option<String>,
        title: String,
        when: String,
        steps: String,
    },
    AddVault {
        name: String,
        username: String,
        password: String,
        totp: String,
        url: String,
    },
    AddConnector {
        name: String,
        command: String,
        env: String,
        url: String,
        headers: String,
        http: bool,
    },
    Project {
        id: Option<String>,
        name: String,
        instructions: String,
    },
    Rename {
        id: String,
        title: String,
    },
    RenameArtifact {
        id: String,
        title: String,
    },
    Confirm {
        text: String,
        command: String,
        params: Value,
    },
}

/// Ready-made backends: (label, kind, base URL, needs a key, hint).
pub const PRESETS: &[(&str, BackendKind, &str, bool, &str)] = &[
    (
        "Claude (Abo)",
        BackendKind::ClaudeCli,
        "claude",
        false,
        "Nutzt dein claude.ai-Abo über die lokale Claude-CLI (vorher `claude auth login`). Ohne Werkzeuge, nur Chat.",
    ),
    ("Claude (API)", BackendKind::Anthropic, "https://api.anthropic.com", true, "API-Schlüssel von console.anthropic.com, Abrechnung nach Verbrauch"),
    ("OpenAI", BackendKind::OpenAi, "https://api.openai.com/v1", true, "API-Schlüssel von platform.openai.com"),
    (
        "AIS.chat API",
        BackendKind::OpenAi,
        "https://api.ais-chat.schule/v1",
        true,
        "API-Schlüssel aus dem AIS.chat-Admin; eigener Verlauf, getrennt von der Web-App",
    ),
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
        let (label, url) = PRESETS.first().map(|p| (p.0, p.2)).unwrap_or(("", "https://"));
        Dialog::AddBackend { preset: 0, name: label.into(), base_url: url.into(), secret: String::new() }
    }
    pub fn project(p: Option<&Project>) -> Self {
        Dialog::Project {
            id: p.map(|p| p.id.clone()),
            name: p.map(|p| p.name.clone()).unwrap_or_default(),
            instructions: p.map(|p| p.instructions.clone()).unwrap_or_default(),
        }
    }
}

/// How to contribute (bug reports, ideas, code), linked from the About tab and the manual.
const CONTRIBUTING_URL: &str = "https://github.com/FWU-DE/aiscraft/blob/main/CONTRIBUTING.md";
const ISSUES_URL: &str = "https://github.com/FWU-DE/aiscraft/issues/new";

/// AIS.ChatCraft is not made by the AIS.chat team: said in the welcome, under "Über" and in the
/// handbook.
pub(crate) const COMMUNITY_NOTICE: &str = "AIS.ChatCraft ist kein offizielles Werkzeug des AIS.chat-Teams, sondern ein Community-Experiment. Es nutzt AIS.chat über die Web-App; Funktionen können sich ändern oder ausfallen.";

/// The notice as a calm info box with the link for feedback.
pub(crate) fn community_notice(ui: &mut Ui, t: &crate::theme::Tokens) {
    egui::Frame::new()
        .fill(t.opaque(t.accent_soft))
        .corner_radius(egui::CornerRadius::same(t.control_radius()))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                if t.designed() {
                    ui.label(crate::icons::Icon::Info.text(17.0).color(t.accent_ink));
                } else {
                    ui.label(RichText::new("i").strong().color(t.accent_ink));
                }
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(COMMUNITY_NOTICE).color(t.accent_ink)).wrap());
                    ui.hyperlink_to(RichText::new("Rückmeldung und Ideen auf GitHub").color(t.link), ISSUES_URL).on_hover_text(ISSUES_URL);
                });
            });
        });
}

/// Space kept free between a dialog and the window edge, above and below.
const DIALOG_SCREEN_GAP: f32 = 24.0;

impl AisCraftApp {
    pub(crate) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut keep = true;
        let designed_settings = self.tokens.designed() && matches!(dialog, Dialog::Settings { .. });
        let frame = if designed_settings { dialog_frame(ctx, &self.tokens).inner_margin(egui::Margin::ZERO) } else { dialog_frame(ctx, &self.tokens) };
        let modal = egui::Modal::new(Id::unique("dialog")).frame(frame).backdrop_color(self.tokens.overlay).show(ctx, |ui| {
            ui.set_width(match dialog {
                Dialog::Settings { .. } if designed_settings => (ctx.content_rect().width() - 48.0).clamp(600.0, 940.0),
                Dialog::Settings { .. } => 820.0,
                Dialog::Welcome { .. } => 640.0,
                Dialog::Login { .. } | Dialog::WelcomeSignIn => 520.0,
                _ => 460.0,
            });
            keep = self.dialog_body(ui, &mut dialog);
        });
        if modal.should_close() {
            keep = false;
        }
        // The look question shows once: picking, going on or closing all count.
        if matches!(dialog, Dialog::WelcomeSignIn | Dialog::Login { .. }) || (!keep && matches!(dialog, Dialog::Welcome { .. })) {
            self.mark_look_chosen();
        }
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    /// Remember that the look was chosen, so the question does not come again.
    pub(crate) fn mark_look_chosen(&self) {
        if !self.state.settings.look_chosen {
            self.run("settings.set", json!({"key": "look_chosen", "value": true}));
        }
    }

    /// Returns whether the dialog stays open.
    fn dialog_body(&mut self, ui: &mut Ui, dialog: &mut Dialog) -> bool {
        let t = self.tokens.clone();
        match dialog {
            Dialog::Welcome { look_only } => {
                let look_only = *look_only;
                ui.label(crate::theme::title(&t, if look_only { "AIS.ChatCraft sieht jetzt neu aus" } else { "Willkommen bei AIS.ChatCraft" }).size(26.0));
                ui.add_space(4.0);
                ui.label(muted(
                    &t,
                    if look_only {
                        "Such dir aus, wie es aussehen soll. Du kannst es jederzeit unter Einstellungen → Darstellung ändern."
                    } else {
                        "Chatten mit AIS.chat und anderen KI-Diensten. Zuerst: Wie soll AIS.ChatCraft aussehen? Das lässt sich später unter Einstellungen → Darstellung ändern."
                    },
                ));
                ui.add_space(12.0);
                community_notice(ui, &t);
                ui.add_space(16.0);
                self.look_cards(ui, false);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.label(crate::theme::semibold("Farbschema"));
                    ui.add_space(8.0);
                    self.mode_picker(ui);
                });
                ui.add_space(20.0);
                let mut keep = true;
                let mut next = None;
                ui.horizontal(|ui| {
                    if look_only {
                        if accent_button(ui, &t, "Fertig").clicked() {
                            keep = false;
                        }
                    } else {
                        if accent_button(ui, &t, "Weiter").clicked() {
                            next = Some(Dialog::WelcomeSignIn);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if link_button(ui, &t, "Später").clicked() {
                                keep = false;
                            }
                        });
                    }
                });
                if let Some(n) = next {
                    *dialog = n;
                }
                keep
            }
            Dialog::WelcomeSignIn => {
                ui.label(crate::theme::title(&t, "Mit einem Dienst verbinden").size(24.0));
                ui.add_space(6.0);
                ui.label(muted(&t, "Chatten mit AIS.chat und anderen KI-Diensten."));
                ui.add_space(10.0);
                for line in [
                    "Deine Chats bleiben auf diesem Rechner und sind durchsuchbar.",
                    "Projekte bündeln Chats mit eigenen Anweisungen.",
                    "Sag „Merke dir: …“, um Notizen anzulegen.",
                ] {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new("•").color(t.accent));
                        ui.add(egui::Label::new(line).wrap());
                    });
                }
                ui.add_space(20.0);
                let ais = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat).map(|b| b.config.id.clone());
                let mut next = None;
                let mut keep = true;
                ui.horizontal(|ui| {
                    if let Some(id) = &ais
                        && accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
                    {
                        next = Some(Dialog::login(id));
                    }
                    if ui.button("Anderen Dienst nutzen").clicked() {
                        next = Some(Dialog::add_backend());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if link_button(ui, &t, "Später").clicked() {
                            keep = false;
                        }
                    });
                });
                if let Some(n) = next {
                    *dialog = n;
                }
                keep
            }
            Dialog::Login { backend_id, secret, sent } => self.login_body(ui, backend_id, secret, sent),
            Dialog::AddBackend { preset, name, base_url, secret } => {
                ui.heading("Dienst hinzufügen");
                ui.add_space(6.0);
                egui::ComboBox::from_id_salt("preset").width(ui.available_width()).selected_text(PRESETS.get(*preset).map(|p| p.0).unwrap_or("")).show_ui(
                    ui,
                    |ui| {
                        for (i, p) in PRESETS.iter().enumerate() {
                            if ui.selectable_label(*preset == i, p.0).clicked() {
                                *preset = i;
                                *name = p.0.trim_end_matches(" (lokal)").to_string();
                                *base_url = p.2.to_string();
                            }
                        }
                    },
                );
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
                ui.label(small_muted(&t, "Gehen an Dienste, die Kontext erlauben (nicht an AIS.chat über die Anmeldung)."));
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
                        ui.add(
                            egui::TextEdit::singleline(command).hint_text("npx -y @modelcontextprotocol/server-filesystem ~/Dokumente").desired_width(320.0),
                        );
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
                ui.label(small_muted(
                    &t,
                    if *recurring {
                        "Minute Stunde Tag Monat Wochentag, Ortszeit. „0 7 * * 1-5“ = werktags 7:00, „*/30 * * * *“ = alle 30 Minuten."
                    } else {
                        "„in 30 min“, „in 2 Tagen“, „morgen 7:30“ oder „2026-12-04 08:00“."
                    },
                ));
                ui.checkbox(watch, "Beobachten: nur melden, wenn sich etwas geändert hat");
                ui.add_space(8.0);
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!title.trim().is_empty() && !prompt.trim().is_empty(), egui::Button::new("Anlegen")).clicked() {
                        let p = if *recurring {
                            json!({"title": title, "prompt": prompt, "cron": when, "watch": watch})
                        } else {
                            json!({"title": title, "prompt": prompt, "at": when, "watch": watch})
                        };
                        self.run("schedule.create", p);
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::AddVault { name, username, password, totp, url } => {
                ui.heading("Tresor-Eintrag");
                egui::Grid::new("vault").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(name).hint_text("z. B. schulportal").desired_width(300.0));
                    ui.end_row();
                    ui.label("Benutzer");
                    ui.add(egui::TextEdit::singleline(username).desired_width(300.0));
                    ui.end_row();
                    ui.label("Passwort");
                    ui.add(egui::TextEdit::singleline(password).password(true).desired_width(300.0));
                    ui.end_row();
                    ui.label("TOTP-Geheimnis");
                    ui.add(egui::TextEdit::singleline(totp).password(true).hint_text("Base32, optional").desired_width(300.0));
                    ui.end_row();
                    ui.label("Adresse");
                    ui.add(egui::TextEdit::singleline(url).hint_text("optional").desired_width(300.0));
                    ui.end_row();
                });
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Speichern")).clicked() {
                        self.run("vault.add", json!({"name": name, "username": username, "password": password, "totp": totp, "url": url}));
                        keep = false;
                    }
                    if ui.button("Abbrechen").clicked() {
                        keep = false;
                    }
                });
                keep
            }
            Dialog::EditSkill { id, title, when, steps } => {
                ui.heading(if id.is_some() { "Vorgehen bearbeiten" } else { "Neues Vorgehen" });
                ui.label("Titel");
                ui.add(egui::TextEdit::singleline(title).desired_width(f32::INFINITY));
                ui.label("Wann es passt");
                ui.add(egui::TextEdit::singleline(when).hint_text("z. B. Wenn ein Elternbrief gebraucht wird").desired_width(f32::INFINITY));
                ui.label("Schritte");
                ui.add(egui::TextEdit::multiline(steps).hint_text("1. …\n2. …").desired_rows(6).desired_width(f32::INFINITY));
                let mut keep = true;
                ui.horizontal(|ui| {
                    if ui.add_enabled(!title.trim().is_empty() && !steps.trim().is_empty(), egui::Button::new("Speichern")).clicked() {
                        match id {
                            Some(id) => self.run("skill.update", json!({"id": id, "title": title, "when": when, "steps": steps})),
                            None => self.run("skill.create", json!({"title": title, "when": when, "steps": steps})),
                        }
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
            ui.label(small_muted(
                &t,
                "Braucht Chrome, Chromium, Edge, Brave oder Vivaldi. Das Profil bleibt, damit die nächste Anmeldung meist ohne Fenster klappt.",
            ));
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
            ui.label(small_muted(
                &t,
                "Die Sitzung gilt 8 Stunden und verlängert sich, solange du AIS.ChatCraft nutzt. Gespeichert im Schlüsselbund des Systems.",
            ));
        }
        keep
    }

    /// Mutig and Glas: a rail with icons on a recessed surface, the section's title in Bold with
    /// a line under it, and a round close button.
    fn settings_body_designed(&mut self, ui: &mut Ui, tab: &mut SettingsTab, new_note: &mut String) -> bool {
        use crate::icons::Icon;
        let t = self.tokens.clone();
        let screen_h = ui.ctx().content_rect().height();
        let h = (screen_h - 2.0 * DIALOG_SCREEN_GAP).clamp(300.0, 640.0);
        ui.set_max_height(h);
        ui.set_min_height(h);
        let mut keep = true;
        let (chapter, title, sub) = match tab {
            SettingsTab::Backends => ("Anmelden", "Dienste", "AIS.chat und andere KI-Dienste, mit denen AIS.ChatCraft spricht."),
            SettingsTab::Connectors => ("Konnektoren und Freigaben", "Konnektoren", "Werkzeuge, die Modelle aufrufen dürfen, und was sie dafür fragen."),
            SettingsTab::Schedules => ("Zeitpläne und Aufgaben", "Zeitpläne", "Aufträge, die regelmäßig oder einmal zu einer Zeit laufen."),
            SettingsTab::Skills => ("Fähigkeiten und Hinweise", "Fähigkeiten", "Gelernte Vorgehen und Hinweise zur rechten Zeit."),
            SettingsTab::Vault => ("Tresor", "Tresor", "Zugangsdaten, die Werkzeuge nutzen dürfen, ohne sie zu sehen."),
            SettingsTab::Usage => ("Nutzung", "Nutzung", "Verbrauch je Dienst und eine Monatsgrenze."),
            SettingsTab::Memory => ("Gedächtnis", "Gedächtnis", "Was AIS.ChatCraft sich von dir gemerkt hat."),
            SettingsTab::Appearance => ("Tastenkürzel", "Darstellung", "So sieht AIS.ChatCraft auf diesem Gerät aus."),
            SettingsTab::About => ("Mitmachen", "Über", "Version, Aktualisierungen und wie du mitmachen kannst."),
        };
        let rail_fill =
            if t.glass.is_some() { if t.dark { egui::Color32::from_white_alpha(8) } else { egui::Color32::from_white_alpha(102) } } else { t.sunken };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let rail = egui::Frame::new()
                .fill(rail_fill)
                .corner_radius(egui::CornerRadius { nw: 24, sw: 24, ne: 0, se: 0 })
                .inner_margin(egui::Margin { left: 14, right: 12, top: 22, bottom: 16 })
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(206.0);
                        ui.set_min_height(h - 38.0);
                        ui.label(crate::theme::bold("Einstellungen").size(22.0).color(t.text));
                        ui.add_space(14.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (k, icon, label) in [
                            (SettingsTab::Backends, Icon::Cloud, "Dienste"),
                            (SettingsTab::Connectors, Icon::Plug, "Konnektoren"),
                            (SettingsTab::Schedules, Icon::Calendar, "Zeitpläne"),
                            (SettingsTab::Skills, Icon::Bulb, "Fähigkeiten"),
                            (SettingsTab::Vault, Icon::Lock, "Tresor"),
                            (SettingsTab::Usage, Icon::Chart, "Nutzung"),
                            (SettingsTab::Memory, Icon::Memory, "Gedächtnis"),
                            (SettingsTab::Appearance, Icon::Palette, "Darstellung"),
                            (SettingsTab::About, Icon::Info, "Über"),
                        ] {
                            let on = *tab == k;
                            let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
                            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label));
                            let p = ui.painter();
                            if on {
                                crate::kit::accent_fill(p, rect, egui::CornerRadius::same(12), &t);
                            } else if r.hovered() {
                                p.rect_filled(rect, egui::CornerRadius::same(12), t.text.gamma_multiply(0.06));
                            }
                            let fg = if on { t.on_accent } else { t.text2 };
                            icon.paint(p, egui::pos2(rect.left() + 21.0, rect.center().y), 18.0, fg);
                            let g = crate::kit::line(ui, label, crate::kit::font(15.0, if on { crate::theme::SEMIBOLD } else { "" }), fg, 160.0);
                            p.galley(egui::pos2(rect.left() + 41.0, rect.center().y - g.size().y / 2.0), g, fg);
                            if r.clicked() {
                                *tab = k;
                            }
                        }
                        ui.add_space((ui.available_height() - 30.0).max(8.0));
                        if crate::kit::pill(ui, &t, crate::kit::Pill::Plain, Some(Icon::Help), &format!("Handbuch: {title}"), 13.5, None)
                            .on_hover_text("Das passende Kapitel im Handbuch (F1)")
                            .clicked()
                        {
                            self.open_manual(chapter);
                        }
                    })
                });
            let line_x = rail.response.rect.right();
            ui.painter().line_segment(
                [egui::pos2(line_x, rail.response.rect.top()), egui::pos2(line_x, rail.response.rect.bottom())],
                egui::Stroke::new(1.0, t.border),
            );
            egui::Frame::new().inner_margin(egui::Margin { left: 32, right: 22, top: 20, bottom: 18 }).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(crate::theme::bold(title).size(28.0).color(t.text));
                            ui.label(RichText::new(sub).size(14.5).color(t.muted));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            let (rect, r) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), egui::Sense::click());
                            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Schließen"));
                            ui.painter().circle_filled(
                                rect.center(),
                                18.0,
                                if r.hovered() { crate::theme::mix(t.opaque(t.chip), t.text, 0.08) } else { t.opaque(t.chip) },
                            );
                            Icon::X.paint(ui.painter(), rect.center(), 20.0, t.text2);
                            if r.on_hover_text("Schließen (Esc)").clicked() {
                                keep = false;
                            }
                        });
                    });
                    ui.add_space(14.0);
                    let max_h = (ui.available_height() - 8.0).max(160.0);
                    self.settings_tab(ui, tab, new_note, max_h);
                });
            });
        });
        keep
    }

    fn settings_body(&mut self, ui: &mut Ui, tab: &mut SettingsTab, new_note: &mut String) -> bool {
        if self.tokens.designed() {
            return self.settings_body_designed(ui, tab, new_note);
        }
        // egui hands a modal's content the rect of its previous frame, so a tab that got taller (or
        // a first frame measured small) would stay clipped there: allow the screen's full height.
        // Set before anything is drawn, since changing the max rect resets the cursor.
        let frame = 2.0 * f32::from(crate::widgets::DIALOG_MARGIN) + 2.0 * DIALOG_SCREEN_GAP;
        ui.set_max_height(ui.ctx().content_rect().height() - frame);
        let mut keep = true;
        let chapter = match tab {
            SettingsTab::Backends => "Anmelden",
            SettingsTab::Connectors => "Konnektoren und Freigaben",
            SettingsTab::Schedules => "Zeitpläne und Aufgaben",
            SettingsTab::Skills => "Fähigkeiten und Hinweise",
            SettingsTab::Vault => "Tresor",
            SettingsTab::Usage => "Nutzung",
            SettingsTab::Memory => "Gedächtnis",
            SettingsTab::Appearance => "Tastenkürzel",
            SettingsTab::About => "Mitmachen",
        };
        ui.horizontal(|ui| {
            ui.heading("Einstellungen");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Schließen").clicked() {
                    keep = false;
                }
                ui.add_space(8.0);
                if ui.link(format!("Handbuch: {chapter}")).on_hover_text("Das passende Kapitel im Handbuch (F1)").clicked() {
                    self.open_manual(chapter);
                }
            });
        });
        ui.add_space(10.0);
        // Scroll only when the tab is taller than the screen: what is left below the header, minus the
        // modal frame's margins and a gap above and below the dialog.
        // The header's height: from the top of the dialog's content to where the tab body starts.
        // (`min_rect` would include the size the modal remembered from the last frame.)
        let header = ui.cursor().top() - ui.max_rect().top();
        let max_h = (ui.ctx().content_rect().height() - header - frame).max(200.0);
        // The sections as a rail on the left: all nine fit without wrapping.
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                for (k, label) in [
                    (SettingsTab::Backends, "Dienste"),
                    (SettingsTab::Connectors, "Konnektoren"),
                    (SettingsTab::Schedules, "Zeitpläne"),
                    (SettingsTab::Skills, "Fähigkeiten"),
                    (SettingsTab::Vault, "Tresor"),
                    (SettingsTab::Usage, "Nutzung"),
                    (SettingsTab::Memory, "Gedächtnis"),
                    (SettingsTab::Appearance, "Darstellung"),
                    (SettingsTab::About, "Über"),
                ] {
                    if ui.add(egui::Button::selectable(*tab == k, label).min_size(egui::vec2(150.0, 30.0))).clicked() {
                        *tab = k;
                    }
                }
            });
            ui.add_space(18.0);
            ui.vertical(|ui| self.settings_tab(ui, tab, new_note, max_h));
        });
        keep
    }

    /// The content of one settings section.
    fn settings_tab(&mut self, ui: &mut Ui, tab: &mut SettingsTab, new_note: &mut String, max_h: f32) {
        let t = self.tokens.clone();
        egui::ScrollArea::vertical().max_height(max_h).show(ui, |ui| match tab {
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
                                // One width for all three, so the row does not shift between states.
                                let action = |ui: &mut Ui, text: &str| ui.add(egui::Button::new(RichText::new(text).small()).min_size(egui::vec2(124.0, 0.0)));
                                if action(ui, "Entfernen").clicked() {
                                    self.dialog = Some(Dialog::Confirm { text: format!("{} entfernen?", b.config.name), command: "backend.remove".into(), params: json!({"id": b.config.id}) });
                                }
                                let is_ais = b.config.kind == BackendKind::AisChat;
                                if b.config.kind == BackendKind::ClaudeCli {
                                    // Signed in through `claude auth login`; nothing to store here.
                                } else if b.has_secret {
                                    if action(ui, if is_ais { "Abmelden" } else { "Schlüssel entfernen" }).clicked() {
                                        self.run("backend.logout", json!({"id": b.config.id}));
                                    }
                                } else if action(ui, if is_ais { "Anmelden" } else { "Schlüssel eingeben" }).clicked() {
                                    self.dialog = Some(Dialog::login(&b.config.id));
                                }
                                if action(ui, "Prüfen").clicked() {
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
                if ui.button("+ Dienst hinzufügen").clicked() {
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
            SettingsTab::Skills => {
                let st = self.state.settings.clone();
                let mut learn = st.learn_skills;
                if ui.checkbox(&mut learn, "Vorgehen lernen: nach Aufgaben mit mehreren Werkzeugschritten").changed() {
                    self.run("settings.set", json!({"key": "learn_skills", "value": learn}));
                }
                let mut recall = st.recall;
                if ui.checkbox(&mut recall, "Erinnern: passende Vorgehen und Ausschnitte aus früheren Chats mitgeben").changed() {
                    self.run("settings.set", json!({"key": "recall", "value": recall}));
                }
                ui.label(small_muted(&t, format!("Gelernte Vorgehen enthalten keine persönlichen Daten, sind nur Hinweise und nie eine Erlaubnis. Höchstens {} werden behalten; die am meisten genutzten bleiben.", 40)));
                ui.label(small_muted(&t, "Bei AIS.chat stehen sie oben in deiner Nachricht, je Chat nur einmal; unter der Nachricht steht, was mitging."));
                ui.add_space(6.0);
                let mut links = st.check_links;
                if ui.checkbox(&mut links, "Links in Antworten automatisch prüfen").changed() {
                    self.run("settings.set", json!({"key": "check_links", "value": links}));
                }
                ui.label(small_muted(&t, "Fragt jede Adresse in einer Antwort einmal bei der fremden Seite an und streicht tote Links durch. Aus: nur mit „Links prüfen“ unter der Antwort."));
                ui.add_space(6.0);
                let mut pro = st.proactive;
                if ui.checkbox(&mut pro, "Hinweise: stündlich auf offene Zusagen, Fristen und Erinnerungswünsche achten").changed() {
                    self.run("settings.set", json!({"key": "proactive", "value": pro}));
                }
                ui.horizontal(|ui| {
                    ui.label(small_muted(&t, "Ruhezeit von"));
                    let mut qs = st.quiet_start;
                    if ui.add(egui::DragValue::new(&mut qs).range(0..=23u8).suffix(" Uhr")).changed() {
                        self.run("settings.set", json!({"key": "quiet_start", "value": qs}));
                    }
                    ui.label(small_muted(&t, "bis"));
                    let mut qe = st.quiet_end;
                    if ui.add(egui::DragValue::new(&mut qe).range(0..=23u8).suffix(" Uhr")).changed() {
                        self.run("settings.set", json!({"key": "quiet_end", "value": qe}));
                    }
                    if ui.small_button("Jetzt prüfen").clicked() {
                        self.run("hints.check", json!({}));
                    }
                });
                ui.label(small_muted(&t, "Nur bei einem konkreten offenen Punkt, nie zweimal derselbe Hinweis."));
                ui.add_space(6.0);
                if self.state.skills.is_empty() {
                    ui.label(muted(&t, "Noch keine Vorgehen."));
                }
                for sk in self.state.skills.clone() {
                    card(&t).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&sk.title).strong());
                            ui.label(small_muted(&t, format!("{}× genutzt", sk.uses)));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Löschen").clicked() {
                                    self.run("skill.delete", json!({"id": sk.id}));
                                }
                                if ui.small_button("Bearbeiten").clicked() {
                                    self.dialog = Some(Dialog::EditSkill { id: Some(sk.id.clone()), title: sk.title.clone(), when: sk.when.clone(), steps: sk.steps.clone() });
                                }
                            });
                        });
                        if !sk.when.is_empty() {
                            ui.label(small_muted(&t, format!("Wann: {}", sk.when)));
                        }
                        ui.label(small_muted(&t, sk.steps.lines().take(3).collect::<Vec<_>>().join(" · ")));
                    });
                    ui.add_space(6.0);
                }
                if ui.button("+ Vorgehen anlegen").clicked() {
                    self.dialog = Some(Dialog::EditSkill { id: None, title: String::new(), when: String::new(), steps: String::new() });
                }
            }
            SettingsTab::Vault => {
                ui.label(muted(&t, "Zugangsdaten, die AIS.ChatCraft selbst in Konnektor-Aufrufe einsetzt. Das Modell schreibt nur Platzhalter wie {{vault:name.password}} und sieht die Werte nie; jeder Aufruf mit Tresor-Daten fragt vorher. Gespeichert im Schlüsselbund des Systems."));
                ui.add_space(6.0);
                for e in self.state.vault.clone() {
                    card(&t).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&e.name).strong().monospace());
                            let mut f = Vec::new();
                            if e.has_username {
                                f.push("Benutzer");
                            }
                            if e.has_password {
                                f.push("Passwort");
                            }
                            if e.has_totp {
                                f.push("Einmalcode");
                            }
                            ui.label(small_muted(&t, f.join(" · ")));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Löschen").clicked() {
                                    self.dialog = Some(Dialog::Confirm { text: format!("Tresor-Eintrag {} löschen?", e.name), command: "vault.remove".into(), params: json!({"id": e.id}) });
                                }
                                if e.has_totp
                                    && ui.small_button("Code kopieren").clicked()
                                    && let Ok(v) = self.engine.call("vault.code", json!({"id": e.id}))
                                {
                                    ui.ctx().copy_text(v.get("code").and_then(serde_json::Value::as_str).unwrap_or("").to_string());
                                }
                            });
                        });
                        if !e.url.is_empty() {
                            ui.label(small_muted(&t, &e.url));
                        }
                    });
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    if ui.button("+ Eintrag").clicked() {
                        self.dialog = Some(Dialog::AddVault { name: String::new(), username: String::new(), password: String::new(), totp: String::new(), url: String::new() });
                    }
                    if ui.button("Aus Bitwarden (CSV) übernehmen …").clicked()
                        && let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).pick_file()
                    {
                        self.run("vault.import_bitwarden", json!({"path": path.display().to_string()}));
                    }
                });
            }
            SettingsTab::Usage => {
                match self.engine.call("usage.report", json!({})) {
                    Ok(r) => {
                        let rows = r.get("rows").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
                        if rows.is_empty() {
                            ui.label(muted(&t, "Noch keine Token-Zahlen. AIS.chat über die Anmeldung meldet keine."));
                        }
                        egui::Grid::new("usage").striped(true).num_columns(5).spacing([14.0, 6.0]).show(ui, |ui| {
                            for h in ["Monat", "Dienst", "Modell", "Tokens (ein/aus)", "Kosten"] {
                                ui.label(RichText::new(h).strong());
                            }
                            ui.end_row();
                            for row in rows {
                                ui.label(row.get("month").and_then(serde_json::Value::as_str).unwrap_or(""));
                                ui.label(row.get("backend").and_then(serde_json::Value::as_str).unwrap_or(""));
                                ui.label(row.get("model").and_then(serde_json::Value::as_str).unwrap_or(""));
                                ui.label(format!("{} / {}", row.get("input_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0), row.get("output_tokens").and_then(serde_json::Value::as_i64).unwrap_or(0)));
                                ui.label(row.get("usd").and_then(serde_json::Value::as_f64).map(|u| format!("≈ {u:.2} $")).unwrap_or_else(|| "–".into()));
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
                ui.label(crate::kit::caps_text(&t, "Erscheinungsbild"));
                ui.add_space(6.0);
                self.look_cards(ui, true);
                ui.add_space(6.0);
                let (look, _) = crate::theme::parse(&self.state.settings.theme);
                setting_row(ui, &t, "Farbschema", "Hell, dunkel oder wie das System eingestellt ist.", |ui| self.mode_picker(ui));
                if look == crate::theme::Look::Glas {
                    setting_row(ui, &t, "Transparenz", "Glas über dem Verlauf, oder deckende Flächen: ruhiger und auf schwachen Rechnern schneller.", |ui| {
                        self.transparency_picker(ui)
                    });
                }
                setting_row(ui, &t, "Zoom", "Größe von Schrift und Bedienelementen, für Chats, Dokumente und die Seitenleiste.", |ui| {
                    let mut zoom = self.state.settings.zoom;
                    let r = ui.add(egui::Slider::new(&mut zoom, 0.7..=2.0).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    if r.drag_stopped() || (r.changed() && !r.dragged()) {
                        self.run("settings.set", json!({"key": "zoom", "value": zoom}));
                    }
                });
                setting_row(ui, &t, "Enter sendet", "Sonst sendet Strg+Enter, und Enter beginnt eine neue Zeile.", |ui| {
                    let mut enter = self.state.settings.enter_sends;
                    if crate::widgets::toggle(ui, &t, &mut enter, "Enter sendet").changed() {
                        self.run("settings.set", json!({"key": "enter_sends", "value": enter}));
                    }
                });
                ui.add_space(6.0);
                ui.label(small_muted(&t, "Die Akzentfarbe bleibt das AIS-Blau, damit AIS.ChatCraft überall gleich erkennbar ist."));
            }
            SettingsTab::About => {
                ui.label(RichText::new(format!("AIS.ChatCraft {}", env!("CARGO_PKG_VERSION"))).strong());
                ui.label(muted(&t, "Nativer Client für AIS.chat und andere KI-Dienste. Rust und egui, ohne Webview."));
                ui.add_space(8.0);
                community_notice(ui, &t);
                ui.add_space(10.0);
                self.update_settings(ui);
                ui.add_space(10.0);
                ui.label(small_muted(&t, "Tastenkürzel: Strg+K Befehle · Strg+N neuer Chat · Strg+B Seitenleiste · Strg+Umschalt+F Suche · Strg+R neu erzeugen · Esc anhalten"));
                ui.label(small_muted(&t, "Für Agenten: aiscraft --control startet den JSON-Steuerkanal; aiscraft-cli spricht dieselben Befehle."));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.hyperlink_to("Mitmachen", CONTRIBUTING_URL).on_hover_text(CONTRIBUTING_URL);
                    ui.label(small_muted(&t, "·"));
                    ui.hyperlink_to("Fehler melden oder Idee vorschlagen", ISSUES_URL).on_hover_text(ISSUES_URL);
                });
            }
        });
    }
}
/// One row of a settings section: title and line on the left, the control on the right, a
/// hairline under it.
fn setting_row(ui: &mut Ui, t: &crate::theme::Tokens, title: &str, sub: &str, control: impl FnOnce(&mut Ui)) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let w = ui.available_width();
        ui.allocate_ui_with_layout(egui::vec2((w * 0.55).max(160.0), 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(crate::theme::semibold(title).size(15.0).color(t.text));
            ui.add(egui::Label::new(RichText::new(sub).size(13.0).color(t.muted)).wrap());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(12.0);
    let y = ui.cursor().top();
    ui.painter().line_segment([egui::pos2(ui.max_rect().left(), y), egui::pos2(ui.max_rect().right(), y)], egui::Stroke::new(1.0, t.border));
}

fn aiscraft_backends_cookie_name() -> &'static str {
    aiscraft_engine::SESSION_COOKIE
}

fn fmt_time(ms: i64) -> String {
    aiscraft_engine::format_local(ms)
}
