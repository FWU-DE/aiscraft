//! „Erstellt mit KI“ (#57): AIS.chat's terms of use ask that output passed on is marked as
//! AI-generated, with the prompt. Every export of an answer or an artifact (Word, PDF,
//! Markdown, HTML, Excel, PowerPoint, worksheets, learning apps, H5P, calendar files) gets a
//! short note: service, model, date, and on request the prompt.
//!
//! Settings: `ai_label` = `ais-chat` (default: only for answers from AIS.chat), `all` or `off`;
//! `ai_label_prompt` writes the prompt along (default off: a prompt may hold things not meant
//! for a handout). Copying to the clipboard stays plain; `message.copy_labeled` copies with it.

use aiscraft_core::{Artifact, BackendKind, Error, Message, Result, Role};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{CommandSpec, req_str};

const MAX_PROMPT: usize = 4000;

/// The note for one export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Note {
    /// „Erstellt mit KI (AIS.chat, Modell GPT-5), 25.11.2026“.
    pub line: String,
    /// The prompt, when `ai_label_prompt` is on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl Note {
    /// As Markdown at the end of a document.
    pub fn markdown(&self) -> String {
        let mut s = format!("\n\n---\n\n*{}.*", self.line);
        if let Some(p) = &self.prompt {
            s.push_str("\n\n**Prompt:**\n\n");
            for l in p.lines() {
                s.push_str("> ");
                s.push_str(l);
                s.push('\n');
            }
        }
        s
    }

    /// As plain text (calendar comments, spreadsheet cells, HTML comments).
    pub fn plain(&self) -> String {
        match &self.prompt {
            Some(p) => format!("{}. Prompt: {p}", self.line),
            None => format!("{}.", self.line),
        }
    }

    /// As an HTML comment and a visible footer line for a learning app (before `</body>`).
    pub fn html(&self) -> String {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace("--", "–");
        format!(
            "<!-- {} -->\n<p style=\"font:12px sans-serif;color:#666;margin:2em 1em 1em\" data-aiscraft=\"ki-hinweis\">{}</p>",
            esc(&self.plain()),
            esc(&format!("{}.", self.line))
        )
    }
}

/// Whether exports from this backend are labelled.
fn wanted(c: &Ctx, kind: Option<BackendKind>) -> bool {
    match c.state().settings.ai_label.as_str() {
        "off" => false,
        "all" => true,
        // `ais-chat` and anything unknown: the default.
        _ => kind == Some(BackendKind::AisChat),
    }
}

fn date_today() -> String {
    chrono::Local::now().format("%d.%m.%Y").to_string()
}

/// The note for content from `backend` with `model`, asked by `prompt`; `None` when the
/// setting says no.
fn build(c: &Ctx, backend_id: &str, model: Option<&str>, prompt: Option<&str>, force: bool) -> Option<Note> {
    let s = c.state();
    let b = s.backend(backend_id);
    if !force && !wanted(c, b.map(|b| b.config.kind)) {
        return None;
    }
    let service = b.map(|b| b.config.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "KI-Dienst".into());
    let model = model.map(str::trim).filter(|m| !m.is_empty()).map(|m| s.model_name(backend_id, m));
    let line = match model {
        Some(m) => format!("Erstellt mit KI ({service}, Modell {m}), {}", date_today()),
        None => format!("Erstellt mit KI ({service}), {}", date_today()),
    };
    let prompt = if s.settings.ai_label_prompt {
        prompt.map(|p| {
            // A format request: the teacher's words, not the format spec (#56).
            let p = crate::learn::formats::split_request(p).map(|r| r.topic).unwrap_or(p);
            p.trim().chars().take(MAX_PROMPT).collect::<String>()
        })
    } else {
        None
    }
    .filter(|p| !p.is_empty());
    Some(Note { line, prompt })
}

/// The user message an answer replied to.
fn prompt_before(messages: &[Message], idx: usize) -> Option<&str> {
    messages.get(..idx)?.iter().rev().find(|m| m.role == Role::User).map(|m| m.content.as_str())
}

/// The note for an answer in the open chat (`None`: the newest one, for files a tool makes).
pub(crate) fn note_for_message(c: &Ctx, message_id: Option<&str>) -> Option<Note> {
    message_note(c, message_id, false)
}

/// [`note_for_message`] whatever the setting says („Mit Kennzeichnung kopieren“).
pub(crate) fn note_for_message_always(c: &Ctx, message_id: &str) -> Option<Note> {
    message_note(c, Some(message_id), true)
}

fn message_note(c: &Ctx, message_id: Option<&str>, force: bool) -> Option<Note> {
    let s = c.state();
    let conv = s.current.as_ref().and_then(|id| s.conversation(id));
    let backend = conv.map(|x| x.backend_id.clone()).unwrap_or_else(|| s.selection.backend_id.clone());
    let idx = match message_id {
        Some(mid) => s.messages.iter().position(|m| m.id == mid)?,
        None => s.messages.len(),
    };
    let model = s.messages.get(idx).and_then(|m| m.model.clone()).or_else(|| conv.map(|x| x.model_id.clone())).unwrap_or_else(|| s.selection.model_id.clone());
    build(c, &backend, Some(&model), prompt_before(&s.messages, idx), force)
}

/// The note for an artifact made in a chat: that chat's service, the model of the newest
/// answer before it was made, and the request before that. Artifacts made outside a chat
/// (an empty document, an imported file) get none.
pub(crate) fn note_for_artifact(c: &Ctx, a: &Artifact) -> Option<Note> {
    let conv_id = a.conversation_id.clone()?;
    let s = c.state();
    let conv = s.conversation(&conv_id)?.clone();
    let messages = if s.current.as_deref() == Some(conv_id.as_str()) { s.messages.clone() } else { c.store(|st| st.messages(&conv_id)).unwrap_or_default() };
    let idx = messages.iter().position(|m| m.created_at > a.created_at).unwrap_or(messages.len());
    let model =
        messages.get(..idx).and_then(|m| m.iter().rev().find(|m| m.role == Role::Assistant)).and_then(|m| m.model.clone()).unwrap_or(conv.model_id.clone());
    build(c, &conv.backend_id, Some(&model), prompt_before(&messages, idx), false)
}

pub(crate) static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "message.copy_labeled",
    label: "Mit Kennzeichnung kopieren",
    group: "Chat",
    shortcut: None,
    params: "message_id: string (gibt den Text mit „Erstellt mit KI …“ zurück; die Oberfläche legt ihn in die Zwischenablage)",
    palette: false,
    run: |c, p| Box::pin(copy_labeled(c, p)),
}];

/// The answer with its note, whatever the setting says (the person asked for it).
async fn copy_labeled(c: Ctx, p: Value) -> Result<Value> {
    let mid = req_str(&p, "message_id")?;
    let m = c.state().messages.iter().find(|m| m.id == mid).cloned().ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    if m.role != Role::Assistant {
        return Err(Error::invalid("Nur Antworten lassen sich mit Kennzeichnung kopieren"));
    }
    let note = note_for_message_always(&c, &mid).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    Ok(json!({"text": format!("{}{}", m.content.trim_end(), note.markdown()), "note": note}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_forms() {
        let n = Note { line: "Erstellt mit KI (AIS.chat, Modell X), 01.12.2026".into(), prompt: Some("Brief <an> Eltern\nzweite -- Zeile".into()) };
        assert!(n.markdown().contains("*Erstellt mit KI (AIS.chat, Modell X), 01.12.2026.*") && n.markdown().contains("> zweite"));
        assert!(n.plain().ends_with("Prompt: Brief <an> Eltern\nzweite -- Zeile"));
        let h = n.html();
        assert!(!h.contains("<an>") && h.matches("--").count() == 2, "{h}");
    }
}
