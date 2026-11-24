//! Markdown to PDF, written directly: A4 pages, the standard fonts Helvetica and Courier (no
//! embedded font program), WinAnsi text, line breaking by craft-pdf's Standard-14 metrics.
//!
//! Headings, paragraphs with bold, italic and code, lists (nested, numbered), quotes, code
//! blocks, tables and rules. Characters outside WinAnsi (emoji, Greek, arrows) become a close
//! ASCII stand-in or `?`. Content streams stay uncompressed, so tests can read the text.

use std::fmt::Write as _;

use aiscraft_core::{Error, Result};
use craft_pdf::fonts::{Std14, win_ansi};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// More pages than this is not a document someone prints from a chat.
pub const MAX_PAGES: usize = 500;

const PAGE_W: f64 = 595.28;
const PAGE_H: f64 = 841.89;
const MARGIN: f64 = 56.0;
const BODY: f64 = 11.0;
const TEXT: (f64, f64, f64) = (0.118, 0.161, 0.231);
const ACCENT: (f64, f64, f64) = (0.063, 0.498, 0.678);
const MUTED: (f64, f64, f64) = (0.39, 0.45, 0.53);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
}

impl Face {
    const ALL: [Face; 5] = [Face::Regular, Face::Bold, Face::Italic, Face::BoldItalic, Face::Mono];

    fn std14(self) -> Std14 {
        match self {
            Face::Regular => Std14::Helvetica,
            Face::Bold => Std14::HelveticaBold,
            Face::Italic => Std14::HelveticaOblique,
            Face::BoldItalic => Std14::HelveticaBoldOblique,
            Face::Mono => Std14::Courier,
        }
    }

    fn resource(self) -> &'static str {
        match self {
            Face::Regular => "F1",
            Face::Bold => "F2",
            Face::Italic => "F3",
            Face::BoldItalic => "F4",
            Face::Mono => "F5",
        }
    }

    fn styled(bold: bool, italic: bool) -> Face {
        match (bold, italic) {
            (false, false) => Face::Regular,
            (true, false) => Face::Bold,
            (false, true) => Face::Italic,
            (true, true) => Face::BoldItalic,
        }
    }

    fn bolder(self) -> Face {
        match self {
            Face::Regular => Face::Bold,
            Face::Italic => Face::BoldItalic,
            f => f,
        }
    }
}

/// Characters WinAnsi lacks that models like to write, as text it has.
fn stand_in(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '→' | '⇒' | '➜' => out.push_str("->"),
            '←' | '⇐' => out.push_str("<-"),
            '↔' | '⇔' => out.push_str("<->"),
            '≤' => out.push_str("<="),
            '≥' => out.push_str(">="),
            '≠' => out.push_str("!="),
            '≈' => out.push('~'),
            '✓' | '✔' | '☑' => out.push('x'),
            '✗' | '✕' | '✘' | '×' => out.push('x'),
            '☐' => out.push_str("[ ]"),
            '\u{a0}' | '\u{2009}' | '\u{202f}' => out.push(' '),
            '\u{200b}' | '\u{fe0f}' | '\u{200d}' => {}
            '\t' => out.push_str("    "),
            c if craft_pdf::fonts::win_ansi_encodable(c) => out.push(c),
            c if c.is_control() => {}
            _ => out.push('?'),
        }
    }
    out
}

/// A PDF string literal of WinAnsi bytes.
fn pdf_string(text: &str) -> String {
    let mut s = String::from("(");
    for b in win_ansi(text) {
        match b {
            b'(' | b')' | b'\\' => {
                s.push('\\');
                s.push(char::from(b));
            }
            0x20..=0x7e => s.push(char::from(b)),
            _ => {
                let _ = write!(s, "\\{b:03o}");
            }
        }
    }
    s.push(')');
    s
}

/// A text string for the document information (UTF-16BE with a byte order mark).
fn pdf_text_string(text: &str) -> String {
    let mut s = String::from("<FEFF");
    for u in text.encode_utf16() {
        let _ = write!(s, "{u:04X}");
    }
    s.push('>');
    s
}

#[derive(Clone, Debug)]
struct Word {
    text: String,
    face: Face,
    /// A space goes in front of it (unless it starts a line).
    space: bool,
}

/// Inline content of a block, split into words.
#[derive(Default)]
struct Inline {
    words: Vec<Word>,
    /// The next word follows whitespace.
    pending_space: bool,
    /// Forced line breaks: indexes into `words` where a new line starts.
    breaks: Vec<usize>,
}

impl Inline {
    fn push(&mut self, text: &str, face: Face) {
        let text = stand_in(text);
        let mut first = true;
        for (i, part) in text.split(' ').enumerate() {
            if i > 0 {
                self.pending_space = true;
            }
            if part.is_empty() {
                continue;
            }
            // Text that continues a word without a space (`**fett**en`) glues on.
            let glue = first && !self.pending_space && !self.words.is_empty() && !self.breaks.contains(&self.words.len());
            if glue && let Some(last) = self.words.last_mut().filter(|w| w.face == face) {
                last.text.push_str(part);
            } else {
                self.words.push(Word { text: part.to_string(), face, space: self.pending_space && !self.words.is_empty() });
            }
            self.pending_space = false;
            first = false;
        }
    }

    fn hard_break(&mut self) {
        self.breaks.push(self.words.len());
        self.pending_space = false;
    }

    fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// One laid-out line: runs of (face, text).
type Line = Vec<(Face, String)>;

fn width(face: Face, text: &str, size: f64) -> f64 {
    face.std14().text_width(text, size)
}

/// Greedy line breaking. Words longer than a line are cut.
fn wrap(inline: &Inline, size: f64, max_w: f64) -> Vec<Line> {
    let space_w = |f: Face| width(f, " ", size);
    let mut lines: Vec<Line> = Vec::new();
    let mut line: Line = Vec::new();
    let mut w = 0.0;
    let push_word = |line: &mut Line, face: Face, text: &str, space: bool| {
        let t = if space { format!(" {text}") } else { text.to_string() };
        match line.last_mut() {
            Some((f, s)) if *f == face => s.push_str(&t),
            _ => line.push((face, t)),
        }
    };
    for (i, word) in inline.words.iter().enumerate() {
        if inline.breaks.contains(&i) && i > 0 {
            lines.push(std::mem::take(&mut line));
            w = 0.0;
        }
        let ww = width(word.face, &word.text, size);
        let sw = if word.space && !line.is_empty() { space_w(word.face) } else { 0.0 };
        if !line.is_empty() && w + sw + ww > max_w {
            lines.push(std::mem::take(&mut line));
            w = 0.0;
        }
        if ww > max_w {
            // Cut a very long word (a URL) into pieces that fit.
            let mut piece = String::new();
            for c in word.text.chars() {
                let mut next = piece.clone();
                next.push(c);
                if !piece.is_empty() && w + width(word.face, &next, size) > max_w {
                    push_word(&mut line, word.face, &piece, false);
                    lines.push(std::mem::take(&mut line));
                    w = 0.0;
                    piece = c.to_string();
                } else {
                    piece = next;
                }
            }
            w += width(word.face, &piece, size);
            push_word(&mut line, word.face, &piece, false);
            continue;
        }
        let space = word.space && !line.is_empty();
        w += if space { sw } else { 0.0 } + ww;
        push_word(&mut line, word.face, &word.text, space);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

struct Pages {
    done: Vec<String>,
    cur: String,
    y: f64,
}

impl Pages {
    fn new() -> Self {
        Pages { done: Vec::new(), cur: String::new(), y: PAGE_H - MARGIN }
    }

    fn new_page(&mut self) -> Result<()> {
        if self.done.len() + 1 >= MAX_PAGES {
            return Err(Error::invalid(format!("Das Dokument ist für ein PDF zu lang (über {MAX_PAGES} Seiten)")));
        }
        self.done.push(std::mem::take(&mut self.cur));
        self.y = PAGE_H - MARGIN;
        Ok(())
    }

    /// Room for `h` points, else the next page.
    fn ensure(&mut self, h: f64) -> Result<()> {
        if self.y - h < MARGIN && self.y < PAGE_H - MARGIN {
            self.new_page()?;
        }
        Ok(())
    }

    fn text_line(&mut self, x: f64, size: f64, color: (f64, f64, f64), line: &Line) {
        let _ = write!(self.cur, "BT {:.3} {:.3} {:.3} rg ", color.0, color.1, color.2);
        let _ = write!(self.cur, "{x:.2} {:.2} Td ", self.y);
        for (face, text) in line {
            let _ = write!(self.cur, "/{} {size:.1} Tf {} Tj ", face.resource(), pdf_string(text));
        }
        self.cur.push_str("ET\n");
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, fill: (f64, f64, f64)) {
        let _ = writeln!(self.cur, "{:.3} {:.3} {:.3} rg {x:.2} {y:.2} {w:.2} {h:.2} re f", fill.0, fill.1, fill.2);
    }

    fn stroke_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let _ = writeln!(self.cur, "0.85 0.90 0.93 RG 0.6 w {x:.2} {y:.2} {w:.2} {h:.2} re S");
    }

    fn rule(&mut self, x0: f64, x1: f64) {
        let _ = writeln!(self.cur, "0.85 0.90 0.93 RG 0.8 w {x0:.2} {:.2} m {x1:.2} {:.2} l S", self.y, self.y);
    }

    /// Lines of a paragraph at `x`, the first line after `prefix` (a bullet) drawn at `prefix_x`.
    fn paragraph(&mut self, lines: &[Line], x: f64, size: f64, color: (f64, f64, f64), prefix: Option<(f64, &str)>) -> Result<()> {
        let leading = size * 1.4;
        for (i, line) in lines.iter().enumerate() {
            self.ensure(leading)?;
            self.y -= leading;
            if i == 0
                && let Some((px, p)) = prefix
            {
                self.text_line(px, size, color, &vec![(Face::Regular, p.to_string())]);
            }
            self.text_line(x, size, color, line);
        }
        Ok(())
    }

    fn finish(mut self) -> Vec<String> {
        if !self.cur.is_empty() || self.done.is_empty() {
            self.done.push(self.cur);
        }
        self.done
    }
}

struct ListState {
    next: Option<u64>,
}

#[derive(Default)]
struct TableState {
    rows: Vec<(bool, Vec<Inline>)>,
    head: bool,
    cell: Option<Inline>,
    row: Vec<Inline>,
}

/// Markdown to the bytes of a PDF file.
pub fn markdown_pdf(title: &str, md: &str) -> Result<Vec<u8>> {
    let pages = layout(md)?;
    Ok(assemble(title, &pages))
}

fn layout(md: &str) -> Result<Vec<String>> {
    let mut p = Pages::new();
    let content_w = PAGE_W - 2.0 * MARGIN;
    let mut inline = Inline::default();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut heading: Option<HeadingLevel> = None;
    let mut lists: Vec<ListState> = Vec::new();
    let mut item_prefix: Option<String> = None;
    let mut quote = 0u32;
    let mut code: Option<String> = None;
    let mut table: Option<TableState> = None;

    let indent = |lists: &Vec<ListState>, quote: u32| MARGIN + 18.0 * lists.len() as f64 + 14.0 * f64::from(quote);

    // Lays out the collected inline text as a paragraph (or list item text).
    macro_rules! flush {
        () => {{
            if !inline.is_empty() {
                let x = indent(&lists, quote);
                let color = if quote > 0 { MUTED } else { TEXT };
                let lines = wrap(&inline, BODY, PAGE_W - MARGIN - x);
                let prefix = item_prefix.take();
                p.paragraph(&lines, x, BODY, color, prefix.as_deref().map(|s| (x - 14.0, s)))?;
                p.y -= if lists.is_empty() { 6.0 } else { 2.0 };
            }
            inline = Inline::default();
        }};
    }

    let face_now = |bold: u32, italic: u32| Face::styled(bold > 0, italic > 0);

    for ev in Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush!();
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                let size = match heading.take() {
                    Some(HeadingLevel::H1) => 20.0,
                    Some(HeadingLevel::H2) => 16.0,
                    Some(HeadingLevel::H3) => 13.5,
                    _ => 12.0,
                };
                for w in inline.words.iter_mut() {
                    w.face = w.face.bolder();
                }
                let lines = wrap(&inline, size, content_w);
                // Keep a heading with at least two lines of what follows.
                p.ensure(size * 1.4 * lines.len() as f64 + size * 0.6 + BODY * 3.0)?;
                p.y -= size * 0.6;
                p.paragraph(&lines, MARGIN, size, ACCENT, None)?;
                p.y -= 6.0;
                inline = Inline::default();
            }
            Event::Start(Tag::Paragraph) => {
                if table.is_none() {
                    flush!();
                }
            }
            Event::End(TagEnd::Paragraph) => flush!(),
            Event::Start(Tag::List(start)) => {
                flush!();
                lists.push(ListState { next: start });
            }
            Event::End(TagEnd::List(_)) => {
                flush!();
                lists.pop();
                if lists.is_empty() {
                    p.y -= 4.0;
                }
            }
            Event::Start(Tag::Item) => {
                flush!();
                item_prefix = Some(match lists.last_mut().and_then(|l| l.next.as_mut()) {
                    Some(n) => {
                        let s = format!("{n}.");
                        *n = n.saturating_add(1);
                        s
                    }
                    None => "\u{2022}".to_string(),
                });
            }
            Event::End(TagEnd::Item) => flush!(),
            Event::Start(Tag::BlockQuote(_)) => {
                flush!();
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush!();
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush!();
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                let text = code.take().unwrap_or_default();
                let size = 9.5;
                let leading = size * 1.35;
                let x = indent(&lists, quote);
                let max_w = PAGE_W - MARGIN - x - 12.0;
                p.y -= 2.0;
                for raw in text.trim_end_matches('\n').split('\n') {
                    let mut src = Inline::default();
                    src.words.push(Word { text: stand_in(raw), face: Face::Mono, space: false });
                    let lines = if raw.is_empty() { vec![vec![]] } else { wrap(&src, size, max_w) };
                    for line in lines {
                        p.ensure(leading)?;
                        p.rect(x, p.y - leading - 1.0, PAGE_W - MARGIN - x, leading + 1.0, (0.945, 0.961, 0.976));
                        p.y -= leading;
                        p.text_line(x + 6.0, size, TEXT, &line);
                    }
                }
                p.y -= 8.0;
            }
            Event::Start(Tag::Table(_)) => {
                flush!();
                table = Some(TableState::default());
            }
            Event::Start(Tag::TableHead) => {
                if let Some(t) = table.as_mut() {
                    t.head = true;
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push((true, row));
                    t.head = false;
                }
            }
            Event::Start(Tag::TableRow) => {}
            Event::End(TagEnd::TableRow) => {
                if let Some(t) = table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push((false, row));
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(t) = table.as_mut() {
                    t.cell = Some(Inline::default());
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = table.as_mut() {
                    let cell = t.cell.take().unwrap_or_default();
                    t.row.push(cell);
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    draw_table(&mut p, t, indent(&lists, quote))?;
                }
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Text(t) => {
                if let Some(c) = code.as_mut() {
                    c.push_str(&t);
                    continue;
                }
                let face = face_now(bold, italic);
                let face = if table.as_ref().is_some_and(|t| t.head) { face.bolder() } else { face };
                let target = match table.as_mut().and_then(|t| t.cell.as_mut()) {
                    Some(cell) => cell,
                    None => &mut inline,
                };
                for (i, part) in t.split('\n').enumerate() {
                    if i > 0 {
                        target.pending_space = true;
                    }
                    target.push(part, face);
                }
            }
            Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                let target = match table.as_mut().and_then(|t| t.cell.as_mut()) {
                    Some(cell) => cell,
                    None => &mut inline,
                };
                target.push(&t, Face::Mono);
            }
            Event::SoftBreak => {
                let target = match table.as_mut().and_then(|t| t.cell.as_mut()) {
                    Some(cell) => cell,
                    None => &mut inline,
                };
                target.pending_space = true;
            }
            Event::HardBreak => inline.hard_break(),
            Event::Rule => {
                flush!();
                p.ensure(14.0)?;
                p.y -= 7.0;
                p.rule(MARGIN, PAGE_W - MARGIN);
                p.y -= 7.0;
            }
            Event::TaskListMarker(done) => inline.push(if done { "[x]" } else { "[ ]" }, Face::Mono),
            Event::FootnoteReference(r) => inline.push(&format!("[{r}]"), face_now(bold, italic)),
            _ => {}
        }
    }
    flush!();
    drop(inline);
    Ok(p.finish())
}

fn draw_table(p: &mut Pages, t: TableState, x: f64) -> Result<()> {
    let cols = t.rows.iter().map(|(_, r)| r.len()).max().unwrap_or(0).clamp(1, 30);
    let total_w = PAGE_W - MARGIN - x;
    let col_w = total_w / cols as f64;
    let size = 9.5;
    let leading = size * 1.35;
    let pad = 4.0;
    p.y -= 4.0;
    for (head, row) in &t.rows {
        let cells: Vec<Vec<Line>> = row.iter().take(cols).map(|c| wrap(c, size, (col_w - 2.0 * pad).max(10.0))).collect();
        // A row taller than a page is cut to what fits.
        let max_lines = ((PAGE_H - 2.0 * MARGIN - 2.0 * pad) / leading).floor().max(1.0) as usize;
        let n = cells.iter().map(Vec::len).max().unwrap_or(1).clamp(1, max_lines);
        let h = n as f64 * leading + 2.0 * pad;
        p.ensure(h)?;
        let top = p.y;
        if *head {
            p.rect(x, top - h, total_w, h, (0.91, 0.95, 0.97));
        }
        for c in 0..cols {
            let cx = x + c as f64 * col_w;
            p.stroke_rect(cx, top - h, col_w, h);
            if let Some(lines) = cells.get(c) {
                p.y = top - pad;
                for line in lines.iter().take(n) {
                    p.y -= leading;
                    p.text_line(cx + pad, size, TEXT, line);
                }
            }
        }
        p.y = top - h;
    }
    p.y -= 10.0;
    Ok(())
}

/// The objects of the file around the laid-out pages.
fn assemble(title: &str, pages: &[String]) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    // 1 catalog, 2 pages, 3..=7 fonts, 8 info, then a page and its content per page.
    let first_page = 9;
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", first_page + 2 * i)).collect();
    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R /Lang (de) >>");
    obj(&mut out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()).as_bytes());
    for face in Face::ALL {
        let body = format!("<< /Type /Font /Subtype /Type1 /BaseFont /{} /Encoding /WinAnsiEncoding >>", face.std14().base_font());
        obj(&mut out, &mut offsets, body.as_bytes());
    }
    obj(&mut out, &mut offsets, format!("<< /Title {} /Producer (AIS.ChatCraft) >>", pdf_text_string(title)).as_bytes());
    let fonts: String = Face::ALL.iter().enumerate().map(|(i, f)| format!("/{} {} 0 R ", f.resource(), 3 + i)).collect();
    let n = pages.len();
    for (i, content) in pages.iter().enumerate() {
        let content_ref = first_page + 2 * i + 1;
        let page = format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << {fonts}>> >> /Contents {content_ref} 0 R >>");
        obj(&mut out, &mut offsets, page.as_bytes());
        let mut stream = content.clone();
        if n > 1 {
            let label = format!("Seite {} von {n}", i + 1);
            let w = width(Face::Regular, &label, 8.5);
            let _ = writeln!(
                stream,
                "BT {:.3} {:.3} {:.3} rg /F1 8.5 Tf {:.2} {:.2} Td {} Tj ET",
                MUTED.0,
                MUTED.1,
                MUTED.2,
                (PAGE_W - w) / 2.0,
                MARGIN / 2.0,
                pdf_string(&label)
            );
        }
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream.as_bytes());
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &body);
    }
    let xref = out.len();
    let mut tail = format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        let _ = writeln!(tail, "{o:010} 00000 n ");
    }
    let _ = write!(tail, "trailer\n<< /Size {} /Root 1 0 R /Info 8 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1);
    out.extend_from_slice(tail.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(pdf: &[u8]) -> String {
        String::from_utf8_lossy(pdf).into_owned()
    }

    #[test]
    fn a_document_becomes_a_pdf() {
        let md = "# Stundenplan 7b\n\nMontag **Mathe** und *Deutsch* mit `Code`.\n\n- Eins\n- Zwei\n  1. Unterpunkt\n\n| Fach | Stunden |\n|---|---|\n| Bio | 2 |\n\n```\nfn main() {}\n```\n\n> Zitat\n\n---\nÄpfel → Birnen ✓ 😀";
        let pdf = markdown_pdf("Plan (7b) ü", md).unwrap();
        let s = text_of(&pdf);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.trim_end().ends_with("%%EOF"));
        assert!(s.contains("/Count 1"));
        assert!(s.contains("(Stundenplan 7b) Tj"));
        assert!(s.contains("/F2 11.0 Tf ( Mathe) Tj") || s.contains("(Mathe) Tj"), "bold run");
        assert!(s.contains("(Bio) Tj") && s.contains("(Stunden) Tj"));
        assert!(s.contains("(fn main\\(\\) {}) Tj"), "parentheses escaped");
        // Ä is 0xC4 in WinAnsi; → becomes ->; the emoji a ?.
        assert!(s.contains("(\\304pfel -> Birnen x ?) Tj"), "{s}");
        assert!(s.contains("/BaseFont /Helvetica-Bold"));
        // The xref offsets point at the objects.
        let xref = s.rfind("startxref\n").map(|i| &s[i + 10..]).and_then(|t| t.lines().next()).and_then(|l| l.parse::<usize>().ok()).unwrap();
        // Byte offsets: on the raw bytes (the binary comment line is not UTF-8).
        assert!(pdf[xref..].starts_with(b"xref"));
        let tail = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        let first = tail.lines().nth(3).unwrap();
        let off: usize = first[..10].parse().unwrap();
        assert!(pdf[off..].starts_with(b"1 0 obj"));
    }

    #[test]
    fn long_documents_get_pages_and_a_cap() {
        let para = "Ein langer Satz mit vielen Wörtern, der umbrochen werden muss. ".repeat(20);
        let md: String = (0..40).map(|i| format!("## Abschnitt {i}\n\n{para}\n\n")).collect();
        let pdf = text_of(&markdown_pdf("Lang", &md).unwrap());
        let count: usize = pdf.split("/Count ").nth(1).and_then(|t| t.split(' ').next()).and_then(|n| n.parse().ok()).unwrap();
        assert!(count > 5, "{count} pages");
        assert_eq!(pdf.matches("/Type /Page ").count(), count);
        assert!(pdf.contains(&format!("(Seite 2 von {count}) Tj")));
        let huge = "Wort ".repeat(2_000_000);
        assert!(markdown_pdf("x", &huge).is_err());
        // Empty input is still a valid one-page file.
        assert!(text_of(&markdown_pdf("", "").unwrap()).contains("/Count 1"));
        // A word wider than the page is cut, not run off the edge.
        let url = "https://example.org/".to_string() + &"a".repeat(400);
        assert!(text_of(&markdown_pdf("u", &url).unwrap()).contains("/Count 1"));
    }

    #[test]
    fn wrapping_keeps_words_and_faces() {
        let mut i = Inline::default();
        i.push("Hallo ", Face::Regular);
        i.push("fette", Face::Bold);
        i.push(" Welt", Face::Regular);
        let lines = wrap(&i, 11.0, 1000.0);
        assert_eq!(lines, vec![vec![(Face::Regular, "Hallo".to_string()), (Face::Bold, " fette".to_string()), (Face::Regular, " Welt".to_string())]]);
        let narrow = wrap(&i, 11.0, 40.0);
        assert_eq!(narrow.len(), 3);
    }
}
