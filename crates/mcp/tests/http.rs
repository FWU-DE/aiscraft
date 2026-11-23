//! Streamable HTTP transport against a scripted server on 127.0.0.1.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aiscraft_core::ErrorKind;
use aiscraft_mcp::{McpClient, ServerConfig, Transport};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

type Log = Arc<Mutex<Vec<Seen>>>;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Unauthorized,
    Legacy,
}

async fn start(mode: Mode) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let l = Arc::clone(&log);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handle(sock, mode, Arc::clone(&l)));
        }
    });
    (url, log)
}

async fn handle(mut sock: TcpStream, mode: Mode, log: Log) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let method = lines.next().unwrap_or("").split(' ').next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
    let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        match sock.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&tmp[..n]),
        }
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let seen = Seen { method: method.clone(), headers, body: body.clone() };
    log.lock().unwrap().push(seen.clone());

    if mode == Mode::Unauthorized {
        return respond(&mut sock, "401 Unauthorized", &[("WWW-Authenticate", "Bearer")], "application/json", r#"{"error":"invalid_token"}"#).await;
    }
    if mode == Mode::Legacy && method == "POST" {
        return respond(&mut sock, "405 Method Not Allowed", &[], "text/plain", "").await;
    }
    match method.as_str() {
        "GET" => return respond(&mut sock, "405 Method Not Allowed", &[], "text/plain", "").await,
        "DELETE" => return respond(&mut sock, "200 OK", &[], "text/plain", "").await,
        _ => {}
    }
    let rpc_method = body.get("method").and_then(Value::as_str).unwrap_or("");
    let id = body.get("id").cloned();
    if rpc_method == "initialize" {
        let r = json!({"jsonrpc": "2.0", "id": id, "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "http-fake", "version": "0.1"}
        }});
        return respond(&mut sock, "200 OK", &[("Mcp-Session-Id", "sess-42")], "application/json", &r.to_string()).await;
    }
    if seen.header("mcp-session-id") != Some("sess-42") {
        return respond(&mut sock, "400 Bad Request", &[], "text/plain", "missing session").await;
    }
    let Some(id) = id.filter(|_| !rpc_method.is_empty()) else {
        // Notifications and our responses to server requests.
        return respond(&mut sock, "202 Accepted", &[], "text/plain", "").await;
    };
    match rpc_method {
        "tools/list" => {
            // SSE: a notification, a server request, then the answer, in pieces.
            let events = [
                json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}),
                json!({"jsonrpc": "2.0", "id": "srv-ping", "method": "ping"}),
                json!({"jsonrpc": "2.0", "id": "srv-roots", "method": "roots/list"}),
                json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{"name": "search", "annotations": {"readOnlyHint": true}}]}}),
            ];
            let mut sse = String::from("id: prime\ndata:\n\n");
            for e in events {
                sse.push_str(&format!("event: message\ndata: {e}\n\n"));
            }
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            let _ = sock.write_all(head.as_bytes()).await;
            let (a, b) = sse.split_at(sse.len() / 2);
            let _ = sock.write_all(a.as_bytes()).await;
            let _ = sock.flush().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = sock.write_all(b.as_bytes()).await;
            // Keep the stream open: the client must stop reading once it has its answer.
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        "tools/call" if body.pointer("/params/name") == Some(&json!("hang")) => tokio::time::sleep(Duration::from_secs(10)).await,
        "tools/call" => {
            let r = json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": "gefunden"}]}});
            respond(&mut sock, "200 OK", &[], "application/json; charset=utf-8", &r.to_string()).await;
        }
        "ping" => respond(&mut sock, "200 OK", &[], "application/json", &json!({"jsonrpc": "2.0", "id": id, "result": {}}).to_string()).await,
        _ => {
            let r = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "nope"}});
            respond(&mut sock, "200 OK", &[], "application/json", &r.to_string()).await;
        }
    }
}

async fn respond(sock: &mut TcpStream, status: &str, extra: &[(&str, &str)], content_type: &str, body: &str) {
    let mut head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(body.as_bytes()).await;
    let _ = sock.shutdown().await;
}

fn config(url: &str) -> ServerConfig {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_string(), "Bearer test-token".to_string());
    ServerConfig { id: "web".into(), name: "Web".into(), transport: Transport::Http { url: url.into(), headers }, enabled: true, timeout_secs: Some(5) }
}

fn posts(log: &Log) -> Vec<Seen> {
    log.lock().unwrap().iter().filter(|s| s.method == "POST").cloned().collect()
}

#[tokio::test]
async fn json_and_sse_responses_with_session() {
    let (url, log) = start(Mode::Normal).await;
    let c = McpClient::connect(&config(&url)).await.unwrap();
    assert_eq!(c.server_info().name, "http-fake");

    let started = Instant::now();
    let tools = c.list_tools().await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(3), "stopped reading the SSE stream after the answer");
    assert_eq!(tools.len(), 1);
    assert!(tools[0].annotations.is_read_only());
    assert!(c.take_tools_changed(), "notification inside the SSE stream");

    let r = c.call_tool("search", json!({"q": "x"})).await.unwrap();
    assert_eq!(r.text(), "gefunden");
    c.ping().await.unwrap();

    // Our answers to the server's requests arrive as separate POSTs.
    let deadline = Instant::now() + Duration::from_secs(2);
    let (ping_reply, roots_reply) = loop {
        let p = posts(&log);
        let ping = p.iter().find(|s| s.body.get("id") == Some(&json!("srv-ping"))).cloned();
        let roots = p.iter().find(|s| s.body.get("id") == Some(&json!("srv-roots"))).cloned();
        if let (Some(a), Some(b)) = (&ping, &roots) {
            break (a.clone(), b.clone());
        }
        assert!(Instant::now() < deadline, "replies to server requests missing: {p:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(ping_reply.body.get("result"), Some(&json!({})));
    assert_eq!(roots_reply.body.pointer("/error/code"), Some(&json!(-32601)));

    let p = posts(&log);
    let init = &p[0];
    assert_eq!(init.body["method"], "initialize");
    assert_eq!(init.body["params"]["protocolVersion"], "2025-06-18");
    assert_eq!(init.body["params"]["clientInfo"]["name"], "AIS.ChatCraft");
    assert_eq!(init.header("accept"), Some("application/json, text/event-stream"));
    assert_eq!(init.header("authorization"), Some("Bearer test-token"));
    assert_eq!(init.header("mcp-session-id"), None);
    assert_eq!(p[1].body["method"], "notifications/initialized");
    for later in &p[1..] {
        assert_eq!(later.header("mcp-session-id"), Some("sess-42"), "{later:?}");
        assert_eq!(later.header("mcp-protocol-version"), Some("2025-06-18"));
        assert_eq!(later.header("authorization"), Some("Bearer test-token"));
    }

    c.close().await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while !log.lock().unwrap().iter().any(|s| s.method == "DELETE") {
        assert!(Instant::now() < deadline, "session not deleted on close");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn hanging_request_times_out_and_is_cancelled() {
    let (url, log) = start(Mode::Normal).await;
    let c = McpClient::connect(&config(&url)).await.unwrap();
    let e = c.call_tool_with_timeout("hang", json!({}), Duration::from_millis(300)).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network);
    let call_id = posts(&log).iter().find(|s| s.body["method"] == "tools/call").map(|s| s.body["id"].clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !posts(&log).iter().any(|s| s.body["method"] == "notifications/cancelled" && s.body["params"]["requestId"] == call_id) {
        assert!(Instant::now() < deadline, "no notifications/cancelled");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(c.call_tool("search", json!({})).await.unwrap().text(), "gefunden");
}

#[tokio::test]
async fn unauthorized_is_an_auth_error_in_german() {
    let (url, _) = start(Mode::Unauthorized).await;
    let e = McpClient::connect(&config(&url)).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert!(e.message.contains("verlangt eine Anmeldung"), "{}", e.message);
    assert!(e.message.contains("OAuth"), "{}", e.message);
}

#[tokio::test]
async fn legacy_sse_servers_get_a_clear_message() {
    let (url, _) = start(Mode::Legacy).await;
    let e = McpClient::connect(&config(&url)).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert!(e.message.contains("HTTP+SSE"), "{}", e.message);
}

#[tokio::test]
async fn unreachable_and_invalid_urls() {
    // Bind and drop to get a port nobody listens on.
    let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    let e = McpClient::connect(&config(&format!("http://127.0.0.1:{port}/mcp"))).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network);
    assert_eq!(McpClient::connect(&config("ftp://example.org")).await.unwrap_err().kind, ErrorKind::Invalid);
    assert_eq!(McpClient::connect(&config("kein url")).await.unwrap_err().kind, ErrorKind::Invalid);
    let mut cfg = config("http://127.0.0.1:1/mcp");
    cfg.transport = Transport::Http { url: "http://127.0.0.1:1/mcp".into(), headers: [("Bad Header".to_string(), "x".to_string())].into() };
    assert_eq!(McpClient::connect(&cfg).await.unwrap_err().kind, ErrorKind::Invalid);
}
