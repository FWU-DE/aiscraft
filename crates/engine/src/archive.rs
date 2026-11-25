//! Archive and trash (#77).
//!
//! Archived chats leave the sidebar and „Zuletzt bearbeitet“ but stay whole: they open, the
//! search over all chats finds them (marked „archiviert“), and recall takes them only with the
//! setting `recall_archived`. Archiving is always local.
//!
//! Deleting moves a chat into the trash for [`TRASH_DAYS`] days, from where it can be restored.
//! Purging („Endgültig löschen“, „Papierkorb leeren“, or the 30 days running out) removes
//! everything of it: messages on every branch, search entries, the copies of its attachments in
//! the data folder and the „Namen schützen“ mapping, and with that the excerpts recall could take.
//! Memory notes from the chat stay unless the owner asks to forget them too; documents,
//! whiteboards and worksheets from the chat stay (they are objects of their own).
//!
//! AIS.chat keeps its own history: a chat deleted here stays there, unless the owner says yes to
//! „Auch in AIS.chat löschen“, for exactly one chat (`ais_chat: true`), never in a batch.

use std::path::{Path, PathBuf};

use aiscraft_core::{BackendKind, Conversation, Error, Result, TRASH_DAYS};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str};
use crate::{Ctx, NoticeAction, NoticeLevel, now_ms};

/// Chats one command handles at most.
const MAX_IDS: usize = 500;

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "chat.archive",
        label: "Chat archivieren",
        group: "Chat",
        shortcut: None,
        params: "id?: string, ids?: [string] (mehrere), project?: string (alle Chats des Projekts). Verschwindet aus der Seitenleiste und „Zuletzt bearbeitet“, bleibt ganz erhalten",
        palette: false,
        run: |c, p| Box::pin(async move { set_archived(&c, &p, true) }),
    },
    CommandSpec {
        id: "chat.unarchive",
        label: "Aus dem Archiv holen",
        group: "Chat",
        shortcut: None,
        params: "id?: string, ids?: [string], project?: string (alle archivierten Chats des Projekts)",
        palette: false,
        run: |c, p| Box::pin(async move { set_archived(&c, &p, false) }),
    },
    CommandSpec {
        id: "chat.delete",
        label: "Chat löschen",
        group: "Chat",
        shortcut: None,
        params: "id?: string, ids?: [string], ais_chat?: bool (auch im Verlauf von AIS.chat löschen; nur für einen Chat und nur auf ausdrückliches Ja). Legt den Chat 30 Tage in den Papierkorb",
        palette: false,
        run: |c, p| Box::pin(delete(c, p)),
    },
    CommandSpec {
        id: "chat.restore",
        label: "Aus dem Papierkorb wiederherstellen",
        group: "Chat",
        shortcut: None,
        params: "id?: string, ids?: [string]",
        palette: false,
        run: |c, p| Box::pin(async move { restore(&c, &p) }),
    },
    CommandSpec {
        id: "chat.purge",
        label: "Endgültig löschen",
        group: "Chat",
        shortcut: None,
        params: "id?: string, ids?: [string] (Chats im Papierkorb), forget_notes?: bool (auch Notizen aus diesen Chats vergessen). Lässt sich nicht rückgängig machen",
        palette: false,
        run: |c, p| Box::pin(async move { purge_command(&c, &p) }),
    },
    CommandSpec {
        id: "chat.empty_trash",
        label: "Papierkorb leeren",
        group: "Chat",
        shortcut: None,
        params: "forget_notes?: bool. Löscht alle Chats im Papierkorb endgültig",
        palette: false,
        run: |c, p| Box::pin(async move { empty_trash(&c, &p) }),
    },
];

/// The chats a command is about: `id` or `ids` (at most [`MAX_IDS`], each once).
fn ids(p: &Value) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    if let Some(id) = opt_str(p, "id") {
        out.push(id);
    }
    if let Some(list) = p.get("ids") {
        let Some(list) = list.as_array() else { return Err(Error::invalid("ids muss eine Liste von Chat-IDs sein")) };
        if list.len() > MAX_IDS {
            return Err(Error::invalid(format!("Höchstens {MAX_IDS} Chats auf einmal")));
        }
        for v in list {
            let Some(id) = v.as_str() else { return Err(Error::invalid("ids muss eine Liste von Chat-IDs sein")) };
            out.push(id.to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|id| seen.insert(id.clone()));
    Ok(out)
}

fn forget_notes(p: &Value) -> bool {
    p.get("forget_notes").and_then(Value::as_bool).unwrap_or(false)
}

fn set_archived(c: &Ctx, p: &Value, on: bool) -> Result<Value> {
    let state = c.state();
    let targets: Vec<String> = match opt_str(p, "project") {
        Some(pid) => {
            if !state.projects.iter().any(|x| x.id == pid) {
                return Err(Error::not_found("Projekt nicht gefunden"));
            }
            state.conversations.iter().filter(|x| x.project_id.as_deref() == Some(&pid) && !x.temporary && x.archived != on).map(|x| x.id.clone()).collect()
        }
        None => {
            let ids = ids(p)?;
            if ids.is_empty() {
                return Err(Error::invalid("id, ids oder project fehlt"));
            }
            for id in &ids {
                let Some(conv) = state.conversation(id) else {
                    return Err(if state.trash.iter().any(|x| &x.id == id) {
                        Error::invalid("Der Chat liegt im Papierkorb. Stelle ihn zuerst wieder her.")
                    } else {
                        Error::not_found("Chat nicht gefunden")
                    });
                };
                if conv.temporary {
                    return Err(Error::invalid("Ein temporärer Chat lässt sich nicht archivieren: er verschwindet ohnehin beim Schließen."));
                }
            }
            ids
        }
    };
    for id in &targets {
        c.store(|st| st.set_archived(id, on))?;
    }
    c.update(|s| {
        for conv in s.conversations.iter_mut().filter(|x| targets.contains(&x.id)) {
            conv.archived = on;
        }
    });
    Ok(if on { json!({"archived": targets.len()}) } else { json!({"restored": targets.len()}) })
}

async fn delete(c: Ctx, p: Value) -> Result<Value> {
    let ids = ids(&p)?;
    if ids.is_empty() {
        return Err(Error::invalid("id oder ids fehlt"));
    }
    let state = c.state();
    let mut convs: Vec<Conversation> = Vec::new();
    for id in &ids {
        match state.conversation(id) {
            Some(conv) => convs.push(conv.clone()),
            // In the trash already: nothing to do.
            None if state.trash.iter().any(|x| &x.id == id) => {}
            None => return Err(Error::not_found("Chat nicht gefunden")),
        }
    }
    let ais_chat = p.get("ais_chat").and_then(Value::as_bool).unwrap_or(false);
    if ais_chat {
        // Only on the owner's explicit yes, for exactly this chat: never in a batch.
        let [conv] = convs.as_slice() else {
            return Err(Error::invalid("„Auch in AIS.chat löschen“ geht nur für einen Chat auf einmal."));
        };
        if state.backend(&conv.backend_id).is_none_or(|b| b.config.kind != BackendKind::AisChat) {
            return Err(Error::invalid("Dieser Chat stammt nicht aus AIS.chat."));
        }
        let remote = conv.remote_id.clone().unwrap_or_else(|| conv.id.clone());
        // When AIS.chat refuses, nothing is deleted here either: the owner can try again or
        // delete only here.
        c.backend(&conv.backend_id)?.delete_conversation(remote).await?;
    }
    let now = now_ms();
    let mut trashed: Vec<Conversation> = Vec::new();
    for conv in convs {
        if conv.temporary {
            crate::temporary::discard(&c, &conv.id);
            continue;
        }
        crate::chat::stop(&c, &conv.id);
        c.store(|st| st.set_deleted(&conv.id, Some(now)))?;
        trashed.push(Conversation { deleted_at: Some(now), ..conv });
    }
    let gone: Vec<String> = trashed.iter().map(|x| x.id.clone()).collect();
    c.update(|s| {
        s.conversations.retain(|x| !gone.contains(&x.id));
        if s.current.as_ref().is_some_and(|cur| gone.contains(cur)) {
            s.current = None;
            s.messages.clear();
            s.variants.clear();
            s.find = None;
        }
        for conv in trashed.iter().rev() {
            s.trash.insert(0, conv.clone());
        }
        if let Some(search) = s.search.as_mut() {
            search.hits.retain(|h| !gone.contains(&h.conversation_id));
        }
    });
    if !gone.is_empty() {
        let text = match trashed.as_slice() {
            [one] => format!("Gelöscht: „{}“", crate::privacy::reveal(&c, &one.id, &one.title)),
            many => format!("{} Chats gelöscht", many.len()),
        };
        c.notice_with(NoticeLevel::Info, text, Some(NoticeAction { label: "Rückgängig".into(), command: "chat.restore".into(), params: json!({"ids": gone}) }));
    }
    Ok(json!({"deleted": gone.len(), "ais_chat": ais_chat}))
}

fn restore(c: &Ctx, p: &Value) -> Result<Value> {
    let ids = ids(p)?;
    if ids.is_empty() {
        return Err(Error::invalid("id oder ids fehlt"));
    }
    let state = c.state();
    let mut back = Vec::new();
    for id in &ids {
        match state.trash.iter().find(|x| &x.id == id) {
            Some(conv) => back.push(Conversation { deleted_at: None, ..conv.clone() }),
            // Back already (a second „Rückgängig“).
            None if state.conversation(id).is_some() => {}
            None => return Err(Error::not_found("Chat nicht im Papierkorb")),
        }
    }
    for conv in &back {
        c.store(|st| st.set_deleted(&conv.id, None))?;
    }
    let restored: Vec<String> = back.iter().map(|x| x.id.clone()).collect();
    c.update(|s| {
        s.trash.retain(|x| !restored.contains(&x.id));
        s.conversations.extend(back);
        s.conversations.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
    });
    Ok(json!({"restored": restored.len()}))
}

fn purge_command(c: &Ctx, p: &Value) -> Result<Value> {
    let ids = ids(p)?;
    if ids.is_empty() {
        return Err(Error::invalid("id oder ids fehlt"));
    }
    let state = c.state();
    if let Some(id) = ids.iter().find(|id| !state.trash.iter().any(|x| &x.id == *id)) {
        return Err(if state.conversation(id).is_some() {
            Error::invalid("Endgültig gelöscht wird nur aus dem Papierkorb. Lösche den Chat zuerst.")
        } else {
            Error::not_found("Chat nicht im Papierkorb")
        });
    }
    let forget = forget_notes(p);
    let mut notes = 0;
    for id in &ids {
        notes += purge(c, id, forget)?;
    }
    Ok(json!({"purged": ids.len(), "notes_forgotten": notes}))
}

fn empty_trash(c: &Ctx, p: &Value) -> Result<Value> {
    let ids: Vec<String> = c.state().trash.iter().map(|x| x.id.clone()).collect();
    let forget = forget_notes(p);
    let mut notes = 0;
    for id in &ids {
        notes += purge(c, id, forget)?;
    }
    Ok(json!({"purged": ids.len(), "notes_forgotten": notes}))
}

/// Chats longer than [`TRASH_DAYS`] days in the trash go for good (at start and hourly).
pub(crate) fn purge_expired(c: &Ctx) {
    let before = now_ms() - TRASH_DAYS * 24 * 3_600_000;
    let due: Vec<String> = c.state().trash.iter().filter(|x| x.deleted_at.is_some_and(|at| at < before)).map(|x| x.id.clone()).collect();
    for id in due {
        if let Err(e) = purge(c, &id, false) {
            log::warn!("Papierkorb leeren: {e}");
        }
    }
}

/// Remove one chat entirely; returns how many memory notes were forgotten with it.
fn purge(c: &Ctx, id: &str, forget_notes: bool) -> Result<usize> {
    crate::chat::stop(c, id);
    let messages = c.store(|st| st.all_messages(id))?;
    // Copies of its files (attached and generated), only those in the attachments folder: a
    // path from the database is never trusted blindly.
    let dirs = [c.0.attachments_dir.clone(), crate::temporary::attachments_dir(c)];
    for path in messages.iter().flat_map(|m| &m.attachments).filter_map(|a| a.local_path.as_deref()).map(PathBuf::from) {
        if in_one_of(&path, &dirs) {
            let _ = std::fs::remove_file(&path);
        }
    }
    let mut forgotten = Vec::new();
    if forget_notes {
        let ids: std::collections::HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();
        for note in c.state().memory.iter().filter(|n| !n.archived && n.source_message_id.as_deref().is_some_and(|s| ids.contains(s))) {
            let note = aiscraft_core::MemoryNote { archived: true, ..note.clone() };
            c.store(|st| st.save_memory(&note))?;
            forgotten.push(note.id);
        }
    }
    c.store(|st| st.delete_conversation(id))?;
    crate::privacy::forget(c, id);
    let id = id.to_string();
    c.update(|s| {
        s.trash.retain(|x| x.id != id);
        s.conversations.retain(|x| x.id != id);
        for n in s.memory.iter_mut().filter(|n| forgotten.contains(&n.id)) {
            n.archived = true;
        }
        for a in s.artifacts.iter_mut().filter(|a| a.conversation_id.as_deref() == Some(id.as_str())) {
            a.conversation_id = None;
        }
    });
    Ok(forgotten.len())
}

/// Whether `path` lies directly in one of `dirs`.
fn in_one_of(path: &Path, dirs: &[PathBuf]) -> bool {
    path.parent().is_some_and(|parent| dirs.iter().any(|d| d.as_path() == parent))
}
