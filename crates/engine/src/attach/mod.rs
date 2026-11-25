//! Attachments prepared on the device before they are sent (#45, #47, #50).
//!
//! AIS.chat takes only some file types (`AIS_ACCEPTS`, from `apps/chat-bot/src/const.ts`), at
//! most 20 MB each and five pictures per chat. What it does not take is read here and uploaded
//! as a file of a type it does take: Markdown for documents, mail, calendars, contacts and
//! slides, CSV for one sheet, JPEG or PNG for pictures. Photos lose their metadata (location,
//! camera) and are scaled to 1000 px. A person can send only some pages of a PDF, one sheet of
//! a workbook or some slides. Other backends keep their behaviour; they get converted text where
//! they could not read the file at all.
//!
//! Commands: `attachment.prepare` (what will happen, shown on the attachment chip),
//! `attachment.set_range` (the "Nur Seiten …" field) and `attachment.remove`.

mod docs;
mod image;
mod pdf;
pub(crate) mod pdf_text;
mod range;
mod sheets;
mod slides;

use std::path::Path;

use aiscraft_core::{Attachment, BackendKind, Error, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{CommandSpec, opt_str, req_str};
use crate::files;

/// File types AIS.chat accepts for upload.
pub const AIS_ACCEPTS: &[&str] =
    &["csv", "docx", "htm", "html", "md", "odp", "ods", "odt", "pdf", "pptx", "tex", "txt", "xlsx", "jpeg", "jpg", "png", "svg", "webp"];
/// AIS.chat's limit for one file.
pub const AIS_MAX_BYTES: usize = 20 * 1024 * 1024;
/// AIS.chat's limit for pictures in one chat.
pub const AIS_MAX_IMAGES: usize = 5;
const IMAGES: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "avif"];
const PREVIEW_CHARS: usize = 400;

/// Whom the file is prepared for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    AisChat,
    Other,
}

impl Target {
    pub fn of(kind: BackendKind) -> Target {
        if kind == BackendKind::AisChat { Target::AisChat } else { Target::Other }
    }
}

/// Which part of a file a person can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RangeKind {
    Pages,
    Sheet,
    Slides,
}

impl RangeKind {
    pub fn of(ext: &str) -> Option<RangeKind> {
        match ext {
            "pdf" => Some(RangeKind::Pages),
            "xls" | "xlsx" | "xlsm" | "ods" => Some(RangeKind::Sheet),
            "ppt" | "pps" | "pptx" => Some(RangeKind::Slides),
            _ => None,
        }
    }
}

/// What will be sent for one attached file: shown on its chip before sending.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AttachmentPreview {
    pub path: String,
    /// The name sent (e.g. "Brief.md" for "Brief.doc").
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// "Wird als Markdown hochgeladen: Brief.md", "Standortdaten entfernt · auf 1000 px verkleinert".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub location_removed: bool,
    /// Pages, sheet or slides can be picked; `range` is the current choice ("" = all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_kind: Option<RangeKind>,
    #[serde(default)]
    pub range: String,
    /// Pages or slides in the file, or the sheet names.
    #[serde(default)]
    pub total: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<String>,
    /// Why the file cannot be sent like this (German, for the chip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The start of the converted text, for a look before sending.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub preview: String,
}

/// A file ready to send.
pub struct Prepared {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub note: Option<String>,
    pub location_removed: bool,
    pub range_kind: Option<RangeKind>,
    pub total: usize,
    pub sheets: Vec<String>,
}

pub(crate) fn is_cfb(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

fn ext_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

fn stem_of(path: &Path) -> String {
    let s = path.file_stem().and_then(|n| n.to_str()).unwrap_or("Datei").trim();
    if s.is_empty() { "Datei".into() } else { s.chars().take(120).collect() }
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{} ist keine Datei", path.display())));
    }
    if meta.len() > files::MAX_FILE_BYTES {
        return Err(Error::invalid(format!("{} ist größer als 25 MB", path.display())));
    }
    std::fs::read(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))
}

fn text_file(name: String, mime: &str, text: String, note: Option<String>) -> Prepared {
    Prepared { name, mime: mime.to_string(), bytes: text.into_bytes(), note, location_removed: false, range_kind: None, total: 0, sheets: Vec::new() }
}

fn as_markdown(stem: &str, md: String, target: Target) -> Prepared {
    let name = format!("{stem}.md");
    let note = if target == Target::AisChat { format!("Wird als Markdown hochgeladen: {name}") } else { format!("Als Text gelesen: {name}") };
    text_file(name, "text/markdown", md, Some(note))
}

/// UTF-8 without control characters other than tabs and line breaks (not a ZIP, not a binary).
fn looks_like_text(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok_and(|t| !t.chars().any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{c}')))
}

/// The sheets of a workbook (class lists, #55).
pub(crate) fn read_sheets(ext: &str, bytes: &[u8]) -> Result<Vec<sheets::Grid>> {
    sheets::read(ext, bytes, Some(0))
}

/// Prepare `path` for `target`, with the part a person picked (`range`, "" = all).
pub fn prepare(path: &Path, range: &str, target: Target) -> Result<Prepared> {
    let bytes = read_file(path)?;
    prepare_bytes(path, bytes, range, target)
}

fn prepare_bytes(path: &Path, bytes: Vec<u8>, range: &str, target: Target) -> Result<Prepared> {
    let ext = ext_of(path);
    let stem = stem_of(path);
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Datei").to_string();
    let range = range.trim();
    let kind = RangeKind::of(&ext);
    let mut p = if !range.is_empty() {
        let Some(kind) = kind else {
            return Err(Error::invalid(format!("Bei .{ext}-Dateien kann man keine Seiten auswählen. Das geht bei PDF, Tabellen und Präsentationen.")));
        };
        part(&ext, &stem, &bytes, range, kind)?
    } else {
        whole(&ext, &stem, &file_name, bytes, target)?
    };
    if let Some(kind) = kind
        && p.range_kind.is_none()
    {
        p.range_kind = Some(kind);
    }
    if target == Target::AisChat && p.bytes.len() > AIS_MAX_BYTES {
        let hint = if kind.is_some() { " Wähle nur einen Teil aus („Nur Seiten …“)." } else { "" };
        return Err(Error::invalid(format!("{} ist größer als 20 MB, so nimmt AIS.chat die Datei nicht an.{hint}", p.name)));
    }
    Ok(p)
}

/// Only the pages, sheet or slides in `range`.
fn part(ext: &str, stem: &str, bytes: &[u8], range: &str, kind: RangeKind) -> Result<Prepared> {
    match kind {
        RangeKind::Pages => {
            let total = pdf::page_count(bytes)?;
            let numbers = range::numbers(range, total, "Seiten")?;
            let label = range::label(&numbers);
            let out = if numbers.len() == total { bytes.to_vec() } else { pdf::subset(bytes, &numbers)? };
            let name = format!("{stem} (Seiten {label}).pdf");
            Ok(Prepared {
                note: Some(format!("Nur Seiten {label} von {total}: {name}")),
                name,
                mime: "application/pdf".into(),
                bytes: out,
                location_removed: false,
                range_kind: Some(kind),
                total,
                sheets: Vec::new(),
            })
        }
        RangeKind::Sheet => {
            let all = sheets::read(ext, bytes, None)?;
            let names: Vec<String> = all.iter().map(|g| g.name.clone()).collect();
            let i = range::sheet(range, &names)?;
            let grid = all.get(i).ok_or_else(|| Error::invalid("Dieses Tabellenblatt gibt es nicht"))?;
            let clean: String = grid.name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
            let name = format!("{stem} - {}.csv", clean.trim());
            let mut p = text_file(name.clone(), "text/csv", sheets::csv(grid), Some(format!("Nur Tabellenblatt „{}“ als CSV: {name}", grid.name)));
            p.range_kind = Some(kind);
            p.total = names.len();
            p.sheets = names;
            Ok(p)
        }
        RangeKind::Slides => {
            let deck = slides::read(ext, bytes)?;
            let total = slides::count(&deck);
            let numbers = range::numbers(range, total, "Folien")?;
            let label = range::label(&numbers);
            let name = format!("{stem} (Folien {label}).md");
            let mut p = text_file(
                name.clone(),
                "text/markdown",
                slides::markdown(stem, &deck, Some(&numbers)),
                Some(format!("Nur Folien {label} von {total} als Markdown: {name}")),
            );
            p.range_kind = Some(kind);
            p.total = total;
            Ok(p)
        }
    }
}

/// The whole file, converted where the target needs it.
fn whole(ext: &str, stem: &str, file_name: &str, bytes: Vec<u8>, target: Target) -> Result<Prepared> {
    let mime = files::mime_of(Path::new(file_name)).to_string();
    let keep = |bytes: Vec<u8>| Prepared {
        name: file_name.to_string(),
        mime: mime.clone(),
        bytes,
        note: None,
        location_removed: false,
        range_kind: None,
        total: 0,
        sheets: Vec::new(),
    };
    let ooxml = matches!(ext, "docx" | "xlsx" | "pptx" | "xlsm");
    if ooxml && is_cfb(&bytes) {
        return Err(Error::unsupported(format!("{file_name} ist mit einem Passwort geschützt. Speichere die Datei ohne Passwort und hänge sie dann an.")));
    }
    // Pictures: for AIS.chat always re-encoded without metadata; elsewhere only formats a
    // model cannot take.
    if IMAGES.contains(&ext) && (target == Target::AisChat || !matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif")) {
        let photo = image::prepare(&bytes, file_name)?;
        let name = format!("{stem}.{}", photo.ext);
        let mut notes = Vec::new();
        if photo.location_removed {
            notes.push("Standortdaten entfernt".to_string());
        } else if photo.metadata_removed {
            notes.push("Kameradaten entfernt".to_string());
        }
        if photo.scaled {
            notes.push(format!("auf {} × {} px verkleinert", photo.width, photo.height));
        }
        let new_type = photo.ext != ext && !(photo.ext == "jpg" && ext == "jpeg");
        if new_type {
            notes.push(format!("wird als {} hochgeladen: {name}", if photo.ext == "png" { "PNG" } else { "JPEG" }));
        }
        let mime = if photo.ext == "png" { "image/png" } else { "image/jpeg" };
        return Ok(Prepared {
            name,
            mime: mime.into(),
            bytes: photo.bytes,
            note: (!notes.is_empty()).then(|| notes.join(" · ")),
            location_removed: photo.location_removed,
            range_kind: None,
            total: 0,
            sheets: Vec::new(),
        });
    }
    let readable_elsewhere = files::is_text(&mime) || files::is_image(&mime) || mime == "application/pdf";
    let pass = match target {
        Target::AisChat => AIS_ACCEPTS.contains(&ext),
        Target::Other => readable_elsewhere,
    };
    if pass {
        return Ok(keep(bytes));
    }
    match ext {
        "doc" | "dot" | "rtf" | "epub" | "eml" | "mbox" | "ics" | "vcf" | "docx" => Ok(as_markdown(stem, docs::markdown(ext, stem, &bytes)?, target)),
        "xls" | "xlsm" | "xlsx" | "ods" => {
            let grids = sheets::read(ext, &bytes, None)?;
            let names: Vec<String> = grids.iter().map(|g| g.name.clone()).collect();
            let mut p = if let [one] = grids.as_slice() {
                let name = format!("{stem}.csv");
                let note = if target == Target::AisChat { format!("Wird als CSV hochgeladen: {name}") } else { format!("Als Text gelesen: {name}") };
                text_file(name, "text/csv", sheets::csv(one), Some(note))
            } else {
                let mut p = as_markdown(stem, sheets::markdown(stem, &grids), target);
                if let Some(n) = p.note.as_mut() {
                    n.push_str(&format!(" ({} Tabellenblätter; „Nur Blatt …“ schickt eines als CSV)", names.len()));
                }
                p
            };
            p.total = names.len();
            p.sheets = names;
            Ok(p)
        }
        "ppt" | "pps" | "pot" | "pptx" | "odp" => {
            if ext == "odp" {
                return Ok(keep(bytes));
            }
            let deck = slides::read(ext, &bytes)?;
            let mut p = as_markdown(stem, slides::markdown(stem, &deck, None), target);
            p.total = slides::count(&deck);
            Ok(p)
        }
        _ if target == Target::AisChat && (files::is_text(&mime) || looks_like_text(&bytes)) && !bytes.is_empty() => {
            let name = format!("{file_name}.txt");
            let text = String::from_utf8_lossy(&bytes).to_string();
            Ok(text_file(name.clone(), "text/plain", text, Some(format!("Wird als Text hochgeladen: {name}"))))
        }
        _ if target == Target::AisChat => Err(Error::unsupported(format!(
            "AIS.chat nimmt {} nicht an. Möglich sind Office-Dateien, PDF, Text, Bilder, E-Books, E-Mails, Kalender und Kontakte.",
            if ext.is_empty() { file_name.to_string() } else { format!(".{ext}-Dateien") }
        ))),
        // Other backends: as before (send says it cannot read the file).
        _ => Ok(keep(bytes)),
    }
}

/// The attachment for a request: text for backends that read text inline, base64 for pictures
/// and PDFs where the backend takes them inline. `local` is the kept copy.
pub(crate) fn attachment(p: &Prepared, local: &Path, inline_binary: bool) -> Attachment {
    let mut a = Attachment {
        name: p.name.clone(),
        mime: p.mime.clone(),
        size: p.bytes.len() as u64,
        local_path: Some(local.display().to_string()),
        ..Default::default()
    };
    if files::is_text(&p.mime) {
        a.text = Some(String::from_utf8_lossy(&p.bytes).chars().take(files::MAX_INLINE_TEXT_CHARS).collect());
    } else if inline_binary && (files::is_image(&p.mime) || p.mime == "application/pdf") {
        a.data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(&p.bytes));
    }
    a
}

/// The target for the selected backend (or `backend`).
fn target_for(c: &Ctx, backend: Option<&str>) -> Target {
    let s = c.state();
    let id = backend.map(str::to_string).unwrap_or_else(|| s.selection.backend_id.clone());
    s.backend(&id).map(|b| Target::of(b.config.kind)).unwrap_or(Target::Other)
}

fn preview_of(path: &str, range: &str, result: &Result<Prepared>) -> AttachmentPreview {
    let p = Path::new(path);
    let ext = ext_of(p);
    let mut v = AttachmentPreview {
        path: path.to_string(),
        name: p.file_name().and_then(|n| n.to_str()).unwrap_or("Datei").to_string(),
        range: range.trim().to_string(),
        range_kind: RangeKind::of(&ext),
        ..AttachmentPreview::default()
    };
    match result {
        Ok(prep) => {
            v.name = prep.name.clone();
            v.mime = prep.mime.clone();
            v.size = prep.bytes.len() as u64;
            v.note = prep.note.clone();
            v.location_removed = prep.location_removed;
            v.range_kind = prep.range_kind.or(v.range_kind);
            v.total = prep.total;
            v.sheets = prep.sheets.clone();
            if files::is_text(&prep.mime) {
                v.preview = String::from_utf8_lossy(&prep.bytes).chars().take(PREVIEW_CHARS).collect();
            }
        }
        Err(e) => v.error = Some(e.message.clone()),
    }
    v
}

/// Prepare and remember what will be sent; the error, if any, is on the chip too.
/// `quiet`: the error is on the chip only (the composer asks this way), not also a notice.
async fn prepare_cmd(c: Ctx, path: String, range: String, backend: Option<String>, quiet: bool) -> Result<Value> {
    if path.trim().is_empty() {
        return Err(Error::invalid("path fehlt"));
    }
    let target = target_for(&c, backend.as_deref());
    let (p, r) = (path.clone(), range.clone());
    let result =
        tokio::task::spawn_blocking(move || prepare(Path::new(&p), &r, target)).await.map_err(|e| Error::invalid(format!("Vorbereiten abgebrochen: {e}")))?;
    let preview = preview_of(&path, &range, &result);
    c.update(|s| {
        s.attachments.insert(path.clone(), preview.clone());
        // Never more than a handful of entries: the composer holds a few files at a time.
        while s.attachments.len() > 50 {
            let Some(first) = s.attachments.keys().next().cloned() else { break };
            s.attachments.remove(&first);
        }
    });
    if !quiet {
        result?;
    }
    serde_json::to_value(&preview).map_err(|e| Error::invalid(e.to_string()))
}

/// The range a person set for `path` in the composer, if any.
pub(crate) fn range_for(c: &Ctx, path: &str) -> String {
    c.state().attachments.get(path).map(|p| p.range.clone()).unwrap_or_default()
}

/// One entry of `chat.send`'s `files`: a path, or `{path, range}`, whose range is remembered
/// for the send.
pub(crate) fn file_param(c: &Ctx, v: &Value) -> Option<String> {
    if let Some(p) = v.as_str() {
        return Some(p.to_string());
    }
    let path = v.get("path")?.as_str()?.to_string();
    if let Some(range) = v.get("range").and_then(Value::as_str) {
        let range: String = range.chars().take(200).collect();
        c.update(|s| {
            s.attachments.entry(path.clone()).or_insert_with(|| AttachmentPreview { path: path.clone(), ..AttachmentPreview::default() }).range = range
        });
    }
    Some(path)
}

/// After sending: forget the previews of the files sent.
pub(crate) fn forget(c: &Ctx, paths: &[String]) {
    if paths.iter().any(|p| c.state().attachments.contains_key(p)) {
        c.update(|s| {
            for p in paths {
                s.attachments.remove(p);
            }
        });
    }
}

/// AIS.chat takes at most five pictures per chat: count those already sent in the open chat.
pub(crate) fn check_image_count(c: &Ctx, new_images: usize) -> Result<()> {
    if new_images == 0 {
        return Ok(());
    }
    let s = c.state();
    let sent =
        s.messages.iter().filter(|m| m.role == aiscraft_core::Role::User).flat_map(|m| m.attachments.iter()).filter(|a| files::is_image(&a.mime)).count();
    if sent + new_images > AIS_MAX_IMAGES {
        return Err(Error::invalid(format!(
            "AIS.chat nimmt höchstens {AIS_MAX_IMAGES} Bilder pro Chat an; hier sind es schon {sent}. Starte für weitere Bilder einen neuen Chat."
        )));
    }
    Ok(())
}

fn quiet(p: &Value) -> bool {
    p.get("quiet").and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "attachment.prepare",
        label: "Anhang vorbereiten",
        group: "Dateien",
        shortcut: None,
        params: "path: string, range?: string (\"3-7\", sheet name or number), backend?: string, quiet?: bool (errors only in the preview)",
        palette: false,
        run: |c, p| {
            Box::pin(async move { prepare_cmd(c, req_str(&p, "path")?, opt_str(&p, "range").unwrap_or_default(), opt_str(&p, "backend"), quiet(&p)).await })
        },
    },
    CommandSpec {
        id: "attachment.set_range",
        label: "Nur einen Teil der Datei senden",
        group: "Dateien",
        shortcut: None,
        params: "path: string, quiet?: bool, range: string (\"\" = whole file; pages \"3-7\", \"1,4\"; a sheet by name or number; slides \"2-5\")",
        palette: false,
        run: |c, p| {
            Box::pin(async move {
                let path = req_str(&p, "path")?;
                let range = p.get("range").and_then(Value::as_str).map(str::to_string).ok_or_else(|| Error::invalid("range fehlt"))?;
                prepare_cmd(c, path, range, opt_str(&p, "backend"), quiet(&p)).await
            })
        },
    },
    CommandSpec {
        id: "attachment.remove",
        label: "Anhang entfernen",
        group: "Dateien",
        shortcut: None,
        params: "path?: string (none = all)",
        palette: false,
        run: |c, p| {
            Box::pin(async move {
                let path = opt_str(&p, "path");
                let removed = c.update(|s| match &path {
                    Some(path) => usize::from(s.attachments.remove(path).is_some()),
                    None => std::mem::take(&mut s.attachments).len(),
                });
                Ok(json!({"removed": removed}))
            })
        },
    },
];

#[cfg(test)]
pub(crate) mod tests;
