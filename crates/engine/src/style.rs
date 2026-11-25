//! The owner's writing style (#20, Nod's "Drafts that sound like you"): drafts of mails, letters
//! to parents and replies sound like the owner.
//!
//! - Learned only from the owner's own messages: never from answers, tool results, attached
//!   files, quoted lines or prompts the app wrote (schedules, tasks), and only from chats with the
//!   service that summarises them, so no text reaches a service it did not reach before.
//! - Before anything leaves the device, names from the class lists, e-mail addresses, phone
//!   numbers and every digit are taken out; „Namen schützen“ protects the call on top. The note
//!   the model writes is cleaned the same way again, and names the owner greets people with
//!   („Liebe Frau Kaya“) are taken out of it.
//! - The result is one note in the memory (area „Schreibstil“) with its provenance. It goes along
//!   only with writing requests, and each chat can switch it off under „Quellen“ (`own_style`).

use std::collections::BTreeSet;

use aiscraft_core::{Error, MemoryArea, MemoryNote, Result};
use serde_json::{Value, json};

use crate::{Ctx, NoticeAction, NoticeLevel, new_id, now_ms};

/// The owner's messages the style is learned from: at most this many, newest first.
const MAX_MESSAGES: usize = 40;
/// Fewer than this many usable messages say too little about a style.
pub(crate) const MIN_MESSAGES: usize = 5;
/// Each message is cut to this many characters.
const MAX_CHARS: usize = 1200;
/// The note is cut to this many characters.
const MAX_NOTE: usize = 800;

/// Words after which the owner writes a name: „Liebe Frau Kaya“, „Hallo Jonas“.
const BEFORE_NAME: [&str; 14] = ["frau", "herr", "herrn", "liebe", "lieber", "hallo", "hi", "hey", "moin", "servus", "dear", "grüße", "gruß", "grüßen"];

/// Takes out every digit and what „Namen schützen“ finds (class-list names, e-mail, phone, IBAN,
/// addresses), and the `names` given; squeezes the spaces left behind.
pub(crate) fn scrub(c: &Ctx, text: &str, names: &BTreeSet<String>) -> String {
    let personal = crate::privacy::strip_personal(c, text);
    let no_digits: String = personal.chars().filter(|ch| !ch.is_ascii_digit()).collect();
    let keep = |w: &&str| {
        let bare: String = w.chars().filter(|ch| ch.is_alphabetic()).collect::<String>().to_lowercase();
        bare.is_empty() || !names.contains(&bare)
    };
    no_digits.lines().map(|l| l.split_whitespace().filter(keep).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// Names the owner writes after a greeting or a title, lowercased: „Liebe Frau Kaya“ → kaya.
pub(crate) fn greeted_names(texts: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for t in texts {
        let words: Vec<String> = t.split_whitespace().map(|w| w.trim_matches(|ch: char| !ch.is_alphabetic()).to_string()).collect();
        for pair in words.windows(2) {
            let [before, name] = pair else { continue };
            if BEFORE_NAME.contains(&before.to_lowercase().as_str())
                && name.chars().next().is_some_and(char::is_uppercase)
                && !BEFORE_NAME.contains(&name.to_lowercase().as_str())
                && name.chars().count() >= 2
            {
                out.insert(name.to_lowercase());
            }
        }
    }
    out
}

/// The owner's own texts for the style: their messages in chats with `backend`, without prompts
/// the app wrote, without quoted lines (`>`), newest first.
pub(crate) fn own_texts(c: &Ctx, backend: &str) -> Result<Vec<String>> {
    let rows = c.store(|st| st.own_messages(0, 400))?;
    Ok(rows
        .into_iter()
        .filter(|m| !m.automatic && m.backend_id == backend)
        .map(|m| m.content.lines().filter(|l| !l.trim_start().starts_with('>')).collect::<Vec<_>>().join("\n").trim().to_string())
        // A word or two says nothing about a style.
        .filter(|t| t.split_whitespace().count() >= 4)
        .take(MAX_MESSAGES)
        .map(|t| t.chars().take(MAX_CHARS).collect())
        .collect())
}

/// Learn the style from the owner's own messages and keep it as the one style note.
pub(crate) async fn learn(c: Ctx, _p: Value) -> Result<Value> {
    let s = c.state().clone();
    if !s.settings.memory {
        return Err(Error::invalid("Das Gedächtnis ist ausgeschaltet (Einstellungen › Gedächtnis)."));
    }
    let backend = s.selection.backend_id.clone();
    let texts = own_texts(&c, &backend)?;
    if texts.len() < MIN_MESSAGES {
        let name = s.backend(&backend).map(|b| b.config.name.clone()).unwrap_or_else(|| "diesem Dienst".into());
        return Err(Error::invalid(format!(
            "Noch zu wenige eigene Nachrichten in Chats mit {name}: {} von mindestens {MIN_MESSAGES}. Schreib noch ein paar Nachrichten und versuch es dann wieder.",
            texts.len()
        )));
    }
    let names = greeted_names(&texts);
    let cleaned: Vec<String> = texts.iter().map(|t| scrub(&c, t, &names)).filter(|t| !t.is_empty()).collect();
    let system = "Du beschreibst den Schreibstil einer Person anhand ihrer eigenen Nachrichten, damit Entwürfe in ihrem Namen (Mails, Elternbriefe, Antworten) nach ihr klingen. Beschreibe nur die Form: Anrede und Gruß (ohne Namen), du oder Sie, Satzlänge, Ton, typische Wendungen, Absätze, Emojis. Keine Namen, keine Zahlen, keine Inhalte, keine längeren Zitate. Höchstens fünf kurze Stichpunkte auf Deutsch, sonst nichts.";
    let prompt = format!("<nachrichten_der_person>\n{}\n</nachrichten_der_person>", cleaned.join("\n---\n"));
    let out = crate::chat::complete(&c, Some(system), &prompt).await?;
    let text: String = scrub(&c, &out, &names).chars().take(MAX_NOTE).collect();
    if text.trim().is_empty() {
        return Err(Error::protocol("Das Modell hat keinen Schreibstil beschrieben. Versuch es später noch einmal."));
    }
    let note = MemoryNote {
        id: new_id(),
        area: MemoryArea::Style,
        text,
        source_message_id: None,
        source_quote: format!("Aus {} deiner eigenen Nachrichten gelernt, ohne Namen und Zahlen", cleaned.len()),
        created_at: now_ms(),
        archived: false,
        replaced_by: None,
    };
    // One style note: the new one replaces the old.
    let old: Vec<MemoryNote> = s.memory.iter().filter(|n| n.area == MemoryArea::Style && !n.archived).cloned().collect();
    c.store(|st| st.save_memory(&note))?;
    for mut o in old.clone() {
        o.archived = true;
        o.replaced_by = Some(note.id.clone());
        c.store(|st| st.save_memory(&o))?;
    }
    let id = note.id.clone();
    c.update(|s| {
        for n in s.memory.iter_mut().filter(|n| old.iter().any(|o| o.id == n.id)) {
            n.archived = true;
            n.replaced_by = Some(id.clone());
        }
        s.memory.push(note.clone());
    });
    c.notice_with(
        NoticeLevel::Success,
        "Schreibstil gelernt. Er geht bei Schreibaufträgen mit.".to_string(),
        Some(NoticeAction { label: "Vergessen".into(), command: "memory.archive".into(), params: json!({"id": id}) }),
    );
    Ok(json!({"id": id, "messages": cleaned.len(), "text": note.text}))
}

#[cfg(test)]
mod tests {
    use super::greeted_names;

    #[test]
    fn names_after_greetings() {
        let t = vec!["Liebe Frau Kaya, danke!".to_string(), "Hallo Jonas, kurz zur Sache.".to_string(), "Viele Grüße".to_string()];
        let n = greeted_names(&t);
        assert!(n.contains("kaya") && n.contains("jonas"));
        assert!(!n.contains("frau") && !n.contains("danke") && !n.contains("viele"));
    }
}
