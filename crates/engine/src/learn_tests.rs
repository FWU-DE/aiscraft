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

fn h5p_json() -> Value {
    json!({
        "format": "aiscraft-h5p", "version": 1, "title": "Igel", "intro": "Teste dein Wissen.",
        "tasks": [
            {"type": "choice", "prompt": "Was frisst der Igel?", "options": [{"text": "Insekten", "correct": true}, {"text": "Gras", "feedback": "Igel fressen Tiere"}]},
            {"type": "true_false", "prompt": "Igel halten Winterschlaf.", "answer": true},
            {"type": "cloze", "prompt": "Ergänze.", "text": "Der Igel hat [[Stacheln]]."},
            {"type": "cloze", "prompt": "Ziehe.", "text": "Igel sind [[nachtaktiv]].", "word_bank": true, "distractors": ["tagaktiv"]}
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
    if req.contains("Werkzeug h5p_create") && !has_tool_result {
        return tool("h5p_create", json!({"h5p": h5p_json()}));
    }
    if req.contains("überarbeitest H5P-Inhalt") {
        let mut h = h5p_json();
        h["title"] = json!("Igel leicht");
        return text(&format!("```aiscraft-h5p\n{h}\n```"));
    }
    if req.contains("reparierst H5P-Inhalt") || (req.contains("H5P-Quiz zum Thema") && !has_tool_result) {
        return text(&format!("Hier dein Quiz:\n```aiscraft-h5p\n{}\n```", h5p_json()));
    }
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
        ("worksheet.request", "worksheet", worksheet_prompt("Bruchrechnung (Klasse: 6)", 8, false, true)),
        ("learning_app.request", "learning_app", app_prompt("Bruchrechnung (Klasse: 6)", false)),
        ("h5p.request", "h5p", crate::learn::h5p::h5p_prompt("Bruchrechnung (Klasse: 6)", 6, false)),
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
        assert!(r.spec.contains(&(r.format.prompt_description)(&formats::PromptOptions::default())), "{command}: the folded part is the spec");
        // The export carries a one-line note, not the spec.
        let md = engine.call("chat.export", json!({})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(md.contains("Bruchrechnung (Klasse: 6)") && md.contains(&format!("_Mit Formatvorgabe: {}_", r.format.label)), "{md}");
        assert!(!md.contains(&(r.format.prompt_description)(&formats::PromptOptions::default())), "{command}: no spec in the export");
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

/// Worksheets without points (#72): the setting makes new sheets from the tools path go without
/// points and tells the model so; checking counts right tasks; the flag survives export, model
/// updates and revisions; the per-sheet switch brings the task points back.
#[test]
fn worksheets_without_points_from_the_setting() {
    let (engine, log) = routed_engine_logged();
    assert!(engine.state().settings.worksheet_points, "on by default");
    engine.call("settings.set", json!({"key": "worksheet_points", "value": false})).unwrap();
    let out = engine.call("worksheet.request", json!({"topic": "Bruchrechnung", "count": 5})).unwrap();
    assert_eq!(out["created"], true, "{out}");
    let s = engine.state();
    assert!(s.messages.iter().any(|m| m.tool_calls.iter().any(|t| t.name == "worksheet_create")), "made by the tool");
    let id = s.artifacts.iter().find(|a| a.kind == aiscraft_core::ArtifactKind::Worksheet).unwrap().id.clone();
    // The model was told: in the request and in the tool description.
    let sent = log.lock().unwrap().clone();
    let first = sent.first().unwrap().to_string();
    assert!(first.contains("keine Punkte") && first.contains("(ohne Punkte)"), "the request says so");
    let tool = sent.first().unwrap()["tools"].as_array().unwrap().iter().find(|t| t["function"]["name"] == "worksheet_create").unwrap().to_string();
    assert!(tool.contains("keine Punkte"), "{tool}");
    // The model sent points anyway: the sheet has none, the task points are kept.
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("\"points\": false") && text.contains("\"points\": 2"), "{text}");
    let r = engine.call("worksheet.check", json!({"id": id, "answers": {"t1": [0], "t2": false, "t3": ["0.5"], "t4": "0,25", "t5": "weil"}})).unwrap();
    assert!(r.get("points").is_none() && r.get("max").is_none() && r["results"][0].get("points").is_none(), "{r}");
    assert_eq!((r["right"].as_u64(), r["checked"].as_u64(), r["open"].as_u64()), (Some(3), Some(4), Some(1)), "{r}");
    // JSON export keeps the flag; the printout names no points.
    let json_out = engine.call("worksheet.export", json!({"id": id, "format": "json"})).unwrap();
    let decoded = String::from_utf8(decode(&json_out)).unwrap();
    assert!(decoded.contains("\"points\": false"));
    for variant in ["arbeitsblatt", "loesung"] {
        let md = engine.call("artifact.export", json!({"id": id, "format": "markdown", "variant": variant})).unwrap()["text"].as_str().unwrap().to_string();
        assert!(md.contains("Welcher Bruch") && !md.contains("Punkt") && !md.contains("Gesamt"), "{variant}: {md}");
    }
    // A model update and an AI revision keep the teacher's switch.
    let r = crate::learn::run_tool(engine.ctx(), "worksheet_update", &json!({"id": id, "worksheet": sheet_json()})).unwrap().unwrap();
    assert!(r.contains("aktualisiert"));
    engine.call("artifact.revise", json!({"id": id, "instruction": "leichter"})).unwrap();
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("Bruchrechnung leicht") && text.contains("\"points\": false"), "{text}");
    // Switched back on: the points are there again.
    engine.call("worksheet.set_points", json!({"id": id, "points": true})).unwrap();
    let r = engine.call("worksheet.check", json!({"id": id, "answers": {"t1": [0], "t3": ["0.5"]}})).unwrap();
    assert_eq!(r["points"], 3.0, "{r}");
    assert_eq!(r["max"], 5.0);
    assert!(!engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().contains("\"points\": false"));
    assert!(engine.call("worksheet.set_points", json!({"id": id})).is_err(), "points must be given");
    assert!(engine.call("worksheet.set_points", json!({"id": "nope", "points": false})).is_err());
    // With the setting on again, a new sheet counts points.
    engine.call("settings.set", json!({"key": "worksheet_points", "value": true})).unwrap();
    let out = engine.call("worksheet.create", json!({"worksheet": sheet_json()})).unwrap();
    let text = engine.call("artifact.get", json!({"id": out["id"]})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(!text.contains("\"points\": false"));
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

/// Sorting into groups, hints one after the other, option feedback and word lists through the
/// commands: validated, stored, checked and printed.
#[test]
fn worksheet_groups_hints_feedback_and_word_bank() {
    let engine = routed_engine();
    let sheet = json!({"title": "Tiere", "tasks": [
        {"id": "g", "type": "categorize", "prompt": "Sortiere die Tiere", "points": 4,
         "categories": [{"name": "Saeugetiere", "items": ["Wal", "Fledermaus"]}, {"name": "Fische", "items": ["Hai", "Forelle"]}]},
        {"id": "c", "type": "cloze", "prompt": "Ergaenze", "text": "Der Wal ist ein [[Saeugetier]].", "word_bank": true, "distractors": ["Fisch"],
         "hints": ["Wie atmet er?", "Er hat Lungen."]},
        {"id": "m", "type": "choice", "prompt": "Wer atmet mit Lungen?", "options": [{"text": "Wal", "correct": true}, {"text": "Hai", "feedback": "Haie haben Kiemen"}]}
    ]});
    let v = engine.call("worksheet.validate", json!({"content": sheet.to_string()})).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let id = engine.call("worksheet.create", json!({"worksheet": sheet})).unwrap()["id"].as_str().unwrap().to_string();
    // Group answers as indices or names; half right gives half the points.
    let r =
        engine.call("worksheet.check", json!({"id": id, "answers": {"g": ["Saeugetiere", 0, "Saeugetiere", null], "c": ["saeugetier"], "m": [0]}})).unwrap();
    assert_eq!(r["results"][0]["points"], 2.0, "{r}");
    assert_eq!(r["results"][0]["parts"], json!([true, true, false, false]));
    assert_eq!(r["results"][1]["correct"], true);
    assert_eq!(r["points"], 4.0);
    // Stored with the new fields.
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(text.contains("\"categorize\"") && text.contains("\"hints\"") && text.contains("Haie haben Kiemen") && text.contains("\"word_bank\": true"));
    // Printed: word list and groups on the sheet, filled groups and option feedback on the solutions.
    use base64::Engine as _;
    let pdf = |variant: &str| {
        let out = engine.call("worksheet.export", json!({"id": id, "format": "pdf", "variant": variant})).unwrap();
        String::from_utf8_lossy(&base64::engine::general_purpose::STANDARD.decode(out["base64"].as_str().unwrap()).unwrap()).into_owned()
    };
    let (a, l) = (pdf("arbeitsblatt"), pdf("loesung"));
    assert!(a.contains("Saeugetiere") && a.contains("Fisch") && a.contains("Wie atmet er?") && !a.contains("Er hat Lungen"));
    assert!(l.contains("Haie haben Kiemen") && l.contains("Er hat Lungen"));
    assert!(!a.contains("Haie haben Kiemen"));
}

// ---- H5P (#59) -------------------------------------------------------------------------------

/// Reads a package the way an LMS would start to: the zip, `h5p.json` and `content.json`.
fn unzip_h5p(bytes: &[u8]) -> (Value, Value, Vec<String>) {
    let mut zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: 1000, max_total_bytes: 50 << 20 }).unwrap();
    assert!(!zip.has_duplicate_names());
    let names: Vec<String> = zip.names().map(|n| n.to_string()).collect();
    let meta: Value = serde_json::from_slice(&zip.read("h5p.json", 1 << 20).unwrap().unwrap()).unwrap();
    let content: Value = serde_json::from_slice(&zip.read("content/content.json", 4 << 20).unwrap().unwrap()).unwrap();
    // What H5P's validator demands of h5p.json.
    let title = meta["title"].as_str().unwrap();
    assert!(!title.is_empty() && title.chars().count() <= 255);
    assert!(meta["language"].as_str().unwrap().chars().all(|c| c.is_ascii_alphabetic() || c == '-'));
    let main = meta["mainLibrary"].as_str().unwrap();
    let deps = meta["preloadedDependencies"].as_array().unwrap();
    assert!(deps.iter().any(|d| d["machineName"] == main), "the main library is a dependency");
    for d in deps {
        assert!(d["machineName"].as_str().unwrap().starts_with("H5P.") || d["machineName"].as_str().unwrap() == "FontAwesome");
        assert!(d["majorVersion"].as_u64().is_some() || d["majorVersion"].as_str().is_some());
    }
    assert!(meta["embedTypes"].as_array().unwrap().iter().all(|e| e == "iframe" || e == "div"));
    // Every sub-content's library is listed.
    for q in content["questions"].as_array().into_iter().flatten() {
        let lib = q["library"].as_str().unwrap();
        let (name, ver) = lib.split_once(' ').unwrap();
        let (major, minor) = ver.split_once('.').unwrap();
        assert!(
            deps.iter().any(|d| d["machineName"] == name
                && d["majorVersion"].to_string().trim_matches('"') == major
                && d["minorVersion"].to_string().trim_matches('"') == minor),
            "{lib} is in preloadedDependencies"
        );
        assert_eq!(q["subContentId"].as_str().unwrap().len(), 36);
    }
    (meta, content, names)
}

fn decode(v: &Value) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(v["base64"].as_str().unwrap()).unwrap()
}

/// H5P without tools: requested as a block, validated, edited as a task list, saved as a
/// package that has what an LMS needs, revised by the model.
#[test]
fn h5p_from_a_block_edited_and_saved() {
    let engine = routed_engine();
    engine.call("settings.set", json!({"key": "tools", "value": false})).unwrap();
    let out = engine.call("h5p.request", json!({"topic": "Igel", "grade": "3"})).unwrap();
    assert_eq!(out["created"], true, "{out}");
    let id = out["id"].as_str().unwrap().to_string();
    let a = engine.state().artifacts.iter().find(|a| a.id == id).unwrap().clone();
    assert_eq!((a.kind, a.title.as_str()), (aiscraft_core::ArtifactKind::H5p, "Igel"));
    assert!(engine.state().messages[0].content.contains("```aiscraft-h5p"));

    let v = engine.call("h5p.validate", json!({"content": h5p_json().to_string()})).unwrap();
    assert_eq!((v["ok"].clone(), v["tasks"].clone()), (json!(true), json!(4)), "{v}");
    let v = engine.call("h5p.validate", json!({"content": "{\"tasks\": [{\"type\": \"matching\", \"pairs\": []}]}"})).unwrap();
    assert_eq!(v["ok"], false);

    // Saved: a QuestionSet with the four content types.
    let dir = tmp_dir("h5p");
    let path = dir.join("igel.h5p");
    let saved = engine.call("h5p.export", json!({"id": id, "path": path.display().to_string()})).unwrap();
    assert_eq!(saved["path"], path.display().to_string());
    let (meta, content, names) = unzip_h5p(&std::fs::read(&path).unwrap());
    assert_eq!(names, vec!["h5p.json", "content/content.json"]);
    assert_eq!(meta["mainLibrary"], "H5P.QuestionSet");
    assert_eq!(meta["title"], "Igel");
    assert_eq!(content["introPage"]["introduction"], "<p>Teste dein Wissen.</p>");
    let libs: Vec<&str> = content["questions"].as_array().unwrap().iter().map(|q| q["library"].as_str().unwrap()).collect();
    assert_eq!(libs, vec!["H5P.MultiChoice 1.16", "H5P.TrueFalse 1.8", "H5P.Blanks 1.14", "H5P.DragText 1.10"]);
    assert_eq!(content["questions"][2]["params"]["questions"][0], "<p>Der Igel hat *Stacheln*.</p>");
    assert_eq!(content["questions"][3]["params"]["textField"], "Igel sind *nachtaktiv*.");
    // The same through artifact.export, and the editable JSON.
    let again = decode(&engine.call("artifact.export", json!({"id": id, "format": "h5p"})).unwrap());
    assert_eq!(again, std::fs::read(&path).unwrap(), "deterministic packages");
    let as_json = engine.call("artifact.export", json!({"id": id, "format": "json"})).unwrap();
    assert!(as_json["text"].as_str().unwrap().contains("aiscraft-h5p"));
    assert!(engine.call("artifact.export", json!({"id": id, "format": "pdf"})).is_err());

    // Task list: add, change, move, remove; types H5P has not are refused.
    engine.call("h5p.update_task", json!({"id": id, "task": {"type": "true_false", "prompt": "Igel sind Säugetiere.", "answer": true}})).unwrap();
    engine.call("h5p.update_task", json!({"id": id, "index": 1, "task": {"type": "true_false", "prompt": "Igel können fliegen.", "answer": false}})).unwrap();
    let out = engine.call("h5p.update_task", json!({"id": id, "index": 4, "move_to": 0})).unwrap();
    assert_eq!(out["tasks"], 5);
    engine.call("h5p.update_task", json!({"id": id, "index": 2, "remove": true})).unwrap();
    let e = engine.call("h5p.update_task", json!({"id": id, "task": {"type": "ordering", "prompt": "x", "items": ["a", "b"]}})).unwrap_err();
    assert!(e.message.contains("ordering"), "{}", e.message);
    assert!(engine.call("h5p.update_task", json!({"id": id, "index": 99, "remove": true})).is_err());
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    let (h, _) = aiscraft_core::h5p::parse_stored(&text).unwrap();
    assert_eq!(h.tasks.len(), 4);
    assert_eq!(h.tasks[0].prompt, "Igel sind Säugetiere.");
    assert!(!h.tasks.iter().any(|t| t.prompt == "Igel halten Winterschlaf."));
    let (_, content, _) = unzip_h5p(&decode(&engine.call("h5p.export", json!({"id": id})).unwrap()));
    assert_eq!(content["questions"].as_array().unwrap().len(), 4);

    // Revised by the model: a new version.
    engine.call("artifact.revise", json!({"id": id, "instruction": "leichter"})).unwrap();
    assert!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().contains("Igel leicht"));
    // From an answer into the open artifact, and as a new one through artifact.create.
    let mid = engine.state().messages.iter().find(|m| m.role == aiscraft_core::Role::Assistant && m.content.contains("```aiscraft-h5p")).unwrap().id.clone();
    let out = engine.call("h5p.create", json!({"from_message": mid, "id": id})).unwrap();
    assert_eq!(out["id"], id);
    assert!(engine.call("artifact.create", json!({"kind": "h5p", "content": h5p_json().to_string()})).is_ok());
    for bad in
        [json!({}), json!({"content": "kein json"}), json!({"h5p": {"tasks": [{"type": "free_text", "prompt": "x"}]}}), json!({"content": "{}", "id": "nope"})]
    {
        assert!(engine.call("h5p.create", bad.clone()).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// With tools, the model calls h5p_create; h5p_update keeps the artifact.
#[test]
fn h5p_with_tools() {
    let engine = routed_engine();
    let out = engine.call("h5p.request", json!({"topic": "Igel", "count": 4})).unwrap();
    assert_eq!(out["created"], true, "{out}");
    let s = engine.state();
    assert!(s.messages.iter().any(|m| m.tool_calls.iter().any(|t| t.name == "h5p_create")));
    let id = s.artifacts.iter().find(|a| a.kind == aiscraft_core::ArtifactKind::H5p).unwrap().id.clone();
    let mut h = h5p_json();
    h["title"] = json!("Igel neu");
    let r = crate::learn::run_tool(engine.ctx(), "h5p_update", &json!({"id": id, "h5p": h})).unwrap().unwrap();
    assert!(r.contains("aktualisiert"));
    assert!(engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().contains("Igel neu"));
    assert!(crate::learn::run_tool(engine.ctx(), "h5p_update", &json!({"id": "nope", "h5p": h})).unwrap().is_err());
    assert!(crate::learn::run_tool(engine.ctx(), "h5p_create", &json!({"h5p": "{\"tasks\": []}"})).unwrap().is_err());
}

fn package(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = craft_zip::Writer::new();
    for (n, d) in entries {
        w.add(n, d, craft_zip::Compression::Deflated).unwrap();
    }
    w.finish().unwrap()
}

/// Opening `.h5p` files: a quiz of known types becomes editable and keeps its pictures and
/// library versions; unchanged it saves byte for byte; other content types are kept and saved
/// unchanged; hostile files are refused. The package lives in the attachments folder.
#[test]
fn h5p_files_open_edit_and_save_back() {
    let engine = routed_engine();
    let dir = tmp_dir("h5p-open");
    let meta = json!({"title": "Vom LMS", "language": "de", "mainLibrary": "H5P.QuestionSet", "embedTypes": ["iframe"], "license": "CC BY",
        "authors": [{"name": "Frau K.", "role": "Author"}],
        "preloadedDependencies": [{"machineName": "H5P.QuestionSet", "majorVersion": "1", "minorVersion": "17"}, {"machineName": "H5P.MultiChoice", "majorVersion": "1", "minorVersion": "14"}, {"machineName": "FontAwesome", "majorVersion": "4", "minorVersion": "5"}]});
    let content = json!({"introPage": {"showIntroPage": true, "introduction": "<p>Los &amp; viel Gl&uuml;ck</p>"}, "questions": [
        {"library": "H5P.MultiChoice 1.14", "params": {"media": {"type": {"library": "H5P.Image 1.1", "params": {"file": {"path": "images/igel.png"}}}}, "question": "<p>Was frisst der Igel?</p>", "answers": [{"text": "<div>Insekten</div>", "correct": true, "tipsAndFeedback": {"chosenFeedback": "<p>Ja!</p>"}}, {"text": "<div>Gras</div>", "correct": false}]}, "subContentId": "x"}
    ]});
    let bytes = package(&[
        ("h5p.json", meta.to_string().as_bytes()),
        ("content/content.json", content.to_string().as_bytes()),
        ("content/images/igel.png", b"\x89PNG bild"),
        ("H5P.MultiChoice-1.14/library.json", b"{}"),
        ("H5P.MultiChoice-1.14/js/multichoice.js", b"/* code */"),
    ]);
    let file = dir.join("vom-lms.h5p");
    std::fs::write(&file, &bytes).unwrap();
    let out = engine.call("h5p.open", json!({"path": file.display().to_string()})).unwrap();
    assert_eq!((out["editable"].clone(), out["tasks"].clone(), out["main_library"].clone()), (json!(true), json!(1), json!("H5P.QuestionSet 1.17")), "{out}");
    let id = out["id"].as_str().unwrap().to_string();
    let a = engine.state().artifacts.iter().find(|a| a.id == id).unwrap().clone();
    assert_eq!((a.kind, a.title.as_str()), (aiscraft_core::ArtifactKind::H5p, "Vom LMS"));
    let text = engine.call("artifact.get", json!({"id": id})).unwrap()["text"].as_str().unwrap().to_string();
    let (h, _) = aiscraft_core::h5p::parse_stored(&text).unwrap();
    assert_eq!(h.intro, "Los & viel Glück");
    assert_eq!(h.tasks[0].prompt, "Was frisst der Igel?");
    let pkg = h.package.clone().unwrap();
    assert!(pkg.notes.iter().any(|n| n.contains("Bilder")), "{:?}", pkg.notes);
    // The bytes are in the attachments folder, not in the database.
    assert!(engine.ctx().0.attachments_dir.join(&pkg.file).is_file());
    assert!(!text.contains("PNG"));

    // Unchanged: saved byte for byte.
    assert_eq!(decode(&engine.call("h5p.export", json!({"id": id})).unwrap()), bytes);
    // Edited: rebuilt, pictures and libraries kept, the package's versions and fields kept.
    engine.call("h5p.update_task", json!({"id": id, "task": {"type": "choice", "prompt": "Wann schläft der Igel?", "options": [{"text": "Im Winter", "correct": true}, {"text": "Im Sommer"}]}})).unwrap();
    let (meta2, content2, names) = unzip_h5p(&decode(&engine.call("h5p.export", json!({"id": id})).unwrap()));
    assert!(names.contains(&"content/images/igel.png".to_string()) && names.contains(&"H5P.MultiChoice-1.14/js/multichoice.js".to_string()), "{names:?}");
    assert_eq!(content2["questions"].as_array().unwrap().len(), 2);
    assert_eq!(content2["questions"][1]["library"], "H5P.MultiChoice 1.14");
    assert_eq!(meta2["authors"][0]["name"], "Frau K.");
    assert_eq!(meta2["license"], "CC BY");
    assert_eq!(meta2["preloadedDependencies"][0]["minorVersion"], 17);

    // Another content type: shown, kept, saved unchanged, not editable.
    let video = package(&[
        ("h5p.json", br#"{"title":"Film","mainLibrary":"H5P.InteractiveVideo","preloadedDependencies":[{"machineName":"H5P.InteractiveVideo","majorVersion":1,"minorVersion":26}]}"#),
        ("content/content.json", br#"{"interactiveVideo":{}}"#),
        ("content/videos/film.mp4", b"mp4"),
    ]);
    let vfile = dir.join("film.h5p");
    std::fs::write(&vfile, &video).unwrap();
    let out = engine.call("h5p.open", json!({"path": vfile.display().to_string()})).unwrap();
    assert_eq!((out["editable"].clone(), out["main_library"].clone()), (json!(false), json!("H5P.InteractiveVideo 1.26")));
    let vid = out["id"].as_str().unwrap().to_string();
    assert_eq!(decode(&engine.call("h5p.export", json!({"id": vid})).unwrap()), video);
    assert_eq!(decode(&engine.call("artifact.export", json!({"id": vid})).unwrap()), video);
    assert_eq!(
        engine.call("h5p.update_task", json!({"id": vid, "task": {"type": "true_false", "prompt": "x", "answer": true}})).unwrap_err().kind,
        aiscraft_core::ErrorKind::Unsupported
    );
    assert!(engine.call("artifact.revise", json!({"id": vid, "instruction": "x"})).is_err());
    assert!(engine.call("h5p.create", json!({"id": vid, "content": h5p_json().to_string()})).is_err());

    // Hostile files.
    let evil = dir.join("evil.h5p");
    for entries in [
        vec![("h5p.json", meta.to_string().into_bytes()), ("content/content.json", content.to_string().into_bytes()), ("../../.bashrc", b"x".to_vec())],
        vec![("h5p.json", meta.to_string().into_bytes()), ("content/content.json", content.to_string().into_bytes()), ("content/x.exe", b"MZ".to_vec())],
        vec![("content/content.json", content.to_string().into_bytes())],
    ] {
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        std::fs::write(&evil, package(&refs)).unwrap();
        assert_eq!(engine.call("h5p.open", json!({"path": evil.display().to_string()})).unwrap_err().kind, aiscraft_core::ErrorKind::Invalid);
    }
    std::fs::write(&evil, b"kein zip").unwrap();
    assert!(engine.call("h5p.open", json!({"path": evil.display().to_string()})).is_err());
    assert!(engine.call("h5p.open", json!({"path": dir.join("gibt-es-nicht.h5p").display().to_string()})).is_err());
    assert!(engine.call("h5p.open", json!({"path": dir.display().to_string()})).is_err());
    // A stored package that vanished: a message, no crash.
    std::fs::remove_file(engine.ctx().0.attachments_dir.join(&pkg.file)).unwrap();
    assert_eq!(engine.call("h5p.export", json!({"id": id})).unwrap_err().kind, aiscraft_core::ErrorKind::NotFound);
    let _ = std::fs::remove_dir_all(dir);
}
