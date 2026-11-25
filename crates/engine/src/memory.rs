//! Notes from the owner's own words, and „Sag es einmal“ (#21, Nod's "Tell it once"): a new note
//! that says the same as an older one differently, or contradicts it („Nicht mehr Anna, sondern
//! Bob macht die Rechnungen“), replaces it. The old note is archived with a pointer to the new
//! one, never deleted, and comes back with „Wiederherstellen“.
//!
//! Which older note a new one replaces: notes that share a subject word are the candidates
//! (`replace_candidates`); a model picks one only where the notes go to that model anyway (the
//! service gets the notes with every chat). Otherwise the owner gets the question as a notice.

use aiscraft_core::{Error, MemoryArea, MemoryNote, Result, replace_candidates};
use serde_json::{Value, json};

use crate::{Ctx, NoticeAction, NoticeLevel, new_id, now_ms};

/// Keep a note from the owner's own words and look for the one it replaces, in the background.
/// Only the owner's own message or the memory panel ever calls this; answers, tool results,
/// pages and files never do.
pub(crate) fn add_from_owner(c: &Ctx, area: MemoryArea, text: String, source_message_id: Option<String>, quote: String) -> Result<MemoryNote> {
    let note = MemoryNote { id: new_id(), area, text, source_message_id, source_quote: quote, created_at: now_ms(), archived: false, replaced_by: None };
    c.store(|st| st.save_memory(&note))?;
    c.update(|s| s.memory.push(note.clone()));
    let ctx = c.clone();
    let id = note.id.clone();
    c.spawn(async move {
        let _ = check_replaces(&ctx, &id).await;
    });
    Ok(note)
}

/// Whether notes may go to the selected model for the comparison: only where they go along with
/// every chat anyway (the memory is on, the chat uses it and the service takes it: in the system
/// prompt with „Notizen mitsenden“, or for AIS.chat after the owner agreed to send it inline).
fn notes_go_to_model(c: &Ctx) -> bool {
    let s = c.state();
    let Some(b) = s.backend(&s.selection.backend_id) else { return false };
    let src = &s.selection.sources;
    s.settings.memory && !s.selection.model_id.is_empty() && src.memory && ((b.capabilities.system_prompt && b.config.send_context) || src.inline)
}

/// The number the model picked, from `{"ersetzt": 2}`; `None` for null, nothing or garbage.
fn picked(out: &str) -> Option<usize> {
    let start = out.find('{')?;
    let end = out.rfind('}')? + 1;
    let v: Value = serde_json::from_str(out.get(start..end)?).ok()?;
    v.get("ersetzt").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok())
}

/// Find the older note `note_id` replaces and replace it. Returns the replaced note's id.
pub(crate) async fn check_replaces(c: &Ctx, note_id: &str) -> Result<Option<String>> {
    let notes = c.state().memory.clone();
    let Some(new) = notes.iter().find(|n| n.id == note_id && !n.archived).cloned() else { return Ok(None) };
    let candidates: Vec<MemoryNote> = replace_candidates(&new, &notes).into_iter().cloned().collect();
    let Some(first) = candidates.first() else { return Ok(None) };
    if !notes_go_to_model(c) {
        // No model may see the notes: the owner decides.
        c.notice_with(
            NoticeLevel::Info,
            format!("Ersetzt die neue Notiz „{}“ die ältere „{}“?", new.text, first.text),
            Some(NoticeAction { label: "Ersetzen".into(), command: "memory.replace".into(), params: json!({"id": first.id, "by": new.id}) }),
        );
        return Ok(None);
    }
    let list: Vec<String> = candidates.iter().enumerate().map(|(i, n)| format!("{}. {}", i + 1, n.text)).collect();
    let system = "Du prüfst Notizen einer Person über sich selbst. Ersetzt die neue Notiz eine der älteren, weil sie dasselbe anders sagt oder ihr widerspricht (zum Beispiel ist jetzt jemand anderes zuständig, ein Termin oder eine Vorliebe hat sich geändert)? Nur wenn das eindeutig ist; eine Ergänzung ersetzt nichts. Antworte nur mit JSON: {\"ersetzt\": Nummer der älteren Notiz oder null}";
    let prompt = format!("Neue Notiz: {}\n\nÄltere Notizen:\n{}", new.text, list.join("\n"));
    let out = crate::chat::complete(c, Some(system), &prompt).await?;
    // Only one of the numbered candidates; any other answer replaces nothing.
    let Some(old) = picked(&out).and_then(|n| n.checked_sub(1)).and_then(|i| candidates.get(i)) else { return Ok(None) };
    replace(c, &old.id, &new.id)?;
    c.notice_with(
        NoticeLevel::Success,
        format!("Ersetzt: „{}“ durch „{}“", old.text, new.text),
        Some(NoticeAction { label: "Rückgängig".into(), command: "memory.restore".into(), params: json!({"id": old.id}) }),
    );
    Ok(Some(old.id.clone()))
}

/// Archive note `id` as replaced by note `by`.
pub(crate) fn replace(c: &Ctx, id: &str, by: &str) -> Result<Value> {
    if id == by {
        return Err(Error::invalid("Eine Notiz kann sich nicht selbst ersetzen"));
    }
    let s = c.state();
    let active = |x: &str| s.memory.iter().find(|n| n.id == x && !n.archived).cloned();
    let mut old = active(id).ok_or_else(|| Error::not_found("Notiz nicht gefunden oder schon vergessen"))?;
    active(by).ok_or_else(|| Error::not_found("Die neue Notiz gibt es nicht (mehr)"))?;
    old.archived = true;
    old.replaced_by = Some(by.to_string());
    c.store(|st| st.save_memory(&old))?;
    c.update(|s| {
        if let Some(n) = s.memory.iter_mut().find(|n| n.id == old.id) {
            *n = old;
        }
    });
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::picked;

    #[test]
    fn only_a_number_counts() {
        assert_eq!(picked(r#"{"ersetzt": 2}"#), Some(2));
        assert_eq!(picked(r#"Klar: {"ersetzt": 1} fertig"#), Some(1));
        assert_eq!(picked(r#"{"ersetzt": null}"#), None);
        assert_eq!(picked(r#"{"ersetzt": "1"}"#), None);
        assert_eq!(picked("ok"), None);
        assert_eq!(picked("}{"), None);
    }
}
