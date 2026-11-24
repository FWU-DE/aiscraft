//! Learning apps in the artifact panel (source with highlighting, "Öffnen", "Speichern …"),
//! the cards for ```` ```html ```` and ```` ```aiscraft-worksheet ```` blocks in answers, and
//! the "Erstellen" menu in the composer (prompt helpers that work with AIS.chat too).
//!
//! No webview here: a learning app opens in the separate `aiscraft-preview` process or in the
//! system browser, always with the engine's Content-Security-Policy.

use std::time::{Duration, Instant};

use aiscraft_core::ArtifactKind;
use aiscraft_core::worksheet;
use egui::text::LayoutJob;
use egui::{Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, TextFormat, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::theme::Tokens;
use crate::widgets::{accent_button, small_muted};

impl AisCraftApp {
    /// "Mit KI überarbeiten" for apps and worksheets: a new version through `artifact.revise`.
    pub(crate) fn revise_bar(&mut self, ui: &mut Ui, hint: &str) {
        let t = self.tokens.clone();
        let Some(v) = self.artifact.as_mut() else { return };
        let mut go = None;
        // Under the content, styled like the chat composer: a tonal pill (surface container)
        // with a frameless field and the main action spelled out ("Überarbeiten", not "Los").
        egui::Panel::bottom("artifact-revise-bar")
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().inner_margin(Margin { left: 14, right: 14, top: 12, bottom: 14 }))
            .show(ui, |ui| {
                let top = ui.max_rect().top() - 12.0;
                ui.painter().line_segment(
                    [egui::pos2(ui.max_rect().left() - 14.0, top), egui::pos2(ui.max_rect().right() + 14.0, top)],
                    egui::Stroke::new(1.0, t.border),
                );
                let stroke = if t.glass.is_some() { egui::Stroke::new(1.0, t.card_stroke) } else { egui::Stroke::new(1.5, t.accent) };
                Frame::new()
                    .fill(t.surface)
                    .stroke(stroke)
                    .corner_radius(egui::CornerRadius::same(22))
                    .shadow(if t.glass.is_some() { t.card_shadow } else { egui::epaint::Shadow::NONE })
                    .inner_margin(Margin { left: 14, right: 6, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(crate::icons::Icon::Sparkles.text(18.0).color(t.accent));
                            let r = ui.add_enabled(
                                !v.revising,
                                egui::TextEdit::singleline(&mut v.revise)
                                    .hint_text(RichText::new(hint).color(t.muted))
                                    .frame(Frame::NONE)
                                    .font(egui::FontId::proportional(14.0))
                                    .margin(Margin::symmetric(0, 6))
                                    .desired_width(ui.available_width() - 130.0),
                            );
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if v.revising {
                                    ui.add(egui::Spinner::new().color(t.accent));
                                } else if (crate::kit::pill(ui, &t, crate::kit::Pill::Primary, None, "Überarbeiten", 13.5, None).clicked() || enter)
                                    && !v.revise.trim().is_empty()
                                {
                                    go = Some(v.revise.trim().to_string());
                                }
                            });
                        });
                    });
            });
        if v.revising && v.revise_done.load(std::sync::atomic::Ordering::Relaxed) {
            v.revising = false;
        }
        if v.revising {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        if let Some(instruction) = go {
            self.save_if_dirty();
            if let Some(v) = self.artifact.as_mut() {
                v.revising = true;
                let id = v.id.clone();
                let engine = self.engine.clone();
                let done = v.revise_done.clone();
                done.store(false, std::sync::atomic::Ordering::Relaxed);
                std::thread::spawn(move || {
                    if let Err(e) = engine.call("artifact.revise", json!({"id": id, "instruction": instruction})) {
                        engine.ctx().notice(aiscraft_engine::NoticeLevel::Error, e.message);
                    }
                    done.store(true, std::sync::atomic::Ordering::Relaxed);
                });
                v.revise.clear();
            }
        }
    }

    /// Save a text artifact a moment after the last edit.
    pub(crate) fn autosave(&mut self, ui: &mut Ui) {
        match self.artifact.as_ref().and_then(|v| v.dirty_since) {
            Some(d) if d.elapsed() >= crate::artifact_panel::SAVE_AFTER => self.save_artifact_now(),
            Some(_) => ui.ctx().request_repaint_after(crate::artifact_panel::SAVE_AFTER),
            None => {}
        }
    }

    pub(crate) fn app_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(v) = self.artifact.as_ref() else { return };
        let id = v.id.clone();
        let kb = v.doc.len().div_ceil(1024);
        let title = self.state.artifacts.iter().find(|a| a.id == id).map(|a| a.title.clone()).unwrap_or_default();
        Frame::new().inner_margin(Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                if accent_button(ui, &t, "▶ Öffnen")
                    .on_hover_text(
                        "In einem eigenen Vorschaufenster öffnen (oder im Browser). Die App kann dabei nichts aus dem Internet laden und nichts senden.",
                    )
                    .clicked()
                {
                    self.save_if_dirty();
                    self.run("learning_app.open", json!({"id": id}));
                }
                if ui.button("Speichern …").on_hover_text("Als HTML-Datei speichern, z. B. zum Weitergeben an Schülerinnen und Schüler").clicked()
                    && let Some(path) = rfd::FileDialog::new().set_file_name(aiscraft_engine::office::file_name(&title, "html")).save_file()
                {
                    self.save_if_dirty();
                    self.run("learning_app.save", json!({"id": id, "path": path.display().to_string()}));
                }
                ui.label(small_muted(&t, format!("{kb} KB · ohne Netzwerkzugriff")));
            });
        });
        self.revise_bar(ui, "Mit KI ändern, z. B. „Mehr Aufgaben und ein Punktestand“");
        let Some(v) = self.artifact.as_mut() else { return };
        let tokens = t.clone();
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let mut job = highlight_html(buf.as_str(), &tokens);
            job.wrap.max_width = wrap;
            ui.fonts_mut(|f| f.layout_job(job))
        };
        ScrollArea::vertical().id_salt(("app", id.clone())).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().fill(t.code_bg).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut v.doc)
                        .code_editor()
                        .hint_text("<!doctype html> …")
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .frame(Frame::NONE)
                        .layouter(&mut layouter),
                );
                if r.changed() {
                    v.dirty_since = Some(Instant::now());
                }
            });
        });
        self.autosave(ui);
    }

    /// The card under an answer for a learning app or worksheet block.
    pub(crate) fn learn_card(&mut self, ui: &mut Ui, kind: ArtifactKind, src: &str) {
        let t = self.tokens.clone();
        let open_same =
            self.state.open_artifact.as_ref().and_then(|id| self.state.artifacts.iter().find(|a| &a.id == id)).filter(|a| a.kind == kind).map(|a| a.id.clone());
        let generating = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        let frame = Frame::new()
            .fill(t.surface)
            .stroke(Stroke::new(1.5, t.accent))
            .corner_radius(CornerRadius::same(16))
            .shadow(t.card_shadow)
            .inner_margin(Margin { left: 10, right: 14, top: 10, bottom: 10 });
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (headline, detail, problems) = match kind {
                ArtifactKind::App => {
                    let title = aiscraft_engine::learn::html_title(src).unwrap_or_else(|| "Lern-App".into());
                    (title, format!("Lern-App · HTML, {} KB", src.len().div_ceil(1024)), Vec::new())
                }
                ArtifactKind::H5p => match aiscraft_core::h5p::parse_lenient(src) {
                    Ok((h, problems)) => (h.title.clone(), format!("H5P · {} Aufgaben", h.tasks.len()), problems),
                    Err(e) => ("H5P".to_string(), "lässt sich nicht lesen".to_string(), vec![e]),
                },
                _ => match worksheet::parse_lenient(src) {
                    Ok((ws, problems)) => (
                        ws.title.clone(),
                        format!("Arbeitsblatt · {} Aufgaben, {} Punkte", ws.tasks.len(), ws.tasks.iter().map(|x| x.points).sum::<f64>()),
                        problems,
                    ),
                    Err(e) => ("Arbeitsblatt".to_string(), "lässt sich nicht lesen".to_string(), vec![e]),
                },
            };
            ui.horizontal(|ui| {
                let (tile, _) = ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                crate::kit::accent_fill(ui.painter(), tile, CornerRadius::same(12), &t);
                kind_lucide(kind).paint(ui.painter(), tile.center(), 20.0, t.on_accent);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(crate::theme::bold(headline).size(15.0).color(t.text));
                    ui.label(RichText::new(detail).size(13.0).color(t.muted));
                });
            });
            for p in problems.iter().take(3) {
                ui.label(RichText::new(p).small().color(t.warning));
            }
            if generating {
                return;
            }
            ui.horizontal_wrapped(|ui| {
                let (open_cmd, open_label, apply_label) = match kind {
                    ArtifactKind::App => ("learning_app.create", "Als Lern-App öffnen", "In die Lern-App übernehmen"),
                    ArtifactKind::H5p => ("h5p.create", "Als H5P öffnen", "In den H5P-Inhalt übernehmen"),
                    _ => ("worksheet.create", "Als Arbeitsblatt öffnen", "Ins Arbeitsblatt übernehmen"),
                };
                if let Some(id) = &open_same
                    && kind == ArtifactKind::H5p
                    && accent_button(ui, &t, apply_label).on_hover_text("Ersetzt die Aufgaben; die vorige Fassung bleibt unter Versionen").clicked()
                {
                    self.run("h5p.create", json!({"id": id, "content": src}));
                } else if let Some(id) = &open_same
                    && kind != ArtifactKind::H5p
                    && accent_button(ui, &t, apply_label).on_hover_text("Ersetzt den Inhalt; die vorige Fassung bleibt unter Versionen").clicked()
                {
                    let text = match kind {
                        ArtifactKind::App => Some(src.trim().to_string()),
                        _ => worksheet::parse_lenient(src).ok().map(|(ws, _)| worksheet::to_json(&ws)),
                    };
                    if let Some(text) = text {
                        self.run("artifact.save", json!({"id": id, "text": text, "version": "Aus einer Antwort übernommen"}));
                    }
                }
                let params = match kind {
                    ArtifactKind::App => json!({"html": src}),
                    _ => json!({"content": src, "repair": true}),
                };
                let clicked = if open_same.is_some() { ui.button(open_label).clicked() } else { accent_button(ui, &t, open_label).clicked() };
                if clicked {
                    self.run(open_cmd, params);
                }
            });
        });
        ui.add_space(6.0);
    }

    /// "Erstellen" in the composer: the composer text becomes the topic of a learning app or
    /// worksheet request.
    /// "Erstellen" as the sparkle icon (narrow composer).
    pub(crate) fn create_menu_icon(&mut self, ui: &mut Ui) {
        self.create_as_icon = true;
        self.create_menu(ui);
        self.create_as_icon = false;
    }

    pub(crate) fn create_menu(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let body = |app: &mut Self, ui: &mut Ui| {
            ui.set_min_width(300.0);
            // The topic field is the composer itself, so text typed before opening the menu is there.
            ui.label(small_muted(&t, "Thema"));
            ui.add(egui::TextEdit::singleline(&mut app.composer).hint_text("z. B. Brüche vergleichen, Klasse 6").desired_width(f32::INFINITY));
            let topic = app.composer.trim().to_string();
            let usable = topic.chars().filter(|ch| ch.is_alphanumeric()).count() >= 2;
            ui.add_space(4.0);
            for (label, cmd, tip) in [
                ("🎮 Lern-App", "learning_app.request", "Eine interaktive Lern-App (eine HTML-Datei) zum Thema im Eingabefeld"),
                ("📝 Arbeitsblatt", "worksheet.request", "Ein interaktives Arbeitsblatt zum Thema im Eingabefeld, zum Lösen am Rechner und zum Drucken"),
                ("📦 H5P-Quiz", "h5p.request", "Ein H5P-Quiz zum Thema im Eingabefeld, als .h5p-Datei für Moodle, Mebis, ByCS und andere Lernplattformen"),
            ] {
                if ui.add_enabled(usable, egui::Button::new(label)).on_hover_text(tip).clicked() {
                    app.composer.clear();
                    app.run(cmd, json!({"topic": topic}));
                    ui.close();
                }
            }
        };
        let tip = "Lern-App, Arbeitsblatt oder H5P-Quiz erstellen lassen";
        let r = if self.create_as_icon {
            crate::kit::icon_button(ui, &t, crate::icons::Icon::Sparkles, "Erstellen: Lern-App, Arbeitsblatt oder H5P-Quiz", t.text2, 19.0, 34.0)
        } else {
            crate::kit::pill(ui, &t, crate::kit::Pill::Chip, None, "Erstellen", 13.5, Some(crate::icons::Icon::ChevronDown)).on_hover_text(tip)
        };
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| body(self, ui));
    }
}

// ---- HTML highlighting -----------------------------------------------------------------------

/// A small HTML highlighter: tags, attributes, strings and comments. Script and style bodies
/// stay plain. Never fails, whatever the input.
pub fn highlight_html(src: &str, t: &Tokens) -> LayoutJob {
    let font = egui::FontId::monospace(12.5);
    let plain = TextFormat { font_id: font.clone(), color: t.text, ..Default::default() };
    let fmt = |c: Color32| TextFormat { font_id: font.clone(), color: c, ..Default::default() };
    let tag = fmt(t.accent);
    let attr = fmt(if t.dark { Color32::from_rgb(0xd8, 0xa6, 0x57) } else { Color32::from_rgb(0x9a, 0x5b, 0x00) });
    let string = fmt(if t.dark { Color32::from_rgb(0x8f, 0xc9, 0x7a) } else { Color32::from_rgb(0x2f, 0x7d, 0x32) });
    let comment = fmt(t.muted);
    let mut job = LayoutJob::default();
    // Very long files: highlight the first part only (typing stays fast).
    let limit = src.len().min(300_000);
    let (head, tail) = match src.get(..limit) {
        Some(h) => (h, src.get(limit..).unwrap_or("")),
        None => ("", src),
    };
    let mut rest = head;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->").map(|i| i + 4 + 3).unwrap_or(rest.len());
            let (a, b) = rest.split_at(end.min(rest.len()));
            job.append(a, 0.0, comment.clone());
            rest = b;
        } else if rest.starts_with('<') {
            // A tag: name in accent, attributes, quoted values.
            let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
            let (tag_src, b) = rest.split_at(end);
            let mut chars = tag_src.char_indices().peekable();
            let mut seg_start = 0;
            let mut state = 0; // 0 name, 1 attrs, 2 in string
            let mut quote = '"';
            let flush = |job: &mut LayoutJob, from: usize, to: usize, f: &TextFormat| {
                if let Some(s) = tag_src.get(from..to)
                    && !s.is_empty()
                {
                    job.append(s, 0.0, f.clone());
                }
            };
            while let Some((i, c)) = chars.next() {
                match state {
                    0 if c.is_whitespace() => {
                        flush(&mut job, seg_start, i, &tag);
                        seg_start = i;
                        state = 1;
                    }
                    1 if c == '"' || c == '\'' => {
                        flush(&mut job, seg_start, i, &attr);
                        seg_start = i;
                        quote = c;
                        state = 2;
                    }
                    2 if c == quote => {
                        flush(&mut job, seg_start, i + 1, &string);
                        seg_start = i + 1;
                        state = 1;
                    }
                    _ => {}
                }
                if chars.peek().is_none() {
                    let f = match state {
                        0 => &tag,
                        1 => &attr,
                        _ => &string,
                    };
                    flush(&mut job, seg_start, tag_src.len(), f);
                }
            }
            rest = b;
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let (a, b) = rest.split_at(end);
            job.append(a, 0.0, plain.clone());
            rest = b;
        }
    }
    if !tail.is_empty() {
        job.append(tail, 0.0, plain);
    }
    job
}

/// The label of a learning artifact for lists: (icon, kind label).
/// The line icon of an artifact kind (Modern, Glas).
pub fn kind_lucide(kind: ArtifactKind) -> crate::icons::Icon {
    use crate::icons::Icon;
    match kind {
        ArtifactKind::Board => Icon::Note,
        ArtifactKind::App => Icon::App,
        ArtifactKind::Worksheet => Icon::Table,
        ArtifactKind::H5p => Icon::Puzzle,
        ArtifactKind::Document => Icon::File,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn highlighting_keeps_every_character() {
        let t = crate::theme::Tokens::modern_dark();
        for src in ["", "<", "<!--", "<a href=\"x\">ä</a>", "<p class='x' data-y=\"<\">text</p><!-- c --> rest", "\"'<<>>"] {
            let job = highlight_html(src, &t);
            assert_eq!(job.text, src, "{src}");
        }
    }
}
