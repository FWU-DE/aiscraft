//! The action log (#23, as Nod's „The log“): every model call (model, tokens, cost) and every
//! tool action with its target and why it was allowed (only reading, the owner's request, a
//! rule, an approval). Secrets never go in: vault values are scrubbed, arguments summarised.

use aiscraft_core::{Error, Result};
use aiscraft_store::LogEntry;
use serde_json::{Value, json};

use crate::{Ctx, now_ms};

const MAX_DETAIL: usize = 300;

fn cap(s: &str, n: usize) -> String {
    let t: String = s.chars().filter(|c| !c.is_control() || *c == ' ').take(n).collect();
    if s.chars().count() > n { format!("{t}…") } else { t }
}

/// Why a tool call ran or did not.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Why {
    /// Built-in tools and read-only connector tools.
    ReadOnly,
    /// Built-in tools that change only AIS.ChatCraft's own data (documents, schedules).
    Own,
    /// The owner's request covered it (`message:N` and the words).
    Request { source: String, quote: String },
    /// One of the owner's rules allowed it.
    Rule(String),
    /// The owner said yes on the card.
    Approved,
    /// It did not run; the reason.
    Refused(String),
}

impl Why {
    pub(crate) fn text(&self) -> String {
        match self {
            Why::ReadOnly => "Nur lesen".into(),
            Why::Own => "Eigene Daten von AIS.ChatCraft".into(),
            Why::Request { source, quote } => format!("Deine Bitte ({source}): „{}“", cap(quote, 160)),
            Why::Rule(r) => format!("Deine Regel: „{}“", cap(r, 160)),
            Why::Approved => "Deine Freigabe".into(),
            Why::Refused(r) => cap(r, 200),
        }
    }
}

/// A short summary of the arguments: the highlighted values, else compact JSON.
pub(crate) fn summary(args: &Value, schema: &Value, secrets: &[String]) -> String {
    let h = crate::highlights::highlights(args, schema);
    let s = if h.is_empty() {
        serde_json::to_string(args).unwrap_or_default()
    } else {
        h.iter().map(|x| format!("{}: {}", x.label, x.value)).collect::<Vec<_>>().join(" · ")
    };
    cap(&crate::vault::scrub(&s, secrets), MAX_DETAIL)
}

fn cost(c: &Ctx, backend_id: &str, model: &str, input: u64, output: u64) -> Option<f64> {
    let kind = c.state().backend(backend_id).map(|b| b.config.kind);
    kind.is_some_and(|k| aiscraft_core::speaks_anthropic(k, model))
        .then(|| crate::usage::price(model).map(|(i, o)| (input as f64 * i + output as f64 * o) / 1e6))
        .flatten()
}

fn add(c: &Ctx, e: LogEntry) {
    let _ = c.store(|st| st.log_action(&e));
}

/// A model call with what it used (services that report no tokens are logged without).
pub(crate) fn model(c: &Ctx, conv: &str, backend_id: &str, model: &str, usage: Option<aiscraft_core::Usage>, what: &str) {
    add(
        c,
        LogEntry {
            at: now_ms(),
            conversation_id: conv.to_string(),
            kind: "model".into(),
            name: cap(model, 120),
            detail: what.to_string(),
            reason: String::new(),
            outcome: "ok".into(),
            input_tokens: usage.map(|u| u.input_tokens as i64),
            output_tokens: usage.map(|u| u.output_tokens as i64),
            usd: usage.and_then(|u| cost(c, backend_id, model, u.input_tokens, u.output_tokens)),
        },
    );
}

/// A tool action: what, with which arguments in short, why, and how it ended.
pub(crate) fn tool(c: &Ctx, conv: &str, name: &str, detail: String, why: &Why, outcome: &str) {
    add(
        c,
        LogEntry {
            at: now_ms(),
            conversation_id: conv.to_string(),
            kind: "tool".into(),
            name: cap(name, 120),
            detail,
            reason: why.text(),
            outcome: outcome.to_string(),
            input_tokens: None,
            output_tokens: None,
            usd: None,
        },
    );
}

pub(crate) async fn list(c: Ctx, p: Value) -> Result<Value> {
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1, 5000) as usize;
    let conv = p.get("conversation").and_then(Value::as_str).map(str::to_string);
    let rows = c.store(|st| st.action_log(limit, conv.as_deref()))?;
    let state = c.state();
    let out: Vec<Value> = rows
        .into_iter()
        .map(|e| {
            let chat = state.conversation(&e.conversation_id).map(|x| x.title.clone()).unwrap_or_default();
            let mut v = serde_json::to_value(&e).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.insert("chat".into(), json!(chat));
            }
            v
        })
        .collect();
    Ok(json!({"entries": out}))
}

/// A CSV cell: quoted, and a leading = + - @ cannot start a formula in a spreadsheet.
fn cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    format!("\"{}\"", s.replace('"', "\"\"").replace(['\n', '\r'], " "))
}

pub(crate) fn csv(rows: &[LogEntry], chat: impl Fn(&str) -> String) -> String {
    let mut out = String::from("Zeit;Chat;Art;Was;Angaben;Warum;Ergebnis;Tokens ein;Tokens aus;Kosten USD\n");
    for e in rows.iter().rev() {
        let fields = [
            crate::format_local(e.at),
            chat(&e.conversation_id),
            (if e.kind == "model" { "Modell" } else { "Werkzeug" }).to_string(),
            e.name.clone(),
            e.detail.clone(),
            e.reason.clone(),
            e.outcome.clone(),
            e.input_tokens.map(|n| n.to_string()).unwrap_or_default(),
            e.output_tokens.map(|n| n.to_string()).unwrap_or_default(),
            e.usd.map(|u| format!("{u:.4}")).unwrap_or_default(),
        ];
        out.push_str(&fields.iter().map(|f| cell(f)).collect::<Vec<_>>().join(";"));
        out.push('\n');
    }
    out
}

pub(crate) async fn export(c: Ctx, p: Value) -> Result<Value> {
    let path = crate::commands::req_str(&p, "path")?;
    if path.trim().is_empty() {
        return Err(Error::invalid("Pfad fehlt"));
    }
    let rows = c.store(|st| st.action_log(usize::MAX, None))?;
    let state = c.state();
    let text = csv(&rows, |id| state.conversation(id).map(|x| x.title.clone()).unwrap_or_default());
    crate::disk::write(std::path::Path::new(&path), text.as_bytes())?;
    Ok(json!({"path": path, "entries": rows.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_cells_cannot_become_formulas_and_summaries_hide_secrets() {
        assert_eq!(cell("=HYPERLINK(1)"), "\"'=HYPERLINK(1)\"");
        assert_eq!(cell("a\"b\nc"), "\"a\"\"b c\"");
        let s = summary(&json!({"an": "x@y.de", "text": "geheim-123"}), &json!({}), &["geheim-123".into()]);
        assert_eq!(s, "An: x@y.de");
        let s = summary(&json!({"q": "geheim-123"}), &json!({}), &["geheim-123".into()]);
        assert!(!s.contains("geheim-123") && s.contains("[Tresor]"), "{s}");
        assert!(summary(&json!({"q": "x".repeat(5000)}), &json!({}), &[]).chars().count() <= MAX_DETAIL + 1);
    }
}
