//! Finding personal data in a text without a model (#55): names from class lists (stage a) and
//! fixed formats by rule and checksum (stage b). Pure functions, no I/O.
//!
//! Stage c finds names that are in no list: a built-in list of common first names and context
//! rules ([`super::common`], switched on with [`Detector::with_common_names`]), and, once the
//! person downloaded it, a local GLiNER model ([`super::ner`], [`Detector::with_model`]).
//! [`Detector::detect`] merges whatever its sources find, rule findings first, then class-list
//! names, longest first, then the common names, then the model's. Findings of stage c are never
//! "sure" on their own (replaced by default, the person can keep them in the preview).

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// What kind of personal data a finding is. The placeholder says it: `[S1]` (first name of a
/// pupil), `[N1]` (their last name), `[Name 1]` (a term added by hand), `[E-Mail 1]` …
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    FirstName,
    LastName,
    Name,
    Email,
    Phone,
    Iban,
    Birthdate,
    Address,
}

impl Kind {
    /// As people read it (preview, manual).
    pub fn label(self) -> &'static str {
        match self {
            Kind::FirstName => "Vorname",
            Kind::LastName => "Nachname",
            Kind::Name => "Name",
            Kind::Email => "E-Mail",
            Kind::Phone => "Telefon",
            Kind::Iban => "IBAN",
            Kind::Birthdate => "Geburtsdatum",
            Kind::Address => "Adresse",
        }
    }
}

/// One pupil of a class list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    #[serde(default)]
    pub first: String,
    #[serde(default)]
    pub last: String,
}

impl Person {
    pub fn full(&self) -> String {
        format!("{} {}", self.first, self.last).trim().to_string()
    }
}

/// One finding: bytes `start..end` of the text.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    /// Identity: the same person or value always has the same key (and so the same placeholder).
    pub key: String,
    /// What the placeholder stands for when it is translated back.
    pub display: String,
    /// Could be an ordinary word or someone else ("Mark", "Hoffmann", "max."): not replaced
    /// unless the person confirms it in the preview.
    pub ambiguous: bool,
}

/// Something already in a chat's mapping: matched like a list name, and never ambiguous.
#[derive(Debug, Clone)]
pub struct Known {
    pub kind: Kind,
    pub key: String,
    pub display: String,
}

/// Names that are also ordinary German words, common surnames that are words, or authors and
/// other people a lesson talks about. Alone they are never replaced without a yes.
const AMBIGUOUS: &[&str] = &[
    // first names that are words
    "mark",
    "max",
    "kai",
    "rose",
    "rosa",
    "iris",
    "erika",
    "heide",
    "jasmin",
    "flora",
    "viola",
    "frank",
    "ernst",
    "will",
    "paris",
    "india",
    "linde",
    "mai",
    "sonne",
    "engel",
    "christ",
    "fee",
    "sky",
    "summer",
    "ali",
    "elf",
    "wolf",
    "art",
    "april",
    "june",
    "august",
    "gabriel",
    "hans",
    "karl",
    "otto",
    "dana",
    "gloria",
    "amor",
    "luna",
    "stella",
    "sina",
    "maja",
    "mia",
    "ella",
    "lina",
    "nina",
    "tim",
    "tom",
    "ben",
    // surnames that are words
    "fuchs",
    "bär",
    "baer",
    "vogel",
    "stern",
    "winter",
    "sommer",
    "lange",
    "klein",
    "groß",
    "gross",
    "kurz",
    "lang",
    "jung",
    "alt",
    "neu",
    "weiß",
    "weiss",
    "schwarz",
    "roth",
    "rot",
    "braun",
    "grün",
    "gruen",
    "kaiser",
    "könig",
    "koenig",
    "graf",
    "herr",
    "mann",
    "frau",
    "kind",
    "koch",
    "fischer",
    "schneider",
    "bauer",
    "becker",
    "wagner",
    "weber",
    "meyer",
    "meier",
    "hoffmann",
    "schulz",
    "schulze",
    "richter",
    "hahn",
    "berg",
    "busch",
    "hase",
    "haas",
    "kraft",
    "freund",
    "glück",
    "glueck",
    "friede",
    "liebe",
    "schmied",
    "schmidt",
    "schröder",
    "schroeder",
    "müller",
    "mueller",
    "zimmermann",
    "hofmann",
    "neumann",
    "krüger",
    "krueger",
    "martin",
    "frei",
    "fromm",
    "gut",
    "böse",
    "reich",
    "arm",
    "blume",
    "baum",
    "hecht",
    "adler",
    "falke",
    "lamm",
    "kuhn",
    "ochs",
    "strauß",
    "strauss",
    "rabe",
    "fink",
    "specht",
    "storch",
    "wald",
    "feld",
    "bach",
    "brunnen",
    "stein",
    "eisen",
    "gold",
    "silber",
    "kupfer",
    "marx",
    "hering",
    "lachs",
    "kranz",
    "krone",
    "ring",
    "schuster",
    "wirth",
    "wirt",
    "pfeiffer",
    "pfeifer",
    "jäger",
    "jaeger",
    "förster",
    "foerster",
    "vetter",
    "sauer",
    "süß",
    "suess",
    "frost",
    "regen",
    "wind",
    "sturm",
    "blitz",
    "donner",
    "nebel",
    "moos",
    // people lessons talk about
    "goethe",
    "schiller",
    "lessing",
    "kafka",
    "brecht",
    "fontane",
    "storm",
    "heine",
    "keller",
    "kleist",
    "büchner",
    "buechner",
    "grass",
    "hesse",
    "böll",
    "boell",
    "frisch",
    "zweig",
    "kästner",
    "kaestner",
    "ende",
    "funke",
    "preußler",
    "preussler",
    "rilke",
    "trakl",
    "benn",
    "kant",
    "hegel",
    "luther",
    "mozart",
    "beethoven",
    "einstein",
    "newton",
    "darwin",
    "curie",
    "napoleon",
    "caesar",
    "cäsar",
    "hitler",
    "bismarck",
    "merkel",
    "grimm",
    "andersen",
    "lindgren",
    "astrid",
    "pippi",
    "emil",
    "momo",
    "faust",
];

fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// A word of the text: letters and digits, joined by inner hyphens ("Lea-Sophie").
#[derive(Debug, Clone)]
pub(super) struct Tok {
    pub start: usize,
    pub end: usize,
    pub fold: String,
}

pub(super) fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let word = c.is_alphanumeric();
        let joiner = c == '-' && start.is_some() && chars.peek().is_some_and(|(_, n)| n.is_alphanumeric());
        match (start, word || joiner) {
            (None, true) if word => start = Some(i),
            (Some(s), false) => {
                out.push(Tok { start: s, end: i, fold: fold(text.get(s..i).unwrap_or("")) });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push(Tok { start: s, end: text.len(), fold: fold(text.get(s..).unwrap_or("")) });
    }
    out
}

/// A name to look for, as folded words.
#[derive(Debug, Clone)]
struct Pattern {
    words: Vec<String>,
    kind: Kind,
    /// Who it may be: (key, display). More than one: a first name several pupils share.
    groups: Vec<(String, String)>,
    /// From a chat's mapping: sure, and this key wins.
    known: Option<(String, String)>,
    /// The list writes it with capitals (so "max" in lower case is probably not "Max").
    capitalised: bool,
    word_like: bool,
}

/// Finds personal data in texts: built once per request from the class lists, the chat's mapping
/// and terms added by hand.
pub struct Detector {
    patterns: HashMap<String, Vec<Pattern>>,
    year: i32,
    common: bool,
    model: Option<super::ner::NameModel>,
}

/// Longest list entries a detector takes (a class list is 30 names, a school maybe 2000).
const MAX_PATTERNS: usize = 20_000;

impl Detector {
    /// `year`: the current year, for telling birth dates from other dates.
    pub fn new(people: &[Person], known: &[Known], manual: &[String], year: i32) -> Self {
        let mut all: BTreeMap<(Vec<String>, Kind), Pattern> = BTreeMap::new();
        let mut add = |text: &str, kind: Kind, key: String, known_entry: bool| {
            let t = text.trim();
            let words: Vec<String> = tokens(t).into_iter().map(|w| w.fold).collect();
            if words.is_empty() || words.iter().all(|w| w.chars().count() < 2) || all.len() >= MAX_PATTERNS {
                return;
            }
            let word_like = words.len() == 1 && words.first().is_some_and(|w| AMBIGUOUS.contains(&w.as_str()));
            let p = all.entry((words.clone(), kind)).or_insert_with(|| Pattern {
                words,
                kind,
                groups: Vec::new(),
                known: None,
                capitalised: t.chars().any(char::is_uppercase),
                word_like,
            });
            if known_entry {
                p.known = Some((key, t.to_string()));
            } else if !p.groups.iter().any(|(k, _)| *k == key) {
                p.groups.push((key, t.to_string()));
            }
        };
        for p in people {
            let key = format!("p:{}|{}", fold(p.first.trim()), fold(p.last.trim()));
            if !p.first.trim().is_empty() {
                add(&p.first, Kind::FirstName, key.clone(), false);
                // "Lea Marie": the first given name is usually the one people use.
                let first_word: Vec<&str> = p.first.split_whitespace().collect();
                if first_word.len() > 1
                    && let Some(w) = first_word.first()
                {
                    add(w, Kind::FirstName, key.clone(), false);
                }
            }
            if !p.last.trim().is_empty() {
                add(&p.last, Kind::LastName, key, false);
            }
        }
        for k in known {
            add(&k.display, k.kind, k.key.clone(), true);
        }
        for m in manual {
            add(m, Kind::Name, format!("m:{}", fold(m.trim())), false);
        }
        let mut patterns: HashMap<String, Vec<Pattern>> = HashMap::new();
        for (_, p) in all {
            if let Some(first) = p.words.first().cloned() {
                patterns.entry(first).or_default().push(p);
            }
        }
        Detector { patterns, year, common: false, model: None }
    }

    /// Also find names that are in no list (stage c without a model, [`super::common`]).
    pub fn with_common_names(mut self) -> Self {
        self.common = true;
        self
    }

    /// Also the names a model finds (stage c with the downloaded model, [`super::ner`]).
    pub fn with_model(mut self, model: Option<super::ner::NameModel>) -> Self {
        self.model = model;
        self
    }

    /// Everything found in `text`, in order, without overlaps.
    pub fn detect(&self, text: &str) -> Vec<Found> {
        let mut found = rules(text, self.year);
        let mut names = self.names(text);
        if self.common {
            names.extend(super::common::find(text));
        }
        if let Some(m) = &self.model {
            names.extend(model_hits(text, &m(text)));
        }
        for n in names {
            if !found.iter().any(|f| n.start < f.end && f.start < n.end) {
                found.push(n);
            }
        }
        found.sort_by_key(|f| f.start);
        found
    }

    fn names(&self, text: &str) -> Vec<Found> {
        let toks = tokens(text);
        // (start, end, words, pattern)
        let mut cands: Vec<(usize, usize, usize, &Pattern)> = Vec::new();
        for (i, t) in toks.iter().enumerate() {
            let genitive = t.fold.strip_suffix('s').filter(|s| s.chars().count() >= 2 && text.get(t.end.saturating_sub(1)..t.end) == Some("s"));
            let direct = self.patterns.get(&t.fold).into_iter().flatten().map(|p| (p, false));
            let by_gen = genitive.and_then(|g| self.patterns.get(g)).into_iter().flatten().filter(|p| p.words.len() == 1).map(|p| (p, true));
            for (p, is_gen) in direct.chain(by_gen) {
                if let Some(end) = self.match_at(text, &toks, i, p, is_gen) {
                    cands.push((t.start, end, p.words.len(), p));
                }
            }
        }
        cands.sort_by(|a, b| b.2.cmp(&a.2).then((b.1 - b.0).cmp(&(a.1 - a.0))).then(a.0.cmp(&b.0)));
        let mut picked: Vec<(usize, usize, &Pattern)> = Vec::new();
        for (s, e, _, p) in cands {
            if !picked.iter().any(|(ps, pe, _)| s < *pe && *ps < e) {
                picked.push((s, e, p));
            }
        }
        picked.sort_by_key(|x| x.0);
        // First and last name next to each other ("Lea Müller", "Müller, Lea"): the same pupil.
        let mut resolved: Vec<Option<String>> = vec![None; picked.len()];
        for i in 0..picked.len().saturating_sub(1) {
            let (Some(a), Some(b)) = (picked.get(i), picked.get(i + 1)) else { continue };
            let between = text.get(a.1..b.0).unwrap_or("x");
            let pair = match (a.2.kind, b.2.kind) {
                (Kind::FirstName, Kind::LastName) => !between.is_empty() && between.trim().is_empty(),
                (Kind::LastName, Kind::FirstName) => between.trim() == "," || (!between.is_empty() && between.trim().is_empty()),
                _ => false,
            };
            if !pair {
                continue;
            }
            let keys = |p: &Pattern| -> Vec<String> {
                let mut k: Vec<String> = p.groups.iter().map(|(k, _)| k.clone()).collect();
                if let Some((kk, _)) = &p.known {
                    k.push(kk.clone());
                }
                k
            };
            let (ka, kb) = (keys(a.2), keys(b.2));
            if let Some(common) = ka.iter().find(|k| k.starts_with("p:") && kb.contains(k)) {
                if let Some(r) = resolved.get_mut(i) {
                    *r = Some(common.clone());
                }
                if let Some(r) = resolved.get_mut(i + 1) {
                    *r = Some(common.clone());
                }
            }
        }
        let mut out = Vec::new();
        for ((s, e, p), res) in picked.into_iter().zip(resolved) {
            let surface = text.get(s..e).unwrap_or("");
            let lower = !surface.chars().any(char::is_uppercase);
            let display_of = |key: &str| -> String {
                p.groups
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, d)| d.clone())
                    .or_else(|| p.known.as_ref().map(|(_, d)| d.clone()))
                    .unwrap_or_else(|| surface.to_string())
            };
            let (key, display, sure) = if let Some(k) = res {
                let d = display_of(&k);
                (k, d, true)
            } else if let Some((k, d)) = &p.known {
                (k.clone(), d.clone(), true)
            } else if let [(k, d)] = p.groups.as_slice() {
                (k.clone(), d.clone(), false)
            } else {
                // A first or last name several pupils share: its own placeholder.
                let prefix = if p.kind == Kind::LastName { "n" } else { "v" };
                let d = p.groups.first().map(|(_, d)| d.clone()).unwrap_or_else(|| surface.to_string());
                (format!("{prefix}:{}", p.words.join(" ")), d, false)
            };
            let ambiguous = !sure && (p.word_like || (lower && p.capitalised));
            out.push(Found { start: s, end: e, kind: p.kind, key, display, ambiguous });
        }
        out
    }

    /// End byte of the name if `p` matches at word `i` (a genitive "s" stays outside).
    fn match_at(&self, text: &str, toks: &[Tok], i: usize, p: &Pattern, genitive: bool) -> Option<usize> {
        let n = p.words.len();
        for (j, w) in p.words.iter().enumerate() {
            let t = toks.get(i + j)?;
            let last = j + 1 == n;
            let ok = if last && genitive { t.fold.strip_suffix('s') == Some(w.as_str()) } else { &t.fold == w };
            if !ok {
                return None;
            }
            if j > 0 {
                let prev = toks.get(i + j - 1)?;
                let between = text.get(prev.end..t.start)?;
                if between.is_empty() || !between.trim().is_empty() {
                    return None;
                }
            }
        }
        let last = toks.get(i + n - 1)?;
        Some(if genitive { last.end.saturating_sub(1) } else { last.end })
    }
}

// ---- the model's names (stage c with a model) ---------------------------------------------------

/// Words of the model's spans as unsure name findings, one per capitalised word and with the
/// same key as the first-name list (`c:klaus`), so "Klaus" gets one placeholder whichever source
/// found it. Lower-case words in a name ("von der") stay; a placeholder already in the text
/// (`[S1]`, `[Name 2]`) is never a name again.
pub(super) fn model_hits(text: &str, spans: &[(usize, usize)]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for &(s, e) in spans.iter().take(10_000) {
        let Some(span) = text.get(s..e) else { continue };
        if e - s > 200 || span.contains(['[', ']']) || text.get(..s).is_some_and(|b| b.ends_with('[')) || text.get(e..).is_some_and(|a| a.starts_with(']')) {
            continue;
        }
        for t in tokens(span) {
            let (a, b) = (s + t.start, s + t.end);
            let Some(word) = text.get(a..b) else { continue };
            if !word.chars().next().is_some_and(char::is_uppercase) || word.chars().count() < 2 || word.chars().any(|c| c.is_ascii_digit()) {
                continue;
            }
            if out.iter().any(|f| a < f.end && f.start < b) {
                continue;
            }
            out.push(Found { start: a, end: b, kind: Kind::Name, key: format!("c:{}", word.to_lowercase()), display: word.to_string(), ambiguous: true });
        }
    }
    out.sort_by_key(|f| f.start);
    out
}

// ---- rules (stage b) -------------------------------------------------------------------------

struct Rules {
    email: Regex,
    iban: Regex,
    phone_intl: Regex,
    phone_nat: Regex,
    date_num: Regex,
    date_word: Regex,
    street: Regex,
    street_prep: Regex,
}

fn compiled() -> Option<&'static Rules> {
    static RULES: OnceLock<Option<Rules>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            let street_suffix = r"(?:straße|strasse|str\.|weg|gasse|allee|platz|ring|damm|ufer|chaussee|pfad|steig|stieg|markt|promenade|zeile)";
            let plz = r"(?:,?\s+\d{5}\s+[A-ZÄÖÜ][\p{L}\-]+(?:\s+(?:am|an\s+der|im|bei)\s+[A-ZÄÖÜ][\p{L}]+)?)?";
            Some(Rules {
                email: Regex::new(r"(?i)\b[a-z0-9][a-z0-9._%+\-]*@[a-z0-9][a-z0-9.\-]*\.[a-z]{2,24}\b").ok()?,
                iban: Regex::new(r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]){11,30}\b").ok()?,
                phone_intl: Regex::new(r"(?:\+|\b00)49[ \-/]?(?:\(0\)[ \-/]?)?\(?\d{2,5}\)?[ \-/]?\d[\d \-/]{3,14}\d\b").ok()?,
                phone_nat: Regex::new(r"(?:\(0\d{2,5}\)|\b0\d{2,5})[ \-/]?\d[\d \-/]{3,12}\d\b").ok()?,
                date_num: Regex::new(r"\b(0?[1-9]|[12]\d|3[01])\.\s?(0?[1-9]|1[0-2])\.\s?((?:19|20)\d{2})\b").ok()?,
                date_word: Regex::new(
                    r"(?i)\b(0?[1-9]|[12]\d|3[01])\.\s?(januar|jänner|februar|märz|maerz|april|mai|juni|juli|august|september|oktober|november|dezember)\s+((?:19|20)\d{2})\b",
                )
                .ok()?,
                street: Regex::new(&format!(
                    r"\b(?:[A-ZÄÖÜ][\p{{L}}]+\s)?[A-ZÄÖÜ][\p{{L}}]*(?:-[\p{{L}}]+)*{street_suffix}\s?\d{{1,4}}\s?[a-zA-Z]?\b{plz}"
                ))
                .ok()?,
                street_prep: Regex::new(&format!(
                    r"\b(?:Am|An\s+der|An\s+den|Im|In\s+der|Auf\s+der|Auf\s+dem|Zum|Zur|Unter\s+den)\s+[A-ZÄÖÜ][\p{{L}}\-]+\s\d{{1,3}}[a-z]?\b{plz}"
                ))
                .ok()?,
            })
        })
        .as_ref()
}

/// ISO 13616 check (mod 97 = 1) on an IBAN without spaces.
pub fn iban_valid(iban: &str) -> bool {
    let len = iban.len();
    if !(15..=34).contains(&len) || !iban.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    if iban.starts_with("DE") && len != 22 {
        return false;
    }
    let (head, tail) = (iban.get(..4).unwrap_or(""), iban.get(4..).unwrap_or(""));
    let mut rem: u32 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => v,
            None => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

fn digits(s: &str) -> String {
    s.chars().filter(char::is_ascii_digit).collect()
}

/// The text just before `at`, at most `n` characters.
fn before(text: &str, at: usize, n: usize) -> String {
    let head = text.get(..at).unwrap_or("");
    let rev: String = head.chars().rev().take(n).collect();
    rev.chars().rev().collect::<String>().to_lowercase()
}

const MONTHS: &[&str] = &["januar", "februar", "märz", "april", "mai", "juni", "juli", "august", "september", "oktober", "november", "dezember"];

fn rules(text: &str, year: i32) -> Vec<Found> {
    let Some(r) = compiled() else { return Vec::new() };
    let mut out: Vec<Found> = Vec::new();
    let free = |out: &Vec<Found>, s: usize, e: usize| !out.iter().any(|f| s < f.end && f.start < e);
    for m in r.email.find_iter(text) {
        out.push(Found {
            start: m.start(),
            end: m.end(),
            kind: Kind::Email,
            key: m.as_str().to_lowercase(),
            display: m.as_str().to_string(),
            ambiguous: false,
        });
    }
    for m in r.iban.find_iter(text) {
        // The pattern may run into the next word: shorten until the checksum fits.
        let s = m.as_str();
        let mut end = s.len();
        while end >= 15 {
            let cand = s.get(..end).unwrap_or("").trim_end();
            let compact: String = cand.chars().filter(|c| !c.is_whitespace()).collect();
            if iban_valid(&compact) {
                let (a, b) = (m.start(), m.start() + cand.len());
                if free(&out, a, b) {
                    out.push(Found { start: a, end: b, kind: Kind::Iban, key: compact, display: cand.to_string(), ambiguous: false });
                }
                break;
            }
            end -= 1;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
        }
    }
    for m in r.date_num.find_iter(text).chain(r.date_word.find_iter(text)) {
        if !free(&out, m.start(), m.end()) {
            continue;
        }
        let Some(c) = r.date_num.captures(m.as_str()).or_else(|| r.date_word.captures(m.as_str())) else { continue };
        let day: u32 = c.get(1).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
        let month_raw = c.get(2).map(|x| x.as_str().to_lowercase()).unwrap_or_default();
        let month: u32 =
            month_raw.parse().ok().or_else(|| MONTHS.iter().position(|mm| *mm == month_raw).map(|p| p as u32 + 1)).unwrap_or(match month_raw.as_str() {
                "jänner" => 1,
                "maerz" => 3,
                _ => 0,
            });
        let y: i32 = c.get(3).and_then(|x| x.as_str().parse().ok()).unwrap_or(0);
        let ctx = before(text, m.start(), 24);
        let context = ctx.contains("geb") || ctx.contains("geburt") || ctx.trim_end().ends_with('*');
        let school_age = (year - 25..=year - 3).contains(&y);
        if !(context || school_age) {
            continue;
        }
        out.push(Found {
            start: m.start(),
            end: m.end(),
            kind: Kind::Birthdate,
            key: format!("{day:02}.{month:02}.{y}"),
            display: m.as_str().to_string(),
            ambiguous: !context,
        });
    }
    for (re, sure) in [(&r.phone_intl, true), (&r.phone_nat, true)] {
        for m in re.find_iter(text) {
            let d = digits(m.as_str());
            if !(7..=15).contains(&d.len()) || !free(&out, m.start(), m.end()) {
                continue;
            }
            let key = if let Some(rest) = d.strip_prefix("0049") {
                format!("0{rest}")
            } else if m.as_str().trim_start().starts_with('+') {
                format!("0{}", d.strip_prefix("49").unwrap_or(&d))
            } else {
                d.clone()
            };
            let key = key.replacen("00", "0", usize::from(key.starts_with("00")));
            out.push(Found { start: m.start(), end: m.end(), kind: Kind::Phone, key, display: m.as_str().to_string(), ambiguous: !sure });
        }
    }
    for (re, sure_alone) in [(&r.street, true), (&r.street_prep, false)] {
        for m in re.find_iter(text) {
            let s = m.as_str().trim_end();
            if !free(&out, m.start(), m.start() + s.len()) {
                continue;
            }
            let has_plz = s.chars().filter(char::is_ascii_digit).count() >= 5 && s.contains(|c: char| c == ',' || c.is_whitespace());
            let plz = s.split(|c: char| !c.is_ascii_digit()).any(|p| p.len() == 5);
            let key = s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            out.push(Found {
                start: m.start(),
                end: m.start() + s.len(),
                kind: Kind::Address,
                key,
                display: s.to_string(),
                ambiguous: !(sure_alone || (has_plz && plz)),
            });
        }
    }
    out.sort_by_key(|f| f.start);
    out
}

// ---- class lists from files --------------------------------------------------------------------

/// At most this many pupils per list.
pub const MAX_PEOPLE: usize = 5000;

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect()
}

/// "Lea Müller", "Müller, Lea", "Lea" (one per line or cell).
pub fn person_from_text(s: &str) -> Option<Person> {
    let s = clean(s.trim_matches(|c: char| c == '"' || c == ';' || c.is_whitespace()));
    if s.is_empty() || !s.chars().any(char::is_alphabetic) {
        return None;
    }
    if let Some((last, first)) = s.split_once(',') {
        return Some(Person { first: clean(first), last: clean(last) });
    }
    let words: Vec<&str> = s.split(' ').collect();
    match words.as_slice() {
        [one] => Some(Person { first: (*one).to_string(), last: String::new() }),
        [first @ .., last] => {
            // "Lea von der Heide": particles belong to the last name.
            let particles = ["von", "van", "de", "der", "den", "zu", "vom", "zur", "di", "da", "le", "la", "ten", "ter"];
            let mut split = first.len();
            while split > 1 && first.get(split - 1).is_some_and(|w| particles.contains(&w.to_lowercase().as_str())) {
                split -= 1;
            }
            let given = first.get(..split).unwrap_or(&[]).join(" ");
            let rest = first.get(split..).unwrap_or(&[]).iter().copied().chain(std::iter::once(*last)).collect::<Vec<_>>().join(" ");
            Some(Person { first: given, last: rest })
        }
        [] => None,
    }
}

/// Pupils from pasted text, one per line.
pub fn people_from_lines(text: &str) -> Vec<Person> {
    text.lines().filter_map(person_from_text).take(MAX_PEOPLE).collect()
}

fn header_col(row: &[String], names: &[&str]) -> Option<usize> {
    row.iter().position(|c| {
        let f = fold(c.trim()).replace(['_', '-'], " ");
        names.contains(&f.as_str())
    })
}

/// Pupils from table rows (CSV, a sheet). A header names the columns (Vorname, Nachname, Name,
/// Rufname, Schüler …); without one, two columns are read as "Nachname, Vorname" (how school
/// lists are sorted), one column as names as written.
pub fn people_from_rows(rows: &[Vec<String>]) -> (Vec<Person>, String) {
    let firsts = ["vorname", "vornamen", "rufname", "first name", "firstname", "given name", "givenname"];
    let lasts = ["nachname", "familienname", "zuname", "name", "last name", "lastname", "surname", "family name"];
    let fulls = ["schüler", "schülerin", "schüler/in", "schüler*in", "schülerinnen", "name, vorname", "vollständiger name", "kind", "full name"];
    let Some(head) = rows.first() else { return (Vec::new(), String::new()) };
    let call = header_col(head, &["rufname"]);
    let first = call.or_else(|| header_col(head, &firsts));
    let last = header_col(head, &lasts);
    let full = header_col(head, &fulls);
    let mut out = Vec::new();
    let cell = |r: &Vec<String>, i: Option<usize>| i.and_then(|i| r.get(i)).map(|s| clean(s)).unwrap_or_default();
    let how;
    if first.is_some() || (last.is_some() && full.is_none() && head.len() > 1 && first.is_some()) {
        how = "Spalten Vorname und Nachname".to_string();
        for r in rows.iter().skip(1) {
            let p = Person { first: cell(r, first), last: cell(r, last) };
            if !p.first.is_empty() || !p.last.is_empty() {
                out.push(p);
            }
        }
    } else if let Some(col) = full.or(last) {
        how = format!("Spalte „{}“", head.get(col).map(|s| s.trim()).unwrap_or(""));
        out.extend(rows.iter().skip(1).filter_map(|r| r.get(col).and_then(|c| person_from_text(c))));
    } else {
        let wide = rows.iter().filter(|r| r.iter().filter(|c| !c.trim().is_empty()).count() >= 2).count() * 2 > rows.len();
        if wide {
            how = "zwei Spalten, gelesen als Nachname, Vorname".to_string();
            for r in rows {
                let cells: Vec<String> = r.iter().map(|c| clean(c)).filter(|c| !c.is_empty()).collect();
                if let [last, first, ..] = cells.as_slice() {
                    out.push(Person { first: first.clone(), last: last.clone() });
                }
            }
        } else {
            how = "eine Spalte mit Namen".to_string();
            out.extend(rows.iter().filter_map(|r| r.iter().find(|c| !c.trim().is_empty()).and_then(|c| person_from_text(c))));
        }
    }
    out.retain(|p| (p.first.chars().any(char::is_alphabetic) || p.last.chars().any(char::is_alphabetic)) && !p.first.chars().any(|c| c.is_ascii_digit()));
    out.truncate(MAX_PEOPLE);
    (out, how)
}

/// Pupils from vCards (contacts exported from an address book).
pub fn people_from_vcards(bytes: &[u8]) -> Vec<Person> {
    craft_vcard::parse_bytes(bytes)
        .into_iter()
        .filter_map(|c| {
            if !c.first.trim().is_empty() || !c.last.trim().is_empty() {
                Some(Person { first: clean(&c.first), last: clean(&c.last) })
            } else {
                person_from_text(&c.name())
            }
        })
        .take(MAX_PEOPLE)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people() -> Vec<Person> {
        ["Lea Müller", "Jonas Schmidt", "Mark Hoffmann", "Ben Koch", "Lea Wagner", "Anna-Lena Yılmaz"].iter().filter_map(|s| person_from_text(s)).collect()
    }

    fn det() -> Detector {
        Detector::new(&people(), &[], &[], 2026)
    }

    fn texts(text: &str) -> Vec<(String, Kind, bool)> {
        det().detect(text).into_iter().map(|f| (text[f.start..f.end].to_string(), f.kind, f.ambiguous)).collect()
    }

    #[test]
    fn names_inflected_case_insensitive_and_bounded() {
        let t = "Jonas' Heft und Schmidts Mappe; JONAS lacht. Jonasson nicht, Jona nicht.";
        assert_eq!(
            texts(t),
            vec![("Jonas".into(), Kind::FirstName, false), ("Schmidt".into(), Kind::LastName, true), ("JONAS".into(), Kind::FirstName, false),]
        );
        // "Schmidt" alone is a word-like surname, next to the first name it is sure.
        let f = det().detect("Jonas Schmidt kam.");
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| !x.ambiguous && x.key == "p:jonas|schmidt"));
        let f = det().detect("Schmidt, Jonas kam.");
        assert!(f.iter().all(|x| !x.ambiguous && x.key == "p:jonas|schmidt"));
    }

    #[test]
    fn shared_first_names_word_like_names_and_hyphens() {
        let d = det();
        // Two pupils called Lea: alone it gets its own key; with the last name, the pupil's.
        let f = d.detect("Lea und Lea Wagner");
        assert_eq!(f[0].key, "v:lea");
        assert_eq!(f[1].key, "p:lea|wagner");
        assert_eq!(f[2].key, "p:lea|wagner");
        // "Mark" alone could be the word; "max." in lower case is not Max.
        let f = d.detect("Das Mark der Knochen. Mark Hoffmann fehlt.");
        assert!(f[0].ambiguous);
        assert!(!f[1].ambiguous && !f[2].ambiguous);
        assert_eq!(texts("Anna-Lena kommt, Anna nicht, Lena nicht"), vec![("Anna-Lena".into(), Kind::FirstName, false)]);
        assert_eq!(texts("Leas Mappe")[0].0, "Lea", "genitive s stays outside");
        assert!(texts("ben").first().is_some_and(|x| x.2), "lower case is unsure");
    }

    #[test]
    fn known_entries_are_sure() {
        let known = [Known { kind: Kind::FirstName, key: "p:mark|hoffmann".into(), display: "Mark".into() }];
        let d = Detector::new(&people(), &known, &["Frau Özdemir".into()], 2026);
        let f = d.detect("Mark und Frau Özdemir");
        assert!(!f[0].ambiguous);
        assert_eq!(f[1].kind, Kind::Name);
    }

    #[test]
    fn rules_find_formats() {
        let d = Detector::new(&[], &[], &[], 2026);
        let t = "Mail: lea.mueller@schule-berlin.de, Tel. 030 12345678 oder +49 (0) 171 234 56 78. \
                 IBAN DE89 3704 0044 0532 0130 00 bitte. geb. 03.05.2014, wohnt Lindenstraße 5, 10115 Berlin. \
                 Am 14.11.2026 Elternabend, 2014 Kinder, Seite 0301.";
        let f = d.detect(t);
        let kinds: Vec<(Kind, &str, bool)> = f.iter().map(|x| (x.kind, &t[x.start..x.end], x.ambiguous)).collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Email, "lea.mueller@schule-berlin.de", false),
                (Kind::Phone, "030 12345678", false),
                (Kind::Phone, "+49 (0) 171 234 56 78", false),
                (Kind::Iban, "DE89 3704 0044 0532 0130 00", false),
                (Kind::Birthdate, "03.05.2014", false),
                (Kind::Address, "Lindenstraße 5, 10115 Berlin", false),
            ]
        );
        assert!(!iban_valid("DE89370400440532013001"));
        assert!(iban_valid("GB82WEST12345698765432"));
        // A date of school age without "geb." is unsure; other dates are no birth dates.
        let f = d.detect("Seit 12.08.2015 dabei, Projekt am 01.02.2026");
        assert_eq!(f.len(), 1);
        assert!(f[0].ambiguous);
        assert_eq!(d.detect("Im Jahr 2020 und Am Montag 3 Stunden").iter().filter(|x| !x.ambiguous).count(), 0);
    }

    #[test]
    fn model_spans_become_unsure_words() {
        let t = "Lea von der Heide traf [Name 1] und Weber. ä";
        let at = |w: &str| t.find(w).map(|i| (i, i + w.len())).unwrap();
        let spans = [at("Lea von der Heide"), at("Name 1"), at("Weber"), (t.len() - 1, t.len()), (5, 3), (0, 999)];
        let f = model_hits(t, &spans);
        let words: Vec<&str> = f.iter().map(|x| &t[x.start..x.end]).collect();
        assert_eq!(words, vec!["Lea", "Heide", "Weber"]);
        assert!(f.iter().all(|x| x.ambiguous && x.kind == Kind::Name && x.key.starts_with("c:")));
        // The class list wins where both find something; the model adds the rest.
        let d = Detector::new(&people(), &[], &[], 2026).with_model(Some(std::sync::Arc::new(|t: &str| vec![(0, t.find(' ').unwrap_or(0))])));
        let f = d.detect("Lea kam.");
        assert_eq!((f.len(), f[0].kind), (1, Kind::FirstName));
    }

    #[test]
    fn class_lists_from_rows_lines_and_vcards() {
        let rows = vec![vec!["Nr".to_string(), "Nachname".into(), "Vorname".into()], vec!["1".into(), "Müller".into(), "Lea".into()]];
        assert_eq!(people_from_rows(&rows).0, vec![Person { first: "Lea".into(), last: "Müller".into() }]);
        let rows = vec![vec!["Schüler".to_string()], vec!["Schmidt, Jonas".into()], vec!["Ben Koch".into()]];
        assert_eq!(people_from_rows(&rows).0.len(), 2);
        let rows = vec![vec!["Müller".to_string(), "Lea".into()], vec!["Koch".into(), "Ben".into()]];
        assert_eq!(people_from_rows(&rows).0[1], Person { first: "Ben".into(), last: "Koch".into() });
        assert_eq!(person_from_text("Lea Marie von der Heide"), Some(Person { first: "Lea Marie".into(), last: "von der Heide".into() }));
        assert_eq!(
            people_from_lines("Lea Müller\n\n  \nMüller, Ben\n42"),
            vec![Person { first: "Lea".into(), last: "Müller".into() }, Person { first: "Ben".into(), last: "Müller".into() }]
        );
        let vcf = b"BEGIN:VCARD\r\nVERSION:3.0\r\nN:Yilmaz;Emre;;;\r\nFN:Emre Yilmaz\r\nEND:VCARD\r\n";
        assert_eq!(people_from_vcards(vcf), vec![Person { first: "Emre".into(), last: "Yilmaz".into() }]);
        // Hostile input never panics.
        let _ = Detector::new(&people_from_lines(&"ä\u{0}".repeat(1000)), &[], &["".into(), "[".into()], 2026).detect(&"Lea ü\u{301}".repeat(5000));
    }
}
