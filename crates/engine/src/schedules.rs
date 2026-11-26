//! Schedules, page watches and check-ins (as in n8n's Nod).
//!
//! A schedule sends its prompt on a cron rule (five fields, local time) or once at a time. The
//! answer goes into the schedule's own chat, or for a check-in the model planned itself
//! ("I'll look again on Friday") into the chat it came from. A watch only reports when the
//! answer changed. Schedules run without a person present: tools that need a yes are refused.
//! Like Nod, they only run while AIS.ChatCraft runs.

use std::time::Duration;

use aiscraft_core::{Conversation, Error, Result};
use chrono::{Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeAction, NoticeLevel, new_id, now_ms};

const KEY: &str = "schedules";
pub const NO_CHANGE: &str = "KEINE_ÄNDERUNG";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub title: String,
    pub prompt: String,
    /// Five-field cron rule in local time, e.g. "0 7 * * 1-5".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    /// Run once at this time (ms since epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_at: Option<i64>,
    pub backend_id: String,
    pub model_id: String,
    /// Where answers go; created on the first run when empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Only report when the answer differs from the last one.
    #[serde(default)]
    pub watch: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
    pub created_at: i64,
}

fn yes() -> bool {
    true
}

// ---- cron -----------------------------------------------------------------------------------

/// One cron field as the set of allowed values.
fn field(spec: &str, min: u32, max: u32) -> Result<Vec<u32>> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| Error::invalid(format!("Cron: Schritt {s:?}")))?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(Error::invalid("Cron: Schritt 0"));
        }
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().map_err(|_| Error::invalid(format!("Cron: {a:?}")))?, b.parse().map_err(|_| Error::invalid(format!("Cron: {b:?}")))?)
        } else {
            let v: u32 = range.parse().map_err(|_| Error::invalid(format!("Cron: {range:?}")))?;
            (v, if part.contains('/') { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(Error::invalid(format!("Cron: {part} liegt außerhalb von {min}–{max}")));
        }
        out.extend((lo..=hi).step_by(step as usize));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days: Vec<u32>,
    months: Vec<u32>,
    weekdays: Vec<u32>,
    any_day: bool,
    any_weekday: bool,
}

impl Cron {
    pub fn parse(rule: &str) -> Result<Self> {
        let f: Vec<&str> = rule.split_whitespace().collect();
        let [m, h, d, mo, w] = f.as_slice() else { return Err(Error::invalid("Cron braucht fünf Felder: Minute Stunde Tag Monat Wochentag")) };
        let mut weekdays = field(w, 0, 7)?;
        for x in weekdays.iter_mut() {
            if *x == 7 {
                *x = 0; // Sunday is 0 and 7
            }
        }
        Ok(Self {
            minutes: field(m, 0, 59)?,
            hours: field(h, 0, 23)?,
            days: field(d, 1, 31)?,
            months: field(mo, 1, 12)?,
            weekdays,
            any_day: *d == "*",
            any_weekday: *w == "*",
        })
    }

    /// Does the rule fire in this minute? Day of month and weekday combine with OR when both are
    /// restricted, as in classic cron.
    pub fn matches<T: Datelike + Timelike>(&self, t: &T) -> bool {
        self.minutes.contains(&t.minute()) && self.hours.contains(&t.hour()) && self.on_day(t)
    }

    /// Does the rule fire on this day (month, day of month, weekday)?
    fn on_day<T: Datelike>(&self, t: &T) -> bool {
        let dom = self.days.contains(&t.day());
        let dow = self.weekdays.contains(&t.weekday().num_days_from_sunday());
        let day = match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (false, true) => dom,
            (true, false) => dow,
            (false, false) => dom || dow,
        };
        self.months.contains(&t.month()) && day
    }

    /// The next time after `after` (ms) the rule fires, in local time; `None` when it never does
    /// within [`HORIZON_DAYS`] (a 30 February).
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        self.next_after_in(&Local, after_ms)
    }

    /// [`Cron::next_after`] in a given time zone. Walks day by day, not minute by minute, so a
    /// rare rule (29 February, once a year) costs a few thousand cheap date checks, not millions
    /// of time conversions; the UI asks for it every frame (Nod #1315).
    ///
    /// Clock changes: a time the clock skips (02:30 on the spring-forward night) runs at the
    /// first minute after the gap, once; a time that happens twice (autumn) runs the first time
    /// only. So a daily schedule runs exactly once on both nights.
    pub fn next_after_in<Tz: TimeZone>(&self, tz: &Tz, after_ms: i64) -> Option<i64> {
        let mut day = tz.timestamp_millis_opt(after_ms).single()?.date_naive();
        for _ in 0..=HORIZON_DAYS {
            if self.on_day(&day) {
                let first = self
                    .hours
                    .iter()
                    .flat_map(|h| self.minutes.iter().map(move |m| (*h, *m)))
                    .filter_map(|(h, m)| local_run(tz, day, h, m))
                    .filter(|t| *t > after_ms)
                    .min();
                if first.is_some() {
                    return first;
                }
            }
            day = day.succ_opt()?;
        }
        None
    }
}

/// How far ahead a rule is looked at: eight years, so 29 February always comes round.
pub const HORIZON_DAYS: u32 = 8 * 366;

/// The instant of `h:m` local time on `day`: the earlier one when the clock goes back, the first
/// minute after the gap when the clock skips it.
fn local_run<Tz: TimeZone>(tz: &Tz, day: chrono::NaiveDate, h: u32, m: u32) -> Option<i64> {
    let naive = day.and_hms_opt(h, m, 0)?;
    if let Some(t) = tz.from_local_datetime(&naive).earliest() {
        return Some(t.timestamp_millis());
    }
    // In the gap: the first local minute that exists (gaps are at most a few hours).
    (1..=24 * 60).find_map(|k| tz.from_local_datetime(&(naive + chrono::Duration::minutes(k))).earliest()).map(|t| t.timestamp_millis())
}

// ---- storage --------------------------------------------------------------------------------

pub(crate) fn load(c: &Ctx) -> Vec<Schedule> {
    c.store(|st| Ok(st.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

fn save(c: &Ctx, all: &[Schedule]) -> Result<()> {
    c.store(|st| st.set_setting(KEY, &serde_json::to_value(all)?))?;
    let all = all.to_vec();
    c.update(|s| s.schedules = all);
    Ok(())
}

/// When a schedule fires next, for display.
pub fn next_run(s: &Schedule, now: i64) -> Option<i64> {
    if !s.enabled {
        return None;
    }
    if let Some(at) = s.run_at {
        return s.last_run.is_none().then_some(at);
    }
    Cron::parse(s.cron.as_deref()?).ok()?.next_after(s.last_run.unwrap_or(now).max(now - 60_000))
}

// ---- running --------------------------------------------------------------------------------

/// Start the clock: every 20 s, run what is due.
pub(crate) fn start(c: &Ctx) {
    let ctx = c.clone();
    c.spawn(async move {
        let mut ticks: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            ticks = ticks.wrapping_add(1);
            // Hourly: tidy up the notes (duplicates are archived, never deleted).
            if ticks.is_multiple_of(180) {
                let _ = crate::skills::lint_memory(&ctx);
                // And empty the trash of chats deleted more than 30 days ago (#77).
                crate::archive::purge_expired(&ctx);
            }
            // A minute after the start and hourly: schedule ideas from recurring requests (#68),
            // found on the device; announced only with the hints below.
            if ticks % 180 == 3 {
                let _ = crate::schedule_ideas::refresh(&ctx);
            }
            // Hourly too, half an hour later: maybe one hint.
            if ticks % 180 == 90 {
                let _ = crate::proactive::check(&ctx, false).await;
            }
            let now = now_ms();
            for s in load(&ctx).into_iter().filter(|s| s.enabled) {
                if due(&s, now) {
                    let ctx2 = ctx.clone();
                    ctx.spawn(async move {
                        let _ = run(&ctx2, s).await;
                    });
                }
            }
        }
    });
}

/// Due now? A cron rule fires once per run time (within the last minute, so a time the clock
/// skips or repeats runs once too); a one-off once at or after its time.
pub fn due(s: &Schedule, now: i64) -> bool {
    due_in(&Local, s, now)
}

fn due_in<Tz: TimeZone>(tz: &Tz, s: &Schedule, now: i64) -> bool {
    if let Some(at) = s.run_at {
        return s.last_run.is_none() && now >= at;
    }
    let Some(rule) = s.cron.as_deref().and_then(|r| Cron::parse(r).ok()) else { return false };
    // Runs since the last run, but at most one minute back: missed runs are not made up.
    let since = s.last_run.unwrap_or(i64::MIN).max(now - 60_000);
    rule.next_after_in(tz, since).is_some_and(|t| t <= now)
}

/// A new rule must fire some day: „30. Februar“ is refused when it is set, never when an old
/// schedule is only paused or renamed (Nod #1514).
fn check_rule(rule: &str) -> Result<()> {
    if Cron::parse(rule)?.next_after(now_ms()).is_none() {
        return Err(Error::invalid(format!("Cron: {rule} trifft nie zu (gibt es diesen Tag, z. B. 30. Februar?)")));
    }
    Ok(())
}

async fn run(c: &Ctx, mut s: Schedule) -> Result<Value> {
    // Mark first, so a slow answer is not started twice.
    s.last_run = Some(now_ms());
    if s.run_at.is_some() {
        s.enabled = false;
    }
    update_one(c, &s)?;

    let conv_id = match s.conversation_id.clone().filter(|id| c.state().conversation(id).is_some()) {
        Some(id) => id,
        None => {
            let now = now_ms();
            let conv = Conversation {
                id: new_id(),
                title: format!("⏰ {}", s.title),
                backend_id: s.backend_id.clone(),
                model_id: s.model_id.clone(),
                project_id: None,
                created_at: now,
                updated_at: now,
                pinned: false,
                sources: Default::default(),
                persona: None,
                kind: Default::default(),
                remote_id: None,
                temporary: false,
                archived: false,
                deleted_at: None,
            };
            c.store(|st| st.save_conversation(&conv))?;
            let id = conv.id.clone();
            c.update(|st| st.conversations.insert(0, conv));
            s.conversation_id = Some(id.clone());
            update_one(c, &s)?;
            id
        }
    };
    // In the status line while it runs (#69).
    let _running = crate::tasks::Running::start(c, &conv_id, if s.watch { crate::JobKind::Watch } else { crate::JobKind::Schedule }, &s.title);
    let mut prompt = format!("(Zeitplan „{}“) {}", s.title, s.prompt);
    if s.watch {
        let last = s.last_result.clone().unwrap_or_else(|| "(noch keine)".into());
        prompt.push_str(&format!(
            "\n\nDas ist eine Beobachtung. Wenn sich gegenüber der letzten Beobachtung nichts Wesentliches geändert hat, antworte nur mit {NO_CHANGE}.\nLetzte Beobachtung:\n{last}"
        ));
    }
    let out = crate::chat::send_background(c, &conv_id, &prompt).await;
    let text = match &out {
        Ok(v) => v.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        Err(e) => format!("Fehler: {}", e.message),
    };
    let unchanged = s.watch && text.trim() == NO_CHANGE;
    if unchanged {
        // Nothing new: remove the run from the chat again so it stays readable.
        if let Ok(v) = &out
            && let Some(mid) = v.get("user_message_id").and_then(Value::as_str)
        {
            let _ = c.store(|st| st.truncate_from(&conv_id, mid));
            let mid = mid.to_string();
            c.update(|st| {
                if st.current.as_deref() == Some(&conv_id)
                    && let Some(p) = st.messages.iter().position(|m| m.id == mid)
                {
                    st.messages.truncate(p);
                }
            });
        }
    } else {
        s.last_result = Some(text.chars().take(4000).collect());
        update_one(c, &s)?;
        let summary: String = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(140).collect();
        crate::inbox::notify(
            c,
            if out.is_ok() { NoticeLevel::Info } else { NoticeLevel::Error },
            &s.title,
            &summary,
            Some(NoticeAction { label: "Öffnen".into(), command: "chat.open".into(), params: json!({"id": conv_id}) }),
        );
    }
    out
}

fn update_one(c: &Ctx, s: &Schedule) -> Result<()> {
    let mut all = load(c);
    match all.iter_mut().find(|x| x.id == s.id) {
        Some(x) => *x = s.clone(),
        None => all.push(s.clone()),
    }
    save(c, &all)
}

/// A desktop notification, best effort (D-Bus on Linux, the system's on macOS and Windows).
pub(crate) fn desktop_notify(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new().appname("AIS.ChatCraft").summary(&title).body(&body).timeout(notify_rust::Timeout::Milliseconds(8000)).show();
    });
}

// ---- when -----------------------------------------------------------------------------------

/// Parse "in 30 min", "in 2 Tagen", "morgen 7:30", "2026-12-04 08:00" or an ISO date.
pub fn parse_when(s: &str, now: i64) -> Option<i64> {
    let t = s.trim().to_lowercase();
    if let Some(rest) = t.strip_prefix("in ") {
        let mut parts = rest.split_whitespace();
        let n: i64 = parts.next()?.parse().ok()?;
        let unit = parts.next().unwrap_or("min");
        let mins = if unit.starts_with("min") {
            n
        } else if unit.starts_with("std") || unit.starts_with("h") {
            n * 60
        } else if unit.starts_with("tag") || unit.starts_with("day") || unit == "d" {
            n * 1440
        } else if unit.starts_with("woche") || unit.starts_with("week") {
            n * 10080
        } else {
            return None;
        };
        return now.checked_add(mins.checked_mul(60_000)?);
    }
    let base = Local.timestamp_millis_opt(now).single()?;
    let (day, time) = if let Some(rest) = t.strip_prefix("morgen") {
        (base.date_naive().succ_opt()?, rest.trim())
    } else if let Some(rest) = t.strip_prefix("heute") {
        (base.date_naive(), rest.trim())
    } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s.trim()) {
        return Some(dt.timestamp_millis());
    } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M") {
        return Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp_millis());
    } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%dT%H:%M") {
        return Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp_millis());
    } else {
        return None;
    };
    let time = time.trim_start_matches("um").trim().trim_end_matches("uhr").trim();
    let (h, m) = if time.is_empty() {
        (8, 0)
    } else {
        time.split_once(':')
            .map(|(a, b)| (a.parse().ok(), b.parse().ok()))
            .map(|(a, b)| (a.unwrap_or(99), b.unwrap_or(99)))
            .unwrap_or((time.parse().unwrap_or(99), 0))
    };
    let dt = day.and_hms_opt(h, m, 0)?;
    Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp_millis())
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) async fn create(c: Ctx, p: Value) -> Result<Value> {
    let title = req_str(&p, "title")?.trim().chars().take(120).collect::<String>();
    let prompt = req_str(&p, "prompt")?;
    if title.is_empty() || prompt.trim().is_empty() {
        return Err(Error::invalid("Titel und Auftrag dürfen nicht leer sein"));
    }
    let now = now_ms();
    let cron = opt_str(&p, "cron").filter(|x| !x.trim().is_empty());
    let run_at = match opt_str(&p, "at").filter(|x| !x.trim().is_empty()) {
        Some(w) => Some(
            parse_when(&w, now).ok_or_else(|| Error::invalid(format!("Zeit nicht verstanden: {w} (z. B. „in 2 Tagen“, „morgen 7:30“, „2026-12-04 08:00“)")))?,
        ),
        None => None,
    };
    match (&cron, run_at) {
        (Some(r), None) => {
            check_rule(r)?;
        }
        (None, Some(at)) if at <= now - 60_000 => return Err(Error::invalid("Dieser Zeitpunkt liegt in der Vergangenheit")),
        (None, Some(_)) => {}
        _ => return Err(Error::invalid("Entweder cron (wiederkehrend) oder at (einmalig) angeben")),
    }
    let sel = c.state().selection.clone();
    let s = Schedule {
        id: new_id(),
        title,
        prompt,
        cron,
        run_at,
        backend_id: opt_str(&p, "backend").unwrap_or(sel.backend_id),
        model_id: opt_str(&p, "model").unwrap_or(sel.model_id),
        conversation_id: opt_str(&p, "conversation"),
        watch: p.get("watch").and_then(Value::as_bool).unwrap_or(false),
        enabled: true,
        last_run: None,
        last_result: None,
        created_at: now,
    };
    if c.state().backend(&s.backend_id).is_none() {
        return Err(Error::not_found("Backend nicht gefunden"));
    }
    let id = s.id.clone();
    let next = next_run(&s, now);
    update_one(&c, &s)?;
    Ok(json!({"id": id, "next_run": next}))
}

pub(crate) async fn edit(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut s = load(&c).into_iter().find(|x| x.id == id).ok_or_else(|| Error::not_found("Zeitplan nicht gefunden"))?;
    if let Some(b) = p.get("enabled").and_then(Value::as_bool) {
        s.enabled = b;
    }
    if let Some(t) = opt_str(&p, "title").filter(|t| !t.trim().is_empty()) {
        s.title = t;
    }
    if let Some(t) = opt_str(&p, "prompt").filter(|t| !t.trim().is_empty()) {
        s.prompt = t;
    }
    if let Some(r) = opt_str(&p, "cron").filter(|r| !r.trim().is_empty()) {
        check_rule(&r)?;
        s.cron = Some(r);
        s.run_at = None;
    }
    // The model per schedule (#68); the chat it writes to follows on the next run.
    if let Some(b) = opt_str(&p, "backend").filter(|b| !b.is_empty()) {
        if c.state().backend(&b).is_none() {
            return Err(Error::not_found("Backend nicht gefunden"));
        }
        s.backend_id = b;
    }
    if let Some(m) = opt_str(&p, "model").filter(|m| !m.trim().is_empty()) {
        s.model_id = m.chars().take(200).collect();
    }
    if (p.get("backend").is_some() || p.get("model").is_some())
        && let Some(conv) = s.conversation_id.clone()
    {
        let (b, m) = (s.backend_id.clone(), s.model_id.clone());
        let _ = crate::commands::edit_conversation(&c, &conv, |x| {
            x.backend_id = b;
            x.model_id = m;
        });
    }
    update_one(&c, &s)?;
    Ok(json!({"next_run": next_run(&s, now_ms())}))
}

pub(crate) async fn delete(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut all = load(&c);
    let before = all.len();
    all.retain(|x| x.id != id);
    if all.len() == before {
        return Err(Error::not_found("Zeitplan nicht gefunden"));
    }
    save(&c, &all)?;
    Ok(json!({}))
}

pub(crate) async fn run_now(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let s = load(&c).into_iter().find(|x| x.id == id).ok_or_else(|| Error::not_found("Zeitplan nicht gefunden"))?;
    run(&c, s).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(y, mo, d, h, mi, 0).single().unwrap()
    }

    /// Berlin in 2026, independent of the machine's zone: summer time from 29 March 01:00 UTC
    /// to 25 October 01:00 UTC.
    #[derive(Debug, Clone, Copy)]
    struct Berlin;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        utc(y, mo, d, h, mi).and_utc().timestamp_millis()
    }

    impl TimeZone for Berlin {
        type Offset = chrono::FixedOffset;
        fn from_offset(_: &chrono::FixedOffset) -> Self {
            Berlin
        }
        fn offset_from_local_date(&self, d: &chrono::NaiveDate) -> chrono::LocalResult<chrono::FixedOffset> {
            self.offset_from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &chrono::NaiveDateTime) -> chrono::LocalResult<chrono::FixedOffset> {
            let fits: Vec<chrono::FixedOffset> = [2, 1]
                .into_iter()
                .map(|h| chrono::FixedOffset::east_opt(h * 3600).unwrap())
                .filter(|o| self.offset_from_utc_datetime(&(*local - chrono::Duration::seconds(o.local_minus_utc().into()))) == *o)
                .collect();
            match fits.as_slice() {
                [one] => chrono::LocalResult::Single(*one),
                [a, b] => chrono::LocalResult::Ambiguous(*a, *b),
                _ => chrono::LocalResult::None,
            }
        }
        fn offset_from_utc_date(&self, d: &chrono::NaiveDate) -> chrono::FixedOffset {
            self.offset_from_utc_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, t: &chrono::NaiveDateTime) -> chrono::FixedOffset {
            let summer = *t >= utc(2026, 3, 29, 1, 0) && *t < utc(2026, 10, 25, 1, 0);
            chrono::FixedOffset::east_opt(if summer { 7200 } else { 3600 }).unwrap()
        }
    }

    /// Nod #1315: monthly and rare rules. The 31st skips short months, 29 February comes round
    /// in a leap year, 30 February never; looking that far costs days, not minutes.
    #[test]
    fn monthly_and_rare_rules() {
        let on_31 = Cron::parse("0 8 31 * *").unwrap();
        // 31 Jan 09:00 Berlin (08:00 UTC): next is 31 March 08:00 Berlin, summer time (06:00 UTC).
        assert_eq!(on_31.next_after_in(&Berlin, ms(2026, 1, 31, 8, 0)), Some(ms(2026, 3, 31, 6, 0)));
        assert_eq!(on_31.next_after_in(&Berlin, ms(2026, 4, 1, 0, 0)), Some(ms(2026, 5, 31, 6, 0)), "not in April");
        let leap = Cron::parse("0 0 29 2 *").unwrap();
        assert_eq!(leap.next_after_in(&Berlin, ms(2026, 3, 1, 0, 0)), Some(ms(2028, 2, 28, 23, 0)));
        assert_eq!(leap.next_after(at(2026, 3, 1, 0, 0).timestamp_millis()), Some(at(2028, 2, 29, 0, 0).timestamp_millis()));
        let yearly = Cron::parse("0 7 1 9 *").unwrap();
        assert_eq!(yearly.next_after_in(&Berlin, ms(2026, 9, 1, 6, 0)), Some(ms(2027, 9, 1, 6, 0)));
        let never = Cron::parse("0 9 30 2 *").unwrap();
        assert_eq!(never.next_after(now_ms()), None);
        assert!(check_rule("0 9 30 2 *").is_err() && check_rule("0 0 29 2 *").is_ok() && check_rule("0 8 31 * *").is_ok());
        // Every first Monday-or-1st (cron's OR) and the last days of the month by list.
        let ends = Cron::parse("0 18 28-31 * *").unwrap();
        assert_eq!(ends.next_after_in(&Berlin, ms(2026, 2, 27, 0, 0)), Some(ms(2026, 2, 28, 17, 0)));
        assert_eq!(ends.next_after_in(&Berlin, ms(2026, 2, 28, 18, 0)), Some(ms(2026, 3, 28, 17, 0)));
    }

    /// Clock changes: 02:30 on the spring night (skipped) runs once at 03:00; on the autumn night
    /// (twice) it runs the first time only.
    #[test]
    fn clock_changes_run_once() {
        let rule = Cron::parse("30 2 * * *").unwrap();
        // Spring: 29 March, 02:00–03:00 does not exist; 03:00 local is 01:00 UTC.
        assert_eq!(rule.next_after_in(&Berlin, ms(2026, 3, 28, 12, 0)), Some(ms(2026, 3, 29, 1, 0)));
        assert_eq!(rule.next_after_in(&Berlin, ms(2026, 3, 29, 1, 0)), Some(ms(2026, 3, 30, 0, 30)));
        // Autumn: 25 October, 02:30 happens at 00:30 and 01:30 UTC; only the first counts.
        assert_eq!(rule.next_after_in(&Berlin, ms(2026, 10, 24, 12, 0)), Some(ms(2026, 10, 25, 0, 30)));
        assert_eq!(rule.next_after_in(&Berlin, ms(2026, 10, 25, 0, 30)), Some(ms(2026, 10, 26, 1, 30)));
        // The daily run in the skipped hour is due once after the gap, and not again in the repeated hour.
        let s = Schedule {
            id: "s".into(),
            title: "t".into(),
            prompt: "p".into(),
            cron: Some("30 2 * * *".into()),
            run_at: None,
            backend_id: "b".into(),
            model_id: "m".into(),
            conversation_id: None,
            watch: false,
            enabled: true,
            last_run: Some(ms(2026, 3, 28, 1, 30)),
            last_result: None,
            created_at: 0,
        };
        assert!(due_in(&Berlin, &s, ms(2026, 3, 29, 1, 0) + 10_000), "the skipped 02:30 runs at 03:00");
        let ran = Schedule { last_run: Some(ms(2026, 10, 25, 0, 30) + 5_000), ..s };
        assert!(due_in(&Berlin, &Schedule { last_run: Some(ms(2026, 10, 24, 0, 30)), ..ran.clone() }, ms(2026, 10, 25, 0, 30) + 5_000));
        assert!(!due_in(&Berlin, &ran, ms(2026, 10, 25, 1, 30) + 5_000), "02:30 again an hour later does not run again");
    }

    #[test]
    fn cron_rules() {
        let weekdays_7 = Cron::parse("0 7 * * 1-5").unwrap();
        assert!(weekdays_7.matches(&at(2026, 11, 23, 7, 0))); // Monday
        assert!(!weekdays_7.matches(&at(2026, 11, 22, 7, 0))); // Sunday
        assert!(!weekdays_7.matches(&at(2026, 11, 23, 7, 1)));
        let every_15 = Cron::parse("*/15 * * * *").unwrap();
        assert!(every_15.matches(&at(2026, 1, 1, 3, 45)) && !every_15.matches(&at(2026, 1, 1, 3, 46)));
        let sunday = Cron::parse("30 18 * * 7").unwrap();
        assert!(sunday.matches(&at(2026, 11, 22, 18, 30)));
        // Day of month OR weekday when both are given.
        let either = Cron::parse("0 9 1 * 1").unwrap();
        assert!(either.matches(&at(2026, 12, 1, 9, 0)) && either.matches(&at(2026, 11, 23, 9, 0)));
        let next = weekdays_7.next_after(at(2026, 11, 21, 12, 0).timestamp_millis()).unwrap();
        assert_eq!(next, at(2026, 11, 23, 7, 0).timestamp_millis());
        for bad in ["", "* * * *", "61 * * * *", "* * * * 8", "*/0 * * * *", "a b c d e", "5-1 * * * *"] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn due_once_per_minute() {
        let s = Schedule {
            id: "s".into(),
            title: "t".into(),
            prompt: "p".into(),
            cron: Some("0 7 * * *".into()),
            run_at: None,
            backend_id: "b".into(),
            model_id: "m".into(),
            conversation_id: None,
            watch: false,
            enabled: true,
            last_run: None,
            last_result: None,
            created_at: 0,
        };
        let seven = at(2026, 11, 23, 7, 0).timestamp_millis() + 5_000;
        assert!(due(&s, seven));
        let ran = Schedule { last_run: Some(seven), ..s.clone() };
        assert!(!due(&ran, seven + 20_000), "not twice in the same minute");
        assert!(due(&Schedule { last_run: Some(seven - 86_400_000), ..s.clone() }, seven));
        let once = Schedule { cron: None, run_at: Some(seven), ..s };
        assert!(!due(&once, seven - 1) && due(&once, seven) && !due(&Schedule { last_run: Some(seven), ..once }, seven + 1));
    }

    #[test]
    fn when_phrases() {
        let now = at(2026, 11, 23, 12, 0).timestamp_millis();
        assert_eq!(parse_when("in 30 min", now), Some(now + 30 * 60_000));
        assert_eq!(parse_when("in 2 Tagen", now), Some(now + 2 * 86_400_000));
        assert_eq!(parse_when("morgen 7:30", now), Some(at(2026, 11, 24, 7, 30).timestamp_millis()));
        assert_eq!(parse_when("morgen", now), Some(at(2026, 11, 24, 8, 0).timestamp_millis()));
        assert_eq!(parse_when("2026-12-04 08:00", now), Some(at(2026, 12, 4, 8, 0).timestamp_millis()));
        assert_eq!(parse_when("heute um 25:00", now), None);
        assert_eq!(parse_when("irgendwann", now), None);
        assert_eq!(parse_when("in 99999999999999 Wochen", now), None);
    }
}
