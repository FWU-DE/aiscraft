//! Everything the UI, CLI and control channel can see. Serde, so agents can read it.

use std::collections::BTreeMap;

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, Conversation, MemoryNote, Message, ModelInfo, Project, RemoteConversation};
use aiscraft_store::SearchHit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Increases with every change; surfaces redraw when it moves.
    pub revision: u64,
    pub backends: Vec<BackendView>,
    pub conversations: Vec<Conversation>,
    pub projects: Vec<Project>,
    pub memory: Vec<MemoryNote>,
    /// The open conversation; `None` = a new chat that starts on the first message.
    pub current: Option<String>,
    pub messages: Vec<Message>,
    /// Backend, model and project for the open (or new) chat.
    pub selection: Selection,
    /// Conversations with an answer being generated right now.
    pub generating: Vec<String>,
    pub remote: BTreeMap<String, Vec<RemoteConversation>>,
    pub search: Option<SearchState>,
    pub notices: Vec<Notice>,
    pub settings: Settings,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub backend_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendView {
    pub config: BackendConfig,
    pub capabilities: Capabilities,
    pub has_secret: bool,
    pub status: BackendStatus,
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum BackendStatus {
    #[default]
    Unknown,
    Checking,
    Ready {
        account: AccountInfo,
    },
    SignedOut {
        message: String,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchState {
    pub query: String,
    pub hits: Vec<SearchHit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub level: NoticeLevel,
    pub text: String,
    /// A command the notice offers ("Rückgängig"), with its label and params.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<NoticeAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoticeAction {
    pub label: String,
    pub command: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// "system", "light", "dark" or "omarchy" (follow the desktop theme).
    pub theme: String,
    /// Keep notes from "Merke dir: …" and send them to backends that allow context.
    pub memory: bool,
    /// UI zoom.
    pub zoom: f32,
    /// Send with Enter (Shift+Enter for a new line); otherwise Ctrl+Enter.
    pub enter_sends: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme: "omarchy".into(), memory: true, zoom: 1.0, enter_sends: true }
    }
}

impl State {
    pub fn backend(&self, id: &str) -> Option<&BackendView> {
        self.backends.iter().find(|b| b.config.id == id)
    }
    pub fn conversation(&self, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == id)
    }
    pub fn is_generating(&self, conv: &str) -> bool {
        self.generating.iter().any(|g| g == conv)
    }
    pub fn model_name(&self, backend: &str, model: &str) -> String {
        self.backend(backend).and_then(|b| b.models.iter().find(|m| m.id == model)).map(|m| m.name.clone()).unwrap_or_else(|| model.to_string())
    }
}
