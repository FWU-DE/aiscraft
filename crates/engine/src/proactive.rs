//! Hints (Nod's "nudges"): off by default. Once an hour, outside quiet hours, a model looks at
//! the owner's recent messages and says whether there is one concrete open need (a promise, a
//! deadline, a wish to be reminded). Only then one short hint; never the same one twice, never
//! a reminder of a reminder.

use aiscraft_core::Result;
use chrono::Timelike;
use serde_json::{Value, json};

use crate::{Ctx, NoticeAction, NoticeLevel, now_ms};

const SENT_KEY: &str = "nudges_sent";

pub fn quiet(hour: u32, start: u8, end: u8) -> bool {
    let (s, e) = (u32::from(start % 24), u32::from(end % 24));
    if s == e {
        false
    } else if s < e {
        hour >= s && hour < e
    } else {
        hour >= s || hour < e
    }
}

fn fingerprint(text: &str) -> String {
    text.to_lowercase().split_whitespace().take(12).collect::<Vec<_>>().join(" ")
}

/// Look once. `force` ignores the switch and the quiet hours (for "Jetzt prüfen").
pub(crate) async fn check(c: &Ctx, force: bool) -> Result<Option<String>> {
    let s = c.state().settings.clone();
    if !force && (!s.proactive || quiet(chrono::Local::now().hour(), s.quiet_start, s.quiet_end)) {
        return Ok(None);
    }
    // A request the owner keeps making may become a schedule (#68): found without a model.
    let _ = crate::schedule_ideas::announce(c);
    let msgs = c.store(|st| st.user_messages_since(now_ms() - 48 * 3_600_000, 60))?;
    if msgs.is_empty() {
        return Ok(None);
    }
    let lines: Vec<String> = msgs.iter().map(|(conv, title, text, _)| format!("[{conv}] ({title}) {}", text.chars().take(400).collect::<String>())).collect();
    let system = "Du prüfst, ob die Person gerade eine konkrete, offene Sache hat, an die ein kurzer Hinweis erinnern sollte: eine Zusage, eine Frist, ein ausdrücklicher Erinnerungswunsch. Kein allgemeiner Rat, nichts Erledigtes, nichts Vages. Im Zweifel: nichts. Antworte nur mit JSON: {\"need\": true|false, \"text\": \"ein kurzer Satz an die Person\", \"conversation_id\": \"ID aus den eckigen Klammern\"}";
    let today = chrono::Local::now().format("%A, %d.%m.%Y %H:%M");
    let prompt = format!("Jetzt: {today}\n\n<nachrichten_der_person>\n{}\n</nachrichten_der_person>", lines.join("\n"));
    let out = crate::chat::complete(c, Some(system), &prompt).await?;
    let start = out.find('{').unwrap_or(0);
    let end = out.rfind('}').map(|e| e + 1).unwrap_or(out.len());
    let v: Value = out.get(start..end).and_then(|x| serde_json::from_str(x).ok()).unwrap_or(Value::Null);
    if v.get("need").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    let Some(text) = v.get("text").and_then(Value::as_str).map(|t| t.trim().chars().take(200).collect::<String>()).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let fp = fingerprint(&text);
    let mut sent: Vec<String> = c.store(|st| Ok(st.setting(SENT_KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()))?;
    if sent.contains(&fp) {
        return Ok(None);
    }
    sent.push(fp);
    let keep = sent.len().saturating_sub(200);
    sent.drain(..keep);
    c.store(|st| st.set_setting(SENT_KEY, &json!(sent)))?;
    let conv = v.get("conversation_id").and_then(Value::as_str).filter(|id| c.state().conversation(id).is_some()).map(str::to_string);
    crate::inbox::notify(
        c,
        NoticeLevel::Info,
        "Hinweis",
        &text,
        conv.map(|id| NoticeAction { label: "Chat öffnen".into(), command: "chat.open".into(), params: json!({"id": id}) }),
    );
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours_wrap_midnight() {
        assert!(quiet(23, 22, 7) && quiet(3, 22, 7) && !quiet(12, 22, 7) && !quiet(7, 22, 7));
        assert!(quiet(13, 12, 14) && !quiet(14, 12, 14));
        assert!(!quiet(5, 0, 0));
    }
}
