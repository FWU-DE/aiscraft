//! Chat backends (layer 1). Every backend streams [`StreamEvent`]s for a [`ChatRequest`]; the
//! engine never knows which wire protocol is behind it.

mod aischat;
mod anthropic;
mod claude_cli;
mod http;
mod openai;
mod sse;

use std::sync::Arc;

use aiscraft_core::{AccountInfo, BackendConfig, BackendKind, Capabilities, ChatRequest, Error, ModelInfo, RemoteConversation, Result, StreamEvent};
use futures::future::BoxFuture;
use futures::stream::BoxStream;

pub use aischat::{SESSION_COOKIE, normalize_session_cookie};
pub use anthropic::DEFAULT_MODEL as ANTHROPIC_DEFAULT_MODEL;

pub type EventStream = BoxStream<'static, Result<StreamEvent>>;

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
    /// The messages of a server-side conversation, oldest first: `(id, role, content)`.
    fn remote_messages(&self, _id: String) -> BoxFuture<'_, Result<Vec<(String, aiscraft_core::Role, String)>>> {
        Box::pin(async { Err(Error::unsupported("Dieses Backend hat keinen Verlauf auf dem Server")) })
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
