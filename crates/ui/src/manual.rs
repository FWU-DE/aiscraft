//! The handbook inside the app (F1): one Markdown file, split into chapters at `## `.

use egui::{RichText, ScrollArea};

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

/// The handbook text (German).
pub const TEXT: &str = include_str!("../manual/de.md");

/// Chapters: (title, body). The part before the first `## ` is the introduction.
pub fn chapters() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut title = "Einführung".to_string();
    let mut body = String::new();
    for line in TEXT.lines() {
        if let Some(t) = line.strip_prefix("## ") {
            if !body.trim().is_empty() {
                out.push((title.clone(), body.trim().to_string()));
            }
            title = t.trim().to_string();
            body.clear();
        } else if !line.starts_with("# ") {
            body.push_str(line);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() {
        out.push((title, body.trim().to_string()));
    }
    out
}

/// Chapters whose title or text contain every word of the query.
pub fn search(query: &str) -> Vec<usize> {
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    chapters()
        .iter()
        .enumerate()
        .filter(|(_, (t, b))| {
            let hay = format!("{t} {b}").to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The chapter for a settings tab or topic, by title.
pub fn chapter_index(title: &str) -> usize {
    chapters().iter().position(|(t, _)| t == title).unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct ManualView {
    pub chapter: usize,
    pub query: String,
}

impl AisCraftApp {
    pub(crate) fn manual_ui(&mut self, ctx: &egui::Context) {
        let Some(mut view) = self.manual.take() else { return };
        let t = self.tokens.clone();
        let chapters = chapters();
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Window::new(RichText::new("Handbuch").strong())
            .id(egui::Id::unique("manual"))
            .open(&mut open)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .default_size([860.0_f32.min(screen.width() - 40.0), (screen.height() * 0.8).max(320.0)])
            .min_size([420.0, 300.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(210.0);
                        ui.add(egui::TextEdit::singleline(&mut view.query).hint_text("Im Handbuch suchen").desired_width(200.0));
                        ui.add_space(6.0);
                        let hits = if view.query.trim().is_empty() { (0..chapters.len()).collect() } else { search(&view.query) };
                        if hits.is_empty() {
                            ui.label(small_muted(&t, "Nichts gefunden."));
                        }
                        ScrollArea::vertical().id_salt("manual-toc").show(ui, |ui| {
                            for i in hits {
                                if let Some((title, _)) = chapters.get(i)
                                    && ui.selectable_label(view.chapter == i, title).clicked()
                                {
                                    view.chapter = i;
                                }
                            }
                        });
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        ScrollArea::vertical().id_salt(("manual-body", view.chapter)).auto_shrink([false, false]).show(ui, |ui| {
                            if let Some((title, body)) = chapters.get(view.chapter) {
                                ui.heading(title);
                                ui.add_space(4.0);
                                egui_commonmark::CommonMarkViewer::new().show(ui, &mut self.md, body);
                            }
                        });
                    });
                });
            });
        if open {
            self.manual = Some(view);
        }
    }

    /// Open the handbook at a chapter.
    pub(crate) fn open_manual(&mut self, chapter: &str) {
        self.manual = Some(ManualView { chapter: chapter_index(chapter), query: String::new() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_and_search() {
        let c = chapters();
        assert!(c.len() >= 12, "{}", c.len());
        assert_eq!(c[0].0, "Einführung");
        assert!(c.iter().any(|(t, _)| t == "Tastenkürzel"));
        assert!(!search("tresor platzhalter").is_empty());
        assert!(search("gibtesnichtinhandbuch").is_empty());
        assert_eq!(chapters()[chapter_index("Tresor")].0, "Tresor");
        assert_eq!(chapter_index("nicht da"), 0);
    }

    /// Every command a person can reach is described: the handbook mentions the main features.
    #[test]
    fn handbook_covers_the_features() {
        let all = TEXT.to_lowercase();
        for word in [
            "quellen",
            "konnektor",
            "zeitplän",
            "aufgaben",
            "tresor",
            "gedächtnis",
            "whiteboard",
            "dokument",
            "diktieren",
            "vorlesen",
            "kontingent",
            "projekt",
            "im browser anmelden",
            "claude (abo)",
            "dialogpartner",
            "lernszenarien",
        ] {
            assert!(all.contains(word), "handbook lacks {word}");
        }
    }
}
