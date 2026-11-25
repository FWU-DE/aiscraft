//! Sources and citations (#39): under an answer the passages that back it, numbered; a click
//! shows the passage where it stands in the attached document, or the quote and the page.

use aiscraft_core::{Citation, Message};
use egui::{RichText, ScrollArea};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

/// How the row names a citation: "[1] igel.txt", "[2] plan.pdf, S. 3".
pub(crate) fn label(i: usize, c: &Citation) -> String {
    let title: String = if c.title.trim().is_empty() { c.url.clone().unwrap_or_else(|| "Dokument".into()) } else { c.title.clone() };
    let title: String = title.chars().take(50).collect();
    let place = c.place();
    if place.is_empty() { format!("[{}] {title}", i + 1) } else { format!("[{}] {title}, {place}", i + 1) }
}

/// Text cut to `n` characters, with an ellipsis.
fn cut(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n).collect::<String>()) }
}

impl AisCraftApp {
    /// The passages under an answer: „Belege:“ and one link per citation.
    pub(crate) fn citations_row(&mut self, ui: &mut egui::Ui, m: &Message) {
        if m.citations.is_empty() {
            return;
        }
        let t = self.tokens.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(small_muted(&t, "Belege:"));
            for (i, c) in m.citations.iter().enumerate().take(50) {
                let mut hover = String::new();
                if !c.claim.trim().is_empty() {
                    hover.push_str(&format!("Aussage: {}\n", cut(&c.claim, 200)));
                }
                hover.push_str(&format!("„{}“", cut(&c.cited_text, 300)));
                if ui.link(RichText::new(label(i, c)).small().color(t.accent)).on_hover_text(hover).clicked() {
                    self.run("message.citation", json!({"message_id": m.id, "index": i}));
                }
            }
        });
    }

    /// The opened citation: the passage marked in the text around it.
    pub(crate) fn citation_ui(&mut self, ctx: &egui::Context) {
        let Some(view) = self.state.citation.clone() else { return };
        let t = self.tokens.clone();
        let c = &view.citation;
        let mut open = true;
        let screen = ctx.content_rect();
        egui::Window::new(RichText::new("Beleg").strong())
            .id(egui::Id::unique("citation"))
            .open(&mut open)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .default_width(620.0_f32.min(screen.width() - 40.0))
            .max_height((screen.height() * 0.8).max(240.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(label(view.index, c)).strong());
                if !c.claim.trim().is_empty() {
                    ui.add(egui::Label::new(small_muted(&t, format!("Aussage der Antwort: {}", cut(&c.claim, 400)))).wrap());
                }
                ui.add_space(6.0);
                ScrollArea::vertical().id_salt(("citation-text", &view.message_id, view.index)).max_height((screen.height() * 0.55).max(160.0)).show(
                    ui,
                    |ui| {
                        let mut job = egui::text::LayoutJob::default();
                        let body = egui::TextStyle::Body.resolve(ui.style());
                        let plain = egui::TextFormat { font_id: body.clone(), color: t.text2, ..Default::default() };
                        let marked = egui::TextFormat { font_id: body, color: t.text, background: t.accent.gamma_multiply(0.25), ..Default::default() };
                        job.append(&view.before, 0.0, plain.clone());
                        job.append(&view.passage, 0.0, marked);
                        job.append(&view.after, 0.0, plain);
                        job.wrap.max_width = ui.available_width();
                        ui.label(job);
                    },
                );
                ui.add_space(6.0);
                if let Some(url) = &c.url {
                    ui.hyperlink_to(format!("Seite öffnen: {}", cut(url, 80)), url);
                } else if !view.found {
                    let note = if c.pages.is_some() {
                        format!("Die Stelle steht im Original auf {}.", c.place())
                    } else {
                        "Die Stelle steht nicht mehr im angehängten Text; oben das Zitat.".to_string()
                    };
                    ui.label(small_muted(&t, note));
                }
            });
        if !open {
            self.run("message.citation", json!({"close": true}));
        }
    }
}
