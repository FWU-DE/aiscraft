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

/// The address of an inline marker: `aiscraft-beleg:<index>`. A click never leaves the app.
pub(crate) const MARKER_SCHEME: &str = "aiscraft-beleg:";

/// Most markers placed in one answer.
const MAX_MARKERS: usize = 50;

/// Whether byte `pos` of `text` lies inside a fenced code block or a `$$` formula: a marker there
/// would show as source or break the formula.
fn in_code_or_math(text: &str, pos: usize) -> bool {
    let (mut fence, mut math) = (false, false);
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if start + line.len() > pos {
            // The line holding `pos`: `$$` before it on this line count too.
            let head = text.get(start..pos).unwrap_or("");
            return fence || (math ^ (head.matches("$$").count() % 2 == 1));
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        } else if !fence && line.matches("$$").count() % 2 == 1 {
            math = !math;
        }
        start += line.len();
    }
    fence || math
}

/// The answer with `[n]` after each cited statement (#39): Markdown links to
/// `aiscraft-beleg:<index>`, clickable like the entries under the answer. A statement is found
/// by its text, in order; one that is not found (changed, cut) only stands in the list. Code and
/// formulas get no markers.
pub(crate) fn with_markers<'a>(content: &'a str, citations: &[Citation]) -> std::borrow::Cow<'a, str> {
    let mut inserts: Vec<(usize, String)> = Vec::new();
    let mut cursor = 0;
    let mut last: Option<(&str, usize)> = None;
    for (i, c) in citations.iter().enumerate().take(MAX_MARKERS) {
        let claim = c.claim.trim();
        if claim.chars().count() < 2 {
            continue;
        }
        let fits = |p: &usize| content.is_char_boundary(*p) && !in_code_or_math(content, *p);
        let after =
            |from: usize| content.get(from..).and_then(|rest| rest.match_indices(claim).map(|(k, _)| from + k + claim.len()).take(20).find(|p| fits(p)));
        let pos = match last {
            Some((prev, at)) if prev == claim => Some(at),
            _ => after(cursor).or_else(|| after(0)),
        };
        let Some(pos) = pos else { continue };
        cursor = cursor.max(pos);
        last = Some((claim, pos));
        // A backslash right before the marker would escape its bracket.
        let gap = if content.get(..pos).is_some_and(|h| h.ends_with('\\')) { " " } else { "" };
        inserts.push((pos, format!("{gap}[\\[{}\\]]({MARKER_SCHEME}{i})", i + 1)));
    }
    if inserts.is_empty() {
        return std::borrow::Cow::Borrowed(content);
    }
    inserts.sort_by_key(|(p, _)| *p);
    let mut out = String::with_capacity(content.len() + inserts.len() * 32);
    let mut from = 0;
    for (p, m) in inserts {
        out.push_str(content.get(from..p).unwrap_or(""));
        out.push_str(&m);
        from = p;
    }
    out.push_str(content.get(from..).unwrap_or(""));
    std::borrow::Cow::Owned(out)
}

/// A marker clicked this frame: takes the `aiscraft-beleg:` addresses out of egui's output (so
/// no browser opens) and gives the first index.
pub(crate) fn take_marker_click(ctx: &egui::Context) -> Option<usize> {
    ctx.output_mut(|o| {
        let mut hit = None;
        o.commands.retain(|cmd| match cmd {
            egui::OutputCommand::OpenUrl(u) if u.url.starts_with(MARKER_SCHEME) => {
                if hit.is_none() {
                    hit = u.url.get(MARKER_SCHEME.len()..).and_then(|n| n.parse::<usize>().ok());
                }
                false
            }
            _ => true,
        });
        hit
    })
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
        let mut open_file = false;
        let screen = ctx.content_rect();
        let shown = egui::Window::new(RichText::new("Beleg").strong())
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
                // The original file: a PDF opens in the browser's PDF viewer at the page.
                if let Some(path) = &view.path {
                    let pdf = path.to_lowercase().ends_with(".pdf");
                    let label = match c.pages {
                        Some((page, _)) if pdf => format!("PDF auf Seite {page} öffnen"),
                        _ => "Datei öffnen".to_string(),
                    };
                    if ui.button(label).on_hover_text(path.as_str()).clicked() {
                        open_file = true;
                    }
                }
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
        // A click beside the passage closes it, as beside a dialog.
        if shown.is_some_and(|r| r.response.clicked_elsewhere()) {
            open = false;
        }
        if open_file {
            self.run("message.citation_file", json!({}));
        }
        if !open {
            self.run("message.citation", json!({"close": true}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cite(claim: &str) -> Citation {
        Citation { claim: claim.into(), cited_text: "x".into(), ..Default::default() }
    }

    #[test]
    fn markers_follow_the_cited_statements() {
        let text = "Laut Datei: Igel schlafen im Winter. Sie fressen **Insekten**.\n\nMehr weiß ich nicht.";
        let out = with_markers(text, &[cite("Igel schlafen im Winter."), cite("Igel schlafen im Winter."), cite("Sie fressen **Insekten**.")]);
        assert_eq!(
            out,
            "Laut Datei: Igel schlafen im Winter.[\\[1\\]](aiscraft-beleg:0)[\\[2\\]](aiscraft-beleg:1) Sie fressen **Insekten**.[\\[3\\]](aiscraft-beleg:2)\n\nMehr weiß ich nicht."
        );
        // Statements not in the text, empty ones and web pages without a statement: no marker.
        assert!(matches!(with_markers(text, &[cite("Kängurus hüpfen."), cite(" "), cite("")]), std::borrow::Cow::Borrowed(_)));
        // The same sentence twice: the markers go to the places in order.
        let twice = with_markers("A ist wahr. B. A ist wahr.", &[cite("A ist wahr."), cite("B."), cite("A ist wahr.")]);
        assert_eq!(twice, "A ist wahr.[\\[1\\]](aiscraft-beleg:0) B.[\\[2\\]](aiscraft-beleg:1) A ist wahr.[\\[3\\]](aiscraft-beleg:2)");
    }

    #[test]
    fn no_markers_in_code_or_formulas() {
        let text = "```\nIgel schlafen\n```\n\n$$\nx = Igel\n$$\n\nIgel schlafen.";
        assert_eq!(with_markers(text, &[cite("Igel schlafen")]).matches("aiscraft-beleg").count(), 1);
        assert!(with_markers(text, &[cite("Igel schlafen")]).ends_with("Igel schlafen[\\[1\\]](aiscraft-beleg:0)."));
        assert_eq!(with_markers("```\nnur Code\n```", &[cite("nur Code")]), "```\nnur Code\n```");
        assert_eq!(with_markers("$$x = Igel$$", &[cite("x = Igel")]), "$$x = Igel$$");
        // A backslash before the marker would escape its bracket.
        assert_eq!(with_markers("Pfad C:\\", &[cite("C:\\")]), "Pfad C:\\ [\\[1\\]](aiscraft-beleg:0)");
        // Markdown with markers stays Markdown: the marker is a link.
        let md = with_markers("- Punkt eins\n- Punkt zwei", &[cite("Punkt eins")]);
        let links = pulldown_cmark::Parser::new(&md).filter(|e| matches!(e, pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { .. }))).count();
        assert_eq!(links, 1, "{md}");
    }
}
