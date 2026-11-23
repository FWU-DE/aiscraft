use serde::{Deserialize, Serialize};

use crate::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    /// The result of a tool call, answering an assistant message's `tool_calls`.
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            _ => None,
        }
    }
}

/// A file attached to a user message. `remote_id` is set once a backend that keeps files
/// server-side (AIS.chat) has accepted the upload; other backends get `text` or `data_base64` inline.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    /// Extracted text for text-like files, sent inline to backends without uploads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Image bytes for vision models, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// The type of an attachment that is a web link (its name is the URL).
pub const LINK_MIME: &str = "text/uri-list";

impl Attachment {
    /// A web link attached to a message. Backends without uploads read it as text.
    pub fn link(url: &str) -> Self {
        Attachment { name: url.to_string(), mime: LINK_MIME.into(), size: 0, text: Some(format!("Link: {url}")), ..Default::default() }
    }
    pub fn is_link(&self) -> bool {
        self.mime == LINK_MIME
    }
}

/// One step of server-side agent work (AIS.chat shows "Websuche", "Datei lesen"…).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActivityStep {
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub failed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ActivityLink>,
    /// A document the server generated (AIS.chat `generateDocument`), offered for saving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<GeneratedDocument>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActivityLink {
    pub title: String,
    pub url: String,
    /// The server could not read this page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GeneratedDocument {
    pub file_name: String,
    pub format: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub role: Role,
    pub content: String,
    pub created_at: Millis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ActivityStep>,
    /// Set when generation failed; the content then holds what arrived before the failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Tools the assistant asked to call in this message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: which call this answers and whether it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultInfo>,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// What happened: "ok", "error", "denied" (set once it ran).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResultInfo {
    pub call_id: String,
    pub name: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub backend_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
    #[serde(default)]
    pub pinned: bool,
    /// What this chat may use besides the conversation itself.
    #[serde(default)]
    pub sources: Sources,
    /// The assistant, character or learning scenario this chat talks to (AIS.chat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<Persona>,
    /// A text chat or an image chat.
    #[serde(default, skip_serializing_if = "ConversationKind::is_chat")]
    pub kind: ConversationKind,
    /// The server's id for this chat when it differs from the local id (AIS.chat image chats,
    /// whose id the server picks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    #[default]
    Chat,
    /// Every message is a prompt for an image model (AIS.chat `generateImageAction`).
    Image,
}

impl ConversationKind {
    pub fn is_chat(&self) -> bool {
        *self == ConversationKind::Chat
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chat" | "text" => Some(ConversationKind::Chat),
            "image" | "image-generation" | "bild" => Some(ConversationKind::Image),
            _ => None,
        }
    }
}

/// What kind of configured partner a chat can talk to on AIS.chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaKind {
    /// A custom assistant ("Assistent", `assistantId`).
    Assistant,
    /// A character, e.g. a historical figure ("Figur", `characterId`).
    Character,
    /// A learning scenario ("Lernszenario", `learningScenarioId`).
    LearningScenario,
}

impl PersonaKind {
    pub const ALL: [PersonaKind; 3] = [PersonaKind::Assistant, PersonaKind::Character, PersonaKind::LearningScenario];
    pub fn as_str(self) -> &'static str {
        match self {
            PersonaKind::Assistant => "assistant",
            PersonaKind::Character => "character",
            PersonaKind::LearningScenario => "learning_scenario",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "assistant" | "assistent" | "custom_gpt" => Some(PersonaKind::Assistant),
            "character" | "figur" => Some(PersonaKind::Character),
            "learning_scenario" | "learning-scenario" | "lernszenario" | "scenario" => Some(PersonaKind::LearningScenario),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PersonaKind::Assistant => "Assistent",
            PersonaKind::Character => "Figur",
            PersonaKind::LearningScenario => "Lernszenario",
        }
    }
    pub fn plural(self) -> &'static str {
        match self {
            PersonaKind::Assistant => "Assistenten",
            PersonaKind::Character => "Figuren",
            PersonaKind::LearningScenario => "Lernszenarien",
        }
    }
}

/// The partner a conversation remembers: enough to send to the right one again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    pub kind: PersonaKind,
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// An assistant, character or learning scenario as the backend lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaInfo {
    pub kind: PersonaKind,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Made by the signed-in person (otherwise shared with them).
    #[serde(default)]
    pub own: bool,
    /// The character's first line, shown before the person writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_message: Option<String>,
    /// The task a learning scenario sets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exercise: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_suggestions: Vec<String>,
    /// The model it was made for, if it says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// A chat's path in AIS.chat's web app (`getConversationPath`).
pub fn ais_conversation_path(persona: Option<&Persona>, conversation_id: &str) -> String {
    match persona {
        Some(p) => {
            let base = match p.kind {
                PersonaKind::Assistant => "assistants",
                PersonaKind::Character => "characters",
                PersonaKind::LearningScenario => "learning-scenarios",
            };
            format!("/{base}/d/{}/{conversation_id}", p.id)
        }
        None => format!("/d/{conversation_id}"),
    }
}

impl PersonaInfo {
    pub fn persona(&self) -> Persona {
        Persona { kind: self.kind, id: self.id.clone(), name: self.name.clone() }
    }
}

/// The sources a chat draws on. All on by default; each can be switched off per chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sources {
    /// Offer tools at all (documents, history search, connectors).
    pub tools: bool,
    /// The owner's memory notes.
    pub memory: bool,
    /// The project's instructions.
    pub project: bool,
    /// Searching the other chats (the history_search tool).
    pub history: bool,
    /// Connectors this chat may use: `None` = every enabled one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectors: Option<Vec<String>>,
}

impl Default for Sources {
    fn default() -> Self {
        Self { tools: true, memory: true, project: true, history: true, connectors: None }
    }
}

impl Sources {
    pub fn allows_connector(&self, id: &str) -> bool {
        self.tools && self.connectors.as_ref().is_none_or(|c| c.iter().any(|x| x == id))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Custom instructions sent as the system prompt of every chat in the project.
    pub instructions: String,
    pub created_at: Millis,
}

/// What kind of artifact: a Markdown document or a whiteboard. Both are CRDTs (yrs) so they can
/// be shared live later without changing their format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Document,
    Board,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Document => "document",
            ArtifactKind::Board => "board",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "document" | "doc" => Some(ArtifactKind::Document),
            "board" | "whiteboard" => Some(ArtifactKind::Board),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ArtifactKind::Document => "Dokument",
            ArtifactKind::Board => "Whiteboard",
        }
    }
}

/// A document or board. Its content (the CRDT state) is stored separately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// One saved version of an artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub artifact_id: String,
    pub version: i64,
    pub created_at: Millis,
    /// Why it was saved: "KI-Überarbeitung", "Manuell", …
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// AIS.chat through the signed-in web session (cookie).
    AisChat,
    /// Any server speaking the OpenAI chat completions API: OpenAI, Ollama, LM Studio,
    /// OpenRouter, vLLM, and AIS.chat's own `/v1` API with an API key.
    OpenAi,
    Anthropic,
    /// The local Claude Code CLI (`claude -p`), signed in with a claude.ai subscription.
    ClaudeCli,
}

impl BackendKind {
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::AisChat => "AIS.chat (Anmeldung)",
            BackendKind::OpenAi => "OpenAI-kompatibel",
            BackendKind::Anthropic => "Anthropic",
            BackendKind::ClaudeCli => "Claude-Abo (lokale CLI)",
        }
    }
    pub fn default_base_url(self) -> &'static str {
        match self {
            BackendKind::AisChat => "https://app.ais-chat.schule",
            BackendKind::OpenAi => "https://api.openai.com/v1",
            BackendKind::Anthropic => "https://api.anthropic.com",
            // For the CLI the "address" is the program to run.
            BackendKind::ClaudeCli => "claude",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ais_chat" | "aischat" | "ais" => Some(BackendKind::AisChat),
            "openai" | "open_ai" => Some(BackendKind::OpenAi),
            "anthropic" => Some(BackendKind::Anthropic),
            "claude_cli" | "claude-cli" | "claude" => Some(BackendKind::ClaudeCli),
            _ => None,
        }
    }
}

/// A configured backend. Secrets (session cookie, API key) are never stored here; they live in
/// the OS keychain under the backend id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendConfig {
    pub id: String,
    pub name: String,
    pub kind: BackendKind,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    /// Send project instructions and memory notes to this backend. Off by default for AIS.chat,
    /// which builds its own system prompt in a school context.
    #[serde(default = "yes")]
    pub send_context: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ModelInfo {
    /// What the backend expects in a request.
    pub id: String,
    /// What a person reads.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub vision: bool,
    /// How many input tokens the model accepts, when the backend says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When the session ends, for cookie sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// Tools and features the backend runs on its own servers (AIS.chat), and whether they are on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<ServerTool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerTool {
    pub id: String,
    pub label: String,
    pub enabled: bool,
}

/// How much of a backend's allowance is used (AIS.chat token points).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    pub used: f64,
    pub max: f64,
}

impl Quota {
    /// Share still available, 0..=1.
    pub fn left(&self) -> f64 {
        if self.max <= 0.0 { 0.0 } else { (1.0 - self.used / self.max).clamp(0.0, 1.0) }
    }
}

/// A conversation that exists on the server (AIS.chat history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteConversation {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// The assistant, character or learning scenario it was held with (name may be empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<Persona>,
    #[serde(default, skip_serializing_if = "ConversationKind::is_chat")]
    pub kind: ConversationKind,
}

/// What a backend can do beyond plain chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub remote_history: bool,
    pub uploads: bool,
    pub system_prompt: bool,
    pub vision: bool,
    /// The backend runs tools the client defines (tool calling).
    pub tools: bool,
    /// The backend calls tools through an MCP server the app provides (the Claude CLI).
    #[serde(default)]
    pub tools_via_mcp: bool,
}

/// Everything a backend needs to answer the next turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// For backends that call tools through MCP themselves (the Claude CLI): where the app's
    /// tool bridge listens and the token for this answer.
    #[serde(skip)]
    pub tool_bridge: Option<ToolBridge>,
    /// The assistant, character or learning scenario to answer as (AIS.chat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<Persona>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolBridge {
    pub url: String,
    pub token: String,
}

/// A prompt for an image model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageRequest {
    pub prompt: String,
    pub model: String,
    /// `quadratic`, `landscape` or `portrait` (AIS.chat's presets).
    pub aspect_ratio: String,
    /// The server's image chat to continue, if there is one already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

/// A generated image.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImageResult {
    #[serde(skip)]
    pub bytes: Vec<u8>,
    pub mime: String,
    /// The server's image chat it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

/// The aspect ratios AIS.chat offers for images.
pub const ASPECT_RATIOS: [(&str, &str); 3] = [("quadratic", "Quadratisch"), ("landscape", "Querformat"), ("portrait", "Hochformat")];

/// One piece of a streamed answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Server-assigned id of the answer (AIS.chat), so later edits refer to the same message.
    MessageId {
        id: String,
    },
    Delta {
        text: String,
    },
    /// The full list of activity steps so far (replaces the previous list).
    Activity {
        steps: Vec<ActivityStep>,
    },
    Usage {
        usage: Usage,
    },
    /// The model wants this tool called (arguments complete).
    ToolCall {
        call: ToolCall,
    },
    Done,
}

/// The first line of a user message, shortened, as a conversation title.
pub fn title_from(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Neuer Chat");
    let mut title: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        title.push('…');
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_first_nonempty_line_capped() {
        assert_eq!(title_from("\n  Hallo Welt \nmehr"), "Hallo Welt");
        assert_eq!(title_from(""), "Neuer Chat");
        let long = "ä".repeat(80);
        assert_eq!(title_from(&long).chars().count(), 61);
    }

    #[test]
    fn stream_event_json_shape() {
        let e = StreamEvent::Delta { text: "x".into() };
        assert_eq!(serde_json::to_string(&e).unwrap(), r#"{"type":"delta","text":"x"}"#);
    }
}
