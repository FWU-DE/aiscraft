//! Temporary chats (#34): the conversation and its messages live only in the store's database in
//! memory (`aiscraft_store::Store` keeps them apart), copies of attached files in their own folder.
//! Once the chat is closed, everything goes: messages, search entries, the „Namen schützen“
//! mapping and the file copies. Memory notes and learned skills are never made from it.

use std::path::PathBuf;

use crate::Ctx;

/// Where copies of files attached in temporary chats go; emptied when they close and at start.
pub(crate) fn attachments_dir(c: &Ctx) -> PathBuf {
    c.0.attachments_dir.join("temporary")
}

/// Empty the folder of a run that ended without closing its temporary chats (a crash).
pub(crate) fn clean_up(c: &Ctx) {
    let _ = std::fs::remove_dir_all(attachments_dir(c));
}

/// Forget one temporary chat entirely.
pub(crate) fn discard(c: &Ctx, id: &str) {
    crate::chat::stop(c, id);
    let dir = attachments_dir(c);
    // Copies of its files (only those in the temporary folder: a path is never trusted blindly).
    let messages = c.store(|st| st.messages(id)).unwrap_or_default();
    for path in messages.iter().flat_map(|m| &m.attachments).filter_map(|a| a.local_path.as_deref()).map(PathBuf::from) {
        if path.parent() == Some(dir.as_path()) {
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = c.store(|st| st.delete_conversation(id));
    crate::privacy::forget(c, id);
    c.update(|s| {
        s.conversations.retain(|x| x.id != id);
        s.generating.retain(|g| g != id);
    });
}

/// Every temporary chat that is not the open one is closed: forget it.
pub(crate) fn discard_closed(c: &Ctx) {
    let state = c.state();
    let closed: Vec<String> =
        state.conversations.iter().filter(|x| x.temporary && state.current.as_deref() != Some(x.id.as_str())).map(|x| x.id.clone()).collect();
    for id in closed {
        discard(c, &id);
    }
}
