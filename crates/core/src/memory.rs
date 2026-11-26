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
    /// How the owner writes (#20): learned from their own messages, without names and numbers.
    /// Goes along only with writing requests, never in the general memory index.
    Style,
}

impl MemoryArea {
    pub const ALL: [MemoryArea; 6] =
        [MemoryArea::About, MemoryArea::Preferences, MemoryArea::People, MemoryArea::Routines, MemoryArea::Lessons, MemoryArea::Style];
    /// The areas of ordinary notes (everything but the writing style).
    pub const NOTES: [MemoryArea; 5] = [MemoryArea::About, MemoryArea::Preferences, MemoryArea::People, MemoryArea::Routines, MemoryArea::Lessons];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryArea::About => "about",
            MemoryArea::Preferences => "preferences",
            MemoryArea::People => "people",
            MemoryArea::Routines => "routines",
            MemoryArea::Lessons => "lessons",
            MemoryArea::Style => "style",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            MemoryArea::About => "Über mich",
            MemoryArea::Preferences => "Vorlieben",
            MemoryArea::People => "Personen",
            MemoryArea::Routines => "Abläufe",
            MemoryArea::Lessons => "Gelerntes",
            MemoryArea::Style => "Schreibstil",
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
    /// The newer note that replaced this one (#21, „Sag es einmal“); set together with
    /// `archived`, cleared when the owner restores the note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
}

/// Polite words in front of the request („Bitte merk dir …“, „Please remember …“, „Lütfen …“).
const POLITE: [&str; 12] =
    ["bitte", "please", "por favor", "s'il te plaît", "s'il vous plaît", "per favore", "alsjeblieft", "lütfen", "proszę", "пожалуйста", "kérlek", "请"];

/// Requests to remember that open the message, in the languages people at school write in
/// (Nod #1490: remembering works in any language, not only German and English).
const LEADING: [&str; 50] = [
    // Deutsch
    "merke dir",
    "merk dir",
    "notiere dir",
    "speichere dir",
    "denk dran",
    "denke daran",
    "vergiss nicht",
    // English
    "remember",
    "keep in mind",
    "don't forget",
    "do not forget",
    // Français
    "souviens-toi",
    "souvenez-vous",
    "rappelle-toi",
    "retiens",
    "n'oublie pas",
    // Español, Português, Italiano
    "recuerda",
    "acuérdate de",
    "no olvides",
    "lembre-se de",
    "lembre-se",
    "lembra-te de",
    "lembra",
    "lembre",
    "ricordati",
    "ricorda",
    "non dimenticare",
    // Nederlands, Skandinavisk, Suomi
    "onthoud",
    "vergeet niet",
    "kom ihåg att",
    "kom ihåg",
    "husk at",
    "husk",
    "muista",
    // Polski, Čeština, Українська, Русский, Ελληνικά
    "zapamiętaj",
    "nie zapomnij",
    "zapamatuj si",
    "запам'ятай",
    "запамʼятай",
    "запомни",
    "не забудь",
    "θυμήσου",
    // Türkçe (also at the end, see TRAILING)
    "şunu hatırla",
    "bunu hatırla",
    "hatırla",
    "unutma",
    // العربية, 中文, 日本語
    "تذكر",
    "记住",
    "記住",
    "覚えておいて",
];

/// Words that join the request and the fact („…, dass“, „that“, „que“, „che“, …).
const JOINERS: [&str; 11] = ["dass", "daß", "that", "que", "che", "dat", "että", "że", "že", "что", "ότι"];

/// Requests to remember that close the message, where the verb comes last (Türkçe, 日本語, 한국어,
/// हिन्दी): „Toplantı salı günü, bunu hatırla“, „会議は火曜日だと覚えておいて“.
const TRAILING: [&str; 17] = [
    "hatırla",
    "hatırlayın",
    "unutma",
    "unutmayın",
    "aklında tut",
    "aklınızda tutun",
    "覚えておいてください",
    "覚えておいて",
    "覚えといて",
    "覚えて",
    "기억해 주세요",
    "기억해 줘",
    "기억해줘",
    "기억해",
    "याद रखना",
    "याद रखो",
    "याद रखें",
];

/// Scripts written without spaces between words: a request there needs no word boundary.
fn unspaced(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF)
}

/// Equal ignoring case, also for the Turkish dotted and dotless i.
fn same(c: char, t: char) -> bool {
    c == t || c.to_lowercase().eq(t.to_lowercase()) || matches!((c, t), ('I', 'ı') | ('İ', 'i') | ('ʼ', '\'') | ('’', '\''))
}

/// Whether `chars` starts with `word` as a whole word; the length in chars if so.
fn leading(chars: &[char], word: &str) -> Option<usize> {
    let w: Vec<char> = word.chars().collect();
    let head = chars.get(..w.len())?;
    if !head.iter().zip(&w).all(|(c, t)| same(*c, *t)) {
        return None;
    }
    let cjk = w.first().copied().is_some_and(unspaced);
    match chars.get(w.len()) {
        Some(next) if next.is_alphanumeric() && !cjk => None,
        _ => Some(w.len()),
    }
}

/// Whether `chars` ends with `word` as a whole word; the length in chars if so.
fn trailing(chars: &[char], word: &str) -> Option<usize> {
    let w: Vec<char> = word.chars().collect();
    let start = chars.len().checked_sub(w.len())?;
    let tail = chars.get(start..)?;
    if !tail.iter().zip(&w).all(|(c, t)| same(*c, *t)) {
        return None;
    }
    let cjk = w.first().copied().is_some_and(unspaced);
    match start.checked_sub(1).and_then(|i| chars.get(i)) {
        Some(prev) if prev.is_alphanumeric() && !cjk => None,
        _ => Some(w.len()),
    }
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, ':' | ',' | '-' | '–' | '：' | '，' | '、' | ';' | '،')
}

/// The longest of `words` that fits, so „merke dir“ wins over a shorter one.
fn longest(chars: &[char], words: &[&str], fits: fn(&[char], &str) -> Option<usize>) -> Option<usize> {
    words.iter().filter_map(|w| fits(chars, w)).max()
}

/// The fact the owner asks to remember, without the request around it; `None` if the message is no
/// such request. Works by characters (lowercasing can change byte lengths, e.g. 'İ').
fn remembered_fact(text: &str) -> Option<String> {
    let mut chars: Vec<char> = text.trim().chars().collect();
    let skip_separators = |chars: &mut Vec<char>| {
        let n = chars.iter().take_while(|c| is_separator(**c)).count();
        chars.drain(..n);
    };
    if let Some(n) = longest(&chars, &POLITE, leading) {
        chars.drain(..n);
        skip_separators(&mut chars);
    }
    let end_marks = |c: &char| matches!(c, '.' | '!' | '。' | '！');
    if let Some(n) = longest(&chars, &LEADING, leading) {
        chars.drain(..n);
        skip_separators(&mut chars);
        if let Some(n) = longest(&chars, &JOINERS, leading) {
            chars.drain(..n);
            skip_separators(&mut chars);
        }
    } else {
        while chars.last().is_some_and(end_marks) {
            chars.pop();
        }
        let n = longest(&chars, &TRAILING, trailing)?;
        chars.truncate(chars.len().saturating_sub(n));
        // „…, bunu hatırla“, „…ことを覚えておいて“: the pointer to the fact goes too.
        while chars.last().is_some_and(|c| is_separator(*c) || matches!(c, 'を' | 'は')) {
            chars.pop();
        }
        for pointer in ["bunu", "şunu", "これを", "이것을", "이걸"] {
            if let Some(n) = trailing(&chars, pointer) {
                chars.truncate(chars.len().saturating_sub(n));
            }
        }
    }
    while chars.last().is_some_and(|c| end_marks(c) || is_separator(*c)) {
        chars.pop();
    }
    let fact: String = chars.into_iter().collect();
    let fact = fact.trim();
    (fact.chars().count() >= 3).then(|| fact.to_string())
}

/// If the owner explicitly asks to remember something („Merke dir: …“, „Remember …“, „Recuerda
/// que …“, „… hatırla“), in any common language, return the area and the note text. Only the
/// owner's own message is ever passed here.
pub fn extract_explicit_memory(user_text: &str) -> Option<(MemoryArea, String)> {
    let rest = remembered_fact(user_text)?;
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

/// Words that say nothing about what a note is about.
const STOP: [&str; 40] = [
    "aber", "alle", "als", "auch", "auf", "aus", "bei", "bin", "bis", "das", "dass", "dem", "den", "der", "die", "ein", "eine", "einen", "für", "hat", "ich",
    "immer", "ist", "jetzt", "macht", "mehr", "mein", "meine", "mit", "nicht", "noch", "nur", "oder", "sich", "sie", "sind", "und", "von", "wie", "wird",
];

/// The words of a note that carry its subject: lowercase, at least four letters, no filler.
fn subject_words(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 4 && !STOP.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// Active notes a new note might replace (#21): they share a subject word with it, most shared
/// words first, at most eight. Which one it really replaces, if any, the owner or a model decides.
pub fn replace_candidates<'a>(new: &MemoryNote, notes: &'a [MemoryNote]) -> Vec<&'a MemoryNote> {
    let words = subject_words(&new.text);
    let mut scored: Vec<(usize, &MemoryNote)> = notes
        .iter()
        .filter(|n| !n.archived && n.id != new.id && n.area != MemoryArea::Style && new.area != MemoryArea::Style)
        .map(|n| (subject_words(&n.text).intersection(&words).count(), n))
        .filter(|(k, _)| *k > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.created_at.cmp(&a.1.created_at)));
    scored.into_iter().take(8).map(|(_, n)| n).collect()
}

/// The memory index for a system prompt: every active note, grouped by area.
pub fn memory_prompt(notes: &[MemoryNote]) -> Option<String> {
    let active: Vec<&MemoryNote> = notes.iter().filter(|n| !n.archived && n.area != MemoryArea::Style).collect();
    if active.is_empty() {
        return None;
    }
    let mut out = String::from("Notizen der Person über sich selbst (von ihr selbst so gesagt):\n");
    for area in MemoryArea::NOTES {
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

/// The writing style for a writing request (#20), when there is a style note.
pub fn style_prompt(notes: &[MemoryNote]) -> Option<String> {
    let note = notes.iter().rev().find(|n| !n.archived && n.area == MemoryArea::Style && !n.text.trim().is_empty())?;
    Some(format!(
        "Schreibstil der Person (aus ihren eigenen Texten gelernt). Schreibe Entwürfe in ihrem Namen (Mails, Briefe, Antworten) so, ohne ihn zu erwähnen:\n{}",
        note.text.trim()
    ))
}

/// Words that mark a request to write something in the owner's name.
const WRITING: [&str; 22] = [
    "schreib",
    "formulier",
    "entwurf",
    "entwirf",
    "entwerf",
    "verfass",
    "mail",
    "brief",
    "nachricht an",
    "antwort an",
    "antworte ",
    "einladung",
    "rundschreiben",
    "elternbrief",
    "anschreiben",
    "glückwunsch",
    "absage",
    "zusage",
    "draft",
    "write",
    "reply",
    "letter",
];

/// Whether the owner asks for a text in their name (a mail, a letter, a reply): only then the
/// writing style goes along (#20).
pub fn is_writing_request(text: &str) -> bool {
    let l = text.to_lowercase();
    WRITING.iter().any(|w| l.contains(w))
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

    /// Nod #1490: a request to remember is saved in any language, with the fact alone as the note.
    #[test]
    fn remembering_works_in_any_language() {
        let cases = [
            ("Remember that I teach maths and physics.", "I teach maths and physics"),
            ("Please remember: my school is in Kiel", "My school is in Kiel"),
            ("Recuerda que la reunión es el martes.", "La reunión es el martes"),
            ("Souviens-toi que j'enseigne le français", "J'enseigne le français"),
            ("Ricorda che il consiglio di classe è giovedì", "Il consiglio di classe è giovedì"),
            ("Lembre-se de que a reunião é na terça", "A reunião é na terça"),
            ("Onthoud dat ik op dinsdag vrij ben", "Ik op dinsdag vrij ben"),
            ("Muista että kokous on tiistaina", "Kokous on tiistaina"),
            ("Zapamiętaj, że zebranie jest we wtorek", "Zebranie jest we wtorek"),
            ("Запомни, что собрание во вторник", "Собрание во вторник"),
            ("Hatırla: toplantı salı günü", "Toplantı salı günü"),
            ("Toplantı salı günü, bunu hatırla!", "Toplantı salı günü"),
            ("Veli toplantısı perşembe günü unutma", "Veli toplantısı perşembe günü"),
            ("HATIRLA: Ahmet matematik öğretmeni", "Ahmet matematik öğretmeni"),
            ("会議は火曜日だと覚えておいて。", "会議は火曜日だと"),
            ("记住：我教数学", "我教数学"),
            ("Bitte denk dran, dass ich montags frei habe", "Ich montags frei habe"),
        ];
        for (input, note) in cases {
            let got = extract_explicit_memory(input).map(|(_, t)| t);
            assert_eq!(got.as_deref(), Some(note), "{input}");
        }
        // Not a request to remember: words that only start the same, questions, mentions.
        for input in
            ["Remembering things is hard", "Recuerdas cómo se llama?", "Was soll ich mir merken?", "Ich habe vergessen, es zu sagen", "hatırla", "覚えて"]
        {
            assert!(extract_explicit_memory(input).is_none(), "{input}");
        }
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
            replaced_by: None,
        };
        let p = memory_prompt(&[n(MemoryArea::About, "Lehrerin", false), n(MemoryArea::People, "alt", true)]).unwrap();
        assert!(p.contains("Über mich:\n- Lehrerin"));
        assert!(!p.contains("alt"));
        assert!(memory_prompt(&[]).is_none());
    }

    #[test]
    fn candidates_share_a_subject_word() {
        let n = |id: &str, text: &str, archived| MemoryNote {
            id: id.into(),
            area: MemoryArea::People,
            text: text.into(),
            source_message_id: None,
            source_quote: text.into(),
            created_at: 0,
            archived,
            replaced_by: None,
        };
        let notes = [
            n("a", "Anna macht die Rechnungen", false),
            n("b", "Ich mag Tee", false),
            n("c", "Rechnungen alt", true),
            n("d", "Bob macht die Rechnungen", false),
        ];
        let new = &notes[3];
        let ids: Vec<&str> = replace_candidates(new, &notes).iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["a"], "shares „rechnungen“; archived notes, the note itself and fillers like „macht“ do not count");
    }

    #[test]
    fn style_goes_only_with_writing_requests() {
        let style = MemoryNote {
            id: "s".into(),
            area: MemoryArea::Style,
            text: "Kurze Sätze, duzt Kolleginnen".into(),
            source_message_id: None,
            source_quote: "Aus 6 deiner eigenen Nachrichten gelernt".into(),
            created_at: 0,
            archived: false,
            replaced_by: None,
        };
        assert!(memory_prompt(std::slice::from_ref(&style)).is_none(), "never in the general memory");
        assert!(style_prompt(std::slice::from_ref(&style)).unwrap().contains("Kurze Sätze"));
        assert!(style_prompt(&[MemoryNote { archived: true, ..style }]).is_none());
        assert!(is_writing_request("Schreib eine Mail an die Eltern der 7b"));
        assert!(is_writing_request("Formuliere eine Absage"));
        assert!(!is_writing_request("Was ist Photosynthese?"));
        assert_eq!(MemoryArea::parse("style"), Some(MemoryArea::Style));
    }
}
