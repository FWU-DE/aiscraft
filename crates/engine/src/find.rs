//! Finding words in the open chat (#41, Strg+F): every hit in the messages as people read them,
//! the current one, and the next or previous one.

use aiscraft_core::{Error, Result, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;

/// Longest search text.
const MAX_QUERY: usize = 200;
/// Most hits counted in one chat.
const MAX_HITS: usize = 2000;

/// The search in the open chat.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FindState {
    pub query: String,
    /// Every hit, in reading order.
    pub hits: Vec<FindHit>,
    /// Index of the current hit in `hits`.
    pub current: usize,
}

/// One place the text occurs: the message and which occurrence in it (from 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindHit {
    pub message_id: String,
    pub n: usize,
}

impl FindState {
    pub fn current_hit(&self) -> Option<&FindHit> {
        self.hits.get(self.current)
    }
}

/// How often `needle` (lower case) occurs in `hay`, ignoring case, without overlaps.
fn count(hay: &str, needle: &[char]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let hay: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    let mut n = 0;
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay.get(i..i + needle.len()) == Some(needle) {
            n += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    n
}

/// Every hit of `query` in the open chat, as people read it (names of „Namen schützen“ shown).
fn hits(c: &Ctx, query: &str) -> Vec<FindHit> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let state = c.state();
    let mut out = Vec::new();
    for m in crate::privacy::shown_messages(c, &state).iter().filter(|m| matches!(m.role, Role::User | Role::Assistant)) {
        for n in 0..count(&m.content, &needle) {
            if out.len() >= MAX_HITS {
                return out;
            }
            out.push(FindHit { message_id: m.id.clone(), n });
        }
    }
    out
}

/// `chat.find`: search the open chat, step to the next or previous hit, or close the search.
pub(crate) fn run(c: &Ctx, p: &Value) -> Result<Value> {
    if p.get("close").and_then(Value::as_bool).unwrap_or(false) {
        c.update(|s| s.find = None);
        return Ok(json!({"total": 0}));
    }
    if c.state().current.is_none() {
        return Err(Error::invalid("Kein Chat offen"));
    }
    let prev = c.state().find.clone().unwrap_or_default();
    let query = match p.get("query") {
        Some(Value::String(q)) => q.chars().take(MAX_QUERY).collect::<String>(),
        Some(_) => return Err(Error::invalid("query muss ein Text sein")),
        None => prev.query.clone(),
    };
    let found = if query.trim().is_empty() { Vec::new() } else { hits(c, &query) };
    // The same search again keeps its place; a new one starts at the first hit.
    let mut current = if query == prev.query { prev.current.min(found.len().saturating_sub(1)) } else { 0 };
    match p.get("step").and_then(Value::as_str) {
        Some("next") if !found.is_empty() => current = (current + 1) % found.len(),
        Some("prev") if !found.is_empty() => current = (current + found.len() - 1) % found.len(),
        Some("next" | "prev") | None => {}
        Some(_) => return Err(Error::invalid("step muss next oder prev sein")),
    }
    let state = FindState { query, hits: found, current };
    let out = json!({
        "total": state.hits.len(),
        "current": if state.hits.is_empty() { 0 } else { state.current + 1 },
        "message_id": state.current_hit().map(|h| h.message_id.clone()),
    });
    c.update(|s| s.find = Some(state));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::count;

    #[test]
    fn counts_without_case_and_overlap() {
        let n = |hay: &str, q: &str| count(hay, &q.chars().flat_map(char::to_lowercase).collect::<Vec<_>>());
        assert_eq!(n("Igel, IGEL und igelig", "igel"), 3);
        assert_eq!(n("aaaa", "aa"), 2);
        assert_eq!(n("Straße", "STRASSE"), 0);
        assert_eq!(n("ÄPFEL äpfel", "äpfel"), 2);
        assert_eq!(n("x", ""), 0);
    }
}
