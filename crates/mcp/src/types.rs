//! What a server tells us, parsed leniently from untrusted JSON.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The server's answer to `initialize`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub title: Option<String>,
    /// The protocol version the server agreed on.
    pub protocol_version: String,
    /// The server's `capabilities` object as sent.
    pub capabilities: Value,
}

impl ServerInfo {
    pub(crate) fn parse(result: &Value) -> Self {
        let info = result.get("serverInfo");
        Self {
            name: info.and_then(|i| string(i, "name")).unwrap_or_default(),
            version: info.and_then(|i| string(i, "version")).unwrap_or_default(),
            title: info.and_then(|i| string(i, "title")),
            protocol_version: string(result, "protocolVersion").unwrap_or_default(),
            capabilities: result.get("capabilities").filter(|c| c.is_object()).cloned().unwrap_or_else(|| Value::Object(Map::new())),
        }
    }

    /// Whether the server says it sends `notifications/tools/list_changed`.
    pub fn tools_list_changed(&self) -> bool {
        self.capabilities.pointer("/tools/listChanged").and_then(Value::as_bool).unwrap_or(false)
    }
}

/// One tool a server offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// JSON Schema of the arguments; always an object (`{"type":"object"}` if the server sent none).
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub annotations: ToolAnnotations,
    /// The interactive view the tool comes with (MCP Apps, SEP-1865), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<ToolUi>,
}

/// A tool's view (`_meta.ui`): a `ui://` resource, and who may call the tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUi {
    pub resource_uri: String,
    /// `model` and/or `app`; empty = both.
    pub visibility: Vec<String>,
}

impl ToolUi {
    /// The model may see and call the tool.
    pub fn for_model(&self) -> bool {
        self.visibility.is_empty() || self.visibility.iter().any(|v| v == "model")
    }

    /// The view may call the tool.
    pub fn for_app(&self) -> bool {
        self.visibility.is_empty() || self.visibility.iter().any(|v| v == "app")
    }

    fn parse(v: &Value) -> Option<Self> {
        let meta = v.get("_meta")?;
        let uri = meta.pointer("/ui/resourceUri").or_else(|| meta.get("ui/resourceUri")).and_then(Value::as_str)?;
        if !uri.starts_with("ui://") || uri.len() > 2048 || uri.chars().any(char::is_control) {
            return None;
        }
        let visibility = meta
            .pointer("/ui/visibility")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).filter(|x| matches!(*x, "model" | "app")).map(str::to_string).collect())
            .unwrap_or_default();
        Some(Self { resource_uri: uri.to_string(), visibility })
    }
}

/// What `resources/read` returned for one address.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    /// Base64 as sent.
    pub blob: Option<String>,
    /// The content's `_meta` (an object, `{}` when none).
    pub meta: Value,
}

impl ResourceContents {
    pub(crate) fn parse(result: &Value, uri: &str) -> Option<Self> {
        let items = result.get("contents")?.as_array()?;
        let item = items.iter().find(|c| c.get("uri").and_then(Value::as_str) == Some(uri)).or_else(|| items.first())?;
        Some(Self {
            uri: string(item, "uri").unwrap_or_default(),
            mime_type: string(item, "mimeType"),
            text: string(item, "text"),
            blob: string(item, "blob"),
            meta: item.get("_meta").filter(|m| m.is_object()).cloned().unwrap_or_else(|| Value::Object(Map::new())),
        })
    }
}

/// Hints about a tool's behaviour. They come from the server and are not guarantees.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    pub title: Option<String>,
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: Option<bool>,
}

impl ToolAnnotations {
    /// True only if the server explicitly marked the tool read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only_hint == Some(true)
    }
}

impl Tool {
    /// `None` for entries without a usable name.
    pub(crate) fn parse(v: &Value) -> Option<Self> {
        let name = string(v, "name").filter(|n| !n.trim().is_empty())?;
        let a = v.get("annotations");
        let annotations = ToolAnnotations {
            title: a.and_then(|a| string(a, "title")),
            read_only_hint: a.and_then(|a| a.get("readOnlyHint")).and_then(Value::as_bool),
            destructive_hint: a.and_then(|a| a.get("destructiveHint")).and_then(Value::as_bool),
            idempotent_hint: a.and_then(|a| a.get("idempotentHint")).and_then(Value::as_bool),
            open_world_hint: a.and_then(|a| a.get("openWorldHint")).and_then(Value::as_bool),
        };
        let input_schema = match v.get("inputSchema") {
            Some(s @ Value::Object(_)) => s.clone(),
            _ => serde_json::json!({ "type": "object" }),
        };
        Some(Self {
            title: string(v, "title").or_else(|| annotations.title.clone()),
            description: string(v, "description"),
            output_schema: v.get("outputSchema").filter(|s| s.is_object()).cloned(),
            ui: ToolUi::parse(v),
            name,
            input_schema,
            annotations,
        })
    }

    /// The name to show people.
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// What `tools/call` returned.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    /// The tool ran but failed; the content says why. The model should see this, not a crash.
    pub is_error: bool,
    pub structured_content: Option<Value>,
}

impl ToolResult {
    pub(crate) fn parse(v: &Value) -> Self {
        let content = v.get("content").and_then(Value::as_array).map(|items| items.iter().map(Content::parse).collect()).unwrap_or_default();
        Self {
            content,
            is_error: v.get("isError").and_then(Value::as_bool).unwrap_or(false),
            structured_content: v.get("structuredContent").filter(|s| !s.is_null()).cloned(),
        }
    }

    /// All text blocks joined by newlines; the structured content as JSON if there is no text.
    pub fn text(&self) -> String {
        let texts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                Content::Resource(EmbeddedResource { text: Some(t), .. }) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        if texts.is_empty() { self.structured_content.as_ref().map(Value::to_string).unwrap_or_default() } else { texts.join("\n") }
    }

    /// What the model gets: the text blocks, plus `structuredContent` as compact JSON when the text
    /// does not already carry it. Many servers send only a summary as text („1 Datei erstellt“) and
    /// put links and ids in `structuredContent`; without it the model cannot act on them (Nod #1476).
    /// A text block that is the same JSON as `structuredContent` is dropped, so the data comes once.
    /// The JSON is capped at [`MAX_STRUCTURED_CHARS`].
    pub fn model_text(&self) -> String {
        let Some(structured) = &self.structured_content else { return self.text() };
        let repeats = |t: &str| serde_json::from_str::<Value>(t.trim()).is_ok_and(|v| &v == structured);
        let parts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                Content::Resource(EmbeddedResource { text: Some(t), .. }) => Some(t.as_str()),
                _ => None,
            })
            .filter(|t| !repeats(t))
            .collect();
        let mut out = parts.join("\n");
        let json = structured.to_string();
        // A text that already holds the same compact JSON needs no second copy.
        if out.contains(&json) {
            return out;
        }
        let mut capped: String = json.chars().take(MAX_STRUCTURED_CHARS).collect();
        if json.chars().count() > MAX_STRUCTURED_CHARS {
            capped.push_str(" … (gekürzt)");
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&capped);
        out
    }
}

/// How much of `structuredContent` the model sees next to the text (characters).
pub const MAX_STRUCTURED_CHARS: usize = 24 * 1024;

/// One content block of a tool result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Content {
    Text {
        text: String,
    },
    /// `data` is base64 as the server sent it; not validated, decode with care.
    Image {
        data: String,
        mime_type: String,
    },
    Audio {
        data: String,
        mime_type: String,
    },
    Resource(EmbeddedResource),
    ResourceLink {
        uri: String,
        name: Option<String>,
        title: Option<String>,
        description: Option<String>,
        mime_type: Option<String>,
    },
    /// A block type this client does not know, kept as sent.
    Unknown {
        raw: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedResource {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    /// Base64 as sent.
    pub blob: Option<String>,
}

impl Content {
    fn parse(v: &Value) -> Self {
        match v.get("type").and_then(Value::as_str) {
            Some("text") => Content::Text { text: string(v, "text").unwrap_or_default() },
            Some("image") => Content::Image { data: string(v, "data").unwrap_or_default(), mime_type: string(v, "mimeType").unwrap_or_default() },
            Some("audio") => Content::Audio { data: string(v, "data").unwrap_or_default(), mime_type: string(v, "mimeType").unwrap_or_default() },
            Some("resource") => {
                let r = v.get("resource").unwrap_or(&Value::Null);
                Content::Resource(EmbeddedResource {
                    uri: string(r, "uri").unwrap_or_default(),
                    mime_type: string(r, "mimeType"),
                    text: string(r, "text"),
                    blob: string(r, "blob"),
                })
            }
            Some("resource_link") => Content::ResourceLink {
                uri: string(v, "uri").unwrap_or_default(),
                name: string(v, "name"),
                title: string(v, "title"),
                description: string(v, "description"),
                mime_type: string(v, "mimeType"),
            },
            _ => Content::Unknown { raw: v.clone() },
        }
    }
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_parsing_is_lenient() {
        let t = Tool::parse(&json!({"name": "read", "inputSchema": "nonsense", "annotations": {"readOnlyHint": true, "title": "Lesen"}})).unwrap();
        assert_eq!(t.input_schema, json!({"type": "object"}));
        assert!(t.annotations.is_read_only());
        assert_eq!(t.display_name(), "Lesen");
        assert!(Tool::parse(&json!({"name": ""})).is_none());
        assert!(Tool::parse(&json!(42)).is_none());
        let plain = Tool::parse(&json!({"name": "x", "annotations": {"readOnlyHint": "yes"}})).unwrap();
        assert!(!plain.annotations.is_read_only());
        assert!(plain.ui.is_none());
    }

    #[test]
    fn tool_views() {
        let t = Tool::parse(&json!({"name": "k", "_meta": {"ui": {"resourceUri": "ui://k/view.html", "visibility": ["app", "x"]}}})).unwrap();
        let ui = t.ui.unwrap();
        assert_eq!(ui.resource_uri, "ui://k/view.html");
        assert!(ui.for_app() && !ui.for_model());
        let legacy = Tool::parse(&json!({"name": "k", "_meta": {"ui/resourceUri": "ui://k/a"}})).unwrap().ui.unwrap();
        assert!(legacy.for_app() && legacy.for_model());
        assert!(Tool::parse(&json!({"name": "k", "_meta": {"ui": {"resourceUri": "https://evil.example/x"}}})).unwrap().ui.is_none());
        let r = ResourceContents::parse(&json!({"contents": [{"uri": "ui://a", "text": "x"}, {"uri": "ui://b", "mimeType": "text/html;profile=mcp-app", "text": "<p>", "_meta": {"ui": {}}}]}), "ui://b").unwrap();
        assert_eq!(r.text.as_deref(), Some("<p>"));
        assert!(r.meta.get("ui").is_some());
        assert!(ResourceContents::parse(&json!({"contents": "x"}), "ui://b").is_none());
    }

    #[test]
    fn result_blocks() {
        let r = ToolResult::parse(&json!({
            "content": [
                {"type": "text", "text": "a"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"},
                {"type": "resource", "resource": {"uri": "file:///x", "text": "b"}},
                {"type": "resource_link", "uri": "file:///y", "name": "y"},
                {"type": "hologram"},
                "garbage"
            ],
            "isError": true,
            "structuredContent": {"n": 1}
        }));
        assert!(r.is_error);
        assert_eq!(r.content.len(), 6);
        assert_eq!(r.text(), "a\nb");
        assert!(matches!(&r.content[1], Content::Image { mime_type, .. } if mime_type == "image/png"));
        assert!(matches!(&r.content[3], Content::ResourceLink { name: Some(n), .. } if n == "y"));
        assert!(matches!(&r.content[4], Content::Unknown { .. }));
        assert_eq!(ToolResult::parse(&json!({"structuredContent": {"n": 1}})).text(), r#"{"n":1}"#);
        assert_eq!(ToolResult::parse(&json!("nope")), ToolResult::default());
    }

    #[test]
    fn the_model_gets_structured_content_next_to_a_summary() {
        let job = json!({"status": "ok", "files": [{"id": "f-17", "url": "https://files.example/f-17"}]});
        // Summary text: the model gets the summary and the data.
        let summary = ToolResult::parse(&json!({"content": [{"type": "text", "text": "Fertig, 1 Datei."}], "structuredContent": job}));
        assert_eq!(summary.text(), "Fertig, 1 Datei.");
        let m = summary.model_text();
        assert!(m.starts_with("Fertig, 1 Datei.\n"), "{m}");
        assert!(m.contains("https://files.example/f-17") && m.contains("f-17"), "{m}");
        // The text is the same JSON (pretty): it comes once, compact.
        let same = ToolResult::parse(&json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&job).unwrap()}], "structuredContent": job}));
        assert_eq!(same.model_text(), job.to_string());
        // The text already holds the compact JSON: nothing is added.
        let inside = ToolResult::parse(&json!({"content": [{"type": "text", "text": format!("Ergebnis: {job}")}], "structuredContent": job}));
        assert_eq!(inside.model_text(), format!("Ergebnis: {job}"));
        // No structured content: as before.
        let plain = ToolResult::parse(&json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}));
        assert_eq!(plain.model_text(), "a\nb");
        // Only structured content: the JSON.
        assert_eq!(ToolResult::parse(&json!({"structuredContent": {"n": 1}})).model_text(), r#"{"n":1}"#);
        // Huge structured content is capped.
        let big = ToolResult::parse(&json!({"content": [{"type": "text", "text": "viel"}], "structuredContent": {"x": "y".repeat(100_000)}}));
        let m = big.model_text();
        assert!(m.chars().count() < MAX_STRUCTURED_CHARS + 100 && m.ends_with("(gekürzt)"), "{}", m.len());
    }
}
