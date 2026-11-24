//! Interactive worksheets in the artifact panel: edit mode for the teacher (tasks, solutions,
//! order) and work mode for pupils (fill in, "Prüfen", hints, score). Native egui, laid out
//! like a calm printed page. The format and the scoring live in `aiscraft_core::worksheet`;
//! "Prüfen" uses the same `score` as the `worksheet.check` command.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use aiscraft_core::worksheet::{self, Category, ChoiceOption, ClozePart, Pair, Response, SheetResult, Task, TaskBody, Worksheet};
use egui::{CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, Ui};
use serde_json::json;

use aiscraft_engine::learn::print::{self, Variant};

use crate::app::AisCraftApp;
use crate::theme::Tokens;
use crate::widgets::{accent_button, icon_button, muted, small_muted};

const PAGE_WIDTH: f32 = 720.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Edit,
    Work,
    /// The printed pages as pictures, rendered from the PDF that prints.
    Preview,
}

/// Print preview: the rendered pages of one variant, for one text of the sheet.
struct Preview {
    /// Sheet text, variant and pixel width the pages were rendered for.
    key: (String, bool, u32),
    pages: Result<Vec<egui::TextureHandle>, String>,
}

/// A pupil's answer while working on the sheet.
#[derive(Debug, Clone, PartialEq)]
enum Ans {
    Chosen(Vec<usize>),
    Bool(Option<bool>),
    Gaps(Vec<String>),
    /// For each left item, the pair index whose right side was picked.
    Matches(Vec<Option<usize>>),
    /// Item indices in the order shown.
    Order(Vec<usize>),
    /// For each item of a categorize task, the group picked.
    Assign(Vec<Option<usize>>),
    Text(String),
}

impl Ans {
    fn fresh(task: &Task) -> Ans {
        match &task.body {
            TaskBody::Choice { .. } => Ans::Chosen(Vec::new()),
            TaskBody::TrueFalse { .. } => Ans::Bool(None),
            TaskBody::Cloze { text, .. } => Ans::Gaps(vec![String::new(); worksheet::cloze_gaps(text).len()]),
            TaskBody::Matching { pairs } => Ans::Matches(vec![None; pairs.len()]),
            TaskBody::Ordering { items } => Ans::Order(worksheet::display_order(items.len(), &task.id)),
            TaskBody::Categorize { categories } => Ans::Assign(vec![None; worksheet::category_items(categories).len()]),
            _ => Ans::Text(String::new()),
        }
    }

    fn response(&self) -> Response {
        match self {
            Ans::Chosen(v) => Response::Chosen(v.clone()),
            Ans::Bool(Some(b)) => Response::Bool(*b),
            Ans::Bool(None) => Response::Empty,
            Ans::Gaps(g) => Response::Gaps(g.clone()),
            Ans::Matches(m) => Response::Matches(m.clone()),
            Ans::Order(o) => Response::Order(o.clone()),
            Ans::Assign(a) => Response::Assign(a.clone()),
            Ans::Text(t) => Response::Text(t.clone()),
        }
    }
}

/// What the panel keeps for an open worksheet.
#[derive(Default)]
pub struct SheetState {
    pub mode: Mode,
    /// The text `ws` was parsed from; parsed again when the document text changes.
    pub(crate) src: String,
    pub(crate) ws: Option<Worksheet>,
    pub(crate) error: Option<String>,
    pub(crate) problems: Vec<String>,
    /// An open H5P artifact: its content (package info included); `ws` holds its tasks.
    pub(crate) h5p: Option<aiscraft_core::h5p::H5p>,
    answers: HashMap<String, Ans>,
    pub(crate) result: Option<SheetResult>,
    /// How many of each task's staged hints are revealed.
    hints: HashMap<String, usize>,
    /// Tasks the pupil answered: an ordering left as shown is no answer.
    touched: HashSet<String>,
    /// The preview shows the solution sheet instead of the worksheet.
    preview_solution: bool,
    preview: Option<Preview>,
}

impl SheetState {
    /// Clears a pupil's answers, hints and score.
    pub(crate) fn reset(&mut self) {
        self.answers.clear();
        self.result = None;
        self.hints.clear();
        self.touched.clear();
    }

    /// Scores the answers given ("Prüfen").
    pub(crate) fn check(&mut self) {
        if let Some(ws) = &self.ws {
            let answers: HashMap<String, Response> =
                self.answers.iter().filter(|(k, _)| self.touched.contains(*k)).map(|(k, a)| (k.clone(), a.response())).collect();
            self.result = Some(worksheet::score(ws, &answers));
        }
    }

    fn sync(&mut self, doc: &str) {
        self.sync_with(doc, worksheet::parse_lenient);
    }

    /// Like `sync`, for H5P content: the tasks as a worksheet, the content kept in `h5p`.
    pub(crate) fn sync_h5p(&mut self, doc: &str) {
        if (self.ws.is_some() || self.error.is_some() || self.h5p.is_some()) && self.src == doc {
            return;
        }
        let parsed = aiscraft_core::h5p::parse_stored(doc);
        self.h5p = parsed.as_ref().ok().map(|(h, _)| h.clone());
        self.sync_with(doc, |_| parsed.clone().map(|(h, problems)| (aiscraft_core::h5p::as_worksheet(&h), problems)));
    }

    fn sync_with(&mut self, doc: &str, parse: impl FnOnce(&str) -> Result<(Worksheet, Vec<String>), String>) {
        if self.ws.is_some() && self.src == doc {
            return;
        }
        if self.error.is_some() && self.src == doc {
            return;
        }
        self.src = doc.to_string();
        match parse(doc) {
            Ok((ws, problems)) => {
                self.ws = Some(ws);
                self.problems = problems;
                self.error = None;
            }
            Err(e) => {
                self.ws = None;
                self.error = Some(e);
            }
        }
        // Answers that no longer fit their task are dropped.
        if let Some(ws) = &self.ws {
            self.answers
                .retain(|id, a| ws.tasks.iter().find(|t| &t.id == id).is_some_and(|t| std::mem::discriminant(a) == std::mem::discriminant(&Ans::fresh(t))));
        }
        self.result = None;
    }
}

pub(crate) fn page<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = avail.min(PAGE_WIDTH);
    let pad = ((avail - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
    })
    .inner
}

fn task_card<R>(ui: &mut Ui, t: &Tokens, stroke: egui::Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(t.radius.saturating_add(2)))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn points_text(p: f64) -> String {
    let s = if (p - p.round()).abs() < 1e-9 { format!("{}", p.round() as i64) } else { format!("{p:.1}").replace('.', ",") };
    if s == "1" { "1 Punkt".into() } else { format!("{s} Punkte") }
}

impl AisCraftApp {
    pub(crate) fn worksheet_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(v) = self.artifact.as_mut() else { return };
        v.sheet.sync(&v.doc);
        let id = v.id.clone();
        let mut mode = v.sheet.mode;
        // Toolbar: mode, print.
        let mut print: Option<(&str, &str, bool)> = None;
        let (mut check, mut reset) = (false, false);
        Frame::new().inner_margin(Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut mode, Mode::Edit, "Bearbeiten");
                ui.selectable_value(&mut mode, Mode::Work, "Lösen");
                ui.selectable_value(&mut mode, Mode::Preview, "Druckvorschau").on_hover_text("Die Seiten so, wie sie gedruckt werden");
                ui.separator();
                if mode == Mode::Work {
                    check = accent_button(ui, &t, "Prüfen").on_hover_text("Antworten prüfen und Punkte zählen").clicked();
                    reset = ui.button("Zurücksetzen").on_hover_text("Alle Antworten löschen").clicked();
                    ui.separator();
                }
                ui.menu_button(RichText::new("Drucken  ⏷").color(t.text), |ui| {
                    ui.set_min_width(220.0);
                    if ui.button("Arbeitsblatt drucken").on_hover_text("Als PDF im PDF-Programm öffnen").clicked() {
                        print = Some(("print", "arbeitsblatt", false));
                        ui.close();
                    }
                    if ui.button("Lösungsblatt drucken").clicked() {
                        print = Some(("print", "loesung", false));
                        ui.close();
                    }
                    ui.separator();
                    for (label, format, variant) in [
                        ("Arbeitsblatt als PDF …", "pdf", "arbeitsblatt"),
                        ("Lösungsblatt als PDF …", "pdf", "loesung"),
                        ("Arbeitsblatt als Word …", "docx", "arbeitsblatt"),
                        ("Lösungsblatt als Word …", "docx", "loesung"),
                    ] {
                        if ui.button(label).clicked() {
                            print = Some((format, variant, true));
                            ui.close();
                        }
                    }
                });
            });
        });
        if let Some(v) = self.artifact.as_mut() {
            v.sheet.mode = mode;
            if reset {
                v.sheet.reset();
            }
            if check {
                v.sheet.check();
            }
        }
        if let Some((what, variant, save)) = print {
            self.save_if_dirty();
            if save {
                let title = self.state.artifacts.iter().find(|a| a.id == id).map(|a| a.title.clone()).unwrap_or_default();
                let suffix = if variant == "loesung" { " Lösungen" } else { "" };
                let name = aiscraft_engine::office::file_name(&format!("{title}{suffix}"), what);
                if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
                    self.run("worksheet.export", json!({"id": id, "format": what, "variant": variant, "path": path.display().to_string()}));
                }
            } else {
                self.run("worksheet.print", json!({"id": id, "variant": variant}));
            }
        }
        self.revise_bar(ui, "Mit KI überarbeiten, z. B. „Zwei Aufgaben mehr, leichter“");
        let Some(v) = self.artifact.as_mut() else { return };
        let mut changed = false;
        ScrollArea::vertical().id_salt(("sheet", id.clone())).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
                page(ui, |ui| {
                    if let Some(err) = &v.sheet.error {
                        ui.label(RichText::new(format!("Das Arbeitsblatt lässt sich nicht lesen: {err}")).color(t.danger));
                        ui.label(small_muted(&t, "Eine frühere Fassung liegt unter Versionen (⟳ oben)."));
                        return;
                    }
                    let sheet = &mut v.sheet;
                    match sheet.mode {
                        Mode::Edit => {
                            if let Some(ws) = sheet.ws.as_mut() {
                                changed = edit_sheet(ui, &t, ws, false);
                            }
                            for p in sheet.problems.iter().take(6) {
                                ui.label(RichText::new(p).small().color(t.warning));
                            }
                        }
                        Mode::Work => work_sheet(ui, &t, sheet),
                        Mode::Preview => preview_sheet(ui, &t, sheet),
                    }
                });
            });
        });
        if changed && let Some(ws) = v.sheet.ws.as_mut() {
            worksheet::normalize_ids(ws);
            let json = worksheet::to_json(ws);
            v.sheet.problems = worksheet::validate(ws);
            v.sheet.src = json.clone();
            v.doc = json;
            v.dirty_since = Some(Instant::now());
        }
        self.autosave(ui);
    }
}

// ---- edit mode -------------------------------------------------------------------------------

enum Move {
    Up(usize),
    Down(usize),
    Remove(usize),
}

fn text_line(ui: &mut Ui, s: &mut String, hint: &str) -> bool {
    ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(ui.available_width())).changed()
}

fn text_area(ui: &mut Ui, s: &mut String, hint: &str, rows: usize) -> bool {
    ui.add(egui::TextEdit::multiline(s).hint_text(hint).desired_rows(rows).desired_width(ui.available_width())).changed()
}

/// A list of strings with remove buttons and an add button. Returns changed.
fn string_list(ui: &mut Ui, t: &Tokens, items: &mut Vec<String>, add_label: &str, placeholder: &str, max: usize, reorder: bool) -> bool {
    let mut changed = false;
    let mut action = None;
    let n = items.len();
    for (i, s) in items.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if reorder {
                ui.label(muted(t, format!("{}.", i + 1)));
            }
            let w = ui.available_width() - if reorder { 90.0 } else { 34.0 };
            changed |= ui.add(egui::TextEdit::singleline(s).hint_text(placeholder).desired_width(w.max(60.0))).changed();
            if reorder {
                if ui.add_enabled(i > 0, egui::Button::new("⏶").frame(false)).on_hover_text("Nach oben").clicked() {
                    action = Some(Move::Up(i));
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new("⏷").frame(false)).on_hover_text("Nach unten").clicked() {
                    action = Some(Move::Down(i));
                }
            }
            if icon_button(ui, t, "×", "Entfernen").clicked() {
                action = Some(Move::Remove(i));
            }
        });
    }
    match action {
        Some(Move::Up(i)) if i > 0 => items.swap(i, i - 1),
        Some(Move::Down(i)) if i + 1 < items.len() => items.swap(i, i + 1),
        Some(Move::Remove(i)) if i < items.len() => {
            items.remove(i);
        }
        _ => {}
    }
    changed |= action.is_some();
    if items.len() < max && ui.small_button(add_label).clicked() {
        items.push(String::new());
        changed = true;
    }
    changed
}

/// The task list editor. `h5p`: for H5P content (no subject, grade or points; only the task
/// types H5P has; tips only where H5P shows them).
pub(crate) fn edit_sheet(ui: &mut Ui, t: &Tokens, ws: &mut Worksheet, h5p: bool) -> bool {
    let mut changed = false;
    ui.add_space(4.0);
    changed |=
        ui.add(egui::TextEdit::singleline(&mut ws.title).font(egui::TextStyle::Heading).hint_text("Titel").desired_width(ui.available_width())).changed();
    if !h5p {
        ui.horizontal(|ui| {
            let l = ui.label(muted(t, "Fach"));
            changed |= ui.add(egui::TextEdit::singleline(&mut ws.subject).hint_text("z. B. Mathematik").desired_width(180.0)).labelled_by(l.id).changed();
            let l = ui.label(muted(t, "Klasse"));
            changed |= ui.add(egui::TextEdit::singleline(&mut ws.grade).hint_text("6").desired_width(60.0)).labelled_by(l.id).changed();
        });
    }
    let intro_hint = if h5p { "Einleitung auf der Startseite des Quiz (optional)" } else { "Arbeitsauftrag für alle Aufgaben (optional)" };
    changed |= text_area(ui, &mut ws.instructions, intro_hint, 2);
    ui.add_space(8.0);
    let mut action = None;
    let n = ws.tasks.len();
    let mut number = 0;
    for (i, task) in ws.tasks.iter_mut().enumerate() {
        let info = matches!(task.body, TaskBody::Info { .. });
        if !info {
            number += 1;
        }
        task_card(ui, t, t.border, |ui| {
            ui.horizontal(|ui| {
                let label = if h5p { aiscraft_core::h5p::task_label(&task.body) } else { task.body.label() };
                let head = if info { label.to_string() } else { format!("Aufgabe {number} · {label}") };
                ui.label(RichText::new(head).strong().color(t.accent));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, t, "×", "Aufgabe entfernen").clicked() {
                        action = Some(Move::Remove(i));
                    }
                    if ui.add_enabled(i + 1 < n, egui::Button::new("⏷").frame(false)).on_hover_text("Nach unten").clicked() {
                        action = Some(Move::Down(i));
                    }
                    if ui.add_enabled(i > 0, egui::Button::new("⏶").frame(false)).on_hover_text("Nach oben").clicked() {
                        action = Some(Move::Up(i));
                    }
                    if !info && !h5p {
                        let l = ui.label(muted(t, "Punkte"));
                        changed |= ui
                            .add(egui::DragValue::new(&mut task.points).range(0.0..=worksheet::MAX_POINTS).speed(0.5).max_decimals(1))
                            .labelled_by(l.id)
                            .changed();
                    }
                });
            });
            if !info || !task.prompt.is_empty() {
                changed |= text_area(ui, &mut task.prompt, if info { "Überschrift (optional)" } else { "Aufgabenstellung" }, 1);
            }
            ui.add_space(4.0);
            changed |= edit_body(ui, t, &mut task.body, h5p);
            if !info {
                let head = if h5p { "Rückmeldung" } else { "Tipps und Erklärung" };
                egui::CollapsingHeader::new(small_muted(t, head)).id_salt(("hint", i)).default_open(!task.hints.is_empty() || !task.feedback.is_empty()).show(
                    ui,
                    |ui| {
                        if !h5p {
                            ui.label(small_muted(t, "Tipps werden beim Lösen einzeln aufgedeckt. Fang allgemein an und werde mit jedem Tipp konkreter."));
                            changed |= string_list(ui, t, &mut task.hints, "+ Tipp", "Tipp", worksheet::MAX_HINTS, true);
                        }
                        changed |= text_line(ui, &mut task.feedback, "Erklärung, die nach dem Prüfen erscheint");
                        if let TaskBody::Choice { options, .. } = &mut task.body {
                            ui.label(small_muted(t, "Rückmeldung je Antwort (erscheint nach dem Prüfen, wenn gewählt)"));
                            for o in options.iter_mut() {
                                ui.horizontal(|ui| {
                                    let name: String = o.text.chars().take(24).collect();
                                    ui.label(muted(t, if name.is_empty() { "Antwort".to_string() } else { name }));
                                    changed |= text_line(ui, &mut o.feedback, "z. B. welche Fehlvorstellung dahintersteckt");
                                });
                            }
                        }
                    },
                );
            }
        });
        ui.add_space(8.0);
    }
    match action {
        Some(Move::Up(i)) if i > 0 => ws.tasks.swap(i, i - 1),
        Some(Move::Down(i)) if i + 1 < ws.tasks.len() => ws.tasks.swap(i, i + 1),
        Some(Move::Remove(i)) if i < ws.tasks.len() => {
            ws.tasks.remove(i);
        }
        _ => {}
    }
    changed |= action.is_some();
    if ws.tasks.len() < worksheet::MAX_TASKS {
        ui.menu_button(RichText::new("+ Aufgabe hinzufügen  ⏷").color(t.accent), |ui| {
            let types = if h5p { aiscraft_core::h5p::TASK_TYPES } else { worksheet::TASK_TYPES };
            for (kind, label) in types {
                if ui.button(*label).clicked() {
                    let body = if h5p { aiscraft_core::h5p::template(kind) } else { TaskBody::template(kind) };
                    if let Some(body) = body {
                        ws.tasks.push(Task::new("", body));
                        changed = true;
                    }
                    ui.close();
                }
            }
        });
    }
    changed
}

fn edit_body(ui: &mut Ui, t: &Tokens, body: &mut TaskBody, h5p: bool) -> bool {
    let mut changed = false;
    match body {
        TaskBody::Info { text } => changed |= text_area(ui, text, "Text", 4),
        TaskBody::Choice { options, multiple } => {
            changed |= ui.checkbox(multiple, "Mehrere Antworten richtig").changed();
            let mut remove = None;
            for (i, o) in options.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let c = ui.checkbox(&mut o.correct, "richtig").on_hover_text("Richtige Antwort");
                    changed |= c.changed();
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut o.text).hint_text("Antwortmöglichkeit").desired_width((ui.available_width() - 34.0).max(60.0)))
                        .changed();
                    if icon_button(ui, t, "×", "Entfernen").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove.filter(|i| *i < options.len()) {
                options.remove(i);
                changed = true;
            }
            if options.len() < worksheet::MAX_OPTIONS && ui.small_button("+ Antwort").clicked() {
                options.push(ChoiceOption::new("", false));
                changed = true;
            }
        }
        TaskBody::TrueFalse { answer } => {
            ui.horizontal(|ui| {
                ui.label(muted(t, "Lösung:"));
                changed |= ui.radio_value(answer, true, "richtig").changed();
                changed |= ui.radio_value(answer, false, "falsch").changed();
            });
        }
        TaskBody::Cloze { text, word_bank, distractors } => {
            changed |= text_area(ui, text, "Text mit [[Lösung|Alternative]] für jede Lücke", 3);
            let gaps = worksheet::cloze_gaps(text).len();
            ui.label(small_muted(t, format!("{gaps} Lücken · Lücken so markieren: [[Lösung|Alternative]]")));
            if h5p {
                changed |= ui
                    .checkbox(word_bank, "Wörter in die Lücken ziehen (statt eintippen)")
                    .on_hover_text("H5P.DragText statt H5P.Blanks; zählt nur die erste Lösung jeder Lücke")
                    .changed();
            } else {
                changed |= ui.checkbox(word_bank, "Wortliste zeigen").on_hover_text("Die Lösungswörter gemischt über dem Text, leichter für Jüngere").changed();
            }
            if *word_bank {
                ui.label(small_muted(t, "Zusätzliche falsche Wörter (optional)"));
                changed |= string_list(ui, t, distractors, "+ falsches Wort", "Ablenker", worksheet::MAX_DISTRACTORS, false);
            }
        }
        TaskBody::Matching { pairs } => {
            let mut remove = None;
            for (i, p) in pairs.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let w = ((ui.available_width() - 60.0) / 2.0).max(60.0);
                    changed |= ui.add(egui::TextEdit::singleline(&mut p.left).hint_text("Begriff").desired_width(w)).changed();
                    ui.label(muted(t, "–"));
                    changed |= ui.add(egui::TextEdit::singleline(&mut p.right).hint_text("passt zu").desired_width(w)).changed();
                    if icon_button(ui, t, "×", "Entfernen").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove.filter(|i| *i < pairs.len()) {
                pairs.remove(i);
                changed = true;
            }
            if pairs.len() < worksheet::MAX_PAIRS && ui.small_button("+ Paar").clicked() {
                pairs.push(Pair { left: String::new(), right: String::new() });
                changed = true;
            }
        }
        TaskBody::Ordering { items } => {
            ui.label(small_muted(t, "In der richtigen Reihenfolge eintragen; beim Lösen erscheinen sie gemischt."));
            changed |= string_list(ui, t, items, "+ Element", "Element", worksheet::MAX_ITEMS, true);
        }
        TaskBody::Categorize { categories } => {
            ui.label(small_muted(t, "Jede Gruppe mit den Elementen, die hineingehören; beim Lösen erscheinen alle gemischt."));
            let mut remove = None;
            let n = categories.len();
            for (i, c) in categories.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |=
                        ui.add(egui::TextEdit::singleline(&mut c.name).hint_text("Gruppe").desired_width((ui.available_width() - 34.0).max(60.0))).changed();
                    if n > 2 && icon_button(ui, t, "×", "Gruppe entfernen").clicked() {
                        remove = Some(i);
                    }
                });
                ui.indent(("group", i), |ui| {
                    changed |= string_list(ui, t, &mut c.items, "+ Element", "Element", worksheet::MAX_ITEMS, false);
                });
            }
            if let Some(i) = remove.filter(|i| *i < categories.len()) {
                categories.remove(i);
                changed = true;
            }
            if categories.len() < worksheet::MAX_CATEGORIES && ui.small_button("+ Gruppe").clicked() {
                categories.push(Category { name: String::new(), items: Vec::new() });
                changed = true;
            }
        }
        TaskBody::ShortAnswer { accepted, case_sensitive } => {
            ui.label(small_muted(t, "Akzeptierte Antworten"));
            changed |= string_list(ui, t, accepted, "+ Variante", "Antwort", worksheet::MAX_ALTERNATIVES, false);
            changed |= ui.checkbox(case_sensitive, "Groß- und Kleinschreibung beachten").changed();
        }
        TaskBody::Numeric { answer, tolerance, unit } => {
            ui.horizontal(|ui| {
                let l = ui.label(muted(t, "Lösung"));
                changed |= ui.add(egui::DragValue::new(answer).speed(0.1).max_decimals(4)).labelled_by(l.id).changed();
                let l = ui.label(muted(t, "± Toleranz"));
                changed |= ui.add(egui::DragValue::new(tolerance).range(0.0..=f64::MAX).speed(0.01).max_decimals(4)).labelled_by(l.id).changed();
                let l = ui.label(muted(t, "Einheit"));
                changed |= ui.add(egui::TextEdit::singleline(unit).hint_text("cm").desired_width(60.0)).labelled_by(l.id).changed();
            });
        }
        TaskBody::FreeText { sample, rubric, lines } => {
            changed |= text_area(ui, sample, "Musterlösung", 2);
            changed |= text_area(ui, rubric, "Bewertungskriterien (Erwartungshorizont)", 2);
            ui.horizontal(|ui| {
                let l = ui.label(muted(t, "Schreibzeilen auf dem Ausdruck"));
                changed |= ui.add(egui::DragValue::new(lines).range(1..=worksheet::MAX_LINES)).labelled_by(l.id).changed();
            });
        }
    }
    changed
}

// ---- work mode -------------------------------------------------------------------------------

// ---- print preview ---------------------------------------------------------------------------

/// Pixel width the preview pages are rendered at: sharp on high-density screens, rounded so a
/// slowly resized panel does not render again on every frame.
fn preview_px(ui: &Ui) -> u32 {
    let px = ui.available_width() * ui.ctx().pixels_per_point();
    ((px / 200.0).ceil() * 200.0).clamp(400.0, 2400.0) as u32
}

fn preview_sheet(ui: &mut Ui, t: &Tokens, sheet: &mut SheetState) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut sheet.preview_solution, false, "Arbeitsblatt");
        ui.selectable_value(&mut sheet.preview_solution, true, "Lösungsblatt");
    });
    ui.add_space(8.0);
    let Some(ws) = &sheet.ws else { return };
    let px = preview_px(ui);
    let key = (sheet.src.clone(), sheet.preview_solution, px);
    if sheet.preview.as_ref().is_none_or(|p| p.key != key) {
        let variant = if sheet.preview_solution { Variant::Solution } else { Variant::Sheet };
        let pages = print::worksheet_preview(ws, variant, px)
            .map(|pages| {
                pages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let image = egui::ColorImage::from_rgba_unmultiplied([p.width as usize, p.height as usize], &p.rgba);
                        ui.ctx().load_texture(format!("sheet-preview-{i}"), image, egui::TextureOptions::LINEAR)
                    })
                    .collect()
            })
            .map_err(|e| e.to_string());
        sheet.preview = Some(Preview { key, pages });
    }
    let Some(preview) = &sheet.preview else { return };
    match &preview.pages {
        Err(e) => {
            ui.label(RichText::new(format!("Die Vorschau lässt sich nicht zeichnen: {e}")).color(t.danger));
        }
        Ok(pages) => {
            let w = ui.available_width();
            for (i, tex) in pages.iter().enumerate() {
                let [tw, th] = tex.size();
                let h = w * th as f32 / tw.max(1) as f32;
                Frame::new()
                    .stroke(Stroke::new(1.0, t.border))
                    .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: egui::Color32::from_black_alpha(28) })
                    .show(ui, |ui| {
                        ui.add(egui::Image::new((tex.id(), egui::vec2(w - 2.0, h))));
                    });
                ui.label(small_muted(t, format!("Seite {} von {}", i + 1, pages.len())));
                ui.add_space(12.0);
            }
            if pages.len() >= print::MAX_PREVIEW_PAGES {
                ui.label(small_muted(t, format!("Die Vorschau zeigt die ersten {} Seiten.", print::MAX_PREVIEW_PAGES)));
            }
        }
    }
}

pub(crate) fn work_sheet(ui: &mut Ui, t: &Tokens, sheet: &mut SheetState) {
    let Some(ws) = sheet.ws.clone() else { return };
    ui.add_space(4.0);
    ui.label(RichText::new(&ws.title).heading().strong());
    let meta: Vec<String> =
        [("Fach", &ws.subject), ("Klasse", &ws.grade)].iter().filter(|(_, x)| !x.trim().is_empty()).map(|(k, x)| format!("{k}: {x}")).collect();
    if !meta.is_empty() {
        ui.label(muted(t, meta.join(" · ")));
    }
    if !ws.instructions.trim().is_empty() {
        ui.label(RichText::new(&ws.instructions).italics());
    }
    if let Some(r) = &sheet.result {
        ui.add_space(6.0);
        let full = r.max > 0.0 && (r.points - r.max).abs() < 1e-9;
        Frame::new()
            .fill(if full { t.success.gamma_multiply(0.15) } else { t.accent.gamma_multiply(0.12) })
            .corner_radius(CornerRadius::same(t.radius))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mut s = format!("{} von {}", points_text(r.points).replace(" Punkte", "").replace(" Punkt", ""), points_text(r.max));
                if r.open > 0 {
                    s.push_str(&format!(" · {} Aufgabe{} bewertet die Lehrkraft", r.open, if r.open == 1 { "" } else { "n" }));
                }
                ui.label(RichText::new(s).strong());
            });
    }
    ui.add_space(10.0);
    let results: HashMap<String, worksheet::TaskResult> =
        sheet.result.as_ref().map(|r| r.results.iter().map(|x| (x.id.clone(), x.clone())).collect()).unwrap_or_default();
    let checked = sheet.result.is_some();
    let mut number = 0;
    let mut touched = false;
    for task in &ws.tasks {
        let info = matches!(task.body, TaskBody::Info { .. });
        if !info {
            number += 1;
        }
        let res = results.get(&task.id);
        let stroke = match res.and_then(|r| r.correct) {
            Some(true) => t.success,
            Some(false) if res.is_some_and(|r| r.points > 0.0) => t.warning,
            Some(false) => t.danger,
            None => t.border,
        };
        let ans = sheet.answers.entry(task.id.clone()).or_insert_with(|| Ans::fresh(task));
        task_card(ui, t, stroke, |ui| {
            if info {
                if !task.prompt.is_empty() {
                    ui.label(RichText::new(&task.prompt).strong());
                }
                if let TaskBody::Info { text } = &task.body {
                    ui.label(text);
                }
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{number}.")).strong().color(t.accent));
                ui.label(RichText::new(&task.prompt).strong());
                ui.label(small_muted(t, format!("({})", points_text(task.points))));
            });
            if let Some(img) = &task.image {
                ui.label(small_muted(t, format!("Bild: {img}")));
            }
            ui.add_space(4.0);
            if work_task(ui, t, task, ans, res, checked) {
                touched = true;
                sheet.touched.insert(task.id.clone());
            }
            ui.horizontal_wrapped(|ui| {
                let total = task.hints.len();
                if total > 0 {
                    let shown = sheet.hints.get(&task.id).copied().unwrap_or(0).min(total);
                    let label = match (shown, total) {
                        (0, 1) => "💡 Tipp".to_string(),
                        (0, _) => format!("💡 Tipp (1 von {total})"),
                        (s, _) if s < total => format!("💡 Nächster Tipp ({} von {total})", s + 1),
                        _ => "💡 Tipps ausblenden".to_string(),
                    };
                    if ui.add(egui::Button::new(small_muted(t, label)).frame(false)).clicked() {
                        let next = if shown >= total { 0 } else { shown + 1 };
                        sheet.hints.insert(task.id.clone(), next);
                    }
                }
                if let Some(r) = res {
                    let (label, color) = match r.correct {
                        Some(true) => ("✔ Richtig".to_string(), t.success),
                        Some(false) if r.points > 0.0 => (
                            format!("Teilweise richtig: {} von {}", points_text(r.points).replace(" Punkte", "").replace(" Punkt", ""), points_text(r.max)),
                            t.warning,
                        ),
                        Some(false) => ("❌ Noch nicht richtig".to_string(), t.danger),
                        None => ("Bewertet die Lehrkraft".to_string(), t.muted),
                    };
                    ui.label(RichText::new(label).color(color).strong());
                }
            });
            let shown = sheet.hints.get(&task.id).copied().unwrap_or(0);
            for (k, h) in task.hints.iter().take(shown).enumerate() {
                let text = if task.hints.len() > 1 { format!("Tipp {}: {h}", k + 1) } else { h.clone() };
                ui.label(RichText::new(text).italics().color(t.muted));
            }
            if checked && !task.feedback.trim().is_empty() {
                ui.label(RichText::new(&task.feedback).color(t.muted));
            }
            if checked
                && let TaskBody::FreeText { sample, .. } = &task.body
                && !sample.trim().is_empty()
            {
                ui.label(small_muted(t, format!("Musterlösung: {sample}")));
            }
        });
        ui.add_space(8.0);
    }
    if touched && sheet.result.is_some() {
        // A changed answer makes the old check stale.
        sheet.result = None;
    }
    ui.add_space(12.0);
}

fn part_color(t: &Tokens, res: Option<&worksheet::TaskResult>, i: usize) -> Option<egui::Color32> {
    res.and_then(|r| r.parts.get(i)).map(|ok| if *ok { t.success } else { t.danger })
}

/// One task's answer widgets. Returns whether the answer changed.
fn work_task(ui: &mut Ui, t: &Tokens, task: &Task, ans: &mut Ans, res: Option<&worksheet::TaskResult>, checked: bool) -> bool {
    let mut changed = false;
    match (&task.body, ans) {
        (TaskBody::Choice { options, multiple }, Ans::Chosen(chosen)) => {
            for (i, o) in options.iter().enumerate() {
                let mut on = chosen.contains(&i);
                let mut text = RichText::new(&o.text);
                if checked && o.correct {
                    text = text.color(t.success);
                } else if checked && on && !o.correct {
                    text = text.color(t.danger);
                }
                let r = if *multiple { ui.checkbox(&mut on, text) } else { ui.radio(on, text) };
                if checked && on && !o.feedback.trim().is_empty() {
                    ui.indent(("optfb", &task.id, i), |ui| ui.label(small_muted(t, &o.feedback)));
                }
                if r.clicked() {
                    if *multiple {
                        if on {
                            chosen.push(i);
                        } else {
                            chosen.retain(|x| *x != i);
                        }
                    } else {
                        *chosen = vec![i];
                    }
                    changed = true;
                }
            }
        }
        (TaskBody::TrueFalse { .. }, Ans::Bool(b)) => {
            ui.horizontal(|ui| {
                if ui.radio(*b == Some(true), "richtig").clicked() {
                    *b = Some(true);
                    changed = true;
                }
                if ui.radio(*b == Some(false), "falsch").clicked() {
                    *b = Some(false);
                    changed = true;
                }
            });
        }
        (TaskBody::Cloze { text, word_bank, distractors }, Ans::Gaps(gaps)) => {
            if *word_bank {
                let words = worksheet::cloze_word_bank(text, distractors, &task.id);
                Frame::new().stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::symmetric(10, 6)).show(
                    ui,
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(small_muted(t, "Wörter:"));
                            for w in &words {
                                // Words already used in a gap are greyed out.
                                let used = gaps.iter().any(|g| g.trim().eq_ignore_ascii_case(w.trim()));
                                ui.label(if used { RichText::new(w).color(t.muted).strikethrough() } else { RichText::new(w).strong() });
                            }
                        });
                    },
                );
                ui.add_space(4.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut g = 0;
                for part in worksheet::cloze_parts(text) {
                    match part {
                        ClozePart::Text(s) => {
                            for (k, word) in s.split(' ').enumerate() {
                                if k > 0 {
                                    ui.add_space(0.0);
                                }
                                if !word.is_empty() {
                                    ui.label(word);
                                }
                            }
                        }
                        ClozePart::Gap(acc) => {
                            if let Some(slot) = gaps.get_mut(g) {
                                let w = (acc.first().map(|a| a.chars().count()).unwrap_or(6) as f32 * 10.0).clamp(90.0, 220.0);
                                let mut edit = egui::TextEdit::singleline(slot).hint_text(format!("Lücke {}", g + 1)).desired_width(w);
                                if let Some(c) = part_color(t, res, g) {
                                    edit = edit.text_color(c);
                                }
                                changed |= ui.add(edit).changed();
                            }
                            g += 1;
                        }
                    }
                }
            });
        }
        (TaskBody::Matching { pairs }, Ans::Matches(m)) => {
            let order = worksheet::display_order(pairs.len(), &task.id);
            egui::Grid::new(("match", &task.id)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for (i, p) in pairs.iter().enumerate() {
                    let mut label = RichText::new(&p.left);
                    if let Some(c) = part_color(t, res, i) {
                        label = label.color(c);
                    }
                    ui.label(label);
                    let cur = m.get(i).copied().flatten();
                    let shown = cur.and_then(|j| pairs.get(j)).map(|x| x.right.clone()).unwrap_or_else(|| "– wählen –".into());
                    egui::ComboBox::from_id_salt(("pick", &task.id, i)).selected_text(shown).width(260.0).show_ui(ui, |ui| {
                        for j in &order {
                            if let Some(pj) = pairs.get(*j)
                                && ui.selectable_label(cur == Some(*j), &pj.right).clicked()
                                && let Some(slot) = m.get_mut(i)
                            {
                                *slot = Some(*j);
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        }
        (TaskBody::Ordering { items }, Ans::Order(order)) => {
            let mut swap = None;
            let n = order.len();
            for (pos, idx) in order.iter().enumerate() {
                ui.horizontal(|ui| {
                    let mut label = RichText::new(format!("{}. {}", pos + 1, items.get(*idx).map(String::as_str).unwrap_or("")));
                    if let Some(c) = part_color(t, res, pos) {
                        label = label.color(c);
                    }
                    if ui.add_enabled(pos > 0, egui::Button::new("⏶").frame(false)).on_hover_text("Nach oben").clicked() {
                        swap = Some((pos, pos - 1));
                    }
                    if ui.add_enabled(pos + 1 < n, egui::Button::new("⏷").frame(false)).on_hover_text("Nach unten").clicked() {
                        swap = Some((pos, pos + 1));
                    }
                    ui.label(label);
                });
            }
            if let Some((a, b)) = swap
                && a < order.len()
                && b < order.len()
            {
                order.swap(a, b);
                changed = true;
            }
        }
        (TaskBody::Categorize { categories }, Ans::Assign(assign)) => {
            let items = worksheet::category_items(categories);
            egui::Grid::new(("groups", &task.id)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for i in worksheet::display_order(items.len(), &task.id) {
                    let Some((_, word)) = items.get(i) else { continue };
                    let mut label = RichText::new(*word);
                    if let Some(c) = part_color(t, res, i) {
                        label = label.color(c);
                    }
                    ui.label(label);
                    let cur = assign.get(i).copied().flatten();
                    let shown = cur.and_then(|c| categories.get(c)).map(|c| c.name.clone()).unwrap_or_else(|| "– Gruppe wählen –".into());
                    egui::ComboBox::from_id_salt(("group-pick", &task.id, i)).selected_text(shown).width(220.0).show_ui(ui, |ui| {
                        for (c, cat) in categories.iter().enumerate() {
                            if ui.selectable_label(cur == Some(c), &cat.name).clicked()
                                && let Some(slot) = assign.get_mut(i)
                            {
                                *slot = Some(c);
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            if checked && res.and_then(|r| r.correct) == Some(false) {
                let solution: Vec<String> = categories
                    .iter()
                    .map(|c| format!("{}: {}", c.name, c.items.iter().filter(|x| !x.trim().is_empty()).cloned().collect::<Vec<_>>().join(", ")))
                    .collect();
                ui.label(small_muted(t, format!("Richtig wäre: {}", solution.join(" · "))));
            }
        }
        (TaskBody::ShortAnswer { accepted, .. }, Ans::Text(s)) => {
            changed |= ui.add(egui::TextEdit::singleline(s).hint_text("Deine Antwort").desired_width(320.0)).changed();
            if checked && res.and_then(|r| r.correct) == Some(false) {
                ui.label(small_muted(t, format!("Richtig wäre: {}", accepted.join(" / "))));
            }
        }
        (TaskBody::Numeric { unit, .. }, Ans::Text(s)) => {
            ui.horizontal(|ui| {
                changed |= ui.add(egui::TextEdit::singleline(s).hint_text("Zahl").desired_width(140.0)).changed();
                if !unit.is_empty() {
                    ui.label(unit);
                }
            });
        }
        (TaskBody::FreeText { lines, .. }, Ans::Text(s)) => {
            changed |= ui
                .add(egui::TextEdit::multiline(s).hint_text("Deine Antwort").desired_rows((*lines as usize).clamp(2, 10)).desired_width(ui.available_width()))
                .changed();
        }
        _ => {}
    }
    changed
}
