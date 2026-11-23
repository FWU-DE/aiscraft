//! Anthropic Messages API over raw HTTP (there is no official Rust SDK).

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, ChatRequest, Error, Message, ModelInfo, Result, Role, StreamEvent, Usage};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::http::{client, json as read_json, net_err, status_error};
use crate::sse::event_stream;
use crate::{Backend, EventStream};

const API_VERSION: &str = "2026-10-21";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Streaming, so a large cap does not risk HTTP timeouts and does not cut answers short.
const MAX_TOKENS: u32 = 64_000;
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

pub struct Anthropic {
    config: BackendConfig,
    base: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl Anthropic {
    pub fn new(config: BackendConfig, base: String, key: Option<String>) -> Result<Self> {
        Ok(Self { config, base, key: key.filter(|k| !k.trim().is_empty()), http: client(true)? })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let key = self.key.as_deref().ok_or_else(|| Error::auth("Kein API-Schlüssel für Anthropic hinterlegt"))?;
        Ok(self.http.request(method, format!("{}{path}", self.base)).header("x-api-key", key.trim()).header("anthropic-version", API_VERSION))
    }
}

/// Message text plus the text of attached files, which backends without uploads get inline.
pub fn text_with_files(m: &Message) -> String {
    let mut text = m.content.clone();
    for a in &m.attachments {
        if let Some(t) = &a.text {
            text.push_str(&format!("\n\n<datei name=\"{}\">\n{t}\n</datei>", a.name.replace('"', "'")));
        }
    }
    text
}

/// Models that refuse with `stop_reason: "refusal"` and take the server-side fallback.
fn takes_fallback(model: &str) -> bool {
    ["claude-fable-5", "claude-opus-5", "claude-sonnet-5-5", "claude-mythos-5"].iter().any(|p| model.starts_with(p))
}

pub fn body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.messages {
        let (role, blocks): (&str, Vec<Value>) = match m.role {
            Role::System => continue,
            Role::Tool => {
                let r = m.tool_result.as_ref();
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": r.map(|r| r.call_id.clone()).unwrap_or_default(),
                    "content": if m.content.is_empty() { "(leer)".to_string() } else { m.content.clone() },
                    "is_error": r.is_some_and(|r| r.is_error),
                });
                ("user", vec![block])
            }
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !m.content.trim().is_empty() {
                    blocks.push(json!({"type": "text", "text": m.content}));
                }
                for c in &m.tool_calls {
                    blocks.push(json!({"type": "tool_use", "id": c.id, "name": c.name, "input": c.arguments}));
                }
                if blocks.is_empty() {
                    continue;
                }
                ("assistant", blocks)
            }
            Role::User => {
                let mut blocks = Vec::new();
                for a in &m.attachments {
                    if let Some(d) = &a.data_base64 {
                        let kind = if a.mime == "application/pdf" { "document" } else { "image" };
                        blocks.push(json!({"type": kind, "source": {"type": "base64", "media_type": a.mime, "data": d}}));
                    }
                }
                let text = text_with_files(m);
                if !text.trim().is_empty() || blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": if text.trim().is_empty() { "…".to_string() } else { text }}));
                }
                ("user", blocks)
            }
        };
        // The API wants strictly alternating roles; merge consecutive messages of one role.
        if let Some(last) = messages.last_mut()
            && last.get("role").and_then(Value::as_str) == Some(role)
            && let Some(arr) = last.get_mut("content").and_then(Value::as_array_mut)
        {
            arr.extend(blocks);
            continue;
        }
        messages.push(json!({"role": role, "content": blocks}));
    }
    let mut b = json!({"model": req.model, "max_tokens": MAX_TOKENS, "stream": true, "messages": messages});
    if let Some(o) = b.as_object_mut() {
        if let Some(s) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
            o.insert("system".into(), json!(s));
        }
        if takes_fallback(&req.model) {
            o.insert("fallbacks".into(), json!("default"));
        }
        if !req.tools.is_empty() {
            let tools: Vec<Value> =
                req.tools.iter().map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema, "eager_input_streaming": true})).collect();
            o.insert("tools".into(), Value::Array(tools));
        }
    }
    b
}

/// tool_use blocks being streamed, by content block index: (id, name, input JSON so far).
#[derive(Default)]
pub struct ToolAcc {
    blocks: std::collections::BTreeMap<u64, (String, String, String)>,
}

pub fn sse_events_with(event: Option<&str>, data: &str, acc: &mut ToolAcc) -> Result<Vec<StreamEvent>> {
    if data.trim().is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(data).map_err(|e| Error::protocol(format!("Antwort nicht lesbar: {e}")))?;
    let ty = v.get("type").and_then(Value::as_str).or(event).unwrap_or("");
    let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0);
    match ty {
        "content_block_start" if v.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") => {
            let s = |p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
            acc.blocks.insert(idx, (s("/content_block/id"), s("/content_block/name"), String::new()));
            Ok(vec![])
        }
        "content_block_delta" if v.pointer("/delta/type").and_then(Value::as_str) == Some("input_json_delta") => {
            if let Some(b) = acc.blocks.get_mut(&idx) {
                let part = v.pointer("/delta/partial_json").and_then(Value::as_str).unwrap_or("");
                if b.2.len() + part.len() > crate::MAX_TOOL_ARGS {
                    return Err(Error::protocol("Werkzeugaufruf zu groß"));
                }
                b.2.push_str(part);
            }
            Ok(vec![])
        }
        "content_block_stop" => Ok(acc.blocks.remove(&idx).map(|(id, name, args)| vec![StreamEvent::ToolCall { call: crate::tool_call(id, name, &args) }]).unwrap_or_default()),
        "message_delta" if v.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("max_tokens") && !acc.blocks.is_empty() => {
            // A tool call cut off mid-arguments must not run.
            acc.blocks.clear();
            sse_events(event, data)
        }
        _ => sse_events(event, data),
    }
}

pub fn sse_events(event: Option<&str>, data: &str) -> Result<Vec<StreamEvent>> {
    if data.trim().is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(data).map_err(|e| Error::protocol(format!("Antwort nicht lesbar: {e}")))?;
    let ty = v.get("type").and_then(Value::as_str).or(event).unwrap_or("");
    let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    Ok(match ty {
        "message_start" => vec![StreamEvent::Usage { usage: Usage { input_tokens: n("/message/usage/input_tokens"), output_tokens: 0 } }],
        "content_block_delta" => match v.pointer("/delta/text").and_then(Value::as_str) {
            Some(t) if v.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") => vec![StreamEvent::Delta { text: t.to_string() }],
            _ => vec![], // thinking and signature deltas are not shown
        },
        "message_delta" => {
            let mut out = vec![StreamEvent::Usage { usage: Usage { input_tokens: 0, output_tokens: n("/usage/output_tokens") } }];
            match v.pointer("/delta/stop_reason").and_then(Value::as_str) {
                Some("refusal") => out.push(StreamEvent::Delta { text: "\n\n_(Das Modell hat diese Anfrage abgelehnt.)_".into() }),
                Some("max_tokens") => out.push(StreamEvent::Delta { text: "\n\n_(Antwort an der Längengrenze abgeschnitten.)_".into() }),
                _ => {}
            }
            out
        }
        "message_stop" => vec![StreamEvent::Done],
        "error" => {
            let msg = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("Fehler des Servers");
            let kind = v.pointer("/error/type").and_then(Value::as_str).unwrap_or("");
            return Err(if kind == "overloaded_error" || kind == "rate_limit_error" { Error::refused(msg.to_string()) } else { Error::protocol(msg.to_string()) });
        }
        _ => vec![],
    })
}

impl Backend for Anthropic {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { remote_history: false, uploads: false, system_prompt: true, vision: true, tools: true, tools_via_mcp: false }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let n = self.models().await?.len();
            Ok(AccountInfo { display_name: None, detail: Some(format!("{n} Modelle verfügbar")), expires: None })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let resp = self.req(reqwest::Method::GET, "/v1/models?limit=100")?.send().await.map_err(net_err)?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Modelle").await);
            }
            let v = read_json(resp, "Modelle").await?;
            Ok(v.get("data")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let name = m.get("display_name").and_then(Value::as_str).unwrap_or(&id).to_string();
                            Some(ModelInfo { id, name, description: None, vision: true, context_window: m.get("max_input_tokens").and_then(Value::as_u64) })
                        })
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>> {
        Box::pin(async move {
            let mut r = self.req(reqwest::Method::POST, "/v1/messages")?;
            if takes_fallback(&req.model) {
                r = r.header("anthropic-beta", FALLBACK_BETA);
            }
            let resp = r.json(&body(&req)).send().await.map_err(net_err)?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Chat").await);
            }
            let mut acc = ToolAcc::default();
            Ok(event_stream(resp, move |ev| sse_events_with(ev.event.as_deref(), &ev.data, &mut acc)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_events() {
        let d = sse_events(None, r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hallo"}}"#).unwrap();
        assert_eq!(d, vec![StreamEvent::Delta { text: "Hallo".into() }]);
        assert!(sse_events(None, r#"{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"…"}}"#).unwrap().is_empty());
        assert_eq!(sse_events(None, r#"{"type":"message_stop"}"#).unwrap(), vec![StreamEvent::Done]);
        assert!(sse_events(None, r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#).is_err());
    }

    #[test]
    fn streamed_tool_use() {
        let mut acc = ToolAcc::default();
        assert!(sse_events_with(None, r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu1","name":"document_create","input":{}}}"#, &mut acc).unwrap().is_empty());
        sse_events_with(None, r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"title\": \"A"}}"#, &mut acc).unwrap();
        sse_events_with(None, r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"}"}}"#, &mut acc).unwrap();
        let ev = sse_events_with(None, r#"{"type":"content_block_stop","index":1}"#, &mut acc).unwrap();
        let StreamEvent::ToolCall { call } = &ev[0] else { panic!() };
        assert_eq!(call.arguments["title"], "A");
        // Broken JSON never becomes arguments.
        sse_events_with(None, r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"tu2","name":"x"}}"#, &mut acc).unwrap();
        sse_events_with(None, r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"a\": "}}"#, &mut acc).unwrap();
        let ev = sse_events_with(None, r#"{"type":"content_block_stop","index":2}"#, &mut acc).unwrap();
        let StreamEvent::ToolCall { call } = &ev[0] else { panic!() };
        assert!(call.arguments.get("_invalid_json").is_some());
    }

    #[test]
    fn body_merges_roles_and_sets_fallback() {
        let m = |role, c: &str| Message {
            id: "x".into(),
            conversation_id: "c".into(),
            role,
            content: c.into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None, tool_calls: vec![], tool_result: None,
        };
        let req = ChatRequest { conversation_id: "c".into(), model: DEFAULT_MODEL.into(), system: None, messages: vec![m(Role::User, "a"), m(Role::User, "b")], tools: vec![], tool_bridge: None };
        let b = body(&req);
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("temperature").is_none());
    }
}
