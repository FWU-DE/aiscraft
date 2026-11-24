//! Chat backends (layer 1). Every backend streams [`StreamEvent`]s for a [`ChatRequest`]; the
//! engine never knows which wire protocol is behind it.

mod aischat;
mod anthropic;
pub mod browser_login;
mod claude_cli;
mod http;
mod openai;
pub mod picture;
mod sse;
pub mod web;

use std::sync::Arc;

use aiscraft_core::{
    AccountInfo, BackendConfig, BackendKind, Capabilities, ChatRequest, Error, ImageRequest, ImageResult, ModelInfo, PersonaInfo, RemoteConversation, Result,
    StreamEvent,
};
use futures::future::BoxFuture;
use futures::stream::BoxStream;

pub use aischat::{SESSION_COOKIE, normalize_session_cookie};
pub use anthropic::DEFAULT_MODEL as ANTHROPIC_DEFAULT_MODEL;

pub type EventStream = BoxStream<'static, Result<StreamEvent>>;

/// Upper bound for one tool call's streamed arguments.
pub(crate) const MAX_TOOL_ARGS: usize = 2 * 1024 * 1024;

/// A finished tool call. Arguments that are not a JSON object become `{"_invalid_json": raw}`, so
/// the engine can answer the model with an error instead of running anything.
pub(crate) fn tool_call(id: String, name: String, raw: &str) -> aiscraft_core::ToolCall {
    let raw = if raw.trim().is_empty() { "{}" } else { raw };
    let arguments = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) if v.is_object() => v,
        _ => serde_json::json!({ "_invalid_json": raw.chars().take(2000).collect::<String>() }),
    };
    let id = if id.is_empty() { format!("call_{}", name) } else { id };
    aiscraft_core::ToolCall { id, name, arguments, outcome: None }
}

/// Called when a backend's secret changes on its own (AIS.chat renews the session cookie).
pub type SecretSink = Arc<dyn Fn(&str, String) + Send + Sync>;

pub trait Backend: Send + Sync {
    fn config(&self) -> &BackendConfig;
    fn capabilities(&self) -> Capabilities;
    /// Who is signed in, or an `Auth` error.
    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>>;
    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>>;
    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>>;
    /// Upload a file the server keeps; returns its id for [`aiscraft_core::Attachment::remote_id`].
    fn upload(&self, _name: String, _mime: String, _bytes: Vec<u8>) -> BoxFuture<'_, Result<String>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend nimmt keine Dateien an")) })
    }
    fn remote_conversations(&self) -> BoxFuture<'_, Result<Vec<RemoteConversation>>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend hat keinen Verlauf auf dem Server")) })
    }
    /// The allowance used so far, for backends with one (AIS.chat token points).
    fn quota(&self) -> BoxFuture<'_, Result<Option<aiscraft_core::Quota>>> {
        Box::pin(async { Ok(None) })
    }
    /// The messages of a server-side conversation, oldest first: `(id, role, content)`.
    fn remote_messages(&self, _id: String) -> BoxFuture<'_, Result<Vec<(String, aiscraft_core::Role, String)>>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend hat keinen Verlauf auf dem Server")) })
    }
    /// Assistants, characters and learning scenarios the person may chat with (own and shared).
    fn personas(&self) -> BoxFuture<'_, Result<Vec<PersonaInfo>>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend kennt keine Assistenten, Figuren oder Lernszenarien")) })
    }
    /// Mark or unmark an assistant, character or learning scenario as a favourite.
    fn set_favorite(&self, _kind: aiscraft_core::PersonaKind, _id: String, _favorite: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend kennt keine Favoriten")) })
    }
    /// Models that draw images instead of writing text.
    fn image_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn generate_image(&self, _req: ImageRequest) -> BoxFuture<'_, Result<ImageResult>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend erzeugt keine Bilder")) })
    }
    /// Let the server read a web page a message refers to. `Ok(false)`: not checked (the backend
    /// cannot), `Ok(true)`: the server read it, `Err`: it cannot.
    fn check_link(&self, _url: String) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async { Ok(false) })
    }
    /// The conversation in the backend's own web app, if it has one.
    fn web_url(&self, _conversation_id: &str) -> Option<String> {
        None
    }
}

/// Build a backend from its config and secret.
pub fn build(config: BackendConfig, secret: Option<String>, on_secret: SecretSink) -> Result<Arc<dyn Backend>> {
    let base = config.base_url.trim().trim_end_matches('/').to_string();
    if config.kind == BackendKind::ClaudeCli {
        return Ok(Arc::new(claude_cli::ClaudeCli::new(config, base)?));
    }
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err(Error::invalid(format!("Adresse muss mit https:// beginnen: {base}")));
    }
    Ok(match config.kind {
        BackendKind::AisChat => Arc::new(aischat::AisChat::new(config, base, secret, on_secret)?),
        BackendKind::OpenAi => Arc::new(openai::OpenAi::new(config, base, secret)?),
        BackendKind::Anthropic => Arc::new(anthropic::Anthropic::new(config, base, secret)?),
        BackendKind::ClaudeCli => Arc::new(claude_cli::ClaudeCli::new(config, base)?),
    })
}
