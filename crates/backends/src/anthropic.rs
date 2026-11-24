//! Anthropic Messages API over raw HTTP (there is no official Rust SDK).

use std::sync::{Arc, Mutex, PoisonError};

use aiscraft_core::{
    AccountInfo, ActivityLink, ActivityStep, BackendConfig, Capabilities, ChatRequest, Error, Message, ModelInfo, Result, Role, StreamEvent, Usage,
};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::http::{attributed, client, json as read_json, net_err, status_error};
use crate::sse::event_stream;
use crate::{Backend, EventStream};

const API_VERSION: &str = "2026-10-21";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
/// Streaming, so a large cap does not risk HTTP timeouts and does not cut answers short.
const MAX_TOKENS: u32 = 64_000;
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
/// Searches and page reads per answer, so one question cannot run up the bill.
const WEB_MAX_USES: u32 = 5;
/// How often an answer that the server paused (`pause_turn`) is continued.
const MAX_CONTINUATIONS: u8 = 3;
/// The answer's content kept for a continuation; beyond this it is not continued.
const MAX_KEPT_BYTES: usize = 8 * 1024 * 1024;

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
        let url = format!("{}{path}", self.base);
        Ok(attributed(self.http.request(method, &url), &url).header("x-api-key", key.trim()).header("anthropic-version", API_VERSION))
    }
}

/// Message text plus the text of attached files, which backends without uploads get inline.
pub fn text_with_files(m: &Message) -> String {
    let mut text = m.content.clone();
    for a in &m.attachments {
        if a.is_link() {
            text.push_str(&format!("\n\nLink: {}", a.name));
        } else if let Some(t) = &a.text {
            text.push_str(&format!("\n\n<datei name=\"{}\">\n{t}\n</datei>", a.name.replace('"', "'")));
        }
    }
    text
}

/// Models that refuse with `stop_reason: "refusal"` and take the server-side fallback.
fn takes_fallback(model: &str) -> bool {
    ["claude-fable-5", "claude-opus-5", "claude-sonnet-5-5", "claude-mythos-5"].iter().any(|p| model.starts_with(p))
}

/// Models that take the web tools with dynamic filtering; older ones get the basic versions.
fn dynamic_web_tools(model: &str) -> bool {
    ["claude-fable", "claude-mythos", "claude-opus-5", "claude-opus-4-6", "claude-opus-4-7", "claude-opus-4-8", "claude-sonnet-5", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// Claude's own web search and page reader (server tools: they run at Anthropic).
fn web_tools(model: &str) -> [Value; 2] {
    let (search, fetch) = if dynamic_web_tools(model) { ("web_search_20260209", "web_fetch_20260209") } else { ("web_search_20250305", "web_fetch_20250910") };
    [
        json!({"type": search, "name": "web_search", "max_uses": WEB_MAX_USES}),
        json!({"type": fetch, "name": "web_fetch", "max_uses": WEB_MAX_USES, "citations": {"enabled": true}, "max_content_tokens": 50_000}),
    ]
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
        let mut tools: Vec<Value> = req
            .tools
            .iter()
            // Our own tools never take the names of the server tools.
            .filter(|t| !req.web || (t.name != "web_search" && t.name != "web_fetch"))
            .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema, "eager_input_streaming": true}))
            .collect();
        if req.web {
            tools.extend(web_tools(&req.model));
        }
        if !tools.is_empty() {
            o.insert("tools".into(), Value::Array(tools));
        }
    }
    b
}

/// tool_use blocks being streamed, by content block index: (id, name, input JSON so far), plus
/// what the web tools did and the answer's content, for continuing a paused answer.
#[derive(Default)]
pub struct ToolAcc {
    blocks: std::collections::BTreeMap<u64, (String, String, String)>,
    /// server_tool_use blocks being streamed: (id, name, input JSON so far).
    server: std::collections::BTreeMap<u64, (String, String, String)>,
    /// Searches and page reads so far, shown as work steps; cited pages are marked.
    steps: Vec<ActivityStep>,
    /// server tool id -> index in `steps`.
    step_of: std::collections::HashMap<String, usize>,
    /// This response's content blocks, rebuilt from the stream, by index.
    content: std::collections::BTreeMap<u64, Value>,
    /// Content of earlier responses of the same answer (before a `pause_turn`).
    prior: Vec<Value>,
    kept_bytes: usize,
    /// The server paused the answer (`pause_turn`); it can be continued.
    paused: bool,
}

impl ToolAcc {
    fn activity(&self) -> Vec<StreamEvent> {
        vec![StreamEvent::Activity { steps: self.steps.clone() }]
    }

    fn keep(&mut self, n: usize) {
        self.kept_bytes = self.kept_bytes.saturating_add(n);
    }

    /// Mark a cited page; a page no step lists yet goes into a "Quellen" step.
    fn cite(&mut self, url: &str, title: &str) -> bool {
        if url.is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            return false;
        }
        let mut found = false;
        for l in self.steps.iter_mut().flat_map(|s| s.links.iter_mut()).filter(|l| l.url == url) {
            if l.cited {
                return false;
            }
            l.cited = true;
            found = true;
        }
        if found {
            return true;
        }
        let link = ActivityLink {
            title: if title.trim().is_empty() { url.to_string() } else { title.chars().take(200).collect() },
            url: url.to_string(),
            failed: false,
            cited: true,
        };
        match self.steps.iter_mut().find(|s| s.label == "Quellen") {
            Some(s) if s.links.len() < 50 => s.links.push(link),
            Some(_) => return false,
            None => self.steps.push(ActivityStep { label: "Quellen".into(), done: true, links: vec![link], ..Default::default() }),
        }
        true
    }

    /// A server tool call is complete: show it as a step.
    fn server_call(&mut self, id: String, name: &str, input: &Value) {
        let s = |k: &str| input.get(k).and_then(Value::as_str).map(|x| x.chars().take(300).collect::<String>());
        let step = match name {
            "web_search" => ActivityStep { label: "Websuche".into(), detail: s("query"), ..Default::default() },
            "web_fetch" => ActivityStep { label: "Webseite lesen".into(), detail: s("url"), ..Default::default() },
            _ => return,
        };
        if self.steps.len() < 100 {
            self.step_of.insert(id, self.steps.len());
            self.steps.push(step);
        }
    }

    /// A web_search_tool_result or web_fetch_tool_result block.
    fn server_result(&mut self, block: &Value) {
        let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
        let Some(step) = self.step_of.get(id).and_then(|i| self.steps.get_mut(*i)) else { return };
        step.done = true;
        let content = block.get("content").unwrap_or(&Value::Null);
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").chars().take(200).collect::<String>();
        let web = |u: &str| u.starts_with("https://") || u.starts_with("http://");
        match content {
            // Search results: a list. An error is an object.
            Value::Array(results) => {
                for r in results.iter().take(20) {
                    let url = text(r, "url");
                    if web(&url) {
                        let title = text(r, "title");
                        step.links.push(ActivityLink { title: if title.is_empty() { url.clone() } else { title }, url, failed: false, cited: false });
                    }
                }
            }
            Value::Object(_) if content.get("type").and_then(Value::as_str) == Some("web_fetch_result") => {
                let url = text(content, "url");
                if web(&url) {
                    let title = content.pointer("/content/title").and_then(Value::as_str).unwrap_or("").chars().take(200).collect::<String>();
                    // A page Claude read for the answer counts as a source.
                    step.links.push(ActivityLink { title: if title.is_empty() { url.clone() } else { title }, url, failed: false, cited: true });
                }
            }
            _ => {
                step.failed = true;
                let code = content.get("error_code").and_then(Value::as_str).unwrap_or("unbekannt");
                step.detail = Some(format!("{} – nicht möglich ({})", step.detail.clone().unwrap_or_default(), web_error(code)));
            }
        }
    }

    /// Everything the answer holds so far, as the assistant content of a continuation.
    fn assistant_content(&self) -> Vec<Value> {
        self.prior
            .iter()
            .chain(self.content.values())
            .filter(|b| b.get("type").and_then(Value::as_str) != Some("text") || b.get("text").and_then(Value::as_str).is_some_and(|t| !t.is_empty()))
            .cloned()
            .collect()
    }

    /// Start the next response of a paused answer: its blocks are numbered from 0 again.
    fn next_response(&mut self) {
        let done: Vec<Value> = std::mem::take(&mut self.content).into_values().collect();
        self.prior.extend(done);
        self.server.clear();
        self.blocks.clear();
        self.paused = false;
    }
}

fn web_error(code: &str) -> &'static str {
    match code {
        "max_uses_exceeded" => "zu viele Suchen für eine Antwort",
        "too_many_requests" => "zu viele Anfragen, später erneut",
        "invalid_input" | "invalid_tool_input" | "url_not_accessible" | "url_not_allowed" => "Adresse nicht erreichbar",
        "url_too_long" => "Adresse zu lang",
        "query_too_long" => "Suchanfrage zu lang",
        "unsupported_content_type" => "Inhalt nicht lesbar",
        _ => "Fehler beim Server",
    }
}

/// Rebuild the response content from a stream event, for continuing a paused answer.
fn track_content(v: &Value, ty: &str, idx: u64, acc: &mut ToolAcc) {
    if acc.kept_bytes > MAX_KEPT_BYTES {
        return;
    }
    match ty {
        "content_block_start" => {
            if let Some(b) = v.get("content_block") {
                acc.keep(b.to_string().len());
                acc.content.insert(idx, b.clone());
            }
        }
        "content_block_delta" => {
            let Some(b) = acc.content.get_mut(&idx) else { return };
            let d = v.get("delta").unwrap_or(&Value::Null);
            let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let mut added = 0;
            if let Some(o) = b.as_object_mut() {
                let append = |o: &mut serde_json::Map<String, Value>, k: &str, part: String| {
                    let cur = o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    o.insert(k.into(), Value::String(cur + &part));
                };
                match d.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        added = s("text").len();
                        append(o, "text", s("text"));
                    }
                    "thinking_delta" => {
                        added = s("thinking").len();
                        append(o, "thinking", s("thinking"));
                    }
                    "signature_delta" => {
                        added = s("signature").len();
                        o.insert("signature".into(), Value::String(s("signature")));
                    }
                    "citations_delta" => {
                        if let Some(c) = d.get("citation") {
                            added = c.to_string().len();
                            match o.get_mut("citations").and_then(Value::as_array_mut) {
                                Some(a) => a.push(c.clone()),
                                None => {
                                    o.insert("citations".into(), Value::Array(vec![c.clone()]));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            acc.keep(added);
        }
        _ => {}
    }
}

pub fn sse_events_with(event: Option<&str>, data: &str, acc: &mut ToolAcc) -> Result<Vec<StreamEvent>> {
    if data.trim().is_empty() {
        return Ok(vec![]);
    }
    let v: Value = serde_json::from_str(data).map_err(|e| Error::protocol(format!("Antwort nicht lesbar: {e}")))?;
    let ty = v.get("type").and_then(Value::as_str).or(event).unwrap_or("");
    let idx = v.get("index").and_then(Value::as_u64).unwrap_or(0);
    track_content(&v, ty, idx, acc);
    let block_type = v.pointer("/content_block/type").and_then(Value::as_str).unwrap_or("");
    match ty {
        "content_block_start" if block_type == "server_tool_use" => {
            let s = |p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
            acc.server.insert(idx, (s("/content_block/id"), s("/content_block/name"), String::new()));
            Ok(vec![])
        }
        "content_block_start" if block_type == "web_search_tool_result" || block_type == "web_fetch_tool_result" => {
            if let Some(b) = v.get("content_block") {
                acc.server_result(b);
            }
            Ok(acc.activity())
        }
        "content_block_start" if block_type == "text" => {
            let mut changed = false;
            for c in v.pointer("/content_block/citations").and_then(Value::as_array).into_iter().flatten() {
                let s = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                changed |= acc.cite(&s("url"), &s("title"));
            }
            Ok(if changed { acc.activity() } else { vec![] })
        }
        "content_block_delta" if v.pointer("/delta/type").and_then(Value::as_str) == Some("citations_delta") => {
            let s = |k: &str| v.pointer(&format!("/delta/citation/{k}")).and_then(Value::as_str).unwrap_or("").to_string();
            Ok(if acc.cite(&s("url"), &s("title")) { acc.activity() } else { vec![] })
        }
        "content_block_delta" if v.pointer("/delta/type").and_then(Value::as_str) == Some("input_json_delta") && acc.server.contains_key(&idx) => {
            if let Some(b) = acc.server.get_mut(&idx) {
                let part = v.pointer("/delta/partial_json").and_then(Value::as_str).unwrap_or("");
                if b.2.len() + part.len() > crate::MAX_TOOL_ARGS {
                    return Err(Error::protocol("Werkzeugaufruf zu groß"));
                }
                b.2.push_str(part);
            }
            Ok(vec![])
        }
        "content_block_stop" if acc.server.contains_key(&idx) => {
            let Some((id, name, raw)) = acc.server.remove(&idx) else { return Ok(vec![]) };
            let input: Value = serde_json::from_str(if raw.trim().is_empty() { "{}" } else { &raw }).unwrap_or(Value::Null);
            // A continuation sends the block back with its input as an object.
            if let Some(o) = acc.content.get_mut(&idx).and_then(Value::as_object_mut)
                && input.is_object()
            {
                o.insert("input".into(), input.clone());
            }
            acc.server_call(id, &name, &input);
            Ok(acc.activity())
        }
        "message_delta" if v.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("pause_turn") => {
            acc.paused = acc.kept_bytes <= MAX_KEPT_BYTES;
            sse_events(event, data)
        }
        "content_block_start" if block_type == "tool_use" => {
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
        "content_block_stop" => {
            Ok(acc.blocks.remove(&idx).map(|(id, name, args)| vec![StreamEvent::ToolCall { call: crate::tool_call(id, name, &args) }]).unwrap_or_default())
        }
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
            return Err(if kind == "overloaded_error" || kind == "rate_limit_error" {
                Error::refused(msg.to_string())
            } else {
                Error::protocol(msg.to_string())
            });
        }
        _ => vec![],
    })
}

impl Backend for Anthropic {
    fn config(&self) -> &BackendConfig {
        &self.config
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { remote_history: false, uploads: false, system_prompt: true, vision: true, tools: true, tools_via_mcp: false, web_search: true }
    }

    fn check(&self) -> BoxFuture<'_, Result<AccountInfo>> {
        Box::pin(async move {
            let n = self.models().await?.len();
            Ok(AccountInfo { display_name: None, detail: Some(format!("{n} Modelle verfügbar")), expires: None, server_tools: vec![] })
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
            let acc = Arc::new(Mutex::new(ToolAcc::default()));
            let inner = sse_stream(resp, acc.clone());
            Ok(continuing(self.http.clone(), self.req_parts(&req)?, body(&req), acc, inner))
        })
    }
}

/// What a follow-up request needs: address, key and beta header.
#[derive(Clone)]
struct ReqParts {
    url: String,
    key: String,
    beta: Option<&'static str>,
}

impl Anthropic {
    fn req_parts(&self, req: &ChatRequest) -> Result<ReqParts> {
        let key = self.key.as_deref().ok_or_else(|| Error::auth("Kein API-Schlüssel für Anthropic hinterlegt"))?.trim().to_string();
        Ok(ReqParts { url: format!("{}/v1/messages", self.base), key, beta: takes_fallback(&req.model).then_some(FALLBACK_BETA) })
    }
}

fn sse_stream(resp: reqwest::Response, acc: Arc<Mutex<ToolAcc>>) -> EventStream {
    event_stream(resp, move |ev| sse_events_with(ev.event.as_deref(), &ev.data, &mut acc.lock().unwrap_or_else(PoisonError::into_inner)))
}

/// The answer's events; when the server pauses a long turn (web searches), the answer is sent
/// back and continued, up to [`MAX_CONTINUATIONS`] times, as one stream.
fn continuing(http: reqwest::Client, parts: ReqParts, body: Value, acc: Arc<Mutex<ToolAcc>>, inner: EventStream) -> EventStream {
    use futures::StreamExt;
    struct Turn {
        http: reqwest::Client,
        parts: ReqParts,
        body: Value,
        messages: usize,
        acc: Arc<Mutex<ToolAcc>>,
        inner: Option<EventStream>,
        left: u8,
    }
    let messages = body.get("messages").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let turn = Turn { http, parts, body, messages, acc, inner: Some(inner), left: MAX_CONTINUATIONS };
    futures::stream::unfold(turn, |mut t| async move {
        loop {
            let inner = t.inner.as_mut()?;
            match inner.next().await {
                Some(Ok(StreamEvent::Done)) | None => {
                    let content = {
                        let mut a = t.acc.lock().unwrap_or_else(PoisonError::into_inner);
                        if a.paused {
                            let c = a.assistant_content();
                            a.next_response();
                            Some(c)
                        } else {
                            None
                        }
                    };
                    t.inner = None;
                    match content {
                        Some(c) if t.left > 0 && !c.is_empty() => {
                            t.left -= 1;
                            // The answer so far goes back as the last assistant message; the server resumes it.
                            if let Some(m) = t.body.get_mut("messages").and_then(Value::as_array_mut) {
                                m.truncate(t.messages);
                                m.push(json!({"role": "assistant", "content": c}));
                            }
                            let mut r =
                                attributed(t.http.post(&t.parts.url), &t.parts.url).header("x-api-key", &t.parts.key).header("anthropic-version", API_VERSION);
                            if let Some(b) = t.parts.beta {
                                r = r.header("anthropic-beta", b);
                            }
                            match r.json(&t.body).send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    t.inner = Some(sse_stream(resp, t.acc.clone()));
                                    continue;
                                }
                                Ok(resp) => return Some((Err(status_error(resp, "Chat").await), t)),
                                Err(e) => return Some((Err(net_err(e)), t)),
                            }
                        }
                        Some(_) => return Some((Ok(StreamEvent::Delta { text: "\n\n_(Die Websuche wurde nach mehreren Runden angehalten.)_".into() }), t)),
                        None => return Some((Ok(StreamEvent::Done), t)),
                    }
                }
                Some(Err(e)) => {
                    t.inner = None;
                    return Some((Err(e), t));
                }
                Some(item) => return Some((item, t)),
            }
        }
    })
    .boxed()
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
        assert!(
            sse_events_with(
                None,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"tu1","name":"document_create","input":{}}}"#,
                &mut acc
            )
            .unwrap()
            .is_empty()
        );
        sse_events_with(None, r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"title\": \"A"}}"#, &mut acc)
            .unwrap();
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
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: DEFAULT_MODEL.into(),
            system: None,
            messages: vec![m(Role::User, "a"), m(Role::User, "b")],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: false,
        };
        let b = body(&req);
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["fallbacks"], "default");
        assert!(b.get("temperature").is_none());
    }

    fn sse(events: &[Value]) -> String {
        events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect()
    }

    /// Reads one HTTP request (headers and a content-length body).
    async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = sock.read(&mut chunk).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(h) = text.find("\r\n\r\n") {
                let len = text[..h]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if buf.len() >= h + 4 + len {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    /// Web search against a scripted Messages API: the server pauses the turn after the search,
    /// the client sends the answer so far back and gets the cited rest; sources end up marked.
    #[tokio::test]
    async fn web_search_with_pause_and_citations() {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let req = read_request(&mut sock).await;
                let continued = req.contains("\"server_tool_use\"");
                seen2.lock().unwrap().push(req);
                let events = if !continued {
                    vec![
                        json!({"type": "message_start", "message": {"usage": {"input_tokens": 10}}}),
                        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Ich suche. "}}),
                        json!({"type": "content_block_stop", "index": 0}),
                        json!({"type": "content_block_start", "index": 1, "content_block": {"type": "server_tool_use", "id": "srv1", "name": "web_search", "input": {}}}),
                        json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"query\": \"Ferien Berlin\"}"}}),
                        json!({"type": "content_block_stop", "index": 1}),
                        json!({"type": "content_block_start", "index": 2, "content_block": {"type": "web_search_tool_result", "tool_use_id": "srv1", "content": [
                            {"type": "web_search_result", "url": "https://example.org/ferien", "title": "Ferientermine", "encrypted_content": "ENC1"},
                            {"type": "web_search_result", "url": "https://example.org/andere", "title": "Andere Seite", "encrypted_content": "ENC2"},
                            {"type": "web_search_result", "url": "javascript:alert(1)", "title": "böse"}
                        ]}}),
                        json!({"type": "content_block_stop", "index": 2}),
                        json!({"type": "message_delta", "delta": {"stop_reason": "pause_turn"}, "usage": {"output_tokens": 5}}),
                        json!({"type": "message_stop"}),
                    ]
                } else {
                    vec![
                        json!({"type": "message_start", "message": {"usage": {"input_tokens": 20}}}),
                        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "citations_delta", "citation": {"type": "web_search_result_location", "url": "https://example.org/ferien", "title": "Ferientermine", "cited_text": "…", "encrypted_index": "x"}}}),
                        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Die Ferien beginnen am 1. Juli."}}),
                        json!({"type": "content_block_stop", "index": 0}),
                        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 9}}),
                        json!({"type": "message_stop"}),
                    ]
                };
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                let _ = sock.write_all(format!("{head}{}", sse(&events)).as_bytes()).await;
            }
        });
        let cfg = BackendConfig {
            id: "a".into(),
            name: "Claude".into(),
            kind: aiscraft_core::BackendKind::Anthropic,
            base_url: format!("http://{addr}"),
            default_model: None,
            send_context: true,
        };
        let b = Anthropic::new(cfg, format!("http://{addr}"), Some("k".into())).unwrap();
        let user = Message {
            id: "u".into(),
            conversation_id: "c".into(),
            role: Role::User,
            content: "Wann sind Ferien?".into(),
            created_at: 0,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        let req = ChatRequest {
            conversation_id: "c".into(),
            model: DEFAULT_MODEL.into(),
            system: None,
            messages: vec![user],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web: true,
        };
        let events: Vec<StreamEvent> = b.chat(req).await.unwrap().map(|e| e.unwrap()).collect().await;
        let text: String = events.iter().filter_map(|e| if let StreamEvent::Delta { text } = e { Some(text.as_str()) } else { None }).collect();
        assert_eq!(text, "Ich suche. Die Ferien beginnen am 1. Juli.");
        assert_eq!(events.iter().filter(|e| **e == StreamEvent::Done).count(), 1, "one answer, one end");
        assert_eq!(events.last(), Some(&StreamEvent::Done));
        let steps = events.iter().rev().find_map(|e| if let StreamEvent::Activity { steps } = e { Some(steps.clone()) } else { None }).unwrap();
        let search = steps.iter().find(|s| s.label == "Websuche").unwrap();
        assert_eq!(search.detail.as_deref(), Some("Ferien Berlin"));
        assert!(search.done && search.links.len() == 2, "only web links: {:?}", search.links);
        let cited: Vec<_> = steps.iter().flat_map(|s| &s.links).filter(|l| l.cited).map(|l| l.url.as_str()).collect();
        assert_eq!(cited, ["https://example.org/ferien"]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "paused once, continued once");
        assert!(seen[0].contains("web_search_20260209") && seen[0].contains("web_fetch_20260209"));
        // The continuation carries the search and its encrypted results back, input as an object.
        assert!(seen[1].contains("\"encrypted_content\":\"ENC1\"") && seen[1].contains("\"query\":\"Ferien Berlin\""));
    }

    #[test]
    fn web_tools_only_when_asked_and_errors_show() {
        let req = |web| ChatRequest {
            conversation_id: "c".into(),
            model: "claude-haiku-4-5".into(),
            system: None,
            messages: vec![],
            tools: vec![],
            tool_bridge: None,
            persona: None,
            web,
        };
        assert!(body(&req(false)).get("tools").is_none());
        let b = body(&req(true));
        assert_eq!(b["tools"][0]["type"], "web_search_20250305", "older models get the basic tools");
        let mut acc = ToolAcc::default();
        sse_events_with(
            None,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"s","name":"web_fetch","input":{}}}"#,
            &mut acc,
        )
        .unwrap();
        sse_events_with(
            None,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"url\":\"https://x.org\"}"}}"#,
            &mut acc,
        )
        .unwrap();
        sse_events_with(None, r#"{"type":"content_block_stop","index":0}"#, &mut acc).unwrap();
        let ev = sse_events_with(
            None,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"web_fetch_tool_result","tool_use_id":"s","content":{"type":"web_fetch_tool_error","error_code":"url_not_accessible"}}}"#,
            &mut acc,
        )
        .unwrap();
        let StreamEvent::Activity { steps } = &ev[0] else { panic!() };
        assert!(steps[0].failed && steps[0].links.is_empty());
        // Hostile shapes do not panic.
        for d in [
            r#"{"type":"content_block_start","content_block":{"type":"web_search_tool_result","content":7}}"#,
            r#"{"type":"content_block_delta","index":9,"delta":{"type":"citations_delta"}}"#,
        ] {
            let _ = sse_events_with(None, d, &mut acc);
        }
    }
}
