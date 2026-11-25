//! Project knowledge (#32): files (curricula, worksheets, PDFs) in a project, used by every chat
//! in it, as Claude Projects do. The text is extracted on the device, cut into parts and indexed
//! (FTS5). Before an answer, the parts that fit the request go along: everything when the files
//! are small, else the best hits, within a budget that follows the model's context window.
//!
//! Claude (API) gets them as documents with citations on (#39), so the answer can name the
//! passage; other services get them as text in front of the message. „Namen schützen“ (#55)
//! applies to both, as the parts pass the chat's guard. Services that keep the chat on their
//! server (AIS.chat) get each part once per chat.

use std::path::Path;

use aiscraft_core::{Conversation, Error, ProjectFile, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::req_str;
use crate::{Ctx, new_id, now_ms};

/// Files per project.
pub const MAX_FILES: usize = 50;
/// Characters of one part.
const CHUNK: usize = 1500;
/// Parts of a chat remembered as sent (services with a server history).
const MAX_SENT_KEYS: usize = 400;
/// Budget when the model's context window is unknown, in characters (about 15 000 tokens).
const DEFAULT_BUDGET: usize = 60_000;
/// The label on a message that took project knowledge along.
pub const KNOWLEDGE: &str = "Projektwissen";

/// Knowledge of one project as the settings show it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// About this many tokens if everything went along (characters / 4).
    pub tokens: u64,
    /// The model's context window, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    /// Share of the window the whole knowledge takes, 0–100.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u64>,
    /// Everything goes along with every message (else only fitting parts).
    pub whole: bool,
}

pub(crate) fn load(c: &Ctx) -> Vec<ProjectFile> {
    c.store(|st| st.project_files()).unwrap_or_default()
}

/// The text of a file on the device: PDFs through craft-pdf, office files, e-books, mails and
/// sheets as for an AIS.chat upload (#45), text as it is.
fn extract(path: &Path) -> Result<(String, String, u64)> {
    let meta = std::fs::metadata(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{} ist keine Datei", path.display())));
    }
    if meta.len() > crate::files::MAX_FILE_BYTES {
        return Err(Error::invalid(format!("{} ist größer als 25 MB", path.display())));
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Datei").chars().take(160).collect::<String>();
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let text = if ext == "pdf" {
        let bytes = std::fs::read(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
        let t = crate::attach::pdf_text::text(&bytes)?;
        if t.trim().is_empty() {
            return Err(Error::unsupported(format!("In {name} steht kein lesbarer Text (eingescannte Seiten?). Lege eine Fassung mit Text ab.")));
        }
        t
    } else {
        let p = crate::attach::prepare(path, "", crate::attach::Target::AisChat)?;
        if !crate::files::is_text(&p.mime) {
            return Err(Error::unsupported(format!("{name}: Projektwissen liest Text, PDF und Office-Dateien, keine Bilder")));
        }
        String::from_utf8_lossy(&p.bytes).to_string()
    };
    let text: String = text.chars().take(crate::attach::pdf_text::MAX_CHARS).collect();
    Ok((name, text, meta.len()))
}

/// Parts of about [`CHUNK`] characters, cut at paragraphs, else at lines or spaces.
pub(crate) fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        if cur.chars().count() + para.chars().count() + 2 > CHUNK && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if para.chars().count() > CHUNK {
            // A long paragraph: cut at spaces.
            for word in para.split_inclusive(char::is_whitespace) {
                if cur.chars().count() + word.chars().count() > CHUNK && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push_str(word);
            }
        } else {
            if !cur.is_empty() {
                cur.push_str("\n\n");
            }
            cur.push_str(para);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn project_exists(c: &Ctx, id: &str) -> Result<()> {
    if c.state().projects.iter().any(|p| p.id == id) { Ok(()) } else { Err(Error::not_found("Dieses Projekt gibt es nicht")) }
}

/// `project.file_add`: put a file into a project's knowledge.
pub(crate) async fn add(c: Ctx, p: Value) -> Result<Value> {
    let project = req_str(&p, "project")?;
    project_exists(&c, &project)?;
    let path = req_str(&p, "path")?;
    if c.state().project_files.iter().filter(|f| f.project_id == project).count() >= MAX_FILES {
        return Err(Error::invalid(format!("Höchstens {MAX_FILES} Dateien pro Projekt")));
    }
    let src = std::path::PathBuf::from(&path);
    let (name, text, size) = tokio::task::spawn_blocking(move || extract(&src)).await.map_err(|e| Error::invalid(format!("Lesen abgebrochen: {e}")))??;
    let bytes = std::fs::read(&path).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    let copy = crate::files::keep_copy(&c.0.attachments_dir.join("projekte"), Path::new(&path), &bytes)?;
    let parts = chunks(&text);
    let file = ProjectFile {
        id: new_id(),
        project_id: project.clone(),
        mime: crate::files::mime_of(Path::new(&name)).to_string(),
        name,
        size,
        chars: text.chars().count() as u64,
        path: copy.display().to_string(),
        added_at: now_ms(),
    };
    c.store(|st| st.add_project_file(&file, &text, &parts))?;
    let out = json!({"id": file.id, "name": file.name, "chars": file.chars, "parts": parts.len()});
    c.update(|s| s.project_files.push(file));
    Ok(out)
}

/// `project.file_remove`.
pub(crate) fn remove(c: &Ctx, p: &Value) -> Result<Value> {
    let id = req_str(p, "id")?;
    let file = c.state().project_files.iter().find(|f| f.id == id).cloned().ok_or_else(|| Error::not_found("Diese Datei gibt es im Projekt nicht"))?;
    c.store(|st| st.delete_project_file(&id))?;
    let _ = std::fs::remove_file(&file.path);
    c.update(|s| s.project_files.retain(|f| f.id != id));
    Ok(json!({}))
}

/// The selected model's context window, when known.
fn window(c: &Ctx, backend: &str, model: &str) -> Option<u64> {
    c.state().backend(backend).and_then(|b| b.models.iter().find(|m| m.id == model)).and_then(|m| m.context_window).filter(|w| *w > 0)
}

/// Characters of knowledge that may go along with one message: a quarter of the context window.
fn budget(window: Option<u64>) -> usize {
    match window {
        Some(w) => usize::try_from(w).unwrap_or(usize::MAX).clamp(8_000, 400_000),
        None => DEFAULT_BUDGET,
    }
}

/// How much of the context a project's knowledge takes, for the selected model.
pub(crate) fn usage(c: &Ctx, project: &str) -> Usage {
    let s = c.state();
    let chars: u64 = s.project_files.iter().filter(|f| f.project_id == project).map(|f| f.chars).sum();
    let window = window(c, &s.selection.backend_id, &s.selection.model_id);
    let tokens = chars / 4;
    Usage { tokens, window, percent: window.map(|w| (tokens.saturating_mul(100) / w).min(999)), whole: chars as usize <= budget(window) }
}

/// `project.files`: a project's files and how much of the context they take.
pub(crate) fn list(c: &Ctx, p: &Value) -> Result<Value> {
    let project = req_str(p, "project")?;
    project_exists(c, &project)?;
    let files: Vec<ProjectFile> = c.state().project_files.iter().filter(|f| f.project_id == project).cloned().collect();
    Ok(json!({"files": files, "usage": usage(c, &project)}))
}

/// Knowledge for one request.
#[derive(Debug, Clone, Default)]
pub(crate) struct Picked {
    /// (file name, text) per file, the parts in their order.
    pub docs: Vec<(String, String)>,
    /// Keys of the parts (`file:idx`), for services that keep them in their history.
    pub keys: Vec<String>,
}

impl Picked {
    /// The text block for services without document blocks.
    pub fn block(&self) -> String {
        let body: Vec<String> = self.docs.iter().map(|(n, t)| format!("## {n}\n{t}")).collect();
        format!("[Projektwissen aus AIS.ChatCraft: Auszüge aus Dateien des Projekts]\n{}\n[Ende Projektwissen]", body.join("\n\n"))
    }
    pub fn names(&self) -> String {
        self.docs.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" · ")
    }
}

fn words(s: &str) -> Vec<String> {
    let mut w: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 3).map(str::to_lowercase).collect();
    w.sort();
    w.dedup();
    w
}

/// The knowledge that goes along with `request` in `conv`: none outside a project, with the source
/// switched off, or without files. `once` = the service keeps the chat (AIS.chat): parts sent
/// before in this chat are left out.
pub(crate) fn pick(c: &Ctx, conv: &Conversation, request: &str, once: bool) -> Option<Picked> {
    let project = conv.project_id.as_ref()?;
    if !conv.sources.knowledge || conv.temporary {
        return None;
    }
    let files: Vec<ProjectFile> = c.state().project_files.iter().filter(|f| &f.project_id == project).cloned().collect();
    if files.is_empty() {
        return None;
    }
    let budget = budget(window(c, &conv.backend_id, &conv.model_id));
    let total: usize = files.iter().map(|f| f.chars as usize).sum();
    // (file id, idx, text) in the order they go along.
    let mut parts: Vec<(String, i64, String)> = Vec::new();
    if total <= budget {
        for f in &files {
            if let Ok(Some(t)) = c.store(|st| st.project_file_text(&f.id)) {
                for (i, ch) in chunks(&t).into_iter().enumerate() {
                    parts.push((f.id.clone(), i as i64, ch));
                }
            }
        }
    } else {
        let hits = c.store(|st| st.search_knowledge(project, &words(request), 200)).unwrap_or_default();
        let mut used = 0;
        for h in hits {
            if used + h.text.len() > budget {
                break;
            }
            used += h.text.len();
            parts.push((h.file_id, h.idx, h.text));
        }
    }
    if once {
        parts.retain(|(f, i, _)| !conv.sources.knowledge_sent.contains(&format!("{f}:{i}")));
    }
    if parts.is_empty() {
        return None;
    }
    let mut picked = Picked::default();
    for f in &files {
        let mut mine: Vec<&(String, i64, String)> = parts.iter().filter(|(id, _, _)| id == &f.id).collect();
        if mine.is_empty() {
            continue;
        }
        mine.sort_by_key(|(_, i, _)| *i);
        let text = mine.iter().map(|(_, _, t)| t.as_str()).collect::<Vec<_>>().join("\n\n[…]\n\n");
        picked.keys.extend(mine.iter().map(|(id, i, _)| format!("{id}:{i}")));
        picked.docs.push((f.name.clone(), if mine.len() == 1 || total <= budget { text.replace("\n\n[…]\n\n", "\n\n") } else { text }));
    }
    Some(picked)
}

/// Remember the parts a service with a server history has now.
pub(crate) fn remember(sources: &mut aiscraft_core::Sources, keys: &[String]) {
    for k in keys {
        if !sources.knowledge_sent.contains(k) {
            sources.knowledge_sent.push(k.clone());
        }
    }
    let extra = sources.knowledge_sent.len().saturating_sub(MAX_SENT_KEYS);
    sources.knowledge_sent.drain(..extra);
}

/// The whole text of a project file named `name` in the chat's project (for citations, #39).
pub(crate) fn text_by_name(c: &Ctx, conv: &str, name: &str) -> Option<(String, String)> {
    let s = c.state();
    let project = s.conversation(conv)?.project_id.clone()?;
    let f = s.project_files.iter().find(|f| f.project_id == project && f.name == name)?;
    let text = c.store(|st| st.project_file_text(&f.id)).ok().flatten()?;
    Some((text, f.path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cut_at_paragraphs_and_long_runs() {
        let text = format!("Erster Absatz.\n\n{}\n\nLetzter.", "Wort ".repeat(800));
        let c = chunks(&text);
        assert!(c.len() >= 3, "{}", c.len());
        assert!(c.iter().all(|x| x.chars().count() <= CHUNK + 10));
        assert!(c[0].starts_with("Erster Absatz."));
        assert!(chunks("").is_empty() && chunks("\n\n\n").is_empty());
    }
}
