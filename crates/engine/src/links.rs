//! Link check for answers: models make up addresses, and teachers pass links on. Each http(s)
//! address in an answer is asked once (HEAD, GET when HEAD is refused); dead ones are struck
//! through under the answer. Contacts other sites, so only on click or with the setting
//! "check_links". Addresses in the local network are never asked.

use std::time::Duration;

use aiscraft_backends::web::{self, LinkState};
use aiscraft_core::{ActivityLink, ActivityStep, Error, Message, Result, Role};
use serde_json::{Value, json};

use crate::Ctx;

/// Label of the step that holds the result.
pub const LINK_CHECK: &str = "Links geprüft";
/// Addresses checked per answer at most.
pub const MAX_LINKS: usize = 20;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

/// Whether a text has an address the link check would ask.
pub fn has_links(text: &str) -> bool {
    !web::urls_in(text, 1).is_empty()
}

/// `message.check_links`: check the links of an answer in the open chat.
pub(crate) async fn check_command(c: Ctx, p: Value) -> Result<Value> {
    let mid = p.get("message_id").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| Error::invalid("message_id fehlt"))?.to_string();
    let timeout = match p.get("timeout_ms").and_then(Value::as_u64) {
        Some(ms) => Duration::from_millis(ms.clamp(200, 20_000)),
        None => DEFAULT_TIMEOUT,
    };
    let state = c.state();
    let m = state.messages.iter().find(|m| m.id == mid).cloned().ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    if m.role != Role::Assistant {
        return Err(Error::invalid("Nur Links in Antworten werden geprüft"));
    }
    if state.is_generating(&m.conversation_id) && state.messages.last().is_some_and(|x| x.id == mid) {
        return Err(Error::invalid("Die Antwort läuft noch"));
    }
    if web::urls_in(&m.content, 1).is_empty() {
        return Err(Error::invalid("Die Antwort enthält keine Links"));
    }
    check_message(&c, &m, timeout).await
}

/// After an answer, with the setting on: check its links in the background.
pub(crate) fn check_later(c: &Ctx, m: &Message) {
    if !c.state().settings.check_links || m.role != Role::Assistant || web::urls_in(&m.content, 1).is_empty() {
        return;
    }
    let ctx = c.clone();
    let m = m.clone();
    c.spawn(async move {
        let _ = check_message(&ctx, &m, DEFAULT_TIMEOUT).await;
    });
}

async fn check_message(c: &Ctx, m: &Message, timeout: Duration) -> Result<Value> {
    let urls = web::urls_in(&m.content, MAX_LINKS);
    let local_ok = c.0.web_local_ok.load(std::sync::atomic::Ordering::Relaxed);
    let states = futures::future::join_all(urls.iter().map(|u| web::check(u, local_ok, timeout))).await;
    let mut links = Vec::new();
    let mut report = Vec::new();
    let (mut alive, mut dead, mut skipped) = (0usize, 0usize, 0usize);
    for (url, state) in urls.iter().zip(states) {
        let short = short_url(url);
        let (title, failed, status, note) = match &state {
            LinkState::Alive => {
                alive += 1;
                (short, false, "ok", String::new())
            }
            LinkState::Dead(why) => {
                dead += 1;
                (format!("{short} ({why})"), true, "dead", why.clone())
            }
            LinkState::Skipped(why) => {
                skipped += 1;
                (format!("{short} (nicht geprüft)"), false, "skipped", why.clone())
            }
        };
        report.push(json!({"url": url, "status": status, "note": note}));
        links.push(ActivityLink { title, url: url.clone(), failed, cited: false });
    }
    let mut detail = format!("{alive} erreichbar");
    if dead > 0 {
        detail.push_str(&format!(", {dead} nicht erreichbar"));
    }
    if skipped > 0 {
        detail.push_str(&format!(", {skipped} nicht geprüft (lokales Netz oder ungültig)"));
    }
    let step = ActivityStep { label: LINK_CHECK.into(), detail: Some(detail), done: true, failed: dead > 0, links, document: None };
    // The message as it is now (it may have moved on while the pages were asked).
    let conv = m.conversation_id.clone();
    let mut current = c.store(|st| st.messages(&conv))?.into_iter().find(|x| x.id == m.id).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?;
    current.activity.retain(|s| s.label != LINK_CHECK);
    current.activity.push(step);
    c.store(|st| st.save_message(&current))?;
    c.update(|s| {
        if s.current.as_deref() == Some(conv.as_str())
            && let Some(x) = s.messages.iter_mut().find(|x| x.id == current.id)
        {
            x.activity = current.activity.clone();
        }
    });
    Ok(json!({"checked": urls.len(), "alive": alive, "dead": dead, "skipped": skipped, "links": report}))
}

/// A link as people read it: host and path, without the scheme, shortened.
fn short_url(url: &str) -> String {
    let s = url.split_once("://").map(|(_, r)| r).unwrap_or(url).trim_end_matches('/');
    if s.chars().count() > 60 { format!("{}…", s.chars().take(59).collect::<String>()) } else { s.to_string() }
}
