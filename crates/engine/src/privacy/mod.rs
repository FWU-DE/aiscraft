//! „Namen schützen“ (#55): names of pupils and other personal data are replaced by placeholders
//! (`[S1]`, `[N1]`, `[E-Mail 1]` …) before anything leaves the device, and translated back
//! locally when the answer is shown, copied or exported. The pattern is LLM Guard's
//! Anonymize/Vault/Deanonymize, like the vault's placeholders (`vault.rs`), in Rust.
//!
//! - Finding: class lists (per project or for all chats), rules with checksums, common first
//!   names with context rules (stage c without a model) and, once downloaded on the person's
//!   yes, a local GLiNER model (stage c with a model), see [`detect`], [`common`] and [`ner`].
//! - The mapping placeholder → original lives per chat in the local database (settings key
//!   `privacy.map.<chat>`), never in logs, never in the state agents read (`#[serde(skip)]`),
//!   never in a request.
//! - Messages keep what the backend saw (AIS.chat's history has only placeholders, so both
//!   views agree); [`reveal`] translates back for people.
//! - Every outgoing request passes [`Guard`] (in `chat.rs`, where the payload is built): user
//!   messages the person reviewed in the preview carry the step [`PROTECTED`] and stay as they
//!   were sent; everything else (system prompt, memory, project instructions, attached documents,
//!   tool results, older messages, background prompts) is protected automatically. Unsure findings
//!   (a name that is also a word, a lower-case name, a date of school age) are replaced too:
//!   privacy first (owner, 2026-11-24); the preview lets the person take single ones back.

pub mod common;
pub mod detect;
pub mod ner;
#[cfg(feature = "ner")]
mod ner_model;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;

use aiscraft_core::{Error, Result, Sources};
use detect::{Detector, Found, Kind, Known, Person};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id, now_ms};

/// Label of the step on a user message that went out through the preview (with placeholders).
pub const PROTECTED: &str = "Namen geschützt";
const LISTS_KEY: &str = "privacy.lists";
const MAX_LISTS: usize = 200;
const MAX_TEXT: usize = 2_000_000;
const MAX_MANUAL: usize = 500;
const MAX_ENTRIES: usize = 20_000;

/// A class list: pupils for one project or for all chats.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassList {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub people: Vec<Person>,
    #[serde(default)]
    pub created_at: i64,
}

/// What the state shows about a list: no names.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClassListInfo {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub count: usize,
}

/// One placeholder of a chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapEntry {
    pub placeholder: String,
    pub original: String,
    pub kind: Kind,
    pub key: String,
}

/// The placeholders of one chat. Stable: the same pupil keeps `[S1]` for the whole chat.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mapping {
    pub entries: Vec<MapEntry>,
    /// Pupils (and shared first or last names) in the order they got a number.
    pub persons: Vec<String>,
    /// Terms the person added by hand.
    pub manual: Vec<String>,
    /// The model was told what the placeholders are (backends without a system prompt keep it
    /// in their history, so once per chat).
    pub noted: bool,
}

impl Mapping {
    fn known(&self) -> Vec<Known> {
        self.entries.iter().map(|e| Known { kind: e.kind, key: e.key.clone(), display: e.original.clone() }).collect()
    }

    /// The placeholder for a finding, new if needed.
    pub fn placeholder_for(&mut self, f: &Found) -> String {
        if let Some(e) = self.entries.iter().find(|e| e.key == f.key && e.kind == f.kind) {
            return e.placeholder.clone();
        }
        let placeholder = match f.kind {
            Kind::FirstName | Kind::LastName => {
                let idx = match self.persons.iter().position(|p| *p == f.key) {
                    Some(i) => i + 1,
                    None => {
                        self.persons.push(f.key.clone());
                        self.persons.len()
                    }
                };
                format!("[{}{idx}]", if f.kind == Kind::FirstName { "S" } else { "N" })
            }
            k => format!("[{} {}]", k.label(), self.entries.iter().filter(|e| e.kind == k).count() + 1),
        };
        if self.entries.len() < MAX_ENTRIES {
            self.entries.push(MapEntry { placeholder: placeholder.clone(), original: f.display.clone(), kind: f.kind, key: f.key.clone() });
        }
        placeholder
    }

    fn lookup(&self) -> HashMap<&str, &str> {
        self.entries.iter().map(|e| (e.placeholder.as_str(), e.original.as_str())).collect()
    }

    /// Placeholders back to what they stand for.
    pub fn reveal(&self, text: &str) -> String {
        if self.entries.is_empty() || !text.contains('[') {
            return text.to_string();
        }
        let map = self.lookup();
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            out.push_str(rest.get(..open).unwrap_or(""));
            let after = rest.get(open..).unwrap_or("");
            let close = after.get(..after.len().min(24)).and_then(|w| w.find(']'));
            match close.and_then(|c| after.get(..=c)).and_then(|ph| map.get(ph).map(|o| (ph.len(), *o))) {
                Some((len, original)) => {
                    out.push_str(original);
                    rest = after.get(len..).unwrap_or("");
                }
                None => {
                    out.push('[');
                    rest = after.get(1..).unwrap_or("");
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Like [`Mapping::reveal`], for text that is still streaming: a placeholder cut off at the
    /// end ("… [S") waits for the next chunk instead of flashing up.
    pub fn reveal_streaming(&self, text: &str) -> String {
        let shown = self.reveal(text);
        if let Some(open) = shown.rfind('[') {
            let tail = shown.get(open..).unwrap_or("");
            if !tail.contains(']') && self.entries.iter().any(|e| e.placeholder.starts_with(tail)) {
                return shown.get(..open).unwrap_or("").to_string();
            }
        }
        shown
    }
}

/// A person's choices in the preview: hits switched off or on (by text and occurrence) and
/// terms added by hand.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Decisions {
    pub off: Vec<(String, usize)>,
    pub on: Vec<(String, usize)>,
    pub add: Vec<String>,
}

impl Decisions {
    pub fn from_value(v: Option<&Value>) -> Decisions {
        let pairs = |k: &str| -> Vec<(String, usize)> {
            v.and_then(|v| v.get(k))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .take(1000)
                        .filter_map(|x| {
                            let t = x.get("text").and_then(Value::as_str)?;
                            let n = x.get("n").and_then(Value::as_u64).unwrap_or(0);
                            Some((t.to_lowercase(), usize::try_from(n).unwrap_or(0)))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let add = v
            .and_then(|v| v.get("add"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().chars().take(80).collect::<String>())
                    .filter(|s| s.chars().count() >= 2)
                    .take(MAX_MANUAL)
                    .collect()
            })
            .unwrap_or_default();
        Decisions { off: pairs("off"), on: pairs("on"), add }
    }
}

/// One hit in the preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hit {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: Kind,
    pub label: String,
    /// The placeholder it becomes (empty while off).
    pub placeholder: String,
    pub ambiguous: bool,
    pub on: bool,
    /// Which occurrence of this text (0 = first), for switching it.
    pub n: usize,
}

/// A part of the outgoing text, for drawing the preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    /// Index into `hits`, when this part is a hit (replaced or kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<usize>,
}

/// What an attached file contributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileNote {
    pub name: String,
    pub replaced: usize,
    /// The text was checked; pictures and PDFs are not.
    pub checked: bool,
}

/// The preview before sending: exactly the text that leaves the device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    /// Fingerprint of the composer text it was made for.
    pub print: String,
    pub active: bool,
    pub outgoing: String,
    pub hits: Vec<Hit>,
    pub segments: Vec<Segment>,
    pub files: Vec<FileNote>,
    pub added: Vec<String>,
}

/// What the state holds about protection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PrivacyState {
    pub lists: Vec<ClassListInfo>,
    /// The last preview, for the composer. Holds names, so agents reading the state never see it.
    #[serde(skip)]
    pub preview: Option<Preview>,
    /// The downloadable name recognition (#73).
    #[serde(default)]
    pub ner: ner::NerState,
}

// ---- storage --------------------------------------------------------------------------------

fn load_lists(c: &Ctx) -> Vec<ClassList> {
    c.store(|st| st.setting(LISTS_KEY)).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

fn info(l: &ClassList) -> ClassListInfo {
    ClassListInfo { id: l.id.clone(), name: l.name.clone(), project_id: l.project_id.clone(), count: l.people.len() }
}

fn save_lists(c: &Ctx, lists: &[ClassList]) -> Result<()> {
    c.store(|st| st.set_setting(LISTS_KEY, &serde_json::to_value(lists)?))?;
    let infos: Vec<ClassListInfo> = lists.iter().map(info).collect();
    c.update(|s| s.privacy.lists = infos);
    Ok(())
}

pub(crate) fn refresh(c: &Ctx) {
    let infos: Vec<ClassListInfo> = load_lists(c).iter().map(info).collect();
    c.update(|s| s.privacy.lists = infos);
    ner::refresh(c);
}

fn map_key(conv: &str) -> String {
    format!("privacy.map.{conv}")
}

/// The mapping of a chat (empty for a new one).
pub(crate) fn mapping(c: &Ctx, conv: &str) -> Arc<Mapping> {
    if conv.is_empty() {
        return Arc::new(Mapping::default());
    }
    if let Some(m) = c.0.privacy_maps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(conv) {
        return m.clone();
    }
    let m: Mapping = c.store(|st| st.setting(&map_key(conv))).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    let m = Arc::new(m);
    c.0.privacy_maps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(conv.to_string(), m.clone());
    m
}

pub(crate) fn save_mapping(c: &Ctx, conv: &str, m: &Mapping) -> Result<()> {
    if conv.is_empty() {
        return Ok(());
    }
    // A temporary chat's mapping stays in memory (#34).
    if !c.store(|st| Ok(st.is_temporary(conv)))? {
        c.store(|st| st.set_setting(&map_key(conv), &serde_json::to_value(m)?))?;
    }
    let mut maps = c.0.privacy_maps.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // The sidebar shows up to 500 chats; keep the cache bounded.
    if maps.len() > 1000 {
        maps.clear();
    }
    maps.insert(conv.to_string(), Arc::new(m.clone()));
    Ok(())
}

/// A chat is gone: its mapping goes with it.
pub(crate) fn forget(c: &Ctx, conv: &str) {
    let _ = c.store(|st| st.delete_setting(&map_key(conv)));
    c.0.privacy_maps.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(conv);
}

/// The pupils for a project's chats: lists for all chats and the project's own.
fn people_for(c: &Ctx, project: Option<&str>) -> Vec<Person> {
    load_lists(c).into_iter().filter(|l| l.project_id.is_none() || l.project_id.as_deref() == project).flat_map(|l| l.people).collect()
}

fn has_list(c: &Ctx, project: Option<&str>) -> bool {
    c.state().privacy.lists.iter().any(|l| l.count > 0 && (l.project_id.is_none() || l.project_id.as_deref() == project))
}

/// Protection on for these sources: switched by the person, else on once a class list exists.
pub(crate) fn active(c: &Ctx, sources: &Sources, project: Option<&str>) -> bool {
    sources.protect.unwrap_or_else(|| has_list(c, project))
}

fn year() -> i32 {
    use chrono::Datelike;
    chrono::Local::now().year()
}

fn detector(c: &Ctx, project: Option<&str>, m: &Mapping, extra: &[String]) -> Detector {
    let manual: Vec<String> = m.manual.iter().chain(extra).cloned().collect();
    Detector::new(&people_for(c, project), &m.known(), &manual, year()).with_common_names().with_model(ner::model(c))
}

/// Takes out what protection would find, with every class list (all projects), whether
/// protection is on or not: for text derived from the owner's messages that must not keep names
/// (the writing style, #20). Findings become „…“; placeholders like `[S1]` go too.
pub(crate) fn strip_personal(c: &Ctx, text: &str) -> String {
    let people: Vec<Person> = load_lists(c).into_iter().flat_map(|l| l.people).collect();
    let d = Detector::new(&people, &[], &[], year());
    let text = text.get(..text.len().min(MAX_TEXT)).unwrap_or(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in d.detect(text) {
        if f.start < at {
            continue;
        }
        out.push_str(text.get(at..f.start).unwrap_or(""));
        out.push('…');
        at = f.end;
    }
    out.push_str(text.get(at..).unwrap_or(""));
    // Placeholders of protected chats name nobody, but they are not style either.
    let mut clean = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(open) = rest.find('[') {
        let (before, from) = rest.split_at(open);
        clean.push_str(before);
        match from.find(']') {
            Some(close) if close <= 16 => {
                clean.push('…');
                rest = from.get(close + 1..).unwrap_or("");
            }
            _ => {
                clean.push('[');
                rest = from.get(1..).unwrap_or("");
            }
        }
    }
    clean.push_str(rest);
    clean
}

// ---- protecting -----------------------------------------------------------------------------

/// Replace what `d` finds in `text`, as the person decided (`None`: automatic, everything found
/// is replaced, unsure hits too).
pub(crate) fn protect_text(d: &Detector, m: &mut Mapping, text: &str, decisions: Option<&Decisions>) -> (String, Vec<Hit>) {
    let text = text.get(..text.len().min(MAX_TEXT)).unwrap_or(text);
    let found = d.detect(text);
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut hits = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for f in found {
        let surface = text.get(f.start..f.end).unwrap_or("").to_string();
        let folded = surface.to_lowercase();
        let n = {
            let e = seen.entry(folded.clone()).or_insert(0);
            *e += 1;
            *e - 1
        };
        let on = match decisions {
            Some(dc) if dc.off.contains(&(folded.clone(), n)) => false,
            Some(dc) if dc.on.contains(&(folded.clone(), n)) => true,
            _ => true,
        };
        let placeholder = if on { m.placeholder_for(&f) } else { String::new() };
        out.push_str(text.get(at..f.start).unwrap_or(""));
        out.push_str(if on { &placeholder } else { &surface });
        at = f.end;
        hits.push(Hit {
            start: f.start,
            end: f.end,
            text: surface,
            kind: f.kind,
            label: f.kind.label().to_string(),
            placeholder,
            ambiguous: f.ambiguous,
            on,
            n,
        });
    }
    out.push_str(text.get(at..).unwrap_or(""));
    (out, hits)
}

fn segments(text: &str, hits: &[Hit]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut at = 0;
    for (i, h) in hits.iter().enumerate() {
        let plain = text.get(at..h.start).unwrap_or("");
        if !plain.is_empty() {
            out.push(Segment { text: plain.to_string(), hit: None });
        }
        out.push(Segment { text: if h.on { h.placeholder.clone() } else { h.text.clone() }, hit: Some(i) });
        at = h.end;
    }
    let rest = text.get(at..).unwrap_or("");
    if !rest.is_empty() {
        out.push(Segment { text: rest.to_string(), hit: None });
    }
    out
}

/// What a message sends after the preview: the text, the files' text, the mapping to keep.
pub(crate) struct Protected {
    pub text: String,
    pub hits: Vec<Hit>,
    pub mapping: Mapping,
}

impl Protected {
    pub fn replaced(&self) -> usize {
        self.hits.iter().filter(|h| h.on).count()
    }
}

/// Protection for a message being sent from the composer (`chat.send`): `None` when it is off.
pub(crate) fn for_send(c: &Ctx, text: &str, decisions: &Decisions) -> Option<(Protected, Detector)> {
    let s = c.state();
    let (sources, project, conv) = match s.current.as_ref().and_then(|id| s.conversation(id)) {
        Some(conv) => (conv.sources.clone(), conv.project_id.clone(), conv.id.clone()),
        None => (s.selection.sources.clone(), s.selection.project_id.clone(), String::new()),
    };
    if !active(c, &sources, project.as_deref()) {
        return None;
    }
    let mut m = (*mapping(c, &conv)).clone();
    for a in &decisions.add {
        if !m.manual.iter().any(|x| x.eq_ignore_ascii_case(a)) && m.manual.len() < MAX_MANUAL {
            m.manual.push(a.clone());
        }
    }
    let d = detector(c, project.as_deref(), &m, &[]);
    let (out, hits) = protect_text(&d, &mut m, text, Some(decisions));
    // Hits confirmed in the preview are known from now on; files and later messages use them.
    let d = detector(c, project.as_deref(), &m, &[]);
    Some((Protected { text: out, hits, mapping: m }, d))
}

/// Protects everything else that goes out for one chat (see the module docs).
pub(crate) struct Guard {
    conv: String,
    detector: Detector,
    pub mapping: Mapping,
    before: (usize, bool),
}

impl Guard {
    /// `None` when protection is off for this chat.
    pub fn for_conversation(c: &Ctx, conv: &aiscraft_core::Conversation) -> Option<Guard> {
        if !active(c, &conv.sources, conv.project_id.as_deref()) {
            return None;
        }
        let mapping = (*mapping(c, &conv.id)).clone();
        let detector = detector(c, conv.project_id.as_deref(), &mapping, &[]);
        let before = (mapping.entries.len(), mapping.noted);
        Some(Guard { conv: conv.id.clone(), detector, mapping, before })
    }

    /// For one model call outside a chat (`complete`): a mapping just for this call.
    pub fn for_call(c: &Ctx) -> Option<Guard> {
        let s = c.state();
        let project = s.selection.project_id.clone();
        if !active(c, &s.selection.sources, project.as_deref()) {
            return None;
        }
        let mapping = Mapping::default();
        let detector = detector(c, project.as_deref(), &mapping, &[]);
        Some(Guard { conv: String::new(), detector, mapping, before: (0, false) })
    }

    pub fn text(&mut self, text: &str) -> String {
        protect_text(&self.detector, &mut self.mapping, text, None).0
    }

    /// Every outgoing message of a request: reviewed user messages stay as sent.
    pub fn messages(&mut self, messages: &mut [aiscraft_core::Message]) {
        use aiscraft_core::Role;
        for m in messages.iter_mut() {
            let reviewed = m.role == Role::User && m.activity.iter().any(|s| s.label == PROTECTED);
            if m.role == Role::Assistant || reviewed {
                continue;
            }
            m.content = self.text(&m.content);
            for a in m.attachments.iter_mut() {
                if let Some(t) = a.text.take() {
                    a.text = Some(self.text(&t));
                }
            }
        }
    }

    /// The note that tells the model what the placeholders are, when there are any.
    pub fn note(&self) -> Option<&'static str> {
        (!self.mapping.entries.is_empty()).then_some(
            "Platzhalter in eckigen Klammern wie [S1], [N1] oder [E-Mail 1] stehen für geschützte Angaben (Namen, Kontaktdaten). Übernimm sie genau so in deine Antwort, ändere sie nicht und erfinde keine Namen dafür.",
        )
    }

    /// Keep new placeholders for the chat.
    pub fn finish(&self, c: &Ctx) {
        if (self.mapping.entries.len(), self.mapping.noted) != self.before {
            let _ = save_mapping(c, &self.conv, &self.mapping);
        }
    }
}

// ---- translating back -----------------------------------------------------------------------

/// Placeholders of chat `conv` in `text` back to the originals.
pub(crate) fn reveal(c: &Ctx, conv: &str, text: &str) -> String {
    mapping(c, conv).reveal(text)
}

/// Every string in a JSON value translated back (arguments of built-in tools).
pub(crate) fn reveal_value(c: &Ctx, conv: &str, v: &Value) -> Value {
    let m = mapping(c, conv);
    if m.entries.is_empty() {
        return v.clone();
    }
    fn walk(m: &Mapping, v: &Value, depth: usize) -> Value {
        match v {
            Value::String(s) => Value::String(m.reveal(s)),
            Value::Array(a) if depth < 32 => Value::Array(a.iter().map(|x| walk(m, x, depth + 1)).collect()),
            Value::Object(o) if depth < 32 => Value::Object(o.iter().map(|(k, x)| (k.clone(), walk(m, x, depth + 1))).collect()),
            other => other.clone(),
        }
    }
    walk(&m, v, 0)
}

/// The messages of the open chat as people read them: placeholders translated back (the last
/// answer while it streams without a cut-off placeholder at its end).
pub(crate) fn shown_messages(c: &Ctx, state: &crate::State) -> Vec<aiscraft_core::Message> {
    let Some(conv) = state.current.as_deref() else { return state.messages.clone() };
    let m = mapping(c, conv);
    if m.entries.is_empty() {
        return state.messages.clone();
    }
    let streaming = state.is_generating(conv);
    let last = state.messages.len().saturating_sub(1);
    state
        .messages
        .iter()
        .enumerate()
        .map(|(i, msg)| {
            let mut x = msg.clone();
            x.content = if streaming && i == last { m.reveal_streaming(&msg.content) } else { m.reveal(&msg.content) };
            for s in x.activity.iter_mut() {
                if let Some(doc) = s.document.as_mut().filter(|d| !["docx", "pptx", "xlsx", "pdf", "odt"].contains(&d.format.as_str())) {
                    doc.content = m.reveal(&doc.content);
                }
            }
            for cite in x.citations.iter_mut() {
                cite.cited_text = m.reveal(&cite.cited_text);
                cite.claim = m.reveal(&cite.claim);
            }
            x
        })
        .collect()
}

// ---- commands -------------------------------------------------------------------------------

fn project_param(c: &Ctx, p: &Value) -> Result<Option<String>> {
    let project = opt_str(p, "project").filter(|s| !s.is_empty());
    if let Some(pid) = &project
        && !c.state().projects.iter().any(|x| &x.id == pid)
    {
        return Err(Error::not_found("Projekt nicht gefunden"));
    }
    Ok(project)
}

fn add_list(c: &Ctx, name: String, project: Option<String>, people: Vec<Person>, how: &str) -> Result<Value> {
    if people.is_empty() {
        return Err(Error::invalid(
            "Keine Namen gefunden. Erwartet wird eine Liste mit Vor- und Nachnamen (eine Person pro Zeile oder Spalten Vorname, Nachname).",
        ));
    }
    let mut lists = load_lists(c);
    if lists.len() >= MAX_LISTS {
        return Err(Error::invalid(format!("Höchstens {MAX_LISTS} Klassenlisten")));
    }
    let name = if name.trim().is_empty() { "Klassenliste".to_string() } else { name.trim().chars().take(80).collect() };
    let list = ClassList { id: new_id(), name, project_id: project, people, created_at: now_ms() };
    let count = list.people.len();
    let out = json!({"id": list.id, "count": count, "how": how});
    lists.push(list);
    save_lists(c, &lists)?;
    c.notice(NoticeLevel::Success, format!("Klassenliste mit {count} Namen übernommen. Namen schützen ist ab jetzt in passenden Chats an."));
    Ok(out)
}

pub(crate) async fn list_import(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let project = project_param(&c, &p)?;
    let pb = std::path::PathBuf::from(&path);
    let ext = pb.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let meta = std::fs::metadata(&pb).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    if !meta.is_file() || meta.len() > 10 * 1024 * 1024 {
        return Err(Error::invalid("Das ist keine Datei oder sie ist größer als 10 MB"));
    }
    let bytes = std::fs::read(&pb).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    let (people, how) = tokio::task::spawn_blocking(move || -> Result<(Vec<Person>, String)> {
        Ok(match ext.as_str() {
            "vcf" | "vcard" => (detect::people_from_vcards(&bytes), "Kontakte".to_string()),
            "xlsx" | "xlsm" | "xls" | "ods" => {
                let grids = crate::attach::read_sheets(&ext, &bytes)?;
                let rows: Vec<Vec<String>> = grids
                    .first()
                    .map(|g| {
                        let mut rows: std::collections::BTreeMap<u32, Vec<String>> = std::collections::BTreeMap::new();
                        for ((r, col), v) in &g.cells {
                            let row = rows.entry(*r).or_default();
                            let col = usize::try_from(*col).unwrap_or(0).min(50);
                            if row.len() <= col {
                                row.resize(col + 1, String::new());
                            }
                            if let Some(cell) = row.get_mut(col) {
                                *cell = v.clone();
                            }
                        }
                        rows.into_values().take(detect::MAX_PEOPLE + 1).collect()
                    })
                    .unwrap_or_default();
                detect::people_from_rows(&rows)
            }
            "csv" | "tsv" | "txt" => {
                let text = craft_csv::decode_text(&bytes);
                let delim = if ext == "tsv" { '\t' } else { char::from(craft_csv::sniff_delimiter(&text, '"')) };
                let rows = craft_csv::records(&text, delim, '"', detect::MAX_PEOPLE + 1, 50);
                detect::people_from_rows(&rows)
            }
            other => {
                return Err(Error::invalid(format!(
                    ".{other}-Dateien gehen nicht. Möglich sind CSV, Excel (xlsx, xls), OpenDocument (ods) und Kontakte (vcf)."
                )));
            }
        })
    })
    .await
    .map_err(|e| Error::invalid(format!("Einlesen abgebrochen: {e}")))??;
    let name = opt_str(&p, "name").unwrap_or_else(|| pb.file_stem().and_then(|s| s.to_str()).unwrap_or("Klassenliste").to_string());
    add_list(&c, name, project, people, &how)
}

pub(crate) async fn list_add(c: Ctx, p: Value) -> Result<Value> {
    let text = req_str(&p, "text")?;
    let project = project_param(&c, &p)?;
    let people = detect::people_from_lines(text.get(..text.len().min(MAX_TEXT)).unwrap_or(""));
    add_list(&c, opt_str(&p, "name").unwrap_or_default(), project, people, "eine Person pro Zeile")
}

pub(crate) async fn list_remove(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut lists = load_lists(&c);
    let before = lists.len();
    lists.retain(|l| l.id != id);
    if lists.len() == before {
        return Err(Error::not_found("Klassenliste nicht gefunden"));
    }
    save_lists(&c, &lists)?;
    Ok(json!({}))
}

/// The lists with their names, for the owner (settings).
pub(crate) async fn lists(c: Ctx, _p: Value) -> Result<Value> {
    let out: Vec<Value> = load_lists(&c)
        .iter()
        .map(|l| json!({"id": l.id, "name": l.name, "project_id": l.project_id, "count": l.people.len(), "names": l.people.iter().map(Person::full).collect::<Vec<_>>()}))
        .collect();
    Ok(json!(out))
}

pub(crate) async fn preview(c: Ctx, p: Value) -> Result<Value> {
    let text = opt_str(&p, "text").unwrap_or_default();
    let decisions = Decisions::from_value(p.get("privacy").or(Some(&p)));
    let print = crate::chat::fingerprint(&text);
    let files: Vec<String> =
        p.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(20).collect()).unwrap_or_default();
    let preview = match for_send(&c, &text, &decisions) {
        None => Preview { print, active: false, outgoing: text.clone(), segments: vec![Segment { text, hit: None }], ..Default::default() },
        Some((prot, d)) => {
            // Nothing is kept: the mapping of the result is thrown away.
            let s = c.state();
            let mut notes = Vec::new();
            for f in &files {
                let prev = s.attachments.get(f);
                let name =
                    prev.map(|x| x.name.clone()).unwrap_or_else(|| std::path::Path::new(f).file_name().and_then(|n| n.to_str()).unwrap_or("Datei").to_string());
                let text_like = prev.is_some_and(|x| crate::files::is_text(&x.mime));
                // The chip's preview is the start of the text; enough to count what shows there.
                let replaced = match prev.filter(|_| text_like) {
                    Some(x) => protect_text(&d, &mut prot.mapping.clone(), &x.preview, None).1.iter().filter(|h| h.on).count(),
                    None => 0,
                };
                notes.push(FileNote { name, replaced, checked: text_like });
            }
            Preview {
                print,
                active: true,
                segments: segments(&text, &prot.hits),
                outgoing: prot.text,
                hits: prot.hits,
                files: notes,
                added: decisions.add.clone(),
            }
        }
    };
    let out = serde_json::to_value(&preview)?;
    c.update(|s| s.privacy.preview = Some(preview));
    Ok(out)
}

/// The placeholders of a chat, for the owner.
pub(crate) async fn show_mapping(c: Ctx, p: Value) -> Result<Value> {
    let conv = opt_str(&p, "conversation").or_else(|| c.state().current.clone()).ok_or_else(|| Error::invalid("Kein Chat gewählt"))?;
    if c.state().conversation(&conv).is_none() {
        return Err(Error::not_found("Chat nicht gefunden"));
    }
    let m = mapping(&c, &conv);
    Ok(
        json!({"entries": m.entries.iter().map(|e| json!({"placeholder": e.placeholder, "original": e.original, "kind": e.kind.label()})).collect::<Vec<_>>(), "manual": m.manual}),
    )
}

pub(crate) async fn reveal_cmd(c: Ctx, p: Value) -> Result<Value> {
    let text = req_str(&p, "text")?;
    let conv = opt_str(&p, "conversation").or_else(|| c.state().current.clone()).unwrap_or_default();
    Ok(json!({"text": reveal(&c, &conv, text.get(..text.len().min(MAX_TEXT)).unwrap_or(""))}))
}

/// The setup of „Namen schützen“ (#73) was gone through: switching protection on no longer
/// opens it. `done: false` brings it back.
pub(crate) async fn onboarding_done(c: Ctx, p: Value) -> Result<Value> {
    let done = match p.get("done") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(Error::invalid("done muss true oder false sein")),
    };
    let mut settings = c.state().settings.clone();
    settings.privacy_onboarded = done;
    c.store(|st| st.set_setting("settings", &serde_json::to_value(&settings)?))?;
    c.update(|s| s.settings.privacy_onboarded = done);
    Ok(json!({"done": done}))
}

impl crate::State {
    /// The switch of the open chat (or the next new one) as set: `None` is automatic (on once a
    /// class list fits).
    pub fn protect_choice(&self) -> Option<bool> {
        match self.current.as_ref().and_then(|id| self.conversation(id)) {
            Some(c) => c.sources.protect,
            None => self.selection.sources.protect,
        }
    }

    /// „Namen schützen“ is on for the open chat (or the next new one).
    pub fn protection_on(&self) -> bool {
        let (sources, project) = match self.current.as_ref().and_then(|id| self.conversation(id)) {
            Some(c) => (&c.sources, c.project_id.as_deref()),
            None => (&self.selection.sources, self.selection.project_id.as_deref()),
        };
        sources.protect.unwrap_or_else(|| self.privacy.lists.iter().any(|l| l.count > 0 && (l.project_id.is_none() || l.project_id.as_deref() == project)))
    }
}
