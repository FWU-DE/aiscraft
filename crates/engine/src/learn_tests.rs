//! Functional tests for learning apps and worksheets (`learn/`). Included from `tests.rs`, so
//! `every_command_has_a_functional_test` sees them.

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

const APP_V1: &str = "<!doctype html>\n<html lang=\"de\"><head><title>Bruch-Trainer</title></head><body><h1>Brüche</h1><script>fetch('https://tracker.example/x')</script></body></html>";
const APP_V2: &str = "<!doctype html>\n<html lang=\"de\"><head><title>Bruch-Trainer</title></head><body><h1>Brüche, Version 2</h1></body></html>";

fn sheet_json() -> Value {
    json!({
        "format": "aiscraft-worksheet", "version": 1, "title": "Bruchrechnung", "subject": "Mathematik", "grade": "6",
        "instructions": "Bearbeite alle Aufgaben.",
        "tasks": [
            {"id": "t1", "type": "choice", "prompt": "Welcher Bruch ist groesser?", "points": 1, "options": [{"text": "1/2", "correct": true}, {"text": "1/3"}]},
            {"id": "t2", "type": "true_false", "prompt": "1/4 ist kleiner als 1/2.", "answer": true},
            {"id": "t3", "type": "cloze", "prompt": "Ergaenze.", "text": "1/2 = [[2/4|0,5]]", "points": 2},
            {"id": "t4", "type": "numeric", "prompt": "Kehrwert von 4 als Dezimalzahl", "answer": 0.25, "tolerance": 0.001},
            {"id": "t5", "type": "free_text", "prompt": "Erklaere, warum 2/4 = 1/2 ist.", "sample": "Kuerzen mit 2", "points": 2}
        ]
    })
}

/// What the scripted model answers, by what the request contains.
fn route(req: &str) -> Vec<Value> {
    let text = |t: &str| vec![json!({"choices": [{"delta": {"content": t}}]})];
    let tool = |name: &str, args: Value| {
        vec![
            json!({"choices": [{"delta": {"tool_calls": [{"index": 0, "id": "c1", "function": {"name": name, "arguments": args.to_string()}}]}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "tool_calls"}]}),
        ]
    };
    let has_tool_result = req.contains("\"role\":\"tool\"");
    if req.contains("Werkzeug worksheet_create") && !has_tool_result {
        return tool("worksheet_create", json!({"worksheet": sheet_json()}));
    }
    if req.contains("Werkzeug learning_app_create") && !has_tool_result {
        return tool("learning_app_create", json!({"title": "Bruch-Trainer", "html": APP_V1}));
    }
    if has_tool_result {
        return text("Fertig.");
    }
    if req.contains("interaktive Lern-App zum Thema") || req.contains("Lern-App zum Thema") {
        return text(&format!("Hier ist deine App:\n\n```html\n{APP_V1}\n```\n\nViel Spaß."));
    }
    if req.contains("Du reparierst ein Arbeitsblatt") || req.contains("wandelst ein Dokument") || req.contains("interaktives Arbeitsblatt zum Thema") {
        return text(&format!("```aiscraft-worksheet\n{}\n```", sheet_json()));
    }
    if req.contains("überarbeitest eine Lern-App") {
        return text(&format!("```html\n{APP_V2}\n```"));
    }
    if req.contains("überarbeitest ein Arbeitsblatt") {
        let mut s = sheet_json();
        s["title"] = json!("Bruchrechnung leicht");
        return text(&format!("```aiscraft-worksheet\n{s}\n```"));
    }
    text("Okay.")
}

fn routed_engine() -> Engine {
    routed_engine_logged().0
}

/// The routed engine and the JSON bodies of the chat requests it sent.
fn routed_engine_logged() -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sent = log.clone();
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let url = engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let sent = sent.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    loop {
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..k]);
                        let s = String::from_utf8_lossy(&buf).to_string();
                        if let Some(h) = s.find("\r\n\r\n") {
                            let len = s[..h]
                                .lines()
                                .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let _ = sock
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                        return;
                    }
                    if let Some(body) = req.split_once("\r\n\r\n").and_then(|(_, b)| serde_json::from_str::<Value>(b).ok()) {
                        sent.lock().unwrap().push(body);
                    }
                    // JSON-escaped in the request body: unescape quotes so routes match plain text.
                    let plain = req.replace("\\\"", "\"").replace("\\n", "\n");
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    for c in route(&plain) {
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, log)
}

/// Format requests (#56): the backend gets the whole format spec, exactly as before; the chat
/// and the Markdown export show the teacher's words with the spec folded away, for every
/// format in the registry.
#[test]
fn format_requests_send_the_spec_but_show_only_the_topic() {
    use crate::learn::{app_prompt, formats, worksheet_prompt};
    let (engine, log) = routed_engine_logged();
    engine.call("settings.set", json!({"key": "tools", "value": false})).unwrap();
    let cases = [
        ("worksheet.request", "worksheet", worksheet_prompt("Bruchrechnung (Klasse: 6)", 8, false)),
        ("learning_app.request", "learning_app", app_prompt("Bruchrechnung (Klasse: 6)", false)),
    ];
    assert_eq!(cases.len(), formats::formats().len(), "every format in the registry is covered");
    for (command, format, expected) in cases {
        engine.call("chat.new", json!({})).unwrap();
        log.lock().unwrap().clear();
        let out = engine.call(command, json!({"topic": "Bruchrechnung", "grade": "6"})).unwrap();
        assert_eq!(out["created"], true, "{command}: {out}");
        // Sent payload unchanged: the last user message the server got is the whole prompt.
        let bodies = log.lock().unwrap().clone();
        let sent = bodies[0]["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "user").unwrap()["content"].clone();
        let sent = sent.as_str().map(str::to_string).unwrap_or_else(|| sent.to_string());
        assert_eq!(sent, expected, "{command}: the backend gets the full format spec");
        // Stored whole, shown split.
        let s = engine.state();
        let user = s.messages.iter().find(|m| m.role == aiscraft_core::Role::User).unwrap();
        let title = &s.conversations.iter().find(|c| Some(&c.id) == s.current.as_ref()).unwrap().title;
        assert!(title.ends_with(": Bruchrechnung (Klasse: 6)"), "{command}: the chat is named after the topic, got {title}");
        assert_eq!(user.content, expected);
        let r = formats::split_request(&user.content).unwrap();
        assert_eq!(r.format.id, format);
        assert_eq!(r.topic, "Bruchrechnung (Klasse: 6)");
        assert!(r.spec.contains(&(r.format.prompt_description)()), "{command}: the folded part is the spec");
        // The export carries a one-line note, not the spec.
        let md = engine.call("chat.export", json!({})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(md.contains("Bruchrechnung (Klasse: 6)") && md.contains(&format!("_Mit Formatvorgabe: {}_", r.format.label)), "{md}");
        assert!(!md.contains(&(r.format.prompt_description)()), "{command}: no spec in the export");
    }
}

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aiscraft-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Without tools (the AIS.chat way): the prompt helper asks for an html block, the answer
/// becomes a learning app; it is saved with the CSP, revised to a new version, and opening it
/// is refused in an engine without a data folder.
#[test]
fn learning_app_from_an_html_block() {
    let engine = routed_engine();
    engine.call("settings.set", json!({"key": "tools", "value": false})).unwrap();
    assert!(engine.call("learning_app.request", json!({"topic": " "})).is_err(), "a topic is needed");
    assert!(engine.call("worksheet.request", json!({"topic": "\""})).is_err(), "a stray quote is no topic");
    let out = engine.call("learning_app.request", json!({"topic": "Brüche vergleichen", "grade": "6"})).unwrap();
    assert_eq!(out["created"], true);
    let id = out["id"].as_str().unwrap().to_string();
    let s = engine.state();
    let a = s.artifacts.iter().find(|a| a.id == id).unwrap();
    assert_eq!(a.kind, aiscraft_core::ArtifactKind::App);
    assert_eq!(a.title, "Bruch-Trainer");
    assert_eq!(s.open_artifact.as_deref(), Some(id.as_str()));
    // The request carried the constraints.
    assert!(s.messages[0].content.contains("keine externen Skripte") && s.messages[0].content.contains("```html"));

    // Saved copy: CSP before the script.
    let dir = tmp_dir("app");
    let path = dir.join("app.html");
    engine.call("learning_app.save", json!({"id": id, "path": path.display().to_string()})).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.find("Content-Security-Policy").unwrap() < saved.find("<script>").unwrap());
    assert!(saved.contains("default-src 'none'"));
    let exported = engine.call("artifact.export", json!({"id": id, "format": "html"})).unwrap();
    assert!(exported["text"].as_str().unwrap().contains("Content-Security-Policy"));
    let _ = std::fs::remove_dir_all(dir);

    // Tests never open windows or browsers.
    let err = engine.call("learning_app.open", json!({"id": id})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Unsupported);

    // Revise: a new version, the old one kept.
    engine.call("artifact.revise", json!({"id": id, "instruction": "Version 2"})).unwrap();
    assert!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().contains("Version 2"));
    assert!(engine.call("artifact.versions", json!({"id": id})).unwrap().as_array().unwrap().len() >= 3);

    // From a message, and refusing snippets and oversized files.
    let mid = engine.state().messages.iter().find(|m| m.role == aiscraft_core::Role::Assistant && m.content.contains("```html")).unwrap().id.clone();
    let id2 = engine.call("learning_app.create", json!({"from_message": mid})).unwrap()["id"].as_str().unwrap().to_string();
    assert_ne!(id, id2);
    assert!(engine.call("learning_app.create", json!({"html": "<b>nur ein Schnipsel</b>"})).is_err());
    let huge = format!("<!doctype html><html>{}</html>", "x".repeat(crate::learn::MAX_APP_BYTES));
    assert!(engine.call("learning_app.create", json!({"html": huge})).is_err());
    assert!(engine.call("learning_app.save", json!({"id": "nope", "path": "/tmp/x.html"})).is_err());
}

/// With tools: the prompt helpers ask the model to use learning_app_create and worksheet_create.
#[test]
fn tool_backends_create_apps_and_worksheets_with_tools() {
    let engine = routed_engine();
    let out = engine.call("worksheet.request", json!({"topic": "Bruchrechnung", "count": 5})).unwrap();
    assert_eq!(out["created"], true);
    let s = engine.state();
    let ws = s.artifacts.iter().find(|a| a.kind == aiscraft_core::ArtifactKind::Worksheet).unwrap();
    assert_eq!(ws.title, "Bruchrechnung");
    assert!(s.messages.iter().any(|m| m.tool_calls.iter().any(|t| t.name == "worksheet_create")));
    engine.call("chat.new", json!({})).unwrap();
    let out = engine.call("learning_app.request", json!({"topic": "Brüche"})).unwrap();
    assert_eq!(out["created"], true);
    assert!(engine.state().artifacts.iter().any(|a| a.kind == aiscraft_core::ArtifactKind::App));
}

/// Worksheets: from a block (AIS.chat), validation, editing tasks, checking answers, export as
/// PDF (sheet and solutions) and Word, printing refused in tests, conversion from a document.
#[test]
fn worksheet_flow_end_to_end() {
    let engine = routed_engine();
    engine.call("settings.set", json!({"key": "tools", "value": false})).unwrap();
    let out = engine.call("worksheet.request", json!({"topic": "Bruchrechnung", "grade": "6"})).unwrap();
    assert_eq!(out["created"], true, "{out}");
    let id = out["id"].as_str().unwrap().to_string();
    assert!(engine.state().messages[0].content.contains("```aiscraft-worksheet"));

    // Validate.
    let v = engine.call("worksheet.validate", json!({"content": sheet_json().to_string()})).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["tasks"], 5);
    let v = engine.call("worksheet.validate", json!({"content": "{kaputt"})).unwrap();
    assert_eq!(v["ok"], false);

    // Check answers.
    let r = engine.call("worksheet.check", json!({"id": id, "answers": {"t1": [0], "t2": false, "t3": ["0.5"], "t4": "0,25", "t5": "weil"}})).unwrap();
    assert_eq!(r["max"], 5.0);
    assert_eq!(r["points"], 4.0, "{r}");
    assert_eq!(r["open"], 1);
    assert_eq!(r["results"][1]["correct"], false);
    assert!(r["results"][4]["correct"].is_null(), "free text is the teacher's");
    assert!(engine.call("worksheet.check", json!({"id": id, "answers": "nope"})).unwrap()["points"] == 0.0);

    // Edit tasks: add, change, move, remove.
    engine.call("worksheet.update_task", json!({"id": id, "task": {"type": "ordering", "prompt": "Sortiere", "items": ["1/4", "1/2", "3/4"]}})).unwrap();
    engine.call("worksheet.update_task", json!({"id": id, "index": 0, "task": {"type": "true_false", "prompt": "Neu", "answer": false}})).unwrap();
    let out = engine.call("worksheet.update_task", json!({"id": id, "index": 5, "move_to": 0})).unwrap();
    assert_eq!(out["tasks"], 6);
    engine.call("worksheet.update_task", json!({"id": id, "index": 1, "remove": true})).unwrap();
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    let (ws, problems) = aiscraft_core::worksheet::parse_lenient(&text).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(ws.tasks.len(), 5);
    assert_eq!(ws.tasks[0].body.type_name(), "ordering");
    assert!(engine.call("worksheet.update_task", json!({"id": id, "index": 99, "remove": true})).is_err());
    assert!(engine.call("worksheet.update_task", json!({"id": id, "task": {"type": "quatsch"}})).is_err());

    // Export: PDF sheet and solutions, Word.
    let dir = tmp_dir("ws");
    let pdf = dir.join("a.pdf");
    engine.call("worksheet.export", json!({"id": id, "format": "pdf", "path": pdf.display().to_string()})).unwrap();
    let a = String::from_utf8_lossy(&std::fs::read(&pdf).unwrap()).into_owned();
    assert!(a.starts_with("%PDF") && a.contains("Sortiere"));
    assert!(!a.contains("Kuerzen mit 2"), "no solutions on the sheet");
    let sol = engine.call("worksheet.export", json!({"id": id, "format": "pdf", "variant": "loesung"})).unwrap();
    use base64::Engine as _;
    let sol = String::from_utf8_lossy(&base64::engine::general_purpose::STANDARD.decode(sol["base64"].as_str().unwrap()).unwrap()).into_owned();
    assert!(sol.contains("Kuerzen mit 2"), "solutions on the solution sheet");
    let docx = dir.join("a.docx");
    engine.call("worksheet.export", json!({"id": id, "format": "docx", "path": docx.display().to_string()})).unwrap();
    assert!(std::fs::read(&docx).unwrap().starts_with(b"PK"));
    assert!(engine.call("worksheet.export", json!({"id": id, "format": "html"})).is_err(), "no interactive HTML export for worksheets");
    let via_artifact = engine.call("artifact.export", json!({"id": id, "format": "json"})).unwrap();
    assert!(via_artifact["text"].as_str().unwrap().contains("aiscraft-worksheet"));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(engine.call("worksheet.print", json!({"id": id})).unwrap_err().kind, aiscraft_core::ErrorKind::Unsupported);

    // Revise through the model: a new version.
    engine.call("artifact.revise", json!({"id": id, "instruction": "leichter"})).unwrap();
    assert!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().contains("Bruchrechnung leicht"));

    // From a document, through a model turn.
    let doc = engine.call("artifact.create", json!({"kind": "document", "content": "# Brüche\n\n1. Was ist größer, 1/2 oder 1/3?"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let out = engine.call("worksheet.from_document", json!({"id": doc})).unwrap();
    assert_eq!(out["tasks"], 5);
    assert!(engine.call("worksheet.from_document", json!({"id": id})).is_err(), "only documents");
}

/// Broken JSON from a model: reported, repaired once by the model, never a crash.
#[test]
fn worksheet_blocks_are_parsed_leniently_and_repaired() {
    let engine = routed_engine();
    // Hostile but readable: aliases, unknown fields, one broken task.
    let src = "```aiscraft-worksheet\n{\"titel\":\"Test\",\"aufgaben\":[{\"typ\":\"true_false\",\"frage\":\"Ja?\",\"answer\":\"ja\",\"x\":[1,2]},{\"type\":\"warp\"}]}\n```";
    let out = engine.call("worksheet.create", json!({"content": src})).unwrap();
    assert_eq!(out["tasks"], 1);
    assert!(out["problems"][0].as_str().unwrap().contains("unbekannten Typ"));
    // Not JSON at all: without repair an error, with repair the model fixes it.
    assert!(engine.call("worksheet.create", json!({"content": "{\"tasks\": [ kaputt"})).is_err());
    let out = engine.call("worksheet.create", json!({"content": "{\"tasks\": [ kaputt", "repair": true})).unwrap();
    assert_eq!(out["tasks"], 5);
    // As a tool argument object, and through artifact.create.
    let out = engine.call("worksheet.create", json!({"worksheet": sheet_json()})).unwrap();
    assert_eq!(out["tasks"], 5);
    assert!(engine.call("artifact.create", json!({"kind": "worksheet", "content": sheet_json().to_string()})).is_ok());
    assert!(engine.call("artifact.create", json!({"kind": "app", "content": APP_V2})).is_ok());
    for bad in [json!(null), json!("x"), json!({"content": "ä".repeat(600_000)}), json!({"worksheet": [1, 2]}), json!({"content": "[]"})] {
        assert!(engine.call("worksheet.create", bad).is_err());
    }
}
