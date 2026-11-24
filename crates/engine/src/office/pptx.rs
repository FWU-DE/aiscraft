//! Slides (.pptx) from a model's JSON, written by craft-pptx.
//!
//! `{"title", "subtitle", "slides": [{"title", "bullets": [..], "table": {"columns", "rows"},
//! "notes"}]}`. With a title, the deck opens with a title slide. Bullets indented by two spaces
//! per level are sub-points. 16:9, text boxes on a blank layout, PowerPoint shrinks text that
//! does not fit.

use std::sync::Arc;

use aiscraft_core::{Error, Result};
use craft_pptx::text::{Align, AutoFit, BodyProps, Bullet, ParaProps};
use craft_pptx::{ColorRef, Defaults, Paragraph, Plain, Presentation, Rgba, Run, RunProps, Shape, ShapeId, ShapeKind, Slide, SlideId, Table, TextBody, Xfrm};
use serde_json::Value;

pub const MAX_SLIDES: usize = 60;
pub const MAX_BULLETS: usize = 20;
pub const MAX_BULLET_CHARS: usize = 400;
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_TABLE_ROWS: usize = 25;
pub const MAX_TABLE_COLS: usize = 10;
pub const MAX_CELL_CHARS: usize = 200;
pub const MAX_NOTES_CHARS: usize = 4_000;

const ACCENT: Rgba = Rgba::rgb(0x10, 0x7f, 0xad);
const TEXT: Rgba = Rgba::rgb(0x1e, 0x29, 0x3b);

fn cut(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(n).collect()
}

fn run(text: &str, size: f64, bold: bool, color: Rgba) -> Run {
    Run {
        text: text.to_string(),
        props: RunProps {
            size: Some(size),
            bold: bold.then_some(true),
            fill: Some(craft_pptx::Fill::Solid { color: ColorRef::rgb(color) }),
            ..RunProps::default()
        },
        ..Run::new("")
    }
}

/// `rect`: x, y, width, height in points.
fn text_box(id: u32, name: &str, [x, y, w, h]: [f64; 4], paragraphs: Vec<Paragraph>, anchor_middle: bool) -> Shape {
    Shape {
        id: ShapeId(id),
        name: name.to_string(),
        xfrm: Some(Xfrm { x, y, w, h, ..Xfrm::default() }),
        text_box: true,
        text: Some(TextBody {
            body: BodyProps {
                wrap: Some(true),
                autofit: Some(AutoFit::Shrink { font_scale: None, line_reduction: 0.0 }),
                anchor: anchor_middle.then_some(craft_pptx::text::Anchor::Middle),
                ..BodyProps::default()
            },
            paragraphs,
            ..TextBody::default()
        }),
        ..Shape::default()
    }
}

fn para(text: &str, size: f64, bold: bool, color: Rgba, align: Option<Align>) -> Paragraph {
    Paragraph { runs: vec![run(text, size, bold, color)], props: ParaProps { align, ..ParaProps::default() }, ..Paragraph::default() }
}

fn bullet(text: &str, level: u8) -> Paragraph {
    let size = if level == 0 { 22.0 } else { 18.0 };
    let indent = 22.0 + 24.0 * f64::from(level);
    Paragraph {
        level,
        runs: vec![run(text, size, false, TEXT)],
        props: ParaProps {
            bullet: Some(Bullet::Char { char: if level == 0 { "\u{2022}".into() } else { "\u{2013}".into() } }),
            margin_left: Some(indent),
            indent: Some(-18.0),
            space_before: Some(craft_pptx::text::Spacing::Pts(6.0)),
            ..ParaProps::default()
        },
        ..Paragraph::default()
    }
}

fn texts(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| match x {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn table_of(v: &Value) -> Result<Option<Table>> {
    let header = texts(v.get("columns"));
    let rows: Vec<Vec<String>> = v
        .get("rows")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .map(|row| if row.is_array() { texts(Some(row)) } else { vec![row.as_str().map(str::to_string).unwrap_or_else(|| row.to_string())] })
                .collect()
        })
        .unwrap_or_default();
    let all: Vec<Vec<String>> = (!header.is_empty()).then_some(header.clone()).into_iter().chain(rows).collect();
    if all.is_empty() {
        return Ok(None);
    }
    if all.len() > MAX_TABLE_ROWS {
        return Err(Error::invalid(format!("Eine Folientabelle hat höchstens {MAX_TABLE_ROWS} Zeilen")));
    }
    let cols = all.iter().map(Vec::len).max().unwrap_or(1).max(1);
    if cols > MAX_TABLE_COLS {
        return Err(Error::invalid(format!("Eine Folientabelle hat höchstens {MAX_TABLE_COLS} Spalten")));
    }
    let mut t = Table::new(all.len(), cols, 864.0, 30.0);
    t.first_row = !header.is_empty();
    for (r, row) in all.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            if let Some(cell) = t.rows.get_mut(r).and_then(|row| row.cells.get_mut(c)) {
                let mut body = TextBody::from_text(&cut(text, MAX_CELL_CHARS).replace('\n', " "));
                for p in body.paragraphs.iter_mut() {
                    for run in p.runs.iter_mut() {
                        run.props.size = Some(14.0);
                    }
                }
                cell.text = body;
            }
        }
    }
    Ok(Some(t))
}

/// The deck described by `spec`, as the bytes of a .pptx file.
pub fn slides_pptx(spec: &Value) -> Result<Vec<u8>> {
    let deck_title = cut(spec.get("title").and_then(Value::as_str).unwrap_or("").trim(), MAX_TITLE_CHARS);
    let subtitle = cut(spec.get("subtitle").and_then(Value::as_str).unwrap_or("").trim(), MAX_TITLE_CHARS * 2);
    let empty = Vec::new();
    let slides = spec.get("slides").and_then(Value::as_array).unwrap_or(&empty);
    if slides.is_empty() && deck_title.is_empty() {
        return Err(Error::invalid("slides fehlt (eine Liste von Folien mit title und bullets)"));
    }
    let title_slide = usize::from(!deck_title.is_empty());
    if slides.len() + title_slide > MAX_SLIDES {
        return Err(Error::invalid(format!("Höchstens {MAX_SLIDES} Folien")));
    }
    let mut p = Presentation::default();
    p.props.title = deck_title.clone();
    let (master, last) = Plain.master(p.slide_size, 0);
    let layout = master.layouts.first().map(|l| l.id).ok_or_else(|| Error::invalid("Folienlayout fehlt"))?;
    p.masters.push(Arc::new(master));
    p.next_id = last;
    let id = |p: &mut Presentation| p.alloc_id().ok_or_else(|| Error::invalid("Zu viele Folienelemente"));

    if !deck_title.is_empty() {
        let mut slide = Slide { id: SlideId(id(&mut p)?), layout, ..Slide::default() };
        slide.shapes.push(text_box(id(&mut p)?, "Titel", [80.0, 150.0, 800.0, 150.0], vec![para(&deck_title, 40.0, true, ACCENT, Some(Align::Center))], true));
        if !subtitle.is_empty() {
            slide.shapes.push(text_box(
                id(&mut p)?,
                "Untertitel",
                [80.0, 310.0, 800.0, 90.0],
                vec![para(&subtitle, 22.0, false, TEXT, Some(Align::Center))],
                false,
            ));
        }
        p.slides.push(Arc::new(slide));
    }
    for (i, s) in slides.iter().enumerate() {
        let title = cut(s.get("title").and_then(Value::as_str).unwrap_or("").trim(), MAX_TITLE_CHARS);
        let bullets = texts(s.get("bullets"));
        if bullets.len() > MAX_BULLETS {
            return Err(Error::invalid(format!("Folie {}: höchstens {MAX_BULLETS} Punkte", i + 1)));
        }
        let table = match s.get("table") {
            Some(t) if t.is_object() => table_of(t)?,
            _ => None,
        };
        let mut slide = Slide { id: SlideId(id(&mut p)?), layout, ..Slide::default() };
        if !title.is_empty() {
            slide.shapes.push(text_box(id(&mut p)?, "Titel", [48.0, 28.0, 864.0, 72.0], vec![para(&title, 30.0, true, ACCENT, None)], true));
        }
        let top = if title.is_empty() { 40.0 } else { 112.0 };
        let paragraphs: Vec<Paragraph> = bullets
            .iter()
            .filter(|b| !b.trim().is_empty())
            .map(|b| {
                let spaces = b.chars().take_while(|c| *c == ' ').count();
                let level = u8::try_from((spaces / 2).min(4)).unwrap_or(0);
                let text = b.trim().trim_start_matches(['-', '*', '\u{2022}']).trim();
                bullet(&cut(text, MAX_BULLET_CHARS), level)
            })
            .collect();
        let rows = table.as_ref().map_or(0, |t| t.rows.len());
        let table_h = rows as f64 * 30.0;
        let body_h = if table.is_some() { (540.0 - top - 32.0 - table_h - 16.0).max(80.0) } else { 540.0 - top - 32.0 };
        if !paragraphs.is_empty() {
            slide.shapes.push(text_box(id(&mut p)?, "Inhalt", [48.0, top, 864.0, body_h], paragraphs.clone(), false));
        }
        if let Some(t) = table {
            let y = if paragraphs.is_empty() { top } else { top + body_h + 16.0 };
            slide.shapes.push(Shape {
                id: ShapeId(id(&mut p)?),
                name: "Tabelle".into(),
                xfrm: Some(Xfrm { x: 48.0, y, w: 864.0, h: table_h, ..Xfrm::default() }),
                kind: ShapeKind::Table(t),
                ..Shape::default()
            });
        }
        let notes = cut(s.get("notes").and_then(Value::as_str).unwrap_or(""), MAX_NOTES_CHARS);
        if !notes.trim().is_empty() {
            slide.notes = TextBody::from_text(&notes);
            slide.has_notes_slide = true;
        }
        p.slides.push(Arc::new(slide));
    }
    craft_pptx::export(&p, &Plain).map_err(|e| Error::invalid(format!("Folien ließen sich nicht schreiben: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn part(bytes: &[u8], name: &str) -> String {
        let pkg = craft_opc::Package::open(bytes, &craft_pptx::LIMITS).unwrap();
        String::from_utf8(pkg.get(name).unwrap_or_else(|| panic!("no part {name}")).to_vec()).unwrap()
    }

    #[test]
    fn title_bullets_table_and_notes() {
        let spec = json!({"title": "Photosynthese", "subtitle": "Klasse 7b", "slides": [
            {"title": "Was passiert?", "bullets": ["Licht wird Energie", "  Chlorophyll", "- CO₂ + Wasser"], "notes": "Langsam sprechen"},
            {"title": "Vergleich", "table": {"columns": ["Pflanze", "Licht"], "rows": [["Farn", "wenig"], ["Kaktus", 12]]}}
        ]});
        let bytes = slides_pptx(&spec).unwrap();
        assert!(bytes.starts_with(b"PK"));
        let s1 = part(&bytes, "ppt/slides/slide1.xml");
        assert!(s1.contains("<a:t>Photosynthese</a:t>") && s1.contains("<a:t>Klasse 7b</a:t>"), "{s1}");
        let s2 = part(&bytes, "ppt/slides/slide2.xml");
        assert!(s2.contains("<a:t>Was passiert?</a:t>") && s2.contains("<a:t>Licht wird Energie</a:t>"));
        assert!(s2.contains("lvl=\"1\""), "sub-point: {s2}");
        assert!(s2.contains("<a:t>CO₂ + Wasser</a:t>"));
        let s3 = part(&bytes, "ppt/slides/slide3.xml");
        assert!(s3.contains("<a:tbl>") && s3.contains("<a:t>Kaktus</a:t>") && s3.contains("<a:t>12</a:t>"), "{s3}");
        assert!(part(&bytes, "docProps/core.xml").contains("Photosynthese"));
        let back = craft_pptx::import(&bytes, &Plain).unwrap();
        assert_eq!(back.slides.len(), 3);
        assert!(back.slides[1].notes_text().contains("Langsam sprechen"));
    }

    #[test]
    fn hostile_decks_are_refused() {
        assert!(slides_pptx(&json!({})).is_err());
        let many: Vec<Value> = (0..MAX_SLIDES).map(|i| json!({"title": format!("F{i}")})).collect();
        assert!(slides_pptx(&json!({"title": "x", "slides": many})).is_err());
        let bullets: Vec<String> = (0..=MAX_BULLETS).map(|i| i.to_string()).collect();
        assert!(slides_pptx(&json!({"slides": [{"bullets": bullets}]})).is_err());
        let rows: Vec<Value> = (0..=MAX_TABLE_ROWS).map(|_| json!(["a"])).collect();
        assert!(slides_pptx(&json!({"slides": [{"table": {"rows": rows}}]})).is_err());
        let wide: Vec<String> = (0..=MAX_TABLE_COLS).map(|i| i.to_string()).collect();
        assert!(slides_pptx(&json!({"slides": [{"table": {"rows": [wide]}}]})).is_err());
        // Long text is cut, odd values become text.
        let long = "x".repeat(10_000);
        let bytes = slides_pptx(&json!({"slides": [{"title": long, "bullets": [1, null, {"a": 2}], "table": "kein Objekt"}]})).unwrap();
        let s1 = part(&bytes, "ppt/slides/slide1.xml");
        assert!(!s1.contains(&"x".repeat(MAX_TITLE_CHARS + 1)));
    }
}
