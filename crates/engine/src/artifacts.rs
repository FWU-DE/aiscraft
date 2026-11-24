//! Artifacts: Markdown documents and whiteboards next to the chat.
//!
//! Both are yrs (Yjs) CRDTs. A document is one `Text` named "content"; every edit is applied as
//! the smallest replace (common prefix and suffix kept), so the history stays mergeable when the
//! collab server arrives. A board is `aiscraft_board::Board`.

use aiscraft_board::Board;
use aiscraft_core::{Artifact, ArtifactKind, Error, Result, Role};
use base64::Engine as _;
use serde_json::{Value, json};
use yrs::updates::decoder::Decode;
use yrs::{GetString, ReadTxn, Text, Transact};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id, now_ms};

pub(crate) const MAX_STATE: usize = 32 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 2_000_000;

// ---- documents ------------------------------------------------------------------------------

fn doc_from_state(state: &[u8]) -> Result<yrs::Doc> {
    // UTF-16 offsets, as Yjs uses in the browser, so web clients can share these documents later.
    let doc = yrs::Doc::with_options(yrs::Options { offset_kind: yrs::OffsetKind::Utf16, ..yrs::Options::default() });
    if !state.is_empty() {
        let update = yrs::Update::decode_v1(state).map_err(|e| Error::invalid(format!("Dokument beschädigt: {e}")))?;
        doc.transact_mut().apply_update(update).map_err(|e| Error::invalid(format!("Dokument beschädigt: {e}")))?;
    }
    Ok(doc)
}

fn encode(doc: &yrs::Doc) -> Vec<u8> {
    doc.transact().encode_state_as_update_v1(&yrs::StateVector::default())
}

pub fn doc_text(state: &[u8]) -> Result<String> {
    let doc = doc_from_state(state)?;
    let text = doc.get_or_insert_text("content");
    Ok(text.get_string(&doc.transact()))
}

/// Replace the document's text with `new`, as one minimal edit.
pub fn doc_replace(state: &[u8], new: &str) -> Result<Vec<u8>> {
    let doc = doc_from_state(state)?;
    let text = doc.get_or_insert_text("content");
    {
        let mut txn = doc.transact_mut();
        let old = text.get_string(&txn);
        let (start, del, ins) = minimal_edit(&old, new);
        // Offsets in UTF-16 code units (the doc is created with OffsetKind::Utf16).
        let start16: u32 = old.chars().take(start).map(|c| c.len_utf16() as u32).sum();
        let del16: u32 = old.chars().skip(start).take(del).map(|c| c.len_utf16() as u32).sum();
        if del16 > 0 {
            text.remove_range(&mut txn, start16, del16);
        }
        if !ins.is_empty() {
            text.insert(&mut txn, start16, &ins);
        }
    }
    Ok(encode(&doc))
}

/// Both states in one (CRDT merge), as a full v1 update.
pub fn merge_states(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
    let doc = doc_from_state(a)?;
    if !b.is_empty() {
        let update = yrs::Update::decode_v1(b).map_err(|e| Error::invalid(format!("Artefakt beschädigt: {e}")))?;
        doc.transact_mut().apply_update(update).map_err(|e| Error::invalid(format!("Artefakt beschädigt: {e}")))?;
    }
    Ok(encode(&doc))
}

pub fn doc_new(content: &str) -> Result<Vec<u8>> {
    doc_replace(&[], content)
}

/// (start, chars to delete, text to insert), all in chars, keeping common prefix and suffix.
fn minimal_edit(old: &str, new: &str) -> (usize, usize, String) {
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|(x, y)| x == y).count();
    let ins: String = b.get(prefix..b.len() - suffix).map(|s| s.iter().collect()).unwrap_or_default();
    (prefix, a.len() - prefix - suffix, ins)
}

/// A title from the first Markdown heading, else the first line.
fn title_of(text: &str, fallback: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(fallback);
    let t = line.trim_start_matches('#').trim();
    let t: String = t.chars().take(80).collect();
    if t.is_empty() { fallback.to_string() } else { t }
}

/// The text of an artifact for a person or a model: a document's Markdown, a board's summary.
pub fn artifact_text(kind: ArtifactKind, state: &[u8]) -> Result<String> {
    match kind {
        ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p => doc_text(state),
        ArtifactKind::Board => Ok(Board::from_state(state).map_err(|e| Error::invalid(format!("Whiteboard beschädigt: {e}")))?.summary_text()),
    }
}

/// Markdown to a standalone HTML page (printable to PDF from any browser).
pub fn markdown_html(title: &str, md: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(md, pulldown_cmark::Options::all());
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, parser);
    let esc = title.replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>{esc}</title><style>body{{font-family:system-ui,sans-serif;max-width:46rem;margin:2rem auto;line-height:1.55;color:#1e293b;padding:0 1rem}}pre,code{{background:#f1f5f9;border-radius:4px}}pre{{padding:.8rem;overflow:auto}}table{{border-collapse:collapse}}td,th{{border:1px solid #d9e6ed;padding:.3rem .6rem}}h1,h2,h3{{color:#107fad}}</style></head><body>{body}</body></html>"
    )
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) fn get_artifact(c: &Ctx, id: &str) -> Result<Artifact> {
    c.state().artifacts.iter().find(|a| a.id == id).cloned().ok_or_else(|| Error::not_found("Artefakt nicht gefunden"))
}

pub(crate) fn state_of(c: &Ctx, id: &str) -> Result<Vec<u8>> {
    c.store(|st| st.artifact_state(id))?.ok_or_else(|| Error::not_found("Artefakt nicht gefunden"))
}

/// Save new content; bumps `updated_at` and the state revision so open views reload.
pub(crate) fn save_state(c: &Ctx, id: &str, state: &[u8], version_note: Option<&str>) -> Result<()> {
    if state.len() > MAX_STATE {
        return Err(Error::invalid("Artefakt ist zu groß (über 32 MB)"));
    }
    let mut a = get_artifact(c, id)?;
    a.updated_at = now_ms();
    c.store(|st| {
        // A shared artifact: merged with what came from the room, and what is new goes there.
        #[cfg(feature = "collab")]
        let merged = crate::collab::merge_local(c, id, state)?;
        #[cfg(feature = "collab")]
        let state: &[u8] = merged.as_deref().unwrap_or(state);
        if state.len() > MAX_STATE {
            return Err(Error::invalid("Artefakt ist zu groß (über 32 MB)"));
        }
        st.save_artifact(&a, Some(state))?;
        if let Some(note) = version_note {
            st.add_artifact_version(id, note, state, a.updated_at)?;
        }
        Ok(())
    })?;
    c.update(|s| {
        if let Some(x) = s.artifacts.iter_mut().find(|x| x.id == id) {
            *x = a;
        }
        s.artifacts.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
        s.artifact_rev = s.artifact_rev.wrapping_add(1);
    });
    Ok(())
}

pub(crate) async fn create(c: Ctx, p: Value) -> Result<Value> {
    let kind =
        ArtifactKind::parse(&opt_str(&p, "kind").unwrap_or_else(|| "document".into())).ok_or_else(|| Error::invalid("kind muss document oder board sein"))?;
    // Learning apps and worksheets are checked and parsed first.
    match kind {
        ArtifactKind::App => {
            return crate::learn::app_create_cmd(c, json!({"html": p.get("content"), "from_message": p.get("from_message"), "title": p.get("title")})).await;
        }
        ArtifactKind::Worksheet => {
            return crate::learn::ws_create_cmd(c, json!({"content": p.get("content"), "from_message": p.get("from_message"), "title": p.get("title")})).await;
        }
        ArtifactKind::H5p => {
            return crate::learn::h5p::create_cmd(c, json!({"content": p.get("content"), "from_message": p.get("from_message"), "title": p.get("title")}))
                .await;
        }
        _ => {}
    }
    let state = c.state();
    let source = match opt_str(&p, "from_message") {
        Some(mid) => Some(state.messages.iter().find(|m| m.id == mid).map(|m| m.content.clone()).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?),
        None => None,
    };
    let content: String = opt_str(&p, "content").or(source).unwrap_or_default().chars().take(MAX_TEXT_CHARS).collect();
    let fallback = if kind == ArtifactKind::Board { "Whiteboard" } else { "Neues Dokument" };
    let title = opt_str(&p, "title").filter(|t| !t.trim().is_empty()).unwrap_or_else(|| title_of(&content, fallback));
    let bytes = match kind {
        ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p => doc_new(&content)?,
        ArtifactKind::Board => {
            let mut b = if content.trim().is_empty() { Board::new() } else { Board::from_outline(&content) };
            b.set_title(&title);
            b.encode_state()
        }
    };
    let now = now_ms();
    let a = Artifact {
        id: new_id(),
        kind,
        title,
        conversation_id: opt_str(&p, "conversation").or_else(|| state.current.clone()),
        project_id: state.selection.project_id.clone(),
        created_at: now,
        updated_at: now,
    };
    c.store(|st| {
        st.save_artifact(&a, Some(&bytes))?;
        st.add_artifact_version(&a.id, "Erstellt", &bytes, now)?;
        Ok(())
    })?;
    let id = a.id.clone();
    c.update(|s| {
        s.artifacts.insert(0, a);
        s.open_artifact = Some(id.clone());
        s.artifact_rev = s.artifact_rev.wrapping_add(1);
    });
    Ok(json!({"id": id}))
}

pub(crate) async fn board_add(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let n = board_add_outline(&c, &id, &req_str(&p, "outline")?)?;
    Ok(json!({"added": n}))
}

pub(crate) async fn get(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let a = get_artifact(&c, &id)?;
    let state = state_of(&c, &id)?;
    Ok(json!({
        "artifact": a,
        "text": artifact_text(a.kind, &state)?,
        "state_b64": base64::engine::general_purpose::STANDARD.encode(&state),
    }))
}

pub(crate) async fn save(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let a = get_artifact(&c, &id)?;
    let note = opt_str(&p, "version");
    let state = if let Some(b64) = opt_str(&p, "state_b64") {
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| Error::invalid("state_b64 ist kein Base64"))?;
        // Reject content that does not parse, before it reaches the store.
        artifact_text(a.kind, &bytes)?;
        // Merged, not replaced: an editor's state that missed a change made meanwhile (by the
        // model, or by someone in a shared room) keeps that change.
        merge_states(&state_of(&c, &id)?, &bytes)?
    } else if let Some(text) = opt_str(&p, "text") {
        if !a.kind.is_text() {
            return Err(Error::invalid("text gibt es nur für Dokumente"));
        }
        let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
        doc_replace(&state_of(&c, &id)?, &text)?
    } else {
        return Err(Error::invalid("state_b64 oder text fehlt"));
    };
    save_state(&c, &id, &state, note.as_deref())?;
    Ok(json!({}))
}

pub(crate) async fn revise(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let instruction = req_str(&p, "instruction")?;
    let a = get_artifact(&c, &id)?;
    if matches!(a.kind, ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p) {
        return crate::learn::revise(&c, &a, &instruction).await;
    }
    if a.kind != ArtifactKind::Document {
        return Err(Error::unsupported("Überarbeiten geht bisher nur für Dokumente"));
    }
    let state = state_of(&c, &id)?;
    let current = doc_text(&state)?;
    let system = "Du überarbeitest ein Markdown-Dokument nach der Anweisung der Person. Antworte nur mit dem vollständigen, überarbeiteten Dokument in Markdown, ohne Einleitung, ohne Erklärung und ohne Codeblock drumherum.";
    let prompt = format!("<dokument>\n{current}\n</dokument>\n\nAnweisung: {instruction}");
    let out = crate::chat::complete(&c, Some(system), &prompt).await?;
    let revised = strip_fence(&out);
    if revised.trim().is_empty() {
        return Err(Error::protocol("Das Modell hat kein Dokument zurückgegeben"));
    }
    c.store(|st| st.add_artifact_version(&id, "Vor KI-Überarbeitung", &state, now_ms()))?;
    let new_state = doc_replace(&state, &revised)?;
    let note = format!("KI: {}", instruction.chars().take(60).collect::<String>());
    save_state(&c, &id, &new_state, Some(&note))?;
    c.notice(NoticeLevel::Success, "Dokument überarbeitet. Die vorige Fassung liegt unter Versionen.");
    Ok(json!({"text": revised}))
}

/// Models sometimes wrap the whole answer in ```markdown fences anyway.
fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```")
        && let Some(nl) = rest.find('\n')
        && let Some(body) = rest.get(nl + 1..).and_then(|b| b.trim_end().strip_suffix("```"))
    {
        return body.trim_end().to_string();
    }
    t.to_string()
}

pub(crate) async fn export(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let a = get_artifact(&c, &id)?;
    if let Some(r) = crate::learn::export_for(&c, &a, &p) {
        return r;
    }
    let state = state_of(&c, &id)?;
    let text = artifact_text(a.kind, &state)?;
    let format = opt_str(&p, "format").unwrap_or_else(|| "markdown".into());
    // Word and PDF are binary: written to `path`, or returned as base64.
    if let Some(bytes) = crate::office::document_bytes(&format, &a.title, &text) {
        let bytes = bytes?;
        if let Some(path) = opt_str(&p, "path") {
            craft_atomic::atomic_write(std::path::Path::new(&path), &bytes).map_err(|e| Error::storage(format!("{path}: {e}")))?;
            c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
            return Ok(json!({"path": path}));
        }
        return Ok(json!({"base64": base64::engine::general_purpose::STANDARD.encode(&bytes), "bytes": bytes.len()}));
    }
    let out = match (a.kind, format.as_str()) {
        (_, "markdown" | "md") => text,
        (_, "html") => markdown_html(&a.title, &text),
        (ArtifactKind::Board, "json") => Board::from_state(&state).map(|b| b.to_json().to_string()).map_err(|e| Error::invalid(e.to_string()))?,
        (_, other) => {
            return Err(Error::invalid(format!(
                "Format {other} gibt es hier nicht (markdown, html, docx, pdf{})",
                if a.kind == ArtifactKind::Board { ", json" } else { "" }
            )));
        }
    };
    if let Some(path) = opt_str(&p, "path") {
        craft_atomic::atomic_write(std::path::Path::new(&path), out.as_bytes()).map_err(|e| Error::storage(format!("{path}: {e}")))?;
        c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
        return Ok(json!({"path": path}));
    }
    Ok(json!({"text": out}))
}

pub(crate) async fn versions(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    Ok(serde_json::to_value(c.store(|st| st.artifact_versions(&id))?)?)
}

pub(crate) async fn restore(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let v = p.get("version").and_then(Value::as_i64).ok_or_else(|| Error::invalid("version fehlt"))?;
    let a = get_artifact(&c, &id)?;
    let old = c.store(|st| st.artifact_version_state(&id, v))?.ok_or_else(|| Error::not_found("Version nicht gefunden"))?;
    let current = state_of(&c, &id)?;
    // Restoring is itself an edit on top of the current state, so nothing is lost.
    let new_state = match a.kind {
        ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p => doc_replace(&current, &doc_text(&old)?)?,
        ArtifactKind::Board => old,
    };
    save_state(&c, &id, &new_state, Some(&format!("Version {v} wiederhergestellt")))?;
    Ok(json!({}))
}

pub(crate) async fn edit_meta(c: Ctx, p: Value, f: impl FnOnce(&mut Artifact, &Value) -> Result<()>) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut a = get_artifact(&c, &id)?;
    f(&mut a, &p)?;
    a.updated_at = now_ms();
    c.store(|st| st.save_artifact(&a, None))?;
    c.update(|s| {
        if let Some(x) = s.artifacts.iter_mut().find(|x| x.id == id) {
            *x = a;
        }
    });
    Ok(json!({}))
}

pub(crate) async fn delete(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    #[cfg(feature = "collab")]
    crate::collab::forget(&c, &id);
    c.store(|st| st.delete_artifact(&id))?;
    c.update(|s| {
        s.artifacts.retain(|a| a.id != id);
        if s.open_artifact.as_deref() == Some(&id) {
            s.open_artifact = None;
        }
    });
    Ok(json!({}))
}

/// Add an outline to a board, to the right of what is there.
pub(crate) fn board_add_outline(c: &Ctx, id: &str, outline: &str) -> Result<usize> {
    let a = get_artifact(c, id)?;
    if a.kind != ArtifactKind::Board {
        return Err(Error::invalid("Das ist kein Whiteboard"));
    }
    if outline.trim().is_empty() {
        return Err(Error::invalid("Die Gliederung ist leer"));
    }
    let mut board = Board::from_state(&state_of(c, id)?).map_err(|e| Error::invalid(format!("Whiteboard beschädigt: {e}")))?;
    let right = board.elements().iter().map(|e| e.x + e.w).fold(0.0_f64, f64::max);
    let ids = board.add_outline(&outline.chars().take(20_000).collect::<String>(), right + 400.0, 0.0);
    save_state(c, id, &board.encode_state(), Some("Vom Modell ergänzt"))?;
    Ok(ids.len())
}

/// Attach the open document's text to a chat message as context (used by "Im Chat verwenden").
/// For backends without tools (AIS.chat), ask for changes as a marked code block the person can
/// apply with one click.
pub(crate) fn context_block(c: &Ctx, id: &str, without_tools: bool) -> Result<String> {
    let a = get_artifact(c, id)?;
    let text = artifact_text(a.kind, &state_of(c, id)?)?;
    let tag = a.kind.label().to_lowercase();
    let mut block = format!("<{tag} titel=\"{}\">\n{text}\n</{tag}>", a.title.replace('"', "'"));
    if without_tools {
        block.push_str(match a.kind {
            ArtifactKind::Document => "\n\n(Wenn du dieses Dokument ändern sollst, gib die vollständige neue Fassung in einem Codeblock mit der Sprache `aiscraft-dokument` aus.)",
            ArtifactKind::Board => "\n\n(Wenn du dieses Whiteboard ergänzen sollst, gib nur die neuen Punkte als eingerückte Markdown-Liste in einem Codeblock mit der Sprache `aiscraft-whiteboard` aus.)",
            kind => crate::learn::change_note(kind),
        });
    }
    Ok(block)
}

/// Marked change blocks in an answer: (kind, content), for "Ins Dokument übernehmen".
pub fn change_blocks(answer: &str) -> Vec<(ArtifactKind, String)> {
    let mut out = Vec::new();
    let mut rest = answer;
    while let Some(start) = rest.find("```aiscraft-") {
        let after = rest.get(start + 3..).unwrap_or("");
        let Some(nl) = after.find('\n') else { break };
        let lang = after.get(..nl).unwrap_or("").trim();
        let body = after.get(nl + 1..).unwrap_or("");
        let Some(end) = body.find("\n```").or_else(|| body.strip_suffix("```").map(|b| b.len())) else { break };
        let content = body.get(..end).unwrap_or("").to_string();
        let kind = match lang {
            "aiscraft-dokument" | "aiscraft-document" => Some(ArtifactKind::Document),
            "aiscraft-whiteboard" | "aiscraft-board" => Some(ArtifactKind::Board),
            _ => None,
        };
        if let Some(k) = kind
            && !content.trim().is_empty()
        {
            out.push((k, content));
        }
        rest = body.get(end + 4..).unwrap_or("");
    }
    out
}

#[allow(dead_code)]
fn _role_used(_: Role) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_edits_and_unicode() {
        let s = doc_new("Hallo Welt").unwrap();
        let s = doc_replace(&s, "Hallo schöne Welt 😀").unwrap();
        assert_eq!(doc_text(&s).unwrap(), "Hallo schöne Welt 😀");
        let s = doc_replace(&s, "😀 Welt").unwrap();
        assert_eq!(doc_text(&s).unwrap(), "😀 Welt");
        assert_eq!(minimal_edit("abc", "abXc"), (2, 0, "X".into()));
        assert_eq!(minimal_edit("aaa", "aa"), (2, 1, String::new()));
        assert!(doc_text(b"\xff\xfe garbage").is_err());
    }

    #[test]
    fn concurrent_edits_merge() {
        let base = doc_new("Eins\nZwei\n").unwrap();
        let a = doc_replace(&base, "Eins!\nZwei\n").unwrap();
        let b = doc_replace(&base, "Eins\nZwei?\n").unwrap();
        // Merge b into a.
        let doc = doc_from_state(&a).unwrap();
        doc.transact_mut().apply_update(yrs::Update::decode_v1(&b).unwrap()).unwrap();
        let merged = doc.get_or_insert_text("content").get_string(&doc.transact());
        assert_eq!(merged, "Eins!\nZwei?\n");
    }

    #[test]
    fn change_blocks_in_answers() {
        let a = "Hier die neue Fassung:\n```aiscraft-dokument\n# Plan\nKürzer\n```\nUnd fürs Board:\n```aiscraft-whiteboard\n- Wald\n  - Bäume\n```";
        let b = change_blocks(a);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], (ArtifactKind::Document, "# Plan\nKürzer".to_string()));
        assert_eq!(b[1].0, ArtifactKind::Board);
        assert!(change_blocks("```python\nx\n```").is_empty());
        assert!(change_blocks("```aiscraft-dokument\nohne Ende").is_empty());
        assert!(change_blocks("```aiscraft-dokument\n\n```").is_empty());
    }

    #[test]
    fn fences_and_titles() {
        assert_eq!(strip_fence("```markdown\n# A\ntext\n```"), "# A\ntext");
        assert_eq!(strip_fence("# A"), "# A");
        assert_eq!(title_of("\n## Stundenplan 7b\n…", "x"), "Stundenplan 7b");
        assert!(markdown_html("T<", "**fett**").contains("<strong>fett</strong>"));
    }
}
