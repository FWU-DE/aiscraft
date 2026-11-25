//! The usage ledger (as in Nod): tokens per month, backend and model, and a monthly limit after
//! which AIS.ChatCraft asks before sending.

use aiscraft_core::{Error, Result};
use serde_json::{Value, json};

use crate::{Ctx, NoticeAction, NoticeLevel};

/// Dollars per million tokens (input, output) for API backends where the price is known.
pub(crate) fn price(model: &str) -> Option<(f64, f64)> {
    let m = model.to_lowercase();
    [
        ("opus 5.5", (4.0, 20.0)),
        ("claude-opus-5-5", (4.0, 20.0)),
        ("sonnet 5.5", (2.0, 10.0)),
        ("claude-sonnet-5-5", (2.0, 10.0)),
        ("haiku 5.5", (0.10, 0.50)),
        ("claude-haiku-5-5", (0.10, 0.50)),
        ("fable 5.2", (8.0, 40.0)),
        ("claude-fable-5-2", (8.0, 40.0)),
    ]
    .iter()
    .find(|(k, _)| m.contains(k))
    .map(|(_, p)| *p)
}

pub(crate) fn this_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

pub(crate) async fn report(c: Ctx, p: Value) -> Result<Value> {
    let months = p.get("months").and_then(Value::as_u64).unwrap_or(6).clamp(1, 36) as u32;
    let rows = c.store(|st| st.usage_by_month(months))?;
    let state = c.state();
    let out: Vec<Value> = rows
        .iter()
        .map(|r| {
            let backend = state.backend(&r.backend_id);
            let kind = backend.map(|b| b.config.kind);
            // Only API backends bill per token; a subscription or AIS.chat does not.
            let usd = kind
                .is_some_and(|k| aiscraft_core::speaks_anthropic(k, &r.model))
                .then(|| price(&r.model).map(|(i, o)| (r.input_tokens as f64 * i + r.output_tokens as f64 * o) / 1e6))
                .flatten();
            json!({
                "month": r.month,
                "backend": backend.map(|b| b.config.name.clone()).unwrap_or_else(|| r.backend_id.clone()),
                "model": r.model,
                "input_tokens": r.input_tokens,
                "output_tokens": r.output_tokens,
                "answers": r.answers,
                "usd": usd,
            })
        })
        .collect();
    Ok(json!({"rows": out, "this_month": this_month(), "limit": state.settings.monthly_token_limit}))
}

pub(crate) fn month_total(c: &Ctx) -> Result<u64> {
    let m = this_month();
    Ok(c.store(|st| st.usage_by_month(1))?.iter().filter(|r| r.month == m).map(|r| (r.input_tokens + r.output_tokens).max(0) as u64).sum())
}

/// Past the limit: refuse with an offer to send anyway.
pub(crate) fn check_limit(c: &Ctx, text: &str, files: &[String], artifact: Option<&str>) -> Result<()> {
    let limit = c.state().settings.monthly_token_limit;
    if limit == 0 {
        return Ok(());
    }
    let used = month_total(c)?;
    if used < limit {
        return Ok(());
    }
    c.notice_with(
        NoticeLevel::Warning,
        format!("Monatsgrenze erreicht: {used} von {limit} Tokens."),
        Some(NoticeAction {
            label: "Trotzdem senden".into(),
            command: "chat.send".into(),
            params: json!({"text": text, "files": files, "artifact": artifact, "force": true}),
        }),
    );
    Err(Error::refused(format!("Die Monatsgrenze von {limit} Tokens ist erreicht ({used} verbraucht).")))
}
