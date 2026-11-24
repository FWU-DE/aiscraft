//! H5P content (#59): the `aiscraft-h5p` format, version 1, and the `content.json` of the
//! H5P content types AIS.ChatCraft writes and reads.
//!
//! An H5P artifact is one JSON object, like a worksheet: a title, an optional introduction and
//! a list of tasks in the worksheet's task format ([`crate::worksheet::Task`]), restricted to
//! the task types that have a plain H5P counterpart. Imported packages also carry
//! [`PackageInfo`]: the package file itself stays in the attachments folder, never in the
//! database.
//!
//! | task                             | H5P content type                    |
//! |----------------------------------|-------------------------------------|
//! | (several tasks or an intro)      | H5P.QuestionSet                     |
//! | `choice`                         | H5P.MultiChoice                     |
//! | `true_false`                     | H5P.TrueFalse                       |
//! | `cloze`                          | H5P.Blanks (`*Lösung/Alternative*`) |
//! | `cloze` with `word_bank`         | H5P.DragText (`*Wort*`)             |
//! | `short_answer`, `numeric`        | H5P.Blanks with one gap             |
//!
//! Everything here is pure (serde, no I/O); the zip package is written and read in the engine.
//! Every input is hostile: sizes and counts are capped through the worksheet parser.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::worksheet::{self, ChoiceOption, ClozePart, Task, TaskBody, Worksheet};

pub const FORMAT: &str = "aiscraft-h5p";
pub const VERSION: u32 = 1;
/// The fence language of H5P blocks in answers.
pub const FENCE: &str = "aiscraft-h5p";
pub const MAX_JSON_BYTES: usize = worksheet::MAX_JSON_BYTES;
/// The largest `content.json` read from a package.
pub const MAX_CONTENT_BYTES: usize = 4 * 1024 * 1024;

/// An H5P library as `h5p.json` and `content.json` name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Library {
    pub machine: &'static str,
    pub major: u32,
    pub minor: u32,
    /// What teachers call it.
    pub label: &'static str,
}

impl Library {
    /// `H5P.MultiChoice 1.16`, as a sub-content's `library` field names it.
    pub fn id(&self) -> String {
        format!("{} {}.{}", self.machine, self.major, self.minor)
    }
}

// Versions: the newest minor version that has been on the H5P Hub for a while, so an LMS that
// keeps its content types up to date has it. QuestionSet 1.21 exists since 2026-09; 1.20 is
// still what most installations have.
pub const QUESTION_SET: Library = Library { machine: "H5P.QuestionSet", major: 1, minor: 20, label: "Quiz (Question Set)" };
pub const MULTI_CHOICE: Library = Library { machine: "H5P.MultiChoice", major: 1, minor: 16, label: "Multiple Choice" };
pub const TRUE_FALSE: Library = Library { machine: "H5P.TrueFalse", major: 1, minor: 8, label: "Richtig oder falsch (True/False)" };
pub const BLANKS: Library = Library { machine: "H5P.Blanks", major: 1, minor: 14, label: "Lückentext (Fill in the Blanks)" };
pub const DRAG_TEXT: Library = Library { machine: "H5P.DragText", major: 1, minor: 10, label: "Wörter ziehen (Drag the Words)" };
/// The content types AIS.ChatCraft writes and can edit.
pub const SUPPORTED: &[Library] = &[QUESTION_SET, MULTI_CHOICE, TRUE_FALSE, BLANKS, DRAG_TEXT];

/// The task types the H5P editor offers: (`type`, label), a subset of [`worksheet::TASK_TYPES`].
pub const TASK_TYPES: &[(&str, &str)] = &[
    ("choice", "Multiple Choice (eine richtige)"),
    ("choice_multi", "Multiple Choice (mehrere richtige)"),
    ("true_false", "Richtig oder falsch"),
    ("cloze", "Lückentext zum Eintippen"),
    ("drag_text", "Wörter in Lücken ziehen"),
];

/// An empty task of a type from [`TASK_TYPES`].
pub fn template(kind: &str) -> Option<TaskBody> {
    match kind {
        "drag_text" => Some(TaskBody::Cloze { text: "Die Sonne ist ein [[Stern]].".into(), word_bank: true, distractors: vec!["Planet".into()] }),
        other => TaskBody::template(other),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct H5p {
    pub format: String,
    pub version: u32,
    pub title: String,
    /// Shown on the start page of a quiz (QuestionSet); a single task with an intro becomes a quiz.
    #[serde(default)]
    pub intro: String,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<PackageInfo>,
}

impl Default for H5p {
    fn default() -> Self {
        H5p { format: FORMAT.into(), version: VERSION, title: "H5P-Quiz".into(), intro: String::new(), tasks: Vec::new(), package: None }
    }
}

/// An imported package, kept as a file in the attachments folder.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PackageInfo {
    /// The file name in the attachments folder (`<uuid>.h5p`), never a path.
    pub file: String,
    /// `H5P.InteractiveVideo 1.26`.
    pub main_library: String,
    /// Every library `h5p.json` names (`H5P.Video 1.6`).
    #[serde(default)]
    pub libraries: Vec<String>,
    /// Files in the package.
    #[serde(default)]
    pub files: usize,
    /// Size of the package file.
    #[serde(default)]
    pub bytes: u64,
    /// The tasks were read from it and may be edited; otherwise it is kept as it is.
    #[serde(default)]
    pub editable: bool,
    /// [`fingerprint`] when imported: unchanged content saves the original package.
    #[serde(default)]
    pub fingerprint: String,
    /// What did not come across when reading (tips, media).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Is `name` a package file name the engine wrote (`<uuid>.h5p`)? Anything else (a path, `..`)
/// is refused before it gets near the file system.
pub fn is_package_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".h5p") else { return false };
    !stem.is_empty() && stem.len() <= 64 && stem.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

// ---- tasks and content types -----------------------------------------------------------------

/// The content type a task becomes, or why it has none.
pub fn library_for(body: &TaskBody) -> Result<Library, &'static str> {
    match body {
        TaskBody::Choice { .. } => Ok(MULTI_CHOICE),
        TaskBody::TrueFalse { .. } => Ok(TRUE_FALSE),
        TaskBody::Cloze { word_bank: true, .. } => Ok(DRAG_TEXT),
        TaskBody::Cloze { .. } | TaskBody::ShortAnswer { .. } | TaskBody::Numeric { .. } => Ok(BLANKS),
        TaskBody::Info { .. } => Err("Info-Texte gibt es in einem H5P-Quiz nicht (Text steht besser in der Einleitung)"),
        TaskBody::Matching { .. } => Err("Zuordnen hat in H5P kein einfaches Gegenstück (Drag and Drop braucht ein Bild-Layout)"),
        TaskBody::Ordering { .. } => Err("Reihenfolgen erzeugt AIS.ChatCraft noch nicht als H5P"),
        TaskBody::FreeText { .. } => Err("freie Texte (H5P.Essay) erzeugt AIS.ChatCraft noch nicht"),
        TaskBody::Categorize { .. } => Err("In Gruppen sortieren hat in H5P kein einfaches Gegenstück"),
    }
}

/// What H5P calls the content type of a task, for people.
pub fn task_label(body: &TaskBody) -> &'static str {
    match library_for(body) {
        Ok(l) => l.label,
        Err(_) => body.label(),
    }
}

// ---- parsing ---------------------------------------------------------------------------------

fn cap(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

fn str_field(v: &Value, names: &[&str], max: usize) -> String {
    names.iter().find_map(|n| v.get(*n).and_then(Value::as_str)).map(|s| cap(s, max)).unwrap_or_default()
}

fn json_slice(src: &str) -> &str {
    let t = src.trim();
    match (t.find('{'), t.rfind('}')) {
        (Some(a), Some(b)) if a < b => t.get(a..=b).unwrap_or(t),
        _ => t,
    }
}

/// Reads H5P content from model or user JSON (also a bare worksheet). Tasks that H5P cannot
/// express are left out and named in the problems. A `package` in the input is ignored: only
/// the engine sets it, when it imports a file ([`parse_stored`]).
pub fn parse_lenient(src: &str) -> Result<(H5p, Vec<String>), String> {
    if src.len() > MAX_JSON_BYTES {
        return Err(format!("Der H5P-Inhalt ist zu groß (über {} KB)", MAX_JSON_BYTES / 1024));
    }
    let v: Value = serde_json::from_str(json_slice(src)).map_err(|e| format!("Der H5P-Inhalt ist kein gültiges JSON: {e}"))?;
    from_value(&v)
}

/// Like [`parse_lenient`], from JSON already parsed (tool arguments).
pub fn from_value(v: &Value) -> Result<(H5p, Vec<String>), String> {
    read(v, false)
}

/// The content of a stored artifact: like [`parse_lenient`], but keeps the package reference
/// (when its file name is one the engine writes).
pub fn parse_stored(src: &str) -> Result<(H5p, Vec<String>), String> {
    if src.len() > MAX_JSON_BYTES {
        return Err(format!("Der H5P-Inhalt ist zu groß (über {} KB)", MAX_JSON_BYTES / 1024));
    }
    let v: Value = serde_json::from_str(json_slice(src)).map_err(|e| format!("Der H5P-Inhalt ist kein gültiges JSON: {e}"))?;
    read(&v, true)
}

fn read(v: &Value, keep_package: bool) -> Result<(H5p, Vec<String>), String> {
    let v = match v.get("h5p") {
        Some(inner) if inner.is_object() => inner,
        _ => v,
    };
    if !v.is_object() {
        return Err("Der H5P-Inhalt muss ein JSON-Objekt sein".into());
    }
    let mut problems = Vec::new();
    if let Some(f) = v.get("format").and_then(Value::as_str)
        && f != FORMAT
        && f != worksheet::FORMAT
    {
        problems.push(format!("Unbekanntes Format „{}“, als {FORMAT} gelesen", cap(f, 40)));
    }
    let package = if keep_package {
        v.get("package").and_then(|p| serde_json::from_value::<PackageInfo>(p.clone()).ok()).filter(|p| is_package_file_name(&p.file)).map(|mut p| {
            p.main_library = cap(&p.main_library, 120);
            p.libraries = p.libraries.iter().take(200).map(|l| cap(l, 120)).collect();
            p.notes = p.notes.iter().take(20).map(|n| cap(n, worksheet::MAX_SHORT)).collect();
            p
        })
    } else {
        None
    };
    let mut h = H5p {
        title: Some(str_field(v, &["title", "titel"], 200)).filter(|t| !t.is_empty()).unwrap_or_else(|| "H5P-Quiz".into()),
        intro: str_field(v, &["intro", "introduction", "einleitung", "instructions", "description"], worksheet::MAX_TEXT),
        package,
        ..H5p::default()
    };
    let has_tasks = ["tasks", "aufgaben", "questions", "items"].iter().any(|k| v.get(*k).and_then(Value::as_array).is_some_and(|a| !a.is_empty()));
    if has_tasks {
        let (ws, ws_problems) = worksheet::from_value(v)?;
        // The worksheet parser numbers its problems by task; keep them.
        problems.extend(ws_problems.into_iter().filter(|p| !p.starts_with("Unbekanntes Format")));
        let (tasks, dropped) = keep_mappable(ws.tasks);
        h.tasks = tasks;
        problems.extend(dropped);
    }
    let read_only = h.package.as_ref().is_some_and(|p| !p.editable);
    if h.tasks.is_empty() && !read_only {
        return Err(problems.into_iter().next().unwrap_or_else(|| "Der H5P-Inhalt hat keine Aufgaben, die H5P abbilden kann".into()));
    }
    Ok((h, problems))
}

/// Splits tasks into those H5P can express and the problems naming the others.
fn keep_mappable(tasks: Vec<Task>) -> (Vec<Task>, Vec<String>) {
    let mut keep = Vec::new();
    let mut dropped = Vec::new();
    for (i, t) in tasks.into_iter().enumerate() {
        match library_for(&t.body) {
            Ok(_) => keep.push(t),
            Err(why) => dropped.push(format!("Aufgabe {} ({}) nicht übernommen: {why}", i + 1, t.body.label())),
        }
    }
    (keep, dropped)
}

/// The canonical JSON of H5P content.
pub fn to_json(h: &H5p) -> String {
    serde_json::to_string_pretty(h).unwrap_or_else(|_| "{}".into())
}

/// The tasks as a worksheet (the editor and "Ausprobieren" work on worksheets).
pub fn as_worksheet(h: &H5p) -> Worksheet {
    Worksheet { title: h.title.clone(), instructions: h.intro.clone(), tasks: h.tasks.clone(), ..Worksheet::default() }
}

/// Takes title, intro and tasks back from the editor's worksheet; the package stays.
pub fn apply_worksheet(h: &mut H5p, ws: &Worksheet) {
    h.title = ws.title.clone();
    h.intro = ws.instructions.clone();
    h.tasks = ws.tasks.clone();
}

/// A short, stable hash of what a person can edit (title, intro, tasks).
pub fn fingerprint(h: &H5p) -> String {
    let src = serde_json::to_string(&(&h.title, &h.intro, &h.tasks)).unwrap_or_default();
    format!("{:016x}", fnv(src.as_bytes()))
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// Has the content changed since the package was imported? (Content without a package: yes.)
pub fn is_edited(h: &H5p) -> bool {
    h.package.as_ref().is_none_or(|p| p.fingerprint != fingerprint(h))
}

/// Problems a person should fix before saving, per task.
pub fn validate(h: &H5p) -> Vec<String> {
    let mut out = worksheet::validate(&as_worksheet(h));
    for (i, t) in h.tasks.iter().enumerate() {
        if let Err(why) = question(t) {
            out.push(format!("Aufgabe {}: {why}", i + 1));
        }
    }
    out
}

// ---- writing content.json --------------------------------------------------------------------

/// The `content.json` of a package and the libraries it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Built {
    pub main: Library,
    pub content: Value,
    /// The main library first, then every content type used inside it.
    pub dependencies: Vec<Library>,
    /// What does not come across (tips, tolerances), for the person.
    pub notes: Vec<String>,
}

/// How to build: for an imported package, keep its structure and its library versions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildOptions {
    /// Always a QuestionSet, also for a single task.
    pub force_set: bool,
    /// (machine name, major, minor) to use instead of ours, from the package's `h5p.json`.
    pub versions: Vec<(String, u32, u32)>,
}

fn versioned(l: Library, o: &BuildOptions) -> Library {
    match o.versions.iter().find(|(m, _, _)| m == l.machine) {
        Some((_, major, minor)) => Library { major: *major, minor: *minor, ..l },
        None => l,
    }
}

pub fn build(h: &H5p) -> Result<Built, String> {
    build_with(h, &BuildOptions::default())
}

pub fn build_with(h: &H5p, o: &BuildOptions) -> Result<Built, String> {
    if h.tasks.is_empty() {
        return Err("Es gibt keine Aufgabe, die sich als H5P speichern lässt".into());
    }
    let mut notes = Vec::new();
    let mut questions = Vec::new();
    for (i, t) in h.tasks.iter().enumerate() {
        let (lib, params, task_notes) = question(t).map_err(|why| format!("Aufgabe {}: {why}", i + 1))?;
        notes.extend(task_notes.into_iter().map(|n| format!("Aufgabe {}: {n}", i + 1)));
        questions.push((versioned(lib, o), params, t));
    }
    let single = questions.len() == 1 && h.intro.trim().is_empty() && !o.force_set;
    if single && let Some((lib, params, _)) = questions.first() {
        return Ok(Built { main: *lib, content: params.clone(), dependencies: vec![*lib], notes });
    }
    let main = versioned(QUESTION_SET, o);
    let mut dependencies = vec![main];
    for (lib, _, _) in &questions {
        if !dependencies.contains(lib) {
            dependencies.push(*lib);
        }
    }
    let list: Vec<Value> = questions
        .iter()
        .enumerate()
        .map(|(i, (lib, params, t))| {
            json!({
                "library": lib.id(),
                "params": params,
                "subContentId": sub_content_id(&h.title, i, &t.id),
                "metadata": {"contentType": lib.label, "license": "U", "title": short_title(t, i)},
            })
        })
        .collect();
    let content = json!({
        "introPage": {
            "showIntroPage": !h.intro.trim().is_empty(),
            "title": h.title,
            "introduction": html_para(&h.intro),
            "startButtonText": "Quiz starten",
        },
        "progressType": "dots",
        "passPercentage": 50,
        "questions": list,
        "texts": {
            "prevButton": "Vorherige Frage", "previous": "Zurück", "nextButton": "Nächste Frage", "next": "Weiter",
            "finishButton": "Beenden", "submitButton": "Absenden",
            "textualProgress": "Frage @current von @total", "jumpToQuestion": "Frage %d von %total",
            "questionLabel": "Frage", "readSpeakerProgress": "Frage @current von @total",
            "unansweredText": "Unbeantwortet", "answeredText": "Beantwortet", "currentQuestionText": "Aktuelle Frage",
            "navigationLabel": "Fragen", "questionSetInstruction": "Wähle die Frage aus", "noQuestionAdded": "Keine Frage vorhanden",
        },
        "disableBackwardsNavigation": false,
        "randomQuestions": false,
        "endGame": {
            "showResultPage": true, "showSolutionButton": true, "showRetryButton": true,
            "noResultMessage": "Fertig", "message": "Dein Ergebnis:", "scoreBarLabel": "Du hast @finals von @totals Punkten",
            "scoreHeader": "Ergebnis",
            "overallFeedback": [{"from": 0, "to": 100, "feedback": "Gut gemacht!"}],
            "solutionButtonText": "Lösung anzeigen", "retryButtonText": "Nochmal", "finishButtonText": "Beenden",
            "submitButtonText": "Absenden", "showAnimations": false, "skippable": false, "skipButtonText": "Video überspringen",
        },
        "override": {"checkButton": true},
    });
    Ok(Built { main, content, dependencies, notes })
}

/// `h5p.json` for built content.
pub fn h5p_json(title: &str, b: &Built) -> Value {
    json!({
        "title": title_for_package(title),
        "language": "de",
        "defaultLanguage": "de",
        "mainLibrary": b.main.machine,
        "embedTypes": ["iframe"],
        "license": "U",
        "preloadedDependencies": b.dependencies.iter().map(|l| json!({"machineName": l.machine, "majorVersion": l.major, "minorVersion": l.minor})).collect::<Vec<_>>(),
    })
}

/// `h5p.json` titles are 1 to 255 characters.
pub fn title_for_package(title: &str) -> String {
    let t: String = title.trim().chars().filter(|c| !c.is_control()).take(255).collect();
    if t.is_empty() { "H5P-Quiz".into() } else { t }
}

/// A UUID-shaped id, the same for the same content, as H5P wants for each sub-content.
fn sub_content_id(title: &str, i: usize, task_id: &str) -> String {
    let a = fnv(format!("{title}\u{1}{i}\u{1}{task_id}").as_bytes());
    let b = fnv(format!("{task_id}\u{2}{i}\u{2}{title}").as_bytes());
    let hex = format!("{a:016x}{b:016x}");
    let part = |r: std::ops::Range<usize>| hex.get(r).unwrap_or("0").to_string();
    format!("{}-{}-4{}-8{}-{}", part(0..8), part(8..12), part(13..16), part(17..20), part(20..32))
}

fn short_title(t: &Task, i: usize) -> String {
    let p = t.prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let p: String = p.chars().take(60).collect();
    if p.is_empty() { format!("Frage {}", i + 1) } else { p }
}

/// HTML-escaped text.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A paragraph of HTML from plain text (line breaks kept), empty for empty text.
fn html_para(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() { String::new() } else { format!("<p>{}</p>", esc(t).replace('\n', "<br>")) }
}

/// One answer for H5P's `*…*` syntax: `/` separates alternatives, `:` starts a tip and `*`
/// ends the gap, so answers containing them cannot be written.
fn gap_answer(a: &str) -> Option<String> {
    let a = a.trim();
    (!a.is_empty() && !a.contains(['*', '/', ':'])).then(|| esc(a))
}

/// A number answer in both spellings (`0,5` and `0.5`).
fn number_spellings(x: f64) -> Vec<String> {
    let dot = if (x - x.round()).abs() < 1e-9 { format!("{}", x.round() as i64) } else { format!("{x}") };
    let comma = dot.replace('.', ",");
    if comma == dot { vec![dot] } else { vec![comma, dot] }
}

/// A cloze text in H5P's gap syntax; `first_only`: DragText takes one word per gap.
fn h5p_gaps(text: &str, first_only: bool, notes: &mut Vec<String>, html: bool) -> Result<String, &'static str> {
    let mut out = String::new();
    let mut gaps = 0;
    for part in worksheet::cloze_parts(text) {
        match part {
            ClozePart::Text(s) => {
                if s.contains('*') && !notes.iter().any(|n| n.contains('*')) {
                    notes.push("Sternchen (*) im Text sind in H5P Lückenzeichen und wurden ersetzt".into());
                }
                let s = s.replace('*', "∗");
                out.push_str(&if html { esc(&s) } else { s });
            }
            ClozePart::Gap(answers) => {
                let total = answers.len();
                let usable: Vec<String> = answers.iter().filter_map(|a| gap_answer(a)).collect();
                if usable.len() < total && !notes.iter().any(|n| n.starts_with("Lösungen mit")) {
                    notes.push("Lösungen mit /, : oder * kann H5P nicht prüfen und wurden weggelassen".into());
                }
                let Some(first) = usable.first() else { return Err("eine Lücke hat nur Lösungen mit /, : oder *, die H5P nicht prüfen kann") };
                if first_only {
                    if usable.len() > 1 && !notes.iter().any(|n| n.starts_with("Beim Ziehen")) {
                        notes.push("Beim Ziehen zählt in H5P nur die erste Lösung jeder Lücke".into());
                    }
                    out.push_str(&format!("*{first}*"));
                } else {
                    out.push_str(&format!("*{}*", usable.join("/")));
                }
                gaps += 1;
            }
        }
    }
    if gaps == 0 {
        return Err("der Lückentext hat keine Lücken");
    }
    Ok(out)
}

fn blanks_lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::trim).filter(|l| !l.is_empty()).map(|l| format!("<p>{l}</p>")).collect()
}

fn confirm() -> (Value, Value) {
    (
        json!({"header": "Beenden?", "body": "Bist du sicher, dass du fertig bist?", "cancelLabel": "Abbrechen", "confirmLabel": "Beenden"}),
        json!({"header": "Nochmal?", "body": "Willst du es wirklich nochmal versuchen?", "cancelLabel": "Abbrechen", "confirmLabel": "Bestätigen"}),
    )
}

fn feedback_list(feedback: &str) -> Value {
    json!([{"from": 0, "to": 100, "feedback": feedback.trim()}])
}

/// One task as an H5P content type: (library, params, notes on what was lost).
pub fn question(t: &Task) -> Result<(Library, Value, Vec<String>), &'static str> {
    let lib = library_for(&t.body)?;
    let mut notes = Vec::new();
    if !t.hints.is_empty() && !matches!(t.body, TaskBody::Cloze { .. } | TaskBody::ShortAnswer { .. }) {
        notes.push("Tipps gibt es bei diesem H5P-Typ nicht".to_string());
    }
    let (check, retry) = confirm();
    let params = match &t.body {
        TaskBody::Choice { options, multiple } => {
            if options.iter().all(|o| o.text.trim().is_empty()) {
                return Err("die Auswahl hat keine Antworten");
            }
            if !options.iter().any(|o| o.correct) {
                return Err("keine Antwort ist als richtig markiert");
            }
            let answers: Vec<Value> = options
                .iter()
                .filter(|o| !o.text.trim().is_empty())
                .map(|o| {
                    json!({
                        "text": format!("<div>{}</div>", esc(o.text.trim())),
                        "correct": o.correct,
                        "tipsAndFeedback": {"tip": "", "chosenFeedback": html_para(&o.feedback), "notChosenFeedback": ""},
                    })
                })
                .collect();
            json!({
                "media": {"disableImageZooming": false},
                "question": html_para(&t.prompt),
                "answers": answers,
                "overallFeedback": feedback_list(&t.feedback),
                "behaviour": {
                    "enableRetry": true, "enableSolutionsButton": true, "enableCheckButton": true,
                    "type": if *multiple { "multi" } else { "single" }, "singlePoint": false, "randomAnswers": true,
                    "showSolutionsRequiresInput": true, "confirmCheckDialog": false, "confirmRetryDialog": false,
                    "autoCheck": false, "passPercentage": 100, "showScorePoints": true,
                },
                "UI": {
                    "checkAnswerButton": "Überprüfen", "submitAnswerButton": "Absenden", "showSolutionButton": "Lösung anzeigen",
                    "tryAgainButton": "Wiederholen", "tipsLabel": "Tipp anzeigen", "scoreBarLabel": "Du hast :num von :total Punkten",
                    "tipAvailable": "Tipp verfügbar", "feedbackAvailable": "Rückmeldung verfügbar", "readFeedback": "Rückmeldung vorlesen",
                    "wrongAnswer": "Falsche Antwort", "correctAnswer": "Richtige Antwort", "shouldCheck": "Hätte gewählt werden müssen",
                    "shouldNotCheck": "Hätte nicht gewählt werden dürfen", "noInput": "Bitte antworte, bevor du die Lösung ansiehst",
                    "a11yCheck": "Antworten überprüfen", "a11yShowSolution": "Lösung anzeigen", "a11yRetry": "Nochmal versuchen",
                },
                "confirmCheck": check,
                "confirmRetry": retry,
            })
        }
        TaskBody::TrueFalse { answer } => {
            json!({
                "media": {"disableImageZooming": false},
                "question": html_para(&t.prompt),
                "correct": if *answer { "true" } else { "false" },
                "l10n": {
                    "trueText": "Richtig", "falseText": "Falsch", "score": "Du hast @score von @total Punkten",
                    "checkAnswer": "Überprüfen", "submitAnswer": "Absenden", "showSolutionButton": "Lösung anzeigen",
                    "tryAgain": "Wiederholen", "wrongAnswerMessage": "Falsche Antwort", "correctAnswerMessage": "Richtige Antwort",
                    "scoreBarLabel": "Du hast :num von :total Punkten",
                    "a11yCheck": "Antworten überprüfen", "a11yShowSolution": "Lösung anzeigen", "a11yRetry": "Nochmal versuchen",
                },
                "behaviour": {
                    "enableRetry": true, "enableSolutionsButton": true, "enableCheckButton": true,
                    "confirmCheckDialog": false, "confirmRetryDialog": false, "autoCheck": false,
                    "feedbackOnCorrect": t.feedback.trim(), "feedbackOnWrong": t.feedback.trim(),
                },
                "confirmCheck": check,
                "confirmRetry": retry,
            })
        }
        TaskBody::Cloze { text, word_bank: true, distractors } => {
            let field = h5p_gaps(text, true, &mut notes, false)?;
            let extra: Vec<String> = distractors.iter().filter_map(|d| gap_answer(d)).map(|d| format!("*{d}*")).collect();
            if !t.hints.is_empty() {
                notes.push("Tipps gibt es beim Wörterziehen nicht".to_string());
            }
            json!({
                "media": {"disableImageZooming": false},
                "taskDescription": if t.prompt.trim().is_empty() { "<p>Ziehe die Wörter in die richtigen Lücken.</p>".to_string() } else { html_para(&t.prompt) },
                "textField": field,
                "distractors": extra.join(" "),
                "overallFeedback": feedback_list(&t.feedback),
                "checkAnswer": "Überprüfen", "submitAnswer": "Absenden", "tryAgain": "Wiederholen", "showSolution": "Lösung anzeigen",
                "dropZoneIndex": "Lücke @index.", "empty": "Lücke @index ist leer.", "contains": "Lücke @index enthält @draggable.",
                "ariaDraggableIndex": "@index von @count Wörtern.", "tipLabel": "Tipp anzeigen", "correctText": "Richtig!",
                "incorrectText": "Falsch!", "resetDropTitle": "Lücke leeren", "resetDropDescription": "Willst du diese Lücke wirklich leeren?",
                "grabbed": "Wort aufgenommen.", "cancelledDragging": "Ziehen abgebrochen.", "correctAnswer": "Richtige Antwort:",
                "feedbackHeader": "Rückmeldung", "scoreBarLabel": "Du hast :num von :total Punkten",
                "a11yCheck": "Antworten überprüfen", "a11yShowSolution": "Lösung anzeigen", "a11yRetry": "Nochmal versuchen",
                "behaviour": {"enableRetry": true, "enableSolutionsButton": true, "enableCheckButton": true, "instantFeedback": false},
            })
        }
        TaskBody::Cloze { text, .. } => {
            if !t.hints.is_empty() {
                notes.push("Tipps gibt es im H5P-Lückentext nur je Lücke; sie wurden weggelassen".to_string());
            }
            let field = h5p_gaps(text, false, &mut notes, true)?;
            blanks(&t.prompt, "Fülle die Lücken aus.", blanks_lines(&field), false, &t.feedback)
        }
        TaskBody::ShortAnswer { accepted, case_sensitive } => {
            let usable: Vec<String> = accepted.iter().filter_map(|a| gap_answer(a)).collect();
            if usable.is_empty() {
                return Err("keine Antwort, die H5P prüfen kann (ohne /, : und *)");
            }
            if usable.len() < accepted.iter().filter(|a| !a.trim().is_empty()).count() {
                notes.push("Antworten mit /, : oder * wurden weggelassen".to_string());
            }
            blanks("", "Beantworte die Frage.", vec![format!("<p>{} *{}*</p>", esc(t.prompt.trim()), usable.join("/"))], *case_sensitive, &t.feedback)
        }
        TaskBody::Numeric { answer, tolerance, unit } => {
            if !answer.is_finite() {
                return Err("die Lösung ist keine Zahl");
            }
            if *tolerance > 0.0 {
                notes.push("H5P prüft Zahlen genau; die Toleranz geht verloren".to_string());
            }
            let unit = if unit.trim().is_empty() { String::new() } else { format!(" {}", esc(unit.trim())) };
            blanks(
                "",
                "Gib die Zahl ein.",
                vec![format!("<p>{} *{}*{unit}</p>", esc(t.prompt.trim()), number_spellings(*answer).join("/"))],
                false,
                &t.feedback,
            )
        }
        _ => return Err("diesen Aufgabentyp gibt es in H5P nicht"),
    };
    Ok((lib, params, notes))
}

fn blanks(prompt: &str, fallback: &str, questions: Vec<String>, case_sensitive: bool, feedback: &str) -> Value {
    let (check, retry) = confirm();
    json!({
        "media": {"disableImageZooming": false},
        "text": if prompt.trim().is_empty() { format!("<p>{fallback}</p>") } else { html_para(prompt) },
        "questions": questions,
        "overallFeedback": feedback_list(feedback),
        "showSolutions": "Lösung anzeigen", "tryAgain": "Wiederholen", "checkAnswer": "Überprüfen", "submitAnswer": "Absenden",
        "notFilledOut": "Bitte fülle alle Lücken aus, um die Lösung zu sehen", "answerIsCorrect": "„:ans“ ist richtig",
        "answerIsWrong": "„:ans“ ist falsch", "answeredCorrectly": "Richtig beantwortet", "answeredIncorrectly": "Falsch beantwortet",
        "solutionLabel": "Richtige Antwort:", "inputLabel": "Lücke @num von @total", "inputHasTipLabel": "Tipp verfügbar",
        "tipLabel": "Tipp", "scoreBarLabel": "Du hast :num von :total Punkten",
        "a11yCheck": "Antworten überprüfen", "a11yShowSolution": "Lösung anzeigen", "a11yRetry": "Nochmal versuchen",
        "behaviour": {
            "enableRetry": true, "enableSolutionsButton": true, "enableCheckButton": true, "autoCheck": false,
            "caseSensitive": case_sensitive, "showSolutionsRequiresInput": true, "separateLines": false,
            "confirmCheckDialog": false, "confirmRetryDialog": false, "acceptSpellingErrors": false,
        },
        "confirmCheck": check,
        "confirmRetry": retry,
    })
}

// ---- reading content.json --------------------------------------------------------------------

/// Plain text from the HTML of an H5P text field: tags dropped, line breaks at `<br>` and block
/// ends, the common entities decoded.
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(rest.get(..i).unwrap_or(""));
        let tail = rest.get(i..).unwrap_or("");
        let tag_start = tail.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic() || c == '/' || c == '!');
        if tail.starts_with('<') && !tag_start {
            out.push('<');
            rest = tail.get(1..).unwrap_or("");
        } else if tail.starts_with('<') {
            let end = tail.find('>').map(|j| j + 1).unwrap_or(tail.len());
            let tag = tail.get(1..end.saturating_sub(1)).unwrap_or("").trim().to_ascii_lowercase();
            let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            if name == "br" || (tag.starts_with('/') && matches!(name.as_str(), "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "tr")) {
                out.push('\n');
            }
            rest = tail.get(end..).unwrap_or("");
        } else {
            let end = tail.find(';').filter(|j| *j <= 10);
            let ent = end.and_then(|j| tail.get(1..j)).unwrap_or("");
            let decoded = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "auml" => Some('ä'),
                "ouml" => Some('ö'),
                "uuml" => Some('ü'),
                "Auml" => Some('Ä'),
                "Ouml" => Some('Ö'),
                "Uuml" => Some('Ü'),
                "szlig" => Some('ß'),
                e if e.starts_with("#x") || e.starts_with("#X") => e.get(2..).and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32),
                e if e.starts_with('#') => e.get(1..).and_then(|d| d.parse::<u32>().ok()).and_then(char::from_u32),
                _ => None,
            };
            match (decoded, end) {
                (Some(c), Some(j)) => {
                    out.push(c);
                    rest = tail.get(j + 1..).unwrap_or("");
                }
                _ => {
                    out.push('&');
                    rest = tail.get(1..).unwrap_or("");
                }
            }
        }
    }
    out.push_str(rest);
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let mut joined = String::new();
    let mut blank = false;
    for l in lines {
        if l.is_empty() {
            blank = !joined.is_empty();
            continue;
        }
        if !joined.is_empty() {
            joined.push_str(if blank { "\n\n" } else { "\n" });
        }
        blank = false;
        joined.push_str(l);
    }
    joined
}

/// H5P's `*Lösung/Alternative:Tipp*` gaps as worksheet gaps `[[Lösung|Alternative]]`.
/// `tips`: set when a gap had a tip (they are dropped).
fn worksheet_gaps(text: &str, tips: &mut bool) -> String {
    let parts: Vec<&str> = text.split('*').collect();
    // An odd number of parts means every `*` has its partner.
    if parts.len().is_multiple_of(2) {
        return text.replace(['[', ']'], "");
    }
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i % 2 == 0 {
            out.push_str(&p.replace("[[", "[").replace("]]", "]"));
        } else {
            let (answers, tip) = p.split_once(':').unwrap_or((p, ""));
            *tips |= !tip.trim().is_empty();
            let alts: Vec<String> = answers.split('/').map(|a| a.trim().replace(['[', ']', '|'], "")).filter(|a| !a.is_empty()).collect();
            if alts.is_empty() {
                continue;
            }
            out.push_str(&format!("[[{}]]", alts.join("|")));
        }
    }
    out
}

fn machine_of(library: &str) -> &str {
    library.split_whitespace().next().unwrap_or("")
}

/// Reads the tasks of a package's `content.json`. `Err` names a content type AIS.ChatCraft
/// cannot edit; the package is then kept as it is.
pub fn read_content(main_library: &str, content: &Value) -> Result<(String, Vec<Task>, Vec<String>), String> {
    let mut notes = Vec::new();
    let machine = machine_of(main_library);
    let (intro, raw) = if machine == QUESTION_SET.machine {
        let intro = content.pointer("/introPage/introduction").and_then(Value::as_str).map(plain).unwrap_or_default();
        let list = content.get("questions").and_then(Value::as_array).ok_or("Das Quiz hat keine Fragen")?;
        if list.len() > worksheet::MAX_TASKS {
            return Err(format!("Das Quiz hat mehr als {} Fragen", worksheet::MAX_TASKS));
        }
        let mut raw = Vec::new();
        for (i, q) in list.iter().enumerate() {
            let lib = q.get("library").and_then(Value::as_str).unwrap_or("");
            let params = q.get("params").unwrap_or(&Value::Null);
            raw.push(read_question(machine_of(lib), params, &mut notes).map_err(|e| format!("Frage {}: {e}", i + 1))?);
        }
        (intro, raw)
    } else {
        (String::new(), vec![read_question(machine, content, &mut notes)?])
    };
    // Through the worksheet parser: caps every text and count, numbers the tasks.
    let (ws, _) = worksheet::from_value(&json!({"tasks": raw}))?;
    Ok((cap(&intro, worksheet::MAX_TEXT), ws.tasks, notes))
}

fn text_at(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).map(plain).unwrap_or_default()
}

fn read_question(machine: &str, p: &Value, notes: &mut Vec<String>) -> Result<Value, String> {
    if p.pointer("/media/type/library").and_then(Value::as_str).is_some_and(|l| !l.is_empty()) && !notes.iter().any(|n| n.starts_with("Bilder")) {
        notes.push("Bilder und Videos an den Fragen bleiben im Paket, zeigt AIS.ChatCraft aber nicht".into());
    }
    let feedback = p.pointer("/overallFeedback/0/feedback").and_then(Value::as_str).map(plain).unwrap_or_default();
    Ok(match machine {
        "H5P.MultiChoice" => {
            let answers = p.get("answers").and_then(Value::as_array).ok_or("Multiple Choice ohne Antworten")?;
            let options: Vec<ChoiceOption> = answers
                .iter()
                .take(worksheet::MAX_OPTIONS)
                .map(|a| ChoiceOption {
                    text: text_at(a, "text"),
                    correct: a.get("correct").and_then(Value::as_bool).unwrap_or(false),
                    feedback: a.pointer("/tipsAndFeedback/chosenFeedback").and_then(Value::as_str).map(plain).unwrap_or_default(),
                })
                .collect();
            let n_correct = options.iter().filter(|o| o.correct).count();
            let multiple = match p.pointer("/behaviour/type").and_then(Value::as_str) {
                Some("multi") => true,
                Some("single") => false,
                _ => n_correct > 1,
            };
            json!({"type": "choice", "prompt": text_at(p, "question"), "options": options, "multiple": multiple, "feedback": feedback})
        }
        "H5P.TrueFalse" => {
            let answer = match p.get("correct") {
                Some(Value::String(s)) => s == "true",
                Some(Value::Bool(b)) => *b,
                _ => true,
            };
            let feedback = p.pointer("/behaviour/feedbackOnCorrect").and_then(Value::as_str).map(plain).unwrap_or_default();
            json!({"type": "true_false", "prompt": text_at(p, "question"), "answer": answer, "feedback": feedback})
        }
        "H5P.Blanks" => {
            let lines: Vec<String> =
                p.get("questions").and_then(Value::as_array).map(|a| a.iter().take(200).filter_map(Value::as_str).map(plain).collect()).unwrap_or_default();
            let mut tips = false;
            let text = worksheet_gaps(&lines.join("\n"), &mut tips);
            if tips && !notes.iter().any(|n| n.starts_with("Tipps")) {
                notes.push("Tipps an einzelnen Lücken liest AIS.ChatCraft nicht; beim Speichern nach Änderungen fehlen sie".into());
            }
            json!({"type": "cloze", "prompt": text_at(p, "text"), "text": text, "feedback": feedback})
        }
        "H5P.DragText" => {
            let mut tips = false;
            let text = worksheet_gaps(&p.get("textField").and_then(Value::as_str).unwrap_or("").replace("\r\n", "\n"), &mut tips);
            let distractors: Vec<String> = p
                .get("distractors")
                .and_then(Value::as_str)
                .map(|d| {
                    d.split('*')
                        .enumerate()
                        .filter(|(i, s)| i % 2 == 1 && !s.trim().is_empty())
                        .map(|(_, s)| s.split(':').next().unwrap_or("").trim().to_string())
                        .collect()
                })
                .unwrap_or_default();
            json!({"type": "cloze", "prompt": text_at(p, "taskDescription"), "text": text, "word_bank": true, "distractors": distractors, "feedback": feedback})
        }
        "" => return Err("Inhaltstyp fehlt".into()),
        other => return Err(format!("{} kann AIS.ChatCraft nicht bearbeiten", cap(other, 60))),
    })
}

/// What models are told about the format (for the prompt and the tool description).
pub fn prompt_schema() -> &'static str {
    r#"Format aiscraft-h5p (JSON, ein Objekt):
{"format":"aiscraft-h5p","version":1,"title":"…","intro":"Einleitung für die Startseite (optional)","tasks":[…]}
Eine Aufgabe ohne intro wird ein einzelner H5P-Inhalt, mehrere werden ein Quiz (H5P.QuestionSet). Jede Aufgabe hat "type", "prompt" (Aufgabenstellung) und optional "feedback" (Rückmeldung nach dem Prüfen). Nur diese Typen:
- {"type":"choice","prompt":"…","multiple":false,"options":[{"text":"…","correct":true},{"text":"…","correct":false,"feedback":"Warum diese Antwort nicht stimmt"}]} (H5P.MultiChoice)
- {"type":"true_false","prompt":"Aussage …","answer":true} (H5P.TrueFalse)
- {"type":"cloze","prompt":"Ergänze.","text":"Die Hauptstadt ist [[Berlin]]. Alternativen mit |: [[Fotosynthese|Photosynthese]]"} (H5P.Blanks, zum Eintippen)
- {"type":"cloze","prompt":"Ziehe die Wörter in die Lücken.","text":"Der [[Igel]] frisst [[Insekten]].","word_bank":true,"distractors":["Gras"]} (H5P.DragText, Wörter ziehen)
Lösungen dürfen kein /, : oder * enthalten."#
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
    use super::*;

    fn sample() -> H5p {
        parse_lenient(
            &json!({
                "format": "aiscraft-h5p", "title": "Brüche", "intro": "Viel Erfolg!",
                "tasks": [
                    {"type": "choice", "prompt": "Größer?", "options": [{"text": "1/2", "correct": true, "feedback": "Ja"}, {"text": "1/3"}]},
                    {"type": "true_false", "prompt": "1/4 < 1/2", "answer": true},
                    {"type": "cloze", "prompt": "Ergänze", "text": "Ein Halb ist [[0,5|0.5]].\nUnd [[ein Viertel|1/4]] ist 0,25."},
                    {"type": "cloze", "prompt": "Ziehe", "text": "Der [[Igel]] frisst [[Insekten|Käfer]].", "word_bank": true, "distractors": ["Gras"]},
                    {"type": "matching", "prompt": "Ordne zu", "pairs": [{"left": "a", "right": "b"}, {"left": "c", "right": "d"}]}
                ]
            })
            .to_string(),
        )
        .unwrap()
        .0
    }

    #[test]
    fn parses_and_drops_what_h5p_cannot_express() {
        let src = json!({"title": "X", "tasks": [{"type": "ordering", "prompt": "p", "items": ["a", "b"]}, {"type": "true_false", "prompt": "Ja?", "answer": false}], "package": {"file": "../../etc/passwd", "main_library": "x"}});
        let (h, problems) = from_value(&src).unwrap();
        assert_eq!(h.tasks.len(), 1);
        assert!(h.package.is_none(), "models never set a package");
        assert!(problems.iter().any(|p| p.contains("Aufgabe 1 (Reihenfolge)")), "{problems:?}");
        assert!(from_value(&json!({"tasks": [{"type": "info", "text": "x"}]})).is_err());
        assert!(parse_lenient("kein json").is_err());
        assert!(parse_lenient("[1]").is_err());
        // Stored content keeps a package with a file name we write, and nothing else.
        let stored = json!({"title": "V", "package": {"file": "0a1b-2c.h5p", "main_library": "H5P.InteractiveVideo 1.26", "editable": false}});
        let (h, _) = parse_stored(&stored.to_string()).unwrap();
        assert_eq!(h.package.as_ref().unwrap().main_library, "H5P.InteractiveVideo 1.26");
        assert!(h.tasks.is_empty());
        let bad = json!({"title": "V", "package": {"file": "../x.h5p", "main_library": "H5P.X 1.0", "editable": false}});
        assert!(parse_stored(&bad.to_string()).is_err());
        assert!(is_package_file_name("3f2a-bc.h5p") && !is_package_file_name("a/b.h5p") && !is_package_file_name(".h5p") && !is_package_file_name("x.zip"));
    }

    #[test]
    fn builds_a_question_set_with_the_right_libraries() {
        let h = sample();
        assert_eq!(h.tasks.len(), 4);
        let b = build(&h).unwrap();
        assert_eq!(b.main, QUESTION_SET);
        let names: Vec<&str> = b.dependencies.iter().map(|l| l.machine).collect();
        assert_eq!(names, vec!["H5P.QuestionSet", "H5P.MultiChoice", "H5P.TrueFalse", "H5P.Blanks", "H5P.DragText"]);
        let q = &b.content["questions"];
        assert_eq!(q.as_array().unwrap().len(), 4);
        assert_eq!(q[0]["library"], "H5P.MultiChoice 1.16");
        assert_eq!(q[0]["params"]["answers"][0]["correct"], true);
        assert_eq!(q[0]["params"]["behaviour"]["type"], "single");
        assert_eq!(q[1]["params"]["correct"], "true");
        // "1/4" cannot be an H5P answer (the slash separates alternatives): dropped, noted.
        assert_eq!(q[2]["params"]["questions"], json!(["<p>Ein Halb ist *0,5/0.5*.</p>", "<p>Und *ein Viertel* ist 0,25.</p>"]));
        assert_eq!(q[3]["params"]["textField"], "Der *Igel* frisst *Insekten*.");
        assert_eq!(q[3]["params"]["distractors"], "*Gras*");
        assert!(b.notes.iter().any(|n| n.contains("/, : oder *")), "{:?}", b.notes);
        assert!(b.notes.iter().any(|n| n.contains("erste Lösung")));
        let id = q[0]["subContentId"].as_str().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(id, build(&h).unwrap().content["questions"][0]["subContentId"].as_str().unwrap(), "stable ids");
        assert_eq!(b.content["introPage"]["showIntroPage"], true);
        let meta = h5p_json(&h.title, &b);
        assert_eq!(meta["mainLibrary"], "H5P.QuestionSet");
        assert_eq!(meta["preloadedDependencies"][0], json!({"machineName": "H5P.QuestionSet", "majorVersion": 1, "minorVersion": 20}));
        assert_eq!(meta["language"], "de");
    }

    #[test]
    fn a_single_task_is_its_own_content_type() {
        let (h, _) = from_value(&json!({"title": "Eine", "tasks": [{"type": "true_false", "prompt": "Wasser ist nass", "answer": true}]})).unwrap();
        let b = build(&h).unwrap();
        assert_eq!(b.main, TRUE_FALSE);
        assert_eq!(b.dependencies, vec![TRUE_FALSE]);
        assert_eq!(b.content["question"], "<p>Wasser ist nass</p>");
        // Imported as a quiz: stays a quiz, with the package's versions.
        let o = BuildOptions { force_set: true, versions: vec![("H5P.QuestionSet".into(), 1, 17), ("H5P.TrueFalse".into(), 1, 6)] };
        let b = build_with(&h, &o).unwrap();
        assert_eq!((b.main.machine, b.main.minor), ("H5P.QuestionSet", 17));
        assert_eq!(b.content["questions"][0]["library"], "H5P.TrueFalse 1.6");
        assert!(build(&H5p::default()).is_err());
    }

    #[test]
    fn short_answers_and_numbers_become_one_gap() {
        let (h, _) = from_value(&json!({"tasks": [
            {"type": "short_answer", "prompt": "Hauptstadt von Frankreich?", "accepted": ["Paris", "a/b"]},
            {"type": "numeric", "prompt": "Wie lang?", "answer": 2.5, "tolerance": 0.1, "unit": "cm"}
        ]}))
        .unwrap();
        let b = build(&h).unwrap();
        assert_eq!(b.content["questions"][0]["params"]["questions"][0], "<p>Hauptstadt von Frankreich? *Paris*</p>");
        assert_eq!(b.content["questions"][1]["params"]["questions"][0], "<p>Wie lang? *2,5/2.5* cm</p>");
        assert!(b.notes.iter().any(|n| n.contains("Toleranz")));
    }

    #[test]
    fn reads_what_it_writes() {
        let h = sample();
        let b = build(&h).unwrap();
        let (intro, tasks, _) = read_content(&b.main.id(), &b.content).unwrap();
        assert_eq!(intro, "Viel Erfolg!");
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].prompt, "Größer?");
        assert_eq!(tasks[0].body, h.tasks[0].body);
        assert_eq!(tasks[1].body, TaskBody::TrueFalse { answer: true });
        match &tasks[2].body {
            TaskBody::Cloze { text, word_bank, .. } => {
                assert_eq!(text, "Ein Halb ist [[0,5|0.5]].\nUnd [[ein Viertel]] ist 0,25.");
                assert!(!word_bank);
            }
            other => panic!("{other:?}"),
        }
        match &tasks[3].body {
            TaskBody::Cloze { text, word_bank, distractors } => {
                assert_eq!(text, "Der [[Igel]] frisst [[Insekten]].");
                assert!(*word_bank);
                assert_eq!(distractors, &vec!["Gras".to_string()]);
            }
            other => panic!("{other:?}"),
        }
        // Each type on its own, too.
        for t in &h.tasks {
            let (lib, params, _) = question(t).unwrap();
            assert_eq!(read_content(&lib.id(), &params).unwrap().1.len(), 1);
        }
    }

    #[test]
    fn unsupported_content_types_are_named() {
        let e = read_content("H5P.InteractiveVideo 1.26", &json!({})).unwrap_err();
        assert!(e.contains("H5P.InteractiveVideo"), "{e}");
        let set = json!({"questions": [{"library": "H5P.MultiChoice 1.16", "params": {"question": "q", "answers": [{"text": "a", "correct": true}]}}, {"library": "H5P.Essay 1.5", "params": {}}]});
        let e = read_content("H5P.QuestionSet 1.20", &set).unwrap_err();
        assert!(e.starts_with("Frage 2") && e.contains("H5P.Essay"), "{e}");
        assert!(read_content("H5P.QuestionSet 1.20", &json!({"questions": "x"})).is_err());
        assert!(read_content("", &json!(null)).is_err());
    }

    #[test]
    fn html_to_plain_text() {
        assert_eq!(plain("<p>Ein &amp; zwei&nbsp;<strong>drei</strong></p>\n<p>vier<br>f&uuml;nf &#252; &#xFC;</p>"), "Ein & zwei drei\n\nvier\nfünf ü ü");
        assert_eq!(plain("a < b & c"), "a < b & c");
        assert_eq!(plain("<p unclosed"), "");
        assert_eq!(plain("&#99999999999;"), "&#99999999999;");
        let mut tips = false;
        assert_eq!(worksheet_gaps("Ein *Hund/Köter:Tier* und *Katze*", &mut tips), "Ein [[Hund|Köter]] und [[Katze]]");
        assert!(tips);
        assert_eq!(worksheet_gaps("ohne Partner * hier", &mut tips), "ohne Partner * hier");
        assert_eq!(esc("<a href=\"x\">&"), "&lt;a href=&quot;x&quot;&gt;&amp;");
    }

    #[test]
    fn fingerprint_tracks_edits() {
        let mut h = sample();
        h.package = Some(PackageInfo { file: "a.h5p".into(), fingerprint: fingerprint(&h), editable: true, ..PackageInfo::default() });
        assert!(!is_edited(&h));
        h.title.push('!');
        assert!(is_edited(&h));
        assert!(is_edited(&H5p::default()));
        let ws = as_worksheet(&h);
        let mut back = H5p::default();
        apply_worksheet(&mut back, &ws);
        assert_eq!((back.title, back.tasks.len()), (h.title.clone(), 4));
        assert!(validate(&h).is_empty(), "{:?}", validate(&h));
        assert!(template("drag_text").is_some() && template("choice").is_some() && template("x").is_none());
        assert_eq!(task_label(&h.tasks[3].body), DRAG_TEXT.label);
        assert_eq!(title_for_package("  "), "H5P-Quiz");
        assert_eq!(number_spellings(3.0), vec!["3"]);
    }
}
