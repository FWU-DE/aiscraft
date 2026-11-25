//! Schedule ideas from the owner's recurring requests (#68, Nod 0.9.4 "Personal schedule ideas
//! from your own messages"): when the owner asks for the same thing again and again on the same
//! weekday or at the same hour („Fass mir die Mails der Schulleitung zusammen“, three Mondays in a
//! row), AIS.ChatCraft suggests a schedule with that request, the rhythm and the model.
//!
//! - Found on the device, without a model, from the owner's own messages only: never answers,
//!   tool results or prompts the app wrote (schedules, tasks).
//! - Shown under Einstellungen › Zeitpläne; the hourly hints (`proactive.rs`, off by default,
//!   with quiet hours) also announce a new idea once.
//! - A declined idea never comes back, and neither does one already turned into a schedule.

use std::collections::{BTreeMap, BTreeSet};

use aiscraft_core::{Error, Result};
use aiscraft_store::OwnMessage;
use chrono::{Datelike, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeAction, NoticeLevel, now_ms};

/// Ideas the owner declined or took (ids), so they do not come back.
const DONE_KEY: &str = "schedule_ideas_done";
/// Ideas a hint announced already.
const ANNOUNCED_KEY: &str = "schedule_ideas_announced";
/// How far back requests count.
const WINDOW_MS: i64 = 10 * 7 * 86_400_000;
/// The same request on at least this many different days.
pub(crate) const MIN_DAYS: usize = 3;
/// At most this many ideas at once.
const MAX_IDEAS: usize = 5;
const MAX_KEPT: usize = 500;

/// A suggested schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleIdea {
    /// Stable for the same request, so a declined idea stays declined.
    pub id: String,
    pub title: String,
    /// The request, in the owner's latest words.
    pub prompt: String,
    /// Five-field cron rule, local time.
    pub cron: String,
    /// The rhythm as people read it: „montags um 8 Uhr“.
    pub rhythm: String,
    pub backend_id: String,
    pub model_id: String,
    /// On how many days the owner asked.
    pub days: usize,
}

const STOP: [&str; 34] = [
    "aber", "alle", "auch", "auf", "aus", "bei", "bitte", "das", "dass", "dem", "den", "der", "die", "ein", "eine", "einen", "für", "heute", "ich", "ist",
    "mal", "mir", "mit", "noch", "und", "von", "was", "wie", "zum", "zur", "kannst", "du", "mich", "uns",
];

fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()) && !w.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// Two requests are the same request: most of their words agree.
fn similar(a: &BTreeSet<String>, b: &BTreeSet<String>) -> bool {
    let shared = a.intersection(b).count();
    let all = a.union(b).count();
    shared >= 2 && shared * 2 >= all
}

const WEEKDAYS: [&str; 7] = ["sonntags", "montags", "dienstags", "mittwochs", "donnerstags", "freitags", "samstags"];

fn fingerprint(w: &BTreeSet<String>) -> String {
    crate::chat::fingerprint(&w.iter().cloned().collect::<Vec<_>>().join(" "))
}

/// The ideas in these messages (newest first), as of `now`.
pub fn find(msgs: &[OwnMessage], now: i64) -> Vec<ScheduleIdea> {
    // Group the owner's requests: each joins the first group whose first request it matches.
    let mut groups: Vec<(BTreeSet<String>, Vec<&OwnMessage>)> = Vec::new();
    for m in msgs.iter().filter(|m| !m.automatic && m.created_at >= now - WINDOW_MS && m.created_at <= now) {
        let w = words(&m.content);
        if w.len() < 2 {
            continue;
        }
        match groups.iter_mut().find(|(first, _)| similar(first, &w)) {
            Some((_, g)) => g.push(m),
            None => groups.push((w, vec![m])),
        }
    }
    let mut out = Vec::new();
    for (w, group) in groups {
        let local: Vec<(chrono::DateTime<Local>, &OwnMessage)> =
            group.iter().filter_map(|m| Local.timestamp_millis_opt(m.created_at).single().map(|t| (t, *m))).collect();
        let days = |set: &[&(chrono::DateTime<Local>, &OwnMessage)]| set.iter().map(|(t, _)| t.date_naive()).collect::<BTreeSet<_>>().len();
        // Same weekday on three different days: weekly.
        let mut by_weekday: BTreeMap<u32, Vec<&(chrono::DateTime<Local>, &OwnMessage)>> = BTreeMap::new();
        for x in &local {
            by_weekday.entry(x.0.weekday().num_days_from_sunday()).or_default().push(x);
        }
        let weekly = by_weekday.iter().filter(|(_, v)| days(v) >= MIN_DAYS).max_by_key(|(_, v)| days(v));
        let (cron, rhythm, picked) = if let Some((wd, v)) = weekly {
            let hour = typical_hour(v);
            (format!("0 {hour} * * {wd}"), format!("{} um {hour} Uhr", WEEKDAYS.get(*wd as usize).copied().unwrap_or("wöchentlich")), v.clone())
        } else {
            // Same hour (give or take one) on three different days: daily, or on workdays.
            let mut by_hour: BTreeMap<u32, Vec<&(chrono::DateTime<Local>, &OwnMessage)>> = BTreeMap::new();
            for x in &local {
                by_hour.entry(x.0.hour()).or_default().push(x);
            }
            let around = |h: u32| local.iter().filter(|(t, _)| t.hour().abs_diff(h) <= 1).collect::<Vec<_>>();
            let Some(v) = by_hour.keys().map(|h| around(*h)).filter(|v| days(v) >= MIN_DAYS).max_by_key(|v| days(v)) else { continue };
            let hour = typical_hour(&v);
            let workdays = v.iter().all(|(t, _)| t.weekday().num_days_from_monday() < 5);
            if workdays {
                (format!("0 {hour} * * 1-5"), format!("werktags um {hour} Uhr"), v)
            } else {
                (format!("0 {hour} * * *"), format!("täglich um {hour} Uhr"), v)
            }
        };
        let Some((_, latest)) = picked.iter().max_by_key(|(t, _)| *t) else { continue };
        let prompt: String = latest.content.trim().chars().take(2000).collect();
        let title_words: Vec<&str> = prompt.split_whitespace().take(6).collect();
        let mut title = title_words.join(" ");
        if prompt.split_whitespace().count() > 6 {
            title.push_str(" …");
        }
        out.push(ScheduleIdea {
            id: fingerprint(&w),
            title,
            prompt,
            cron,
            rhythm,
            backend_id: latest.backend_id.clone(),
            model_id: latest.model_id.clone(),
            days: days(&picked),
        });
    }
    out.sort_by(|a, b| b.days.cmp(&a.days));
    out
}

/// The hour most of these requests came in (the earliest on a tie), so the schedule is ready by then.
fn typical_hour(v: &[&(chrono::DateTime<Local>, &OwnMessage)]) -> u32 {
    let mut count: BTreeMap<u32, usize> = BTreeMap::new();
    for (t, _) in v {
        *count.entry(t.hour()).or_default() += 1;
    }
    let best = count.values().copied().max().unwrap_or(0);
    count.into_iter().find(|(_, n)| *n == best).map(|(h, _)| h).unwrap_or(8)
}

fn load_ids(c: &Ctx, key: &str) -> Vec<String> {
    c.store(|st| Ok(st.setting(key)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

fn add_id(c: &Ctx, key: &str, id: &str) -> Result<()> {
    let mut ids = load_ids(c, key);
    if !ids.iter().any(|x| x == id) {
        ids.push(id.to_string());
    }
    let extra = ids.len().saturating_sub(MAX_KEPT);
    ids.drain(..extra);
    c.store(|st| st.set_setting(key, &json!(ids)))
}

/// Look again: the ideas that are neither declined, taken, nor covered by a schedule already.
pub(crate) fn refresh(c: &Ctx) -> Result<Vec<ScheduleIdea>> {
    let msgs = c.store(|st| st.own_messages(now_ms() - WINDOW_MS, 2000))?;
    let done = load_ids(c, DONE_KEY);
    let schedules = c.state().schedules.clone();
    let ideas: Vec<ScheduleIdea> = find(&msgs, now_ms())
        .into_iter()
        .filter(|i| !done.contains(&i.id))
        .filter(|i| !schedules.iter().any(|s| similar(&words(&s.prompt), &words(&i.prompt))))
        .filter(|i| c.state().backend(&i.backend_id).is_some())
        .take(MAX_IDEAS)
        .collect();
    let shown = ideas.clone();
    c.update(|s| s.schedule_ideas = shown);
    Ok(ideas)
}

/// For the hourly hints: announce one new idea once. Returns its title.
pub(crate) fn announce(c: &Ctx) -> Result<Option<String>> {
    let ideas = refresh(c)?;
    let announced = load_ids(c, ANNOUNCED_KEY);
    let Some(idea) = ideas.into_iter().find(|i| !announced.contains(&i.id)) else { return Ok(None) };
    add_id(c, ANNOUNCED_KEY, &idea.id)?;
    crate::inbox::notify(
        c,
        NoticeLevel::Info,
        "Zeitplan-Idee",
        &format!("Du bittest {} um „{}“. Soll das als Zeitplan laufen?", idea.rhythm, idea.title),
        Some(NoticeAction { label: "Zeitplan anlegen".into(), command: "schedule.idea_accept".into(), params: json!({"id": idea.id}) }),
    );
    Ok(Some(idea.title))
}

pub(crate) async fn list(c: Ctx, _p: Value) -> Result<Value> {
    Ok(serde_json::to_value(refresh(&c)?)?)
}

/// Turn an idea into a schedule, with the owner's changes (title, prompt, cron, backend, model).
pub(crate) async fn accept(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let idea = match c.state().schedule_ideas.iter().find(|i| i.id == id).cloned() {
        Some(i) => i,
        None => refresh(&c)?.into_iter().find(|i| i.id == id).ok_or_else(|| Error::not_found("Diese Zeitplan-Idee gibt es nicht (mehr)"))?,
    };
    let pick = |k: &str, d: &str| opt_str(&p, k).filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
    let out = crate::schedules::create(
        c.clone(),
        json!({
            "title": pick("title", &idea.title),
            "prompt": pick("prompt", &idea.prompt),
            "cron": pick("cron", &idea.cron),
            "backend": pick("backend", &idea.backend_id),
            "model": pick("model", &idea.model_id),
        }),
    )
    .await?;
    add_id(&c, DONE_KEY, &id)?;
    c.update(|s| s.schedule_ideas.retain(|i| i.id != id));
    Ok(out)
}

/// „Nein danke“: the idea never comes back.
pub(crate) async fn dismiss(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    if id.trim().is_empty() || id.len() > 64 {
        return Err(Error::invalid("id fehlt"));
    }
    add_id(&c, DONE_KEY, &id)?;
    c.update(|s| s.schedule_ideas.retain(|i| i.id != id));
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(text: &str, t: chrono::DateTime<Local>) -> OwnMessage {
        OwnMessage {
            conversation_id: "c".into(),
            backend_id: "b".into(),
            model_id: "m".into(),
            content: text.into(),
            created_at: t.timestamp_millis(),
            automatic: false,
        }
    }

    fn at(d: u32, h: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, d, h, 10, 0).single().unwrap()
    }

    #[test]
    fn weekly_daily_and_nothing() {
        // Mondays 2, 9, 16 November 2026.
        let mondays = [
            msg("Fass mir die Mails der Schulleitung zusammen", at(16, 8)),
            msg("Bitte fass die Mails der Schulleitung zusammen", at(9, 8)),
            msg("Fass mir die Mails der Schulleitung zusammen", at(2, 9)),
        ];
        let now = at(20, 12).timestamp_millis();
        let ideas = find(&mondays, now);
        assert_eq!(ideas.len(), 1);
        assert_eq!(ideas[0].cron, "0 8 * * 1");
        assert_eq!(ideas[0].rhythm, "montags um 8 Uhr");
        assert_eq!(ideas[0].prompt, "Fass mir die Mails der Schulleitung zusammen");
        // Twice is not a habit.
        assert!(find(&mondays[..2], now).is_empty());
        // Different requests do not add up.
        let mixed = [
            msg("Fass mir die Mails der Schulleitung zusammen", at(16, 8)),
            msg("Wie wird das Wetter morgen", at(9, 8)),
            msg("Schreib einen Elternbrief zum Ausflug", at(2, 8)),
        ];
        assert!(find(&mixed, now).is_empty());
        // Same hour on workdays of different weekdays.
        let mornings = [
            msg("Was steht heute im Vertretungsplan", at(17, 7)),
            msg("Was steht heute im Vertretungsplan", at(18, 7)),
            msg("Was steht im Vertretungsplan heute", at(19, 8)),
        ];
        let daily = find(&mornings, now);
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].cron, "0 7 * * 1-5");
        // Prompts the app wrote never count.
        let auto: Vec<OwnMessage> = mondays
            .iter()
            .cloned()
            .map(|mut m| {
                m.automatic = true;
                m
            })
            .collect();
        assert!(find(&auto, now).is_empty());
    }
}
