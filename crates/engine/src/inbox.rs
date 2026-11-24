//! Benachrichtigungen in der App: what schedules, page watches, check-ins, hints and background
//! tasks report. Each one becomes a short toast, an entry in this list (kept in the settings
//! table, at most [`MAX`]) and, in the real app, a desktop notification.

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::opt_str;
use crate::{Ctx, NoticeAction, NoticeLevel, new_id, now_ms};

const KEY: &str = "inbox";
/// Entries kept; older ones drop out.
pub const MAX: usize = 50;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: String,
    /// When it happened (ms since epoch).
    pub at: i64,
    pub level: NoticeLevel,
    /// Who reports: the schedule's title, "Hinweis", "Aufgabe".
    pub title: String,
    pub text: String,
    /// What a click does ("Öffnen" → `chat.open`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<NoticeAction>,
    #[serde(default)]
    pub read: bool,
}

pub(crate) fn load(c: &Ctx) -> Vec<InboxItem> {
    c.store(|st| Ok(st.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

fn save(c: &Ctx, all: Vec<InboxItem>) -> Result<()> {
    c.store(|st| st.set_setting(KEY, &serde_json::to_value(&all)?))?;
    c.update(|s| s.inbox = all);
    Ok(())
}

/// Report something that happened without the owner: a toast, an entry in the list and (in the
/// real app only) a desktop notification.
pub(crate) fn notify(c: &Ctx, level: NoticeLevel, title: &str, text: &str, action: Option<NoticeAction>) {
    let toast = if title.is_empty() { text.to_string() } else { format!("{title}: {text}") };
    c.notice_with(level, toast, action.clone());
    let item =
        InboxItem { id: new_id(), at: now_ms(), level, title: title.chars().take(200).collect(), text: text.chars().take(600).collect(), action, read: false };
    let mut all = c.state().inbox.clone();
    all.insert(0, item);
    all.truncate(MAX);
    if let Err(e) = save(c, all) {
        log::warn!("Benachrichtigung nicht gespeichert: {e}");
    }
    if c.desktop_allowed() {
        let head = if title.is_empty() { "AIS.ChatCraft".to_string() } else { format!("AIS.ChatCraft · {title}") };
        crate::schedules::desktop_notify(&head, text);
    }
}

/// `notice.read`: one entry (`id`) or all as read.
pub(crate) async fn read(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id");
    let mut all = c.state().inbox.clone();
    if let Some(id) = &id
        && !all.iter().any(|i| &i.id == id)
    {
        return Err(Error::not_found("Benachrichtigung nicht gefunden"));
    }
    for i in all.iter_mut().filter(|i| id.as_ref().is_none_or(|id| &i.id == id)) {
        i.read = true;
    }
    save(&c, all)?;
    Ok(json!({"unread": c.state().inbox.iter().filter(|i| !i.read).count()}))
}

/// `notice.clear`: remove one entry (`id`) or all.
pub(crate) async fn clear(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id");
    let mut all = c.state().inbox.clone();
    let before = all.len();
    all.retain(|i| id.as_ref().is_some_and(|id| &i.id != id));
    if id.is_some() && all.len() == before {
        return Err(Error::not_found("Benachrichtigung nicht gefunden"));
    }
    save(&c, all)?;
    Ok(json!({"removed": before - c.state().inbox.len()}))
}
