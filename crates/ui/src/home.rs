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
use crate::icons::Icon;
use crate::widgets::{bell_button, muted, plain, small_muted, status_dot};

/// Gap between cards.
const GAP: f32 = 12.0;
/// From this width on, cards sit in two columns.
const TWO_COLUMNS: f32 = 640.0;
/// The start page's text keyline: the inner left padding of the composer, the cards and the
/// entries, and the indent of date, greeting and section titles, so every text block starts on
/// the same x („Text auf Text“).
pub(crate) const KEYLINE: f32 = 20.0;
pub(crate) const KEYLINE_I8: i8 = 20;

/// A card on the start page: the card surface with the keyline as horizontal padding.
fn keyline_card(t: &crate::theme::Tokens, vertical: i8) -> egui::Frame {
    crate::kit::card_frame(t).inner_margin(egui::Margin::symmetric(KEYLINE_I8, vertical))
}

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
        ui.label(crate::theme::bold(title).size(16.0));
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
        let frame = keyline_card(&t, 16);
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(self, ui);
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
                    "{} von {} Token-Punkten verbraucht. Sind sie aufgebraucht, nimmt AIS.chat bis zum nächsten Zeitraum keine Nachrichten mehr an; andere Dienste gehen weiter.",
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
                            let head = RichText::new(plain(&i.title));
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
        let r = bell_button(ui, &t, unread);
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

/// Prompt starters on the designed start page: icon, title, line, and the text they put into
/// the composer.
const STARTERS: [(Icon, &str, &str, &str); 4] = [
    (Icon::Book, "Stundenentwurf", "Photosynthese, Klasse 7", "Stundenentwurf für Klasse 7 zum Thema Photosynthese"),
    (Icon::Table, "Arbeitsblatt", "Brüche addieren, Klasse 6", "Arbeitsblatt zum Addieren von Brüchen für Klasse 6"),
    (Icon::Mail, "Elternbrief", "Klassenfahrt Edersee", "Formuliere einen freundlichen Elternbrief zur Klassenfahrt an den Edersee"),
    (Icon::Quiz, "Quiz", "Zellbestandteile, 5 Fragen", "Erstelle ein Quiz mit 5 Fragen zu den Zellbestandteilen"),
];

const WEEKDAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const MONTHS: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];

/// "Dienstag, 24. November" and the greeting for the hour.
pub(crate) fn date_and_greeting(now: i64) -> (String, &'static str) {
    match aiscraft_engine::local_date_parts(now) {
        Some((wd, day, month, hour)) => {
            let date = format!(
                "{}, {day}. {}",
                WEEKDAYS.get(wd as usize).copied().unwrap_or_default(),
                MONTHS.get(month.saturating_sub(1) as usize).copied().unwrap_or_default()
            );
            let hello = match hour {
                4..=10 => "Guten Morgen",
                11..=17 => "Guten Tag",
                _ => "Guten Abend",
            };
            (date, hello)
        }
        None => (String::new(), "Hallo"),
    }
}

impl AisCraftApp {
    /// The start page of Modern and Glas: date, greeting, the large composer, prompt starters,
    /// favourites from AIS.chat, what was worked on last, then schedules, usage and notices.
    pub(crate) fn home_page(&mut self, ui: &mut Ui, first: Option<String>) {
        let t = self.tokens.clone();
        self.refresh_home_usage();
        self.decode_budget = self.decode_budget.max(4);
        let now = aiscraft_engine::now_ms();
        let (date, hello) = date_and_greeting(now);
        let w = ui.available_width();
        let narrow = w < 640.0;
        let greeting = match &first {
            Some(n) => format!("{hello}, {n}."),
            None => format!("{hello}."),
        };
        let project = self.state.selection.project_id.as_ref().and_then(|pid| self.state.projects.iter().find(|p| &p.id == pid)).map(|p| p.name.clone());
        // Indented by the keyline: the greeting starts where the text inside the boxes starts.
        egui::Frame::new().inner_margin(egui::Margin { left: KEYLINE_I8, ..Default::default() }).show(ui, |ui| {
            ui.label(crate::theme::semibold(date).size(14.0).color(if t.dark { t.link } else { t.accent_ink }));
            ui.add_space(2.0);
            ui.add(egui::Label::new(crate::theme::bold(greeting).size(if narrow { 36.0 } else { 44.0 }).color(t.text)).wrap());
            ui.label(crate::theme::medium("Woran arbeiten wir heute?").size(if narrow { 19.0 } else { 22.0 }).color(t.text2));
            if let Some(name) = project {
                ui.label(muted(&t, format!("Neuer Chat im Projekt {name}")));
            }
        });
        ui.add_space(20.0);
        self.not_connected_card(ui);
        self.composer_big = true;
        self.composer(ui);
        ui.add_space(14.0);
        // Prompt starters: four in a row, two on narrow windows.
        let cols = if narrow { 2 } else { 4 };
        let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).max(80.0);
        for chunk in STARTERS.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (icon, title, line, prompt) in chunk {
                    if starter_card(ui, &t, *icon, title, line, cw).on_hover_text(*prompt).clicked() {
                        self.composer = prompt.to_string();
                        self.focus_composer = true;
                    }
                }
            });
            ui.add_space(GAP);
        }
        self.templates_section(ui);
        ui.add_space(16.0);
        self.favorites_section(ui, w);
        ui.add_space(24.0);
        self.recent_section(ui, w);
        ui.add_space(24.0);
        let unread = self.state.inbox.iter().any(|i| !i.read);
        if unread {
            self.inbox_card(ui);
            ui.add_space(GAP);
        }
        let two = w >= TWO_COLUMNS;
        self.card_row(ui, two, Self::schedules_card, Self::usage_card);
        if !unread && !self.state.inbox.is_empty() {
            ui.add_space(GAP);
            self.inbox_card(ui);
        }
        ui.add_space(24.0);
    }

    /// Before any service is ready: sign in to AIS.chat or add another service.
    pub(crate) fn not_connected_card(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        if self.state.backends.iter().any(|b| matches!(b.status, BackendStatus::Ready { .. })) {
            return;
        }
        let frame = keyline_card(&t, 14);
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(crate::theme::bold("Noch nicht verbunden"));
            ui.label(muted(&t, "Melde dich bei AIS.chat an oder füge einen anderen Dienst hinzu (OpenAI, Claude, Ollama, LM Studio …)."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(ais) = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat)
                    && crate::widgets::accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
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

    /// A section title with a leading icon and an optional link on the right; true when the link
    /// was clicked.
    fn section_title(&mut self, ui: &mut Ui, icon: Icon, icon_color: egui::Color32, title: &str, link: Option<&str>) -> bool {
        let t = self.tokens.clone();
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(KEYLINE);
            ui.label(icon.text(18.0).color(icon_color));
            ui.label(crate::theme::bold(title).size(18.0).color(t.text));
            if let Some(link) = link {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    clicked = crate::kit::pill(ui, &t, crate::kit::Pill::Link, None, link, 14.0, Some(Icon::ChevronRight)).clicked();
                });
            }
        });
        ui.add_space(6.0);
        clicked
    }

    fn favorites_section(&mut self, ui: &mut Ui, w: f32) {
        let t = self.tokens.clone();
        let ais = self.ais_ready();
        if self.section_title(ui, Icon::Star, t.accent2, "Favoriten aus AIS.chat", (!ais.is_empty()).then_some("Alle anzeigen")) {
            self.open_explore();
        }
        if ais.is_empty() {
            keyline_card(&t, 14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(
                    egui::Label::new(muted(
                        &t,
                        "Deine Lieblings-Assistenten, -Dialogpartner und -Lernszenarien aus AIS.chat stehen hier, sobald du angemeldet bist.",
                    ))
                    .wrap(),
                );
                if let Some(id) = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat).map(|b| b.config.id.clone()) {
                    ui.add_space(6.0);
                    if crate::kit::pill(ui, &t, crate::kit::Pill::Soft, None, "Bei AIS.chat anmelden", 13.5, None).clicked() {
                        self.dialog = Some(Dialog::login(&id));
                    }
                }
            });
            return;
        }
        self.prefetch_personas(false);
        let favorites: Vec<(String, PersonaInfo)> =
            ais.iter().flat_map(|b| b.personas.iter().flatten().filter(|p| p.favorite).map(|p| (b.config.id.clone(), p.clone()))).take(8).collect();
        if favorites.is_empty() {
            keyline_card(&t, 14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if ais.iter().all(|b| b.personas.is_none() && b.personas_error.is_none()) {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(small_muted(&t, "Lädt …"));
                    });
                } else if let Some(e) = ais.iter().find_map(|b| b.personas_error.clone()) {
                    ui.add(egui::Label::new(small_muted(&t, e)).wrap());
                } else {
                    ui.add(
                        egui::Label::new(muted(&t, "Noch keine Favoriten. Markiere Assistenten, Dialogpartner oder Lernszenarien mit einem Stern, dann startest du sie hier mit einem Klick."))
                            .wrap(),
                    );
                    ui.add_space(6.0);
                    if crate::kit::pill(ui, &t, crate::kit::Pill::Soft, Some(Icon::Explore), "Entdecken", 13.5, None).clicked() {
                        self.open_explore();
                    }
                }
            });
            return;
        }
        let n = if w < 640.0 { 2 } else { 4 };
        let tile_w = ((w - GAP * (n as f32 - 1.0)) / n as f32).max(60.0);
        for chunk in favorites.chunks(n) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (backend, p) in chunk {
                    if self.favorite_card(ui, backend, p, tile_w).clicked() {
                        self.start_persona(backend, p.kind, &p.id, None);
                    }
                }
            });
            ui.add_space(GAP);
        }
    }

    fn favorite_card(&mut self, ui: &mut Ui, backend: &str, p: &PersonaInfo, w: f32) -> egui::Response {
        let t = self.tokens.clone();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, 78.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &p.name));
        paint_card(ui, &t, rect, resp.hovered() || resp.has_focus());
        let pic = egui::Rect::from_min_size(egui::pos2(rect.left() + KEYLINE, rect.center().y - 21.0), vec2(42.0, 42.0));
        self.paint_persona_picture(ui, pic, backend, p, 21, false);
        let text = egui::Rect::from_min_max(egui::pos2(pic.right() + 12.0, rect.top() + 10.0), egui::pos2(rect.right() - 10.0, rect.bottom() - 8.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
        child.spacing_mut().item_spacing.y = 1.0;
        let name = crate::kit::line(&child, &p.name, crate::kit::font(14.5, crate::theme::BOLD), t.text, text.width());
        let wrap = egui::text::LayoutJob {
            wrap: egui::text::TextWrapping { max_width: text.width(), max_rows: 2, break_anywhere: false, ..Default::default() },
            ..egui::text::LayoutJob::single_section(
                p.name.clone(),
                egui::text::TextFormat { font_id: crate::kit::font(14.5, crate::theme::BOLD), color: t.text, line_height: Some(18.0), ..Default::default() },
            )
        };
        let _ = name;
        let g = child.fonts_mut(|f| f.layout_job(wrap));
        let gh = g.size().y;
        let top = rect.center().y - (gh + 17.0) / 2.0;
        child.painter().galley(egui::pos2(text.left(), top), g, t.text);
        child.painter().text(egui::pos2(text.left(), top + gh + 2.0), egui::Align2::LEFT_TOP, p.kind.label(), crate::kit::font(12.5, ""), t.muted);
        resp.on_hover_text(format!("Neuer Chat: {} {}", p.kind.label(), p.name))
    }

    /// The three things worked on last: chats, documents and whiteboards by their last change.
    fn recent_section(&mut self, ui: &mut Ui, w: f32) {
        let t = self.tokens.clone();
        self.section_title(ui, Icon::History, t.text2, "Zuletzt bearbeitet", None);
        enum Item {
            Chat(String),
            Artifact(String),
        }
        let mut items: Vec<(i64, Icon, String, String, Item)> = Vec::new();
        for c in self.state.conversations.iter().filter(|c| !c.temporary) {
            let title = if c.title.trim().is_empty() { "Ohne Titel".to_string() } else { c.title.clone() };
            items.push((c.updated_at, Icon::Chat, title, "Chat".into(), Item::Chat(c.id.clone())));
        }
        for a in &self.state.artifacts {
            let kind = match a.kind {
                aiscraft_core::ArtifactKind::Document => "Dokument",
                aiscraft_core::ArtifactKind::Board => "Whiteboard",
                aiscraft_core::ArtifactKind::Worksheet => "Arbeitsblatt",
                aiscraft_core::ArtifactKind::App => "Lern-App",
                aiscraft_core::ArtifactKind::H5p => "H5P-Inhalt",
            };
            items.push((a.updated_at, crate::learn_panel::kind_lucide(a.kind), a.title.clone(), kind.into(), Item::Artifact(a.id.clone())));
        }
        items.sort_by_key(|i| std::cmp::Reverse(i.0));
        if items.is_empty() {
            ui.label(muted(&t, "Noch nichts. Schreib oben einfach los."));
            return;
        }
        let now = aiscraft_engine::now_ms();
        let n = if w < 640.0 { 1 } else { 3 };
        let tile_w = ((w - GAP * (n as f32 - 1.0)) / n as f32).max(60.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(GAP, GAP);
            for (at, icon, title, kind, item) in items.into_iter().take(3) {
                let (rect, resp) = ui.allocate_exact_size(vec2(tile_w, 54.0), Sense::click());
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &title));
                let p = ui.painter();
                let hovered = resp.hovered() || resp.has_focus();
                let fill = if t.glass.is_some() || hovered { t.surface } else { egui::Color32::TRANSPARENT };
                let stroke = if t.glass.is_some() { t.card_stroke } else { t.border };
                p.rect(rect, CornerRadius::same(14), fill, egui::Stroke::new(1.0, if hovered { t.accent } else { stroke }), egui::StrokeKind::Inside);
                let tile = egui::Rect::from_min_size(egui::pos2(rect.left() + KEYLINE, rect.center().y - 17.0), vec2(34.0, 34.0));
                let (tf, ti) = if t.glass.is_some() { (t.accent_soft, t.accent_ink) } else { (t.sunken, t.text2) };
                crate::kit::icon_tile(p, tile, 10, tf, icon, 17.0, ti);
                let tw = rect.right() - tile.right() - 22.0;
                let g1 = crate::kit::line(ui, &title, crate::kit::font(14.0, crate::theme::SEMIBOLD), t.text, tw);
                let g2 = crate::kit::line(ui, &format!("{kind} · {}", aiscraft_engine::friendly_time(at, now)), crate::kit::font(12.5, ""), t.muted, tw);
                let x = tile.right() + 11.0;
                p.galley(egui::pos2(x, rect.center().y - 17.0), g1, t.text);
                p.galley(egui::pos2(x, rect.center().y + 1.0), g2, t.muted);
                if resp.clicked() {
                    match &item {
                        Item::Chat(id) => self.run("chat.open", json!({"id": id})),
                        Item::Artifact(id) => {
                            self.save_if_dirty();
                            self.run("artifact.open", json!({"id": id}));
                        }
                    }
                }
            }
        });
    }
}

/// The surface of a clickable card in the designed looks, lit on hover.
fn paint_card(ui: &Ui, t: &crate::theme::Tokens, rect: egui::Rect, hovered: bool) {
    let p = ui.painter();
    let r = CornerRadius::same(t.r_card);
    if t.card_shadow.color.a() > 0 {
        p.add(t.card_shadow.as_shape(rect, r));
    }
    p.rect(rect, r, t.surface, egui::Stroke::new(1.0, if hovered { t.accent } else { t.card_stroke }), egui::StrokeKind::Inside);
    if let Some(g) = &t.glass {
        crate::backdrop::glass_edge(p, rect, f32::from(t.r_card), g.edge);
    }
}

/// A prompt starter: an icon tile, a chevron, the title and a line.
fn starter_card(ui: &mut Ui, t: &crate::theme::Tokens, icon: Icon, title: &str, line: &str, w: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 118.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    paint_card(ui, t, rect, resp.hovered() || resp.has_focus());
    let p = ui.painter();
    let tile = egui::Rect::from_min_size(rect.min + vec2(KEYLINE, 14.0), vec2(36.0, 36.0));
    crate::kit::icon_tile(p, tile, 11, t.accent_soft, icon, 19.0, t.accent_ink);
    Icon::ChevronRight.paint(p, egui::pos2(rect.right() - 22.0, tile.center().y), 16.0, t.muted);
    let g1 = crate::kit::line(ui, title, crate::kit::font(16.0, crate::theme::BOLD), t.text, w - 2.0 * KEYLINE);
    let g2 = crate::kit::line(ui, line, crate::kit::font(13.5, ""), t.muted, w - 2.0 * KEYLINE);
    p.galley(egui::pos2(rect.left() + KEYLINE, rect.bottom() - 52.0), g1, t.text);
    p.galley(egui::pos2(rect.left() + KEYLINE, rect.bottom() - 30.0), g2, t.muted);
    resp
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
            // Start page, sidebar, message actions and header.
            for c in ['×', '📄', '▣', '★', '☆', '+', '•', '?', '⚙', '☰', '⌘', '⟳', '🗐', '🔊', '🎤', '📎', '🌐'] {
                if !ui.fonts_mut(|f| f.has_glyph(&font, c)) {
                    missing.push(c);
                }
            }
        });
        out.textures_delta.clear();
        assert!(missing.is_empty(), "missing glyphs {missing:?}");
    }
}
