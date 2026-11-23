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
}

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
}
