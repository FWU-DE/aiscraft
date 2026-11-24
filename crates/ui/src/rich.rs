//! Markdown in Modern and Glas. egui_commonmark draws headings and table heads in the regular
//! cut ("strong" is only a colour in egui) and lists with plain numbers; the designs want Barlow
//! Bold titles, tables with a coloured bold head, numbered steps in accent circles, quotes as a
//! recessed block with an accent bar, and code on a dark block with a head. So the top-level
//! blocks are split with pulldown-cmark: those kinds are drawn here, everything else (and any
//! block with links, images or HTML inside) still goes to egui_commonmark.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontFamily, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::theme::{self, Tokens};

#[derive(Debug, PartialEq)]
enum Block<'a> {
    Heading(HeadingLevel, String),
    /// Head cells, then body rows; each cell is its Markdown source.
    Table(Vec<&'a str>, Vec<Vec<&'a str>>),
    /// Start number (None: bullets) and each item's Markdown source.
    List(Option<u64>, Vec<String>),
    Quote(String),
    Code(String, String),
    Markdown(&'a str),
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// The top-level blocks of `text`; runs of other blocks stay one Markdown piece.
fn blocks<'a>(text: &'a str) -> Vec<Block<'a>> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    // The block being read: its kind, start, and what was collected.
    let mut start = 0usize;
    let mut kind: Option<Tag> = None;
    let mut plain = String::new();
    let mut simple = true;
    let mut rows: Vec<Vec<&'a str>> = Vec::new();
    let mut head: Vec<&'a str> = Vec::new();
    let mut in_head = false;
    let mut items: Vec<String> = Vec::new();
    let mut item_depth = 0usize;
    let mut other: Option<Range<usize>> = None;
    let flush_other = |other: &mut Option<Range<usize>>, out: &mut Vec<Block<'a>>| {
        if let Some(r) = other.take()
            && let Some(s) = text.get(r)
            && !s.trim().is_empty()
        {
            out.push(Block::Markdown(s));
        }
    };
    for (ev, range) in Parser::new_ext(text, options()).into_offset_iter() {
        match &ev {
            Event::Start(tag) => {
                if depth == 0 {
                    start = range.start;
                    kind = Some(tag.clone());
                    plain.clear();
                    simple = true;
                    rows.clear();
                    head.clear();
                    items.clear();
                } else if matches!(kind, Some(Tag::List(_))) {
                    match tag {
                        Tag::Item if depth == 1 => {
                            item_depth = depth;
                            let src = text.get(range.clone()).unwrap_or("");
                            items.push(item_text(src));
                        }
                        Tag::Paragraph => {}
                        Tag::Strong | Tag::Emphasis | Tag::Strikethrough => {}
                        _ => simple = false,
                    }
                } else if matches!(kind, Some(Tag::Table(_))) {
                    match tag {
                        Tag::TableHead => in_head = true,
                        Tag::TableRow => rows.push(Vec::new()),
                        Tag::TableCell => {
                            let cell = text.get(range.clone()).unwrap_or("").trim().trim_matches('|').trim();
                            if in_head {
                                head.push(cell);
                            } else if let Some(r) = rows.last_mut() {
                                r.push(cell);
                            }
                        }
                        Tag::Link { .. } | Tag::Image { .. } => simple = false,
                        _ => {}
                    }
                } else if matches!(kind, Some(Tag::BlockQuote(_))) {
                    match tag {
                        Tag::Paragraph | Tag::Strong | Tag::Emphasis | Tag::Strikethrough => {}
                        _ => simple = false,
                    }
                } else if matches!(tag, Tag::Link { .. } | Tag::Image { .. }) {
                    simple = false;
                }
                depth += 1;
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                if matches!(end, TagEnd::TableHead) {
                    in_head = false;
                }
                if matches!(end, TagEnd::Paragraph) && matches!(kind, Some(Tag::BlockQuote(_))) && !plain.is_empty() {
                    plain.push('\n');
                }
                let _ = item_depth;
                if depth == 0 {
                    let block = match kind.take() {
                        Some(Tag::Heading { level, .. }) if simple => Some(Block::Heading(level, plain.trim().to_string())),
                        Some(Tag::Table(_)) if simple && !head.is_empty() => Some(Block::Table(std::mem::take(&mut head), std::mem::take(&mut rows))),
                        Some(Tag::List(n)) if simple && !items.is_empty() && items.len() <= 60 => Some(Block::List(n, std::mem::take(&mut items))),
                        Some(Tag::BlockQuote(_)) if simple => Some(Block::Quote(quote_text(text.get(start..range.end).unwrap_or("")))),
                        Some(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                            Some(Block::Code(lang.split_whitespace().next().unwrap_or("").to_string(), plain.clone()))
                        }
                        Some(Tag::CodeBlock(CodeBlockKind::Indented)) => Some(Block::Code(String::new(), plain.clone())),
                        _ => None,
                    };
                    match block {
                        Some(b) => {
                            flush_other(&mut other, &mut out);
                            out.push(b);
                        }
                        None => {
                            other = Some(other.map_or(start..range.end, |o| o.start..range.end));
                        }
                    }
                }
            }
            Event::Text(s) | Event::Code(s) => plain.push_str(s),
            Event::SoftBreak | Event::HardBreak => plain.push(' '),
            Event::Html(_) | Event::InlineHtml(_) | Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => simple = false,
            Event::Rule | Event::TaskListMarker(_) => {
                if depth == 0 {
                    other = Some(other.map_or(range.clone(), |o| o.start..range.end));
                } else {
                    simple = false;
                }
            }
        }
    }
    flush_other(&mut other, &mut out);
    out
}

/// An item's text without its marker ("1. ", "- "), its lines joined.
fn item_text(src: &str) -> String {
    let s = src.trim_start();
    let s = s.trim_start_matches(|c: char| c.is_ascii_digit());
    let s = s.strip_prefix(['.', ')', '-', '*', '+']).unwrap_or(s);
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A quote's text without the "> " marks.
fn quote_text(src: &str) -> String {
    src.lines().map(|l| l.trim_start().trim_start_matches('>').trim()).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Inline Markdown (bold, italic, code, strikethrough) as a layout job.
fn inline_job(src: &str, size: f32, color: Color32, family: &str, t: &Tokens, wrap: f32, line_height: Option<f32>) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap.max(10.0);
    let (mut bold, mut italic, mut strike) = (0, 0, 0);
    let fam = |bold: bool| -> FontFamily {
        if bold {
            FontFamily::Name(theme::BOLD.into())
        } else if family.is_empty() {
            FontFamily::Proportional
        } else {
            FontFamily::Name(family.into())
        }
    };
    for ev in Parser::new_ext(src, options()) {
        let fmt = |bold: i32, italic: i32, strike: i32| TextFormat {
            font_id: FontId::new(size, fam(bold > 0)),
            color,
            italics: italic > 0,
            strikethrough: if strike > 0 { Stroke::new(1.0, color) } else { Stroke::NONE },
            line_height,
            ..Default::default()
        };
        match ev {
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold -= 1,
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic -= 1,
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike -= 1,
            Event::Start(Tag::Paragraph) if !job.text.is_empty() => job.append("\n", 0.0, fmt(bold, italic, strike)),
            Event::Text(s) => job.append(&s, 0.0, fmt(bold, italic, strike)),
            Event::Code(s) => job.append(
                &s,
                0.0,
                TextFormat { font_id: FontId::monospace(size * 0.88), color, background: t.opaque(t.sunken), line_height, ..Default::default() },
            ),
            Event::SoftBreak | Event::HardBreak => job.append(" ", 0.0, fmt(bold, italic, strike)),
            _ => {}
        }
    }
    job
}

/// Draw Markdown the designed way.
pub fn show(
    ui: &mut Ui,
    md: &mut egui_commonmark::CommonMarkCache,
    t: &Tokens,
    text: &str,
    viewer: impl Fn(&mut Ui, &mut egui_commonmark::CommonMarkCache, &str),
) {
    let body = 15.5;
    ui.spacing_mut().item_spacing.y = 10.0;
    for (i, b) in blocks(text).into_iter().enumerate() {
        ui.push_id(i, |ui| match b {
            Block::Heading(level, s) => {
                let size = match level {
                    HeadingLevel::H1 => 26.0,
                    HeadingLevel::H2 => 23.0,
                    HeadingLevel::H3 => 19.0,
                    _ => 16.5,
                };
                ui.add_space(4.0);
                ui.add(egui::Label::new(theme::bold(s).size(size).color(t.text)).wrap());
            }
            Block::Table(head, rows) => table(ui, t, &head, &rows),
            Block::List(start, items) => list(ui, t, start, &items, body),
            Block::Quote(s) => quote(ui, t, &s),
            Block::Code(lang, src) => code(ui, md, t, &lang, &src, &viewer),
            Block::Markdown(s) => viewer(ui, md, s),
        });
    }
}

fn list(ui: &mut Ui, t: &Tokens, start: Option<u64>, items: &[String], body: f32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 9.0;
        for (k, item) in items.iter().enumerate() {
            let w = ui.available_width();
            let job = inline_job(item, body, t.text, "", t, w - 36.0, Some(24.0));
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let h = galley.size().y.max(24.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            let p = ui.painter();
            match start {
                Some(n) => {
                    let c = pos2(rect.left() + 12.0, rect.top() + 12.0);
                    p.circle_filled(c, 12.0, t.accent_soft);
                    p.text(c, egui::Align2::CENTER_CENTER, (n + k as u64).to_string(), FontId::new(12.5, FontFamily::Name(theme::BOLD.into())), t.accent_ink);
                }
                None => {
                    p.circle_filled(pos2(rect.left() + 8.0, rect.top() + 12.0), 3.0, t.accent);
                }
            }
            let x = if start.is_some() { 36.0 } else { 22.0 };
            p.galley(pos2(rect.left() + x, rect.top() + (h - galley.size().y) / 2.0), galley.clone(), t.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, galley.text()));
        }
    });
}

fn quote(ui: &mut Ui, t: &Tokens, s: &str) {
    egui::Frame::new().fill(t.sunken).corner_radius(CornerRadius::same(14)).inner_margin(egui::Margin { left: 16, right: 16, top: 12, bottom: 12 }).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            let w = ui.available_width();
            let job = inline_job(s, 16.5, t.text, "", t, w - 18.0, None);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let h = galley.size().y.max(24.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
            let p = ui.painter();
            p.rect_filled(Rect::from_min_size(pos2(rect.left(), rect.center().y - h.min(48.0) / 2.0), vec2(4.0, h.min(48.0))), CornerRadius::same(2), t.accent);
            p.galley(pos2(rect.left() + 18.0, rect.top() + (h - galley.size().y) / 2.0), galley.clone(), t.text);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, galley.text()));
        },
    );
}

fn code(
    ui: &mut Ui,
    md: &mut egui_commonmark::CommonMarkCache,
    t: &Tokens,
    lang: &str,
    src: &str,
    viewer: &impl Fn(&mut Ui, &mut egui_commonmark::CommonMarkCache, &str),
) {
    let bg = t.opaque(t.code_bg);
    egui::Frame::new().fill(bg).corner_radius(CornerRadius::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        ui.painter().rect_filled(head, CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }, over_head(t, bg));
        let label = if lang.is_empty() { "Code".to_string() } else { capitalize(lang) };
        let g = crate::kit::line(ui, &label, crate::kit::font(12.0, theme::SEMIBOLD), t.code_muted, 300.0);
        ui.painter().galley(pos2(head.left() + 16.0, head.center().y - g.size().y / 2.0), g, t.code_muted);
        egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
            let v = ui.visuals_mut();
            v.extreme_bg_color = bg;
            v.widgets.noninteractive.bg_stroke = Stroke::NONE;
            v.override_text_color = Some(t.code_text);
            let fence = format!("```{lang}\n{}\n```", src.trim_end_matches('\n'));
            viewer(ui, md, &fence);
        });
    });
}

fn over_head(t: &Tokens, bg: Color32) -> Color32 {
    theme::over(t.code_head, bg)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// A table with the coloured bold head, row lines, the first column in SemiBold, and a closing
/// row in bold (a total) on the tinted accent.
fn table(ui: &mut Ui, t: &Tokens, head: &[&str], rows: &[Vec<&str>]) {
    let n = head.len().clamp(1, 12);
    let w = ui.available_width();
    let pad_x = 16.0;
    let size = 14.5;
    // Natural widths: the longest single line per column, capped.
    let mut natural = vec![0.0f32; n];
    let measure = |ui: &Ui, s: &str, family: &str| ui.fonts_mut(|f| f.layout_job(inline_job(s, size, t.text, family, t, f32::INFINITY, None))).size().x;
    for (i, nat) in natural.iter_mut().enumerate() {
        let mut m = head.get(i).map(|s| measure(ui, s, theme::BOLD)).unwrap_or(0.0);
        for r in rows.iter().take(200) {
            if let Some(c) = r.get(i) {
                m = m.max(measure(ui, c, if i == 0 { theme::SEMIBOLD } else { "" }));
            }
        }
        *nat = m.min(w * 0.6) + 2.0 * pad_x;
    }
    let total: f32 = natural.iter().sum();
    let widths: Vec<f32> = if total <= w {
        // Extra room goes to the columns after the first, as in the design.
        let extra = w - total;
        let rest = (n - 1).max(1) as f32;
        natural.iter().enumerate().map(|(i, x)| if n == 1 || i > 0 { x + extra / rest } else { *x }).collect()
    } else {
        natural.iter().map(|x| (x / total * w).max(56.0)).collect()
    };
    let last_bold = rows.last().is_some_and(|r| !r.is_empty() && r.iter().all(|c| c.trim().starts_with("**") && c.trim().ends_with("**")));
    let layout =
        |ui: &Ui, s: &str, family: &str, color: Color32, cw: f32| ui.fonts_mut(|f| f.layout_job(inline_job(s, size, color, family, t, cw - 2.0 * pad_x, None)));
    let stroke = if t.glass.is_some() { t.card_stroke } else { t.border };
    let (outer, _) = {
        // Measure all rows first, then paint into one allocated rect.
        let mut heights = Vec::with_capacity(rows.len() + 1);
        let hh =
            head.iter().enumerate().map(|(i, s)| layout(ui, s, theme::BOLD, t.th_text, widths.get(i).copied().unwrap_or(80.0)).size().y).fold(0.0, f32::max)
                + 20.0;
        heights.push(hh);
        for r in rows.iter().take(200) {
            let h = (0..n)
                .map(|i| {
                    layout(ui, r.get(i).copied().unwrap_or(""), if i == 0 { theme::SEMIBOLD } else { "" }, t.text, widths.get(i).copied().unwrap_or(80.0))
                        .size()
                        .y
                })
                .fold(0.0, f32::max)
                + 18.0;
            heights.push(h);
        }
        let total_h: f32 = heights.iter().sum();
        let (rect, resp) = ui.allocate_exact_size(vec2(w, total_h), Sense::hover());
        let p = ui.painter();
        let r14 = CornerRadius::same(14);
        p.rect_filled(rect, r14, t.surface);
        let mut y = rect.top();
        for (row_i, h) in heights.iter().enumerate() {
            let row_rect = Rect::from_min_size(pos2(rect.left(), y), vec2(w, *h));
            let is_head = row_i == 0;
            let is_total = last_bold && row_i == heights.len() - 1;
            if is_head {
                p.rect_filled(row_rect, CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }, t.th_fill);
            } else if is_total {
                p.rect_filled(row_rect, CornerRadius { nw: 0, ne: 0, sw: 14, se: 14 }, t.accent_soft);
            }
            if !is_head && row_i + 1 < heights.len() {
                p.line_segment([pos2(rect.left(), row_rect.bottom()), pos2(rect.right(), row_rect.bottom())], Stroke::new(1.0, t.row_line));
            }
            let mut x = rect.left();
            for i in 0..n {
                let cw = widths.get(i).copied().unwrap_or(80.0);
                let (s, family, color) = if is_head {
                    (head.get(i).copied().unwrap_or(""), theme::BOLD, t.th_text)
                } else {
                    let s = rows.get(row_i - 1).and_then(|r| r.get(i)).copied().unwrap_or("");
                    (s, if i == 0 && !is_total { theme::SEMIBOLD } else { "" }, t.text)
                };
                let g = layout(ui, s, family, color, cw);
                p.galley(pos2(x + pad_x, y + (h - g.size().y) / 2.0), g, color);
                x += cw;
            }
            y += h;
        }
        p.rect_stroke(rect, r14, Stroke::new(1.0, stroke), StrokeKind::Inside);
        (rect, resp)
    };
    let _ = outer;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_blocks_drawn_here() {
        let md = "Vorher **fett**.\n\n## Was passiert?\n\n1. Eins\n2. Zwei **fett**\n\n> 6 CO₂ → Zucker\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```python\nx = 1\n```\n\nNachher";
        let b = blocks(md);
        assert!(matches!(b.first(), Some(Block::Markdown(s)) if s.trim() == "Vorher **fett**."));
        assert_eq!(b.get(1), Some(&Block::Heading(HeadingLevel::H2, "Was passiert?".into())));
        assert_eq!(b.get(2), Some(&Block::List(Some(1), vec!["Eins".into(), "Zwei **fett**".into()])));
        assert_eq!(b.get(3), Some(&Block::Quote("6 CO₂ → Zucker".into())));
        assert_eq!(b.get(4), Some(&Block::Table(vec!["A", "B"], vec![vec!["1", "2"]])));
        assert_eq!(b.get(5), Some(&Block::Code("python".into(), "x = 1\n".into())));
        assert!(matches!(b.get(6), Some(Block::Markdown(s)) if s.trim() == "Nachher"));
    }

    #[test]
    fn links_and_nesting_stay_with_commonmark() {
        let b = blocks("1. [Link](https://x.de)\n2. b\n");
        assert!(matches!(b.first(), Some(Block::Markdown(_))));
        let b = blocks("- a\n  - b\n");
        assert!(matches!(b.first(), Some(Block::Markdown(_))));
        assert!(blocks("").is_empty());
    }

    #[test]
    fn hostile_markdown_never_panics() {
        for s in ["|", "| a |\n|-|\n| b | c | d |", "> ", "```", "1.", "# ", "**", "\u{0}", "|a|\n|---|\n|**x**|"] {
            let _ = blocks(s);
            let _ = item_text(s);
            let _ = quote_text(s);
        }
    }
}
