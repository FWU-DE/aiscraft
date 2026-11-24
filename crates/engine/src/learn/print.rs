//! Worksheets on paper: the sheet (answer lines and boxes, gaps as lines) and the solution
//! sheet, as Markdown that the PDF writer (`office/pdf.rs`) and craft-docx turn into files.
//! Only WinAnsi characters, so the PDF prints them: `( )` and `[ ]` instead of ballot boxes.

use aiscraft_core::Result;
use aiscraft_core::worksheet::{ClozePart, TaskBody, Worksheet, cloze_parts, display_order};

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

fn points(p: f64) -> String {
    let s = if (p - p.round()).abs() < 1e-9 { format!("{}", p.round() as i64) } else { format!("{p:.1}").replace('.', ",") };
    if s == "1" { "1 Punkt".into() } else { format!("{s} Punkte") }
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
        o.push_str(&format!("**{n}. {}** ({})\n\n", md(&prompt), points(t.points)));
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
                    let text = if sol && opt.correct { format!("**{}**", md(&opt.text)) } else { md(&opt.text) };
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
            TaskBody::Cloze { text } => {
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
        if !t.hint.trim().is_empty() && !sol {
            o.push_str(&format!("*Tipp: {}*\n\n", md(&t.hint)));
        }
        if sol && !t.feedback.trim().is_empty() {
            o.push_str(&format!("*Erklärung: {}*\n\n", md(&t.feedback)));
        }
    }
    if n > 0 {
        o.push_str(&format!("---\n\nGesamt: {} {}\n", if sol { "" } else { "      von" }, points(total)));
    }
    o
}

pub fn worksheet_pdf(ws: &Worksheet, v: Variant) -> Result<Vec<u8>> {
    crate::office::pdf::markdown_pdf(&format!("{} ({})", ws.title, v.label()), &worksheet_markdown(ws, v))
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
