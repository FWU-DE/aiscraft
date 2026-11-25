//! Backup models (#66, Nod 0.9.5 "Continue on the owner's backup models when a model request
//! fails"): when a model request fails because of the model (overloaded, a server error, a
//! timeout, the quota used up: `ErrorKind::Busy`), the answer continues on the next model of the
//! owner's ordered list, and says which one stepped in and why. Errors about the content (a
//! content filter, `ResponsibleAIError`, invalid input) never switch.
//!
//! Privacy: a backup model at another service would get the chat. That happens only when the
//! owner agreed (`fallback_other_services`), like recall from other services (#49), and never for
//! chats whose history lives on the server (AIS.chat) or that talk to an assistant there.

use std::sync::Arc;

use aiscraft_core::{Error, ErrorKind, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;

/// Label of the step on an answer a backup model gave.
pub const FALLBACK: &str = "Ersatzmodell";
/// At most this many backup models.
pub const MAX_FALLBACKS: usize = 5;

/// One backup model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackModel {
    pub backend_id: String,
    pub model_id: String,
}

/// A backup model ready to take over.
pub(crate) struct Next {
    pub backend: Arc<dyn aiscraft_backends::Backend>,
    pub backend_id: String,
    pub model_id: String,
    pub name: String,
}

/// Whether this error is the model's (and another model may do better).
pub(crate) fn switches(e: &Error) -> bool {
    e.kind == ErrorKind::Busy
}

/// The backup models a chat may continue on, in the owner's order.
pub(crate) fn chain(c: &Ctx, conv: &aiscraft_core::Conversation) -> Vec<Next> {
    let s = c.state();
    let source_remote = s.backend(&conv.backend_id).is_some_and(|b| b.capabilities.remote_history);
    s.settings
        .fallback_models
        .iter()
        .filter(|f| !(f.backend_id == conv.backend_id && f.model_id == conv.model_id) && !f.model_id.is_empty())
        .filter(|f| {
            f.backend_id == conv.backend_id
                || (s.settings.fallback_other_services
                    && !source_remote
                    && conv.persona.is_none()
                    && s.backend(&f.backend_id).is_some_and(|b| !b.capabilities.remote_history))
        })
        .filter_map(|f| {
            let backend = c.backend(&f.backend_id).ok()?;
            Some(Next { backend, backend_id: f.backend_id.clone(), model_id: f.model_id.clone(), name: s.model_name(&f.backend_id, &f.model_id) })
        })
        .collect()
}

/// Why the backup stepped in, for the note under the answer.
pub(crate) fn note(name: &str, service: Option<&str>, e: &Error) -> String {
    let why: String = e.message.chars().take(160).collect();
    match service {
        Some(svc) => format!("Mit Ersatzmodell {name} ({svc}) beantwortet, weil das gewählte Modell nicht konnte: {why}"),
        None => format!("Mit Ersatzmodell {name} beantwortet, weil das gewählte Modell nicht konnte: {why}"),
    }
}

/// `fallback.set`: the ordered list, and whether other services may step in.
pub(crate) async fn set(c: Ctx, p: Value) -> Result<Value> {
    let mut settings = c.state().settings.clone();
    if let Some(list) = p.get("models") {
        let arr = list.as_array().ok_or_else(|| Error::invalid("models muss eine Liste von {backend, model} sein"))?;
        if arr.len() > MAX_FALLBACKS {
            return Err(Error::invalid(format!("Höchstens {MAX_FALLBACKS} Ersatzmodelle")));
        }
        let mut out: Vec<FallbackModel> = Vec::new();
        for v in arr {
            let backend_id = v.get("backend").and_then(Value::as_str).unwrap_or("").to_string();
            let model_id = v.get("model").and_then(Value::as_str).unwrap_or("").trim().chars().take(200).collect::<String>();
            if c.state().backend(&backend_id).is_none() {
                return Err(Error::not_found(format!("Dienst {} nicht gefunden", backend_id.chars().take(40).collect::<String>())));
            }
            if model_id.is_empty() {
                return Err(Error::invalid("Ein Ersatzmodell braucht ein Modell"));
            }
            let f = FallbackModel { backend_id, model_id };
            if !out.contains(&f) {
                out.push(f);
            }
        }
        settings.fallback_models = out;
    }
    if let Some(v) = p.get("other_services") {
        settings.fallback_other_services = v.as_bool().ok_or_else(|| Error::invalid("other_services muss true oder false sein"))?;
    }
    c.store(|st| st.set_setting("settings", &serde_json::to_value(&settings)?))?;
    let out = json!({"models": settings.fallback_models, "other_services": settings.fallback_other_services});
    c.update(|s| s.settings = settings);
    Ok(out)
}
