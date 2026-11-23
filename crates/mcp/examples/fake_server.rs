//! A scripted MCP stdio server for the tests. Not a real server; it misbehaves on purpose.
//!
//! `FAKE_MCP_MODE`: unset = normal, `silent` = never answers, `exit` = dies at once.
//! Tools: `echo`, `fail`, `slow` (never answers), `delay` (answers after `ms`), `crash`,
//! `image`, `ask` (sends the client a `roots/list` request first), `notify`, `huge`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

type Out = Arc<Mutex<std::io::Stdout>>;

fn write_raw(out: &Out, line: &str) {
    let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(o, "{line}");
    let _ = o.flush();
}

fn write(out: &Out, v: &Value) {
    write_raw(out, &v.to_string());
}

fn result(out: &Out, id: &Value, r: Value) {
    write(out, &json!({"jsonrpc": "2.0", "id": id, "result": r}));
}

fn text(t: &str) -> Value {
    json!({"content": [{"type": "text", "text": t}]})
}

fn main() {
    let mode = std::env::var("FAKE_MCP_MODE").unwrap_or_default();
    eprintln!("fake server starting, mode={mode:?}");
    if mode == "exit" {
        eprintln!("fatal: missing API key");
        std::process::exit(3);
    }
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let mut asking: Option<Value> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            eprintln!("bad line from client");
            continue;
        };
        if mode == "silent" {
            continue;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let Some(method) = msg.get("method").and_then(Value::as_str) else {
            // A response from the client, to our `roots/list` request.
            if let Some(ask_id) = asking.take() {
                let code = msg.pointer("/error/code").and_then(Value::as_i64).unwrap_or(0);
                result(&out, &ask_id, text(&format!("client answered {code}")));
            }
            continue;
        };
        match method {
            "initialize" => {
                write_raw(&out, "this is not json, the client must skip it");
                write_raw(&out, "");
                result(
                    &out,
                    &id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": {"tools": {"listChanged": true}},
                        "serverInfo": {"name": "fake", "version": "1.2.3", "title": "Fake Server"},
                        "instructions": "Use echo to echo."
                    }),
                );
            }
            "notifications/initialized" => eprintln!("initialized"),
            "notifications/cancelled" => {
                eprintln!("cancelled {}", msg.pointer("/params/requestId").cloned().unwrap_or(Value::Null));
            }
            "ping" => result(&out, &id, json!({})),
            "tools/list" => {
                let cursor = msg.pointer("/params/cursor").and_then(Value::as_str);
                match cursor {
                    None => result(
                        &out,
                        &id,
                        json!({
                            "tools": [
                                {"name": "echo", "description": "Echo text", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}},
                                 "annotations": {"readOnlyHint": true}},
                                {"description": "no name, must be skipped"}
                            ],
                            "nextCursor": "page-2"
                        }),
                    ),
                    Some("page-2") => result(
                        &out,
                        &id,
                        json!({"tools": [
                            {"name": "fail", "annotations": {"destructiveHint": true}},
                            {"name": "slow"}, {"name": "delay"}, {"name": "crash"}, {"name": "image"},
                            {"name": "ask"}, {"name": "notify"}, {"name": "huge"}
                        ]}),
                    ),
                    Some(_) => write(&out, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "bad cursor"}})),
                }
            }
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = msg.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
                match name {
                    "echo" => {
                        let t = args.get("text").and_then(Value::as_str).unwrap_or("");
                        result(&out, &id, json!({"content": [{"type": "text", "text": t}], "structuredContent": {"echoed": t}}));
                    }
                    "fail" => result(&out, &id, json!({"content": [{"type": "text", "text": "it broke"}], "isError": true})),
                    "slow" => {}
                    "delay" => {
                        let ms = args.get("ms").and_then(Value::as_u64).unwrap_or(100);
                        let out = Arc::clone(&out);
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(ms));
                            result(&out, &id, text(&format!("waited {ms}")));
                        });
                    }
                    "crash" => {
                        eprintln!("panic: something went terribly wrong");
                        std::process::exit(7);
                    }
                    "image" => result(
                        &out,
                        &id,
                        json!({"content": [
                            {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
                            {"type": "resource_link", "uri": "file:///tmp/x.txt", "name": "x.txt", "mimeType": "text/plain"},
                            {"type": "resource", "resource": {"uri": "file:///tmp/y.txt", "text": "inline"}}
                        ]}),
                    ),
                    "ask" => {
                        write(&out, &json!({"jsonrpc": "2.0", "id": "srv-1", "method": "roots/list", "params": {}}));
                        asking = Some(id);
                    }
                    "notify" => {
                        write(&out, &json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}));
                        write(&out, &json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "hello"}}));
                        result(&out, &id, text("notified"));
                    }
                    "huge" => {
                        // A line over the 16 MiB cap with this request's id, then the real answer.
                        let junk = "x".repeat(17 * 1024 * 1024);
                        write_raw(&out, &format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"junk\":\"{junk}\"}}}}"));
                        result(&out, &id, text("after huge"));
                    }
                    _ => write(&out, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": format!("unknown tool {name}")}})),
                }
            }
            _ if msg.get("id").is_some() => write(&out, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}})),
            _ => {}
        }
    }
    eprintln!("stdin closed, bye");
}
