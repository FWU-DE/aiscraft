//! Everything the UI, CLI and control channel can see. Serde, so agents can read it.

use std::collections::BTreeMap;

use aiscraft_core::{AccountInfo, Artifact, BackendConfig, Capabilities, Conversation, MemoryNote, Message, ModelInfo, Project, RemoteConversation};
use aiscraft_store::SearchHit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// The last quick entry (#35): the window comes to the front with a new chat.
    pub quick: Option<crate::quick::QuickEntry>,
    /// The citation opened from an answer (#39): the passage where it stands.
    pub citation: Option<crate::citations::CitationView>,
    /// The search in the open chat (Strg+F, #41).
    pub find: Option<crate::find::FindState>,
    /// Increases with every change; surfaces redraw when it moves.
    pub revision: u64,
    pub backends: Vec<BackendView>,
    pub conversations: Vec<Conversation>,
    pub projects: Vec<Project>,
    /// Files in the projects' knowledge (#32), without their text.
    #[serde(default)]
    pub project_files: Vec<aiscraft_core::ProjectFile>,
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
    /// Benachrichtigungen (schedules, watches, check-ins, hints, tasks), newest first, kept on disk.
    #[serde(default)]
    pub inbox: Vec<crate::inbox::InboxItem>,
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
    /// „Namen schützen“: class lists (without names) and the composer's preview (#55).
    #[serde(default)]
    pub privacy: crate::privacy::PrivacyState,
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
    /// The update check: running version, what the last check found.
    #[serde(default)]
    pub update: UpdateState,
    /// Templates for recurring tasks (#54).
    #[serde(default)]
    pub templates: Vec<crate::templates::Template>,
    /// Text for the composer from a template or a connector (#53, #54); the UI takes each `seq` once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composer_fill: Option<crate::templates::ComposerFill>,
    /// Readability shown under answers (`message.readability`, #48), by message id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub readability: BTreeMap<String, crate::readability::Readability>,
}

/// The update check (`app.check_update`). No download, no install: a hint with a link.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// The running version (`0.1.0`).
    pub current: String,
    /// A newer published release to point out; `None` when there is none, it was skipped or
    /// dismissed for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<UpdateInfo>,
    pub checking: bool,
    /// Last check (ms since 1970), 0 = never.
    pub last_check: i64,
    /// The result of the last manual check, for the settings ("Du hast die neueste Version").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// "Diese Version überspringen".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// Hidden until the next start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissed: Option<String>,
}

/// A published release newer than the running version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateInfo {
    /// `0.2.0`
    pub version: String,
    pub tag: String,
    pub name: String,
    /// The release page ("Was ist neu").
    pub url: String,
    /// This computer's download, else the release page.
    pub download: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
    /// Release notes (Markdown, shortened).
    pub notes: String,
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
    /// The few values to check, shown in bold on the card (#67); the yes still covers exactly
    /// `arguments`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlights: Vec<Highlight>,
    /// A warning for the card, e.g. that the same action may have run already (#24).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One value worth checking before a yes: „An“ kollegium@schule.de.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Highlight {
    pub label: String,
    pub value: String,
}

/// Whether the owner's own words cover an action (Nod's trust check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustVerdict {
    pub covered: bool,
    /// The owner's words the permission rests on, taken by the app from the named source (never
    /// copied from the checker's answer).
    pub quote: String,
    pub reason: String,
    /// Where the permission comes from: `message:N` (one of the owner's messages) or `rule:N`
    /// (one of the owner's rules, #22).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Each explicit term of the source next to the action's value (#63).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terms: Vec<TrustTerm>,
    /// The owner's prohibition that applies (#22): then the action never runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forbidden: Option<String>,
    /// The allowing rule the permission rests on, in the owner's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

/// One explicit term of the owner's words (recipient, date, file, amount) and the action's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustTerm {
    pub term: String,
    pub value: String,
    pub matches: bool,
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
    /// The next new chat is temporary (#34): not kept, not searched, no memory notes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub temporary: bool,
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
    /// The look, one of [`THEMES`]: "modern", "glas" (each also "-light" or "-dark", else like the
    /// system) or "omarchy" (follow the desktop theme). "system", "light" and "dark" are the
    /// values of the removed Klassisch look, still accepted and shown as Modern.
    pub theme: String,
    /// Glas: "gradient" (translucent panels over the gradient) or "off" (opaque panels).
    pub transparency: String,
    /// The person has picked a look (or closed the question) once; until then the welcome
    /// dialog asks on start.
    pub look_chosen: bool,
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
    /// Check the links in every answer once it is complete (off: only on click).
    pub check_links: bool,
    /// Hourly hints for concrete open needs (off by default).
    pub proactive: bool,
    /// No hints from this hour …
    pub quiet_start: u8,
    /// … until this hour.
    pub quiet_end: u8,
    /// Look for a new version on GitHub once a day (one anonymous request).
    pub update_check: bool,
    /// New worksheets count points (on by default); off: they count right tasks, and models
    /// are told to leave points out. Each sheet has its own switch afterwards.
    pub worksheet_points: bool,
    /// „Erstellt mit KI“ on exports (#57): `ais-chat` (only content from AIS.chat, the
    /// default, as its terms of use ask), `all` or `off`.
    pub ai_label: String,
    /// Write the prompt along with the label (off: a prompt may hold things not meant for a
    /// handout).
    pub ai_label_prompt: bool,
    /// The owner's own rules for the trust check (#22); changed with `trust.rule_add` and
    /// `trust.rule_remove`.
    pub trust_rules: Vec<TrustRule>,
}

/// What a rule does: let matching actions run without asking, or never let them run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Allow,
    Forbid,
}

/// One of the owner's rules, in their words („Lesen in Obsidian immer“, „nie Mails an Eltern
/// senden“).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrustRule {
    pub id: String,
    pub kind: RuleKind,
    pub text: String,
    /// Only for this connector (id); empty: every connector.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub connector: String,
}

/// Accepted values of the `theme` setting.
pub const THEMES: &[&str] = &["modern", "modern-light", "modern-dark", "glas", "glas-light", "glas-dark", "system", "light", "dark", "omarchy"];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "modern".into(),
            transparency: "gradient".into(),
            look_chosen: false,
            memory: true,
            zoom: 1.0,
            enter_sends: true,
            tools: true,
            trust_check: true,
            auto_allow_covered: true,
            monthly_token_limit: 0,
            learn_skills: true,
            recall: true,
            check_links: false,
            proactive: false,
            quiet_start: 22,
            quiet_end: 7,
            update_check: true,
            worksheet_points: true,
            ai_label: "ais-chat".into(),
            ai_label_prompt: false,
            trust_rules: Vec::new(),
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
