//! Office files and PDF (#38): documents as Word and PDF, spreadsheets and slides from the
//! model. AIS.ChatCraft runs no code a model wrote; every file comes from our own writers on
//! the shared Craft libraries (craft-markdown + craft-docx, craft-xlsx, craft-pptx) and a small
//! PDF writer on craft-pdf's standard-font metrics (`pdf.rs`, see docs/craft-libs.md).
//!
//! A created file goes to the attachments folder in the app's data directory (non-destructive,
//! so the model's tools run without asking) and shows in the chat as a download card. Saving it
//! anywhere else goes through the person's save dialog (`chat.save_file`).

pub mod message;
pub mod pdf;
pub mod pptx;
pub mod xlsx;

use aiscraft_core::{Attachment, Error, Result, ToolSpec};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel};

/// A created file larger than this is refused (the attachment limit).
const MAX_FILE: usize = crate::files::MAX_FILE_BYTES as usize;
const MAX_MARKDOWN_CHARS: usize = 2_000_000;

/// Markdown as a Word document.
pub fn markdown_docx(title: &str, md: &str) -> Result<Vec<u8>> {
    let md: String = md.chars().take(MAX_MARKDOWN_CHARS).collect();
    let mut doc = craft_markdown::import(&md);
    doc.core.title = title.chars().take(200).collect();
    doc.core.creator = "AIS.ChatCraft".into();
    craft_docx::write(&doc).map_err(|e| Error::invalid(format!("Word-Datei ließ sich nicht schreiben: {e}")))
}

/// A Markdown document in a binary format: `docx` or `pdf`; `None` for other formats.
pub fn document_bytes(format: &str, title: &str, md: &str) -> Option<Result<Vec<u8>>> {
    match format {
        "docx" | "word" => Some(markdown_docx(title, md)),
        "pdf" => Some(pdf::markdown_pdf(title, &md.chars().take(MAX_MARKDOWN_CHARS).collect::<String>())),
        _ => None,
    }
}

/// A file name from a title: letters, digits, spaces and dashes, at most 80 characters.
pub fn file_name(title: &str, ext: &str) -> String {
    let base: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).take(80).collect();
    let base = base.trim().trim_matches('_').trim();
    format!("{}.{ext}", if base.is_empty() { "Datei" } else { base })
}

/// Builds the file `p` describes: (bytes, extension).
fn build(p: &Value) -> Result<(Vec<u8>, &'static str)> {
    let format = opt_str(p, "format").ok_or_else(|| Error::invalid("format fehlt (xlsx, pptx, docx oder pdf)"))?;
    let title = opt_str(p, "title").unwrap_or_default();
    let bytes = match format.as_str() {
        "xlsx" => (xlsx::workbook_xlsx(p)?, "xlsx"),
        "pptx" => (pptx::slides_pptx(p)?, "pptx"),
        "docx" | "pdf" => {
            let md = req_str(p, "markdown")?;
            let ext = if format == "pdf" { "pdf" } else { "docx" };
            (document_bytes(ext, &title, &md).ok_or_else(|| Error::invalid("Format unbekannt"))??, ext)
        }
        other => return Err(Error::invalid(format!("Format {other} gibt es nicht (xlsx, pptx, docx, pdf)"))),
    };
    if bytes.0.len() > MAX_FILE {
        return Err(Error::invalid("Die Datei wäre größer als 25 MB"));
    }
    Ok(bytes)
}

/// Creates a file in the attachments folder; the attachment that points at it.
pub(crate) fn create_attachment(c: &Ctx, p: &Value) -> Result<Attachment> {
    let (bytes, ext) = build(p)?;
    let name = file_name(&opt_str(p, "title").unwrap_or_default(), ext);
    let path = crate::files::keep_copy(&c.0.attachments_dir, std::path::Path::new(&name), &bytes)?;
    Ok(Attachment {
        mime: crate::files::mime_of(&path).to_string(),
        size: bytes.len() as u64,
        local_path: Some(path.display().to_string()),
        name,
        ..Attachment::default()
    })
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "file.create",
        label: "Datei erzeugen",
        group: "Dateien",
        shortcut: None,
        params: "format: xlsx|pptx|docx|pdf, title?: string, markdown?: string (docx, pdf), sheets?: [{name, columns, rows, widths}] (xlsx), subtitle?: string, slides?: [{title, bullets, table, notes}] (pptx)",
        palette: false,
        run: |c, p| Box::pin(file_create(c, p)),
    },
    CommandSpec {
        id: "chat.save_file",
        label: "Datei aus dem Chat speichern",
        group: "Chat",
        shortcut: None,
        params: "message_id: string, index?: number, path: string",
        palette: false,
        run: |c, p| Box::pin(save_file(c, p)),
    },
];

async fn file_create(c: Ctx, p: Value) -> Result<Value> {
    Ok(serde_json::to_value(create_attachment(&c, &p)?)?)
}

/// Copies a file attached to a message (a created file) to `path`, chosen in the save dialog.
async fn save_file(c: Ctx, p: Value) -> Result<Value> {
    let mid = req_str(&p, "message_id")?;
    let index = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let path = req_str(&p, "path")?;
    let a = c
        .state()
        .messages
        .iter()
        .find(|m| m.id == mid)
        .and_then(|m| m.attachments.get(index).cloned())
        .ok_or_else(|| Error::not_found("Datei nicht gefunden"))?;
    let src = a.local_path.ok_or_else(|| Error::not_found("Die Datei liegt nicht auf diesem Rechner"))?;
    // Only files AIS.ChatCraft keeps itself, never some other path a message names.
    let dir = std::fs::canonicalize(&c.0.attachments_dir).map_err(|_| Error::not_found("Datei nicht gefunden"))?;
    let real = std::fs::canonicalize(&src).map_err(|_| Error::not_found("Die Datei gibt es nicht mehr"))?;
    if !real.starts_with(&dir) {
        return Err(Error::refused("Diese Datei liegt nicht im Anhänge-Ordner"));
    }
    let meta = std::fs::metadata(&real).map_err(|e| Error::storage(e.to_string()))?;
    if meta.len() > crate::files::MAX_FILE_BYTES {
        return Err(Error::invalid("Die Datei ist größer als 25 MB"));
    }
    let bytes = std::fs::read(&real).map_err(|e| Error::storage(format!("{}: {e}", a.name)))?;
    crate::disk::write(std::path::Path::new(&path), &bytes)?;
    c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
    Ok(json!({"path": path}))
}

// ---- model tools ----------------------------------------------------------------------------

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    let cell = json!({"type": ["string", "number", "boolean", "null"]});
    vec![
        ToolSpec {
            name: "spreadsheet_create".into(),
            description: format!(
                "Erstellt eine Excel-Tabelle (.xlsx) als Download im Chat, z. B. eine Notenliste oder einen Plan. Zellen: Text, Zahl, true/false oder null; Text mit = am Anfang ist eine Formel (z. B. \"=SUMME(B2:B9)\" auf Englisch: \"=SUM(B2:B9)\"). columns ist die Kopfzeile. Höchstens {} Blätter, {} Zeilen je Blatt.",
                xlsx::MAX_SHEETS,
                xlsx::MAX_ROWS
            ),
            input_schema: json!({"type": "object", "properties": {
                "title": {"type": "string", "description": "Dateiname ohne Endung"},
                "sheets": {"type": "array", "items": {"type": "object", "properties": {
                    "name": {"type": "string"},
                    "columns": {"type": "array", "items": {"type": "string"}, "description": "Kopfzeile"},
                    "rows": {"type": "array", "items": {"type": "array", "items": cell}},
                    "widths": {"type": "array", "items": {"type": "number"}, "description": "Spaltenbreiten in Zeichen (optional)"}
                }, "required": ["rows"]}}
            }, "required": ["title", "sheets"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "slides_create".into(),
            description: format!(
                "Erstellt eine PowerPoint-Präsentation (.pptx) als Download im Chat: eine Titelfolie aus title und subtitle, dann Folien mit Titel, Stichpunkten (zwei Leerzeichen am Anfang = Unterpunkt), optional einer Tabelle und Sprechernotizen. Höchstens {} Folien, {} Stichpunkte je Folie.",
                pptx::MAX_SLIDES,
                pptx::MAX_BULLETS
            ),
            input_schema: json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "subtitle": {"type": "string"},
                "slides": {"type": "array", "items": {"type": "object", "properties": {
                    "title": {"type": "string"},
                    "bullets": {"type": "array", "items": {"type": "string"}},
                    "table": {"type": "object", "properties": {
                        "columns": {"type": "array", "items": {"type": "string"}},
                        "rows": {"type": "array", "items": {"type": "array", "items": {"type": ["string", "number"]}}}
                    }},
                    "notes": {"type": "string", "description": "Sprechernotizen"}
                }, "required": ["title"]}}
            }, "required": ["title", "slides"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "document_file_create".into(),
            description: "Erstellt aus Markdown eine Word-Datei (.docx) oder ein PDF als Download im Chat, z. B. ein Arbeitsblatt zum Ausdrucken. Für Texte, die die Person im Programm weiterbearbeiten will, lieber document_create.".into(),
            input_schema: json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "markdown": {"type": "string"},
                "format": {"type": "string", "enum": ["docx", "pdf"]}
            }, "required": ["title", "markdown", "format"], "additionalProperties": false}),
        },
    ]
}

/// Runs one of the tools above; `None` when `name` is none of them. The file is kept for the
/// chat in `pending`, which the chat loop puts on the answer.
pub(crate) fn run_tool(c: &Ctx, name: &str, arguments: &Value) -> Option<Result<(String, Attachment)>> {
    let format = match name {
        "spreadsheet_create" => "xlsx",
        "slides_create" => "pptx",
        "document_file_create" => match arguments.get("format").and_then(Value::as_str) {
            Some("pdf") => "pdf",
            _ => "docx",
        },
        _ => return None,
    };
    let mut p = arguments.clone();
    match p.as_object_mut() {
        Some(o) => {
            o.insert("format".into(), json!(format));
        }
        None => return Some(Err(Error::invalid("Die Argumente müssen ein Objekt sein"))),
    }
    Some(create_attachment(c, &p).map(|a| {
        let kb = a.size.div_ceil(1024);
        (format!("Datei „{}“ erstellt ({kb} KB). Sie steht im Chat zum Speichern bereit; nenne keinen Speicherort.", a.name), a)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_documents_from_markdown() {
        let bytes =
            markdown_docx("Brief an Eltern", "# Elternabend\n\nAm **Donnerstag** um 19 Uhr.\n\n- Tagesordnung\n- Fragen\n\n| A | B |\n|---|---|\n| 1 | 2 |")
                .unwrap();
        let pkg = craft_opc::Package::open(&bytes, &craft_opc::Limits::new(1000, 64 << 20, 256 << 20)).unwrap();
        let doc = String::from_utf8(pkg.get("word/document.xml").unwrap().to_vec()).unwrap();
        assert!(doc.contains("Elternabend") && doc.contains("Donnerstag") && doc.contains("<w:b/>") && doc.contains("<w:tbl>"), "{doc}");
        let core = String::from_utf8(pkg.get("docProps/core.xml").unwrap().to_vec()).unwrap();
        assert!(core.contains("Brief an Eltern"));
        let back = craft_docx::read(&bytes).unwrap();
        assert!(back.plain_text(craft_wordmodel::StoryRef::Body).contains("Tagesordnung"));
    }

    #[test]
    fn file_names_and_formats() {
        assert_eq!(file_name("Noten 7b/8a: Mathe", "xlsx"), "Noten 7b_8a_ Mathe.xlsx");
        assert_eq!(file_name("../..", "pdf"), "Datei.pdf");
        assert!(document_bytes("odt", "x", "y").is_none());
        assert!(build(&json!({"format": "exe"})).is_err());
        assert!(build(&json!({"format": "docx"})).is_err(), "markdown missing");
        assert!(build(&json!({})).is_err());
    }
}
