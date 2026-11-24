//! Interactive worksheets: the `aiscraft-worksheet` format, version 1.
//!
//! A worksheet is one JSON object. Models write it in a fenced ```` ```aiscraft-worksheet ````
//! block (AIS.chat, no tools) or through the `worksheet_create` tool; teachers edit it in the
//! app; pupils' answers are checked against it. Everything here is pure (serde, no I/O), and
//! everything that comes in is hostile: [`parse_lenient`] caps sizes and counts, ignores
//! unknown fields, accepts common aliases and reports broken tasks instead of failing.
//!
//! ```json
//! {"format": "aiscraft-worksheet", "version": 1, "title": "Brüche", "subject": "Mathematik",
//!  "grade": "6", "instructions": "Bearbeite alle Aufgaben.",
//!  "tasks": [
//!   {"type": "choice", "prompt": "Welcher Bruch ist größer?", "points": 1,
//!    "options": [{"text": "1/2", "correct": true}, {"text": "1/3"}]},
//!   {"type": "cloze", "prompt": "Ergänze.", "text": "1/2 = [[2/4|0,5]]"}]}
//! ```
//!
//! Task types and their later H5P counterparts (H5P export is a separate job; this is only the
//! map, so the fields stay convertible):
//!
//! | `type`         | Fields                                   | H5P                               |
//! |----------------|------------------------------------------|-----------------------------------|
//! | (the sheet)    | title, instructions, tasks               | H5P.QuestionSet                   |
//! | `info`         | text (+ image)                           | H5P.AdvancedText / H5P.Image      |
//! | `choice`       | options [{text, correct}], multiple      | H5P.MultiChoice (single/multi)    |
//! | `true_false`   | answer                                   | H5P.TrueFalse                     |
//! | `cloze`        | text with `[[Lösung|Alternative]]` gaps   | H5P.Blanks (`*Lösung/Alternative*`)|
//! | `matching`     | pairs [{left, right}]                    | H5P.DragText (or DragQuestion)    |
//! | `ordering`     | items in the right order                 | H5P.SortParagraphs / Summary      |
//! | `short_answer` | accepted [..], case_sensitive            | H5P.Blanks (one gap)              |
//! | `numeric`      | answer, tolerance, unit                  | H5P.Blanks (one gap, no tolerance)|
//! | `free_text`    | sample, rubric, lines                    | H5P.Essay (keywords from rubric)  |
//! | `categorize`   | categories [{name, items}]               | H5P.DragQuestion (one zone each)  |
//!
//! Every task has `prompt`, `points`, `hints`, `feedback` (shown after checking) and an
//! optional `image` (the name of an attachment). `hints` is a list the pupil uncovers one at a
//! time, so each one may give away a little more than the last. A choice option can carry its
//! own `feedback`, shown when it was picked (for a wrong one: which mix-up leads there). A
//! cloze text can show its answers as a `word_bank`, with extra wrong `distractors` mixed in.
//!
//! Sheets written before `hints` existed have a single `"hint": "…"`; it still reads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FORMAT: &str = "aiscraft-worksheet";
pub const VERSION: u32 = 1;
/// The fence language of worksheet blocks in answers.
pub const FENCE: &str = "aiscraft-worksheet";

pub const MAX_JSON_BYTES: usize = 512 * 1024;
pub const MAX_TASKS: usize = 100;
pub const MAX_TEXT: usize = 4000;
pub const MAX_SHORT: usize = 300;
pub const MAX_OPTIONS: usize = 12;
pub const MAX_PAIRS: usize = 16;
pub const MAX_ITEMS: usize = 16;
pub const MAX_GAPS: usize = 40;
pub const MAX_ALTERNATIVES: usize = 10;
pub const MAX_POINTS: f64 = 100.0;
pub const MAX_LINES: u32 = 30;
pub const MAX_HINTS: usize = 5;
pub const MAX_CATEGORIES: usize = 6;
/// Items of a categorize task, over all categories.
pub const MAX_CATEGORY_ITEMS: usize = 30;
pub const MAX_DISTRACTORS: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Worksheet {
    pub format: String,
    pub version: u32,
    pub title: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub grade: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub tasks: Vec<Task>,
}

impl Default for Worksheet {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            title: "Arbeitsblatt".into(),
            subject: String::new(),
            grade: String::new(),
            instructions: String::new(),
            tasks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub points: f64,
    /// Hints, uncovered one at a time; each may say a little more than the one before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub feedback: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(flatten)]
    pub body: TaskBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChoiceOption {
    pub text: String,
    #[serde(default)]
    pub correct: bool,
    /// Shown after checking when this option was picked: why it is right or what the
    /// misconception behind it is.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub feedback: String,
}

impl ChoiceOption {
    pub fn new(text: impl Into<String>, correct: bool) -> ChoiceOption {
        ChoiceOption { text: text.into(), correct, feedback: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub name: String,
    #[serde(default)]
    pub items: Vec<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskBody {
    Info {
        text: String,
    },
    Choice {
        options: Vec<ChoiceOption>,
        multiple: bool,
    },
    TrueFalse {
        answer: bool,
    },
    Cloze {
        text: String,
        /// Show the answers (and the distractors) as a word list.
        #[serde(default, skip_serializing_if = "is_false")]
        word_bank: bool,
        /// Wrong words for the word list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        distractors: Vec<String>,
    },
    Matching {
        pairs: Vec<Pair>,
    },
    Ordering {
        items: Vec<String>,
    },
    ShortAnswer {
        accepted: Vec<String>,
        case_sensitive: bool,
    },
    Numeric {
        answer: f64,
        tolerance: f64,
        unit: String,
    },
    FreeText {
        sample: String,
        rubric: String,
        lines: u32,
    },
    Categorize {
        categories: Vec<Category>,
    },
}

/// The task types: (`type`, German label), in the order the editor offers them.
pub const TASK_TYPES: &[(&str, &str)] = &[
    ("choice", "Auswahl (eine richtige)"),
    ("choice_multi", "Auswahl (mehrere richtige)"),
    ("true_false", "Richtig oder falsch"),
    ("cloze", "Lückentext"),
    ("matching", "Zuordnen"),
    ("ordering", "Reihenfolge"),
    ("categorize", "In Gruppen sortieren"),
    ("short_answer", "Kurze Antwort"),
    ("numeric", "Zahl"),
    ("free_text", "Freier Text"),
    ("info", "Info-Text"),
];

impl TaskBody {
    pub fn type_name(&self) -> &'static str {
        match self {
            TaskBody::Info { .. } => "info",
            TaskBody::Choice { .. } => "choice",
            TaskBody::TrueFalse { .. } => "true_false",
            TaskBody::Cloze { .. } => "cloze",
            TaskBody::Matching { .. } => "matching",
            TaskBody::Ordering { .. } => "ordering",
            TaskBody::ShortAnswer { .. } => "short_answer",
            TaskBody::Numeric { .. } => "numeric",
            TaskBody::FreeText { .. } => "free_text",
            TaskBody::Categorize { .. } => "categorize",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TaskBody::Info { .. } => "Info-Text",
            TaskBody::Choice { multiple: false, .. } => "Auswahl",
            TaskBody::Choice { multiple: true, .. } => "Mehrfachauswahl",
            TaskBody::TrueFalse { .. } => "Richtig oder falsch",
            TaskBody::Cloze { .. } => "Lückentext",
            TaskBody::Matching { .. } => "Zuordnen",
            TaskBody::Ordering { .. } => "Reihenfolge",
            TaskBody::ShortAnswer { .. } => "Kurze Antwort",
            TaskBody::Numeric { .. } => "Zahl",
            TaskBody::FreeText { .. } => "Freier Text",
            TaskBody::Categorize { .. } => "In Gruppen sortieren",
        }
    }

    /// An empty task of a type from [`TASK_TYPES`], ready to fill in.
    pub fn template(kind: &str) -> Option<TaskBody> {
        let opt = ChoiceOption::new;
        Some(match kind {
            "choice" => TaskBody::Choice { options: vec![opt("Antwort A", true), opt("Antwort B", false), opt("Antwort C", false)], multiple: false },
            "choice_multi" => TaskBody::Choice { options: vec![opt("Antwort A", true), opt("Antwort B", true), opt("Antwort C", false)], multiple: true },
            "true_false" => TaskBody::TrueFalse { answer: true },
            "cloze" => TaskBody::Cloze { text: "Ein Satz mit einer [[Lücke|Leerstelle]].".into(), word_bank: false, distractors: Vec::new() },
            "matching" => TaskBody::Matching {
                pairs: vec![Pair { left: "Begriff 1".into(), right: "Erklärung 1".into() }, Pair { left: "Begriff 2".into(), right: "Erklärung 2".into() }],
            },
            "ordering" => TaskBody::Ordering { items: vec!["Erster Schritt".into(), "Zweiter Schritt".into(), "Dritter Schritt".into()] },
            "short_answer" => TaskBody::ShortAnswer { accepted: vec!["Antwort".into()], case_sensitive: false },
            "numeric" => TaskBody::Numeric { answer: 0.0, tolerance: 0.0, unit: String::new() },
            "free_text" => TaskBody::FreeText { sample: String::new(), rubric: String::new(), lines: 5 },
            "categorize" => TaskBody::Categorize {
                categories: vec![
                    Category { name: "Gruppe 1".into(), items: vec!["Element A".into(), "Element B".into()] },
                    Category { name: "Gruppe 2".into(), items: vec!["Element C".into(), "Element D".into()] },
                ],
            },
            "info" => TaskBody::Info { text: "Lies den Text.".into() },
            _ => return None,
        })
    }

    /// Can the app check this task by itself? (Free text and info blocks: no.)
    pub fn auto_checked(&self) -> bool {
        !matches!(self, TaskBody::Info { .. } | TaskBody::FreeText { .. })
    }
}

impl Task {
    pub fn new(id: impl Into<String>, body: TaskBody) -> Task {
        let points = if body.auto_checked() || matches!(body, TaskBody::FreeText { .. }) { 1.0 } else { 0.0 };
        Task { id: id.into(), prompt: String::new(), points, hints: Vec::new(), feedback: String::new(), image: None, body }
    }
}

// ---- parsing ---------------------------------------------------------------------------------

fn cap(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n { t.to_string() } else { t.chars().take(n).collect() }
}

/// A string field under any of `names`; numbers and booleans count as text.
fn text_of(v: &Value, names: &[&str], max: usize) -> String {
    for n in names {
        match v.get(*n) {
            Some(Value::String(s)) => return cap(s, max),
            Some(Value::Number(x)) => return x.to_string(),
            Some(Value::Bool(b)) => return if *b { "wahr".into() } else { "falsch".into() },
            _ => {}
        }
    }
    String::new()
}

fn num_of(v: &Value, names: &[&str]) -> Option<f64> {
    for n in names {
        match v.get(*n) {
            Some(Value::Number(x)) => return x.as_f64().filter(|f| f.is_finite()),
            Some(Value::String(s)) => {
                if let Some(f) = parse_number(s) {
                    return Some(f);
                }
            }
            _ => {}
        }
    }
    None
}

fn bool_of(v: &Value, names: &[&str]) -> Option<bool> {
    for n in names {
        match v.get(*n) {
            Some(Value::Bool(b)) => return Some(*b),
            Some(Value::String(s)) => match s.trim().to_lowercase().as_str() {
                "true" | "wahr" | "richtig" | "ja" | "yes" | "r" => return Some(true),
                "false" | "falsch" | "nein" | "no" | "f" => return Some(false),
                _ => {}
            },
            Some(Value::Number(x)) => return Some(x.as_f64().unwrap_or(0.0) != 0.0),
            _ => {}
        }
    }
    None
}

fn list_of<'a>(v: &'a Value, names: &[&str]) -> Option<&'a Vec<Value>> {
    names.iter().find_map(|n| v.get(*n).and_then(Value::as_array))
}

/// A number as people write it: `3,5`, `3.5`, `1 000`, `-2`.
pub fn parse_number(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').collect();
    if t.is_empty() || t.len() > 40 {
        return None;
    }
    let t = if t.contains(',') && !t.contains('.') { t.replace(',', ".") } else { t.replace(',', "") };
    t.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// A list of strings (strings, numbers, or objects with a `text`), each at most `max_len`.
fn strings_of_len(v: &Value, names: &[&str], max_items: usize, max_len: usize) -> Vec<String> {
    let Some(list) = list_of(v, names) else {
        // A single string where a list belongs.
        let one = text_of(v, names, max_len);
        return if one.is_empty() { Vec::new() } else { vec![one] };
    };
    list.iter()
        .take(max_items)
        .filter_map(|x| match x {
            Value::String(s) => Some(cap(s, max_len)),
            Value::Number(n) => Some(n.to_string()),
            Value::Object(_) => Some(text_of(x, &["text", "value", "label"], max_len)),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn strings_of(v: &Value, names: &[&str], max_items: usize) -> Vec<String> {
    strings_of_len(v, names, max_items, MAX_SHORT)
}

/// Hints in the order they are revealed: a `hints` list, and the single `hint` of older sheets
/// in front of it.
fn hints_of(v: &Value) -> Vec<String> {
    let mut hints = match list_of(v, &["hints", "tipps"]) {
        Some(_) => strings_of_len(v, &["hints", "tipps"], MAX_HINTS, MAX_TEXT),
        None => Vec::new(),
    };
    let single = text_of(v, &["hint", "tipp", "hilfe"], MAX_TEXT);
    if !single.is_empty() && !hints.contains(&single) {
        hints.insert(0, single);
    }
    hints.truncate(MAX_HINTS);
    hints
}

fn feedback_of(v: &Value) -> String {
    text_of(v, &["feedback", "explanation", "erklaerung", "erklärung"], MAX_TEXT)
}

/// One task from hostile JSON. `Err` says what is wrong, in German.
fn parse_task(v: &Value) -> Result<Task, String> {
    if !v.is_object() {
        return Err("ist kein Objekt".into());
    }
    let ty = text_of(v, &["type", "typ", "kind"], 40).to_lowercase().replace(['-', ' '], "_");
    let multiple_hint = bool_of(v, &["multiple", "multi", "mehrfach"]);
    let has_groups = list_of(v, GROUP_KEYS).is_some();
    let body = match ty.as_str() {
        "info" | "text" | "text_block" | "information" | "hinweis" => TaskBody::Info { text: text_of(v, &["text", "content", "body"], MAX_TEXT) },
        "choice" | "multiple_choice" | "single_choice" | "mc" | "auswahl" | "multichoice" | "choice_multi" => {
            let list = list_of(v, &["options", "choices", "answers", "antworten"]).ok_or("hat keine Antwortmöglichkeiten (options)")?;
            let correct_idx: Vec<usize> = match v.get("correct").or_else(|| v.get("solution")) {
                Some(Value::Number(n)) => n.as_u64().map(|i| vec![i as usize]).unwrap_or_default(),
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_u64).map(|i| i as usize).collect(),
                _ => Vec::new(),
            };
            let options: Vec<ChoiceOption> = list
                .iter()
                .take(MAX_OPTIONS)
                .enumerate()
                .filter_map(|(i, o)| match o {
                    Value::String(s) => Some(ChoiceOption::new(cap(s, MAX_SHORT), correct_idx.contains(&i))),
                    Value::Number(n) => Some(ChoiceOption::new(n.to_string(), correct_idx.contains(&i))),
                    Value::Object(_) => Some(ChoiceOption {
                        text: text_of(o, &["text", "label", "value"], MAX_SHORT),
                        correct: bool_of(o, &["correct", "richtig", "is_correct"]).unwrap_or(false) || correct_idx.contains(&i),
                        feedback: feedback_of(o),
                    }),
                    _ => None,
                })
                .filter(|o| !o.text.is_empty())
                .collect();
            if options.len() < 2 {
                return Err("braucht mindestens zwei Antwortmöglichkeiten".into());
            }
            let n_correct = options.iter().filter(|o| o.correct).count();
            let multiple = multiple_hint.unwrap_or(ty == "choice_multi" || n_correct > 1);
            TaskBody::Choice { options, multiple }
        }
        "true_false" | "truefalse" | "tf" | "richtig_falsch" | "wahr_falsch" | "boolean" => {
            let answer = bool_of(v, &["answer", "correct", "solution", "loesung", "lösung"]).ok_or("hat keine Lösung (answer: true oder false)")?;
            TaskBody::TrueFalse { answer }
        }
        "cloze" | "gaps" | "blanks" | "fill_in" | "fill_in_the_blanks" | "lueckentext" | "lückentext" => {
            let text = text_of(v, &["text", "content", "body"], MAX_TEXT);
            if cloze_gaps(&text).is_empty() {
                return Err("hat keine Lücken (so markieren: [[Lösung|Alternative]])".into());
            }
            let distractors = strings_of(v, &["distractors", "extra_words"], MAX_DISTRACTORS);
            let word_bank = bool_of(v, &["word_bank", "word_list"]).unwrap_or(false);
            TaskBody::Cloze { text, word_bank, distractors }
        }
        "matching" | "match" | "pairs" | "zuordnen" | "zuordnung" => {
            let list = list_of(v, &["pairs", "paare", "items"]).ok_or("hat keine Paare (pairs)")?;
            let pairs: Vec<Pair> = list
                .iter()
                .take(MAX_PAIRS)
                .filter_map(|p| match p {
                    Value::Array(a) => Some(Pair {
                        left: a.first().and_then(Value::as_str).map(|s| cap(s, MAX_SHORT)).unwrap_or_default(),
                        right: a.get(1).and_then(Value::as_str).map(|s| cap(s, MAX_SHORT)).unwrap_or_default(),
                    }),
                    Value::Object(_) => Some(Pair {
                        left: text_of(p, &["left", "term", "a", "links"], MAX_SHORT),
                        right: text_of(p, &["right", "match", "b", "rechts"], MAX_SHORT),
                    }),
                    _ => None,
                })
                .filter(|p| !p.left.is_empty() && !p.right.is_empty())
                .collect();
            if pairs.len() < 2 {
                return Err("braucht mindestens zwei Paare".into());
            }
            TaskBody::Matching { pairs }
        }
        "categorize" | "categorise" | "classify" | "group" | "groups" | "gruppieren" | "einordnen" => categorize_of(v)?,
        // "Sort" with groups means sorting into groups, without them putting in order.
        "sort" | "sortieren" if has_groups => categorize_of(v)?,
        "ordering" | "order" | "sequence" | "sort" | "reihenfolge" | "sortieren" => {
            let items = strings_of(v, &["items", "steps", "order", "elemente"], MAX_ITEMS);
            if items.len() < 2 {
                return Err("braucht mindestens zwei Elemente".into());
            }
            TaskBody::Ordering { items }
        }
        "short_answer" | "short" | "shortanswer" | "kurzantwort" | "kurze_antwort" => {
            let accepted: Vec<String> = strings_of(v, &["accepted", "answers", "answer", "solution", "loesung", "lösung"], MAX_ALTERNATIVES);
            if accepted.is_empty() {
                return Err("hat keine akzeptierte Antwort (accepted)".into());
            }
            TaskBody::ShortAnswer { accepted, case_sensitive: bool_of(v, &["case_sensitive"]).unwrap_or(false) }
        }
        "numeric" | "number" | "zahl" | "numerical" | "calculation" | "rechnung" => {
            let answer = num_of(v, &["answer", "solution", "value", "loesung", "lösung"]).ok_or("hat keine Zahl als Lösung (answer)")?;
            let tolerance = num_of(v, &["tolerance", "toleranz"]).unwrap_or(0.0).abs();
            TaskBody::Numeric { answer, tolerance, unit: text_of(v, &["unit", "einheit"], 40) }
        }
        "free_text" | "freetext" | "essay" | "open" | "freitext" | "freier_text" | "long_answer" => TaskBody::FreeText {
            sample: text_of(v, &["sample", "solution", "sample_solution", "musterloesung", "musterlösung", "answer"], MAX_TEXT),
            rubric: text_of(v, &["rubric", "criteria", "kriterien", "erwartungshorizont"], MAX_TEXT),
            lines: num_of(v, &["lines", "zeilen"]).map(|n| n.clamp(1.0, MAX_LINES as f64) as u32).unwrap_or(5),
        },
        "" => return Err("hat keinen Typ (type)".into()),
        other => return Err(format!("hat einen unbekannten Typ „{}“", cap(other, 30))),
    };
    let default_points = if body.auto_checked() || matches!(body, TaskBody::FreeText { .. }) { 1.0 } else { 0.0 };
    let points = num_of(v, &["points", "punkte", "score"]).map(|p| p.clamp(0.0, MAX_POINTS)).unwrap_or(default_points);
    let image = Some(text_of(v, &["image", "bild"], MAX_SHORT)).filter(|s| !s.is_empty());
    Ok(Task {
        id: text_of(v, &["id"], 40),
        prompt: text_of(v, &["prompt", "question", "frage", "title", "aufgabe", "instruction"], MAX_TEXT),
        points,
        hints: hints_of(v),
        feedback: feedback_of(v),
        image,
        body,
    })
}

const GROUP_KEYS: &[&str] = &["categories", "groups", "gruppen"];

fn categorize_of(v: &Value) -> Result<TaskBody, String> {
    let list = list_of(v, GROUP_KEYS).ok_or("hat keine Gruppen (categories)")?;
    let mut total = 0usize;
    let mut categories = Vec::new();
    for c in list.iter().take(MAX_CATEGORIES) {
        let name = text_of(c, &["name", "title", "label"], MAX_SHORT);
        let mut items = strings_of(c, &["items", "members"], MAX_ITEMS);
        items.truncate(MAX_CATEGORY_ITEMS.saturating_sub(total));
        total += items.len();
        if !name.is_empty() {
            categories.push(Category { name, items });
        }
    }
    if categories.len() < 2 {
        return Err("braucht mindestens zwei Gruppen".into());
    }
    if total < 2 {
        return Err("braucht mindestens zwei Elemente zum Einsortieren".into());
    }
    Ok(TaskBody::Categorize { categories })
}

/// The JSON object inside `src`: a bare object, one inside a fence, or one between prose.
fn json_slice(src: &str) -> &str {
    let t = src.trim();
    let t = match t.strip_prefix("```") {
        Some(rest) => {
            let body = rest.find('\n').and_then(|nl| rest.get(nl + 1..)).unwrap_or(rest);
            body.trim_end().strip_suffix("```").unwrap_or(body)
        }
        None => t,
    };
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if a < b => t.get(a..=b).unwrap_or(t),
        _ => t,
    }
}

/// Reads a worksheet from model or user JSON. Unknown fields are ignored, aliases accepted
/// (`question` for `prompt`, `multiple_choice` for `choice` …); a task that cannot be read is
/// left out and named in the problems. `Err` only when there is no usable worksheet at all.
pub fn parse_lenient(src: &str) -> Result<(Worksheet, Vec<String>), String> {
    if src.len() > MAX_JSON_BYTES {
        return Err(format!("Das Arbeitsblatt ist zu groß (über {} KB)", MAX_JSON_BYTES / 1024));
    }
    let v: Value = serde_json::from_str(json_slice(src)).map_err(|e| format!("Das Arbeitsblatt ist kein gültiges JSON: {e}"))?;
    from_value(&v)
}

/// Like [`parse_lenient`], from JSON already parsed (tool arguments).
pub fn from_value(v: &Value) -> Result<(Worksheet, Vec<String>), String> {
    let v = match v.get("worksheet") {
        Some(inner) if inner.is_object() => inner,
        _ => v,
    };
    let (root, tasks_v) = match v {
        Value::Object(_) => (v, list_of(v, &["tasks", "aufgaben", "questions", "items"])),
        Value::Array(a) => (&Value::Null, Some(a)),
        _ => return Err("Das Arbeitsblatt muss ein JSON-Objekt sein".into()),
    };
    let mut problems = Vec::new();
    if let Some(f) = root.get("format").and_then(Value::as_str)
        && f != FORMAT
    {
        problems.push(format!("Unbekanntes Format „{}“, als {FORMAT} gelesen", cap(f, 40)));
    }
    if let Some(ver) = root.get("version").and_then(Value::as_u64)
        && ver > VERSION as u64
    {
        problems.push(format!("Version {ver} ist neuer als diese App (liest Version {VERSION})"));
    }
    let mut ws = Worksheet {
        title: Some(text_of(root, &["title", "titel"], 200)).filter(|t| !t.is_empty()).unwrap_or_else(|| "Arbeitsblatt".into()),
        subject: text_of(root, &["subject", "fach"], 100),
        grade: text_of(root, &["grade", "klasse", "jahrgang", "level"], 40),
        instructions: text_of(root, &["instructions", "description", "anleitung", "hinweise"], MAX_TEXT),
        ..Worksheet::default()
    };
    let list = tasks_v.ok_or("Das Arbeitsblatt hat keine Aufgaben (tasks)")?;
    if list.len() > MAX_TASKS {
        problems.push(format!("Nur die ersten {MAX_TASKS} von {} Aufgaben übernommen", list.len()));
    }
    for (i, t) in list.iter().take(MAX_TASKS).enumerate() {
        match parse_task(t) {
            Ok(task) => ws.tasks.push(task),
            Err(e) => problems.push(format!("Aufgabe {} {e}", i + 1)),
        }
    }
    if ws.tasks.is_empty() {
        return Err(match problems.first() {
            Some(p) => format!("Keine Aufgabe ließ sich lesen ({p})"),
            None => "Das Arbeitsblatt hat keine Aufgaben".into(),
        });
    }
    normalize_ids(&mut ws);
    problems.extend(validate(&ws));
    Ok((ws, problems))
}

/// Unique, non-empty task ids (`a1`, `a2` … for missing or repeated ones).
pub fn normalize_ids(ws: &mut Worksheet) {
    let mut seen = std::collections::HashSet::new();
    let mut n = 0usize;
    for t in ws.tasks.iter_mut() {
        let id: String = t.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
        if id.is_empty() || !seen.insert(id.clone()) {
            loop {
                n += 1;
                let candidate = format!("a{n}");
                if seen.insert(candidate.clone()) {
                    t.id = candidate;
                    break;
                }
            }
        } else {
            t.id = id;
        }
    }
}

/// What is wrong with a worksheet that parsed: tasks pupils could not answer correctly.
pub fn validate(ws: &Worksheet) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in ws.tasks.iter().enumerate() {
        let n = i + 1;
        match &t.body {
            TaskBody::Choice { options, multiple } => {
                let c = options.iter().filter(|o| o.correct).count();
                if c == 0 {
                    out.push(format!("Aufgabe {n}: keine Antwort ist als richtig markiert"));
                } else if c > 1 && !multiple {
                    out.push(format!("Aufgabe {n}: mehrere richtige Antworten, aber nur eine wählbar"));
                }
            }
            TaskBody::Cloze { text, .. } if cloze_gaps(text).is_empty() => out.push(format!("Aufgabe {n}: der Lückentext hat keine Lücken")),
            TaskBody::Categorize { categories } => {
                if categories.len() < 2 {
                    out.push(format!("Aufgabe {n}: braucht mindestens zwei Gruppen"));
                } else if categories.iter().any(|c| c.name.trim().is_empty()) {
                    out.push(format!("Aufgabe {n}: eine Gruppe hat keinen Namen"));
                } else if category_items(categories).len() < 2 {
                    out.push(format!("Aufgabe {n}: braucht mindestens zwei Elemente zum Einsortieren"));
                }
            }
            TaskBody::Matching { pairs } if pairs.len() < 2 => out.push(format!("Aufgabe {n}: braucht mindestens zwei Paare")),
            TaskBody::Ordering { items } if items.len() < 2 => out.push(format!("Aufgabe {n}: braucht mindestens zwei Elemente")),
            TaskBody::ShortAnswer { accepted, .. } if accepted.iter().all(|a| a.trim().is_empty()) => {
                out.push(format!("Aufgabe {n}: keine akzeptierte Antwort"))
            }
            TaskBody::Numeric { answer, tolerance, .. } if !answer.is_finite() || !tolerance.is_finite() => {
                out.push(format!("Aufgabe {n}: die Lösung ist keine Zahl"))
            }
            _ => {}
        }
        if t.body.auto_checked() && t.prompt.trim().is_empty() && !matches!(t.body, TaskBody::Cloze { .. }) {
            out.push(format!("Aufgabe {n}: die Aufgabenstellung fehlt"));
        }
    }
    out
}

/// The canonical JSON of a worksheet (pretty, stable field order).
pub fn to_json(ws: &Worksheet) -> String {
    serde_json::to_string_pretty(ws).unwrap_or_else(|_| "{}".into())
}

// ---- cloze -----------------------------------------------------------------------------------

/// A piece of a cloze text: plain text or a gap with its accepted answers.
#[derive(Debug, Clone, PartialEq)]
pub enum ClozePart<'a> {
    Text(&'a str),
    Gap(Vec<String>),
}

/// Splits a cloze text at its gaps, `[[Lösung|Alternative]]` (also `{{…}}`).
pub fn cloze_parts(text: &str) -> Vec<ClozePart<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while out.len() < MAX_GAPS * 2 + 1 {
        let open = [("[[", "]]"), ("{{", "}}")].iter().filter_map(|(o, c)| rest.find(o).map(|i| (i, *o, *c))).min_by_key(|(i, _, _)| *i);
        let Some((i, o, c)) = open else { break };
        let after = rest.get(i + o.len()..).unwrap_or("");
        let Some(j) = after.find(c) else { break };
        let inner = after.get(..j).unwrap_or("");
        let answers: Vec<String> = inner.split('|').map(|a| cap(a, MAX_SHORT)).filter(|a| !a.is_empty()).take(MAX_ALTERNATIVES).collect();
        if inner.contains('\n') || answers.is_empty() {
            // Not a gap: keep the brackets as text.
            out.push(ClozePart::Text(rest.get(..i + o.len()).unwrap_or("")));
            rest = after;
            continue;
        }
        if i > 0 {
            out.push(ClozePart::Text(rest.get(..i).unwrap_or("")));
        }
        out.push(ClozePart::Gap(answers));
        rest = after.get(j + c.len()..).unwrap_or("");
    }
    if !rest.is_empty() {
        out.push(ClozePart::Text(rest));
    }
    out
}

/// The word list of a cloze task: the first answer of every gap plus the distractors, each
/// once, in a stable order that is not the order of the gaps.
pub fn cloze_word_bank(text: &str, distractors: &[String], seed: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for w in cloze_gaps(text).into_iter().filter_map(|g| g.into_iter().next()).chain(distractors.iter().take(MAX_DISTRACTORS).map(|d| cap(d, MAX_SHORT))) {
        if !w.is_empty() && !words.contains(&w) {
            words.push(w);
        }
    }
    display_order(words.len(), seed).into_iter().filter_map(|i| words.get(i).cloned()).collect()
}

/// The items of a categorize task in solution order, each with the index of its category.
pub fn category_items(categories: &[Category]) -> Vec<(usize, &str)> {
    categories
        .iter()
        .enumerate()
        .flat_map(|(c, cat)| cat.items.iter().filter(|x| !x.trim().is_empty()).map(move |x| (c, x.as_str())))
        .take(MAX_CATEGORY_ITEMS)
        .collect()
}

/// The accepted answers of each gap.
pub fn cloze_gaps(text: &str) -> Vec<Vec<String>> {
    cloze_parts(text)
        .into_iter()
        .filter_map(|p| match p {
            ClozePart::Gap(a) => Some(a),
            ClozePart::Text(_) => None,
        })
        .take(MAX_GAPS)
        .collect()
}

// ---- answers and scoring ---------------------------------------------------------------------

/// A pupil's answer to one task.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Response {
    #[default]
    Empty,
    /// Chosen option indices (choice).
    Chosen(Vec<usize>),
    Bool(bool),
    /// One text per gap (cloze).
    Gaps(Vec<String>),
    /// For each left item, the index of the right item picked (matching).
    Matches(Vec<Option<usize>>),
    /// Item indices in the order the pupil put them (ordering).
    Order(Vec<usize>),
    /// For each item (in [`category_items`] order), the category it was put in (categorize).
    Assign(Vec<Option<usize>>),
    Text(String),
}

impl Response {
    /// An answer from JSON, read the way the task needs it: choice `[0, 2]` or `1`, true_false
    /// `true`, cloze `["…", "…"]`, matching `[2, 0, 1]` (right index per left item), ordering
    /// `[2, 0, 1]` (item indices in the chosen order), categorize `[0, 1, null]` (category per
    /// item) or the category names `["Säugetiere", …]`, others a string or number.
    pub fn from_json(body: &TaskBody, v: &Value) -> Response {
        let idx = |x: &Value| x.as_u64().map(|i| i as usize).filter(|i| *i < 1000);
        match (body, v) {
            (_, Value::Null) => Response::Empty,
            (TaskBody::Choice { .. }, Value::Array(a)) => Response::Chosen(a.iter().take(MAX_OPTIONS).filter_map(idx).collect()),
            (TaskBody::Choice { .. }, Value::Number(_)) => Response::Chosen(idx(v).into_iter().collect()),
            (TaskBody::TrueFalse { .. }, _) => match bool_of(&serde_json::json!({"v": v}), &["v"]) {
                Some(b) => Response::Bool(b),
                None => Response::Empty,
            },
            (TaskBody::Cloze { .. }, Value::Array(a)) => {
                Response::Gaps(a.iter().take(MAX_GAPS).map(|x| x.as_str().map(|s| cap(s, MAX_SHORT)).unwrap_or_default()).collect())
            }
            (TaskBody::Cloze { .. }, Value::String(s)) => Response::Gaps(vec![cap(s, MAX_SHORT)]),
            (TaskBody::Matching { .. }, Value::Array(a)) => Response::Matches(a.iter().take(MAX_PAIRS).map(idx).collect()),
            (TaskBody::Ordering { .. }, Value::Array(a)) => Response::Order(a.iter().take(MAX_ITEMS).filter_map(idx).collect()),
            (TaskBody::Categorize { categories }, Value::Array(a)) => Response::Assign(
                a.iter()
                    .take(MAX_CATEGORY_ITEMS)
                    .map(|x| match x {
                        Value::String(name) => {
                            let n = norm(name, false);
                            categories.iter().position(|c| norm(&c.name, false) == n)
                        }
                        _ => idx(x),
                    })
                    .collect(),
            ),
            (_, Value::String(s)) => Response::Text(cap(s, MAX_TEXT)),
            (_, Value::Number(n)) => Response::Text(n.to_string()),
            _ => Response::Empty,
        }
    }
}

/// The check of one task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskResult {
    pub id: String,
    pub points: f64,
    pub max: f64,
    /// `None`: not checked by the app (free text, info).
    pub correct: Option<bool>,
    /// Per gap, pair or position where the task has parts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<bool>,
}

fn norm(s: &str, case_sensitive: bool) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = t.trim_end_matches(['.', '!']).to_string();
    if case_sensitive { t } else { t.to_lowercase() }
}

fn text_matches(given: &str, accepted: &[String], case_sensitive: bool) -> bool {
    let g = norm(given, case_sensitive);
    if g.is_empty() {
        return false;
    }
    // Numbers compare as numbers ("0,5" = "0.5").
    if let Some(x) = parse_number(&g)
        && accepted.iter().filter_map(|a| parse_number(a)).any(|a| (a - x).abs() < 1e-9)
    {
        return true;
    }
    accepted.iter().any(|a| norm(a, case_sensitive) == g)
}

/// Checks one answer. Partial credit for multiple choice (right minus wrong picks), gaps, pairs
/// and positions; all or nothing for the rest.
pub fn score_task(task: &Task, r: &Response) -> TaskResult {
    let max = task.points.clamp(0.0, MAX_POINTS);
    let mut res = TaskResult { id: task.id.clone(), points: 0.0, max, correct: Some(false), parts: Vec::new() };
    let fraction: f64 = match (&task.body, r) {
        (TaskBody::Info { .. }, _) | (TaskBody::FreeText { .. }, _) => {
            res.correct = None;
            return res;
        }
        (TaskBody::Choice { options, multiple }, Response::Chosen(chosen)) => {
            let mut picked: Vec<usize> = chosen.iter().copied().filter(|i| *i < options.len()).collect();
            picked.sort_unstable();
            picked.dedup();
            let n_correct = options.iter().filter(|o| o.correct).count();
            if !multiple && picked.len() > 1 {
                0.0
            } else {
                let right = picked.iter().filter(|i| options.get(**i).is_some_and(|o| o.correct)).count();
                let wrong = picked.len() - right;
                res.parts = options.iter().enumerate().map(|(i, o)| o.correct == picked.contains(&i)).collect();
                if n_correct == 0 { 0.0 } else { ((right as f64 - wrong as f64) / n_correct as f64).max(0.0) }
            }
        }
        (TaskBody::TrueFalse { answer }, Response::Bool(b)) => f64::from(u8::from(answer == b)),
        (TaskBody::Cloze { text, .. }, Response::Gaps(given)) => {
            let gaps = cloze_gaps(text);
            res.parts = gaps.iter().enumerate().map(|(i, acc)| given.get(i).is_some_and(|g| text_matches(g, acc, false))).collect();
            if gaps.is_empty() { 0.0 } else { res.parts.iter().filter(|x| **x).count() as f64 / gaps.len() as f64 }
        }
        (TaskBody::Matching { pairs }, Response::Matches(m)) => {
            res.parts = (0..pairs.len()).map(|i| m.get(i).copied().flatten() == Some(i)).collect();
            if pairs.is_empty() { 0.0 } else { res.parts.iter().filter(|x| **x).count() as f64 / pairs.len() as f64 }
        }
        (TaskBody::Ordering { items }, Response::Order(order)) => {
            res.parts = (0..items.len()).map(|pos| order.get(pos) == Some(&pos)).collect();
            if items.is_empty() { 0.0 } else { res.parts.iter().filter(|x| **x).count() as f64 / items.len() as f64 }
        }
        (TaskBody::Categorize { categories }, Response::Assign(given)) => {
            let items = category_items(categories);
            res.parts = items.iter().enumerate().map(|(i, (c, _))| given.get(i).copied().flatten() == Some(*c)).collect();
            if items.is_empty() { 0.0 } else { res.parts.iter().filter(|x| **x).count() as f64 / items.len() as f64 }
        }
        (TaskBody::ShortAnswer { accepted, case_sensitive }, Response::Text(t)) => f64::from(u8::from(text_matches(t, accepted, *case_sensitive))),
        (TaskBody::Numeric { answer, tolerance, unit }, Response::Text(t)) => {
            let cleaned = if unit.is_empty() { t.clone() } else { t.replace(unit.as_str(), "") };
            match parse_number(&cleaned) {
                Some(x) => f64::from(u8::from((x - answer).abs() <= tolerance.abs() + 1e-9)),
                None => 0.0,
            }
        }
        _ => 0.0,
    };
    let fraction = fraction.clamp(0.0, 1.0);
    res.points = (max * fraction * 100.0).round() / 100.0;
    res.correct = Some(fraction >= 1.0 - 1e-9);
    res
}

/// The check of a whole sheet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SheetResult {
    pub results: Vec<TaskResult>,
    pub points: f64,
    /// Points the app could check (free text not included).
    pub max: f64,
    /// Tasks the teacher checks (free text).
    pub open: usize,
}

/// Checks all answers; `answers` maps task id to answer.
pub fn score(ws: &Worksheet, answers: &std::collections::HashMap<String, Response>) -> SheetResult {
    let results: Vec<TaskResult> = ws.tasks.iter().map(|t| score_task(t, answers.get(&t.id).unwrap_or(&Response::Empty))).collect();
    let points = results.iter().filter(|r| r.correct.is_some()).map(|r| r.points).sum::<f64>();
    let max = results.iter().filter(|r| r.correct.is_some()).map(|r| r.max).sum::<f64>();
    let open = ws.tasks.iter().filter(|t| matches!(t.body, TaskBody::FreeText { .. })).count();
    SheetResult { results, points: (points * 100.0).round() / 100.0, max, open }
}

/// A stable shuffle of `0..n` for showing items or matches in a different order than the
/// solution (never the identity for n >= 2).
pub fn display_order(n: usize, seed: &str) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut h: u64 = 1469598103934665603;
    for b in seed.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(1099511628211);
    }
    for i in (1..n).rev() {
        h ^= h << 13;
        h ^= h >> 7;
        h ^= h << 17;
        let j = (h % (i as u64 + 1)) as usize;
        idx.swap(i, j);
    }
    if n >= 2 && idx.iter().enumerate().all(|(i, x)| i == *x) {
        idx.rotate_left(1);
    }
    idx
}

// ---- prompts ---------------------------------------------------------------------------------

/// The format, compact, for a model that writes a worksheet without tools.
pub fn prompt_schema() -> &'static str {
    r#"Format (JSON, ein Objekt):
{"format":"aiscraft-worksheet","version":1,"title":"…","subject":"…","grade":"…","instructions":"…","tasks":[…]}
Jede Aufgabe hat "type", "prompt" (Aufgabenstellung), "points", optional "hints" (Liste von Tipps, die man einzeln aufdeckt; jeder wird etwas konkreter als der vorige) und "feedback" (Erklärung nach dem Prüfen). Typen:
- {"type":"choice","prompt":"…","multiple":false,"options":[{"text":"…","correct":true},{"text":"…","correct":false,"feedback":"Warum diese Antwort nicht stimmt"}]}
- {"type":"true_false","prompt":"Aussage …","answer":true}
- {"type":"cloze","prompt":"Ergänze.","text":"Die Hauptstadt ist [[Berlin]]. Alternativen mit |: [[Fotosynthese|Photosynthese]]"} (für Jüngere mit Wortliste: "word_bank":true, "distractors":["falsches Wort"])
- {"type":"matching","prompt":"Ordne zu.","pairs":[{"left":"…","right":"…"}]}
- {"type":"ordering","prompt":"Bringe in die richtige Reihenfolge.","items":["erster","zweiter","dritter"]} (items in der richtigen Reihenfolge)
- {"type":"categorize","prompt":"Sortiere in die Gruppen.","categories":[{"name":"Säugetiere","items":["Wal","Fledermaus"]},{"name":"Fische","items":["Hai","Forelle"]}]}
- {"type":"short_answer","prompt":"…","accepted":["Antwort","Variante"]}
- {"type":"numeric","prompt":"…","answer":3.5,"tolerance":0.1,"unit":"cm"}
- {"type":"free_text","prompt":"…","sample":"Musterlösung","rubric":"Bewertungskriterien","lines":6}
- {"type":"info","text":"Lesetext oder Hinweis"} (ohne Punkte)"#
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use super::*;
    use serde_json::json;

    fn sample() -> Worksheet {
        let src = json!({
            "format": "aiscraft-worksheet", "version": 1, "title": "Brüche", "subject": "Mathematik", "grade": "6",
            "tasks": [
                {"type": "choice", "prompt": "Größer?", "options": [{"text": "1/2", "correct": true}, {"text": "1/3"}]},
                {"type": "choice", "prompt": "Gleich 1/2?", "options": [{"text": "2/4", "correct": true}, {"text": "3/6", "correct": true}, {"text": "1/3"}]},
                {"type": "true_false", "prompt": "1/4 < 1/2", "answer": true},
                {"type": "cloze", "prompt": "Ergänze", "text": "1/2 = [[2/4|0,5]] und 1 = [[2/2]]"},
                {"type": "matching", "prompt": "Ordne zu", "pairs": [{"left": "1/2", "right": "0,5"}, {"left": "1/4", "right": "0,25"}]},
                {"type": "ordering", "prompt": "Sortiere", "items": ["1/4", "1/2", "3/4"]},
                {"type": "short_answer", "prompt": "Kehrwert von 2?", "accepted": ["1/2", "0,5"]},
                {"type": "numeric", "prompt": "3/4 als Dezimalzahl", "answer": 0.75, "tolerance": 0.01},
                {"type": "free_text", "prompt": "Erkläre", "sample": "…", "points": 3},
                {"type": "info", "text": "Merke: …"}
            ]
        });
        from_value(&src).unwrap().0
    }

    #[test]
    fn round_trip_is_stable() {
        let ws = sample();
        assert_eq!(ws.tasks.len(), 10);
        let json = to_json(&ws);
        let (back, problems) = parse_lenient(&json).unwrap();
        assert_eq!(back, ws);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(json.contains("\"type\": \"cloze\""));
        // Ids are filled in and unique.
        let ids: std::collections::HashSet<_> = ws.tasks.iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn lenient_parsing_of_hostile_json() {
        // Fenced, with prose around, aliases and unknown fields.
        let src = "Hier:\n```aiscraft-worksheet\n{\"titel\":\"X\",\"aufgaben\":[{\"typ\":\"multiple_choice\",\"question\":\"Q\",\"choices\":[\"a\",\"b\"],\"correct\":1,\"zzz\":{}},{\"type\":\"warp\"},{\"type\":\"true_false\"},42]}\n```";
        let (ws, problems) = parse_lenient(src).unwrap();
        assert_eq!(ws.title, "X");
        assert_eq!(ws.tasks.len(), 1);
        match &ws.tasks[0].body {
            TaskBody::Choice { options, multiple } => {
                assert!(!multiple);
                assert!(options[1].correct && !options[0].correct);
            }
            b => panic!("{b:?}"),
        }
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("unbekannten Typ"));
        for bad in ["", "null", "[]", "{}", "{\"tasks\":5}", "{\"tasks\":[]}", "nicht json", "{\"tasks\":[{\"type\":\"cloze\",\"text\":\"ohne\"}]}"] {
            assert!(parse_lenient(bad).is_err(), "{bad}");
        }
        assert!(parse_lenient(&"x".repeat(MAX_JSON_BYTES + 1)).is_err());
        // Counts and sizes are capped.
        let many: Vec<Value> = (0..500).map(|_| json!({"type": "true_false", "prompt": "ä".repeat(10_000), "answer": "ja", "points": 1e300})).collect();
        let (ws, problems) = from_value(&json!({"tasks": many})).unwrap();
        assert_eq!(ws.tasks.len(), MAX_TASKS);
        assert_eq!(ws.tasks[0].prompt.chars().count(), MAX_TEXT);
        assert_eq!(ws.tasks[0].points, MAX_POINTS);
        assert!(problems[0].contains("ersten"));
        // Semantic problems are reported, not fatal.
        let (_, problems) = from_value(&json!({"tasks": [{"type": "choice", "prompt": "Q", "options": ["a", "b"]}]})).unwrap();
        assert!(problems.iter().any(|p| p.contains("keine Antwort ist als richtig")));
    }

    #[test]
    fn scoring_per_task_type() {
        let ws = sample();
        let t = |i: usize| &ws.tasks[i];
        assert_eq!(score_task(t(0), &Response::Chosen(vec![0])).correct, Some(true));
        assert_eq!(score_task(t(0), &Response::Chosen(vec![0, 1])).points, 0.0, "single choice with two picks");
        // Multi: one right of two = half; right + wrong = 0.
        assert_eq!(score_task(t(1), &Response::Chosen(vec![0])).points, 0.5);
        assert_eq!(score_task(t(1), &Response::Chosen(vec![0, 2])).points, 0.0);
        assert_eq!(score_task(t(1), &Response::Chosen(vec![1, 0, 99])).correct, Some(true));
        assert_eq!(score_task(t(2), &Response::Bool(true)).correct, Some(true));
        assert_eq!(score_task(t(2), &Response::Bool(false)).correct, Some(false));
        let r = score_task(t(3), &Response::Gaps(vec![" 0.5 ".into(), "1".into()]));
        assert_eq!(r.parts, vec![true, false]);
        assert_eq!(r.points, 0.5);
        assert_eq!(score_task(t(3), &Response::Gaps(vec!["2/4".into(), "2/2".into()])).correct, Some(true));
        assert_eq!(score_task(t(4), &Response::Matches(vec![Some(0), Some(1)])).correct, Some(true));
        assert_eq!(score_task(t(4), &Response::Matches(vec![Some(1), None])).points, 0.0);
        assert_eq!(score_task(t(5), &Response::Order(vec![0, 1, 2])).correct, Some(true));
        assert_eq!(score_task(t(5), &Response::Order(vec![0, 2, 1])).parts, vec![true, false, false]);
        assert_eq!(score_task(t(6), &Response::Text("0.5".into())).correct, Some(true));
        assert_eq!(score_task(t(6), &Response::Text("1/3".into())).correct, Some(false));
        assert_eq!(score_task(t(7), &Response::Text("0,755".into())).correct, Some(true));
        assert_eq!(score_task(t(7), &Response::Text("0,8".into())).correct, Some(false));
        assert_eq!(score_task(t(8), &Response::Text("lang".into())).correct, None);
        assert_eq!(score_task(t(9), &Response::Empty).correct, None);
        // Wrong answer shape never scores, never panics.
        assert_eq!(score_task(t(0), &Response::Text("a".into())).points, 0.0);
        let mut answers = std::collections::HashMap::new();
        answers.insert(t(0).id.clone(), Response::Chosen(vec![0]));
        let sheet = score(&ws, &answers);
        assert_eq!(sheet.points, 1.0);
        assert_eq!(sheet.max, 8.0);
        assert_eq!(sheet.open, 1);
        assert!(matches!(Response::from_json(&t(2).body, &json!("richtig")), Response::Bool(true)));
        assert!(matches!(Response::from_json(&t(0).body, &json!(1)), Response::Chosen(v) if v == vec![1]));
    }

    #[test]
    fn categorize_parses_scores_and_round_trips() {
        let src = json!({"tasks": [{"type": "categorize", "prompt": "Sortiere", "points": 4,
            "categories": [{"name": "Säugetiere", "items": ["Wal", "Fledermaus"]}, {"name": "Fische", "items": ["Hai", "Forelle"]}]}]});
        let (ws, problems) = from_value(&src).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let t = &ws.tasks[0];
        let TaskBody::Categorize { categories } = &t.body else { panic!("{:?}", t.body) };
        assert_eq!(category_items(categories), vec![(0, "Wal"), (0, "Fledermaus"), (1, "Hai"), (1, "Forelle")]);
        assert_eq!(score_task(t, &Response::Assign(vec![Some(0), Some(0), Some(1), Some(1)])).correct, Some(true));
        let r = score_task(t, &Response::Assign(vec![Some(0), Some(1), None]));
        assert_eq!(r.parts, vec![true, false, false, false]);
        assert_eq!(r.points, 1.0);
        // Answers as category names (tool calls), case and spacing ignored; unknown names count as no answer.
        assert!(
            matches!(Response::from_json(&t.body, &json!(["säugetiere", "Säugetiere ", 1, "Vögel"])), Response::Assign(v) if v == vec![Some(0), Some(0), Some(1), None])
        );
        let (back, _) = parse_lenient(&to_json(&ws)).unwrap();
        assert_eq!(back, ws);
        // "sort" with groups is categorize, without them ordering.
        let (ws, _) = from_value(&json!({"tasks": [
            {"type": "sort", "prompt": "Ordne ein", "groups": [{"name": "A", "items": ["x"]}, {"name": "B", "items": ["y"]}]},
            {"type": "sortieren", "prompt": "Ordne", "items": ["1", "2"]}]}))
        .unwrap();
        assert_eq!(ws.tasks[0].body.type_name(), "categorize");
        assert_eq!(ws.tasks[1].body.type_name(), "ordering");
        // Too few groups or items is reported; hostile sizes are capped.
        for bad in [
            json!({"type": "categorize", "categories": [{"name": "A", "items": ["x", "y"]}]}),
            json!({"type": "categorize", "categories": [{"name": "A", "items": ["x"]}, {"name": "B"}]}),
        ] {
            assert!(from_value(&json!({"tasks": [bad]})).is_err());
        }
        let huge: Vec<Value> = (0..20).map(|i| json!({"name": format!("G{i}"), "items": vec!["x"; 50]})).collect();
        let (ws, _) = from_value(&json!({"tasks": [{"type": "categorize", "prompt": "p", "categories": huge}]})).unwrap();
        let TaskBody::Categorize { categories } = &ws.tasks[0].body else { panic!() };
        assert_eq!(categories.len(), MAX_CATEGORIES);
        assert_eq!(category_items(categories).len(), MAX_CATEGORY_ITEMS);
    }

    #[test]
    fn staged_hints_option_feedback_and_word_bank() {
        // An older sheet with one `hint` still reads, and a hint list is kept in order.
        let (ws, _) = from_value(&json!({"tasks": [
            {"type": "true_false", "prompt": "p", "answer": true, "hint": "Alter Tipp"},
            {"type": "true_false", "prompt": "p", "answer": true, "hints": ["Erst", "Dann", "Fast die Lösung"]},
            {"type": "true_false", "prompt": "p", "answer": true, "hints": (0..9).map(|i| i.to_string()).collect::<Vec<_>>()}]}))
        .unwrap();
        assert_eq!(ws.tasks[0].hints, vec!["Alter Tipp".to_string()]);
        assert_eq!(ws.tasks[1].hints.len(), 3);
        assert_eq!(ws.tasks[2].hints.len(), MAX_HINTS);
        assert!(to_json(&ws).contains("\"hints\""));
        // Feedback per option, next to the feedback of the task.
        let (ws, _) = from_value(&json!({"tasks": [{"type": "choice", "prompt": "p", "feedback": "Gut.",
            "options": [{"text": "a", "correct": true}, {"text": "b", "feedback": "b verwechselt Zähler und Nenner"}]}]}))
        .unwrap();
        let TaskBody::Choice { options, .. } = &ws.tasks[0].body else { panic!() };
        assert_eq!(options[1].feedback, "b verwechselt Zähler und Nenner");
        assert_eq!(ws.tasks[0].feedback, "Gut.");
        assert!(options[0].feedback.is_empty());
        // Word bank: each answer once plus the distractors, never in gap order.
        let (ws, _) = from_value(&json!({"tasks": [{"type": "cloze", "prompt": "p", "text": "[[eins]] [[zwei|2]] [[drei]] [[eins]]", "word_bank": true, "distractors": ["vier", "zwei"]}]})).unwrap();
        let TaskBody::Cloze { text, word_bank, distractors } = &ws.tasks[0].body else { panic!() };
        assert!(*word_bank);
        let bank = cloze_word_bank(text, distractors, &ws.tasks[0].id);
        let mut sorted = bank.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["drei", "eins", "vier", "zwei"]);
        assert_ne!(bank, vec!["eins", "zwei", "drei", "vier"]);
        // Old cloze JSON without the new fields reads, and the new fields are not written when off.
        let (ws, _) = parse_lenient(r#"{"tasks":[{"type":"cloze","prompt":"p","text":"[[a]]"}]}"#).unwrap();
        assert!(!to_json(&ws).contains("word_bank") && !to_json(&ws).contains("distractors"));
    }

    #[test]
    fn cloze_and_shuffles() {
        let parts = cloze_parts("A [[b|c]] d {{e}} [[\n]] f [[");
        assert_eq!(parts[0], ClozePart::Text("A "));
        assert_eq!(parts[1], ClozePart::Gap(vec!["b".into(), "c".into()]));
        assert_eq!(cloze_gaps("A [[b|c]] d {{e}} [[\n]] f [[").len(), 2);
        assert!(cloze_gaps("").is_empty());
        for n in 0..10 {
            let o = display_order(n, "x");
            let mut s = o.clone();
            s.sort_unstable();
            assert_eq!(s, (0..n).collect::<Vec<_>>());
            if n >= 2 {
                assert_ne!(o, (0..n).collect::<Vec<_>>());
            }
        }
        assert_eq!(parse_number("1 000,5"), Some(1000.5));
        assert_eq!(parse_number("NaN"), None);
    }
}
