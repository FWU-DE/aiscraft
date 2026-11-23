//! Memory, modelled on n8n's Nod (Apache-2.0): notes come only from the owner's own words, each
//! note keeps the message it came from, and nothing a model, a web page or a file says can
//! create one. Notes are archived, never silently deleted.

use serde::{Deserialize, Serialize};

use crate::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryArea {
    /// Who the owner is: role, school, subjects.
    About,
    /// How they want answers: language, tone, format.
    Preferences,
    People,
    Routines,
    Lessons,
}

impl MemoryArea {
    pub const ALL: [MemoryArea; 5] = [MemoryArea::About, MemoryArea::Preferences, MemoryArea::People, MemoryArea::Routines, MemoryArea::Lessons];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryArea::About => "about",
            MemoryArea::Preferences => "preferences",
            MemoryArea::People => "people",
            MemoryArea::Routines => "routines",
            MemoryArea::Lessons => "lessons",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            MemoryArea::About => "Über mich",
            MemoryArea::Preferences => "Vorlieben",
            MemoryArea::People => "Personen",
            MemoryArea::Routines => "Abläufe",
            MemoryArea::Lessons => "Gelerntes",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryNote {
    pub id: String,
    pub area: MemoryArea,
    pub text: String,
    /// The message the note came from; `None` when the owner typed it into the memory panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<String>,
    /// The owner's words, verbatim, that the note rests on.
    pub source_quote: String,
    pub created_at: Millis,
    #[serde(default)]
    pub archived: bool,
}

const TRIGGERS: [&str; 9] =
    ["bitte merke dir", "bitte merk dir", "merke dir", "merk dir", "notiere dir", "speichere dir", "please remember", "remember that", "remember"];

/// If the owner explicitly asks to remember something ("Merke dir: …"), return the area and the
/// note text. Only the owner's own message is ever passed here.
pub fn extract_explicit_memory(user_text: &str) -> Option<(MemoryArea, String)> {
    let trimmed = user_text.trim();
    let lower = trimmed.to_lowercase();
    let trigger = TRIGGERS.iter().find(|t| lower.starts_with(*t))?;
    // Lowercasing can change byte lengths (e.g. 'İ'), so cut by characters, not bytes.
    let rest: String = trimmed.chars().skip(trigger.chars().count()).collect();
    let rest = rest.trim_start_matches(|c: char| c == ':' || c == ',' || c == '-' || c.is_whitespace());
    let rest = rest.strip_prefix("dass ").or_else(|| rest.strip_prefix("that ")).unwrap_or(rest).trim();
    let rest = rest.trim_end_matches(['.', '!']).trim();
    if rest.chars().count() < 3 {
        return None;
    }
    let mut text: String = rest.chars().take(500).collect();
    if let Some(first) = text.chars().next() {
        let upper: String = first.to_uppercase().collect();
        text = format!("{upper}{}", text.chars().skip(1).collect::<String>());
    }
    Some((guess_area(&text.to_lowercase()), text))
}

fn guess_area(l: &str) -> MemoryArea {
    let has = |words: &[&str]| words.iter().any(|w| l.contains(w));
    if has(&["jeden ", "jede woche", "immer montags", "immer dienstags", "immer mittwochs", "immer donnerstags", "immer freitags", "every ", "routine"]) {
        MemoryArea::Routines
    } else if has(&["antworte", "bevorzug", "ich mag", "lieber ", "bitte immer", "duze", "sieze", "format", "kurz", "prefer", "answer"]) {
        MemoryArea::Preferences
    } else if has(&["kollege", "kollegin", "schüler", "schulleit", "mein mann", "meine frau", "mein sohn", "meine tochter", "colleague"]) {
        MemoryArea::People
    } else if has(&["ich bin", "ich heiße", "ich unterrichte", "meine schule", "ich arbeite", "i am", "i teach", "my school"]) {
        MemoryArea::About
    } else {
        MemoryArea::Lessons
    }
}

/// The memory index for a system prompt: every active note, grouped by area.
pub fn memory_prompt(notes: &[MemoryNote]) -> Option<String> {
    let active: Vec<&MemoryNote> = notes.iter().filter(|n| !n.archived).collect();
    if active.is_empty() {
        return None;
    }
    let mut out = String::from("Notizen der Person über sich selbst (von ihr selbst so gesagt):\n");
    for area in MemoryArea::ALL {
        let in_area: Vec<_> = active.iter().filter(|n| n.area == area).collect();
        if in_area.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{}:\n", area.label()));
        for n in in_area {
            out.push_str(&format!("- {}\n", n.text));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_triggers() {
        let (a, t) = extract_explicit_memory("Merke dir: ich unterrichte Mathe und Physik.").unwrap();
        assert_eq!(a, MemoryArea::About);
        assert_eq!(t, "Ich unterrichte Mathe und Physik");
        let (a, _) = extract_explicit_memory("bitte merk dir, dass du mir immer kurz antwortest").unwrap();
        assert_eq!(a, MemoryArea::Preferences);
        assert!(extract_explicit_memory("Was soll ich mir merken?").is_none());
        assert!(extract_explicit_memory("merke dir").is_none());
    }

    #[test]
    fn odd_unicode_never_panics() {
        for s in ["İİİ merke dir xyz", "merke dir: 😀😀😀", "remember\u{0}\u{0}\u{0}abc", "MERKE DIR İstanbul ist schön"] {
            let _ = extract_explicit_memory(s);
        }
    }

    #[test]
    fn prompt_groups_and_skips_archived() {
        let n = |area, text: &str, archived| MemoryNote {
            id: text.into(),
            area,
            text: text.into(),
            source_message_id: None,
            source_quote: text.into(),
            created_at: 0,
            archived,
        };
        let p = memory_prompt(&[n(MemoryArea::About, "Lehrerin", false), n(MemoryArea::People, "alt", true)]).unwrap();
        assert!(p.contains("Über mich:\n- Lehrerin"));
        assert!(!p.contains("alt"));
        assert!(memory_prompt(&[]).is_none());
    }
}
