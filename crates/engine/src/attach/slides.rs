//! Presentations (.ppt, .pptx) as Markdown: a heading per slide, its text, tables and notes.

use aiscraft_core::{Error, Result};
use craft_pptx::{PhType, Plain, Presentation, Shape, ShapeKind};

const MAX_SLIDES: usize = 2_000;
const MAX_CHARS: usize = 2_000_000;

pub fn read(ext: &str, bytes: &[u8]) -> Result<Presentation> {
    match ext {
        "ppt" | "pps" | "pot" => craft_ppt_binary::import(bytes, &Plain).map_err(|e| match e {
            craft_ppt_binary::PptError::Encrypted => {
                Error::unsupported("Die Präsentation ist mit einem Passwort geschützt. Speichere sie ohne Passwort und hänge sie dann an.")
            }
            other => Error::invalid(format!("Die PowerPoint-Datei lässt sich nicht lesen: {other}")),
        }),
        "pptx" | "ppsx" | "potx" => craft_pptx::import(bytes, &Plain).map_err(|e| Error::invalid(format!("Die PowerPoint-Datei lässt sich nicht lesen: {e}"))),
        _ => Err(Error::unsupported(format!(".{ext} ist keine Präsentation, die AIS.ChatCraft lesen kann"))),
    }
}

pub fn count(p: &Presentation) -> usize {
    p.slides.len().min(MAX_SLIDES)
}

fn is_title(s: &Shape) -> bool {
    s.ph.as_ref().is_some_and(|ph| matches!(ph.kind, PhType::Title | PhType::CtrTitle))
}

fn shape_text(s: &Shape, out: &mut Vec<String>, depth: usize) {
    if depth > 32 || s.hidden {
        return;
    }
    match &s.kind {
        ShapeKind::Group { children, .. } => {
            for c in children {
                shape_text(c, out, depth + 1);
            }
        }
        ShapeKind::Table(t) => {
            let mut rows = Vec::new();
            for (i, row) in t.rows.iter().enumerate() {
                let cells: Vec<String> = row.cells.iter().map(|c| c.text.text().replace('|', "\\|").replace('\n', " ")).collect();
                rows.push(format!("| {} |", cells.join(" | ")));
                if i == 0 {
                    rows.push(format!("|{}", " --- |".repeat(cells.len().max(1))));
                }
            }
            out.push(rows.join("\n"));
        }
        _ => {
            if let Some(body) = &s.text {
                let lines: Vec<String> = body
                    .paragraphs
                    .iter()
                    .map(|p| (p.level, p.text()))
                    .filter(|(_, t)| !t.trim().is_empty())
                    .map(|(level, t)| if is_title(s) { t } else { format!("{}- {}", "  ".repeat(usize::from(level.min(8))), t.trim()) })
                    .collect();
                if !lines.is_empty() {
                    out.push(lines.join("\n"));
                }
            }
        }
    }
}

/// The slides `numbers` (1-based; all when `None`) as Markdown.
pub fn markdown(title: &str, p: &Presentation, numbers: Option<&[usize]>) -> String {
    let mut out = format!("# {title}\n");
    for (i, slide) in p.slides.iter().enumerate().take(MAX_SLIDES) {
        let n = i + 1;
        if numbers.is_some_and(|ns| !ns.contains(&n)) {
            continue;
        }
        let heading =
            slide.shapes.iter().filter(|s| is_title(s)).filter_map(|s| s.text.as_ref()).map(|t| t.text().trim().replace('\n', " ")).find(|t| !t.is_empty());
        out.push_str(&format!("\n## Folie {n}{}\n\n", heading.map(|h| format!(": {h}")).unwrap_or_default()));
        let mut parts = Vec::new();
        for s in slide.shapes.iter().filter(|s| !is_title(s)) {
            shape_text(s, &mut parts, 0);
        }
        if parts.is_empty() {
            out.push_str("(ohne Text)\n");
        } else {
            out.push_str(&parts.join("\n\n"));
            out.push('\n');
        }
        let notes = slide.notes.text();
        if !notes.trim().is_empty() {
            out.push_str(&format!("\nNotizen: {}\n", notes.trim()));
        }
        if out.len() > MAX_CHARS {
            out.push_str("\n(gekürzt)\n");
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pptx_slides_to_markdown() {
        let bytes = crate::office::pptx::slides_pptx(&serde_json::json!({"title": "Zellen", "slides": [
            {"title": "Aufbau", "bullets": ["Zellkern", "Zellmembran"]},
            {"title": "Teilung", "bullets": ["Mitose"]},
            {"title": "Quiz", "bullets": ["Frage 1"]}
        ]}))
        .unwrap();
        let p = read("pptx", &bytes).unwrap();
        assert!(count(&p) >= 3);
        let all = markdown("Zellen", &p, None);
        assert!(all.contains("Zellkern") && all.contains("Mitose"), "{all}");
        let some = markdown("Zellen", &p, Some(&[count(&p)]));
        assert!(some.contains("Frage 1") && !some.contains("Zellkern"), "{some}");
        assert!(read("pptx", b"kaputt").is_err());
        assert!(read("ppt", b"kaputt").is_err());
    }
}
