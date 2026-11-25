//! Chat features after Claude's apps: thinking (#30), answer styles (#31), temporary chats (#34),
//! quick entry (#35), sources and citations (#39), finding in the open chat (#41).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Engine, Options};

pub(crate) type Seen = Arc<Mutex<Vec<String>>>;

/// An OpenAI-compatible server that records every request and streams the chunks `answer`
/// makes from the request body.
pub(crate) fn scripted(engine: &Engine, answer: fn(&str) -> Vec<Value>) -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
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
                    seen.lock().unwrap().push(req.clone());
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m"},{"id":"claude-opus-5-5"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    for c in answer(&req) {
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

pub(crate) fn engine_with(answer: fn(&str) -> Vec<Value>) -> (Engine, Seen) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, answer);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Test"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, seen)
}

pub(crate) fn plain(_: &str) -> Vec<Value> {
    vec![json!({"choices":[{"delta":{"content":"Antwort"}}]})]
}

pub(crate) fn last_chat_request(seen: &Seen, marker: &str) -> String {
    seen.lock().unwrap().iter().rev().find(|r| r.contains(marker)).cloned().unwrap_or_default()
}

/// #30: the model's thinking is kept with the answer, apart from its text, and survives a reload.
#[test]
fn thinking_is_kept_beside_the_answer() {
    fn thinks(_: &str) -> Vec<Value> {
        vec![
            json!({"choices":[{"delta":{"reasoning_content":"Erst die Frage verstehen. "}}]}),
            json!({"choices":[{"delta":{"reasoning_content":"Dann knapp antworten."}}]}),
            json!({"choices":[{"delta":{"content":"Zwölf."}}]}),
        ]
    }
    let (engine, seen) = engine_with(thinks);
    engine.call("chat.send", json!({"text": "Wie viel ist 3 mal 4?"})).unwrap();
    assert!(!last_chat_request(&seen, "3 mal 4").contains("Erst die Frage"), "the thinking is not sent back as text");
    let answer = engine.state().messages.last().cloned().unwrap();
    assert_eq!(answer.content, "Zwölf.");
    assert_eq!(answer.thinking, "Erst die Frage verstehen. Dann knapp antworten.");
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.close", json!({})).unwrap();
    engine.call("chat.open", json!({"id": conv})).unwrap();
    assert_eq!(engine.state().messages.last().unwrap().thinking, "Erst die Frage verstehen. Dann knapp antworten.", "kept in the store");
}

/// #30: the thinking depth is a per-chat switch; unknown levels are refused.
#[test]
fn effort_is_a_per_chat_switch() {
    let (engine, _) = engine_with(plain);
    engine.call("chat.set_sources", json!({"effort": "high"})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    assert_eq!(engine.state().conversation(&conv).unwrap().sources.effort.as_deref(), Some("high"));
    assert!(engine.call("chat.set_sources", json!({"effort": "ganz viel"})).is_err());
    engine.call("chat.set_sources", json!({"effort": null})).unwrap();
    assert!(engine.state().conversation(&conv).unwrap().sources.effort.is_none());
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().selection.sources.effort.is_none(), "a new chat starts with the model's default");
}

/// #31: the style lands in the system prompt; switching back to normal takes it out again.
#[test]
fn answer_style_lands_in_the_system_prompt() {
    let (engine, seen) = engine_with(plain);
    engine.call("chat.set_style", json!({"style": "concise"})).unwrap();
    engine.call("chat.send", json!({"text": "Erste Frage"})).unwrap();
    let req = last_chat_request(&seen, "Erste Frage");
    let body: Value = serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("Antwortstil: knapp"), "{body}");
    // A style of one's own, from an example text.
    engine.call("chat.set_style", json!({"style": "custom", "sample": "Moin! Kurz und schnackig, so wie hier oben im Norden."})).unwrap();
    engine.call("chat.send", json!({"text": "Zweite Frage"})).unwrap();
    let req = last_chat_request(&seen, "Zweite Frage");
    assert!(req.contains("schnackig") && req.contains("nur den Stil"), "the example goes along");
    engine.call("chat.set_style", json!({"style": "normal"})).unwrap();
    engine.call("chat.send", json!({"text": "Dritte Frage"})).unwrap();
    assert!(!last_chat_request(&seen, "Dritte Frage").contains("Antwortstil"));
    assert!(engine.call("chat.set_style", json!({"style": "lustig"})).is_err());
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.call("chat.set_style", json!({"style": "custom", "sample": "  "})).is_err(), "a custom style needs an example");
}

/// #31: backends without a system prompt (AIS.chat) get the style as a hint in the message,
/// once, and again only when it changes (also back to normal).
#[test]
fn answer_style_goes_inline_once_without_system_prompt() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let ctx = engine.ctx();
    let mut conv = aiscraft_core::Conversation {
        id: "c".into(),
        title: "t".into(),
        backend_id: "b".into(),
        model_id: "m".into(),
        project_id: None,
        created_at: 0,
        updated_at: 0,
        pinned: false,
        sources: aiscraft_core::Sources { style: Some("formal".into()), ..Default::default() },
        persona: None,
        kind: Default::default(),
        remote_id: None,
        temporary: false,
        archived: false,
        deleted_at: None,
    };
    let sent = crate::chat::inline_context(ctx, &conv, false, "").expect("the style goes along");
    assert!(sent.block.contains("Antwortstil: formell") && sent.labels.contains(&"Antwortstil".to_string()));
    assert!(crate::chat::inline_context(ctx, &conv, true, "").is_none(), "with a system prompt it is not in the message");
    conv.sources.style_sent = sent.style_print.clone();
    assert!(crate::chat::inline_context(ctx, &conv, false, "").is_none(), "not again while it is the same");
    conv.sources.style = None;
    let back = crate::chat::inline_context(ctx, &conv, false, "").expect("back to normal is said once");
    assert!(back.block.contains("wieder wie üblich"));
}

/// #34: a temporary chat is answered like any other, but after closing nothing of it remains:
/// not in the store, not in the search, no memory note, no copy of its files.
#[test]
fn temporary_chat_leaves_nothing_behind() {
    let (engine, seen) = engine_with(plain);
    let notes = engine.state().memory.len();
    engine.call("chat.temporary", json!({})).unwrap();
    assert!(engine.state().selection.temporary);
    let dir = std::env::temp_dir().join(format!("aiscraft-temp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notiz.txt");
    std::fs::write(&file, "Geheimprojekt Kastanie").unwrap();
    engine.call("chat.send", json!({"text": "Merk dir: Kastanienfest am Freitag", "files": [file.display().to_string()]})).unwrap();
    assert!(last_chat_request(&seen, "Kastanienfest").contains("Geheimprojekt"), "it is answered as usual");
    let conv = engine.state().current.clone().unwrap();
    assert!(engine.state().conversation(&conv).unwrap().temporary);
    assert_eq!(engine.state().messages.len(), 2);
    assert_eq!(engine.state().memory.len(), notes, "no memory note");
    let copy = engine.state().messages[0].attachments[0].local_path.clone().unwrap();
    assert!(std::path::Path::new(&copy).exists());
    let search = engine.call("search", json!({"query": "Kastanienfest"})).unwrap();
    assert!(!search.to_string().contains(&conv), "not searchable while open: {search}");
    // Closing: everything goes.
    engine.call("chat.close", json!({})).unwrap();
    assert!(engine.state().conversation(&conv).is_none(), "gone from the list");
    engine
        .ctx()
        .store(|st| {
            assert!(st.conversation(&conv)?.is_none());
            assert!(st.messages(&conv)?.is_empty());
            assert!(st.search("Kastanienfest", 10)?.is_empty());
            assert!(st.search_any(&["Kastanienfest".to_string()], None, true, 10)?.is_empty());
            assert!(st.conversations(500)?.iter().all(|c| c.id != conv));
            Ok(())
        })
        .unwrap();
    assert!(!std::path::Path::new(&copy).exists(), "the file copy is deleted");
    assert!(engine.call("chat.open", json!({"id": conv})).is_err());
    assert_eq!(engine.state().memory.len(), notes);
    // The next chat is a normal one again and is kept.
    assert!(!engine.state().selection.temporary);
    engine.call("chat.send", json!({"text": "Normal"})).unwrap();
    let normal = engine.state().current.clone().unwrap();
    assert!(engine.ctx().store(|st| st.conversation(&normal)).unwrap().is_some());
    engine.call("chat.temporary", json!({"on": false})).unwrap();
    assert!(engine.call("chat.temporary", json!({"on": "ja"})).is_err());
}

/// #34: AIS.chat keeps every chat in its own history, so a temporary chat is only for other
/// backends: it cannot be switched on with AIS.chat, nor moved there before or after the first
/// message, and a normal chat with AIS.chat is unaffected.
#[test]
fn temporary_chat_is_not_offered_with_ais_chat() {
    let (engine, _) = engine_with(plain);
    let other = engine.state().selection.backend_id.clone();
    assert!(engine.state().temporary_possible(&other));
    assert!(!engine.state().temporary_possible("ais-chat"));

    // Before the first message: switching AIS.chat in is refused, the selection stays.
    engine.call("chat.temporary", json!({})).unwrap();
    let err = engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m"})).unwrap_err();
    assert!(err.message.contains("nur mit anderen Diensten"), "{}", err.message);
    assert_eq!(engine.state().selection.backend_id, other);
    assert!(engine.state().selection.temporary);

    // Mid-chat: the temporary chat cannot move to AIS.chat either.
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    assert!(engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m"})).is_err());
    assert_eq!(engine.state().conversation(&conv).unwrap().backend_id, other);

    // With AIS.chat selected, a temporary chat cannot be started.
    engine.call("chat.new", json!({"backend": "ais-chat"})).unwrap();
    assert!(!engine.state().selection.temporary);
    let err = engine.call("chat.temporary", json!({})).unwrap_err();
    assert!(err.message.contains("AIS.chat speichert"), "{}", err.message);
    assert!(!engine.state().selection.temporary);
    engine.call("chat.temporary", json!({"on": false})).unwrap();

    // Even a selection that says temporary never creates a temporary chat at AIS.chat.
    engine.ctx().update(|s| s.selection.temporary = true);
    assert!(engine.call("chat.send", json!({"text": "Hallo"})).is_err());
    assert!(engine.state().conversations.iter().all(|c| !(c.temporary && c.backend_id == "ais-chat")));
}

/// #41: finding in the open chat counts every hit, steps forward and back around the end, and
/// forgets the search with the chat.
#[test]
fn find_in_the_open_chat() {
    fn igel(_: &str) -> Vec<Value> {
        vec![json!({"choices":[{"delta":{"content":"Der Igel schläft. Igel fressen Käfer."}}]})]
    }
    let (engine, _) = engine_with(igel);
    assert!(engine.call("chat.find", json!({"query": "igel"})).is_err(), "no chat open");
    engine.call("chat.send", json!({"text": "Erzähl vom IGEL"})).unwrap();
    let r = engine.call("chat.find", json!({"query": "igel"})).unwrap();
    assert_eq!((r["total"].as_u64(), r["current"].as_u64()), (Some(3), Some(1)));
    let first = engine.state().messages[0].id.clone();
    assert_eq!(r["message_id"], json!(first), "the question comes first");
    assert_eq!(engine.call("chat.find", json!({"step": "next"})).unwrap()["current"], 2);
    assert_eq!(engine.call("chat.find", json!({"step": "next"})).unwrap()["current"], 3);
    assert_eq!(engine.call("chat.find", json!({"step": "next"})).unwrap()["current"], 1, "around the end");
    assert_eq!(engine.call("chat.find", json!({"step": "prev"})).unwrap()["current"], 3);
    assert_eq!(engine.call("chat.find", json!({"query": "Käfer"})).unwrap()["total"], 1);
    assert_eq!(engine.call("chat.find", json!({"query": "Wal"})).unwrap()["total"], 0);
    assert!(engine.call("chat.find", json!({"step": "seitwärts"})).is_err());
    engine.call("chat.find", json!({"close": true})).unwrap();
    assert!(engine.state().find.is_none());
    engine.call("chat.find", json!({"query": "igel"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().find.is_none(), "a search belongs to its chat");
}

/// #35: a quick entry with text opens a new chat and sends it; without text the chat waits, the
/// window is asked to come to the front each time. Screenshots are refused in tests.
#[test]
fn quick_entry_sends_or_waits() {
    let (engine, seen) = engine_with(plain);
    engine.call("chat.send", json!({"text": "Alter Chat"})).unwrap();
    let old = engine.state().current.clone().unwrap();
    let r = engine.call("chat.quick_ask", json!({"text": "Schnelle Frage"})).unwrap();
    assert_eq!(r["sent"], true);
    assert_ne!(engine.state().current.as_ref(), Some(&old), "a new chat");
    assert!(!last_chat_request(&seen, "Schnelle Frage").contains("Alter Chat"));
    assert_eq!(engine.state().quick.as_ref().unwrap().seq, 1);
    let r = engine.call("chat.quick_ask", json!({})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(engine.state().current.is_none());
    assert_eq!(engine.state().quick.as_ref().unwrap().seq, 2);
    let e = engine.call("chat.quick_ask", json!({"screenshot": true})).unwrap_err();
    assert!(e.message.contains("Bildschirmfoto"), "{}", e.message);
    assert!(engine.call("chat.quick_ask", json!({"text": 5})).is_err());
}

/// A Claude API server (Anthropic messages, streamed) on the scripted server: answers with a
/// sentence backed by a passage of the attached `igel.txt` and a page of the web.
fn claude_engine() -> (Engine, Seen) {
    fn cites(req: &str) -> Vec<Value> {
        if !req.contains("\"type\":\"document\"") {
            return vec![
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Kein Dokument."}}),
                json!({"type": "message_stop"}),
            ];
        }
        vec![
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 10}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Laut deiner Datei gilt: "}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "citations_delta", "citation": {
                "type": "char_location", "cited_text": "Im Winter halten sie Winterschlaf.", "document_index": 0,
                "document_title": "igel.txt", "start_char_index": 23, "end_char_index": 57}}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "citations_delta", "citation": {
                "type": "web_search_result_location", "url": "https://example.org/igel", "title": "Igel im Garten", "cited_text": "Igel schlafen von November bis März."}}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Igel schlafen im Winter."}}),
            json!({"type": "content_block_stop", "index": 1}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 9}}),
            json!({"type": "message_stop"}),
        ]
    }
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, cites);
    let base = url.trim_end_matches("/v1").to_string();
    let id = engine.call("backend.add", json!({"kind": "anthropic", "base_url": base, "name": "Claude", "secret": "sk-ant-test"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    engine.call("chat.select_model", json!({"backend": id, "model": "claude-opus-5-5"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, seen)
}

/// #39: an attached file goes to Claude as a document with citations on; the answer keeps the
/// passages that back it, and `message.citation` shows the passage where it stands in the file.
#[test]
fn citations_are_kept_and_open_at_the_passage() {
    let (engine, seen) = claude_engine();
    let dir = std::env::temp_dir().join(format!("aiscraft-cite-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("igel.txt");
    std::fs::write(&file, "Igel fressen Insekten. Im Winter halten sie Winterschlaf. Sie sind nachtaktiv.").unwrap();
    engine.call("chat.send", json!({"text": "Was machen Igel im Winter?", "files": [file.display().to_string()]})).unwrap();
    let req = last_chat_request(&seen, "Igel im Winter");
    assert!(req.contains("\"citations\":{\"enabled\":true}") && req.contains("\"title\":\"igel.txt\""), "{req}");
    let answer = engine.state().messages.last().cloned().unwrap();
    assert_eq!(answer.content, "Laut deiner Datei gilt: Igel schlafen im Winter.");
    assert_eq!(answer.citations.len(), 2, "{:?}", answer.citations);
    assert_eq!(answer.citations[0].claim, "Igel schlafen im Winter.");
    assert_eq!(answer.citations[1].url.as_deref(), Some("https://example.org/igel"));
    // Kept in the store.
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.close", json!({})).unwrap();
    engine.call("chat.open", json!({"id": conv})).unwrap();
    assert_eq!(engine.state().messages.last().unwrap().citations, answer.citations);

    let v = engine.call("message.citation", json!({"message_id": answer.id, "index": 0})).unwrap();
    assert_eq!(v["found"], true, "{v}");
    assert_eq!(v["passage"], "Im Winter halten sie Winterschlaf.");
    assert_eq!(v["before"], "Igel fressen Insekten. ");
    assert_eq!(v["after"], " Sie sind nachtaktiv.");
    assert!(v["path"].as_str().is_some_and(|p| p.ends_with(".txt")), "the kept copy: {v}");
    assert_eq!(engine.state().citation.as_ref().map(|c| c.index), Some(0));
    // A web page: the quoted words and the address, no file.
    let web = engine.call("message.citation", json!({"message_id": answer.id, "index": 1})).unwrap();
    assert_eq!((web["found"].as_bool(), web["passage"].as_str()), (Some(false), Some("Igel schlafen von November bis März.")));
    assert!(web.get("path").is_none());
    // Hostile or stale references are refused, never panic.
    assert!(engine.call("message.citation", json!({"message_id": answer.id, "index": 7})).is_err());
    assert!(engine.call("message.citation", json!({"message_id": "weg", "index": 0})).is_err());
    assert!(engine.call("message.citation", json!({"message_id": answer.id, "index": -1})).is_err());
    assert!(engine.call("message.citation", json!({})).is_err());
    engine.call("message.citation", json!({"close": true})).unwrap();
    assert!(engine.state().citation.is_none());
    engine.call("message.citation", json!({"message_id": answer.id, "index": 0})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.state().citation.is_none(), "a citation belongs to its chat");
    let _ = std::fs::remove_dir_all(dir);
}
