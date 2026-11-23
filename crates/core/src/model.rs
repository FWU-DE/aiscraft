use serde::{Deserialize, Serialize};

use crate::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Custom instructions sent as the system prompt of every chat in the project.
    pub instructions: String,
    pub created_at: Millis,
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
}

/// A conversation that exists on the server (AIS.chat history).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteConversation {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// What a backend can do beyond plain chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub remote_history: bool,
    pub uploads: bool,
    pub system_prompt: bool,
    pub vision: bool,
}

/// Everything a backend needs to answer the next turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub conversation_id: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<Message>,
}

/// One piece of a streamed answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Server-assigned id of the answer (AIS.chat), so later edits refer to the same message.
    MessageId { id: String },
    Delta { text: String },
    /// The full list of activity steps so far (replaces the previous list).
    Activity { steps: Vec<ActivityStep> },
    Usage { usage: Usage },
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
