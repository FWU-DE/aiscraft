//! The few values a person should check before allowing a tool call (#67): recipient, subject or
//! title, date, file and amount, picked by key name and schema type and written readably. The
//! approval stays bound to the exact arguments; this only decides what the card shows in bold.

use serde_json::Value;

use crate::Highlight;

/// At most this many values are highlighted.
pub const MAX_HIGHLIGHTS: usize = 5;
const MAX_VALUE_CHARS: usize = 80;
const MAX_DEPTH: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Kind {
    Command,
    Recipient,
    Subject,
    Title,
    Start,
    Date,
    End,
    File,
    Limit,
    Amount,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Command => "Befehl",
            Kind::Recipient => "An",
            Kind::Subject => "Betreff",
            Kind::Title => "Titel",
            Kind::Start => "Beginn",
            Kind::Date => "Datum",
            Kind::End => "Ende",
            Kind::File => "Datei",
            Kind::Limit => "Zeitlimit",
            Kind::Amount => "Betrag",
        }
    }
}

/// The words of a key: `start_time`, `startTime` and `start-time` all give `start`, `time`.
fn words(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in key.chars().take(80) {
        if !ch.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_lowercase();
        cur.extend(ch.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn kind_of(key: &str, schema: Option<&Value>) -> Option<Kind> {
    let w = words(key);
    let has = |list: &[&str]| w.iter().any(|x| list.contains(&x.as_str()));
    // A command to run (the own computer, #94, or a shell connector) is checked first.
    let k = if has(&["command", "cmd", "befehl", "script", "shell"]) {
        Some(Kind::Command)
    } else if has(&["timeout", "zeitlimit"]) {
        Some(Kind::Limit)
    } else if has(&[
        "to",
        "recipient",
        "recipients",
        "email",
        "emails",
        "mail",
        "an",
        "empfänger",
        "empfaenger",
        "cc",
        "bcc",
        "attendees",
        "teilnehmer",
        "adressat",
    ]) {
        Some(Kind::Recipient)
    } else if has(&["subject", "betreff"]) {
        Some(Kind::Subject)
    } else if has(&["start", "beginn", "from", "von"]) && !has(&["path", "file", "dir"]) {
        Some(Kind::Start)
    } else if has(&["end", "ende", "until", "bis"]) && !has(&["path", "file", "dir"]) {
        Some(Kind::End)
    } else if has(&["date", "datum", "time", "zeit", "due", "when", "datetime", "termin", "deadline", "frist", "day", "tag"]) {
        Some(Kind::Date)
    } else if has(&["path", "file", "filename", "datei", "pfad", "destination", "target", "ziel", "folder", "ordner", "dir", "directory", "source", "quelle"]) {
        Some(Kind::File)
    } else if has(&["amount", "betrag", "price", "preis", "total", "summe", "sum", "cost", "kosten", "quantity", "anzahl"]) {
        Some(Kind::Amount)
    } else if has(&["title", "titel", "name", "summary", "thema", "topic"]) {
        Some(Kind::Title)
    } else {
        None
    };
    k.or_else(|| match schema.and_then(|s| s.get("format")).and_then(Value::as_str) {
        Some("email") => Some(Kind::Recipient),
        Some("date") | Some("date-time") => Some(Kind::Date),
        _ => None,
    })
}

/// Control characters, bidi overrides and runs of whitespace become one space; long values end
/// with „…“.
pub fn clean(s: &str) -> String {
    let t: String = s
        .chars()
        .take(2000)
        .map(|c| if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}'..='\u{200f}' | '\u{feff}') { ' ' } else { c })
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > MAX_VALUE_CHARS {
        let mut out: String = t.chars().take(MAX_VALUE_CHARS - 1).collect();
        out.push('…');
        out
    } else {
        t
    }
}

const WEEKDAYS: [&str; 7] = ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"];
const MONTHS: [&str; 12] = ["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."];

/// `2026-12-12T19:00:00+01:00` → „Sa, 12. Dez. 2026, 19:00“; anything else stays as it is.
fn readable_date(s: &str) -> Option<String> {
    use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};
    let t = s.trim();
    let (date, time): (NaiveDate, Option<(u32, u32)>) = if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        (dt.date_naive(), Some((dt.hour(), dt.minute())))
    } else if let Some(dt) =
        ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"].iter().find_map(|f| NaiveDateTime::parse_from_str(t, f).ok())
    {
        (dt.date(), Some((dt.hour(), dt.minute())))
    } else {
        (NaiveDate::parse_from_str(t, "%Y-%m-%d").ok()?, None)
    };
    let wd = WEEKDAYS.get(date.weekday().num_days_from_monday() as usize)?;
    let mon = MONTHS.get(date.month0() as usize)?;
    let mut out = format!("{wd}, {}. {mon} {}", date.day(), date.year());
    if let Some((h, m)) = time {
        out.push_str(&format!(", {h:02}:{m:02}"));
    }
    Some(out)
}

fn readable(v: &Value, kind: Kind) -> Option<String> {
    let s = match v {
        Value::String(s) if s.contains("{{vault:") => "ein Tresor-Eintrag".to_string(),
        Value::String(s) => {
            if matches!(kind, Kind::Date | Kind::Start | Kind::End)
                && let Some(d) = readable_date(s)
            {
                return Some(d);
            }
            clean(s)
        }
        Value::Number(n) if kind == Kind::Limit => format!("{n} s"),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => (if *b { "ja" } else { "nein" }).to_string(),
        Value::Array(a) => {
            let items: Vec<String> = a.iter().filter(|x| !x.is_object() && !x.is_array()).filter_map(|x| readable(x, kind)).collect();
            if items.is_empty() {
                return None;
            }
            let more = items.len().saturating_sub(3);
            let mut s = items.into_iter().take(3).collect::<Vec<_>>().join(", ");
            if more > 0 {
                s.push_str(&format!(" und {more} weitere"));
            }
            return Some(clean(&s));
        }
        Value::Object(_) | Value::Null => return None,
    };
    (!s.is_empty()).then_some(s)
}

fn walk(v: &Value, schema: Option<&Value>, depth: usize, found: &mut Vec<(Kind, usize, Highlight)>, seen: &mut usize) {
    let Value::Object(o) = v else { return };
    if depth > MAX_DEPTH {
        return;
    }
    for (key, val) in o.iter().take(100) {
        *seen += 1;
        let sub = schema.and_then(|s| s.get("properties")).and_then(|p| p.get(key));
        if val.is_object() {
            walk(val, sub, depth + 1, found, seen);
            continue;
        }
        let Some(kind) = kind_of(key, sub) else { continue };
        if let Some(value) = readable(val, kind) {
            found.push((kind, *seen, Highlight { label: kind.label().into(), value }));
        }
    }
}

/// The values to check, most important first: recipients, subject, title, dates, file, amount.
pub fn highlights(args: &Value, schema: &Value) -> Vec<Highlight> {
    let mut found = Vec::new();
    let mut seen = 0;
    walk(args, Some(schema), 0, &mut found, &mut seen);
    found.sort_by_key(|(k, i, _)| (*k, *i));
    let mut out: Vec<Highlight> = Vec::new();
    for (_, _, h) in found {
        if !out.iter().any(|x| x.value == h.value) {
            out.push(h);
        }
        if out.len() == MAX_HIGHLIGHTS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]
    use super::*;
    use serde_json::json;

    fn pairs(h: &[Highlight]) -> Vec<(String, String)> {
        h.iter().map(|x| (x.label.clone(), x.value.clone())).collect()
    }

    #[test]
    fn picks_recipient_subject_and_date_readably() {
        let h = highlights(&json!({"an": "kollegium@schule.de", "betreff": "Konferenz am 4. Dezember", "body": "langer Text"}), &json!({}));
        assert_eq!(pairs(&h), [("An".into(), "kollegium@schule.de".into()), ("Betreff".into(), "Konferenz am 4. Dezember".into())]);
        let h = highlights(&json!({"eventId": "abc", "summary": "Elternabend 7b", "startTime": "2026-12-12T19:00:00+01:00"}), &json!({}));
        assert_eq!(pairs(&h), [("Titel".into(), "Elternabend 7b".into()), ("Beginn".into(), "Sa, 12. Dez. 2026, 19:00".into())]);
    }

    #[test]
    fn nested_arguments_schema_formats_and_the_cap() {
        let args = json!({
            "message": {"toRecipients": ["a@x.de", "b@x.de", "c@x.de", "d@x.de"], "subject": "Ausflug"},
            "options": {"deep": {"deeper": {"file_path": "/home/a/Brief.docx"}}},
            "kontakt": "eltern@schule.de", "amount": 12.5, "title": "Plan", "due": "2026-12-01", "extra_date": "2026-12-02"
        });
        let schema = json!({"properties": {"kontakt": {"type": "string", "format": "email"}}});
        let h = highlights(&args, &schema);
        assert_eq!(h.len(), MAX_HIGHLIGHTS);
        let firsts: Vec<&str> = h.iter().take(2).map(|x| x.value.as_str()).collect();
        assert!(h[0].label == "An" && h[1].label == "An", "{h:?}");
        assert!(firsts.contains(&"a@x.de, b@x.de, c@x.de und 1 weitere"), "{h:?}");
        assert!(firsts.contains(&"eltern@schule.de"), "picked by the schema's format");
        assert_eq!(h[2].label, "Betreff");
        assert!(h.iter().any(|x| x.value == "Di, 1. Dez. 2026"));
    }

    #[test]
    fn hostile_values_are_cleaned_and_capped() {
        let long = "x".repeat(10_000);
        let h = highlights(&json!({"to": "a\u{202e}b\n\t\u{0}c  d", "subject": long, "title": {"x": 1}, "file": null}), &json!({}));
        assert_eq!(h[0].value, "a b c d");
        assert_eq!(h[1].value.chars().count(), MAX_VALUE_CHARS);
        assert!(h[1].value.ends_with('…'));
        // Deep nesting, huge objects and non-objects never panic.
        let mut deep = json!({"to": "tief@x.de"});
        for _ in 0..200 {
            deep = json!({"a": deep});
        }
        assert!(highlights(&deep, &json!(null)).is_empty(), "too deep to matter");
        let wide: serde_json::Map<String, Value> = (0..5000).map(|i| (format!("k{i}"), json!(i))).collect();
        assert!(highlights(&Value::Object(wide), &json!({})).is_empty());
        assert!(highlights(&json!("text"), &json!({})).is_empty());
        assert!(highlights(&json!([1, 2]), &json!({})).is_empty());
        let h = highlights(&json!({"password": "{{vault:portal.password}}", "to": "{{vault:portal.username}}"}), &json!({}));
        assert_eq!(h[0].value, "ein Tresor-Eintrag");
    }

    #[test]
    fn key_words() {
        assert_eq!(words("startTime"), ["start", "time"]);
        assert_eq!(words("file_path"), ["file", "path"]);
        assert_eq!(words("E-Mail"), ["e", "mail"]);
    }
}
