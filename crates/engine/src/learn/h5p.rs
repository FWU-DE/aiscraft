//! H5P artifacts (#59): quizzes teachers take to Moodle, Mebis, ByCS or any other LMS.
//!
//! The artifact is `aiscraft-h5p` JSON ([`aiscraft_core::h5p`]): title, intro and tasks in the
//! worksheet's task format, edited in the app as a task list. "Speichern" writes a `.h5p`
//! package (a zip with `h5p.json` and `content/content.json`) through craft-zip. The package
//! names the H5P content types it needs but does not bring their code: the LMS has them.
//!
//! An opened `.h5p` file is checked as hostile input (size, entry count, paths, file types,
//! duplicate and overlapping entries) and kept as a file in `<data>/attachments`, never in the
//! database. Quizzes made of the content types AIS.ChatCraft knows become editable tasks;
//! everything else (Interactive Video, Course Presentation …) is shown with what it is and
//! saved back unchanged. An edited import keeps the package's other files (pictures,
//! libraries) and its library versions.

use std::path::Path;

use aiscraft_core::h5p::{self, BuildOptions, H5p, PackageInfo};
use aiscraft_core::worksheet::{self, Task};
use aiscraft_core::{Artifact, ArtifactKind, Error, Result, ToolSpec};
use serde_json::{Value, json};

use crate::artifacts::{doc_replace, doc_text, get_artifact, save_state, state_of};
use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel, now_ms};

/// The largest package opened or written (the attachment limit).
pub const MAX_PACKAGE_BYTES: u64 = crate::files::MAX_FILE_BYTES;
/// What all entries of a package may expand to together.
pub const MAX_UNPACKED_BYTES: u64 = 200 * 1024 * 1024;
/// Most entries a package may have (libraries bring many small files).
pub const MAX_ENTRIES: u64 = 5000;
const MAX_NAME: usize = 255;
/// File types H5P itself accepts in content (H5P core's default whitelist, without Flash).
const CONTENT_TYPES: &[&str] = &[
    "json", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "svg", "webp", "eot", "ttf", "woff", "woff2", "otf", "webm", "mp4", "ogg", "oga", "mp3", "m4a",
    "wav", "txt", "pdf", "rtf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "xml", "csv", "diff", "patch", "md", "textile", "vtt",
    "webvtt", "gltf", "glb",
];
/// Libraries also bring their code (never run by AIS.ChatCraft).
const LIBRARY_TYPES: &[&str] = &["js", "css"];
pub const H5P_INTRO: &str = "Erstelle ein H5P-Quiz zum Thema: ";

// ---- the package -----------------------------------------------------------------------------

fn zip_err(e: craft_zip::Error) -> Error {
    if e.is_limit() {
        Error::invalid(format!("Das H5P-Paket ist zu groß oder hat zu viele Dateien ({e})"))
    } else {
        Error::invalid(format!("Das ist kein lesbares H5P-Paket ({e})"))
    }
}

/// Files a packer adds that H5P ignores (`.DS_Store`, `__MACOSX/…`); left out when rewriting.
fn ignored(name: &str) -> bool {
    name.split('/').any(|part| part.starts_with('.') || part == "__MACOSX")
}

/// Why an entry name is refused, if it is: absolute, `..`, backslashes, control characters,
/// overlong, or a file type H5P does not allow.
fn bad_entry(name: &str, is_dir: bool) -> Option<String> {
    let shown: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    if name.is_empty() || name.len() > MAX_NAME {
        return Some(format!("Dateiname zu lang oder leer: {shown}"));
    }
    if name.starts_with('/') || name.contains('\\') || name.chars().any(char::is_control) || name.contains(':') {
        return Some(format!("unzulässiger Pfad: {shown}"));
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return Some(format!("unzulässiger Pfad: {shown}"));
    }
    if is_dir || ignored(name) {
        return None;
    }
    let file = trimmed.rsplit('/').next().unwrap_or(trimmed);
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let in_content = name.starts_with("content/");
    let ok = CONTENT_TYPES.contains(&ext.as_str()) || (!in_content && LIBRARY_TYPES.contains(&ext.as_str()));
    (!ok).then(|| format!("Dateityp nicht erlaubt: {shown}"))
}

/// A package that passed the checks.
#[derive(Debug, Clone)]
pub struct Package {
    pub h5p_json: Value,
    pub content: Value,
    /// `H5P.QuestionSet 1.20` (the version from `preloadedDependencies`, when listed).
    pub main_library: String,
    pub libraries: Vec<String>,
    /// Files (not directories) in the package.
    pub files: usize,
    pub title: String,
}

fn limits() -> craft_zip::Limits {
    craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: MAX_UNPACKED_BYTES }
}

fn lib_string(d: &Value) -> Option<String> {
    let name = d.get("machineName")?.as_str()?;
    if name.is_empty() || name.len() > 100 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-') {
        return None;
    }
    let num = |k: &str| d.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
    Some(match (num("majorVersion"), num("minorVersion")) {
        (Some(a), Some(b)) => format!("{name} {a}.{b}"),
        _ => name.to_string(),
    })
}

/// Checks and reads a package. Everything in it is hostile.
pub fn inspect(bytes: &[u8]) -> Result<Package> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(Error::invalid(format!("Das H5P-Paket ist größer als {} MB", MAX_PACKAGE_BYTES / (1024 * 1024))));
    }
    let mut zip = craft_zip::Archive::open(bytes, limits()).map_err(zip_err)?;
    if zip.has_duplicate_names() {
        return Err(Error::invalid("Das H5P-Paket enthält eine Datei doppelt"));
    }
    if zip.has_overlapping_files().map_err(zip_err)? {
        return Err(Error::invalid("Das H5P-Paket ist manipuliert (Dateien überlappen sich)"));
    }
    let mut files = 0;
    for i in 0..zip.len() {
        let name = zip.entry_name(i).map_err(zip_err)?.unwrap_or_default();
        if zip.entry_is_symlink(i).map_err(zip_err)?.unwrap_or(false) {
            return Err(Error::invalid("Das H5P-Paket enthält einen Verweis auf eine andere Datei (Symlink)"));
        }
        let is_dir = zip.entry_is_dir(i).map_err(zip_err)?.unwrap_or(false);
        if let Some(why) = bad_entry(&name, is_dir) {
            return Err(Error::invalid(format!("Das H5P-Paket wird nicht geöffnet: {why}")));
        }
        if !is_dir {
            files += 1;
        }
    }
    let read_json = |zip: &mut craft_zip::Archive, name: &str| -> Result<Value> {
        let raw = zip.read(name, h5p::MAX_CONTENT_BYTES as u64).map_err(zip_err)?.ok_or_else(|| Error::invalid(format!("Im H5P-Paket fehlt {name}")))?;
        // Some editors write a byte order mark.
        let text = String::from_utf8_lossy(&raw);
        serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| Error::invalid(format!("{name} im H5P-Paket ist kein gültiges JSON: {e}")))
    };
    let h5p_json = read_json(&mut zip, "h5p.json")?;
    let content = read_json(&mut zip, "content/content.json")?;
    let main = h5p_json
        .get("mainLibrary")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| Error::invalid("h5p.json nennt keine Hauptbibliothek (mainLibrary)"))?;
    let libraries: Vec<String> =
        h5p_json.get("preloadedDependencies").and_then(Value::as_array).map(|a| a.iter().take(200).filter_map(lib_string).collect()).unwrap_or_default();
    let main_library = libraries.iter().find(|l| l.split_whitespace().next() == Some(main)).cloned().unwrap_or_else(|| main.chars().take(100).collect());
    let title = h5p_json.get("title").and_then(Value::as_str).unwrap_or("").trim().chars().take(200).collect();
    Ok(Package { h5p_json, content, main_library, libraries, files, title })
}

/// Writes a package: `h5p.json`, `content/content.json` and, from `base`, every other file of
/// an imported package (pictures, libraries) unchanged.
pub fn write_package(h5p_json: &Value, content: &Value, base: Option<&[u8]>) -> Result<Vec<u8>> {
    let werr = |e: craft_zip::Error| Error::storage(format!("H5P-Paket ließ sich nicht schreiben: {e}"));
    let mut w = craft_zip::Writer::new();
    let meta = serde_json::to_vec_pretty(h5p_json)?;
    let body = serde_json::to_vec(content)?;
    w.add("h5p.json", &meta, craft_zip::Compression::Deflated).map_err(werr)?;
    w.add("content/content.json", &body, craft_zip::Compression::Deflated).map_err(werr)?;
    if let Some(base) = base {
        // Checked again: the stored file could have been replaced.
        inspect(base)?;
        let mut zip = craft_zip::Archive::open(base, limits()).map_err(zip_err)?;
        for i in 0..zip.len() {
            let name = zip.entry_name(i).map_err(zip_err)?.unwrap_or_default();
            if zip.entry_is_dir(i).map_err(zip_err)?.unwrap_or(false) || ignored(&name) || name == "h5p.json" || name == "content/content.json" {
                continue;
            }
            let data = zip.read_index(i, MAX_PACKAGE_BYTES).map_err(zip_err)?.unwrap_or_default();
            w.add(&name, &data, craft_zip::Compression::Deflated).map_err(werr)?;
        }
    }
    let out = w.finish().map_err(werr)?;
    if out.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(Error::invalid(format!("Das H5P-Paket wäre größer als {} MB", MAX_PACKAGE_BYTES / (1024 * 1024))));
    }
    Ok(out)
}

/// `h5p.json` of an edited import: the original's fields (authors, license …) with our title,
/// main library and the union of both dependency lists.
fn merged_h5p_json(original: &Value, title: &str, built: &h5p::Built) -> Value {
    let mut meta = original.clone();
    let ours = h5p::h5p_json(title, built);
    if let (Some(m), Some(o)) = (meta.as_object_mut(), ours.as_object()) {
        for key in ["title", "mainLibrary"] {
            if let Some(v) = o.get(key) {
                m.insert(key.into(), v.clone());
            }
        }
        let mut deps: Vec<Value> = o.get("preloadedDependencies").and_then(Value::as_array).cloned().unwrap_or_default();
        for d in original.get("preloadedDependencies").and_then(Value::as_array).into_iter().flatten().take(200) {
            let name = d.get("machineName").and_then(Value::as_str);
            if name.is_some() && !deps.iter().any(|x| x.get("machineName").and_then(Value::as_str) == name) {
                deps.push(d.clone());
            }
        }
        m.insert("preloadedDependencies".into(), Value::Array(deps));
    } else {
        return ours;
    }
    meta
}

fn package_path(c: &Ctx, p: &PackageInfo) -> Result<std::path::PathBuf> {
    if !h5p::is_package_file_name(&p.file) {
        return Err(Error::refused("Das H5P-Paket liegt nicht im Anhänge-Ordner"));
    }
    Ok(c.0.attachments_dir.join(&p.file))
}

fn read_package_file(c: &Ctx, p: &PackageInfo) -> Result<Vec<u8>> {
    let path = package_path(c, p)?;
    let meta = std::fs::metadata(&path).map_err(|_| Error::not_found("Das H5P-Paket gibt es nicht mehr (Anhänge-Ordner)"))?;
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(Error::invalid("Das H5P-Paket ist zu groß"));
    }
    std::fs::read(&path).map_err(|e| Error::storage(format!("H5P-Paket: {e}")))
}

/// The `.h5p` bytes of an artifact's content, and notes on what did not come across.
pub(crate) fn package_bytes(c: &Ctx, h: &H5p) -> Result<(Vec<u8>, Vec<String>)> {
    let Some(pkg) = &h.package else {
        let built = h5p::build(h).map_err(Error::invalid)?;
        let bytes = write_package(&h5p::h5p_json(&h.title, &built), &built.content, None)?;
        return Ok((bytes, built.notes));
    };
    let original = read_package_file(c, pkg)?;
    if !pkg.editable || !h5p::is_edited(h) {
        return Ok((original, Vec::new()));
    }
    let old = inspect(&original)?;
    let versions = old
        .libraries
        .iter()
        .filter_map(|l| {
            let (name, ver) = l.split_once(' ')?;
            let (a, b) = ver.split_once('.')?;
            Some((name.to_string(), a.parse().ok()?, b.parse().ok()?))
        })
        .collect();
    let force_set = old.main_library.split_whitespace().next() == Some(h5p::QUESTION_SET.machine);
    let built = h5p::build_with(h, &BuildOptions { force_set, versions }).map_err(Error::invalid)?;
    let bytes = write_package(&merged_h5p_json(&old.h5p_json, &h.title, &built), &built.content, Some(&original))?;
    Ok((bytes, built.notes))
}

// ---- artifacts -------------------------------------------------------------------------------

pub(crate) fn h5p_of(c: &Ctx, id: &str) -> Result<(Artifact, H5p, Vec<u8>)> {
    let a = get_artifact(c, id)?;
    if a.kind != ArtifactKind::H5p {
        return Err(Error::invalid("Das ist kein H5P-Inhalt"));
    }
    let state = state_of(c, id)?;
    let (h, _) = h5p::parse_stored(&doc_text(&state)?).map_err(|e| Error::invalid(format!("H5P-Inhalt beschädigt: {e}")))?;
    Ok((a, h, state))
}

pub(crate) fn save_h5p(c: &Ctx, id: &str, base: &[u8], h: &H5p, note: &str) -> Result<()> {
    save_state(c, id, &doc_replace(base, &h5p::to_json(h))?, Some(note))
}

pub(crate) fn create_h5p(c: &Ctx, h: &H5p, title: Option<String>, conversation: Option<String>) -> Result<String> {
    let mut h = h.clone();
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        h.title = t.chars().take(200).collect();
    }
    super::insert_artifact(c, ArtifactKind::H5p, &h.title.clone(), &h5p::to_json(&h), conversation)
}

/// The ```` ```aiscraft-h5p ```` blocks in an answer.
pub fn h5p_blocks(answer: &str) -> Vec<&str> {
    super::fenced_blocks(answer).into_iter().filter(|(l, _)| l == h5p::FENCE).map(|(_, b)| b).collect()
}

/// The request for H5P content.
pub fn h5p_prompt(topic: &str, count: usize, tools: bool) -> String {
    let deliver = if tools {
        "Lege den Inhalt mit dem Werkzeug h5p_create an und gib das JSON nicht zusätzlich im Chat aus."
    } else {
        "Antworte mit genau einem ```aiscraft-h5p-Codeblock (gültiges JSON) und danach höchstens zwei Sätzen."
    };
    format!(
        "{H5P_INTRO}{topic}\n\nEtwa {count} Aufgaben, gemischt aus Multiple Choice, Richtig oder falsch, Lückentext und Wörterziehen, vom Leichten zum Schweren, jede mit kurzer Rückmeldung (feedback). Deutsch, Du-Form.\n\n{}\n\n{deliver}",
        h5p::prompt_schema()
    )
}

fn repair_system() -> String {
    format!(
        "Du reparierst H5P-Inhalt im JSON-Format aiscraft-h5p. Behalte Inhalt und Aufgaben bei, behebe nur die genannten Probleme und verwende nur die erlaubten Aufgabentypen. Antworte nur mit dem korrigierten JSON in einem ```aiscraft-h5p-Codeblock.\n\n{}",
        h5p::prompt_schema()
    )
}

/// Parse, and when that fails or tasks were lost, ask the model once to repair it.
async fn parse_or_repair(c: &Ctx, src: &str, repair: bool) -> Result<(H5p, Vec<String>)> {
    let first = h5p::parse_lenient(src);
    let needs = match &first {
        Err(_) => true,
        Ok((_, problems)) => !problems.is_empty(),
    };
    if !repair || !needs {
        return first.map_err(Error::invalid);
    }
    let problems = match &first {
        Err(e) => vec![e.clone()],
        Ok((_, p)) => p.clone(),
    };
    let capped: String = src.chars().take(h5p::MAX_JSON_BYTES).collect();
    let prompt = format!("Probleme:\n- {}\n\n<h5p>\n{capped}\n</h5p>", problems.join("\n- "));
    match crate::chat::complete(c, Some(&repair_system()), &prompt).await {
        Ok(out) => {
            let block = h5p_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
            match (h5p::parse_lenient(&block), first) {
                (Ok(fixed), Ok(orig)) => Ok(if fixed.1.len() <= orig.1.len() { fixed } else { orig }),
                (Ok(fixed), Err(_)) => Ok(fixed),
                (Err(_), Ok(orig)) => Ok(orig),
                (Err(_), Err(e)) => Err(Error::invalid(e)),
            }
        }
        Err(_) => first.map_err(Error::invalid),
    }
}

/// Edits one task of a list (shared by worksheets and H5P): change, add, remove or move.
/// Returns the version note.
pub(crate) fn apply_task_edit(tasks: &mut Vec<Task>, p: &Value) -> Result<String> {
    let index = p.get("index").and_then(Value::as_u64).map(|i| i as usize);
    if p.get("remove").and_then(Value::as_bool).unwrap_or(false) {
        let i = index.filter(|i| *i < tasks.len()).ok_or_else(|| Error::invalid("index fehlt oder gibt es nicht"))?;
        tasks.remove(i);
        return Ok(format!("Aufgabe {} entfernt", i + 1));
    }
    if let Some(to) = p.get("move_to").and_then(Value::as_u64) {
        let i = index.filter(|i| *i < tasks.len()).ok_or_else(|| Error::invalid("index fehlt oder gibt es nicht"))?;
        let to = (to as usize).min(tasks.len().saturating_sub(1));
        let t = tasks.remove(i);
        tasks.insert(to, t);
        return Ok(format!("Aufgabe {} verschoben", i + 1));
    }
    let tv = p.get("task").ok_or_else(|| Error::invalid("task fehlt"))?;
    let (one, _) = worksheet::from_value(&json!({"tasks": [tv]})).map_err(Error::invalid)?;
    let mut task = one.tasks.into_iter().next().ok_or_else(|| Error::invalid("Die Aufgabe ließ sich nicht lesen"))?;
    match index {
        Some(i) if i < tasks.len() => {
            if let Some(slot) = tasks.get_mut(i) {
                if tv.get("id").is_none() {
                    task.id = slot.id.clone();
                }
                *slot = task;
            }
            Ok(format!("Aufgabe {} geändert", i + 1))
        }
        Some(_) => Err(Error::invalid("index gibt es nicht")),
        None => {
            if tasks.len() >= worksheet::MAX_TASKS {
                return Err(Error::invalid(format!("Höchstens {} Aufgaben", worksheet::MAX_TASKS)));
            }
            task.id = String::new();
            tasks.push(task);
            Ok("Aufgabe ergänzt".into())
        }
    }
}

fn normalize(h: &mut H5p) {
    let mut ws = h5p::as_worksheet(h);
    worksheet::normalize_ids(&mut ws);
    h.tasks = ws.tasks;
}

fn write_out(c: &Ctx, bytes: &[u8], path: Option<String>) -> Result<Value> {
    if let Some(path) = path {
        crate::disk::write(Path::new(&path), bytes)?;
        c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
        return Ok(json!({"path": path, "bytes": bytes.len()}));
    }
    use base64::Engine as _;
    Ok(json!({"base64": base64::engine::general_purpose::STANDARD.encode(bytes), "bytes": bytes.len()}))
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "h5p.create",
        label: "Als H5P öffnen",
        group: "H5P",
        shortcut: None,
        params: "content?: string (aiscraft-h5p-JSON oder eine Antwort mit dem Block), h5p?: object, from_message?: string, title?: string, repair?: bool, id?: string (diesen H5P-Inhalt ersetzen, das Paket bleibt)",
        palette: false,
        run: |c, p| Box::pin(create_cmd(c, p)),
    },
    CommandSpec {
        id: "h5p.request",
        label: "H5P-Quiz erstellen lassen",
        group: "H5P",
        shortcut: None,
        params: "topic: string, subject?: string, grade?: string, count?: number",
        palette: false,
        run: |c, p| Box::pin(request(c, p)),
    },
    CommandSpec {
        id: "h5p.open",
        label: "H5P-Datei öffnen",
        group: "H5P",
        shortcut: None,
        params: "path: string (eine .h5p-Datei)",
        palette: false,
        run: |c, p| Box::pin(open_file(c, p)),
    },
    CommandSpec {
        id: "h5p.update_task",
        label: "H5P-Aufgabe ändern",
        group: "H5P",
        shortcut: None,
        params: "id: string, index?: number (ohne: neue Aufgabe am Ende), task?: object, remove?: bool, move_to?: number",
        palette: false,
        run: |c, p| Box::pin(update_task(c, p)),
    },
    CommandSpec {
        id: "h5p.export",
        label: "Als H5P speichern",
        group: "H5P",
        shortcut: None,
        params: "id: string, path?: string (sonst base64)",
        palette: false,
        run: |c, p| Box::pin(export(c, p)),
    },
    CommandSpec {
        id: "h5p.validate",
        label: "H5P-Inhalt prüfen",
        group: "H5P",
        shortcut: None,
        params: "content: string",
        palette: false,
        run: |_c, p| {
            Box::pin(async move {
                let content = req_str(&p, "content")?;
                Ok(match h5p::parse_lenient(&content) {
                    Ok((h, mut problems)) => {
                        problems.extend(h5p::validate(&h));
                        json!({"ok": problems.is_empty(), "tasks": h.tasks.len(), "problems": problems, "h5p": h})
                    }
                    Err(e) => json!({"ok": false, "tasks": 0, "problems": [e]}),
                })
            })
        },
    },
];

pub(crate) async fn create_cmd(c: Ctx, p: Value) -> Result<Value> {
    let repair = p.get("repair").and_then(Value::as_bool).unwrap_or(false);
    let (h, problems) = if let Some(obj) = p.get("h5p").filter(|v| v.is_object()) {
        h5p::from_value(obj).map_err(Error::invalid)?
    } else {
        let src = match opt_str(&p, "from_message") {
            Some(mid) => super::message_text(&c, &mid)?,
            None => req_str(&p, "content")?,
        };
        let block = h5p_blocks(&src).into_iter().next().map(str::to_string).unwrap_or(src);
        parse_or_repair(&c, &block, repair).await?
    };
    if let Some(id) = opt_str(&p, "id") {
        let (_, old, state) = h5p_of(&c, &id)?;
        if old.package.as_ref().is_some_and(|pk| !pk.editable) {
            return Err(Error::unsupported("Dieses H5P-Paket kann AIS.ChatCraft nur unverändert speichern"));
        }
        let mut new = H5p { package: old.package, ..h };
        normalize(&mut new);
        save_h5p(&c, &id, &state, &new, "Aus einer Antwort übernommen")?;
        return Ok(json!({"id": id, "tasks": new.tasks.len(), "problems": problems}));
    }
    let id = create_h5p(&c, &h, opt_str(&p, "title"), None)?;
    if !problems.is_empty() {
        c.notice(NoticeLevel::Warning, format!("H5P-Inhalt geöffnet, bitte prüfen: {}", problems.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
    }
    Ok(json!({"id": id, "tasks": h.tasks.len(), "problems": problems}))
}

async fn request(c: Ctx, p: Value) -> Result<Value> {
    let topic = super::topic_line(&p)?;
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(6).clamp(1, 30) as usize;
    let tools = super::tools_on(&c);
    let since = now_ms();
    crate::chat::send_with(c.clone(), h5p_prompt(&topic, count, tools), vec![], vec![], None, false).await?;
    if let Some(id) = super::made_by_tool(&c, ArtifactKind::H5p, since) {
        return Ok(json!({"id": id, "created": true}));
    }
    let answer = super::last_answer(&c);
    let Some(block) = h5p_blocks(&answer).into_iter().next().map(str::to_string) else {
        c.notice(NoticeLevel::Warning, "Die Antwort enthält keinen H5P-Inhalt. Bitte das Modell, ihn als ```aiscraft-h5p-Block auszugeben.");
        return Ok(json!({"created": false}));
    };
    let (h, problems) = parse_or_repair(&c, &block, true).await?;
    let id = create_h5p(&c, &h, None, None)?;
    Ok(json!({"id": id, "created": true, "tasks": h.tasks.len(), "problems": problems}))
}

/// Opens a `.h5p` file: checked, kept in the attachments folder, shown as an artifact.
async fn open_file(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let meta = std::fs::metadata(&path).map_err(|_| Error::not_found(format!("Datei nicht gefunden: {path}")))?;
    if !meta.is_file() {
        return Err(Error::invalid("Das ist keine Datei"));
    }
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(Error::invalid(format!("Die H5P-Datei ist größer als {} MB", MAX_PACKAGE_BYTES / (1024 * 1024))));
    }
    let bytes = std::fs::read(&path).map_err(|e| Error::storage(format!("{path}: {e}")))?;
    let pkg = inspect(&bytes)?;
    let stem = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("H5P").to_string();
    let title = if pkg.title.is_empty() { stem } else { pkg.title.clone() };
    let copy = crate::files::keep_copy(&c.0.attachments_dir, Path::new("paket.h5p"), &bytes)?;
    let file = copy.file_name().and_then(|f| f.to_str()).unwrap_or("").to_string();
    let mut info = PackageInfo {
        file,
        main_library: pkg.main_library.clone(),
        libraries: pkg.libraries.clone(),
        files: pkg.files,
        bytes: meta.len(),
        ..PackageInfo::default()
    };
    let mut h = H5p { title: title.chars().take(200).collect(), ..H5p::default() };
    let supported = match h5p::read_content(&pkg.main_library, &pkg.content) {
        Ok((intro, tasks, notes)) if !tasks.is_empty() => {
            h.intro = intro;
            h.tasks = tasks;
            info.editable = true;
            info.notes = notes;
            true
        }
        Ok(_) => {
            info.notes = vec!["Keine Aufgaben gefunden".into()];
            false
        }
        Err(why) => {
            info.notes = vec![why];
            false
        }
    };
    info.fingerprint = h5p::fingerprint(&h);
    h.package = Some(info);
    let id = create_h5p(&c, &h, None, None)?;
    if !supported {
        c.notice(NoticeLevel::Info, format!("{}: AIS.ChatCraft zeigt, was im Paket ist, und speichert es unverändert.", pkg.main_library));
    }
    Ok(json!({"id": id, "editable": supported, "main_library": pkg.main_library, "tasks": h.tasks.len(), "files": pkg.files}))
}

async fn update_task(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (_, mut h, state) = h5p_of(&c, &id)?;
    if h.package.as_ref().is_some_and(|pk| !pk.editable) {
        return Err(Error::unsupported("Dieses H5P-Paket kann AIS.ChatCraft nur unverändert speichern"));
    }
    if let Some(tv) = p.get("task")
        && let Some(kind) = tv.get("type").and_then(Value::as_str)
        && let Some(body) = worksheet::from_value(&json!({"tasks": [tv]})).ok().and_then(|(w, _)| w.tasks.into_iter().next()).map(|t| t.body)
        && let Err(why) = h5p::library_for(&body)
    {
        return Err(Error::invalid(format!("{kind}: {why}")));
    }
    let note = apply_task_edit(&mut h.tasks, &p)?;
    normalize(&mut h);
    save_h5p(&c, &id, &state, &h, &note)?;
    Ok(json!({"tasks": h.tasks.len(), "problems": h5p::validate(&h)}))
}

async fn export(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (_, h, _) = h5p_of(&c, &id)?;
    let (bytes, notes) = package_bytes(&c, &h)?;
    if !notes.is_empty() && opt_str(&p, "path").is_some() {
        c.notice(NoticeLevel::Info, format!("Als H5P gespeichert. Hinweis: {}", notes.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
    }
    let mut out = write_out(&c, &bytes, opt_str(&p, "path"))?;
    if let Some(o) = out.as_object_mut() {
        o.insert("notes".into(), json!(notes));
    }
    Ok(out)
}

/// `artifact.export` for H5P content (`h5p`, or `json` for the editable content).
pub(crate) fn export_bytes(c: &Ctx, a: &Artifact, format: Option<&str>) -> Result<Vec<u8>> {
    let (_, h, _) = h5p_of(c, &a.id)?;
    match format.unwrap_or("h5p") {
        "h5p" => package_bytes(c, &h).map(|(b, _)| b),
        "json" => Ok(h5p::to_json(&h).into_bytes()),
        other => Err(Error::invalid(format!("Format {other} gibt es für H5P nicht (h5p, json)"))),
    }
}

/// What a model without tools is told when H5P content goes along as context.
pub(crate) const CHANGE_NOTE: &str =
    "\n\n(Wenn du diesen H5P-Inhalt ändern sollst, gib die vollständige neue Fassung im selben JSON-Format in einem ```aiscraft-h5p-Codeblock aus.)";

/// `artifact.revise` for H5P content: the system prompt and the parsed answer.
pub(crate) fn revise_system() -> String {
    format!(
        "Du überarbeitest H5P-Inhalt im JSON-Format aiscraft-h5p nach der Anweisung der Person. Verwende nur die erlaubten Aufgabentypen. Antworte nur mit dem vollständigen, überarbeiteten JSON in einem ```aiscraft-h5p-Codeblock.\n\n{}",
        h5p::prompt_schema()
    )
}

pub(crate) async fn revised(c: &Ctx, id: &str, out: String) -> Result<String> {
    let (_, old, _) = h5p_of(c, id)?;
    if old.package.as_ref().is_some_and(|pk| !pk.editable) {
        return Err(Error::unsupported("Dieses H5P-Paket kann AIS.ChatCraft nur unverändert speichern"));
    }
    let block = h5p_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
    let (h, _) = parse_or_repair(c, &block, true).await?;
    let mut new = H5p { package: old.package, ..h };
    normalize(&mut new);
    Ok(h5p::to_json(&new))
}

// ---- model tools ----------------------------------------------------------------------------

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "h5p_create".into(),
            description: format!(
                "Legt H5P-Inhalt an (ein Quiz für Moodle, Mebis, ByCS und andere Lernplattformen), den Lehrkräfte in AIS.ChatCraft bearbeiten und als .h5p-Datei speichern. Nur nehmen, wenn H5P gewünscht ist; sonst worksheet_create.\n{}",
                h5p::prompt_schema()
            ),
            input_schema: json!({"type": "object", "properties": {
                "h5p": {"type": "object", "description": "der Inhalt im Format aiscraft-h5p (title, intro, tasks)"}
            }, "required": ["h5p"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "h5p_update".into(),
            description: "Ersetzt H5P-Inhalt durch eine neue, vollständige Fassung im selben Format (die alte bleibt als Version erhalten).".into(),
            input_schema: json!({"type": "object", "properties": {"id": {"type": "string"}, "h5p": {"type": "object"}}, "required": ["id", "h5p"], "additionalProperties": false}),
        },
    ]
}

fn parse_tool_value(v: &Value) -> std::result::Result<(H5p, Vec<String>), String> {
    match v {
        Value::String(src) => h5p::parse_lenient(src),
        _ => h5p::from_value(v),
    }
}

/// Runs one of the tools above; `None` when `name` is none of them.
pub(crate) fn run_tool(c: &Ctx, name: &str, a: &Value) -> Option<Result<String>> {
    Some(match name {
        "h5p_create" => {
            let v = a.get("h5p").cloned().unwrap_or_else(|| a.clone());
            parse_tool_value(&v).map_err(Error::invalid).and_then(|(h, problems)| {
                let id = create_h5p(c, &h, None, None)?;
                let mut out = format!("H5P-Inhalt „{}“ mit {} Aufgaben angelegt und geöffnet (ID {id}).", h.title, h.tasks.len());
                if !problems.is_empty() {
                    out.push_str(&format!(" Probleme, bitte mit h5p_update beheben: {}", problems.join("; ")));
                }
                Ok(out)
            })
        }
        "h5p_update" => (|| {
            let id = a.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let (_, old, state) = h5p_of(c, &id)?;
            if old.package.as_ref().is_some_and(|pk| !pk.editable) {
                return Err(Error::unsupported("Dieses H5P-Paket kann AIS.ChatCraft nur unverändert speichern"));
            }
            let (h, problems) = parse_tool_value(a.get("h5p").unwrap_or(&Value::Null)).map_err(Error::invalid)?;
            let mut new = H5p { package: old.package, ..h };
            normalize(&mut new);
            save_h5p(c, &id, &state, &new, "Vom Modell geändert")?;
            Ok(if problems.is_empty() { "H5P-Inhalt aktualisiert.".to_string() } else { format!("H5P-Inhalt aktualisiert. Probleme: {}", problems.join("; ")) })
        })(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = craft_zip::Writer::new();
        for (n, d) in entries {
            w.add(n, d, craft_zip::Compression::Deflated).unwrap();
        }
        w.finish().unwrap()
    }

    const META: &[u8] =
        br#"{"title":"T","mainLibrary":"H5P.TrueFalse","preloadedDependencies":[{"machineName":"H5P.TrueFalse","majorVersion":"1","minorVersion":"6"}]}"#;
    const CONTENT: &[u8] = br#"{"question":"<p>Ja?</p>","correct":"false"}"#;

    #[test]
    fn hostile_packages_are_refused() {
        let ok = zip(&[("h5p.json", META), ("content/content.json", CONTENT), ("content/images/a.png", b"png"), ("H5P.TrueFalse-1.6/x.js", b"js")]);
        let p = inspect(&ok).unwrap();
        assert_eq!(p.main_library, "H5P.TrueFalse 1.6");
        assert_eq!(p.files, 4);
        for (name, why) in [
            ("../evil.json", "Pfad"),
            ("/abs.json", "Pfad"),
            ("content/a\\b.json", "Pfad"),
            ("content/x.exe", "Dateityp"),
            ("content/run.js", "Dateityp"),
            ("C:/x.json", "Pfad"),
            ("content//x.json", "Pfad"),
            ("ohne-endung", "Dateityp"),
        ] {
            let bad = zip(&[("h5p.json", META), ("content/content.json", CONTENT), (name, b"x")]);
            let e = inspect(&bad).unwrap_err();
            assert!(e.message.contains(why), "{name}: {}", e.message);
        }
        // Hidden packer files are fine and left out when rewriting.
        let mac = zip(&[("h5p.json", META), ("content/content.json", CONTENT), ("__MACOSX/._x", b"x"), ("content/.DS_Store", b"x")]);
        assert!(inspect(&mac).is_ok());
        let rewritten = write_package(&json!({"title": "x", "mainLibrary": "H5P.TrueFalse"}), &json!({}), Some(&mac)).unwrap();
        assert_eq!(inspect(&rewritten).unwrap().files, 2);
        assert!(inspect(b"PK\x03\x04 kaputt").is_err());
        assert!(inspect(&zip(&[("h5p.json", META)])).unwrap_err().message.contains("content/content.json"));
        assert!(inspect(&zip(&[("h5p.json", b"{}"), ("content/content.json", CONTENT)])).unwrap_err().message.contains("mainLibrary"));
        assert!(inspect(&zip(&[("h5p.json", b"nope"), ("content/content.json", CONTENT)])).is_err());
        // A bomb: one entry expanding past what a content.json may be.
        let big = vec![b' '; h5p::MAX_CONTENT_BYTES + 10];
        assert!(inspect(&zip(&[("h5p.json", META), ("content/content.json", &big)])).is_err());
        // Too many entries.
        let many: Vec<(String, Vec<u8>)> = (0..=MAX_ENTRIES).map(|i| (format!("content/{i}.json"), Vec::new())).collect();
        let refs: Vec<(&str, &[u8])> = many.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
        assert!(inspect(&zip(&refs)).unwrap_err().message.contains("zu viele"));
        assert!(inspect(&vec![0u8; MAX_PACKAGE_BYTES as usize + 1]).is_err());
    }

    #[test]
    fn written_packages_reopen() {
        let (h, _) = h5p::from_value(&json!({"title": "Quiz", "tasks": [{"type": "true_false", "prompt": "Ja?", "answer": true}, {"type": "choice", "prompt": "Welche?", "options": [{"text": "a", "correct": true}, {"text": "b"}]}]})).unwrap();
        let b = h5p::build(&h).unwrap();
        let bytes = write_package(&h5p::h5p_json(&h.title, &b), &b.content, None).unwrap();
        let p = inspect(&bytes).unwrap();
        assert_eq!(p.main_library, "H5P.QuestionSet 1.20");
        assert_eq!(p.libraries, vec!["H5P.QuestionSet 1.20", "H5P.TrueFalse 1.8", "H5P.MultiChoice 1.16"]);
        assert_eq!(p.title, "Quiz");
        let (_, tasks, _) = h5p::read_content(&p.main_library, &p.content).unwrap();
        assert_eq!(tasks.len(), 2);
        let merged = merged_h5p_json(
            &json!({"title": "alt", "authors": [{"name": "A"}], "preloadedDependencies": [{"machineName": "FontAwesome", "majorVersion": 4, "minorVersion": 5}]}),
            "neu",
            &b,
        );
        assert_eq!(merged["title"], "neu");
        assert_eq!(merged["authors"][0]["name"], "A");
        assert_eq!(merged["preloadedDependencies"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn task_edits() {
        let mut tasks = Vec::new();
        assert!(apply_task_edit(&mut tasks, &json!({"task": {"type": "true_false", "prompt": "a", "answer": true}})).is_ok());
        assert!(apply_task_edit(&mut tasks, &json!({"task": {"type": "true_false", "prompt": "b", "answer": true}})).is_ok());
        assert_eq!(apply_task_edit(&mut tasks, &json!({"index": 1, "move_to": 0})).unwrap(), "Aufgabe 2 verschoben");
        assert_eq!(tasks[0].prompt, "b");
        assert!(apply_task_edit(&mut tasks, &json!({"index": 9, "remove": true})).is_err());
        assert!(apply_task_edit(&mut tasks, &json!({"index": 9, "task": {"type": "true_false", "prompt": "c", "answer": true}})).is_err());
        assert!(apply_task_edit(&mut tasks, &json!({})).is_err());
        assert!(h5p_prompt("Igel", 4, false).contains("```aiscraft-h5p") && h5p_prompt("Igel", 4, true).contains("h5p_create"));
    }
}
