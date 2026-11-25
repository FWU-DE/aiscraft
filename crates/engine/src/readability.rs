//! Readability of an answer (#48): word count, Flesch reading ease for German (Amstad), LIX,
//! an approximate school grade (first Vienna formula, Wiener Sachtextformel) and the long
//! sentences. Computed on the device; nothing leaves it.
//!
//! Syllables are approximated: the pieces between the German hyphenation points of craft-text
//! (`hyphen` + `german`, Liang patterns) that hold a vowel, or the number of vowel groups when
//! that is more („Theater“ has one group but three pieces). The formulas are ours.

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{CommandSpec, opt_str};

/// Text beyond this is not looked at.
const MAX_CHARS: usize = 200_000;
/// A sentence with more words than this is long (Leichte Sprache aims at far fewer).
pub const LONG_SENTENCE: usize = 20;
const MAX_LONG_LISTED: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Readability {
    pub words: usize,
    pub sentences: usize,
    pub syllables: usize,
    /// Flesch reading ease for German (Amstad), 0 (very hard) to 100 (very easy).
    pub flesch: f64,
    /// Läsbarhetsindex: words per sentence plus the share of words over six letters.
    pub lix: f64,
    /// Approximate school grade (Wiener Sachtextformel), 1 to 13.
    pub grade: u8,
    /// German word for the Flesch value („mittelschwer“).
    pub level: String,
    /// How many sentences have more than [`LONG_SENTENCE`] words.
    pub long_sentences: usize,
    /// The first of them, shortened.
    pub long_examples: Vec<String>,
}

impl Readability {
    /// One discreet line: „142 Wörter · Flesch 58 (mittelschwer) · LIX 45 · etwa Klasse 9 · 2 lange Sätze“.
    pub fn summary(&self) -> String {
        if self.words == 0 {
            return "Kein Text".into();
        }
        let long = match self.long_sentences {
            0 => "keine langen Sätze".to_string(),
            1 => "1 langer Satz".to_string(),
            n => format!("{n} lange Sätze"),
        };
        format!("{} Wörter · Flesch {:.0} ({}) · LIX {:.0} · etwa Klasse {} · {long}", self.words, self.flesch, self.level, self.lix, self.grade)
    }
}

fn level(flesch: f64) -> &'static str {
    match flesch {
        f if f >= 90.0 => "sehr leicht",
        f if f >= 80.0 => "leicht",
        f if f >= 70.0 => "eher leicht",
        f if f >= 60.0 => "mittel",
        f if f >= 50.0 => "eher schwer",
        f if f >= 30.0 => "schwer",
        _ => "sehr schwer",
    }
}

/// The prose of a Markdown answer: code blocks, link targets, table rules and markup removed;
/// each heading, list item and table row ends a sentence.
fn prose(md: &str) -> String {
    let mut out = String::with_capacity(md.len().min(MAX_CHARS));
    let mut in_code = false;
    for line in md.lines() {
        if out.len() > MAX_CHARS {
            break;
        }
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() || (t.starts_with('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))) {
            out.push('\n');
            continue;
        }
        let t = t.trim_start_matches(['#', '>', '-', '*', '+', '|', ' ']).trim_end_matches(['|', ' ']);
        let t = t.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start_matches(['.', ')']).trim_start();
        let mut s = String::with_capacity(t.len());
        let mut chars = t.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '*' | '_' | '`' | '#' => {}
                // A table row is one sentence, its cells parts of it.
                '|' => s.push_str(", "),
                ']' if chars.peek() == Some(&'(') => {
                    for d in chars.by_ref() {
                        if d == ')' {
                            break;
                        }
                    }
                }
                '[' => {}
                _ => s.push(c),
            }
        }
        let s = s.trim();
        out.push_str(s);
        // A line of its own (heading, list item) is a sentence even without a full stop.
        if !s.ends_with(['.', '!', '?', ':']) && !s.is_empty() {
            out.push('.');
        }
        out.push('\n');
    }
    out
}

const ABBREVIATIONS: &[&str] = &[
    "z", "b", "d", "h", "u", "a", "o", "ä", "s", "v", "bzw", "usw", "etc", "ca", "nr", "vgl", "evtl", "ggf", "inkl", "max", "min", "bspw", "dr", "prof", "hr",
    "fr", "str", "abb", "kap", "s", "jh", "mio", "mrd",
];

/// The sentences of a text, each a list of its words.
fn sentences(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for raw in text.split_whitespace() {
        let ends = raw.ends_with(['.', '!', '?', ';']) || raw.ends_with(".\"") || raw.ends_with(".“");
        let word: String = raw.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
        let has_letter = word.chars().any(char::is_alphabetic);
        let core = word.to_lowercase();
        // „z. B.“, „3. Dezember“, „usw.“ don't end a sentence.
        let abbreviation = raw.ends_with('.') && (ABBREVIATIONS.contains(&core.as_str()) || (!has_letter && !word.is_empty()));
        if has_letter {
            cur.push(word);
        }
        if ends && !abbreviation && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'ä' | 'ö' | 'ü' | 'y' | 'é' | 'è' | 'à')
}

/// Approximate syllables of one word.
pub fn syllables(word: &str) -> usize {
    let lower = word.to_lowercase();
    let mut groups = 0;
    let mut prev = false;
    for c in lower.chars() {
        let v = is_vowel(c);
        if v && !prev {
            groups += 1;
        }
        prev = v;
    }
    // The patterns are unrestricted („Punk-t“): count only the pieces that hold a vowel.
    let pieces = match craft_text::hyphen::Lang::for_language("de") {
        Some(lang) if word.chars().count() <= 64 => {
            let (points, _) = craft_text::hyphen::word_breaks_in(word, lang);
            let mut start = 0;
            let mut n = 0;
            for end in points.into_iter().chain(std::iter::once(word.len())) {
                if word.get(start..end).is_some_and(|p| p.to_lowercase().chars().any(is_vowel)) {
                    n += 1;
                }
                start = end;
            }
            n
        }
        _ => 0,
    };
    pieces.max(groups).max(1)
}

/// The readability of a Markdown text.
pub fn measure(md: &str) -> Readability {
    let text: String = md.chars().take(MAX_CHARS).collect();
    let sents = sentences(&prose(&text));
    let words: Vec<&String> = sents.iter().flatten().collect();
    let n = words.len();
    if n == 0 {
        return Readability { level: "kein Text".into(), ..Readability::default() };
    }
    let s = sents.len().max(1);
    let syl: Vec<usize> = words.iter().map(|w| syllables(w)).collect();
    let total_syl: usize = syl.iter().sum();
    let (nf, sf) = (n as f64, s as f64);
    let asl = nf / sf;
    let asw = total_syl as f64 / nf;
    let flesch = (180.0 - asl - 58.5 * asw).clamp(0.0, 100.0);
    let long_words = words.iter().filter(|w| w.chars().count() > 6).count() as f64;
    let lix = asl + 100.0 * long_words / nf;
    // Wiener Sachtextformel 1: MS % words with 3+ syllables, SL mean sentence length,
    // IW % words over six letters, ES % one-syllable words.
    let pct = |k: usize| 100.0 * k as f64 / nf;
    let ms = pct(syl.iter().filter(|&&x| x >= 3).count());
    let es = pct(syl.iter().filter(|&&x| x == 1).count());
    let iw = 100.0 * long_words / nf;
    let wstf = 0.1935 * ms + 0.1672 * asl + 0.1297 * iw - 0.0327 * es - 0.875;
    let grade = wstf.round().clamp(1.0, 13.0) as u8;
    let long: Vec<&Vec<String>> = sents.iter().filter(|x| x.len() > LONG_SENTENCE).collect();
    let long_examples = long
        .iter()
        .take(MAX_LONG_LISTED)
        .map(|x| {
            let head: Vec<&str> = x.iter().take(12).map(String::as_str).collect();
            format!("{} … ({} Wörter)", head.join(" "), x.len())
        })
        .collect();
    Readability {
        words: n,
        sentences: s,
        syllables: total_syl,
        flesch: (flesch * 10.0).round() / 10.0,
        lix: (lix * 10.0).round() / 10.0,
        grade,
        level: level(flesch).into(),
        long_sentences: long.len(),
        long_examples,
    }
}

/// Answers whose readability is shown at once, at most.
const MAX_SHOWN: usize = 50;

pub(crate) static COMMANDS: &[CommandSpec] = &[CommandSpec {
    id: "message.readability",
    label: "Lesbarkeit anzeigen",
    group: "Chat",
    shortcut: None,
    params: "message_id?: string (zeigt die Werte unter der Antwort an oder blendet sie aus), show?: bool, text?: string (nur berechnen)",
    palette: false,
    run: |c, p| Box::pin(command(c, p)),
}];

async fn command(c: Ctx, p: Value) -> Result<Value> {
    if let Some(mid) = opt_str(&p, "message_id") {
        let text = c.state().messages.iter().find(|m| m.id == mid).map(|m| m.content.clone()).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
        let show = p.get("show").and_then(Value::as_bool).unwrap_or(true);
        let r = measure(&text);
        c.update(|s| {
            if show {
                if s.readability.len() >= MAX_SHOWN
                    && let Some(k) = s.readability.keys().next().cloned()
                {
                    s.readability.remove(&k);
                }
                s.readability.insert(mid.clone(), r.clone());
            } else {
                s.readability.remove(&mid);
            }
        });
        return Ok(json!({"readability": r, "summary": r.summary(), "shown": show}));
    }
    let text = opt_str(&p, "text").ok_or_else(|| Error::invalid("message_id oder text fehlt"))?;
    let r = measure(&text);
    Ok(json!({"readability": r, "summary": r.summary()}))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn syllables_are_close() {
        for (w, n) in [("Haus", 1), ("Abend", 2), ("Schule", 2), ("Fotosynthese", 5), ("Unterrichtsstunde", 5), ("Eis", 1), ("Bäume", 2), ("x", 1)] {
            assert_eq!(syllables(w), n, "{w}");
        }
    }

    #[test]
    fn easy_text_reads_easier_than_hard_text() {
        let easy = measure("Der Hund ist klein. Er spielt im Garten. Die Sonne scheint.");
        assert_eq!((easy.words, easy.sentences), (11, 3));
        assert!(easy.flesch > 70.0, "{easy:?}");
        assert!(easy.grade <= 5, "{easy:?}");
        assert_eq!(easy.long_sentences, 0);
        let hard = measure(
            "Die Implementierung bildungspolitischer Digitalisierungsstrategien erfordert hinsichtlich der Berücksichtigung datenschutzrechtlicher Rahmenbedingungen sowie organisationaler Gegebenheiten eine umfassende, interdisziplinär abgestimmte Vorgehensweise sämtlicher beteiligter Institutionen und Verwaltungsebenen.",
        );
        assert!(hard.flesch < 20.0 && hard.grade >= 12 && hard.long_sentences == 1, "{hard:?}");
        assert!(hard.lix > easy.lix);
        assert!(hard.summary().contains("1 langer Satz") && hard.summary().contains("etwa Klasse"));
    }

    #[test]
    fn markdown_and_abbreviations() {
        let r = measure(
            "# Überschrift\n\n- erster Punkt\n- zweiter Punkt\n\nZ. B. am 3. Dezember gehen wir usw. los.\n\n```rust\nfn main() {}\n```\n\n| Name | Alter |\n|---|---|\n| Ada | zwölf |\n\n[Link](https://example.org) hier.",
        );
        // Heading, two items, one sentence with abbreviations, two table rows, the link.
        assert_eq!(r.sentences, 7, "{r:?}");
        assert!(!r.summary().contains("fn"));
        assert_eq!(measure("").words, 0);
        assert_eq!(measure("```\ncode\n```").summary(), "Kein Text");
        let _ = measure(&"Wort ".repeat(100_000));
        let _ = measure("....!!!???;;; 1. 2. 3.");
    }
}
