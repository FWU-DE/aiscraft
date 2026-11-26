//! The update hint above the account row and the update part of the "Über" tab. The engine
//! checks (`app.check_update`, at start and hourly) and, after „Jetzt aktualisieren“, lets
//! craft-update download, check and install the new version (`app.update_now`, #125); this shows
//! what it found and how far it is, and closes the window for the restart.

use aiscraft_engine::{UpdatePhase, UpdateState};
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, card, small_muted, status_dot};

/// The first line of the release notes that says something (no heading), shortened.
fn summary(notes: &str) -> Option<String> {
    let line = notes.lines().map(|l| l.trim().trim_start_matches(['-', '*']).trim()).find(|l| !l.is_empty() && !l.starts_with('#'))?;
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        s.push_str(" …");
    }
    Some(s)
}

/// What the running update does, in a few words.
fn phase_text(u: &UpdateState) -> Option<(String, Option<f32>)> {
    let p = u.progress.as_ref()?;
    Some(match p.phase {
        UpdatePhase::Download => {
            let share = if p.total > 0 { (p.done as f32 / p.total as f32).clamp(0.0, 1.0) } else { 0.0 };
            let text = if p.total > 0 { format!("Lädt: {} von {} MB", p.done / 1_000_000, p.total.div_ceil(1_000_000)) } else { "Lädt …".to_string() };
            (text, Some(share))
        }
        UpdatePhase::Verify => ("Prüft das Paket …".into(), None),
        UpdatePhase::WaitAnswer => ("Wartet, bis die laufende Antwort fertig ist.".into(), None),
        UpdatePhase::Install => ("Installiert …".into(), None),
        UpdatePhase::Ready => ("Startet neu …".into(), None),
    })
}

impl AisCraftApp {
    /// Each frame: once the update is ready, restart with the composer's text kept for the next
    /// start; once the engine started the restart, close the window.
    pub(crate) fn update_frame(&mut self, ctx: &egui::Context) {
        let u = &self.state.update;
        if u.restart {
            let id = egui::Id::unique("update_closing");
            if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
                ctx.data_mut(|d| d.insert_temp(id, true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        let Some(p) = &u.progress else { return };
        if p.phase != UpdatePhase::Ready || !self.state.generating.is_empty() {
            return;
        }
        // Asked once per version; the engine says in the state if it failed.
        let id = egui::Id::unique("update_restart_asked");
        if ctx.data(|d| d.get_temp::<String>(id)).as_deref() == Some(p.version.as_str()) {
            return;
        }
        ctx.data_mut(|d| d.insert_temp(id, p.version.clone()));
        if let Err(e) = self.engine.call("app.update_restart", json!({"draft": self.composer})) {
            log::warn!("Neustart für das Update: {e}");
        }
    }

    /// A calm card when a newer published release exists or an update runs; nothing otherwise.
    pub(crate) fn update_banner(&mut self, ui: &mut Ui) {
        let u = self.state.update.clone();
        if u.progress.is_some() {
            self.update_progress_card(ui, &u);
            return;
        }
        let Some(a) = u.available.clone() else { return };
        let t = self.tokens.clone();
        card(&t).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, t.accent);
                ui.label(RichText::new(format!("Neue Version {}", a.version)).strong());
            });
            ui.label(small_muted(&t, format!("Du nutzt AIS.ChatCraft {}.", u.current)));
            if let Some(s) = summary(&a.notes) {
                ui.label(small_muted(&t, s));
            }
            if ui.link("Was ist neu").on_hover_text("Öffnet die Release-Seite im Browser").clicked() {
                self.run("app.update_open", json!({"what": "notes"}));
            }
            if let Some(e) = &u.error {
                ui.label(RichText::new(e).small().color(t.danger));
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if u.can_install {
                    let label = if u.error.is_some() { "Erneut versuchen" } else { "Jetzt aktualisieren" };
                    if accent_button(ui, &t, label).on_hover_text("Lädt die neue Version im Hintergrund, prüft sie und startet neu").clicked() {
                        self.run("app.update_now", json!({}));
                    }
                } else if accent_button(ui, &t, "Herunterladen").on_hover_text("Öffnet den Download im Browser").clicked() {
                    self.run("app.update_open", json!({"what": "download"}));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(small_muted(&t, "Später")).frame(false))
                    .on_hover_text("Fragt in 4 Stunden oder beim nächsten Start wieder")
                    .clicked()
                {
                    self.run("app.update_later", json!({}));
                }
                ui.label(small_muted(&t, "·"));
                if ui.add(egui::Button::new(small_muted(&t, "Überspringen")).frame(false)).on_hover_text("Für diese Version nicht mehr fragen").clicked() {
                    self.run("app.update_skip", json!({"version": a.version}));
                }
            });
        });
        ui.add_space(6.0);
    }

    /// The running update: progress, waiting for an answer, „Abbrechen“.
    fn update_progress_card(&mut self, ui: &mut Ui, u: &UpdateState) {
        let Some(p) = &u.progress else { return };
        let Some((text, share)) = phase_text(u) else { return };
        let t = self.tokens.clone();
        card(&t).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                status_dot(ui, t.accent);
                ui.label(RichText::new(format!("Update auf Version {}", p.version)).strong());
            });
            match share {
                Some(share) => {
                    ui.add(egui::ProgressBar::new(share).text(text));
                }
                None => {
                    ui.label(small_muted(&t, text));
                }
            }
            ui.horizontal_wrapped(|ui| {
                if p.phase == UpdatePhase::WaitAnswer
                    && ui.button("Antwort anhalten").on_hover_text("Hält die laufende Antwort an, dann geht das Update weiter").clicked()
                {
                    self.run("chat.stop", json!({}));
                }
                if matches!(p.phase, UpdatePhase::Download | UpdatePhase::Verify | UpdatePhase::WaitAnswer)
                    && ui.add(egui::Button::new(small_muted(&t, "Abbrechen")).frame(false)).clicked()
                {
                    self.run("app.update_cancel", json!({}));
                }
            });
        });
        ui.add_space(6.0);
    }

    /// Version, the switch and "Jetzt prüfen" in the "Über" tab.
    pub(crate) fn update_settings(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let u = self.state.update.clone();
        ui.label(RichText::new("Updates").strong());
        let mut on = self.state.settings.update_check;
        if ui.checkbox(&mut on, "Stündlich nach einer neuen Version suchen").changed() {
            self.run("settings.set", json!({"key": "update_check", "value": on}));
        }
        ui.label(small_muted(
            &t,
            "Dabei geht eine anonyme Anfrage an api.github.com (Releases von FWU-DE/aiscraft). Heruntergeladen und installiert wird nur, wenn du „Jetzt aktualisieren“ wählst.",
        ));
        ui.horizontal(|ui| {
            let button = ui.add_enabled(!u.checking, egui::Button::new(if u.checking { "Prüfe …" } else { "Jetzt prüfen" }));
            if button.clicked() {
                self.run_quiet("app.check_update", json!({}));
            }
            if u.last_check > 0 {
                ui.label(small_muted(&t, format!("Zuletzt geprüft: {}", aiscraft_engine::format_local(u.last_check))));
            }
        });
        if let Some(m) = &u.message {
            ui.label(small_muted(&t, m));
        }
        if let Some((text, share)) = phase_text(&u) {
            match share {
                Some(share) => {
                    ui.add(egui::ProgressBar::new(share).text(text));
                }
                None => {
                    ui.label(small_muted(&t, text));
                }
            }
        } else if let Some(a) = &u.available {
            if let Some(e) = &u.error {
                ui.label(RichText::new(e).small().color(t.danger));
            }
            ui.horizontal_wrapped(|ui| {
                if u.can_install {
                    if accent_button(ui, &t, &format!("Auf Version {} aktualisieren", a.version)).clicked() {
                        self.run("app.update_now", json!({}));
                    }
                } else if accent_button(ui, &t, &format!("Version {} herunterladen", a.version)).clicked() {
                    self.run("app.update_open", json!({"what": "download"}));
                }
                if ui.link("Was ist neu").clicked() {
                    self.run("app.update_open", json!({"what": "notes"}));
                }
            });
            if !u.can_install {
                ui.label(small_muted(&t, "Diese Installation aktualisiert sich nicht selbst; der Download öffnet sich im Browser."));
            }
        } else if let Some(s) = &u.skipped {
            ui.label(small_muted(&t, format!("Übersprungen: Version {s}")));
        }
    }

    /// A command whose error the settings show themselves (no extra notice).
    fn run_quiet(&self, cmd: &str, params: serde_json::Value) {
        self.engine.dispatch_quiet(cmd, params);
    }
}
