//! The start page: what the app shows when no chat is open. Under the greeting and the composer:
//! notifications, favourites from AIS.chat, quick starts, recent chats, schedules and usage.
//! Everything shown comes from the engine's state; nothing here is estimated.

use std::time::{Duration, Instant};

use aiscraft_core::{BackendKind, PersonaInfo};
use aiscraft_engine::{BackendStatus, InboxItem, NoticeLevel};
use egui::{Align, CornerRadius, Layout, RichText, Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::dialogs::{Dialog, SettingsTab};
use crate::widgets::{card, muted, small_muted, status_dot};

/// Gap between cards.
const GAP: f32 = 12.0;
/// From this width on, cards sit in two columns.
const TWO_COLUMNS: f32 = 640.0;

/// Tokens this month per backend, from the usage ledger (`usage.report`), read at most every few
/// seconds instead of every frame.
#[derive(Default)]
pub(crate) struct HomeCache {
    usage: Option<Usage>,
    fetched: Option<(u64, Instant)>,
}

#[derive(Clone)]
struct Usage {
    /// Backend name and tokens (in + out) this month.
    rows: Vec<(String, u64)>,
    total: u64,
    limit: u64,
}

fn usage_from(report: &Value) -> Usage {
    let month = report.get("this_month").and_then(Value::as_str).unwrap_or_default();
    let mut rows: Vec<(String, u64)> = Vec::new();
    for r in report.get("rows").and_then(Value::as_array).into_iter().flatten() {
        if r.get("month").and_then(Value::as_str) != Some(month) {
            continue;
        }
        let name = r.get("backend").and_then(Value::as_str).unwrap_or("?").to_string();
        let n = |k: &str| r.get(k).and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
        let tokens = n("input_tokens").saturating_add(n("output_tokens"));
        match rows.iter_mut().find(|(b, _)| *b == name) {
            Some(row) => row.1 = row.1.saturating_add(tokens),
            None => rows.push((name, tokens)),
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    let total = rows.iter().map(|r| r.1).fold(0u64, u64::saturating_add);
    Usage { rows, total, limit: report.get("limit").and_then(Value::as_u64).unwrap_or(0) }
}

/// "12.345" with German thousands separators.
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// "gerade eben", "vor 5 min", "vor 3 Std." or the date.
pub(crate) fn when(ms: i64, now: i64) -> String {
    let mins = now.saturating_sub(ms) / 60_000;
    match mins {
        i64::MIN..=0 => "gerade eben".into(),
        1..=59 => format!("vor {mins} min"),
        60..=1439 => format!("vor {} Std.", mins / 60),
        _ => aiscraft_engine::format_local(ms),
    }
}

/// A one-line summary without Markdown marks (answers often start with **bold** or a heading).
fn plain(s: &str) -> String {
    s.replace("**", "").replace("__", "").replace('`', "").trim_start_matches('#').trim().to_string()
}

/// A bar for a share used (0..=1), coloured by what is left.
fn bar(ui: &mut Ui, t: &crate::theme::Tokens, used: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), t.surface_hi);
    let used = used.clamp(0.0, 1.0);
    let mut fill = rect;
    fill.set_width((rect.width() * used).max(if used > 0.0 { 3.0 } else { 0.0 }));
    let color = if used > 0.9 {
        t.danger
    } else if used > 0.7 {
        t.warning
    } else {
        t.accent
    };
    ui.painter().rect_filled(fill, CornerRadius::same(3), color);
    resp
}

/// The title row of a card, with an optional link on the right. Returns true when it was clicked.
fn card_head(ui: &mut Ui, t: &crate::theme::Tokens, title: &str, link: Option<&str>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).strong());
        if let Some(link) = link {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = ui.add(egui::Button::new(RichText::new(link).color(t.accent).small()).frame(false)).clicked();
            });
        }
    });
    ui.add_space(4.0);
    clicked
}

impl AisCraftApp {
    fn refresh_home_usage(&mut self) {
        let rev = self.state.revision;
        let fresh = self.home.fetched.is_some_and(|(r, at)| r == rev || at.elapsed() < Duration::from_secs(3));
        if fresh && self.home.usage.is_some() {
            return;
        }
        self.home.fetched = Some((rev, Instant::now()));
        self.home.usage = self.engine.call("usage.report", json!({"months": 1})).ok().map(|r| usage_from(&r));
    }

    fn ais_ready(&self) -> Vec<aiscraft_engine::BackendView> {
        self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. })).cloned().collect()
    }

    /// The cards under the composer on the start page.
    pub(crate) fn home_cards(&mut self, ui: &mut Ui) {
        self.refresh_home_usage();
        self.decode_budget = self.decode_budget.max(4);
        let two = ui.available_width() >= TWO_COLUMNS;
        ui.add_space(24.0);
        let unread = self.state.inbox.iter().any(|i| !i.read);
        if unread {
            self.inbox_card(ui);
            ui.add_space(GAP);
        }
        self.favorites_card(ui);
        ui.add_space(GAP);
        self.card_row(ui, two, Self::quick_card, Self::recent_card);
        ui.add_space(GAP);
        self.card_row(ui, two, Self::schedules_card, Self::usage_card);
        if !unread && !self.state.inbox.is_empty() {
            ui.add_space(GAP);
            self.inbox_card(ui);
        }
        ui.add_space(24.0);
    }

    fn card_row(&mut self, ui: &mut Ui, two: bool, a: fn(&mut Self, &mut Ui), b: fn(&mut Self, &mut Ui)) {
        if two {
            let w = ((ui.available_width() - GAP) / 2.0).max(0.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(w);
                    a(self, ui);
                });
                ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
                    ui.set_width(w);
                    b(self, ui);
                });
            });
        } else {
            a(self, ui);
            ui.add_space(GAP);
            b(self, ui);
        }
    }

    /// A card that fills the width it is given.
    fn home_card(&mut self, ui: &mut Ui, add: impl FnOnce(&mut Self, &mut Ui)) {
        let t = self.tokens.clone();
        card(&t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(self, ui);
        });
    }

    fn favorites_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            let ais = app.ais_ready();
            if card_head(ui, &t, "Favoriten", (!ais.is_empty()).then_some("Alle entdecken")) {
                app.open_explore();
            }
            if ais.is_empty() {
                ui.add(egui::Label::new(muted(&t, "Deine Lieblings-Assistenten, -Figuren und -Lernszenarien aus AIS.chat stehen hier, sobald du angemeldet bist.")).wrap());
                if let Some(id) = app.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat).map(|b| b.config.id.clone()) {
                    ui.add_space(6.0);
                    if ui.button("Bei AIS.chat anmelden").clicked() {
                        app.dialog = Some(Dialog::login(&id));
                    }
                }
                return;
            }
            app.prefetch_personas(false);
            let favorites: Vec<(String, PersonaInfo)> =
                ais.iter().flat_map(|b| b.personas.iter().flatten().filter(|p| p.favorite).map(|p| (b.config.id.clone(), p.clone()))).take(8).collect();
            if favorites.is_empty() {
                if ais.iter().all(|b| b.personas.is_none() && b.personas_error.is_none()) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(small_muted(&t, "Lädt …"));
                    });
                } else if let Some(e) = ais.iter().find_map(|b| b.personas_error.clone()) {
                    ui.add(egui::Label::new(small_muted(&t, e)).wrap());
                } else {
                    ui.add(
                        egui::Label::new(muted(&t, "Noch keine Favoriten. Markiere Assistenten, Figuren oder Lernszenarien mit einem Stern, dann startest du sie hier mit einem Klick."))
                            .wrap(),
                    );
                    ui.add_space(6.0);
                    if ui.button("👥  Entdecken").clicked() {
                        app.open_explore();
                    }
                }
                return;
            }
            let w = ui.available_width();
            let n = (((w + 8.0) / (190.0 + 8.0)).floor() as usize).clamp(1, 4);
            let tile_w = ((w - 8.0 * (n as f32 - 1.0)) / n as f32).max(60.0);
            for chunk in favorites.chunks(n) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (backend, p) in chunk {
                        if app.favorite_tile(ui, backend, p, tile_w).clicked() {
                            app.start_persona(backend, p.kind, &p.id, None);
                        }
                    }
                });
                ui.add_space(4.0);
            }
        });
    }

    fn favorite_tile(&mut self, ui: &mut Ui, backend: &str, p: &PersonaInfo, w: f32) -> egui::Response {
        let t = self.tokens.clone();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 52.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &p.name));
        let hovered = resp.hovered() || resp.has_focus();
        ui.painter().rect(
            rect,
            CornerRadius::same(t.radius),
            if hovered { t.surface_hi } else { t.surface },
            egui::Stroke::new(1.0, if hovered { t.accent } else { t.border }),
            egui::StrokeKind::Inside,
        );
        let pic = egui::Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(36.0, 36.0));
        self.paint_persona_picture(ui, pic, backend, p, 8, false);
        let text = egui::Rect::from_min_max(egui::pos2(pic.right() + 10.0, rect.top() + 7.0), egui::pos2(rect.right() - 8.0, rect.bottom() - 6.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing.y = 1.0;
        child.add(egui::Label::new(RichText::new(&p.name).strong()).truncate().selectable(false));
        child.add(egui::Label::new(small_muted(&t, p.kind.label())).truncate().selectable(false));
        resp.on_hover_text(format!("Neuer Chat: {} {}", p.kind.label(), p.name))
    }

    fn quick_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            card_head(ui, &t, "Neu starten", None);
            let w = ui.available_width();
            let row = |ui: &mut Ui, text: &str, tip: &str| {
                ui.add(egui::Button::new(text).min_size(vec2(w, 28.0)).corner_radius(CornerRadius::same(t.radius.saturating_sub(2)))).on_hover_text(tip)
            };
            if row(ui, "+  Chat", "Neuer Chat (Strg+N); Modell wählst du unten im Eingabefeld").clicked() {
                app.new_chat();
            }
            let sel = app.state.selection.backend_id.clone();
            let image = app
                .state
                .backends
                .iter()
                .filter(|b| matches!(b.status, BackendStatus::Ready { .. }) && !b.image_models.is_empty())
                .min_by_key(|b| b.config.id != sel)
                .map(|b| (b.config.id.clone(), b.config.name.clone()));
            if let Some((id, name)) = image
                && row(ui, "+  Bild", &format!("Bild-Chat mit {name}")).clicked()
            {
                app.explore = None;
                app.run("chat.new_image", json!({"backend": id}));
                app.focus_composer = true;
            }
            if row(ui, "📄  Dokument", "Ein neues, leeres Dokument neben dem Chat").clicked() {
                app.run("artifact.create", json!({"kind": "document"}));
            }
            if row(ui, "▣  Whiteboard", "Ein neues, leeres Whiteboard neben dem Chat").clicked() {
                app.run("artifact.create", json!({"kind": "board"}));
            }
        });
    }

    fn recent_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            card_head(ui, &t, "Zuletzt", None);
            let recent: Vec<_> = {
                let mut all: Vec<_> = app.state.conversations.iter().collect();
                all.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
                all.into_iter().take(5).map(|c| (c.id.clone(), c.title.clone(), c.updated_at)).collect()
            };
            if recent.is_empty() {
                ui.label(muted(&t, "Noch keine Chats. Schreib oben einfach los."));
                return;
            }
            let now = aiscraft_engine::now_ms();
            let w = ui.available_width();
            for (id, title, at) in recent {
                let title = if title.trim().is_empty() { "Ohne Titel".to_string() } else { title };
                let r = ui.add(egui::Button::new(title).right_text(small_muted(&t, when(at, now))).frame(false).truncate().min_size(vec2(w, 24.0)));
                if r.clicked() {
                    app.run("chat.open", json!({"id": id}));
                }
            }
        });
    }

    fn schedules_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            if card_head(ui, &t, "Zeitpläne", (!app.state.schedules.is_empty()).then_some("Alle")) {
                app.dialog = Some(Dialog::Settings { tab: SettingsTab::Schedules, new_note: String::new() });
            }
            let now = aiscraft_engine::now_ms();
            let mut list: Vec<_> = app.state.schedules.iter().map(|s| (s.clone(), aiscraft_engine::schedule_next_run(s, now))).collect();
            // Active ones by their next run first, then the paused ones.
            list.sort_by_key(|(s, next)| (!s.enabled || next.is_none(), next.unwrap_or(i64::MAX)));
            if list.is_empty() {
                ui.add(egui::Label::new(muted(&t, "Lass einen Auftrag regelmäßig laufen, etwa jeden Morgen einen Tagesüberblick.")).wrap());
                ui.add_space(6.0);
            }
            for (s, next) in list.iter().take(4) {
                ui.horizontal(|ui| {
                    status_dot(ui, if s.enabled { t.success } else { t.muted });
                    let detail = match (s.enabled, next) {
                        (false, _) => "pausiert".to_string(),
                        (true, Some(n)) => aiscraft_engine::format_local(*n),
                        (true, None) => "kein weiterer Lauf".to_string(),
                    };
                    let r = ui.add(
                        egui::Button::new(&s.title).right_text(small_muted(&t, detail)).frame(false).truncate().min_size(vec2(ui.available_width(), 22.0)),
                    );
                    if r.on_hover_text(s.prompt.chars().take(200).collect::<String>()).clicked() {
                        match &s.conversation_id {
                            Some(c) => app.run("chat.open", json!({"id": c})),
                            None => app.dialog = Some(Dialog::Settings { tab: SettingsTab::Schedules, new_note: String::new() }),
                        }
                    }
                });
            }
            if list.len() > 4 {
                ui.label(small_muted(&t, format!("und {} weitere", list.len() - 4)));
            }
            if ui.button("+ Zeitplan anlegen").clicked() {
                app.dialog =
                    Some(Dialog::AddSchedule { title: String::new(), prompt: String::new(), when: "0 7 * * 1-5".into(), recurring: true, watch: false });
            }
        });
    }

    fn usage_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            if card_head(ui, &t, "Kontingent & Verbrauch", Some("Details")) {
                app.dialog = Some(Dialog::Settings { tab: SettingsTab::Usage, new_note: String::new() });
            }
            let mut shown = false;
            // AIS.chat token points, as AIS.chat reports them after each answer.
            for b in app.ais_ready() {
                let Some(q) = b.quota else { continue };
                shown = true;
                let used = if q.max > 0.0 { (q.used / q.max) as f32 } else { 1.0 };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{} · Token-Punkte", b.config.name)).small());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(small_muted(&t, format!("{:.0} % übrig", q.left() * 100.0)));
                    });
                });
                bar(ui, &t, used).on_hover_text(format!(
                    "{} von {} Token-Punkten verbraucht. Sind sie aufgebraucht, nimmt AIS.chat bis zum nächsten Zeitraum keine Nachrichten mehr an; andere Backends gehen weiter.",
                    thousands(q.used.max(0.0) as u64),
                    thousands(q.max.max(0.0) as u64)
                ));
                ui.label(small_muted(&t, format!("{} von {} verbraucht", thousands(q.used.max(0.0) as u64), thousands(q.max.max(0.0) as u64))));
                if q.left() <= 0.0 {
                    ui.label(RichText::new("Aufgebraucht: AIS.chat antwortet erst im nächsten Zeitraum wieder.").small().color(t.danger));
                }
                ui.add_space(6.0);
            }
            // The app's own ledger: tokens this month per backend, and the monthly limit.
            if let Some(u) = app.home.usage.clone() {
                if u.limit > 0 {
                    shown = true;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Monatsgrenze dieser App").small());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(small_muted(&t, format!("{} / {}", thousands(u.total), thousands(u.limit))));
                        });
                    });
                    bar(ui, &t, u.total as f32 / u.limit as f32).on_hover_text("Ab der Grenze fragt AIS.ChatCraft vor dem Senden nach.");
                    ui.add_space(4.0);
                }
                if !u.rows.is_empty() {
                    shown = true;
                    ui.label(small_muted(&t, "Tokens diesen Monat"));
                    for (name, n) in u.rows.iter().take(4) {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(small_muted(&t, thousands(*n)));
                            });
                        });
                    }
                    ui.add_space(4.0);
                }
            }
            // Sign-ins that end: the Claude subscription and AIS.chat sessions.
            let now = aiscraft_engine::now_ms();
            for b in app.state.backends.clone() {
                let BackendStatus::Ready { account } = &b.status else { continue };
                let Some(exp) = account.expires.as_deref().filter(|e| !e.trim().is_empty()) else { continue };
                shown = true;
                let at = aiscraft_engine::parse_when(exp, now);
                let text = match at {
                    Some(ms) if ms <= now => format!("{}: Anmeldung abgelaufen", b.config.name),
                    Some(ms) => format!("{}: angemeldet bis {}", b.config.name, aiscraft_engine::format_local(ms)),
                    None => format!("{}: angemeldet bis {exp}", b.config.name),
                };
                let soon = at.is_some_and(|ms| ms - now < 3_600_000);
                ui.horizontal(|ui| {
                    status_dot(ui, if soon { t.warning } else { t.success });
                    ui.add(egui::Label::new(RichText::new(text).small()).truncate());
                });
            }
            if !shown {
                ui.add(egui::Label::new(muted(&t, "Noch kein Verbrauch in diesem Monat.")).wrap());
                if app.home.usage.as_ref().is_some_and(|u| u.limit == 0) {
                    ui.label(small_muted(&t, "Eine Monatsgrenze setzt du unter Details."));
                }
            }
        });
    }

    fn inbox_card(&mut self, ui: &mut Ui) {
        self.home_card(ui, |app, ui| {
            let t = app.tokens.clone();
            let unread = app.state.inbox.iter().filter(|i| !i.read).count();
            let title = if unread > 0 { format!("Benachrichtigungen ({unread} neu)") } else { "Benachrichtigungen".to_string() };
            if card_head(ui, &t, &title, (unread > 0).then_some("Alle gelesen")) {
                app.run("notice.read", json!({}));
            }
            app.inbox_list(ui, 4);
        });
    }

    /// The newest `max` notifications; a click marks one read and runs what it offers.
    pub(crate) fn inbox_list(&mut self, ui: &mut Ui, max: usize) {
        let t = self.tokens.clone();
        let items: Vec<InboxItem> = self.state.inbox.iter().take(max).cloned().collect();
        if items.is_empty() {
            ui.label(muted(&t, "Keine Benachrichtigungen. Zeitpläne, Hinweise und fertige Aufgaben melden sich hier."));
            return;
        }
        let now = aiscraft_engine::now_ms();
        for i in items {
            let color = match i.level {
                NoticeLevel::Error => t.danger,
                NoticeLevel::Warning => t.warning,
                _ if i.read => t.border,
                _ => t.accent,
            };
            let r = ui
                .horizontal_top(|ui| {
                    ui.add_space(2.0);
                    ui.vertical(|ui| {
                        ui.add_space(5.0);
                        status_dot(ui, color);
                    });
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let head = RichText::new(&i.title);
                            ui.label(if i.read { head.color(t.muted) } else { head.strong() });
                            ui.label(small_muted(&t, when(i.at, now)));
                        });
                        ui.add(egui::Label::new(small_muted(&t, plain(&i.text))).wrap().selectable(false));
                    });
                })
                .response
                .interact(Sense::click());
            let r = r.on_hover_text(i.action.as_ref().map(|a| a.label.clone()).unwrap_or_else(|| "Als gelesen markieren".into()));
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if r.clicked() {
                if !i.read {
                    self.run("notice.read", json!({"id": i.id}));
                }
                if let Some(a) = &i.action {
                    self.run(&a.command, a.params.clone());
                }
            }
            ui.add_space(4.0);
        }
        if self.state.inbox.len() > max {
            ui.label(small_muted(&t, format!("und {} ältere", self.state.inbox.len() - max)));
        }
    }

    /// The bell in the header: unread count, and the list as a popover.
    pub(crate) fn inbox_button(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let unread = self.state.inbox.iter().filter(|i| !i.read).count();
        let label = if unread > 0 { RichText::new(format!("🔔 {unread}")).color(t.accent).strong() } else { RichText::new("🔔").color(t.muted) };
        let r = ui.add(egui::Button::new(label.size(14.0)).frame(false)).on_hover_text(if unread > 0 {
            format!("Benachrichtigungen: {unread} neu")
        } else {
            "Benachrichtigungen".to_string()
        });
        egui::Popup::from_toggle_button_response(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(340.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Benachrichtigungen").strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !self.state.inbox.is_empty() && ui.small_button("Leeren").clicked() {
                        self.run("notice.clear", json!({}));
                    }
                    if unread > 0 && ui.small_button("Alle gelesen").clicked() {
                        self.run("notice.read", json!({}));
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| self.inbox_list(ui, 20));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_times_read_well() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1.000");
        assert_eq!(thousands(12_345_678), "12.345.678");
        let now = 10_000_000_000;
        assert_eq!(when(now, now), "gerade eben");
        assert_eq!(when(now - 5 * 60_000, now), "vor 5 min");
        assert_eq!(when(now - 3 * 3_600_000, now), "vor 3 Std.");
        assert_eq!(plain("## Die **Photosynthese** ist `x`"), "Die Photosynthese ist x");
    }

    #[test]
    fn usage_sums_this_month_per_backend() {
        let r = json!({"this_month": "2026-11", "limit": 5000, "rows": [
            {"month": "2026-11", "backend": "Claude", "model": "a", "input_tokens": 100, "output_tokens": 50},
            {"month": "2026-11", "backend": "Claude", "model": "b", "input_tokens": 10, "output_tokens": 5},
            {"month": "2026-11", "backend": "Ollama", "model": "c", "input_tokens": 1000, "output_tokens": -3},
            {"month": "2026-10", "backend": "Claude", "model": "a", "input_tokens": 9999, "output_tokens": 0}
        ]});
        let u = usage_from(&r);
        assert_eq!(u.rows, vec![("Ollama".to_string(), 1000), ("Claude".to_string(), 165)]);
        assert_eq!((u.total, u.limit), (1165, 5000));
        assert!(usage_from(&json!(null)).rows.is_empty());
    }

    /// The symbols the start page and header use exist in the bundled fonts (kittest panics on
    /// missing glyphs, the app would draw boxes).
    #[test]
    fn start_page_glyphs_are_bundled() {
        let ctx = egui::Context::default();
        let font = egui::FontId::proportional(14.0);
        let mut missing = Vec::new();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            for c in ['×', '🔔', '📄', '▣', '👥', '★', '+'] {
                if !ui.fonts_mut(|f| f.has_glyph(&font, c)) {
                    missing.push(c);
                }
            }
        });
        out.textures_delta.clear();
        assert!(missing.is_empty(), "missing glyphs {missing:?}");
    }
}
