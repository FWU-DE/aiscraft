//! Archive and trash (#77): archiving keeps a chat whole but out of the sidebar and recall,
//! deleting moves it to the trash with an undo, purging leaves nothing behind.

use serde_json::json;

use crate::chatux_tests::{engine_with, plain};
use crate::{Engine, NoticeLevel};

/// Sends `text` in a new chat and returns the chat's id.
fn chat_with(engine: &Engine, text: &str) -> String {
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": text})).unwrap();
    engine.state().current.clone().unwrap()
}

fn listed(engine: &Engine) -> Vec<String> {
    engine.state().listed_conversations().map(|c| c.id.clone()).collect()
}

/// Excerpts recall would take for `request` in a fresh chat.
fn recalled(engine: &Engine, request: &str) -> Vec<String> {
    let probe = chat_with(engine, "Probe");
    let conv = engine.state().conversation(&probe).cloned().unwrap();
    let out = crate::skills::recall(engine.ctx(), &conv, request).items.into_iter().map(|r| r.label).collect();
    engine.call("chat.delete", json!({"id": probe})).unwrap();
    engine.call("chat.purge", json!({"id": probe})).unwrap();
    out
}

#[test]
fn archived_chats_leave_the_sidebar_and_recall_but_stay_whole() {
    let (engine, _) = engine_with(plain);
    let old = chat_with(&engine, "Kastanienfest planen mit der 7b");
    let kept = chat_with(&engine, "Elternabend vorbereiten");
    assert!(recalled(&engine, "Kastanienfest").iter().any(|l| l.contains("Kastanienfest")), "recalled while in use");

    let out = engine.call("chat.archive", json!({"id": old})).unwrap();
    assert_eq!(out["archived"], 1);
    assert!(!listed(&engine).contains(&old) && listed(&engine).contains(&kept), "out of the sidebar");
    assert!(engine.state().conversation(&old).unwrap().archived, "still there, marked");
    assert_eq!(engine.state().archived_conversations().len(), 1);
    // Found by the search over all chats, marked; not recalled while the switch is off.
    let hits = engine.call("search", json!({"query": "Kastanienfest"})).unwrap();
    assert!(hits.as_array().unwrap().iter().any(|h| h["conversation_id"] == old.as_str() && h["archived"] == true), "{hits}");
    assert!(recalled(&engine, "Kastanienfest").is_empty(), "not recalled with the switch off");
    engine.call("settings.set", json!({"key": "recall_archived", "value": true})).unwrap();
    assert!(recalled(&engine, "Kastanienfest").iter().any(|l| l.contains("Kastanienfest")), "recalled with the switch on");
    engine.call("settings.set", json!({"key": "recall_archived", "value": false})).unwrap();
    // It opens as it was, and survives a reload of the list from the store.
    engine.call("chat.open", json!({"id": old})).unwrap();
    assert_eq!(engine.state().messages.len(), 2);
    assert!(engine.ctx().store(|st| st.conversations(500)).unwrap().iter().all(|c| c.id != old));
    assert!(engine.ctx().store(|st| st.archived_conversations(500)).unwrap().iter().any(|c| c.id == old));

    let out = engine.call("chat.unarchive", json!({"id": old})).unwrap();
    assert_eq!(out["restored"], 1);
    assert!(listed(&engine).contains(&old) && !engine.state().conversation(&old).unwrap().archived);

    // A project as a whole, and back.
    let pid = engine.call("project.create", json!({"name": "Bio 7b"})).unwrap()["id"].as_str().unwrap().to_string();
    for id in [&old, &kept] {
        engine.call("chat.move", json!({"id": id, "project": pid})).unwrap();
    }
    assert_eq!(engine.call("chat.archive", json!({"project": pid})).unwrap()["archived"], 2);
    assert!(!listed(&engine).contains(&old) && !listed(&engine).contains(&kept));
    assert_eq!(engine.call("chat.unarchive", json!({"project": pid})).unwrap()["restored"], 2);
    assert!(listed(&engine).contains(&old) && listed(&engine).contains(&kept));
    // Several at once.
    assert_eq!(engine.call("chat.archive", json!({"ids": [old, kept]})).unwrap()["archived"], 2);
    engine.call("chat.unarchive", json!({"ids": [old, kept]})).unwrap();

    // Refused: nothing named, unknown chats and projects, a temporary chat.
    assert!(engine.call("chat.archive", json!({})).is_err());
    assert!(engine.call("chat.archive", json!({"id": "gibt-es-nicht"})).is_err());
    assert!(engine.call("chat.archive", json!({"project": "gibt-es-nicht"})).is_err());
    assert!(engine.call("chat.archive", json!({"ids": "kein-array"})).is_err());
    engine.call("chat.temporary", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Nur kurz"})).unwrap();
    let temp = engine.state().current.clone().unwrap();
    assert!(engine.call("chat.archive", json!({"id": temp})).is_err(), "temporary chats are not archived");
}

#[test]
fn deleted_chats_go_to_the_trash_with_an_undo() {
    let (engine, _) = engine_with(plain);
    let a = chat_with(&engine, "Kastanienfest planen");
    let b = chat_with(&engine, "Elternabend vorbereiten");
    let c = chat_with(&engine, "Wandertag organisieren");

    let out = engine.call("chat.delete", json!({"id": c})).unwrap();
    assert_eq!(out["deleted"], 1);
    let s = engine.state();
    assert!(s.conversation(&c).is_none() && s.current.is_none(), "out of the list, closed");
    assert!(s.trash.iter().any(|x| x.id == c && x.deleted_at.is_some()), "in the trash");
    let notice = s.notices.iter().find(|n| n.text.contains("Gelöscht")).cloned().unwrap();
    assert_eq!(notice.level, NoticeLevel::Info);
    let undo = notice.action.clone().unwrap();
    assert_eq!((undo.label.as_str(), undo.command.as_str()), ("Rückgängig", "chat.restore"));
    // Not searched, not opened while in the trash; still kept in the store.
    assert!(!engine.call("search", json!({"query": "Wandertag"})).unwrap().to_string().contains(&c));
    assert!(engine.call("chat.open", json!({"id": c})).is_err());
    assert!(engine.call("chat.archive", json!({"id": c})).is_err());
    assert_eq!(engine.ctx().store(|st| st.all_messages(&c)).unwrap().len(), 2);
    assert!(engine.call("chat.purge", json!({"id": a})).is_err(), "purged only from the trash");
    // „Rückgängig“ is the notice's own command.
    engine.call(&undo.command, undo.params.clone()).unwrap();
    assert!(engine.state().conversation(&c).is_some() && engine.state().trash.is_empty());
    assert!(engine.call("search", json!({"query": "Wandertag"})).unwrap().to_string().contains(&c));
    engine.call(&undo.command, undo.params).unwrap();

    // Several at once; restored one by one.
    let out = engine.call("chat.delete", json!({"ids": [a, b]})).unwrap();
    assert_eq!(out["deleted"], 2);
    assert!(engine.state().notices.iter().any(|n| n.text == "2 Chats gelöscht"));
    assert_eq!(engine.state().trash.len(), 2);
    assert_eq!(engine.call("chat.restore", json!({"id": a})).unwrap()["restored"], 1);
    assert!(engine.call("chat.restore", json!({"id": "gibt-es-nicht"})).is_err());
    assert!(engine.call("chat.restore", json!({})).is_err());
    assert!(engine.call("chat.delete", json!({})).is_err());
    assert!(engine.call("chat.delete", json!({"id": "gibt-es-nicht"})).is_err());

    // A reload from the store keeps the trash apart.
    let trash = engine.ctx().store(|st| st.trashed_conversations(10)).unwrap();
    assert_eq!(trash.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), [b.as_str()]);

    // Empty the trash; chats longer than 30 days in it go at the hourly tidy-up.
    assert_eq!(engine.call("chat.empty_trash", json!({})).unwrap()["purged"], 1);
    assert!(engine.state().trash.is_empty() && engine.ctx().store(|st| st.conversation(&b)).unwrap().is_none());
    engine.call("chat.delete", json!({"id": a})).unwrap();
    crate::archive::purge_expired(engine.ctx());
    assert_eq!(engine.state().trash.len(), 1, "a fresh deletion stays");
    let old = crate::now_ms() - (aiscraft_core::TRASH_DAYS + 1) * 86_400_000;
    engine.ctx().store(|st| st.set_deleted(&a, Some(old))).unwrap();
    engine.ctx().update(|s| s.trash.iter_mut().for_each(|x| x.deleted_at = Some(old)));
    crate::archive::purge_expired(engine.ctx());
    assert!(engine.state().trash.is_empty() && engine.ctx().store(|st| st.conversation(&a)).unwrap().is_none(), "older than 30 days: gone");
}

/// Quality gate (#77): after `chat.purge` nothing of the chat is left: not in the store, not in
/// the full-text index, no copy of its files in the data folder, no placeholder mapping of
/// „Namen schützen“, no excerpt for recall. Memory notes stay unless asked; documents stay.
#[test]
fn a_purged_chat_leaves_nothing_behind() {
    let (engine, _) = engine_with(plain);
    let dir = std::env::temp_dir().join(format!("aiscraft-purge-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notiz.txt");
    std::fs::write(&file, "Geheimprojekt Kastanie").unwrap();
    engine.call("chat.set_sources", json!({"protect": true})).unwrap();
    engine.call("chat.send", json!({"text": "Merk dir: Kastanienfest am Freitag mit Klaus", "files": [file.display().to_string()]})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.regenerate", json!({})).unwrap();
    let copy = engine.state().messages[0].attachments[0].local_path.clone().unwrap();
    assert!(std::path::Path::new(&copy).exists());
    let map_key = format!("privacy.map.{conv}");
    assert!(engine.ctx().store(|st| st.setting(&map_key)).unwrap().is_some(), "Klaus was replaced");
    let note = engine.state().memory.iter().find(|n| n.text.contains("Kastanienfest")).cloned().expect("a note from the chat");
    let doc = engine.call("artifact.create", json!({"kind": "document", "title": "Festplan", "content": "Ablauf"})).unwrap()["id"].as_str().unwrap().to_string();
    let words = ["Kastanienfest".to_string()];

    // In the trash, all is kept, for a restore.
    engine.call("chat.delete", json!({"id": conv})).unwrap();
    assert!(std::path::Path::new(&copy).exists() && engine.ctx().store(|st| st.setting(&map_key)).unwrap().is_some());

    let out = engine.call("chat.purge", json!({"id": conv})).unwrap();
    assert_eq!((out["purged"].as_u64(), out["notes_forgotten"].as_u64()), (Some(1), Some(0)));
    engine
        .ctx()
        .store(|st| {
            assert!(st.conversation(&conv)?.is_none(), "conversation");
            assert!(st.all_messages(&conv)?.is_empty(), "messages, every branch");
            assert!(st.search("Kastanienfest", 10)?.is_empty(), "full-text index");
            assert!(st.search_any(&words, None, true, 10)?.is_empty(), "recall excerpts");
            assert!(st.setting(&map_key)?.is_none(), "placeholder mapping");
            assert!(st.trashed_conversations(10)?.is_empty() && st.archived_conversations(10)?.is_empty());
            Ok(())
        })
        .unwrap();
    assert!(!std::path::Path::new(&copy).exists(), "the attachment copy in the data folder");
    assert!(crate::privacy::mapping(engine.ctx(), &conv).entries.is_empty());
    let s = engine.state();
    assert!(s.trash.is_empty() && s.conversation(&conv).is_none());
    assert!(s.memory.iter().any(|n| n.id == note.id && !n.archived), "the memory note stays");
    let kept = s.artifacts.iter().find(|a| a.id == doc).expect("the document stays");
    assert!(kept.conversation_id.as_deref() != Some(conv.as_str()));
    assert!(std::fs::read_to_string(&file).is_ok(), "the original file is never touched");

    // With „Auch Notizen aus diesem Chat vergessen“ the chat's notes go too (archived).
    let other = chat_with(&engine, "Merk dir: Wandertag immer im Mai");
    let note = engine.state().memory.iter().find(|n| n.text.contains("Wandertag")).cloned().expect("a note");
    engine.call("chat.delete", json!({"id": other})).unwrap();
    assert_eq!(engine.call("chat.purge", json!({"ids": [other], "forget_notes": true})).unwrap()["notes_forgotten"], 1);
    assert!(engine.state().memory.iter().any(|n| n.id == note.id && n.archived));
    assert!(engine.call("chat.purge", json!({"id": other})).is_err(), "gone: nothing to purge");
    let _ = std::fs::remove_dir_all(dir);
}

/// #77: a chat with AIS.chat is deleted there only on an explicit yes, for one chat; when AIS.chat
/// refuses, nothing is deleted here either.
#[test]
fn deleting_in_ais_chat_needs_an_explicit_yes_for_one_chat() {
    let (engine, posted) = crate::tests::ais_engine();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    // deleteConversationAction has the id 5d5d… in the scripted AIS.chat.
    let delete_action = format!("next-action: {}", "5d".repeat(21));
    let deletes = || posted.lock().unwrap().iter().filter(|r| r.contains(&delete_action)).count();
    // Without the yes: only here, AIS.chat is not asked.
    let second = chat_with(&engine, "Noch einer");
    engine.call("chat.delete", json!({"id": second})).unwrap();
    assert_eq!(deletes(), 0, "deleting here leaves AIS.chat's history alone");
    // A batch never deletes in AIS.chat.
    let third = chat_with(&engine, "Und noch einer");
    assert!(engine.call("chat.delete", json!({"ids": [conv, third], "ais_chat": true})).is_err());
    assert_eq!(deletes(), 0);
    // With the yes, for this one chat.
    let out = engine.call("chat.delete", json!({"id": conv, "ais_chat": true})).unwrap();
    assert_eq!(out["ais_chat"], true);
    assert_eq!(deletes(), 1);
    let req = posted.lock().unwrap().iter().rev().find(|r| r.contains(&delete_action)).cloned().unwrap();
    assert!(req.ends_with(&format!(r#"[{{"conversationId":"{conv}"}}]"#)), "{req}");
    assert!(engine.state().trash.iter().any(|x| x.id == conv));
    // AIS.chat refuses: the chat stays here.
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let refused = engine.state().current.clone().unwrap();
    engine.ctx().update(|s| s.conversations.iter_mut().filter(|x| x.id == refused).for_each(|x| x.remote_id = Some("geschuetzt".into())));
    assert!(engine.call("chat.delete", json!({"id": refused, "ais_chat": true})).is_err());
    assert!(engine.state().conversation(&refused).is_some(), "kept here when AIS.chat refuses");
}

/// #77: the yes for AIS.chat is refused for a chat with another service.
#[test]
fn deleting_in_ais_chat_only_for_ais_chats() {
    let (engine, _) = engine_with(plain);
    let conv = chat_with(&engine, "Hallo");
    assert!(engine.call("chat.delete", json!({"id": conv, "ais_chat": true})).is_err());
    assert!(engine.state().conversation(&conv).is_some());
}
