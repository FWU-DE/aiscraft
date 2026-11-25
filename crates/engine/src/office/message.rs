//! An answer as a file (#46): Word and PDF from its Markdown, Excel from its Markdown tables,
//! Markdown as it is. The same writers as documents (#38); nothing a model wrote runs.

use std::path::Path;

use aiscraft_core::{Error, Result, Role};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel};

/// Tables taken from one answer at most.
const MAX_TABLES: usize = 20;

/// One Markdown table: header and rows of cell texts (markup removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // `\|` is a pipe inside a cell.
    t.replace("\\|", "\u{1}").split('|').map(|c| c.replace('\u{1}', "|").replace("**", "").replace('`', "").trim().to_string()).collect()
}

fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// The pipe tables of a Markdown text (outside code blocks).
pub fn markdown_tables(md: &str) -> Vec<Table> {
    let mut out = Vec::new();
    let lines: Vec<&str> = md.lines().take(100_000).collect();
    let mut in_code = false;
    let mut i = 0;
    while i < lines.len() && out.len() < MAX_TABLES {
        let line = lines.get(i).copied().unwrap_or("");
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if in_code || !t.contains('|') || !lines.get(i + 1).is_some_and(|l| is_rule(l)) {
            i += 1;
            continue;
        }
        let columns = cells(line);
        let mut rows = Vec::new();
        i += 2;
        while let Some(l) = lines.get(i).filter(|l| l.contains('|') && !l.trim().is_empty()) {
            let mut r = cells(l);
            r.resize(columns.len().max(r.len().min(columns.len())), String::new());
            rows.push(r);
            i += 1;
        }
        out.push(Table { columns, rows });
    }
    out
}

/// A cell for Excel: a number where the text is one (German `3,5`, `1.234,5`, or `3.5`, also
/// with % or €), else the text. Text starting with `=` stays text, never a formula.
pub fn cell_value(s: &str) -> Value {
    let t = s.trim();
    let bare = t.trim_end_matches(['%', '€']).trim();
    let looks = !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-' | '+')) && bare.chars().any(|c| c.is_ascii_digit());
    // Thousands groups have three digits each; „3.12.2026“ is a date, not a number.
    let grouped = |sep: char, part: &str| part.split(sep).skip(1).all(|g| g.len() == 3);
    let int_part = bare.split(',').next().unwrap_or("");
    let single_decimal = !bare.contains(',') && bare.matches('.').count() == 1;
    let looks = looks && (!int_part.contains('.') || single_decimal || grouped('.', int_part));
    if looks {
        let normal = if bare.contains(',') { bare.replace('.', "").replace(',', ".") } else if bare.matches('.').count() > 1 { bare.replace('.', "") } else { bare.to_string() };
        if let Ok(n) = normal.parse::<f64>()
            && n.is_finite()
        {
            return json!(if t.ends_with('%') { n / 100.0 } else { n });
        }
    }
    if t.starts_with('=') {
        return json!(format!(" {t}"));
    }
    json!(t)
}

/// The workbook spec (`office::xlsx`) of an answer's tables; an error when it has none.
pub fn tables_spec(title: &str, md: &str) -> Result<Value> {
    let tables = markdown_tables(md);
    if tables.is_empty() {
        return Err(Error::invalid("Die Antwort enthält keine Tabelle. Excel geht nur mit Tabellen; Word oder PDF gehen immer."));
    }
    let sheets: Vec<Value> = tables
        .iter()
        .enumerate()
        .map(|(i, t)| {
            json!({
                "name": if tables.len() == 1 { "Tabelle".to_string() } else { format!("Tabelle {}", i + 1) },
                "columns": t.columns,
                "rows": t.rows.iter().map(|r| r.iter().map(|c| cell_value(c)).collect::<Vec<_>>()).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({"title": title, "sheets": sheets}))
}

/// A title for the file: the answer's first heading or first words.
pub fn title_of(md: &str) -> String {
    let first = md.lines().map(|l| l.trim().trim_start_matches(['#', '-', '*', ' ', '>'])).find(|l| !l.is_empty() && !l.starts_with("```")).unwrap_or("Antwort");
    let t: String = first.replace(['*', '_', '`'], "").chars().take(60).collect();
    if t.trim().is_empty() { "Antwort".into() } else { t.trim().to_string() }
}

pub(crate) static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "message.export",
    label: "Antwort speichern als …",
    group: "Chat",
    shortcut: None,
    params: "message_id: string, format: docx|pdf|xlsx|markdown, path?: string (ohne: Base64 oder Text zurück), title?: string",
    palette: false,
    run: |c, p| Box::pin(export(c, p)),
}];

/// The bytes of an answer as `format`: (bytes, extension).
pub(crate) fn answer_bytes(title: &str, md: &str, format: &str) -> Result<(Vec<u8>, &'static str)> {
    Ok(match format {
        "docx" | "word" => (super::markdown_docx(title, md)?, "docx"),
        "pdf" => (super::pdf::markdown_pdf(title, md)?, "pdf"),
        "xlsx" | "excel" => (super::xlsx::workbook_xlsx(&tables_spec(title, md)?)?, "xlsx"),
        "markdown" | "md" => (md.as_bytes().to_vec(), "md"),
        other => return Err(Error::invalid(format!("Format {other} gibt es hier nicht (docx, pdf, xlsx, markdown)"))),
    })
}

async fn export(c: Ctx, p: Value) -> Result<Value> {
    let mid = req_str(&p, "message_id")?;
    let format = req_str(&p, "format")?.to_lowercase();
    let m = c.state().messages.iter().find(|m| m.id == mid).cloned().ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    if m.role != Role::Assistant || m.content.trim().is_empty() {
        return Err(Error::invalid("Nur Antworten mit Text lassen sich speichern"));
    }
    let title = opt_str(&p, "title").filter(|t| !t.trim().is_empty()).unwrap_or_else(|| title_of(&m.content));
    let (bytes, ext) = answer_bytes(&title, &m.content, &format)?;
    if bytes.len() > super::MAX_FILE {
        return Err(Error::invalid("Die Datei wäre größer als 25 MB"));
    }
    if let Some(path) = opt_str(&p, "path") {
        crate::disk::write(Path::new(&path), &bytes)?;
        c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
        return Ok(json!({"path": path, "bytes": bytes.len()}));
    }
    let name = super::file_name(&title, ext);
    if ext == "md" {
        return Ok(json!({"text": String::from_utf8_lossy(&bytes), "file_name": name}));
    }
    use base64::Engine as _;
    Ok(json!({"base64": base64::engine::general_purpose::STANDARD.encode(&bytes), "bytes": bytes.len(), "file_name": name}))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn tables_and_cells() {
        let md = "Noten:\n\n| Name | Note | Anteil |\n|:---|---:|---|\n| **Ada** | 1,5 | 20 % |\n| Bo \\| Ca | 2 | =SUMME(A1) |\n\n```\n| a | b |\n|---|---|\n```\n| Kurz |\n|---|\n| x | y | z |";
        let t = markdown_tables(md);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].columns, vec!["Name", "Note", "Anteil"]);
        assert_eq!(t[0].rows[0], vec!["Ada", "1,5", "20 %"]);
        assert_eq!(t[0].rows[1][0], "Bo | Ca");
        assert_eq!(cell_value("1,5"), json!(1.5));
        assert_eq!(cell_value("1.234,5"), json!(1234.5));
        assert_eq!(cell_value("3.5"), json!(3.5));
        assert_eq!(cell_value("20 %"), json!(0.2));
        assert_eq!(cell_value("=SUMME(A1)"), json!(" =SUMME(A1)"));
        assert_eq!(cell_value("3.12.2026"), json!("3.12.2026"), "a date stays text");
        assert_eq!(cell_value("1.234.567"), json!(1_234_567.0));
        assert_eq!(cell_value("Klasse 7b"), json!("Klasse 7b"));
        assert!(tables_spec("x", "kein Tisch").is_err());
        assert_eq!(title_of("## Elternbrief\n\nText"), "Elternbrief");
        assert_eq!(title_of(""), "Antwort");
        for s in ["|", "|\n|-|", "||\n|--|\n||", "| a |\n|---|\n|"] {
            let _ = markdown_tables(s);
        }
    }
}
