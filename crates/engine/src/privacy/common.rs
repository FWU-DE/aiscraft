//! Stage c without a model (#73): names that are in no class list. A built-in list of common
//! first names (`vornamen.txt`, about 3000 names: Berlin's newborns 2012–2021 under CC BY 3.0 DE,
//! plus adults' and refugee children's names, see the file's header) and context rules: "Herr/Frau
//! X", "Schülerin X", "mit X gesprochen". Every hit is unsure: replaced by default, the preview
//! lets the person keep it. Names that are also words ("Mark", "Rose", "Mai") count only with
//! context: a form of address before them, or a verb next to them and no article in front.
//! Pure functions, no I/O, no network.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::detect::{Found, Kind, Tok, tokens};

const LIST: &str = include_str!("vornamen.txt");

struct Names {
    /// Folded names that are only names.
    plain: HashSet<String>,
    /// Folded names that are also ordinary words.
    word: HashSet<String>,
}

fn names() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut n = Names { plain: HashSet::new(), word: HashSet::new() };
        for line in LIST.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            match line.strip_prefix('~') {
                Some(w) => n.word.insert(w.to_lowercase()),
                None => n.plain.insert(line.to_lowercase()),
            };
        }
        n
    })
}

/// How many names the built-in list has (handbook, settings).
pub fn count() -> usize {
    let n = names();
    n.plain.len() + n.word.len()
}

/// Forms of address after which any capitalised word is a name ("Frau Özdemir").
const ADDRESS: &[&str] =
    &["herr", "herrn", "frau", "schülerin", "mitschülerin", "kollegin", "kollege", "lehrerin", "referendarin", "erzieherin", "mutter", "vater"];
/// Words after which only a listed name counts ("Schüler" can be plural: "die Schüler Mathe …").
const ADDRESS_LISTED: &[&str] = &["schüler", "mitschüler", "kollegen", "lehrer", "referendar", "erzieher", "sohn", "tochter", "bruder", "schwester"];
/// Titles between the form of address and the name ("Frau Dr. Weber").
const TITLES: &[&str] = &["dr", "prof", "professor", "professorin"];
/// Not a name after "Frau"/"Herr" ("Frau Doktor", "Herr Lehrer").
const NOT_AFTER_ADDRESS: &[&str] = &["doktor", "direktor", "direktorin", "lehrer", "lehrerin", "kollegin", "kollege", "schulleiter", "schulleiterin", "holle"];
/// "mit X gesprochen".
const TALKED: &[&str] = &["gesprochen", "telefoniert", "geredet", "besprochen", "vereinbart", "geschrieben", "getroffen", "gestritten", "geübt", "gelernt"];
/// Articles, pronouns and prepositions: a word-like name after one of these is the word ("die Rose").
const DETERMINERS: &[&str] = &[
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "eines", "im", "am", "vom", "zum", "zur", "beim", "ins", "ans", "mein",
    "meine", "meinen", "dein", "deine", "sein", "seine", "seinen", "ihr", "ihre", "ihren", "unser", "unsere", "kein", "keine", "diese", "dieser", "dieses",
    "jede", "jeder", "jedes", "welche", "welcher", "in", "an", "auf", "aus", "bei", "nach", "von", "zu", "für", "über", "unter", "vor", "durch", "bis", "seit",
    "ab", "per", "pro", "wie", "als", "viel", "viele", "etwas", "mehr", "kleine", "kleinen", "große", "großen", "rote", "roten", "schöne",
];
/// Finite verbs that make a word-like name a person ("Mark hat gefehlt", "war Rose krank").
const VERBS: &[&str] = &[
    "ist",
    "war",
    "hat",
    "hatte",
    "wird",
    "wurde",
    "kommt",
    "kam",
    "geht",
    "ging",
    "sagt",
    "sagte",
    "meint",
    "meinte",
    "fehlt",
    "fehlte",
    "kann",
    "konnte",
    "will",
    "wollte",
    "soll",
    "sollte",
    "muss",
    "musste",
    "darf",
    "durfte",
    "möchte",
    "braucht",
    "brauchte",
    "hilft",
    "half",
    "spielt",
    "spielte",
    "lernt",
    "lernte",
    "arbeitet",
    "schreibt",
    "schrieb",
    "liest",
    "las",
    "sitzt",
    "saß",
    "stört",
    "störte",
    "redet",
    "weint",
    "weinte",
    "lacht",
    "lachte",
    "macht",
    "machte",
    "bekommt",
    "bekam",
    "schafft",
    "versteht",
    "verstand",
    "erzählt",
    "erzählte",
    "fragt",
    "fragte",
    "antwortet",
    "wohnt",
    "heißt",
];

fn is_capitalised(text: &str, t: &Tok) -> bool {
    text.get(t.start..t.end).and_then(|s| s.chars().next()).is_some_and(char::is_uppercase)
}

fn in_list(fold: &str) -> Option<bool> {
    let n = names();
    if n.plain.contains(fold) {
        return Some(false);
    }
    if n.word.contains(fold) {
        return Some(true);
    }
    // "Lea-Sophie": every part a name.
    let parts: Vec<&str> = fold.split('-').collect();
    (parts.len() > 1 && parts.iter().all(|p| n.plain.contains(*p) || n.word.contains(*p))).then_some(false)
}

/// Only white space between two words (no full stop, comma or line break).
fn joined(text: &str, a: &Tok, b: &Tok) -> bool {
    text.get(a.end..b.start).is_some_and(|s| !s.is_empty() && s.chars().all(|c| c == ' ' || c == '\t'))
}

/// Same sentence: no full stop, colon, question or exclamation mark between.
fn same_sentence(text: &str, a: &Tok, b: &Tok) -> bool {
    text.get(a.end..b.start).is_some_and(|s| !s.contains(['.', '!', '?', ':', ';', '\n']))
}

fn found(text: &str, t: &Tok, genitive: bool) -> Found {
    let end = if genitive { t.end.saturating_sub(1) } else { t.end };
    let display = text.get(t.start..end).unwrap_or("").to_string();
    Found { start: t.start, end, kind: Kind::Name, key: format!("c:{}", display.to_lowercase()), display, ambiguous: true }
}

/// Every name the list and the context rules find in `text`, in order, without overlaps.
pub fn find(text: &str) -> Vec<Found> {
    let toks = tokens(text);
    let mut hit = vec![false; toks.len()];
    let mut genitive = vec![false; toks.len()];
    for (i, t) in toks.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|p| toks.get(p)).filter(|p| same_sentence(text, p, t));
        let next = toks.get(i + 1).filter(|n| same_sentence(text, t, n));
        // Context rules: a form of address, then (a title and) a capitalised word.
        if ADDRESS.contains(&t.fold.as_str()) || ADDRESS_LISTED.contains(&t.fold.as_str()) {
            let listed_only = ADDRESS_LISTED.contains(&t.fold.as_str());
            // The address and the name are in one run of words; only a title may end in a dot.
            let mut j = i + 1;
            let mut ok = toks.get(j).is_some_and(|x| joined(text, t, x));
            while ok && let Some(x) = toks.get(j).filter(|x| TITLES.contains(&x.fold.as_str())) {
                let gap = toks.get(j + 1).and_then(|y| text.get(x.end..y.start)).unwrap_or("");
                ok = matches!(gap.trim_start_matches('.'), " " | "\t");
                j += 1;
            }
            if let Some(x) = toks.get(j)
                && ok
                && is_capitalised(text, x)
                && !NOT_AFTER_ADDRESS.contains(&x.fold.as_str())
                && !ADDRESS.contains(&x.fold.as_str())
                && (!listed_only || in_list(&x.fold).is_some())
            {
                mark(&mut hit, j);
                // "Frau Anna Schmidt": a listed first name, then the last name.
                if in_list(&x.fold).is_some()
                    && let Some(y) = toks.get(j + 1)
                    && joined(text, x, y)
                    && is_capitalised(text, y)
                    && in_list(&y.fold).is_none()
                    && !VERBS.contains(&y.fold.as_str())
                {
                    mark(&mut hit, j + 1);
                }
            }
        }
        // "mit X (Y) gesprochen".
        if t.fold == "mit" {
            for n in [1usize, 2] {
                let words: Vec<&Tok> = (1..=n).filter_map(|k| toks.get(i + k)).collect();
                let verb = toks.get(i + n + 1);
                if words.len() == n
                    && words.iter().all(|w| is_capitalised(text, w))
                    && verb.is_some_and(|v| TALKED.contains(&v.fold.as_str()))
                    && words.first().is_some_and(|w| !DETERMINERS.contains(&w.fold.as_str()) && !ADDRESS.contains(&w.fold.as_str()))
                    && words.windows(2).all(|p| matches!(p, [a, b] if joined(text, a, b)))
                    && toks.get(i + 1).is_some_and(|w| joined(text, t, w))
                {
                    for k in 1..=n {
                        mark(&mut hit, i + k);
                    }
                }
            }
        }
        // The list.
        if !is_capitalised(text, t) {
            continue;
        }
        let (fold, is_gen) = match in_list(&t.fold) {
            Some(w) => (Some(w), false),
            None => match t.fold.strip_suffix('s').map(in_list) {
                // "Pauls Heft": only for names that are only names.
                Some(Some(false)) => (Some(false), true),
                _ => (None, false),
            },
        };
        match fold {
            Some(false) => {
                mark(&mut hit, i);
                if let Some(g) = genitive.get_mut(i) {
                    *g = is_gen;
                }
            }
            Some(true) => {
                let after_article = prev.is_some_and(|p| DETERMINERS.contains(&p.fold.as_str()));
                let verb_next = prev.is_some_and(|p| VERBS.contains(&p.fold.as_str())) || next.is_some_and(|n| VERBS.contains(&n.fold.as_str()));
                if !after_article && verb_next {
                    mark(&mut hit, i);
                }
            }
            None => {}
        }
    }
    toks.iter().zip(hit).zip(genitive).filter(|((_, h), _)| *h).map(|((t, _), g)| found(text, t, g)).collect()
}

fn mark(hit: &mut [bool], i: usize) {
    if let Some(h) = hit.get_mut(i) {
        *h = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hits(text: &str) -> Vec<&str> {
        find(text).into_iter().map(|f| &text[f.start..f.end]).collect()
    }

    #[test]
    fn the_list_is_big_and_marks_words() {
        assert!(count() > 2500, "{}", count());
        assert_eq!(in_list("klaus"), Some(false));
        assert_eq!(in_list("mark"), Some(true));
        assert_eq!(in_list("rose"), Some(true));
        assert_eq!(in_list("lea-sophie"), Some(false));
        assert_eq!(in_list("tisch"), None);
    }

    #[test]
    fn common_names_without_a_class_list() {
        assert_eq!(hits("Gestern war Klaus wieder unruhig."), vec!["Klaus"]);
        assert_eq!(hits("Zeynep und Mehmet haben mit Oleksandr gespielt."), vec!["Zeynep", "Mehmet", "Oleksandr"]);
        assert_eq!(hits("Pauls Heft liegt bei Lea-Sophie."), vec!["Paul", "Lea-Sophie"]);
        let f = find("Pauls Heft");
        assert_eq!((f[0].kind, f[0].ambiguous, f[0].key.as_str()), (Kind::Name, true, "c:paul"));
        // Lower case is no name ("ali" in a word list, "paul" as a typo stays).
        assert!(hits("der tisch, die klasse, paul").is_empty());
    }

    #[test]
    fn words_that_are_names_need_context() {
        assert!(hits("Das Mark der Knochen. Die Rose ist rot. Im Mai fahren wir. Koch das Essen.").is_empty());
        assert_eq!(hits("Mark hat heute gefehlt, und gestern war Rose krank."), vec!["Mark", "Rose"]);
        assert_eq!(hits("Ich habe mit Frau Koch gesprochen."), vec!["Koch"]);
    }

    #[test]
    fn an_ordinary_letter_has_no_hits() {
        let t = "Liebe Eltern, die Klasse hat heute im Unterricht über Photosynthese gesprochen. Am Montag schreiben wir eine \
                 Arbeit in Mathematik. Bitte bringt Lineal, Zirkel und Taschenrechner mit. Der Ausflug nach Potsdam findet im Juni \
                 statt, die Kosten betragen 12 Euro. Für Fragen bin ich in der Sprechstunde am Dienstag da. Viele Grüße, Ihre \
                 Klassenleitung. PS: Die Bücherei hat neue Bücher über Rom, Ägypten und das Mittelalter.";
        assert!(hits(t).is_empty(), "{:?}", hits(t));
    }

    #[test]
    fn forms_of_address_and_talking() {
        assert_eq!(hits("Herr Özdemir und Frau Dr. Weber, Frau Anna Schmidt kommt."), vec!["Özdemir", "Weber", "Anna", "Schmidt"]);
        assert_eq!(hits("Ich habe mit Bettel Wunderlich gesprochen."), vec!["Bettel", "Wunderlich"]);
        assert_eq!(hits("Die Schülerin Aylin fehlt. Die Schüler Mathe lernen."), vec!["Aylin"]);
        assert!(hits("Frau Holle, Frau Doktor, Herr der Ringe, mit dem Buch gearbeitet").is_empty());
        // A sentence end is not a form of address.
        assert!(hits("Das sagte die Frau. Später kam der Bus.").is_empty());
    }
}
