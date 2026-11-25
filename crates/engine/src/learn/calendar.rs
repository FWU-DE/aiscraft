//! Dates from an answer as a calendar file (#51), an output format of the registry (#56).
//!
//! An answer brings dates as a ```` ```ics ```` block (asked for by `calendar.request`) or as a
//! Markdown table with a date column. Both are read leniently into [`Event`]s; the `.ics` file
//! is always written by us: the events (`VEVENT`) are ours, content lines, folding, escaping and
//! the time zone (`VTIMEZONE` for Europe/Berlin through `write_vtimezone`) come from craft-ical.
//! So a model's broken folding, missing `UID`/`DTSTAMP` or wrong escaping never reaches the
//! calendar program.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use craft_ical::values::{escape_text, format_date, format_local, format_utc, unescape_text};
use craft_ical::{ContentLine, Lines, Transition, When, ZoneRules, fold, write_vtimezone};
use serde::Serialize;
use serde_json::{Value, json};

use aiscraft_core::{Attachment, Error, Result, ToolSpec};

use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel, now_ms};

/// First words of a calendar request; the chat folds what follows the topic away.
pub const CAL_INTRO: &str = "Erstelle Kalendertermine zum Thema: ";
/// The zone every written time is in.
pub const TZID: &str = "Europe/Berlin";
pub const MAX_EVENTS: usize = 200;
const MAX_SRC: usize = 512 * 1024;
const MAX_TEXT: usize = 2000;

/// One date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    pub title: String,
    /// Wall-clock time in Europe/Berlin; midnight for a whole day.
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub all_day: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub location: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

impl Event {
    /// „Do., 03.12.2026, 19:00–20:30“ for cards and notices.
    pub fn when_label(&self) -> String {
        let day = format!("{}, {}", weekday_de(self.start.weekday()), self.start.format("%d.%m.%Y"));
        if self.all_day {
            return day;
        }
        match self.end {
            Some(e) if e.date() == self.start.date() => format!("{day}, {}–{}", self.start.format("%H:%M"), e.format("%H:%M")),
            _ => format!("{day}, {}", self.start.format("%H:%M")),
        }
    }
}

fn weekday_de(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "Mo.",
        Weekday::Tue => "Di.",
        Weekday::Wed => "Mi.",
        Weekday::Thu => "Do.",
        Weekday::Fri => "Fr.",
        Weekday::Sat => "Sa.",
        Weekday::Sun => "So.",
    }
}

fn cap(s: &str) -> String {
    let t: String = s.trim().chars().filter(|c| !c.is_control() || *c == '\n').take(MAX_TEXT).collect();
    t
}

// ---- Europe/Berlin -----------------------------------------------------------------------------

/// Central European Time with the EU's summer time (last Sunday of March to the last Sunday of
/// October, 01:00 UTC). Enough for school dates; years before 1996 are written with today's rule.
struct Berlin;

fn last_sunday(year: i32, month: u32) -> Option<NaiveDate> {
    let first_next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) }?;
    let last = first_next.pred_opt()?;
    Some(last - Duration::days(i64::from(last.weekday().num_days_from_sunday())))
}

fn summer_bounds(year: i32) -> Option<(i64, i64)> {
    let at = |d: NaiveDate| d.and_hms_opt(1, 0, 0).map(|t| t.and_utc().timestamp());
    Some((at(last_sunday(year, 3)?)?, at(last_sunday(year, 10)?)?))
}

impl ZoneRules for Berlin {
    fn offset_at(&self, instant: i64) -> i32 {
        let year = chrono::DateTime::from_timestamp(instant, 0).map(|d| d.year()).unwrap_or(2000);
        match summer_bounds(year) {
            Some((a, b)) if instant >= a && instant < b => 7200,
            _ => 3600,
        }
    }

    fn transitions(&self, from: i64, to: i64) -> Vec<Transition> {
        let year = |t: i64| chrono::DateTime::from_timestamp(t, 0).map(|d| d.year()).unwrap_or(2000);
        let mut out = Vec::new();
        for y in year(from)..=year(to).min(year(from).saturating_add(200)) {
            let Some((a, b)) = summer_bounds(y) else { continue };
            for t in [Transition { at: a, before: 3600, after: 7200 }, Transition { at: b, before: 7200, after: 3600 }] {
                if t.at >= from && t.at < to {
                    out.push(t);
                }
            }
        }
        out
    }
}

// ---- reading -----------------------------------------------------------------------------------

/// The ```` ```ics ```` blocks of an answer.
pub fn ics_blocks(answer: &str) -> Vec<&str> {
    super::fenced_blocks(answer)
        .into_iter()
        .filter(|(l, b)| matches!(l.as_str(), "ics" | "ical" | "icalendar" | "vcalendar" | "aiscraft-termine") || b.trim_start().starts_with("BEGIN:VCALENDAR"))
        .map(|(_, b)| b)
        .collect()
}

pub(crate) fn description() -> String {
    format!(
        "Format iCalendar in einem ```ics-Codeblock: BEGIN:VCALENDAR, je Termin ein BEGIN:VEVENT mit SUMMARY (Titel), DTSTART und DTEND (Uhrzeit: DTSTART;TZID={TZID}:20261203T190000, ganzer Tag: DTSTART;VALUE=DATE:20261203), optional LOCATION und DESCRIPTION, END:VEVENT, END:VCALENDAR. Nur Termine mit festem Datum, höchstens {MAX_EVENTS}.\nBeispiel:\nBEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Elternabend 7b\nDTSTART;TZID={TZID}:20261203T190000\nDTEND;TZID={TZID}:20261203T203000\nLOCATION:Raum 112\nEND:VEVENT\nEND:VCALENDAR"
    )
}

/// The request for dates as a calendar file.
pub fn calendar_prompt(topic: &str, tools: bool) -> String {
    let deliver = if tools {
        "Lege die Kalenderdatei mit dem Werkzeug calendar_file_create an und gib die Termine danach kurz als Liste aus."
    } else {
        "Antworte mit genau einem ```ics-Codeblock und danach einer kurzen Liste der Termine."
    };
    format!("{CAL_INTRO}{topic}\n\nTermine mit Datum, Uhrzeit (wenn sinnvoll) und kurzem Titel, Deutsch. Wenn ein Datum unklar ist, lass den Termin weg und sag es.\n\n{}\n\n{deliver}", description())
}

/// Reads the events of an iCalendar text: the problems found (skipped events) next to them.
pub fn parse_ics(src: &str) -> std::result::Result<(Vec<Event>, Vec<String>), String> {
    if src.len() > MAX_SRC {
        return Err("zu groß (höchstens 512 KB)".into());
    }
    let mut events = Vec::new();
    let mut problems = Vec::new();
    let mut cur: Option<Vec<ContentLine>> = None;
    let mut n = 0;
    for raw in Lines::new(src) {
        let Some(line) = ContentLine::parse(raw.trim_start()) else { continue };
        match (line.name.as_str(), line.value.trim().to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => cur = Some(Vec::new()),
            ("END", "VEVENT") => {
                n += 1;
                if let Some(props) = cur.take() {
                    match event_of(&props) {
                        Ok(e) if events.len() < MAX_EVENTS => events.push(e),
                        Ok(_) => {
                            problems.push(format!("nur die ersten {MAX_EVENTS} Termine übernommen"));
                            break;
                        }
                        Err(e) => problems.push(format!("Termin {n}: {e}")),
                    }
                }
            }
            _ => {
                if let Some(p) = cur.as_mut()
                    && p.len() < 200
                {
                    p.push(line);
                }
            }
        }
    }
    if events.is_empty() {
        return Err(problems.first().cloned().unwrap_or_else(|| "keine Termine (BEGIN:VEVENT) gefunden".into()));
    }
    problems.dedup();
    Ok((events, problems))
}

fn event_of(props: &[ContentLine]) -> std::result::Result<Event, String> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    let text = |name: &str| get(name).map(|p| cap(&unescape_text(&p.value))).unwrap_or_default();
    let when = |name: &str| {
        get(name).and_then(|p| {
            let w = When::parse(&p.value)?;
            Some(match w {
                // UTC times are shown in Berlin.
                When::Utc(_) => (w.local(&|ts| Berlin.offset_at(ts) / 60), false),
                When::Date(_) => (w.wall(), true),
                When::Local(t) => (t, false),
            })
        })
    };
    let title = text("SUMMARY");
    if title.is_empty() {
        return Err("ohne Titel (SUMMARY)".into());
    }
    let (start, all_day) = when("DTSTART").ok_or("ohne gültiges Datum (DTSTART)")?;
    let end = when("DTEND").map(|(e, _)| e).or_else(|| {
        get("DURATION").and_then(|d| craft_ical::values::parse_duration(&d.value)).map(|s| start + Duration::seconds(s.clamp(0, 366 * 86_400)))
    });
    let end = end.filter(|e| *e > start);
    Ok(Event { title, start, end, all_day, location: text("LOCATION"), description: text("DESCRIPTION") })
}

/// A date as Germans write it: `3.12.2026`, `03.12.26`, `2026-12-03`, `3. Dezember 2026`,
/// optionally after a weekday (`Do., 3.12.2026`).
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim().trim_matches(|c: char| c == '*' || c == '_');
    // Weekday in front.
    let s = match s.find(|c: char| c.is_ascii_digit()) {
        Some(i) => s.get(i..)?,
        None => return None,
    };
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s.get(..s.len().min(10))?, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s.split(|c: char| c == '.' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    let day: u32 = parts.first()?.parse().ok()?;
    let month: u32 = match parts.get(1)?.parse() {
        Ok(m) => m,
        Err(_) => month_de(parts.get(1)?)?,
    };
    let year: i32 = match parts.get(2).map(|y| y.trim_end_matches(',')).and_then(|y| y.parse::<i32>().ok()) {
        Some(y) if y < 100 => 2000 + y,
        Some(y) => y,
        None => return None,
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

fn month_de(s: &str) -> Option<u32> {
    let s = s.to_lowercase();
    let names = ["jan", "feb", "mär", "apr", "mai", "jun", "jul", "aug", "sep", "okt", "nov", "dez"];
    let s = s.replace("maerz", "mär").replace("marz", "mär");
    names.iter().position(|n| s.starts_with(n)).map(|i| i as u32 + 1)
}

/// `19:00`, `19 Uhr`, `19.30 Uhr`, `8:15–9:00` (start and end).
pub fn parse_times(s: &str) -> (Option<NaiveTime>, Option<NaiveTime>) {
    let one = |t: &str| {
        let t = t.trim().trim_end_matches("Uhr").trim();
        let (h, m) = match t.split_once([':', '.']) {
            Some((h, m)) => (h.trim().parse::<u32>().ok()?, m.trim().get(..2).unwrap_or(m.trim()).parse::<u32>().ok()?),
            None => (t.parse::<u32>().ok()?, 0),
        };
        NaiveTime::from_hms_opt(h, m, 0)
    };
    let mut parts = s.splitn(2, ['–', '-', '—']).map(str::to_string).collect::<Vec<_>>();
    if parts.len() < 2 {
        parts = s.splitn(2, " bis ").map(str::to_string).collect();
    }
    (parts.first().and_then(|p| one(p)), parts.get(1).and_then(|p| one(p)))
}

/// The rows of Markdown tables with a date column, as events: the date column (Datum, Tag,
/// Termin, Wann), a time column (Uhrzeit, Zeit, Beginn), a title (Titel, Was, Thema, Anlass,
/// Veranstaltung, Inhalt, Ereignis, else the first other column), a place (Ort, Raum).
pub fn table_events(answer: &str) -> Vec<Event> {
    let mut out = Vec::new();
    let lines: Vec<&str> = answer.lines().take(20_000).collect();
    let cells = |l: &str| -> Vec<String> { l.trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect() };
    let mut i = 0;
    while i + 1 < lines.len() && out.len() < MAX_EVENTS {
        let (Some(head), Some(sep)) = (lines.get(i), lines.get(i + 1)) else { break };
        let is_sep = sep.contains('-') && sep.trim().chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
        if !head.trim_start().starts_with('|') || !is_sep {
            i += 1;
            continue;
        }
        let h: Vec<String> = cells(head).iter().map(|c| c.to_lowercase().replace(['*', '_'], "")).collect();
        let find = |names: &[&str]| h.iter().position(|c| names.iter().any(|n| c.starts_with(n)));
        let date = find(&["datum", "tag", "termin", "wann", "date"]);
        let time = find(&["uhrzeit", "zeit", "beginn", "von", "time"]);
        let title = find(&["titel", "was", "thema", "anlass", "veranstaltung", "inhalt", "ereignis", "beschreibung", "aktivität"]);
        let place = find(&["ort", "raum", "wo"]);
        i += 2;
        let Some(dc) = date else { continue };
        while let Some(l) = lines.get(i).filter(|l| l.trim_start().starts_with('|')) {
            i += 1;
            let row = cells(l);
            let Some(d) = row.get(dc).and_then(|c| parse_date(c)) else { continue };
            let title = title
                .and_then(|t| row.get(t))
                .or_else(|| row.iter().enumerate().find(|(k, c)| Some(*k) != Some(dc) && Some(*k) != time && Some(*k) != place && !c.is_empty()).map(|(_, c)| c))
                .map(|t| cap(&t.replace("**", "")))
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            // A time may stand in the date cell too („3.12.2026, 19 Uhr“).
            let tcell = time.and_then(|t| row.get(t)).cloned().or_else(|| row.get(dc).and_then(|c| c.split_once(',').map(|(_, t)| t.to_string())));
            let (from, to) = tcell.as_deref().map(parse_times).unwrap_or((None, None));
            let start = d.and_time(from.unwrap_or(NaiveTime::MIN));
            out.push(Event {
                title,
                start,
                end: to.map(|t| d.and_time(t)).filter(|e| *e > start),
                all_day: from.is_none(),
                location: place.and_then(|p| row.get(p)).map(|p| cap(p)).unwrap_or_default(),
                description: String::new(),
            });
            if out.len() >= MAX_EVENTS {
                break;
            }
        }
    }
    out
}

/// The events of an answer: its ```` ```ics ```` blocks, else its date tables.
pub fn events_in(answer: &str) -> (Vec<Event>, Vec<String>) {
    let mut events = Vec::new();
    let mut problems = Vec::new();
    for b in ics_blocks(answer) {
        match parse_ics(b) {
            Ok((e, p)) => {
                events.extend(e);
                problems.extend(p);
            }
            Err(e) => problems.push(e),
        }
    }
    if events.is_empty() {
        events = table_events(answer);
    }
    events.truncate(MAX_EVENTS);
    (events, problems)
}

pub(crate) fn check(src: &str) -> std::result::Result<Vec<String>, String> {
    parse_ics(src).map(|(_, p)| p)
}

// ---- writing -----------------------------------------------------------------------------------

/// A complete `.ics` file: one `VEVENT` per event, times in Europe/Berlin with its `VTIMEZONE`.
/// `stamp`: ms since 1970 for `DTSTAMP`; `note`: the AI label (#57), written as each event's
/// `COMMENT` and the calendar's description.
pub fn write_ics(events: &[Event], title: &str, stamp: i64, note: Option<&str>) -> String {
    let mut out = String::new();
    fold(&mut out, "BEGIN:VCALENDAR");
    fold(&mut out, "VERSION:2.0");
    fold(&mut out, "PRODID:-//AIS.ChatCraft//Termine//DE");
    fold(&mut out, "CALSCALE:GREGORIAN");
    fold(&mut out, "METHOD:PUBLISH");
    if !title.trim().is_empty() {
        fold(&mut out, &format!("X-WR-CALNAME:{}", escape_text(&cap(title))));
    }
    if let Some(n) = note {
        fold(&mut out, &format!("X-WR-CALDESC:{}", escape_text(n)));
    }
    fold(&mut out, &format!("X-WR-TIMEZONE:{TZID}"));
    let timed: Vec<&Event> = events.iter().filter(|e| !e.all_day).collect();
    if let (Some(a), Some(b)) = (timed.iter().map(|e| e.start.year()).min(), timed.iter().map(|e| e.end.unwrap_or(e.start).year()).max()) {
        write_vtimezone(&mut out, TZID, &Berlin, a, b);
    }
    let dtstamp = format_utc(stamp / 1000);
    for (i, e) in events.iter().take(MAX_EVENTS).enumerate() {
        fold(&mut out, "BEGIN:VEVENT");
        // Stable for the same event, so importing twice doesn't double it.
        let uid = format!("{:x}-{i}@aiscraft", fnv(&format!("{}|{}", e.title, e.start)));
        fold(&mut out, &format!("UID:{uid}"));
        fold(&mut out, &format!("DTSTAMP:{dtstamp}"));
        if e.all_day {
            let end = e.end.map(|x| x.date()).filter(|d| *d > e.start.date()).unwrap_or_else(|| e.start.date() + Duration::days(1));
            fold(&mut out, &format!("DTSTART;VALUE=DATE:{}", format_date(e.start.date())));
            fold(&mut out, &format!("DTEND;VALUE=DATE:{}", format_date(end)));
        } else {
            let end = e.end.unwrap_or(e.start + Duration::hours(1));
            fold(&mut out, &format!("DTSTART;TZID={TZID}:{}", format_local(e.start)));
            fold(&mut out, &format!("DTEND;TZID={TZID}:{}", format_local(end)));
        }
        fold(&mut out, &format!("SUMMARY:{}", escape_text(&e.title)));
        if !e.location.is_empty() {
            fold(&mut out, &format!("LOCATION:{}", escape_text(&e.location)));
        }
        if !e.description.is_empty() {
            fold(&mut out, &format!("DESCRIPTION:{}", escape_text(&e.description)));
        }
        if let Some(n) = note {
            fold(&mut out, &format!("COMMENT:{}", escape_text(n)));
        }
        fold(&mut out, "END:VEVENT");
    }
    fold(&mut out, "END:VCALENDAR");
    out
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// Events from a tool's arguments: `[{title, start: "2026-12-03T19:00" | "2026-12-03", end?,
/// location?, description?}]`.
fn events_of_value(v: &Value) -> std::result::Result<(Vec<Event>, Vec<String>), String> {
    let list = v.get("events").and_then(Value::as_array).ok_or("events fehlt (eine Liste von Terminen)")?;
    let mut events = Vec::new();
    let mut problems = Vec::new();
    for (i, e) in list.iter().take(MAX_EVENTS).enumerate() {
        let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let title = cap(&s("title"));
        let start = When::parse(&s("start"));
        match (title.is_empty(), start) {
            (false, Some(w)) => {
                let all_day = matches!(w, When::Date(_));
                let start = w.local(&|ts| Berlin.offset_at(ts) / 60);
                let end = When::parse(&s("end")).map(|x| x.local(&|ts| Berlin.offset_at(ts) / 60)).filter(|x| *x > start);
                events.push(Event { title, start, end, all_day, location: cap(&s("location")), description: cap(&s("description")) });
            }
            _ => problems.push(format!("Termin {}: Titel oder Datum fehlt", i + 1)),
        }
    }
    if events.is_empty() {
        return Err(problems.first().cloned().unwrap_or_else(|| "keine Termine".into()));
    }
    Ok((events, problems))
}

// ---- commands ----------------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "calendar.request",
        label: "Termine als Kalenderdatei erstellen lassen",
        group: "Kalender",
        shortcut: None,
        params: "topic: string, subject?: string, grade?: string",
        palette: false,
        run: |c, p| Box::pin(request(c, p)),
    },
    CommandSpec {
        id: "calendar.validate",
        label: "Termine prüfen",
        group: "Kalender",
        shortcut: None,
        params: "content?: string (ics-Block oder eine Antwort mit Datumstabelle), from_message?: string",
        palette: false,
        run: |c, p| Box::pin(validate(c, p)),
    },
    CommandSpec {
        id: "calendar.save",
        label: "Kalenderdatei speichern",
        group: "Kalender",
        shortcut: None,
        params: "path: string, content?: string, from_message?: string, title?: string",
        palette: false,
        run: |c, p| Box::pin(save(c, p)),
    },
];

fn source(c: &Ctx, p: &Value) -> Result<(String, Option<String>)> {
    match opt_str(p, "from_message") {
        Some(mid) => Ok((super::message_text(c, &mid)?, Some(mid))),
        None => Ok((req_str(p, "content")?, None)),
    }
}

async fn validate(c: Ctx, p: Value) -> Result<Value> {
    let (src, _) = source(&c, &p)?;
    let (events, problems) = events_in(&src);
    Ok(json!({"ok": !events.is_empty() && problems.is_empty(), "events": events, "problems": problems}))
}

async fn save(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let (src, mid) = source(&c, &p)?;
    // A bare iCalendar text (not inside a block) counts as one.
    let (events, problems) = match events_in(&src) {
        (e, p) if !e.is_empty() => (e, p),
        _ => parse_ics(&src).map_err(|e| Error::invalid(format!("Keine Termine gefunden: {e}")))?,
    };
    let title = opt_str(&p, "title").unwrap_or_else(|| "Termine".into());
    let note = crate::ai_label::note_for_message(&c, mid.as_deref());
    let ics = write_ics(&events, &title, now_ms(), note.as_ref().map(|n| n.line.as_str()));
    crate::disk::write(std::path::Path::new(&path), ics.as_bytes())?;
    c.notice(NoticeLevel::Success, format!("{} Termine gespeichert: {path}", events.len()));
    Ok(json!({"path": path, "events": events.len(), "problems": problems}))
}

async fn request(c: Ctx, p: Value) -> Result<Value> {
    let topic = super::topic_line(&p)?;
    let tools = super::tools_on(&c);
    crate::chat::send_with(c.clone(), calendar_prompt(&topic, tools), vec![], vec![], None, false).await?;
    let answer = super::last_answer(&c);
    let (events, problems) = events_in(&answer);
    if events.is_empty() && !tools {
        c.notice(NoticeLevel::Warning, "Die Antwort enthält keine Termine. Bitte das Modell, sie als ```ics-Block auszugeben.");
    }
    Ok(json!({"events": events.len(), "problems": problems}))
}

pub(crate) fn tool_specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "calendar_file_create".into(),
        description: format!(
            "Erstellt eine Kalenderdatei (.ics) als Download im Chat, z. B. Klassenarbeitstermine oder einen Elternabend; Zeiten gelten in {TZID}. Höchstens {MAX_EVENTS} Termine."
        ),
        input_schema: json!({"type": "object", "properties": {
            "title": {"type": "string", "description": "Name des Kalenders und der Datei"},
            "events": {"type": "array", "items": {"type": "object", "properties": {
                "title": {"type": "string"},
                "start": {"type": "string", "description": "2026-12-03T19:00 oder für einen ganzen Tag 2026-12-03"},
                "end": {"type": "string"},
                "location": {"type": "string"},
                "description": {"type": "string"}
            }, "required": ["title", "start"]}}
        }, "required": ["title", "events"], "additionalProperties": false}),
    }]
}

/// Runs `calendar_file_create`: the file goes to the attachments folder and shows in the chat.
pub(crate) fn run_tool(c: &Ctx, name: &str, a: &Value) -> Option<Result<(String, Attachment)>> {
    if name != "calendar_file_create" {
        return None;
    }
    Some((|| {
        let (events, problems) = events_of_value(a).map_err(Error::invalid)?;
        let title = a.get("title").and_then(Value::as_str).unwrap_or("Termine").to_string();
        let note = crate::ai_label::note_for_message(c, None);
        let ics = write_ics(&events, &title, now_ms(), note.as_ref().map(|n| n.line.as_str()));
        let name = crate::office::file_name(&title, "ics");
        let path = crate::files::keep_copy(&c.0.attachments_dir, std::path::Path::new(&name), ics.as_bytes())?;
        let att = Attachment { mime: "text/calendar".into(), size: ics.len() as u64, local_path: Some(path.display().to_string()), name, ..Attachment::default() };
        let mut msg = format!("Kalenderdatei „{}“ mit {} Terminen erstellt. Sie steht im Chat zum Speichern bereit; nenne keinen Speicherort.", att.name, events.len());
        if !problems.is_empty() {
            msg.push_str(&format!(" Übersprungen: {}", problems.join("; ")));
        }
        Ok((msg, att))
    })())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Elternabend 7b\\, mit Wahl\nDTSTART;TZID=Europe/Berlin:20261203T190000\nDTEND;TZID=Europe/Berlin:20261203T203000\nLOCATION:Raum 112\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:Wandertag\nDTSTART;VALUE=DATE:20270615\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20270101\nEND:VEVENT\nBEGIN:VEVENT\nSUMMARY:UTC\nDTSTART:20270701T080000Z\nEND:VEVENT\nEND:VCALENDAR";

    #[test]
    fn ics_round_trip_with_timezone() {
        let (events, problems) = parse_ics(ICS).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(events[0].title, "Elternabend 7b, mit Wahl");
        assert_eq!(events[0].when_label(), "Do., 03.12.2026, 19:00–20:30");
        assert!(events[1].all_day);
        assert_eq!(events[2].start.format("%H:%M").to_string(), "10:00", "UTC in summer is 10:00 in Berlin");
        let out = write_ics(&events, "Termine 7b", 1_700_000_000_000, Some("Erstellt mit KI"));
        assert!(out.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin") && out.contains("DAYLIGHT") && out.contains("STANDARD"), "{out}");
        assert!(out.contains("DTSTART;TZID=Europe/Berlin:20261203T190000") && out.contains("DTEND;VALUE=DATE:20270616"));
        assert!(out.contains("SUMMARY:Elternabend 7b\\, mit Wahl") && out.contains("UID:") && out.contains("DTSTAMP:"));
        assert!(out.lines().all(|l| l.len() <= 76), "folded");
        assert!(out.contains("COMMENT:Erstellt mit KI"));
        let back = parse_ics(&out).unwrap().0;
        assert_eq!(back, events);
    }

    #[test]
    fn tables_with_dates_become_events() {
        let md = "Die Termine:\n\n| Datum | Uhrzeit | Was | Ort |\n|---|---|---|---|\n| Do., 3.12.2026 | 19–20:30 Uhr | **Elternabend** | Aula |\n| 15. Juni 2027 | | Wandertag | |\n| bald | 8:00 | unklar | |\n\nFertig.";
        let ev = table_events(md);
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].title.as_str(), ev[0].location.as_str()), ("Elternabend", "Aula"));
        assert_eq!(ev[0].when_label(), "Do., 03.12.2026, 19:00–20:30");
        assert!(ev[1].all_day && ev[1].start.date() == NaiveDate::from_ymd_opt(2027, 6, 15).unwrap());
        assert!(table_events("| Name | Note |\n|---|---|\n| Ada | 1 |").is_empty());
        assert_eq!(parse_date("2026-12-03"), NaiveDate::from_ymd_opt(2026, 12, 3));
        assert_eq!(parse_date("03.12.26"), NaiveDate::from_ymd_opt(2026, 12, 3));
        assert_eq!(parse_date("31.02.2026"), None);
        assert_eq!(events_in(&format!("```ics\n{ICS}\n```\n{md}")).0.len(), 3, "an ics block wins over a table");
    }

    #[test]
    fn hostile_calendar_input() {
        for s in ["", "BEGIN:VEVENT", "END:VEVENT", "BEGIN:VEVENT\nDTSTART:9999\nSUMMARY:x\nEND:VEVENT", ":::\n;;;", "BEGIN:VEVENT\nSUMMARY:x\nDTSTART:20261301\nEND:VEVENT"] {
            let _ = parse_ics(s);
            let _ = table_events(s);
        }
        assert!(parse_ics(&"A".repeat(MAX_SRC + 1)).is_err());
        let many: String = (0..300).map(|i| format!("BEGIN:VEVENT\nSUMMARY:T{i}\nDTSTART:20270101\nEND:VEVENT\n")).collect();
        let (e, p) = parse_ics(&many).unwrap();
        assert_eq!(e.len(), MAX_EVENTS);
        assert!(!p.is_empty());
        let _ = write_ics(&e, &"x".repeat(5000), 0, None);
        assert!(events_of_value(&json!({"events": [{"title": "", "start": "x"}]})).is_err());
        let (e, _) = events_of_value(&json!({"events": [{"title": "Konferenz", "start": "2026-12-03T14:00", "end": "2026-12-03T15:30"}]})).unwrap();
        assert_eq!(e[0].when_label(), "Do., 03.12.2026, 14:00–15:30");
    }
}
