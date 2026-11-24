//! stdio transport against the scripted `examples/fake_server.rs` (built by `cargo test`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aiscraft_core::ErrorKind;
use aiscraft_mcp::{Content, McpClient, ServerConfig, Transport};
use serde_json::json;

fn fake_server() -> PathBuf {
    // The test binary lives somewhere below target/<profile>/ (deps/ or build/<pkg>/<hash>/);
    // `cargo test` puts the examples in target/<profile>/examples/.
    let exe = std::env::current_exe().unwrap();
    let name = format!("fake_server{}", std::env::consts::EXE_SUFFIX);
    exe.ancestors()
        .map(|d| d.join("examples").join(&name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("examples/{name} fehlt neben {}; `cargo test` baut die Beispiele mit", exe.display()))
}

fn config(mode: &str) -> ServerConfig {
    let mut env = BTreeMap::new();
    env.insert("FAKE_MCP_MODE".to_string(), mode.to_string());
    ServerConfig {
        id: "fake".into(),
        name: "Fake".into(),
        transport: Transport::Stdio { command: fake_server().to_string_lossy().into_owned(), args: vec![], env, cwd: None },
        enabled: true,
        timeout_secs: Some(10),
    }
}

async fn connect() -> McpClient {
    McpClient::connect(&config("")).await.unwrap()
}

async fn wait_for_stderr(client: &McpClient, needle: &str) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        if client.stderr_tail().contains(needle) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn initialize_reports_server_info() {
    let c = connect().await;
    let info = c.server_info();
    assert_eq!(info.name, "fake");
    assert_eq!(info.version, "1.2.3");
    assert_eq!(info.title.as_deref(), Some("Fake Server"));
    assert_eq!(info.protocol_version, "2025-06-18");
    assert!(info.tools_list_changed());
    assert_eq!(c.instructions(), Some("Use echo to echo."));
    assert_eq!(c.id(), "fake");
    assert!(wait_for_stderr(&c, "initialized").await, "notifications/initialized was sent");
    c.ping().await.unwrap();
    c.close().await;
}

#[tokio::test]
async fn list_tools_follows_pages_and_skips_nameless() {
    let c = connect().await;
    let tools = c.list_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "fail", "slow", "delay", "crash", "image", "ask", "notify", "huge"]);
    assert!(tools[0].annotations.is_read_only());
    assert_eq!(tools[0].description.as_deref(), Some("Echo text"));
    assert_eq!(tools[1].annotations.destructive_hint, Some(true));
    assert!(!tools[1].annotations.is_read_only());
    assert_eq!(tools[2].input_schema, json!({"type": "object"}));
}

#[tokio::test]
async fn call_tool_text_error_and_blocks() {
    let c = connect().await;
    let r = c.call_tool("echo", json!({"text": "hallo"})).await.unwrap();
    assert!(!r.is_error);
    assert_eq!(r.text(), "hallo");
    assert_eq!(r.structured_content, Some(json!({"echoed": "hallo"})));

    let r = c.call_tool("fail", serde_json::Value::Null).await.unwrap();
    assert!(r.is_error);
    assert_eq!(r.text(), "it broke");

    let r = c.call_tool("image", json!({})).await.unwrap();
    assert!(matches!(&r.content[0], Content::Image { data, mime_type } if data == "iVBORw0KGgo=" && mime_type == "image/png"));
    assert!(matches!(&r.content[1], Content::ResourceLink { uri, .. } if uri == "file:///tmp/x.txt"));
    assert!(matches!(&r.content[2], Content::Resource(res) if res.text.as_deref() == Some("inline")));

    let e = c.call_tool("nope", json!({})).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
    assert!(e.message.contains("unknown tool"), "{}", e.message);

    let e = c.call_tool("echo", json!([1, 2])).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn concurrent_calls_answered_out_of_order() {
    let c = Arc::new(connect().await);
    let slow = {
        let c = Arc::clone(&c);
        tokio::spawn(async move { c.call_tool("delay", json!({"ms": 300})).await })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    let started = Instant::now();
    let fast = c.call_tool("echo", json!({"text": "zuerst"})).await.unwrap();
    assert_eq!(fast.text(), "zuerst");
    assert!(started.elapsed() < Duration::from_millis(250), "fast call did not wait for the slow one");
    assert_eq!(slow.await.unwrap().unwrap().text(), "waited 300");
}

#[tokio::test]
async fn timeout_cancels_and_client_stays_usable() {
    let c = connect().await;
    let started = Instant::now();
    let e = c.call_tool_with_timeout("slow", json!({}), Duration::from_millis(300)).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(e.kind, ErrorKind::Network);
    assert!(e.message.contains("300 ms"), "{}", e.message);
    assert!(wait_for_stderr(&c, "cancelled").await, "notifications/cancelled was sent");
    assert_eq!(c.call_tool("echo", json!({"text": "noch da"})).await.unwrap().text(), "noch da");
}

#[tokio::test]
async fn dropping_the_future_cancels_the_request() {
    let c = connect().await;
    let _ = tokio::time::timeout(Duration::from_millis(200), c.call_tool("slow", json!({}))).await;
    assert!(wait_for_stderr(&c, "cancelled").await);
}

#[tokio::test]
async fn server_crash_fails_calls_with_its_output() {
    let c = connect().await;
    let e = c.call_tool("crash", json!({})).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network);
    assert!(e.message.contains("wurde beendet"), "{}", e.message);
    assert!(e.message.contains("Exit-Code 7"), "{}", e.message);
    assert!(e.message.contains("neu verbinden"), "names the next step: {}", e.message);
    assert!(e.message.contains("something went terribly wrong"), "{}", e.message);
    assert!(c.is_closed());
    let started = Instant::now();
    assert!(c.list_tools().await.is_err());
    assert!(started.elapsed() < Duration::from_millis(100), "later calls fail at once");
}

#[tokio::test]
async fn unsupported_server_requests_get_method_not_found() {
    let c = connect().await;
    assert_eq!(c.call_tool("ask", json!({})).await.unwrap().text(), "client answered -32601");
}

#[tokio::test]
async fn tools_list_changed_is_exposed() {
    let c = connect().await;
    let mut rx = c.tools_changed();
    assert!(!c.take_tools_changed());
    assert_eq!(c.call_tool("notify", json!({})).await.unwrap().text(), "notified");
    tokio::time::timeout(Duration::from_secs(2), rx.changed()).await.unwrap().unwrap();
    assert_eq!(*rx.borrow(), 1);
    assert!(c.take_tools_changed());
    assert!(!c.take_tools_changed());
}

#[tokio::test]
async fn oversized_lines_are_skipped() {
    let c = connect().await;
    assert_eq!(c.call_tool("huge", json!({})).await.unwrap().text(), "after huge");
}

#[tokio::test]
async fn silent_server_times_out_on_connect() {
    let mut cfg = config("silent");
    cfg.timeout_secs = Some(1);
    let started = Instant::now();
    let e = McpClient::connect(&cfg).await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(e.kind, ErrorKind::Network);
    assert!(e.message.contains("initialize"), "{}", e.message);
}

#[tokio::test]
async fn server_dying_at_start_reports_stderr() {
    let e = McpClient::connect(&config("exit")).await.unwrap_err();
    assert!(e.message.contains("fatal: missing API key"), "{}", e.message);
    assert!(e.message.contains("Exit-Code 3"), "{}", e.message);
}

#[tokio::test]
async fn bad_configs_are_rejected() {
    let mut cfg = config("");
    cfg.transport = Transport::Stdio { command: "/nonexistent/mcp-server".into(), args: vec![], env: BTreeMap::new(), cwd: None };
    assert_eq!(McpClient::connect(&cfg).await.unwrap_err().kind, ErrorKind::NotFound);

    let mut cfg = config("");
    cfg.transport = Transport::Stdio { command: "  ".into(), args: vec![], env: BTreeMap::new(), cwd: None };
    assert_eq!(McpClient::connect(&cfg).await.unwrap_err().kind, ErrorKind::Invalid);

    let mut cfg = config("");
    if let Transport::Stdio { cwd, .. } = &mut cfg.transport {
        *cwd = Some("/nonexistent/dir".into());
    }
    assert_eq!(McpClient::connect(&cfg).await.unwrap_err().kind, ErrorKind::NotFound);

    let mut cfg = config("");
    cfg.enabled = false;
    assert!(McpClient::connect(&cfg).await.is_err());
}

#[tokio::test]
async fn close_ends_the_connection() {
    let c = connect().await;
    c.close().await;
    assert!(c.is_closed());
    assert_eq!(c.ping().await.unwrap_err().kind, ErrorKind::Cancelled);
}

#[test]
fn config_round_trips_through_serde() {
    let cfg = config("x");
    let text = serde_json::to_string(&cfg).unwrap();
    assert!(text.contains(r#""type":"stdio""#), "{text}");
    assert_eq!(serde_json::from_str::<ServerConfig>(&text).unwrap(), cfg);
    let http: ServerConfig = serde_json::from_value(json!({"id": "gh", "name": "GitHub", "transport": {"type": "http", "url": "https://x/mcp"}})).unwrap();
    assert!(http.enabled);
    assert_eq!(http.request_timeout(), aiscraft_mcp::DEFAULT_TIMEOUT);
    let debug = format!("{:?}", Transport::Http { url: "u".into(), headers: [("Authorization".to_string(), "Bearer geheim".to_string())].into() });
    assert!(!debug.contains("geheim"), "{debug}");
}

/// Runs the real AIS.ChatCraft CLI MCP server, if it has been built.
#[tokio::test]
#[ignore]
async fn real_aiscraft_cli_server() {
    let Some(home) = std::env::var_os("HOME") else { return };
    let exe = PathBuf::from(home).join("code/aiscraft/target/release/aiscraft-cli");
    if !exe.exists() {
        eprintln!("{} not built, skipping", exe.display());
        return;
    }
    let cfg = ServerConfig {
        id: "aiscraft".into(),
        name: "AIS.ChatCraft".into(),
        transport: Transport::Stdio { command: exe.to_string_lossy().into_owned(), args: vec!["mcp".into()], env: BTreeMap::new(), cwd: None },
        enabled: true,
        timeout_secs: Some(30),
    };
    let c = McpClient::connect(&cfg).await.unwrap();
    eprintln!("server: {:?}", c.server_info());
    let tools = c.list_tools().await.unwrap();
    assert!(!tools.is_empty());
    for t in &tools {
        eprintln!("{} -> {} (read-only: {})", t.name, aiscraft_mcp::llm_tool_name(c.id(), &t.name), t.annotations.is_read_only());
    }
    c.ping().await.unwrap();
    c.close().await;
}
