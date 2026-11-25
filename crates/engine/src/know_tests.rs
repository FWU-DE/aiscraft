//! Tests for project knowledge (#32), research (#37), voice mode (#40), connector results for
//! AIS.chat (#53) and templates (#54). In the list `every_command_has_a_functional_test` reads.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

/// One request a scripted server saw: the first line and the body.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    pub line: String,
    pub body: String,
}

/// A local HTTP server: `route(first line, body)` gives (content type, body). Bodies of
/// `text/event-stream` are sent as they are. Every request is recorded.
pub(crate) fn scripted(engine: &Engine, route: impl Fn(&str, &str) -> (String, Vec<u8>) + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let seen = Arc::new(Mutex::new(Vec::<Seen>::new()));
    let seen2 = seen.clone();
    let route = Arc::new(route);
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                let route = route.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    let mut head_end = None;
                    loop {
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..k]);
                        if head_end.is_none() {
                            head_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
                        }
                        if let Some(h) = head_end {
                            let head = String::from_utf8_lossy(&buf[..h]).to_lowercase();
                            let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let h = head_end.unwrap_or(buf.len());
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let line = text.lines().next().unwrap_or("").to_string();
                    let body = String::from_utf8_lossy(buf.get(h + 4..).unwrap_or(&[])).to_string();
                    seen.lock().unwrap().push(Seen { line: line.clone(), body: body.clone() });
                    let (ctype, out) = route(&line, &body);
                    let head = if ctype == "text/event-stream" {
                        format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\n\r\n")
                    } else {
                        format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", out.len())
                    };
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(&out).await;
                });
            }
        });
        format!("http://{addr}")
    });
    (url, seen)
}

/// An OpenAI-compatible answer stream with `text`.
pub(crate) fn sse(text: &str) -> (String, Vec<u8>) {
    let c = json!({"choices":[{"delta":{"content": text}}]});
    ("text/event-stream".into(), format!("data: {c}\n\ndata: [DONE]\n\n").into_bytes())
}

pub(crate) fn models() -> (String, Vec<u8>) {
    ("application/json".into(), br#"{"data":[{"id":"m","context_length":100000}]}"#.to_vec())
}

/// An engine with an OpenAI-compatible backend at `url` (with a key) and a new chat.
pub(crate) fn engine_at(engine: &Engine, url: &str) -> String {
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": format!("{url}/v1"), "name": "Test", "secret": "sk-test"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    id
}

fn posts(seen: &Arc<Mutex<Vec<Seen>>>, path: &str) -> Vec<String> {
    seen.lock().unwrap().iter().filter(|s| s.line.starts_with("POST") && s.line.contains(path)).map(|s| s.body.clone()).collect()
}

/// #54: templates with fields: starters on a fresh profile, save with fields, fill into the
/// composer, missing fields named, send, values remembered, delete.
#[test]
fn templates_fill_fields_and_send() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, |line, _| if line.starts_with("GET") { models() } else { sse("Liebe Eltern …") });
    engine_at(&engine, &url);
    let starters: Vec<String> = engine.state().templates.iter().map(|t| t.title.clone()).collect();
    assert!(starters.contains(&"Brief an die Eltern".to_string()) && starters.len() == 3, "{starters:?}");

    let out =
        engine.call("template.save", json!({"title": "Wandertag", "text": "Brief an {{Klasse}} zum Wandertag am {{Datum}}, Ton {{Ton=locker}}."})).unwrap();
    let id = out["id"].as_str().unwrap().to_string();
    let fields: Vec<&str> = out["fields"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(fields, ["Klasse", "Datum", "Ton"]);
    assert!(engine.call("template.save", json!({"title": " ", "text": "x"})).is_err());

    let err = engine.call("template.use", json!({"id": id, "values": {"Klasse": "7b"}})).unwrap_err();
    assert!(err.message.contains("Datum"), "{}", err.message);
    let out = engine.call("template.use", json!({"id": id, "values": {"Klasse": "7b", "Datum": "3. Mai"}})).unwrap();
    assert_eq!(out["text"], "Brief an 7b zum Wandertag am 3. Mai, Ton locker.");
    let fill = engine.state().composer_fill.clone().unwrap();
    assert!(fill.replace && fill.text == out["text"].as_str().unwrap(), "goes into the composer");
    assert!(posts(&seen, "/chat/completions").is_empty(), "nothing sent yet");
    let t = engine.state().templates.iter().find(|t| t.id == id).cloned().unwrap();
    assert_eq!(t.last.get("Klasse").map(String::as_str), Some("7b"), "values are offered next time");

    let out = engine.call("template.use", json!({"id": id, "values": {"Klasse": "8a", "Datum": "4. Mai", "Ton": "förmlich"}, "send": true})).unwrap();
    assert_eq!(out["sent"], true);
    let sent = posts(&seen, "/chat/completions");
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("Brief an 8a zum Wandertag am 4. Mai, Ton förmlich."));

    engine.call("template.delete", json!({"id": id})).unwrap();
    assert!(engine.state().templates.iter().all(|t| t.id != id));
    assert!(engine.call("template.delete", json!({"id": id})).is_err());
    assert!(engine.call("template.use", json!({"id": "nope"})).is_err());
}

/// #53: a read-only connector tool with the person's own arguments; the result goes into the
/// composer and nothing is sent; tools that change something and vault values are refused; the
/// action log names the call.
#[test]
fn connector_result_goes_into_the_composer() {
    let Some(bin) = crate::trust_tests::fake_server() else {
        eprintln!("skipped: build it with `cargo build -p aiscraft-mcp --example fake_server`");
        return;
    };
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, |line, _| if line.starts_with("GET") { models() } else { sse("ok") });
    engine_at(&engine, &url);
    engine.call("connector.add", json!({"name": "Kalender", "command": bin.display().to_string()})).unwrap();
    let s = engine.state();
    let echo = s.connectors[0].tools.iter().find(|t| t.name == "echo").unwrap().llm_name.clone();
    let notify = s.connectors[0].tools.iter().find(|t| t.name == "notify").unwrap().llm_name.clone();

    let out = engine.call("connector.insert", json!({"tool": echo, "arguments": {"text": "Elternabend am Dienstag"}})).unwrap();
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("Elternabend am Dienstag") && text.starts_with("[Aus Kalender"), "{text}");
    assert!(!text.contains("untrusted_tool_result"), "the person reads it as it is");
    let fill = engine.state().composer_fill.clone().unwrap();
    assert!(!fill.replace && fill.text == text, "added to the composer");
    assert!(posts(&seen, "/chat/completions").is_empty(), "nothing leaves before sending");

    let err = engine.call("connector.insert", json!({"tool": notify, "arguments": {}})).unwrap_err();
    assert!(err.message.contains("nur lesen"), "{}", err.message);
    assert!(engine.call("connector.insert", json!({"tool": echo, "arguments": {"text": "{{vault:x}}"}})).is_err());
    assert!(engine.call("connector.insert", json!({"tool": "gibt_es_nicht"})).is_err());
    assert!(engine.call("connector.insert", json!({"tool": echo, "arguments": "text"})).is_err());

    let log = engine.call("log.list", json!({})).unwrap();
    assert!(log.to_string().contains("Kalender"), "the call is in the action log: {log}");
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("aiscraft-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// #32: files in a project: kept, text read (also from a PDF), shown with how much of the
/// context they take; every chat in the project takes them along (all of it when small, the
/// fitting parts when large), names protected; switched off per chat; gone with the project.
#[test]
fn project_knowledge_goes_along_with_every_chat_in_the_project() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, |line, _| {
        if line.starts_with("GET") { ("application/json".into(), br#"{"data":[{"id":"m","context_length":10000}]}"#.to_vec()) } else { sse("Gern.") }
    });
    engine_at(&engine, &url);
    let dir = temp_dir("knowledge");
    let txt = dir.join("lehrplan.txt");
    std::fs::write(&txt, "Lehrplan Biologie 7: Igel und Winterschlaf. Ansprechpartnerin ist Lea Müller.").unwrap();
    let pdf = dir.join("arbeitsblatt.pdf");
    std::fs::write(&pdf, crate::office::pdf::markdown_pdf("Blatt", "# Arbeitsblatt\n\nDie Fotosynthese braucht Licht.").unwrap()).unwrap();
    let pid = engine.call("project.create", json!({"name": "Bio 7b"})).unwrap()["id"].as_str().unwrap().to_string();
    let a = engine.call("project.file_add", json!({"project": pid, "path": txt.display().to_string()})).unwrap();
    engine.call("project.file_add", json!({"project": pid, "path": pdf.display().to_string()})).unwrap();
    assert!(engine.call("project.file_add", json!({"project": "nope", "path": txt.display().to_string()})).is_err());
    assert!(engine.call("project.file_add", json!({"project": pid, "path": dir.display().to_string()})).is_err());
    let listed = engine.call("project.files", json!({"project": pid})).unwrap();
    assert_eq!(listed["files"].as_array().unwrap().len(), 2);
    assert!(listed["usage"]["tokens"].as_u64().unwrap() > 0 && listed["usage"]["whole"] == true, "{listed}");
    assert_eq!(listed["usage"]["window"], 10000);

    // A chat in the project: both files go along, the name protected.
    engine.call("privacy.list_add", json!({"text": "Lea Müller"})).unwrap();
    engine.call("chat.new", json!({"project": pid})).unwrap();
    engine.call("chat.send", json!({"text": "Plane die Stunde"})).unwrap();
    let body = posts(&seen, "/chat/completions").last().cloned().unwrap();
    assert!(
        body.contains("Projektwissen aus AIS.ChatCraft") && body.contains("Igel und Winterschlaf") && body.contains("Fotosynthese braucht Licht"),
        "{body}"
    );
    assert!(!body.contains("Lea") && !body.contains("Müller"), "names stay on the device");
    let user = engine.state().messages[0].clone();
    let step = user.activity.iter().find(|s| s.label == "Projektwissen").expect("the message names the knowledge");
    assert!(step.detail.as_deref().unwrap().contains("lehrplan.txt"));

    // Switched off for this chat: nothing goes along.
    engine.call("chat.set_sources", json!({"knowledge": false})).unwrap();
    engine.call("chat.send", json!({"text": "Und noch was"})).unwrap();
    assert!(!posts(&seen, "/chat/completions").last().unwrap().contains("Projektwissen aus"));
    // A chat outside the project gets none.
    engine.call("chat.new", json!({"project": null})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let body = posts(&seen, "/chat/completions").last().cloned().unwrap();
    assert!(!body.contains("Winterschlaf") && !body.contains("Projektwissen aus"), "{body}");

    // A large file: only the fitting parts go along (the window is small).
    let big = dir.join("gross.md");
    let mut text = String::new();
    for i in 0..60 {
        text.push_str(&format!("Abschnitt {i}: {}\n\n", "Füllwort ".repeat(40)));
    }
    text.push_str("Abschnitt Spezial: Die Wasserspinne atmet unter Wasser.\n\n");
    std::fs::write(&big, &text).unwrap();
    engine.call("project.file_add", json!({"project": pid, "path": big.display().to_string()})).unwrap();
    assert_eq!(engine.call("project.files", json!({"project": pid})).unwrap()["usage"]["whole"], false);
    engine.call("chat.new", json!({"project": pid})).unwrap();
    engine.call("chat.send", json!({"text": "Wie atmet die Wasserspinne?"})).unwrap();
    let body = posts(&seen, "/chat/completions").last().cloned().unwrap();
    assert!(body.contains("Wasserspinne atmet"), "the fitting part");
    assert!(body.matches("Füllwort").count() < 400, "not the whole file");

    engine.call("project.file_remove", json!({"id": a["id"]})).unwrap();
    assert_eq!(engine.state().project_files.len(), 2);
    assert!(engine.call("project.file_remove", json!({"id": "nope"})).is_err());
    engine.call("project.delete", json!({"id": pid})).unwrap();
    assert!(engine.state().project_files.is_empty());
    assert!(engine.ctx().store(|st| st.project_files()).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// #32 with Claude (API): project files go as documents with citations on; a citation of one
/// opens at the passage in the project file; AIS.chat-like services get each part once.
#[test]
fn project_knowledge_is_citable_for_claude() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, |_, body| {
        let ev = |v: Value| format!("data: {v}\n\n");
        let mut out = String::new();
        if body.contains("\"type\":\"document\"") {
            for v in [
                json!({"type": "message_start", "message": {"usage": {"input_tokens": 10}}}),
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "citations_delta", "citation": {
                    "type": "char_location", "cited_text": "Igel halten Winterschlaf.", "document_index": 0, "document_title": "lehrplan.txt",
                    "start_char_index": 0, "end_char_index": 25}}}),
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Igel schlafen im Winter."}}),
                json!({"type": "content_block_stop", "index": 0}),
                json!({"type": "message_stop"}),
            ] {
                out.push_str(&ev(v));
            }
        } else {
            out.push_str(&ev(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Kein Dokument."}})));
            out.push_str(&ev(json!({"type": "message_stop"})));
        }
        ("text/event-stream".into(), out.into_bytes())
    });
    let id = engine.call("backend.add", json!({"kind": "anthropic", "base_url": url, "name": "Claude", "secret": "sk-ant-test"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("chat.select_model", json!({"backend": id, "model": "claude-opus-5-5"})).unwrap();
    let dir = temp_dir("knowledge-claude");
    let txt = dir.join("lehrplan.txt");
    std::fs::write(&txt, "Igel halten Winterschlaf. Sie fressen Insekten.").unwrap();
    let pid = engine.call("project.create", json!({"name": "Bio"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("project.file_add", json!({"project": pid, "path": txt.display().to_string()})).unwrap();
    engine.call("chat.new", json!({"project": pid})).unwrap();
    engine.call("chat.send", json!({"text": "Was machen Igel?"})).unwrap();
    let body = seen.lock().unwrap().iter().rev().find(|s| s.line.starts_with("POST")).unwrap().body.clone();
    assert!(
        body.contains("\"type\":\"document\"") && body.contains("\"title\":\"lehrplan.txt\"") && body.contains("\"citations\":{\"enabled\":true}"),
        "{body}"
    );
    let answer = engine.state().messages.last().cloned().unwrap();
    assert_eq!(answer.citations.len(), 1, "{answer:?}");
    let v = engine.call("message.citation", json!({"message_id": answer.id, "index": 0})).unwrap();
    assert_eq!(v["found"], true, "{v}");
    assert_eq!(v["after"], " Sie fressen Insekten.");

    // Once per chat for services that keep the chat on their server.
    let conv = engine.state().conversation(engine.state().current.as_ref().unwrap()).cloned().unwrap();
    let first = crate::knowledge::pick(engine.ctx(), &conv, "Igel", true).unwrap();
    let mut sources = conv.sources.clone();
    crate::knowledge::remember(&mut sources, &first.keys);
    let again = aiscraft_core::Conversation { sources, ..conv };
    assert!(crate::knowledge::pick(engine.ctx(), &again, "Igel", true).is_none(), "each part goes once");
    assert!(crate::knowledge::pick(engine.ctx(), &again, "Igel", false).is_some(), "services without a history get it every time");
    let _ = std::fs::remove_dir_all(dir);
}

/// #37: research with a scripted model and a scripted search server (SearXNG): the plan's
/// searches run, the pages are read, the report becomes a document with a list of sources, the
/// progress is in the state, the starting chat and the inbox hear about it; stopping works; a
/// service without a web search and without a search server is told what is missing.
#[test]
fn research_searches_reads_and_writes_a_report_with_sources() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.allow_local_web().unwrap();
    let base = Arc::new(Mutex::new(String::new()));
    let b2 = base.clone();
    let (url, seen) = scripted(&engine, move |line, body| {
        let base = b2.lock().unwrap().clone();
        if line.starts_with("GET /v1/models") {
            return models();
        }
        if line.starts_with("GET /search") {
            let hits = if line.contains("Nahrung") {
                json!({"results": [{"title": "Was Igel fressen", "url": format!("{base}/nahrung"), "content": "Insekten"}, {"title": "Doppelt", "url": format!("{base}/schlaf")}]})
            } else {
                json!({"results": [{"title": "Igel im Winter", "url": format!("{base}/schlaf"), "content": "Winterschlaf"}, {"title": "Kaputt", "url": "javascript:alert(1)"}]})
            };
            return ("application/json".into(), hits.to_string().into_bytes());
        }
        if line.starts_with("GET /schlaf") {
            return (
                "text/html".into(),
                b"<html><head><title>Igel im Winter</title></head><body><p>Igel halten von November bis Maerz Winterschlaf.</p></body></html>".to_vec(),
            );
        }
        if line.starts_with("GET /nahrung") {
            return ("text/html".into(), b"<html><body><p>Igel fressen Kaefer und Regenwuermer.</p></body></html>".to_vec());
        }
        if body.contains("Du planst eine Webrecherche") {
            if body.contains("langsam") {
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
            return sse("1. Igel Winterschlaf\n2. Igel Nahrung");
        }
        if body.contains("Recherchebericht") {
            return sse("# Igel im Jahreslauf\n\nIgel schlafen im Winter [1] und fressen Käfer [2].");
        }
        sse("ok")
    });
    *base.lock().unwrap() = url.clone();
    engine_at(&engine, &url);

    let err = engine.call("research.start", json!({"question": "Was machen Igel?"})).unwrap_err();
    assert!(err.message.contains("Suchdienst"), "{}", err.message);
    assert!(engine.call("research.set_search", json!({"url": "ftp://x"})).is_err());
    engine.call("research.set_search", json!({"url": url})).unwrap();
    assert_eq!(engine.state().research.search_url, url);

    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let origin = engine.state().current.clone().unwrap();
    let id = engine.call("research.start", json!({"question": "Was machen Igel im Jahreslauf?"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(engine.call("research.start", json!({"question": "  "})).is_err());
    assert!(engine.wait_for(std::time::Duration::from_secs(20), |s| s.research.runs.iter().any(|r| r.id == id && r.status != "running")), "research ends");
    let run = engine.state().research.runs.iter().find(|r| r.id == id).cloned().unwrap();
    assert_eq!(run.status, "done", "{run:?}");
    assert_eq!(run.sources.len(), 2, "two pages, the double and the bad link left out: {:?}", run.sources);
    let doc = engine.call("artifact.get", json!({"id": run.artifact_id.clone().unwrap()})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(doc.starts_with("# Igel im Jahreslauf") && doc.contains("## Quellen") && doc.contains(&format!("({url}/schlaf)")), "{doc}");
    assert!(engine.state().open_artifact.as_deref() != run.artifact_id.as_deref(), "no panel pushed open");
    let searches: Vec<String> = seen.lock().unwrap().iter().filter(|s| s.line.starts_with("GET /search")).map(|s| s.line.clone()).collect();
    assert_eq!(searches.len(), 2, "{searches:?}");
    let write = posts(&seen, "/chat/completions").into_iter().find(|b| b.contains("Recherchebericht")).unwrap();
    assert!(write.contains("November bis Maerz") && write.contains("Kaefer"), "the pages were read");
    assert!(engine.state().inbox.iter().any(|i| i.text.contains("fertig")));
    let msgs = engine.ctx().store(|st| st.messages(&origin)).unwrap();
    assert!(msgs.last().unwrap().content.contains("Recherche"), "the starting chat hears about it");

    // Stopping a running research.
    let id2 = engine.call("research.start", json!({"question": "langsam: Igel"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("research.stop", json!({"id": id2})).unwrap();
    assert!(engine.wait_for(std::time::Duration::from_secs(20), |s| s.research.runs.iter().any(|r| r.id == id2 && r.status != "running")));
    assert_eq!(engine.state().research.runs.iter().find(|r| r.id == id2).unwrap().status, "stopped");
    assert!(engine.call("research.stop", json!({"id": id2})).is_err(), "not running any more");
    engine.call("research.dismiss", json!({"id": id})).unwrap();
    assert!(engine.state().research.runs.iter().all(|r| r.id != id));
    assert!(engine.call("artifact.get", json!({"id": run.artifact_id.clone().unwrap()})).is_ok(), "the report stays");
    assert!(engine.call("research.dismiss", json!({"id": id})).is_err());
    engine.call("research.set_search", json!({"url": ""})).unwrap();
    assert!(engine.state().research.search_url.is_empty());
}
