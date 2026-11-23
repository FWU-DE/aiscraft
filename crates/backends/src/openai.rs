//! OpenAI chat completions: OpenAI itself, Ollama, LM Studio, OpenRouter, vLLM, and AIS.chat's
//! own `/v1` API (Bearer API key).

use aiscraft_core::{AccountInfo, BackendConfig, Capabilities, ChatRequest, Error, ModelInfo, Result, Role, StreamEvent, Usage};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::http::{client, json as read_json, net_err, status_error};
use crate::sse::event_stream;
use crate::{Backend, EventStream};

pub struct OpenAi {
    config: BackendConfig,
    base: String,
    key: Option<String>,
    http: reqwest::Client,
}

impl OpenAi {
    pub fn new(config: BackendConfig, base: String, key: Option<String>) -> Result<Self> {
        Ok(Self { config, base, key: key.filter(|k| !k.trim().is_empty()), http: client(true)? })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}{path}", self.base));
        match &self.key {
            Some(k) => r.bearer_auth(k.trim()),
            None => r,
        }
    }
}

/// The chat completions request body.
pub fn body(req: &ChatRequest) -> Value {
    let mut messages = Vec::new();
    if let Some(s) = req.system.as_deref().filter(|s| !s.trim().is_empty()) {
        messages.push(json!({"role": "system", "content": s}));
    }
    for m in &req.messages {
        match m.role {
            Role::Tool => {
                let id = m.tool_result.as_ref().map(|r| r.call_id.clone()).unwrap_or_default();
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": m.content}));
                continue;
            }
            Role::Assistant if !m.tool_calls.is_empty() => {
                let calls: Vec<Value> = m
                    .tool_calls
                    .iter()
                    .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments.to_string()}}))
                    .collect();
                let content = if m.content.is_empty() { Value::Null } else { json!(m.content) };
                messages.push(json!({"role": "assistant", "content": content, "tool_calls": calls}));
                continue;
            }
            Role::Assistant if m.content.is_empty() => continue,
            _ => {}
        }
        let images: Vec<_> = m.attachments.iter().filter_map(|a| a.data_base64.as_ref().map(|d| (a, d))).collect();
        let text = crate::anthropic::text_with_files(m);
        if images.is_empty() {
            messages.push(json!({"role": m.role.as_str(), "content": text}));
        } else {
            let mut parts = vec![json!({"type": "text", "text": text})];
            for (a, d) in images {
                parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{};base64,{d}", a.mime)}}));
            }
            messages.push(json!({"role": m.role.as_str(), "content": parts}));
        }
    }
    let mut b = json!({"model": req.model, "messages": messages, "stream": true, "stream_options": {"include_usage": true}});
    if !req.tools.is_empty()
        && let Some(o) = b.as_object_mut()
    {
        let tools: Vec<Value> = req.tools.iter().map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}})).collect();
        o.insert("tools".into(), Value::Array(tools));
    }
    b
}

/// Tool calls being streamed, by index: (id, name, arguments so far).
#[derive(Default)]
pub struct ToolAcc {
    calls: std::collections::BTreeMap<u64, (String, String, String)>,
}

impl ToolAcc {
    fn take(&mut self) -> Vec<StreamEvent> {
        std::mem::take(&mut self.calls)
            .into_values()
            .filter(|(_, name, _)| !name.is_empty())
            .map(|(id, name, args)| StreamEvent::ToolCall { call: crate::tool_call(id, name, &args) })
            .collect()
    }
}

/// Map one streamed chunk to events (without tool-call state; for tests).
#[cfg(test)]
pub fn chunk_events(data: &str) -> Result<Vec<StreamEvent>> {
    chunk_events_with(data, &mut ToolAcc::default())
}

pub fn chunk_events_with(data: &str, acc: &mut ToolAcc) -> Result<Vec<StreamEvent>> {
    if data.trim() == "[DONE]" {
        let mut out = acc.take();
        out.push(StreamEvent::Done);
        return Ok(out);
    }
    let v: Value = serde_json::from_str(data).map_err(|e| Error::protocol(format!("Antwort nicht lesbar: {e}")))?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).unwrap_or("Fehler des Servers");
        return Err(Error::refused(msg.to_string()));
    }
    let mut out = Vec::new();
    if let Some(choice) = v.get("choices").and_then(Value::as_array).and_then(|a| a.first()) {
        if let Some(t) = choice.pointer("/delta/content").and_then(Value::as_str).filter(|t| !t.is_empty()) {
            out.push(StreamEvent::Delta { text: t.to_string() });
        }
        for tc in choice.pointer("/delta/tool_calls").and_then(Value::as_array).into_iter().flatten() {
            let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
            let entry = acc.calls.entry(idx).or_default();
            if let Some(id) = tc.get("id").and_then(Value::as_str) {
                entry.0 = id.to_string();
            }
            if let Some(n) = tc.pointer("/function/name").and_then(Value::as_str) {
                entry.1.push_str(n);
            }
            if let Some(a) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                if entry.2.len() + a.len() > crate::MAX_TOOL_ARGS {
                    return Err(Error::protocol("Werkzeugaufruf zu groß"));
                }
                entry.2.push_str(a);
            }
        }
        match choice.get("finish_reason").and_then(Value::as_str) {
            Some("content_filter") => out.push(StreamEvent::Delta { text: "\n\n_(Vom Inhaltsfilter des Servers beendet.)_".into() }),
            Some("tool_calls") => out.extend(acc.take()),
            _ => {}
        }
    }
    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        out.push(StreamEvent::Usage { usage: Usage { input_tokens: n("prompt_tokens"), output_tokens: n("completion_tokens") } });
    }
    Ok(out)
}

impl Backend for OpenAi {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { remote_history: false, uploads: false, system_prompt: true, vision: true, tools: true, tools_via_mcp: false }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let n = self.models().await?.len();
            Ok(AccountInfo { display_name: None, detail: Some(format!("{n} Modelle verfügbar")), expires: None, server_tools: vec![] })
        })
    }

    fn models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let resp = self.req(reqwest::Method::GET, "/models").send().await.map_err(net_err)?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Modelle").await);
            }
            let v = read_json(resp, "Modelle").await?;
            let mut models: Vec<ModelInfo> = v
                .get("data")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let name = m.get("name").and_then(Value::as_str).unwrap_or(&id).to_string();
                            let context_window = ["context_length", "context_window", "max_model_len", "max_context_length"].iter().find_map(|k| m.get(*k).and_then(Value::as_u64));
                            Some(ModelInfo { id, name, description: None, vision: false, context_window })
                        })
                        .collect()
                })
                .unwrap_or_default();
            models.sort_by_key(|m| m.name.to_lowercase());
            Ok(models)
        })
    }

    fn chat(&self, req: ChatRequest) -> BoxFuture<'_, Result<EventStream>> {
        Box::pin(async move {
            let resp = self.req(reqwest::Method::POST, "/chat/completions").json(&body(&req)).send().await.map_err(net_err)?;
            if !resp.status().is_success() {
                return Err(status_error(resp, "Chat").await);
            }
            let mut acc = ToolAcc::default();
            Ok(event_stream(resp, move |ev| chunk_events_with(&ev.data, &mut acc)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiscraft_core::{Attachment, Message};

    fn msg(role: Role, content: &str) -> Message {
        Message {
            id: "m".into(),
            conversation_id: "c".into(),
            role,
            content: content.into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None, tool_calls: vec![], tool_result: None,
        }
    }

    #[test]
    fn body_has_system_and_images() {
        let mut u = msg(Role::User, "Was ist das?");
        u.attachments.push(Attachment { name: "a.png".into(), mime: "image/png".into(), data_base64: Some("QUJD".into()), ..Default::default() });
        let req = ChatRequest { conversation_id: "c".into(), model: "gpt".into(), system: Some("Sei kurz".into()), messages: vec![u, msg(Role::Assistant, "")], tools: vec![], tool_bridge: None };
        let b = body(&req);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/png;base64,QUJD");
        assert_eq!(b["messages"].as_array().unwrap().len(), 2, "empty assistant placeholder is dropped");
    }

    #[test]
    fn streamed_tool_calls() {
        let mut acc = ToolAcc::default();
        let a = chunk_events_with(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"history_search","arguments":"{\"qu"}}]}}]}"#, &mut acc).unwrap();
        assert!(a.is_empty());
        chunk_events_with(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ery\":\"bio\"}"}}]}}]}"#, &mut acc).unwrap();
        let done = chunk_events_with(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#, &mut acc).unwrap();
        let StreamEvent::ToolCall { call } = &done[0] else { panic!() };
        assert_eq!(call.name, "history_search");
        assert_eq!(call.arguments["query"], "bio");
    }

    #[test]
    fn chunks() {
        assert_eq!(chunk_events(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#).unwrap(), vec![StreamEvent::Delta { text: "Hi".into() }]);
        assert_eq!(chunk_events("[DONE]").unwrap(), vec![StreamEvent::Done]);
        assert!(chunk_events(r#"{"error":{"message":"quota"}}"#).is_err());
        assert!(matches!(chunk_events(r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#).unwrap()[0], StreamEvent::Usage { .. }));
    }
}
