//! Everything the UI, CLI and control channel can see. Serde, so agents can read it.

use std::collections::BTreeMap;

use aiscraft_core::{AccountInfo, Artifact, BackendConfig, Capabilities, Conversation, MemoryNote, Message, ModelInfo, Project, RemoteConversation};
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
    /// Messages of the open chat that have other versions (after edit or regenerate), by id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, aiscraft_store::Variant>,
    /// Backend, model and project for the open (or new) chat.
    pub selection: Selection,
    /// Conversations with an answer being generated right now.
    pub generating: Vec<String>,
    pub remote: BTreeMap<String, Vec<RemoteConversation>>,
    pub search: Option<SearchState>,
    pub notices: Vec<Notice>,
    pub settings: Settings,
    pub artifacts: Vec<Artifact>,
    /// The artifact shown next to the chat.
    pub open_artifact: Option<String>,
    /// Increases when any artifact's content changes, so open views reload.
    pub artifact_rev: u64,
    pub connectors: Vec<ConnectorView>,
    /// Tool calls waiting for the owner's yes or no.
    pub approvals: Vec<Approval>,
    pub schedules: Vec<crate::schedules::Schedule>,
    pub skills: Vec<crate::skills::Skill>,
    /// Dictation is recording.
    pub recording: bool,
    /// Vault entries, without their secrets.
    pub vault: Vec<crate::vault::EntryInfo>,
    /// Live collaboration (the `collab` plug-in); empty and unavailable without it.
    #[serde(default)]
    pub collab: CollabState,
    /// Pictures of assistants, characters and scenarios fetched so far, by
    /// `backend/kind/id`: the cached file, or `None` when there is none (or it failed).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub persona_pictures: BTreeMap<String, Option<String>>,
    /// Files in the composer, by path: what will be sent (converted, pages picked, location
    /// removed) or why not. Filled by `attachment.prepare`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attachments: BTreeMap<String, crate::attach::AttachmentPreview>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CollabState {
    /// The plug-in is built in.
    pub available: bool,
    /// The relay new shares go to; empty = none set.
    pub server: String,
    pub shares: Vec<CollabView>,
}

/// One shared artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollabView {
    pub artifact_id: String,
    pub server: String,
    /// The join code (`XXXXX-XXXXX`) and link, to hand on.
    pub code: String,
    pub link: String,
    /// Made this room (may end it for everyone).
    pub owner: bool,
    /// "connecting", "online", "offline" or "ended".
    pub status: String,
    /// The status for people, in German.
    pub detail: String,
    pub peers: u32,
}

/// The key of a persona's picture in [`State::persona_pictures`].
pub fn picture_key(backend: &str, kind: aiscraft_core::PersonaKind, id: &str) -> String {
    format!("{backend}/{}/{id}", kind.as_str())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// "Befehl …" or the URL, without secrets.
    pub target: String,
    /// "off", "connecting", "connected", "error".
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub tools: Vec<ConnectorTool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorTool {
    /// The name the model sees (unique across connectors).
    pub llm_name: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub read_only: bool,
    pub destructive: bool,
    #[serde(skip)]
    pub input_schema: serde_json::Value,
}

/// A tool call the model wants to make and the owner has to allow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub conversation_id: String,
    pub tool: String,
    /// Connector and tool title, for people.
    pub label: String,
    pub arguments: serde_json::Value,
    pub destructive: bool,
    /// The trust check's verdict, when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<TrustVerdict>,
}

/// Whether the owner's own words cover an action (Nod's trust check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustVerdict {
    pub covered: bool,
    /// The owner's words that allow it, verbatim (checked against what they actually wrote).
    pub quote: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub backend_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Sources of the open chat, or of the next new one.
    #[serde(default)]
    pub sources: aiscraft_core::Sources,
    /// The assistant, character or learning scenario of the open (or next) chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<aiscraft_core::Persona>,
    /// A text chat or an image chat.
    #[serde(default, skip_serializing_if = "aiscraft_core::ConversationKind::is_chat")]
    pub kind: aiscraft_core::ConversationKind,
    /// Aspect ratio for the next image (`quadratic`, `landscape`, `portrait`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub aspect_ratio: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendView {
    pub config: BackendConfig,
    pub capabilities: Capabilities,
    pub has_secret: bool,
    pub status: BackendStatus,
    pub models: Vec<ModelInfo>,
    /// The allowance used, where the backend has one (AIS.chat token points).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<aiscraft_core::Quota>,
    /// Assistants, characters and learning scenarios (AIS.chat), once loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personas: Option<Vec<aiscraft_core::PersonaInfo>>,
    /// Models that draw images (AIS.chat image generation), if the backend has any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_models: Vec<ModelInfo>,
    /// Why the last attempt to load them failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personas_error: Option<String>,
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
    /// "system" (AIS look, light or dark like the system), "light", "dark" or "omarchy" (follow the desktop theme).
    pub theme: String,
    /// Keep notes from "Merke dir: …" and send them to backends that allow context.
    pub memory: bool,
    /// UI zoom.
    pub zoom: f32,
    /// Send with Enter (Shift+Enter for a new line); otherwise Ctrl+Enter.
    pub enter_sends: bool,
    /// Offer tools (connectors, history search, documents) to backends that can call them.
    pub tools: bool,
    /// Before asking, check with a model whether the owner's words cover a tool call.
    pub trust_check: bool,
    /// Run a tool call without asking when the trust check found the owner's words cover it.
    pub auto_allow_covered: bool,
    /// Ask before sending once this month's tokens reach this number (0 = no limit).
    pub monthly_token_limit: u64,
    /// After answers with several tool steps, keep the procedure as a skill.
    pub learn_skills: bool,
    /// Put fitting skills and excerpts from earlier chats into the context.
    pub recall: bool,
    /// Hourly hints for concrete open needs (off by default).
    pub proactive: bool,
    /// No hints from this hour …
    pub quiet_start: u8,
    /// … until this hour.
    pub quiet_end: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            memory: true,
            zoom: 1.0,
            enter_sends: true,
            tools: true,
            trust_check: true,
            auto_allow_covered: false,
            monthly_token_limit: 0,
            learn_skills: true,
            recall: true,
            proactive: false,
            quiet_start: 22,
            quiet_end: 7,
        }
    }
}

impl State {
    pub fn backend(&self, id: &str) -> Option<&BackendView> {
        self.backends.iter().find(|b| b.config.id == id)
    }
    pub fn conversation(&self, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == id)
    }
    pub fn share(&self, artifact: &str) -> Option<&CollabView> {
        self.collab.shares.iter().find(|s| s.artifact_id == artifact)
    }
    pub fn is_generating(&self, conv: &str) -> bool {
        self.generating.iter().any(|g| g == conv)
    }
    /// What a backend lists about a persona (description, greeting, suggestions), if loaded.
    pub fn persona_info(&self, backend: &str, p: &aiscraft_core::Persona) -> Option<&aiscraft_core::PersonaInfo> {
        self.backend(backend)?.personas.as_ref()?.iter().find(|x| x.kind == p.kind && x.id == p.id)
    }
    pub fn model_name(&self, backend: &str, model: &str) -> String {
        self.backend(backend)
            .and_then(|b| b.models.iter().chain(&b.image_models).find(|m| m.id == model))
            .map(|m| m.name.clone())
            .unwrap_or_else(|| model.to_string())
    }
}
