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
        if m.role == Role::Assistant && m.content.is_empty() {
            continue;
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
    json!({"model": req.model, "messages": messages, "stream": true, "stream_options": {"include_usage": true}})
}

/// Map one streamed chunk to events.
pub fn chunk_events(data: &str) -> Result<Vec<StreamEvent>> {
    if data.trim() == "[DONE]" {
        return Ok(vec![StreamEvent::Done]);
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
        if choice.get("finish_reason").and_then(Value::as_str) == Some("content_filter") {
            out.push(StreamEvent::Delta { text: "\n\n_(Vom Inhaltsfilter des Servers beendet.)_".into() });
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
        Capabilities { remote_history: false, uploads: false, system_prompt: true, vision: true }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let n = self.models().await?.len();
            Ok(AccountInfo { display_name: None, detail: Some(format!("{n} Modelle verfügbar")), expires: None })
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
                            Some(ModelInfo { id, name, description: None, vision: false })
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
            Ok(event_stream(resp, |ev| chunk_events(&ev.data)))
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
            usage: None,
        }
    }

    #[test]
    fn body_has_system_and_images() {
        let mut u = msg(Role::User, "Was ist das?");
        u.attachments.push(Attachment { name: "a.png".into(), mime: "image/png".into(), data_base64: Some("QUJD".into()), ..Default::default() });
        let req = ChatRequest { conversation_id: "c".into(), model: "gpt".into(), system: Some("Sei kurz".into()), messages: vec![u, msg(Role::Assistant, "")] };
        let b = body(&req);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/png;base64,QUJD");
        assert_eq!(b["messages"].as_array().unwrap().len(), 2, "empty assistant placeholder is dropped");
    }

    #[test]
    fn chunks() {
        assert_eq!(chunk_events(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#).unwrap(), vec![StreamEvent::Delta { text: "Hi".into() }]);
        assert_eq!(chunk_events("[DONE]").unwrap(), vec![StreamEvent::Done]);
        assert!(chunk_events(r#"{"error":{"message":"quota"}}"#).is_err());
        assert!(matches!(chunk_events(r#"{"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#).unwrap()[0], StreamEvent::Usage { .. }));
    }
}
