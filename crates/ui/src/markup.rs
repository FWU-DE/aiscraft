//! What an answer or a document is made of beyond plain Markdown: AIS.chat plots
//! (```` ```jsxgraph-json ````), Mermaid diagrams (```` ```mermaid ````), display formulas
//! (`$$…$$`, `\[…\]`, ```` ```math ````) and paragraphs with formulas in the text (`$…$`,
//! `\(…\)`). Everything else stays Markdown for egui_commonmark.
//!
//! Code (fenced blocks and `inline code`) is never searched for formulas. A `$` only opens a
//! formula when a non-space follows it and only closes one when a non-space precedes it and no
//! digit follows (so "5 $ und 10 $" stays text). Unclosed formulas and fences (still streaming)
//! stay Markdown.

use std::borrow::Cow;

use egui::{Color32, Frame, Margin, RichText, Stroke, Ui};

use crate::theme::Tokens;

/// A piece of an answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Segment<'a> {
    Markdown(Cow<'a, str>),
    /// A ```` ```jsxgraph-json ```` spec.
    Plot(&'a str),
    /// A ```` ```mermaid ```` source.
    Mermaid(&'a str),
    /// A formula on its own line (TeX source).
    Math(&'a str),
    /// A Markdown paragraph (or list) with formulas inside the text.
    Paragraph(&'a str),
    /// A whole HTML document in a ```` ```html ```` block (a learning app).
    App(&'a str),
    /// An ```` ```aiscraft-worksheet ```` block.
    Worksheet(&'a str),
    /// An ```` ```aiscraft-h5p ```` block.
    H5p(&'a str),
}

/// Split an answer for drawing.
pub fn segments(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    for s in crate::plot::segments(text) {
        match s {
            crate::plot::Segment::Plot(p) => out.push(Segment::Plot(p)),
            crate::plot::Segment::Markdown(md) => split_blocks(md, &mut out),
        }
    }
    if out.is_empty() {
        out.push(Segment::Markdown(Cow::Borrowed("")));
    }
    out
}

fn line_end(s: &str, from: usize) -> usize {
    s.get(from..).and_then(|r| r.find('\n')).map(|i| from + i + 1).unwrap_or(s.len())
}

/// A fence opener: (fence char, length, language).
fn fence_open(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let t = line.trim_start_matches(' ');
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    if n < 3 {
        return None;
    }
    let lang = t.get(n..)?.trim();
    if c == '`' && lang.contains('`') {
        return None;
    }
    Some((c, n, lang.split_whitespace().next().unwrap_or("")))
}

fn fence_closes(line: &str, c: char, n: usize) -> bool {
    let t = line.trim();
    t.chars().take_while(|x| *x == c).count() >= n && t.chars().all(|x| x == c)
}

/// Where the fence opened at `start` (line ends at `body_start`) closes: (close line start,
/// close line end).
fn find_close(md: &str, body_start: usize, c: char, n: usize) -> Option<(usize, usize)> {
    let mut q = body_start;
    while q < md.len() {
        let le = line_end(md, q);
        if fence_closes(md.get(q..le).unwrap_or(""), c, n) {
            return Some((q, le));
        }
        q = le;
    }
    None
}

/// The first `$$` (anywhere outside code) or `\[` (at the start of a line) in `line`.
fn display_open(line: &str) -> Option<(usize, &'static str)> {
    if line.trim_start().starts_with("\\[") {
        return Some((line.len() - line.trim_start().len(), "\\["));
    }
    let b = line.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'\\' => i += 2,
            b'`' => i = skip_code_span(b, i),
            b'$' if b.get(i + 1) == Some(&b'$') => return Some((i, "$$")),
            _ => i += 1,
        }
    }
    None
}

/// Past an inline code span starting at `i` (or past the backticks when it doesn't close).
fn skip_code_span(b: &[u8], i: usize) -> usize {
    let n = b.get(i..).map(|r| r.iter().take_while(|c| **c == b'`').count()).unwrap_or(1).max(1);
    let mut j = i + n;
    while j < b.len() {
        if b.get(j) == Some(&b'`') {
            let m = b.get(j..).map(|r| r.iter().take_while(|c| **c == b'`').count()).unwrap_or(1);
            if m == n {
                return j + m;
            }
            j += m.max(1);
        } else {
            j += 1;
        }
    }
    i + n
}

fn split_blocks<'a>(md: &'a str, out: &mut Vec<Segment<'a>>) {
    let mut pending = 0;
    let mut pos = 0;
    while pos < md.len() {
        let le = line_end(md, pos);
        let line = md.get(pos..le).unwrap_or("");
        if let Some((c, n, lang)) = fence_open(line) {
            match find_close(md, le, c, n) {
                Some((cs, ce)) => {
                    let body = md.get(le..cs).unwrap_or("");
                    let seg = match lang {
                        "mermaid" => Some(Segment::Mermaid(body)),
                        "aiscraft-worksheet" | "aiscraft-arbeitsblatt" => Some(Segment::Worksheet(body)),
                        "aiscraft-h5p" => Some(Segment::H5p(body)),
                        "html" | "htm" | "xhtml" | "" if aiscraft_engine::learn::is_html_document(body) => Some(Segment::App(body)),
                        "math" | "latex" | "tex" if !body.trim().is_empty() && body.len() <= crate::math::MAX_TEX => Some(Segment::Math(body.trim())),
                        _ => None,
                    };
                    if let Some(seg) = seg {
                        text_blocks(md.get(pending..pos).unwrap_or(""), out);
                        out.push(seg);
                        pending = ce;
                    }
                    pos = ce;
                    continue;
                }
                // Still streaming: the rest is code.
                None => break,
            }
        }
        if let Some((off, open)) = display_open(line) {
            let start = pos + off;
            let inner_start = start + open.len();
            let closer = if open == "$$" { "$$" } else { "\\]" };
            if let Some(ci) = md.get(inner_start..).and_then(|r| r.find(closer)) {
                let inner = md.get(inner_start..inner_start + ci).unwrap_or("");
                if !inner.trim().is_empty() && inner.len() <= crate::math::MAX_TEX && !inner.contains("\n\n") {
                    text_blocks(md.get(pending..start).unwrap_or(""), out);
                    out.push(Segment::Math(inner.trim()));
                    let after = inner_start + ci + closer.len();
                    pending = after;
                    pos = after;
                    continue;
                }
            }
        }
        pos = le;
    }
    text_blocks(md.get(pending..).unwrap_or(""), out);
}

/// Text between formulas and diagrams: paragraphs with inline formulas are split out.
fn text_blocks<'a>(region: &'a str, out: &mut Vec<Segment<'a>>) {
    if region.trim().is_empty() {
        return;
    }
    // Blocks: paragraphs separated by blank lines; a fenced code block is one block, never math.
    let mut run_start: Option<usize> = None;
    let mut pos = 0;
    let flush_run = |out: &mut Vec<Segment<'a>>, run_start: &mut Option<usize>, end: usize| {
        if let Some(s) = run_start.take()
            && let Some(t) = region.get(s..end).filter(|t| !t.trim().is_empty())
        {
            out.push(Segment::Markdown(Cow::Borrowed(t)));
        }
    };
    while pos < region.len() {
        let le = line_end(region, pos);
        let line = region.get(pos..le).unwrap_or("");
        if line.trim().is_empty() {
            run_start.get_or_insert(pos);
            pos = le;
            continue;
        }
        // One block: up to the next blank line, or a whole fence.
        let mut end = le;
        let is_code = if let Some((c, n, _)) = fence_open(line) {
            end = find_close(region, le, c, n).map(|(_, ce)| ce).unwrap_or(region.len());
            true
        } else {
            while end < region.len() {
                let e2 = line_end(region, end);
                let l2 = region.get(end..e2).unwrap_or("");
                if l2.trim().is_empty() || fence_open(l2).is_some() {
                    break;
                }
                end = e2;
            }
            false
        };
        let block = region.get(pos..end).unwrap_or("");
        if !is_code && !inline_math(block).is_empty() {
            flush_run(out, &mut run_start, pos);
            if block.lines().any(|l| l.trim_start().starts_with('|') || l.trim_start().starts_with('>')) || block.starts_with("    ") {
                out.push(Segment::Markdown(Cow::Owned(math_as_text(block))));
            } else {
                out.push(Segment::Paragraph(block));
            }
        } else {
            run_start.get_or_insert(pos);
        }
        pos = end;
    }
    flush_run(out, &mut run_start, region.len());
}

/// An inline formula: the whole span and the TeX inside, as byte ranges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub inner: (usize, usize),
}

/// The inline formulas of a paragraph.
pub fn inline_math(s: &str) -> Vec<Span> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if out.len() >= 500 {
            break;
        }
        match c {
            b'\\' if b.get(i + 1) == Some(&b'(') => {
                let from = i + 2;
                match s.get(from..).and_then(|r| r.find("\\)")) {
                    Some(k) if k > 0 && k <= crate::math::MAX_TEX && !s.get(from..from + k).unwrap_or("").contains("\n\n") => {
                        out.push(Span { start: i, end: from + k + 2, inner: (from, from + k) });
                        i = from + k + 2;
                    }
                    _ => i += 2,
                }
            }
            b'\\' => i += 2,
            b'`' => i = skip_code_span(b, i),
            b'$' => {
                let next = b.get(i + 1).copied();
                if next == Some(b'$') {
                    i += 2;
                    continue;
                }
                if next.is_none_or(|n| n.is_ascii_whitespace()) {
                    i += 1;
                    continue;
                }
                let mut j = i + 1;
                let mut found = None;
                while let Some(&d) = b.get(j) {
                    if j - i > crate::math::MAX_TEX || (d == b'\n' && b.get(j + 1) == Some(&b'\n')) {
                        break;
                    }
                    if d == b'\\' {
                        j += 2;
                        continue;
                    }
                    if d == b'$' {
                        let before = b.get(j - 1).copied().unwrap_or(b' ');
                        let after = b.get(j + 1).copied();
                        if !before.is_ascii_whitespace() && !after.is_some_and(|a| a.is_ascii_digit()) {
                            found = Some(j);
                        }
                        break;
                    }
                    j += 1;
                }
                match found {
                    Some(j) => {
                        out.push(Span { start: i, end: j + 1, inner: (i + 1, j) });
                        i = j + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Formulas replaced by their Unicode text (escaped for Markdown), for tables and quotes.
fn math_as_text(block: &str) -> String {
    let mut out = String::with_capacity(block.len());
    let mut last = 0;
    for sp in inline_math(block) {
        out.push_str(block.get(last..sp.start).unwrap_or(""));
        let t = pretty_scripts(&crate::math::to_text(block.get(sp.inner.0..sp.inner.1).unwrap_or("")));
        for c in t.chars() {
            if "\\*_|`[]<>#".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        last = sp.end;
    }
    out.push_str(block.get(last..).unwrap_or(""));
    out
}

/// `x^2` → `x²`, `a_1` → `a₁` for single characters that have a Unicode script form.
fn pretty_scripts(s: &str) -> String {
    const SUP: [(char, char); 13] = [
        ('0', '⁰'),
        ('1', '¹'),
        ('2', '²'),
        ('3', '³'),
        ('4', '⁴'),
        ('5', '⁵'),
        ('6', '⁶'),
        ('7', '⁷'),
        ('8', '⁸'),
        ('9', '⁹'),
        ('n', 'ⁿ'),
        ('+', '⁺'),
        ('-', '⁻'),
    ];
    const SUB: [(char, char); 10] = [('0', '₀'), ('1', '₁'), ('2', '₂'), ('3', '₃'), ('4', '₄'), ('5', '₅'), ('6', '₆'), ('7', '₇'), ('8', '₈'), ('9', '₉')];
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let table: &[(char, char)] = match c {
            '^' => &SUP,
            '_' => &SUB,
            _ => &[],
        };
        let next = chars.get(i + 1).copied();
        let after = chars.get(i + 2).copied();
        if let Some(n) = next
            && !after.is_some_and(|a| a.is_alphanumeric() || a == '(')
            && let Some((_, m)) = table.iter().find(|(k, _)| *k == n)
        {
            out.push(*m);
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

// ---- drawing -----------------------------------------------------------------------------

/// Something a segment asks the app to do.
pub enum Action {
    /// Open this outline as a new whiteboard.
    Board(String),
}

/// Draw one segment (plots too, without the app's extras).
pub fn show(ui: &mut Ui, md: &mut egui_commonmark::CommonMarkCache, t: &Tokens, seg: &Segment, salt: impl std::hash::Hash + std::fmt::Debug) -> Option<Action> {
    match seg {
        Segment::Markdown(text) => {
            ui.scope(|ui| {
                // Tables are striped grids, and the stripe made a single row look selected: no stripes.
                ui.visuals_mut().faint_bg_color = Color32::TRANSPARENT;
                for (i, (table, piece)) in table_pieces(text).into_iter().enumerate() {
                    ui.push_id(i, |ui| {
                        if table {
                            // egui_commonmark draws a table as a grid in Frame::group (6 px margin,
                            // default item spacing): cells sat cramped against each other. More
                            // room between the cells, and our own card frame around the group.
                            ui.spacing_mut().item_spacing = egui::vec2(28.0, 4.0);
                            ui.visuals_mut().widgets.noninteractive.bg_stroke = Stroke::NONE;
                            Frame::new()
                                .stroke(Stroke::new(1.0, t.border))
                                .corner_radius(egui::CornerRadius::same(t.radius))
                                .inner_margin(Margin::symmetric(8, 4))
                                .show(ui, |ui| markdown_viewer(ui, md, t, piece));
                            ui.add_space(8.0);
                        } else {
                            markdown_viewer(ui, md, t, piece);
                        }
                    });
                }
            });
            None
        }
        Segment::Math(tex) => {
            crate::math::show_display(ui, tex, t.text);
            None
        }
        Segment::Paragraph(text) => {
            show_paragraph(ui, t, text);
            None
        }
        Segment::Plot(src) => {
            framed(ui, t, |ui| match crate::plot::parse_plot(src) {
                Ok(p) => crate::plot::show(ui, &p, t.text, t.muted, t.code_bg),
                Err(e) => {
                    ui.label(RichText::new(format!("Diagramm: {e}")).color(t.warning).small());
                }
            });
            None
        }
        Segment::Mermaid(src) => mermaid_block(ui, t, src, salt),
        // In documents: the source, folded (the chat shows these as cards).
        Segment::App(src) | Segment::Worksheet(src) | Segment::H5p(src) => {
            let label = match seg {
                Segment::App(_) => "Lern-App (HTML)",
                Segment::H5p(_) => "H5P (JSON)",
                _ => "Arbeitsblatt (JSON)",
            };
            egui::CollapsingHeader::new(small_muted(t, label)).id_salt(("learn", salt)).default_open(false).show(ui, |ui| source_view(ui, t, src));
            None
        }
    }
}

fn markdown_viewer(ui: &mut Ui, md: &mut egui_commonmark::CommonMarkCache, t: &Tokens, text: &str) {
    egui_commonmark::CommonMarkViewer::new()
        .default_width(Some(ui.available_width().max(1.0) as usize))
        .syntax_theme_light(t.code_theme())
        .syntax_theme_dark(t.code_theme())
        .show(ui, md, text);
}

/// Split Markdown into runs of text and GFM tables (`true`), outside code fences, so tables can be
/// drawn with more room. Without a table the text comes back as one piece.
pub fn table_pieces(text: &str) -> Vec<(bool, &str)> {
    let is_delim = |l: &str| {
        let l = l.trim();
        l.contains('-') && l.contains('|') && l.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
    };
    let mut out = Vec::new();
    let mut start = 0;
    let mut pos = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut table_from: Option<usize> = None;
    while pos < text.len() {
        let end = line_end(text, pos);
        let line = text.get(pos..end).unwrap_or("");
        if let Some((c, n)) = fence {
            if fence_closes(line, c, n) {
                fence = None;
            }
        } else if let Some(from) = table_from {
            if line.trim().is_empty() || !line.contains('|') {
                if from > start {
                    out.push((false, text.get(start..from).unwrap_or("")));
                }
                out.push((true, text.get(from..pos).unwrap_or("")));
                start = pos;
                table_from = None;
            }
        } else if let Some((c, n, _)) = fence_open(line) {
            fence = Some((c, n));
        } else if line.trim_start().starts_with('|') {
            let next_end = line_end(text, end);
            if is_delim(text.get(end..next_end).unwrap_or("")) {
                table_from = Some(pos);
            }
        }
        pos = end;
    }
    if let Some(from) = table_from {
        if from > start {
            out.push((false, text.get(start..from).unwrap_or("")));
        }
        out.push((true, text.get(from..).unwrap_or("")));
    } else if start < text.len() || out.is_empty() {
        out.push((false, text.get(start..).unwrap_or("")));
    }
    out
}

fn framed<R>(ui: &mut Ui, t: &Tokens, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new().stroke(Stroke::new(1.0, t.border)).corner_radius(egui::CornerRadius::same(t.radius)).inner_margin(Margin::same(10)).show(ui, add).inner
}

fn small_muted(t: &Tokens, s: impl Into<String>) -> RichText {
    RichText::new(s).small().color(t.muted)
}

fn source_view(ui: &mut Ui, t: &Tokens, src: &str) {
    Frame::new().fill(t.code_bg).corner_radius(egui::CornerRadius::same(4)).inner_margin(Margin::same(8)).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(src.chars().take(6000).collect::<String>()).monospace().small()).wrap());
    });
}

fn mermaid_block(ui: &mut Ui, t: &Tokens, src: &str, salt: impl std::hash::Hash + std::fmt::Debug) -> Option<Action> {
    let pal = crate::mermaid::Palette { text: t.text, muted: t.muted, fill: t.surface_hi, stroke: t.accent, accent: t.accent, bg: t.bg };
    let mut action = None;
    framed(ui, t, |ui| {
        let drawn = match crate::mermaid::parse(src) {
            Ok(crate::mermaid::Diagram::Flowchart(f)) => {
                crate::mermaid::show_flowchart(ui, &f, &pal);
                if f.truncated {
                    ui.label(small_muted(t, "Nicht alles dargestellt: zu groß oder Angaben, die AIS.ChatCraft nicht kennt."));
                }
                true
            }
            Ok(crate::mermaid::Diagram::Mindmap(root)) => {
                // Top right, like the copy button of a code block.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.small_button("Als Whiteboard").on_hover_text("Mindmap als bearbeitbares Whiteboard öffnen").clicked() {
                        action = Some(Action::Board(crate::mermaid::mindmap_outline(&root)));
                    }
                });
                crate::mermaid::show_mindmap(ui, &root, &pal);
                true
            }
            Ok(crate::mermaid::Diagram::Unsupported(kind)) => {
                ui.label(small_muted(t, format!("Mermaid-Diagramm „{kind}“: AIS.ChatCraft zeichnet bisher nur Flussdiagramme und Mindmaps. Quelltext:")));
                source_view(ui, t, src);
                false
            }
            Err(e) => {
                ui.label(RichText::new(format!("Mermaid-Diagramm: {e}")).color(t.warning).small());
                source_view(ui, t, src);
                false
            }
        };
        if drawn {
            egui::CollapsingHeader::new(small_muted(t, "Quelltext")).id_salt(("mermaid", salt)).default_open(false).show(ui, |ui| {
                source_view(ui, t, src);
                if ui.small_button("Kopieren").clicked() {
                    ui.ctx().copy_text(src.to_string());
                }
            });
        }
    });
    action
}

/// One piece of running text.
enum Piece {
    Text(RichText),
    Math(usize),
    Link(RichText, String),
}

/// A paragraph, heading or list whose text holds formulas: drawn as wrapped labels with the
/// formulas in line. Bold, italic, code, strikethrough and links keep their look.
pub fn show_paragraph(ui: &mut Ui, t: &Tokens, text: &str) {
    use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};
    let spans = inline_math(text);
    // Each formula becomes one private-use character the Markdown parser leaves alone.
    let mut src = String::with_capacity(text.len());
    let mut formulas: Vec<&str> = Vec::new();
    let mut last = 0;
    for sp in spans.iter().take(6000) {
        src.push_str(text.get(last..sp.start).unwrap_or(""));
        src.push(char::from_u32(0xE000 + formulas.len() as u32).unwrap_or('\u{E000}'));
        formulas.push(text.get(sp.inner.0..sp.inner.1).unwrap_or(""));
        last = sp.end;
    }
    src.push_str(text.get(last..).unwrap_or(""));

    let body = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size).unwrap_or(14.0);
    let mut pieces: Vec<Piece> = Vec::new();
    let (mut bold, mut italic, mut strike) = (0u32, 0u32, 0u32);
    let mut heading: Option<HeadingLevel> = None;
    let mut link: Option<(String, String)> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut prefix: Option<String> = None;
    let mut first_block = true;

    let style = |s: &str, bold: u32, italic: u32, strike: u32, heading: Option<HeadingLevel>| {
        let mut r = RichText::new(s).color(t.text);
        if let Some(h) = heading {
            r = r.size(
                body * match h {
                    HeadingLevel::H1 => 1.6,
                    HeadingLevel::H2 => 1.35,
                    HeadingLevel::H3 => 1.15,
                    _ => 1.0,
                },
            );
            r = r.strong();
        }
        if bold > 0 {
            r = r.strong();
        }
        if italic > 0 {
            r = r.italics();
        }
        if strike > 0 {
            r = r.strikethrough();
        }
        r
    };
    let flush = |ui: &mut Ui, pieces: &mut Vec<Piece>, prefix: &mut Option<String>, depth: usize, first: &mut bool| {
        if pieces.is_empty() && prefix.is_none() {
            return;
        }
        if !*first {
            ui.add_space(body * 0.35);
        }
        *first = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            if depth > 0 {
                ui.add_space(body * 1.2 * depth as f32);
            }
            if let Some(p) = prefix.take() {
                ui.label(RichText::new(p).color(t.text));
            }
            for p in pieces.drain(..) {
                match p {
                    Piece::Text(r) => {
                        ui.label(r);
                    }
                    Piece::Math(i) => {
                        crate::math::show_inline(ui, formulas.get(i).copied().unwrap_or(""), t.text);
                    }
                    Piece::Link(r, url) => {
                        ui.hyperlink_to(r, url);
                    }
                }
            }
        });
    };

    let push_text = |pieces: &mut Vec<Piece>, s: &str, link: &mut Option<(String, String)>, bold, italic, strike, heading| {
        if let Some((label, _)) = link.as_mut() {
            label.push_str(s);
            return;
        }
        let mut buf = String::new();
        for c in s.chars() {
            let code = c as u32;
            if (0xE000..0xE000 + 6000).contains(&code) {
                if !buf.is_empty() {
                    pieces.push(Piece::Text(style(&buf, bold, italic, strike, heading)));
                    buf.clear();
                }
                pieces.push(Piece::Math((code - 0xE000) as usize));
            } else {
                buf.push(c);
            }
        }
        if !buf.is_empty() {
            pieces.push(Piece::Text(style(&buf, bold, italic, strike, heading)));
        }
    };

    let parser = pulldown_cmark::Parser::new_ext(&src, pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    for ev in parser.take(20_000) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => heading = Some(level),
            Event::End(TagEnd::Heading(_)) => {
                flush(ui, &mut pieces, &mut prefix, lists.len(), &mut first_block);
                heading = None;
            }
            Event::Start(Tag::List(start)) => {
                flush(ui, &mut pieces, &mut prefix, lists.len(), &mut first_block);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(ui, &mut pieces, &mut prefix, lists.len(), &mut first_block);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(ui, &mut pieces, &mut prefix, lists.len().saturating_sub(1), &mut first_block);
                prefix = Some(match lists.last_mut() {
                    Some(Some(n)) => {
                        let p = format!("{n}. ");
                        *n += 1;
                        p
                    }
                    _ => "•  ".to_string(),
                });
            }
            Event::End(TagEnd::Item) | Event::End(TagEnd::Paragraph) => {
                flush(ui, &mut pieces, &mut prefix, lists.len().saturating_sub(1), &mut first_block);
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => link = Some((String::new(), dest_url.to_string())),
            Event::End(TagEnd::Link) => {
                if let Some((label, url)) = link.take() {
                    pieces.push(Piece::Link(style(&label, bold, italic, strike, heading), url));
                }
            }
            Event::Text(s) => push_text(&mut pieces, &s, &mut link, bold, italic, strike, heading),
            Event::Code(s) => pieces.push(Piece::Text(RichText::new(s.to_string()).code().color(t.text))),
            Event::SoftBreak => push_text(&mut pieces, " ", &mut link, bold, italic, strike, heading),
            Event::HardBreak => flush(ui, &mut pieces, &mut prefix, lists.len().saturating_sub(1), &mut first_block),
            Event::Html(s) | Event::InlineHtml(s) => push_text(&mut pieces, &s, &mut link, bold, italic, strike, heading),
            _ => {}
        }
    }
    flush(ui, &mut pieces, &mut prefix, lists.len().saturating_sub(1), &mut first_block);
    ui.add_space(body * 0.5);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(s: &str) -> Segment<'_> {
        Segment::Markdown(Cow::Borrowed(s))
    }

    #[test]
    fn tables_are_split_out_of_markdown() {
        let s = "Vorher\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nNachher";
        assert_eq!(table_pieces(s), vec![(false, "Vorher\n\n"), (true, "| A | B |\n|---|---|\n| 1 | 2 |\n"), (false, "\nNachher")]);
        assert_eq!(table_pieces("| A |\n|---|\n| 1 |"), vec![(true, "| A |\n|---|\n| 1 |")]);
        // Not a table: no delimiter row, or inside a code fence.
        assert_eq!(table_pieces("| nur | Text |\nweiter"), vec![(false, "| nur | Text |\nweiter")]);
        assert_eq!(table_pieces("```\n| A |\n|---|\n```\n"), vec![(false, "```\n| A |\n|---|\n```\n")]);
        assert_eq!(table_pieces(""), vec![(false, "")]);
    }

    #[test]
    fn finds_inline_formulas_but_not_prices_or_code() {
        let s = "Es gilt $a^2 + b^2 = c^2$ und \\(x\\).";
        let spans = inline_math(s);
        assert_eq!(spans.len(), 2);
        assert_eq!(&s[spans[0].inner.0..spans[0].inner.1], "a^2 + b^2 = c^2");
        assert_eq!(&s[spans[1].inner.0..spans[1].inner.1], "x");
        assert!(inline_math("Das kostet 5 $ und 10 $.").is_empty());
        assert!(inline_math("Von $5 bis $10").is_empty(), "a closing $ needs a non-space before it");
        assert!(inline_math("Preis: $5$10").is_empty(), "no digit after a closing $");
        assert!(inline_math("Code: `$x$` bleibt").is_empty());
        assert!(inline_math("Escaped \\$x\\$ bleibt").is_empty());
        assert!(inline_math("$x").is_empty(), "unclosed: still streaming");
        assert!(inline_math("$$").is_empty());
    }

    #[test]
    fn splits_answers_into_blocks() {
        let text = "Die Lösung:\n\n$$x = \\frac{1}{2}$$\n\nMit $x$ gerechnet.\n\nEnde.";
        assert_eq!(segments(text), vec![md("Die Lösung:\n\n"), Segment::Math("x = \\frac{1}{2}"), Segment::Paragraph("Mit $x$ gerechnet.\n"), md("\nEnde.")]);

        let text = "Davor\n\\[\na+b\n\\]\nDanach";
        assert_eq!(segments(text), vec![md("Davor\n"), Segment::Math("a+b"), md("\nDanach")]);

        let text = "Ablauf:\n\n```mermaid\ngraph TD\nA-->B\n```\n\nFertig";
        assert_eq!(segments(text), vec![md("Ablauf:\n\n"), Segment::Mermaid("graph TD\nA-->B\n"), md("\nFertig")]);

        // Code is code: no formulas, no diagrams inside other fences.
        let text = "```python\nprint('$x$')\n$$y$$\n```";
        assert_eq!(segments(text), vec![md(text)]);
        let text = "```markdown\n```mermaid\ngraph TD\n```";
        assert!(!segments(text).iter().any(|s| matches!(s, Segment::Mermaid(_))));

        // Streaming: nothing closed yet.
        assert_eq!(segments("Gleich: $$x = "), vec![md("Gleich: $$x = ")]);
        assert_eq!(segments("```mermaid\ngraph TD\nA-->"), vec![md("```mermaid\ngraph TD\nA-->")]);

        // Plots still come out as plots.
        let text = "Graph:\n```jsxgraph-json\n{}\n```\nund $x$";
        let s = segments(text);
        assert!(matches!(s[1], Segment::Plot("{}\n")));
        assert!(matches!(s[2], Segment::Paragraph(_)));

        // Tables keep Markdown; their formulas become text.
        let s = segments("| a | b |\n|---|---|\n| $x^2$ | $\\alpha$ |");
        let [Segment::Markdown(Cow::Owned(t))] = s.as_slice() else { panic!("{s:?}") };
        assert!(t.contains('α') && t.contains("x²") && !t.contains('$'), "{t}");

        assert_eq!(segments(""), vec![md("")]);
        assert_eq!(segments("Nur Text."), vec![md("Nur Text.")]);
    }

    #[test]
    fn hostile_text_never_panics() {
        for s in ["$", "$$", "\\[", "\\(", "`", "```", "$\\", "\\", "$ä$", "ä$ö$ü", "$$\n\n$$", "\\(\\)", "~~~mermaid\n~~~", "$`$`$"] {
            let _ = segments(s);
            let _ = inline_math(s);
            let _ = math_as_text(s);
        }
        let many = "$x$ ".repeat(5000);
        assert!(inline_math(&many).len() <= 500);
    }
}
