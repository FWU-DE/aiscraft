//! Chats from claude.ai and ChatGPT (#42): their data export (a ZIP with `conversations.json`,
//! or that file alone) becomes local chats that are listed, searched and can be continued.
//!
//! The file is hostile input: sizes and counts are capped, the ZIP is read in memory (nothing is
//! unpacked to disk, so no entry name can reach outside), only one entry is read and its name is
//! checked. Names protection does not apply after the fact: imported chats keep the names as they
//! stood at claude.ai or ChatGPT (the dialog says so).

use std::collections::HashSet;

use aiscraft_core::{Conversation, Error, Message, Result, Role};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id, now_ms};

/// Largest export file read.
const MAX_FILE: u64 = 512 * 1024 * 1024;
/// Largest `conversations.json` (unpacked).
const MAX_JSON: u64 = 256 * 1024 * 1024;
/// Most entries in the ZIP (ChatGPT exports carry images and audio next to the JSON).
const MAX_ENTRIES: u64 = 200_000;
const MAX_CHATS: usize = 10_000;
const MAX_MESSAGES: usize = 5_000;
/// Longest message kept, in characters.
const MAX_TEXT: usize = 200_000;
const MAX_TITLE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Claude,
    ChatGpt,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Claude => "claude.ai",
            Source::ChatGpt => "ChatGPT",
        }
    }
}

/// One chat as read from the export.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chat {
    pub source_id: String,
    pub title: String,
    pub created: i64,
    pub updated: i64,
    /// (role, text, time in ms, model)
    pub messages: Vec<(Role, String, i64, Option<String>)>,
}

/// The `conversations.json` of an export: from a ZIP or the file itself.
fn conversations_json(bytes: &[u8]) -> Result<Vec<u8>> {
    if !bytes.starts_with(b"PK") {
        if bytes.len() as u64 > MAX_JSON {
            return Err(Error::invalid(format!("Die Datei ist größer als {} MB", MAX_JSON / (1024 * 1024))));
        }
        return Ok(bytes.to_vec());
    }
    let zip_err = |e: craft_zip::Error| {
        if e.is_limit() {
            Error::invalid(format!("Der Export ist zu groß oder hat zu viele Dateien ({e})"))
        } else {
            Error::invalid(format!("Das ist kein lesbares ZIP ({e})"))
        }
    };
    // Images and audio count against no budget: only the JSON is read.
    let mut zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: u64::MAX }).map_err(zip_err)?;
    let mut name = None;
    for i in 0..zip.len() {
        let n = zip.entry_name(i).map_err(zip_err)?.unwrap_or_default();
        if crate::ext::package::bad_name(&n).is_some() {
            continue;
        }
        // At the top or in one folder ("export-2026-11-01/conversations.json").
        let parts: Vec<&str> = n.split('/').collect();
        if parts.last() == Some(&"conversations.json") && parts.len() <= 2 && name.as_ref().is_none_or(|x: &String| x.len() > n.len()) {
            name = Some(n);
        }
    }
    let Some(name) = name else {
        return Err(Error::invalid("Im ZIP fehlt conversations.json. Ist das der Datenexport von claude.ai oder ChatGPT?"));
    };
    zip.read(&name, MAX_JSON).map_err(zip_err)?.ok_or_else(|| Error::invalid("conversations.json ist leer"))
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((at, _)) => format!("{}\n\n… (gekürzt)", text.get(..at).unwrap_or("")),
        None => text.to_string(),
    }
}

fn iso_ms(t: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp_millis())
}

fn secs_ms(v: Option<&Value>) -> Option<i64> {
    v.and_then(Value::as_f64).filter(|x| x.is_finite() && *x > 0.0 && *x < 1e11).map(|x| (x * 1000.0) as i64)
}

/// Read an export: which service it is from and its chats (empty ones left out).
pub(crate) fn parse(bytes: &[u8]) -> Result<(Source, Vec<Chat>)> {
    let json = conversations_json(bytes)?;
    let all: Value = serde_json::from_slice(&json).map_err(|e| Error::invalid(format!("conversations.json ist kein gültiges JSON ({e})")))?;
    let Some(list) = all.as_array() else {
        return Err(Error::invalid("conversations.json enthält keine Liste von Chats"));
    };
    let source = match list.first() {
        Some(c) if c.get("chat_messages").is_some() => Source::Claude,
        Some(c) if c.get("mapping").is_some() => Source::ChatGpt,
        Some(_) => return Err(Error::invalid("Das ist kein Export von claude.ai oder ChatGPT")),
        None => return Ok((Source::Claude, Vec::new())),
    };
    let now = now_ms();
    let chats = list
        .iter()
        .take(MAX_CHATS)
        .filter_map(|c| match source {
            Source::Claude => claude_chat(c, now),
            Source::ChatGpt => chatgpt_chat(c, now),
        })
        .filter(|c| !c.messages.is_empty())
        .collect();
    Ok((source, chats))
}

fn title(t: &str, messages: &[(Role, String, i64, Option<String>)]) -> String {
    let t = t.trim();
    if t.is_empty() {
        messages.iter().find(|m| m.0 == Role::User).map(|m| aiscraft_core::title_from(&m.1)).unwrap_or_else(|| "Übernommener Chat".into())
    } else {
        t.chars().take(MAX_TITLE).collect()
    }
}

fn claude_chat(c: &Value, now: i64) -> Option<Chat> {
    let created = iso_ms(s(c, "created_at")).unwrap_or(now);
    let mut messages = Vec::new();
    for (i, m) in c.get("chat_messages")?.as_array()?.iter().take(MAX_MESSAGES).enumerate() {
        let role = match s(m, "sender") {
            "human" => Role::User,
            "assistant" => Role::Assistant,
            _ => continue,
        };
        let parts: Vec<&str> = m
            .get("content")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter(|p| s(p, "type") == "text").map(|p| s(p, "text")).filter(|t| !t.trim().is_empty()).collect())
            .unwrap_or_default();
        let mut text = if parts.is_empty() { s(m, "text").to_string() } else { parts.join("\n\n") };
        let files: Vec<&str> = ["attachments", "files"]
            .iter()
            .filter_map(|k| m.get(*k).and_then(Value::as_array))
            .flatten()
            .map(|f| s(f, "file_name"))
            .filter(|n| !n.is_empty())
            .take(50)
            .collect();
        if !files.is_empty() {
            text.push_str(&format!("\n\n(Anhang: {})", files.join(", ")));
        }
        if text.trim().is_empty() {
            continue;
        }
        let at = iso_ms(s(m, "created_at")).unwrap_or(created + i as i64);
        messages.push((role, clip(text.trim()), at, (role == Role::Assistant).then(|| "claude.ai".to_string())));
    }
    let updated = iso_ms(s(c, "updated_at")).or_else(|| messages.last().map(|m| m.2)).unwrap_or(created);
    Some(Chat { source_id: s(c, "uuid").to_string(), title: title(s(c, "name"), &messages), created, updated, messages })
}

/// ChatGPT keeps a tree of edits; the chat people saw is the path from `current_node` up.
fn chatgpt_chat(c: &Value, now: i64) -> Option<Chat> {
    let mapping = c.get("mapping")?.as_object()?;
    let created = secs_ms(c.get("create_time")).unwrap_or(now);
    let mut node = c.get("current_node").and_then(Value::as_str).filter(|n| mapping.contains_key(*n)).map(str::to_string).or_else(|| {
        // No current node: the newest leaf.
        mapping
            .iter()
            .filter(|(_, n)| n.get("children").and_then(Value::as_array).is_none_or(Vec::is_empty))
            .max_by(|a, b| {
                let t = |n: &Value| n.get("message").and_then(|m| m.get("create_time")).and_then(Value::as_f64).unwrap_or(0.0);
                t(a.1).total_cmp(&t(b.1))
            })
            .map(|(k, _)| k.clone())
    });
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    while let Some(id) = node.take() {
        if !seen.insert(id.clone()) || path.len() >= MAX_MESSAGES * 4 {
            break;
        }
        let Some(n) = mapping.get(&id) else { break };
        path.push(n);
        node = n.get("parent").and_then(Value::as_str).map(str::to_string);
    }
    path.reverse();
    let mut messages = Vec::new();
    for (i, n) in path.into_iter().enumerate() {
        let Some(m) = n.get("message").filter(|m| m.is_object()) else { continue };
        let role = match m.get("author").map(|a| s(a, "role")) {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        let meta = m.get("metadata");
        if meta.and_then(|x| x.get("is_visually_hidden_from_conversation")).and_then(Value::as_bool) == Some(true) {
            continue;
        }
        // Calls to tools (browsing, code) are not part of what people read.
        if role == Role::Assistant && m.get("recipient").and_then(Value::as_str).is_some_and(|r| r != "all") {
            continue;
        }
        let content = m.get("content");
        let text = match content.map(|c| s(c, "content_type")) {
            Some("text" | "multimodal_text") => content
                .and_then(|c| c.get("parts"))
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter_map(|p| match p {
                            Value::String(t) => Some(t.clone()),
                            Value::Object(_) if s(p, "content_type").contains("image") => Some("(Bild)".into()),
                            Value::Object(_) if s(p, "content_type").contains("audio") => Some("(Sprachnachricht)".into()),
                            _ => None,
                        })
                        .filter(|t| !t.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n")
                })
                .unwrap_or_default(),
            Some("code") => content.map(|c| format!("```\n{}\n```", s(c, "text"))).unwrap_or_default(),
            _ => continue,
        };
        if text.trim().is_empty() {
            continue;
        }
        let at = secs_ms(m.get("create_time")).unwrap_or(created + i as i64);
        let model = (role == Role::Assistant).then(|| {
            let slug = meta.map(|x| s(x, "model_slug")).unwrap_or("");
            if slug.is_empty() { "ChatGPT".to_string() } else { slug.chars().take(80).collect() }
        });
        messages.push((role, clip(text.trim()), at, model));
        if messages.len() >= MAX_MESSAGES {
            break;
        }
    }
    let updated = secs_ms(c.get("update_time")).or_else(|| messages.last().map(|m| m.2)).unwrap_or(created);
    let id = c.get("conversation_id").or_else(|| c.get("id")).and_then(Value::as_str).unwrap_or("").to_string();
    Some(Chat { source_id: id, title: title(s(c, "title"), &messages), created, updated, messages })
}

/// `chat.import`: read an export file and add its chats; chats imported before are skipped.
pub(crate) async fn import(c: Ctx, p: Value) -> Result<Value> {
    let path = req_str(&p, "path")?;
    let project = opt_str(&p, "project").filter(|x| !x.is_empty());
    if let Some(pid) = &project
        && !c.state().projects.iter().any(|x| &x.id == pid)
    {
        return Err(Error::not_found(format!("Projekt nicht gefunden: {pid}")));
    }
    let meta = std::fs::metadata(&path).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{path} ist keine Datei")));
    }
    if meta.len() > MAX_FILE {
        return Err(Error::invalid(format!("Die Datei ist größer als {} MB", MAX_FILE / (1024 * 1024))));
    }
    let bytes = std::fs::read(&path).map_err(|e| Error::invalid(format!("{path}: {e}")))?;
    let (source, chats) = tokio::task::spawn_blocking(move || parse(&bytes)).await.map_err(|e| Error::invalid(format!("Einlesen abgebrochen: {e}")))??;
    let state = c.state();
    let (backend_id, model_id) = (state.selection.backend_id.clone(), state.selection.model_id.clone());
    let mut have: HashSet<String> = state.conversations.iter().map(|x| x.id.clone()).collect();
    drop(state);
    let (mut imported, mut skipped, mut count) = (Vec::new(), 0usize, 0usize);
    for chat in chats {
        // The service's id keeps the chat the same chat on a second import.
        let id = uuid::Uuid::parse_str(&chat.source_id).map(|u| u.to_string()).unwrap_or_else(|_| new_id());
        if have.contains(&id) || c.store(|st| st.conversation(&id)).ok().flatten().is_some() {
            skipped += 1;
            continue;
        }
        have.insert(id.clone());
        let conv = Conversation {
            id: id.clone(),
            title: chat.title.clone(),
            backend_id: backend_id.clone(),
            model_id: model_id.clone(),
            project_id: project.clone(),
            created_at: chat.created,
            updated_at: chat.updated,
            pinned: false,
            sources: Default::default(),
            persona: None,
            kind: Default::default(),
            remote_id: None,
            temporary: false,
        };
        c.store(|st| {
            st.save_conversation(&conv)?;
            for (role, content, at, model) in &chat.messages {
                st.save_message(&Message {
                    id: new_id(),
                    conversation_id: id.clone(),
                    role: *role,
                    content: content.clone(),
                    created_at: *at,
                    model: model.clone(),
                    attachments: vec![],
                    activity: vec![],
                    error: None,
                    usage: None,
                    tool_calls: vec![],
                    tool_result: None,
                    thinking: String::new(),
                    citations: Vec::new(),
                })?;
            }
            Ok(())
        })?;
        count += chat.messages.len();
        imported.push(conv);
    }
    let n = imported.len();
    c.update(|s| {
        s.conversations.extend(imported);
        s.conversations.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
    });
    let text = match (n, skipped) {
        (0, 0) => format!("In der Datei von {} sind keine Chats.", source.label()),
        (0, k) => format!("Alle {k} Chats von {} waren schon übernommen.", source.label()),
        (n, 0) => format!("{n} Chats von {} übernommen.", source.label()),
        (n, k) => format!("{n} Chats von {} übernommen, {k} waren schon da.", source.label()),
    };
    c.notice(NoticeLevel::Success, text);
    Ok(json!({"source": source.label(), "imported": n, "skipped": skipped, "messages": count}))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const CLAUDE: &str = r#"[{"uuid":"1b4e28ba-2fa1-11d2-883f-0016d3cca427","name":"Gedichtanalyse Erlkönig","created_at":"2025-03-01T10:00:00.000000Z","updated_at":"2025-03-01T10:05:00Z",
      "chat_messages":[
        {"uuid":"a","text":"","sender":"human","created_at":"2025-03-01T10:00:00Z","content":[{"type":"text","text":"Hilf mir beim Erlkönig."}],"attachments":[{"file_name":"erlkoenig.pdf","extracted_content":"Wer reitet so spät"}],"files":[]},
        {"uuid":"b","text":"Gern: Ballade von Goethe.","sender":"assistant","created_at":"2025-03-01T10:00:05Z","content":[{"type":"tool_use","name":"x"}]}
      ]},
      {"uuid":"leer","name":"","created_at":null,"chat_messages":[]}]"#;

    pub(crate) const CHATGPT: &str = r#"[{"title":"Bruchrechnung","create_time":1700000000.5,"update_time":1700000100.0,"conversation_id":"6f9619ff-8b86-d011-b42d-00cf4fc964ff","current_node":"n4",
      "mapping":{
        "root":{"id":"root","message":null,"parent":null,"children":["n1"]},
        "n1":{"id":"n1","message":{"author":{"role":"system"},"content":{"content_type":"text","parts":[""]},"metadata":{"is_visually_hidden_from_conversation":true}},"parent":"root","children":["n2","n2b"]},
        "n2":{"id":"n2","message":{"author":{"role":"user"},"create_time":1700000001.0,"content":{"content_type":"multimodal_text","parts":[{"content_type":"image_asset_pointer"},"Erkläre 3/4 + 1/8"]}},"parent":"n1","children":["n3"]},
        "n2b":{"id":"n2b","message":{"author":{"role":"user"},"content":{"content_type":"text","parts":["verworfene Fassung"]}},"parent":"n1","children":[]},
        "n3":{"id":"n3","message":{"author":{"role":"assistant"},"recipient":"python","content":{"content_type":"code","text":"3/4+1/8"}},"parent":"n2","children":["n4"]},
        "n4":{"id":"n4","message":{"author":{"role":"assistant"},"recipient":"all","create_time":1700000002.0,"content":{"content_type":"text","parts":["Hauptnenner 8: 6/8 + 1/8 = 7/8."]},"metadata":{"model_slug":"gpt-4o"}},"parent":"n3","children":[]}
      }}]"#;

    #[test]
    fn claude_export_is_read() {
        let (src, chats) = parse(CLAUDE.as_bytes()).unwrap();
        assert_eq!(src, Source::Claude);
        assert_eq!(chats.len(), 1, "empty chats are left out");
        let c = &chats[0];
        assert_eq!(c.title, "Gedichtanalyse Erlkönig");
        assert_eq!(c.messages[0].0, Role::User);
        assert_eq!(c.messages[0].1, "Hilf mir beim Erlkönig.\n\n(Anhang: erlkoenig.pdf)");
        assert_eq!(c.messages[1].1, "Gern: Ballade von Goethe.", "no text part: the plain text");
        assert_eq!(c.created, 1_740_823_200_000);
    }

    #[test]
    fn chatgpt_export_follows_the_shown_branch() {
        let (src, chats) = parse(CHATGPT.as_bytes()).unwrap();
        assert_eq!(src, Source::ChatGpt);
        let m: Vec<(Role, &str)> = chats[0].messages.iter().map(|m| (m.0, m.1.as_str())).collect();
        assert_eq!(m, vec![(Role::User, "(Bild)\n\nErkläre 3/4 + 1/8"), (Role::Assistant, "Hauptnenner 8: 6/8 + 1/8 = 7/8.")]);
        assert_eq!(chats[0].messages[1].3.as_deref(), Some("gpt-4o"));
        assert_eq!(chats[0].source_id, "6f9619ff-8b86-d011-b42d-00cf4fc964ff");
    }

    #[test]
    fn hostile_exports_are_refused_or_capped() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"[{\"x\":1}]").is_err());
        assert!(parse(b"not json").is_err());
        assert!(parse(b"PK\x03\x04garbage").is_err());
        // A cycle in the tree ends.
        let cyc = r#"[{"title":"x","current_node":"a","mapping":{"a":{"message":{"author":{"role":"user"},"content":{"content_type":"text","parts":["hi"]}},"parent":"b"},"b":{"message":null,"parent":"a"}}}]"#;
        assert_eq!(parse(cyc.as_bytes()).unwrap().1[0].messages.len(), 1);
        // Deep nesting is refused by the JSON reader, not by a stack overflow.
        let deep = format!("[{}{}]", "[".repeat(100_000), "]".repeat(100_000));
        assert!(parse(deep.as_bytes()).is_err());
        // Only conversations.json at the top or one folder down is read; "../" names are skipped.
        let mut w = craft_zip::Writer::new();
        let _ = w.add("../conversations.json", CLAUDE.as_bytes(), craft_zip::Compression::Deflated);
        w.add("a/b/conversations.json", CLAUDE.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        assert!(parse(&w.finish().unwrap()).is_err());
        let mut w = craft_zip::Writer::new();
        w.add("export/conversations.json", CHATGPT.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        w.add("export/file-abc.png", b"\x89PNG", craft_zip::Compression::Stored).unwrap();
        assert_eq!(parse(&w.finish().unwrap()).unwrap().0, Source::ChatGpt);
        assert!(clip(&"x".repeat(MAX_TEXT + 5)).ends_with("(gekürzt)"));
    }

    /// `chat.import` end to end: the chats are listed, searchable, open with their messages, and a
    /// second import adds nothing.
    #[test]
    fn imported_chats_are_listed_searchable_and_not_doubled() {
        let engine = crate::Engine::open(crate::Options { data_dir: None, secrets: None }).unwrap();
        let dir = std::env::temp_dir().join(format!("aiscraft-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = craft_zip::Writer::new();
        w.add("conversations.json", CLAUDE.as_bytes(), craft_zip::Compression::Deflated).unwrap();
        w.add("users.json", b"[]", craft_zip::Compression::Deflated).unwrap();
        let zip = dir.join("data-2025.zip");
        std::fs::write(&zip, w.finish().unwrap()).unwrap();
        let r = engine.call("chat.import", json!({"path": zip.display().to_string()})).unwrap();
        assert_eq!((r["source"].as_str(), r["imported"].as_u64(), r["messages"].as_u64()), (Some("claude.ai"), Some(1), Some(2)));
        let gpt = dir.join("conversations.json");
        std::fs::write(&gpt, CHATGPT).unwrap();
        assert_eq!(engine.call("chat.import", json!({"path": gpt.display().to_string()})).unwrap()["imported"], 1);
        let s = engine.state();
        assert!(s.conversations.iter().any(|c| c.title == "Gedichtanalyse Erlkönig"));
        assert!(s.conversations.iter().any(|c| c.title == "Bruchrechnung"));
        let hits = engine.call("search", json!({"query": "Hauptnenner"})).unwrap();
        assert_eq!(hits[0]["title"], "Bruchrechnung");
        engine.call("chat.open", json!({"id": "1b4e28ba-2fa1-11d2-883f-0016d3cca427"})).unwrap();
        assert_eq!(engine.state().messages.len(), 2);
        let again = engine.call("chat.import", json!({"path": zip.display().to_string()})).unwrap();
        assert_eq!((again["imported"].as_u64(), again["skipped"].as_u64()), (Some(0), Some(1)));
        assert!(engine.call("chat.import", json!({"path": dir.display().to_string()})).is_err(), "a folder is no export");
        assert!(engine.call("chat.import", json!({"path": gpt.display().to_string(), "project": "weg"})).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
