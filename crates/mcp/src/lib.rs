//! MCP (Model Context Protocol) client for AIS.ChatCraft's connectors ("Konnektoren").
//!
//! A [`McpClient`] talks to one MCP server, either a local program over stdio or a remote
//! endpoint over Streamable HTTP (spec 2025-06-18), and lets the models list and call its tools.
//! Everything a server sends is treated as untrusted: messages are size-capped, garbage is
//! skipped, a server that dies or never answers turns into an error, never a crash.

mod client;
mod config;
mod http;
mod names;
mod sse;
mod stdio;
mod types;

use std::time::Duration;

pub use client::McpClient;
pub use config::{ServerConfig, Transport};
pub use names::llm_tool_name;
pub use types::{Content, EmbeddedResource, ResourceContents, ServerInfo, Tool, ToolAnnotations, ToolResult, ToolUi};

/// The resource type of an MCP Apps view (SEP-1865).
pub const APP_MIME: &str = "text/html;profile=mcp-app";

/// The protocol version AIS.ChatCraft asks for in `initialize`.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Older versions servers may answer with; they are close enough for tools.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Largest single JSON-RPC message accepted from or sent to a server.
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// How long a request may take unless the config says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
