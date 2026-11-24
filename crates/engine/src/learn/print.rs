//! Worksheets on paper: the sheet (answer lines and boxes, gaps as lines) and the solution
//! sheet, as Markdown that the PDF writer (`office/pdf.rs`) and craft-docx turn into files.
//! Only WinAnsi characters, so the PDF prints them: `( )` and `[ ]` instead of ballot boxes.

use aiscraft_core::Result;
use aiscraft_core::worksheet::{ClozePart, TaskBody, Worksheet, category_items, cloze_parts, cloze_word_bank, display_order, points_text as points};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// For pupils: empty lines and boxes.
    Sheet,
    /// For the teacher: the solutions filled in.
    Solution,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Variant::Sheet => "Arbeitsblatt",
            Variant::Solution => "Lösungsblatt",
        }
    }
}

/// An answer line of `n` escaped underscores (a bare `___` line would be a Markdown rule).
fn line(n: usize) -> String {
    "\\_".repeat(n)
}

/// Text as Markdown that cannot start a heading, list or block by accident.
fn md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(ch, '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.replace('\n', "  \n")
}

fn letter(i: usize) -> char {
    char::from(b'A' + (i % 26) as u8)
}

/// The worksheet as Markdown for printing.
pub fn worksheet_markdown(ws: &Worksheet, v: Variant) -> String {
    let sol = v == Variant::Solution;
    let mut o = String::new();
    let title = if sol { format!("{} – Lösungen", ws.title) } else { ws.title.clone() };
    o.push_str(&format!("# {}\n\n", md(&title)));
    let meta: Vec<String> =
        [("Fach", &ws.subject), ("Klasse", &ws.grade)].iter().filter(|(_, x)| !x.trim().is_empty()).map(|(k, x)| format!("{k}: {}", md(x))).collect();
    if !meta.is_empty() {
        o.push_str(&format!("{}\n\n", meta.join(" · ")));
    }
    if !sol {
        o.push_str(&format!("Name: {}   Datum: {}\n\n", line(24), line(12)));
    }
    if !ws.instructions.trim().is_empty() {
        o.push_str(&format!("*{}*\n\n", md(ws.instructions.trim())));
    }
    let mut n = 0;
    let total: f64 = ws.tasks.iter().filter(|t| !matches!(t.body, TaskBody::Info { .. })).map(|t| t.points).sum();
    for t in &ws.tasks {
        if let TaskBody::Info { text } = &t.body {
            if !t.prompt.trim().is_empty() {
                o.push_str(&format!("### {}\n\n", md(&t.prompt)));
            }
            o.push_str(&format!("> {}\n\n", md(text).replace("  \n", "  \n> ")));
            continue;
        }
        n += 1;
        let prompt = if t.prompt.trim().is_empty() { t.body.label().to_string() } else { t.prompt.trim().to_string() };
        if ws.points {
            o.push_str(&format!("**{n}. {}** ({})\n\n", md(&prompt), points(t.points)));
        } else {
            o.push_str(&format!("**{n}. {}**\n\n", md(&prompt)));
        }
        if let Some(img) = &t.image {
            o.push_str(&format!("(Bild: {})\n\n", md(img)));
        }
        match &t.body {
            TaskBody::Info { .. } => {}
            TaskBody::Choice { options, multiple } => {
                for opt in options {
                    let mark = match (multiple, sol && opt.correct) {
                        (true, true) => "[x]",
                        (true, false) => "[ ]",
                        (false, true) => "(x)",
                        (false, false) => "( )",
                    };
                    let mut text = if sol && opt.correct { format!("**{}**", md(&opt.text)) } else { md(&opt.text) };
                    if sol && !opt.feedback.trim().is_empty() {
                        text.push_str(&format!(" – *{}*", md(opt.feedback.trim())));
                    }
                    o.push_str(&format!("{} {text}  \n", mark.replace('[', "\\[").replace(']', "\\]")));
                }
                o.push('\n');
            }
            TaskBody::TrueFalse { answer } => {
                let (r, f) = match (sol, answer) {
                    (true, true) => ("(x) **richtig**", "( ) falsch"),
                    (true, false) => ("( ) richtig", "(x) **falsch**"),
                    _ => ("( ) richtig", "( ) falsch"),
                };
                o.push_str(&format!("{r}     {f}\n\n"));
            }
            TaskBody::Cloze { text, word_bank, distractors } => {
                if *word_bank && !sol {
                    let words: Vec<String> = cloze_word_bank(text, distractors, &t.id).iter().map(|w| md(w)).collect();
                    o.push_str(&format!("Wörter: {}\n\n", words.join("  ·  ")));
                }
                let mut s = String::new();
                for part in cloze_parts(text) {
                    match part {
                        ClozePart::Text(x) => s.push_str(&md(x)),
                        ClozePart::Gap(answers) => {
                            if sol {
                                s.push_str(&format!("**{}**", md(&answers.join(" / "))));
                            } else {
                                s.push_str(&line(answers.first().map(|a| a.chars().count()).unwrap_or(8).clamp(8, 20)));
                            }
                        }
                    }
                }
                o.push_str(&format!("{s}\n\n"));
            }
            TaskBody::Matching { pairs } => {
                let order = display_order(pairs.len(), &t.id);
                // The right column shuffled and lettered; pupils write the letter.
                o.push_str("| | Zuordnung | |\n|---|---|---|\n");
                for (row, (i, p)) in pairs.iter().enumerate().enumerate() {
                    let right = order.get(row).and_then(|r| pairs.get(*r)).map(|x| md(&x.right)).unwrap_or_default();
                    let answer = if sol { order.iter().position(|r| *r == i).map(|pos| format!("**{}**", letter(pos))).unwrap_or_default() } else { line(4) };
                    o.push_str(&format!("| {} | {answer} | {}) {right} |\n", md(&p.left), letter(row)));
                }
                o.push('\n');
            }
            TaskBody::Ordering { items } => {
                if sol {
                    for (i, it) in items.iter().enumerate() {
                        o.push_str(&format!("{}. {}  \n", i + 1, md(it)));
                    }
                } else {
                    for i in display_order(items.len(), &t.id) {
                        if let Some(it) = items.get(i) {
                            o.push_str(&format!("{} {}  \n", line(3), md(it)));
                        }
                    }
                }
                o.push('\n');
            }
            TaskBody::Categorize { categories } => {
                let items = category_items(categories);
                if !sol {
                    let words: Vec<String> = display_order(items.len(), &t.id).into_iter().filter_map(|i| items.get(i)).map(|(_, w)| md(w)).collect();
                    o.push_str(&format!("Wörter: {}\n\n", words.join("  ·  ")));
                }
                // One column per group; on the sheet as many empty rows as the largest group.
                let head: Vec<String> = categories.iter().map(|c| md(&c.name)).collect();
                o.push_str(&format!("| {} |\n|{}\n", head.join(" | "), "---|".repeat(head.len())));
                let columns: Vec<Vec<&str>> = (0..categories.len()).map(|c| items.iter().filter(|(k, _)| *k == c).map(|(_, w)| *w).collect()).collect();
                let rows = columns.iter().map(Vec::len).max().unwrap_or(0).max(1);
                for r in 0..rows {
                    let cells: Vec<String> = columns.iter().map(|col| if sol { col.get(r).map(|x| md(x)).unwrap_or_default() } else { line(10) }).collect();
                    o.push_str(&format!("| {} |\n", cells.join(" | ")));
                }
                o.push('\n');
            }
            TaskBody::ShortAnswer { accepted, .. } => {
                if sol {
                    o.push_str(&format!("Antwort: **{}**\n\n", md(&accepted.join(" / "))));
                } else {
                    o.push_str(&format!("Antwort: {}\n\n", line(40)));
                }
            }
            TaskBody::Numeric { answer, tolerance, unit } => {
                if sol {
                    let num = format!("{answer}").replace('.', ",");
                    let tol = if *tolerance > 0.0 { format!(" (±{})", format!("{tolerance}").replace('.', ",")) } else { String::new() };
                    o.push_str(&format!("Ergebnis: **{num}** {}{tol}\n\n", md(unit)));
                } else {
                    o.push_str(&format!("Ergebnis: {} {}\n\n", line(16), md(unit)));
                }
            }
            TaskBody::FreeText { sample, rubric, lines } => {
                if sol {
                    if !sample.trim().is_empty() {
                        o.push_str(&format!("Musterlösung: {}\n\n", md(sample)));
                    }
                    if !rubric.trim().is_empty() {
                        o.push_str(&format!("Bewertung: {}\n\n", md(rubric)));
                    }
                } else {
                    for _ in 0..(*lines).clamp(1, 30) {
                        o.push_str(&format!("{}\n\n", line(60)));
                    }
                }
            }
        }
        // On paper nothing can be uncovered: the sheet prints the first hint only, the solutions all.
        let hints: Vec<&String> = t.hints.iter().filter(|h| !h.trim().is_empty()).collect();
        if !sol && let Some(h) = hints.first() {
            o.push_str(&format!("*Tipp: {}*\n\n", md(h)));
        }
        if sol && !hints.is_empty() {
            let list: Vec<String> = hints.iter().enumerate().map(|(i, h)| format!("{}. {}", i + 1, md(h))).collect();
            o.push_str(&format!("*Tipps: {}*\n\n", list.join("  ")));
        }
        if sol && !t.feedback.trim().is_empty() {
            o.push_str(&format!("*Erklärung: {}*\n\n", md(&t.feedback)));
        }
    }
    if n > 0 && ws.points {
        o.push_str(&format!("---\n\nGesamt: {} {}\n", if sol { "" } else { "      von" }, points(total)));
    }
    o
}

pub fn worksheet_pdf(ws: &Worksheet, v: Variant) -> Result<Vec<u8>> {
    crate::office::pdf::markdown_pdf(&format!("{} ({})", ws.title, v.label()), &worksheet_markdown(ws, v))
}

/// One page of the print preview: `width` × `height` RGBA8 pixels, opaque.
#[derive(Debug, Clone)]
pub struct PreviewPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Pages the print preview shows at most.
pub const MAX_PREVIEW_PAGES: usize = 30;

/// The sheet's pages as pictures `width` pixels wide, rendered from the very PDF that prints, so
/// the preview shows exactly what comes out of the printer.
pub fn worksheet_preview(ws: &Worksheet, v: Variant, width: u32) -> Result<Vec<PreviewPage>> {
    use aiscraft_core::Error;
    use craft_pdf::{interpret::InterpreterSettings, raster};
    let pdf = worksheet_pdf(ws, v)?;
    let doc = craft_pdf::open_slice(&pdf, None).map_err(|e| Error::invalid(format!("PDF für die Vorschau: {e}")))?;
    let cache = raster::RenderCache::new();
    let settings = InterpreterSettings::default();
    let mut out = Vec::new();
    for page in doc.pages().iter().take(MAX_PREVIEW_PAGES) {
        let (w, _) = page.render_dimensions();
        if !w.is_finite() || w <= 0.0 {
            return Err(Error::invalid("PDF-Seite ohne Größe"));
        }
        let scale = width.clamp(100, 3000) as f32 / w;
        // White, opaque paper: premultiplied pixels are then the same as straight ones.
        let options = raster::Options { scale: [scale, scale], background: [255, 255, 255, 255], ..Default::default() };
        let r = raster::render(page, &cache, &settings, &options).map_err(|e| Error::invalid(format!("Vorschau: {e}")))?;
        out.push(PreviewPage { width: r.width, height: r.height, rgba: r.bytes().to_vec() });
    }
    Ok(out)
}

pub fn worksheet_docx(ws: &Worksheet, v: Variant) -> Result<Vec<u8>> {
    crate::office::markdown_docx(&format!("{} ({})", ws.title, v.label()), &worksheet_markdown(ws, v))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use aiscraft_core::worksheet::from_value;
    use serde_json::json;

    fn sheet(extra: usize) -> Worksheet {
        let mut tasks = vec![
            json!({"type": "choice", "prompt": "Welcher Bruch ist groesser?", "options": [{"text": "Einhalb", "correct": true}, {"text": "Eindrittel"}]}),
            json!({"type": "cloze", "prompt": "Ergaenze", "text": "Ein Halb ist [[Zweiviertel]]."}),
            json!({"type": "matching", "prompt": "Ordne zu", "pairs": [["Haelfte", "Einhalb"], ["Viertel", "Einviertel"], ["Drittel", "Eindrittel"]]}),
            json!({"type": "ordering", "prompt": "Sortiere", "items": ["klein", "mittel", "gross"]}),
            json!({"type": "numeric", "prompt": "Dezimal", "answer": 0.75, "unit": "m"}),
            json!({"type": "free_text", "prompt": "Erklaere", "sample": "Musterantwort", "lines": 3}),
        ];
        for i in 0..extra {
            tasks.push(json!({"type": "free_text", "prompt": format!("Zusatzaufgabe {i}"), "lines": 8}));
        }
        from_value(&json!({"title": "Brueche", "subject": "Mathe", "grade": "6", "tasks": tasks})).unwrap().0
    }

    #[test]
    fn sheet_and_solution_differ() {
        let ws = sheet(0);
        let a = worksheet_markdown(&ws, Variant::Sheet);
        let l = worksheet_markdown(&ws, Variant::Solution);
        assert!(a.contains("( ) Einhalb") && !a.contains("Zweiviertel") && !a.contains("Musterantwort"));
        assert!(l.contains("(x) **Einhalb**") && l.contains("**Zweiviertel**") && l.contains("Musterantwort"));
        assert!(a.contains("Name:"));
        let pdf = String::from_utf8_lossy(&worksheet_pdf(&ws, Variant::Sheet).unwrap()).into_owned();
        assert!(pdf.contains("Welcher Bruch ist groesser?") || pdf.contains("groesser"), "task text in the PDF");
        assert!(pdf.contains("/Count 1"));
        let docx = worksheet_docx(&ws, Variant::Solution).unwrap();
        assert!(docx.starts_with(b"PK"));
    }

    #[test]
    fn preview_renders_every_printed_page() {
        let ws = sheet(12);
        let pdf = String::from_utf8_lossy(&worksheet_pdf(&ws, Variant::Sheet).unwrap()).into_owned();
        let count: usize = pdf.split("/Count ").nth(1).and_then(|t| t.split(' ').next()).and_then(|n| n.parse().ok()).unwrap();
        let pages = worksheet_preview(&ws, Variant::Sheet, 600).unwrap();
        assert_eq!(pages.len(), count, "one picture per printed page");
        let ink = |p: &PreviewPage| p.rgba.as_chunks::<4>().0.iter().filter(|px| px[0] < 128).count();
        for p in &pages {
            assert_eq!(p.width, 600);
            assert!(p.height > p.width, "portrait paper");
            assert_eq!(p.rgba.len(), (p.width * p.height * 4) as usize);
            assert!(p.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 255), "opaque paper");
        }
        assert!(ink(&pages[0]) > 500, "text drawn on the first page");
        // The solution sheet looks different from the worksheet.
        let sol = worksheet_preview(&ws, Variant::Solution, 600).unwrap();
        assert!(sol.first().map(|p| &p.rgba) != pages.first().map(|p| &p.rgba));
        // Absurd widths are clamped, never a huge allocation.
        assert_eq!(worksheet_preview(&ws, Variant::Sheet, u32::MAX).unwrap()[0].width, 3000);
    }

    #[test]
    fn new_task_parts_print_on_sheet_and_solution() {
        let ws = from_value(&json!({"title": "Tiere", "tasks": [
            {"type": "categorize", "prompt": "Sortiere", "categories": [{"name": "Saeugetiere", "items": ["Wal", "Fledermaus", "Igel"]}, {"name": "Fische", "items": ["Hai"]}]},
            {"type": "cloze", "prompt": "Ergaenze", "text": "Der Wal ist ein [[Saeugetier]].", "word_bank": true, "distractors": ["Fisch"]},
            {"type": "choice", "prompt": "Welches?", "hints": ["Erster Tipp", "Zweiter Tipp"],
             "options": [{"text": "Wal", "correct": true}, {"text": "Hai", "feedback": "Der Hai atmet mit Kiemen"}]}
        ]})).unwrap().0;
        let a = worksheet_markdown(&ws, Variant::Sheet);
        let l = worksheet_markdown(&ws, Variant::Solution);
        // Categorize: word box and one column per group, filled only on the solution sheet.
        assert!(a.contains("| Saeugetiere | Fische |") && l.contains("| Saeugetiere | Fische |"));
        assert!(a.contains("Wörter:") && a.contains("Fledermaus") && a.contains("Hai"));
        assert!(l.contains("| Igel |  |") && !l.contains("Wörter:"));
        assert_eq!(a.lines().filter(|r| r.starts_with("| \\_")).count(), 3, "three empty rows for the largest group");
        // Word bank with the distractor on the sheet only.
        assert!(a.contains("Fisch") && a.contains("Saeugetier"));
        // Hints: the first on the sheet, all on the solution sheet; option feedback only there.
        assert!(a.contains("Tipp: Erster Tipp") && !a.contains("Zweiter Tipp"));
        assert!(l.contains("1. Erster Tipp") && l.contains("2. Zweiter Tipp"));
        assert!(l.contains("Kiemen") && !a.contains("Kiemen"));
        for v in [Variant::Sheet, Variant::Solution] {
            let pdf = String::from_utf8_lossy(&worksheet_pdf(&ws, v).unwrap()).into_owned();
            assert!(pdf.contains("Saeugetiere"), "group names in the PDF");
            assert!(worksheet_docx(&ws, v).unwrap().starts_with(b"PK"));
        }
    }

    #[test]
    fn sheets_without_points_print_no_points() {
        let mut ws = sheet(0);
        for v in [Variant::Sheet, Variant::Solution] {
            let md = worksheet_markdown(&ws, v);
            assert!(md.contains("(1 Punkt)") && md.contains("Gesamt:"), "with points: {md}");
        }
        ws.points = false;
        for v in [Variant::Sheet, Variant::Solution] {
            let md = worksheet_markdown(&ws, v);
            assert!(!md.contains("Punkt") && !md.contains("Gesamt"), "{v:?}: {md}");
            assert!(md.contains("**1. Welcher Bruch ist groesser?**"), "the task line stays");
            // The PDF (which the print preview renders) and Word say no points either.
            let pdf = String::from_utf8_lossy(&worksheet_pdf(&ws, v).unwrap()).into_owned();
            assert!(pdf.contains("groesser") && !pdf.contains("Punkt") && !pdf.contains("Gesamt"));
            let docx = worksheet_docx(&ws, v).unwrap();
            assert!(docx.starts_with(b"PK"));
            let mut zip = craft_zip::Archive::open(&docx, craft_zip::Limits { max_entries: 1000, max_total_bytes: 50 << 20 }).unwrap();
            let xml = String::from_utf8(zip.read("word/document.xml", 8 << 20).unwrap().unwrap()).unwrap();
            assert!(xml.contains("groesser") && !xml.contains("Punkt"), "{v:?}");
        }
        // The preview pages differ from the sheet with points (the lines are gone).
        let mut with = ws.clone();
        with.points = true;
        let a = worksheet_preview(&ws, Variant::Sheet, 600).unwrap();
        let b = worksheet_preview(&with, Variant::Sheet, 600).unwrap();
        assert!(a.first().map(|p| &p.rgba) != b.first().map(|p| &p.rgba));
    }

    #[test]
    fn long_sheets_get_more_pages() {
        let pdf = String::from_utf8_lossy(&worksheet_pdf(&sheet(12), Variant::Sheet).unwrap()).into_owned();
        let count: usize = pdf.split("/Count ").nth(1).and_then(|t| t.split(' ').next()).and_then(|n| n.parse().ok()).unwrap();
        assert!(count >= 3, "{count} pages");
        assert_eq!(pdf.matches("/Type /Page ").count(), count);
        // The solution sheet has no answer lines, so it is shorter.
        let sol = String::from_utf8_lossy(&worksheet_pdf(&sheet(12), Variant::Solution).unwrap()).into_owned();
        let sol_count: usize = sol.split("/Count ").nth(1).and_then(|t| t.split(' ').next()).and_then(|n| n.parse().ok()).unwrap();
        assert!(sol_count < count);
    }
}
