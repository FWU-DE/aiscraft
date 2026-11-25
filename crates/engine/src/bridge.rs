//! The tool bridge: a small MCP server (Streamable HTTP, JSON answers) on 127.0.0.1 that lets
//! backends which call tools through MCP themselves (the Claude CLI) use the app's tools: the
//! built-in ones and the chat's connectors, under the same rules (sources, approvals, trust
//! check). Each answer gets its own token, bound to its conversation; the token ends with it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use aiscraft_core::{Result, ToolBridge, ToolCall};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Ctx, new_id};

const MAX_REQUEST: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Bridge {
    port: tokio::sync::OnceCell<u16>,
    sessions: Mutex<HashMap<String, String>>,
}

/// Open a bridge session for a conversation. Drop the guard to end it.
pub(crate) async fn open(c: &Ctx, conv_id: &str) -> Result<(ToolBridge, Guard)> {
    let port = *c
        .0
        .bridge
        .port
        .get_or_try_init(|| async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| aiscraft_core::Error::network(format!("Werkzeug-Brücke: {e}")))?;
            let port = listener.local_addr().map_err(|e| aiscraft_core::Error::network(e.to_string()))?.port();
            let ctx = c.clone();
            c.spawn(async move {
                loop {
                    let Ok((sock, peer)) = listener.accept().await else { return };
                    if !peer.ip().is_loopback() {
                        continue;
                    }
                    let ctx = ctx.clone();
                    ctx.clone().spawn(async move {
                        let _ = serve(&ctx, sock).await;
                    });
                }
            });
            Ok::<u16, aiscraft_core::Error>(port)
        })
        .await?;
    let token = new_id().replace('-', "");
    c.0.bridge.sessions.lock().unwrap_or_else(PoisonError::into_inner).insert(token.clone(), conv_id.to_string());
    Ok((ToolBridge { url: format!("http://127.0.0.1:{port}/mcp"), token: token.clone() }, Guard { ctx: c.clone(), token }))
}

pub(crate) struct Guard {
    ctx: Ctx,
    token: String,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.ctx.0.bridge.sessions.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.token);
    }
}

async fn serve(c: &Ctx, mut sock: tokio::net::TcpStream) -> std::io::Result<()> {
    // Read the head, then exactly content-length bytes.
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    let (head_end, len) = loop {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        if buf.len() > MAX_REQUEST {
            return respond(&mut sock, 413, "", &Value::Null).await;
        }
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(buf.get(..pos).unwrap_or_default()).to_string();
            let len =
                head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
            break (pos + 4, len.min(MAX_REQUEST));
        }
    };
    while buf.len() < head_end + len {
        let n = sock.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
    }
    let head = String::from_utf8_lossy(buf.get(..head_end).unwrap_or_default()).to_string();
    if !head.starts_with("POST /mcp") {
        return respond(&mut sock, 405, "", &Value::Null).await;
    }
    let token = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim().eq_ignore_ascii_case("authorization")).then(|| v.trim().strip_prefix("Bearer ").unwrap_or("").to_string())
    });
    let conv = token.and_then(|t| c.0.bridge.sessions.lock().unwrap_or_else(PoisonError::into_inner).get(&t).cloned());
    let Some(conv) = conv else { return respond(&mut sock, 401, "", &json!({"error": "unbekannter Token"})).await };
    let body: Value = serde_json::from_slice(buf.get(head_end..).unwrap_or_default()).unwrap_or(Value::Null);
    match handle(c, &conv, &body).await {
        Some(reply) => respond(&mut sock, 200, "", &reply).await,
        None => respond(&mut sock, 202, "", &Value::Null).await,
    }
}

/// One JSON-RPC message. `None` for notifications.
pub(crate) async fn handle(c: &Ctx, conv: &str, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let ok = |result: Value| json!({"jsonrpc": "2.0", "id": id, "result": result});
    Some(match method {
        "initialize" => ok(json!({
            "protocolVersion": req.pointer("/params/protocolVersion").cloned().unwrap_or(json!("2025-06-18")),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "aischatcraft", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => ok(json!({})),
        "tools/list" => {
            let sources = c.state().conversation(conv).map(|x| x.sources.clone()).unwrap_or_default();
            let tools: Vec<Value> = crate::tools::specs_for(c, &sources)
                .into_iter()
                .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema}))
                .collect();
            ok(json!({"tools": tools}))
        }
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("").to_string();
            let arguments = req.pointer("/params/arguments").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
            let call = ToolCall { id: new_id(), name, arguments, outcome: None };
            let evidence = crate::trust::evidence(&c.store(|st| st.messages(conv)).unwrap_or_default());
            let (text, is_error, outcome) = crate::tools::run(c, conv, &call, &evidence, true).await;
            record(c, conv, ToolCall { outcome: Some(outcome.to_string()), ..call });
            ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
        }
        other => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Methode {other} gibt es nicht")}}),
    })
}

/// Show the call as a chip on the answer being written.
fn record(c: &Ctx, conv: &str, call: ToolCall) {
    c.update(|s| {
        if s.current.as_deref() == Some(conv)
            && let Some(m) = s.messages.iter_mut().rev().find(|m| m.role == aiscraft_core::Role::Assistant)
        {
            m.tool_calls.push(call.clone());
        }
    });
    if let Ok(mut msgs) = c.store(|st| st.messages(conv))
        && let Some(m) = msgs.iter_mut().rev().find(|m| m.role == aiscraft_core::Role::Assistant)
    {
        m.tool_calls.push(call);
        let _ = c.store(|st| st.save_message(m));
    }
}

async fn respond(sock: &mut tokio::net::TcpStream, status: u16, _ct: &str, body: &Value) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        401 => "Unauthorized",
        405 => "Method Not Allowed",
        _ => "Payload Too Large",
    };
    let text = if body.is_null() { String::new() } else { body.to_string() };
    let resp = format!("HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{text}", text.len());
    sock.write_all(resp.as_bytes()).await?;
    sock.shutdown().await
}

#[allow(dead_code)]
fn _arc(_: Arc<()>) {}
