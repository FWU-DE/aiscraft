//! Local storage (layer 1): one SQLite file with conversations, messages, projects, memory,
//! backends (without secrets) and settings, plus an FTS5 index over every message.

use std::collections::BTreeMap;
use std::path::Path;

use aiscraft_core::{
    Artifact, ArtifactKind, ArtifactVersion, BackendConfig, Conversation, Error, MemoryArea, MemoryNote, Message, Project, ProjectFile, Result, Role,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SCHEMA_VERSION: i64 = 4;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS backends (id TEXT PRIMARY KEY, position INTEGER NOT NULL, config TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, instructions TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY, title TEXT NOT NULL, backend_id TEXT NOT NULL, model_id TEXT NOT NULL,
    project_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS conversations_updated ON conversations(updated_at DESC);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at INTEGER NOT NULL, extra TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_conv ON messages(conversation_id, seq);
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(content, message_id UNINDEXED, conversation_id UNINDEXED, tokenize = 'unicode61 remove_diacritics 2');
CREATE TABLE IF NOT EXISTS memory (
    id TEXT PRIMARY KEY, area TEXT NOT NULL, text TEXT NOT NULL, source_message_id TEXT, source_quote TEXT NOT NULL,
    created_at INTEGER NOT NULL, archived INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS message_heads (conversation_id TEXT PRIMARY KEY, head TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, conversation_id TEXT, project_id TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, state BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS action_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT, at INTEGER NOT NULL, conversation_id TEXT NOT NULL, kind TEXT NOT NULL,
    name TEXT NOT NULL, detail TEXT NOT NULL, reason TEXT NOT NULL, outcome TEXT NOT NULL,
    input_tokens INTEGER, output_tokens INTEGER, usd REAL
);
CREATE TABLE IF NOT EXISTS tool_claims (
    id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL, tool TEXT NOT NULL, args_hash TEXT NOT NULL,
    state TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS tool_claims_args ON tool_claims(conversation_id, tool, args_hash);
CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE, version INTEGER NOT NULL,
    created_at INTEGER NOT NULL, note TEXT NOT NULL, state BLOB NOT NULL, PRIMARY KEY (artifact_id, version)
);
CREATE TABLE IF NOT EXISTS project_files (
    id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL,
    chars INTEGER NOT NULL, path TEXT NOT NULL, text TEXT NOT NULL, added_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS project_files_project ON project_files(project_id);
CREATE VIRTUAL TABLE IF NOT EXISTS project_chunks USING fts5(text, file_id UNINDEXED, project_id UNINDEXED, idx UNINDEXED, tokenize = 'unicode61 remove_diacritics 2');
"#;

/// What a message carries besides its text, stored as one JSON column.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Extra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<aiscraft_core::Attachment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    activity: Vec<aiscraft_core::ActivityStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<aiscraft_core::Usage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<aiscraft_core::ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_result: Option<aiscraft_core::ToolResultInfo>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    thinking: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    citations: Vec<aiscraft_core::Citation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    pub month: String,
    pub backend_id: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub answers: i64,
}

/// The action log keeps this many entries (#23).
pub const MAX_LOG: i64 = 5000;

/// One entry of the action log: a model call or a tool action, with why it was allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: i64,
    pub conversation_id: String,
    /// "model" or "tool".
    pub kind: String,
    /// The model, or connector and tool.
    pub name: String,
    /// The arguments in short (secrets scrubbed), or what the call was for.
    pub detail: String,
    /// Why it ran: „Nur lesen“, „Deine Freigabe“, the owner's words, a rule; or why not.
    pub reason: String,
    pub outcome: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub conversation_id: String,
    pub message_id: String,
    pub title: String,
    /// Text around the match, with `«` `»` around matched words.
    pub snippet: String,
    /// The backend (service) of the chat the hit is in; empty when the chat is gone.
    #[serde(default)]
    pub backend_id: String,
}

/// Conversation fields that live in one JSON column (added in schema 3).
#[derive(Default, Serialize, Deserialize)]
struct ConvExtra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    persona: Option<aiscraft_core::Persona>,
    #[serde(default)]
    kind: aiscraft_core::ConversationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_id: Option<String>,
}

pub struct Store {
    db: Connection,
    /// Temporary chats (#34): their conversation, messages and search entries live in this
    /// database in memory, never in the file, and are not searched or recalled.
    temp: Option<Box<Store>>,
    temp_ids: std::sync::Mutex<std::collections::HashSet<String>>,
}

fn db_err(e: rusqlite::Error) -> Error {
    // A full disk is named as such (SQLITE_FULL, or the I/O error under it), from the codes.
    let full = match &e {
        rusqlite::Error::SqliteFailure(f, _) => f.code == rusqlite::ErrorCode::DiskFull,
        _ => false,
    };
    if full { Error::disk_full(format_args!("Datenbank: {e}")) } else { Error::storage(format!("Datenbank: {e}")) }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(format_args!("Ordner {}", dir.display()), &e))?;
        }
        Self::with_temp(Self::init(Connection::open(path).map_err(db_err)?)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::with_temp(Self::init(Connection::open_in_memory().map_err(db_err)?)?)
    }

    fn with_temp(mut st: Self) -> Result<Self> {
        st.temp = Some(Box::new(Self::init(Connection::open_in_memory().map_err(db_err)?)?));
        Ok(st)
    }

    /// Whether this conversation is a temporary one (kept in memory only).
    pub fn is_temporary(&self, conversation_id: &str) -> bool {
        self.temp_ids.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(conversation_id)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;").map_err(db_err)?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(db_err)?;
        if version > SCHEMA_VERSION {
            return Err(Error::storage("Die Datenbank stammt aus einer neueren AIS.ChatCraft-Version"));
        }
        db.execute_batch(SCHEMA).map_err(db_err)?;
        // 1 → 2: per-chat sources.
        let has_sources: bool =
            db.prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'sources'").and_then(|mut st| st.exists([])).map_err(db_err)?;
        if !has_sources {
            db.execute_batch("ALTER TABLE conversations ADD COLUMN sources TEXT NOT NULL DEFAULT '{}'").map_err(db_err)?;
        }
        // 2 → 3: persona (assistant, character, learning scenario), kind and server id.
        let has_extra: bool =
            db.prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'extra'").and_then(|mut st| st.exists([])).map_err(db_err)?;
        if !has_extra {
            db.execute_batch("ALTER TABLE conversations ADD COLUMN extra TEXT NOT NULL DEFAULT '{}'").map_err(db_err)?;
        }
        // 3 → 4: messages form a tree (variants after edit and regenerate). Existing chats become
        // one linear branch: each message follows the one before it.
        let has_parent: bool =
            db.prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = 'parent_id'").and_then(|mut st| st.exists([])).map_err(db_err)?;
        if !has_parent {
            let tx = db.unchecked_transaction().map_err(db_err)?;
            tx.execute_batch(
                "ALTER TABLE messages ADD COLUMN parent_id TEXT NOT NULL DEFAULT '';
                 ALTER TABLE messages ADD COLUMN picked INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE messages ADD COLUMN active INTEGER NOT NULL DEFAULT 1;
                 UPDATE messages SET picked = seq, parent_id = COALESCE((SELECT p.id FROM messages p
                   WHERE p.conversation_id = messages.conversation_id AND p.seq < messages.seq ORDER BY p.seq DESC LIMIT 1), '');",
            )
            .map_err(db_err)?;
            tx.commit().map_err(db_err)?;
        }
        db.execute_batch("CREATE INDEX IF NOT EXISTS messages_parent ON messages(conversation_id, parent_id)").map_err(db_err)?;
        db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}")).map_err(db_err)?;
        Ok(Self { db, temp: None, temp_ids: Default::default() })
    }

    // ---- backends -------------------------------------------------------------------------

    pub fn backends(&self) -> Result<Vec<BackendConfig>> {
        let mut st = self.db.prepare("SELECT config FROM backends ORDER BY position, id").map_err(db_err)?;
        let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            // A config we cannot read (written by a newer version) is skipped, not fatal.
            if let Ok(c) = serde_json::from_str(&row.map_err(db_err)?) {
                out.push(c);
            }
        }
        Ok(out)
    }

    pub fn save_backend(&self, b: &BackendConfig) -> Result<()> {
        let pos: i64 = self.db.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM backends", [], |r| r.get(0)).map_err(db_err)?;
        self.db
            .execute(
                "INSERT INTO backends (id, position, config) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET config = excluded.config",
                params![b.id, pos, serde_json::to_string(b)?],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_backend(&self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM backends WHERE id = ?1", [id]).map_err(db_err)?;
        Ok(())
    }

    // ---- conversations --------------------------------------------------------------------

    fn conv_row(r: &rusqlite::Row) -> rusqlite::Result<Conversation> {
        let extra: ConvExtra = r.get::<_, String>(9).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Ok(Conversation {
            id: r.get(0)?,
            title: r.get(1)?,
            backend_id: r.get(2)?,
            model_id: r.get(3)?,
            project_id: r.get(4)?,
            created_at: r.get(5)?,
            updated_at: r.get(6)?,
            pinned: r.get::<_, i64>(7)? != 0,
            sources: r.get::<_, String>(8).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            persona: extra.persona,
            kind: extra.kind,
            remote_id: extra.remote_id,
            temporary: false,
        })
    }

    const CONV_COLS: &'static str = "id, title, backend_id, model_id, project_id, created_at, updated_at, pinned, sources, extra";

    /// Pinned first, then most recently updated.
    pub fn conversations(&self, limit: usize) -> Result<Vec<Conversation>> {
        let sql = format!("SELECT {} FROM conversations ORDER BY pinned DESC, updated_at DESC LIMIT ?1", Self::CONV_COLS);
        let mut st = self.db.prepare(&sql).map_err(db_err)?;
        let rows = st.query_map([limit as i64], Self::conv_row).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    pub fn conversation(&self, id: &str) -> Result<Option<Conversation>> {
        if self.is_temporary(id)
            && let Some(t) = &self.temp
        {
            return Ok(t.conversation(id)?.map(|c| Conversation { temporary: true, ..c }));
        }
        let sql = format!("SELECT {} FROM conversations WHERE id = ?1", Self::CONV_COLS);
        self.db.query_row(&sql, [id], Self::conv_row).optional().map_err(db_err)
    }

    pub fn save_conversation(&self, c: &Conversation) -> Result<()> {
        if c.temporary
            && let Some(t) = &self.temp
        {
            self.temp_ids.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(c.id.clone());
            return t.save_conversation(&Conversation { temporary: false, ..c.clone() });
        }
        self.db
            .execute(
                "INSERT INTO conversations (id, title, backend_id, model_id, project_id, created_at, updated_at, pinned, sources, extra)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(id) DO UPDATE SET title = excluded.title, backend_id = excluded.backend_id, model_id = excluded.model_id,
                   project_id = excluded.project_id, updated_at = excluded.updated_at, pinned = excluded.pinned, sources = excluded.sources,
                   extra = excluded.extra",
                params![
                    c.id,
                    c.title,
                    c.backend_id,
                    c.model_id,
                    c.project_id,
                    c.created_at,
                    c.updated_at,
                    c.pinned as i64,
                    serde_json::to_string(&c.sources)?,
                    serde_json::to_string(&ConvExtra { persona: c.persona.clone(), kind: c.kind, remote_id: c.remote_id.clone() })?
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        if self.is_temporary(id)
            && let Some(t) = &self.temp
        {
            t.delete_conversation(id)?;
            self.temp_ids.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id);
            return Ok(());
        }
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute("DELETE FROM messages_fts WHERE conversation_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM messages WHERE conversation_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM message_heads WHERE conversation_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM conversations WHERE id = ?1", [id]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    // ---- messages -------------------------------------------------------------------------
    //
    // Messages form a tree: each one follows its parent (`parent_id`, '' for the first). Edit and
    // regenerate add a sibling instead of deleting, so earlier versions stay. Among siblings the
    // one with the highest `picked` is shown. The head (`message_heads`) is the last message of
    // the shown branch; a new message goes after it. `active` marks the shown branch, and only
    // the shown branch is in the search index.

    /// Every message of a conversation with its place in the tree, in creation order.
    fn rows(&self, conversation_id: &str) -> Result<Vec<(Node, Message)>> {
        let mut st = self
            .db
            .prepare("SELECT id, role, content, created_at, extra, parent_id, picked, seq FROM messages WHERE conversation_id = ?1 ORDER BY seq")
            .map_err(db_err)?;
        let rows = st
            .query_map([conversation_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, role, content, created_at, extra, parent, picked, seq) = row.map_err(db_err)?;
            let Some(role) = Role::parse(&role) else { continue };
            let extra: Extra = serde_json::from_str(&extra).unwrap_or_default();
            out.push((
                Node { id: id.clone(), parent, picked, seq },
                Message {
                    id,
                    conversation_id: conversation_id.to_string(),
                    role,
                    content,
                    created_at,
                    model: extra.model,
                    attachments: extra.attachments,
                    activity: extra.activity,
                    error: extra.error,
                    usage: extra.usage,
                    tool_calls: extra.tool_calls,
                    tool_result: extra.tool_result,
                    thinking: extra.thinking,
                    citations: extra.citations,
                },
            ));
        }
        Ok(out)
    }

    fn nodes(&self, conversation_id: &str) -> Result<Vec<Node>> {
        let mut st = self.db.prepare("SELECT id, parent_id, picked, seq FROM messages WHERE conversation_id = ?1 ORDER BY seq").map_err(db_err)?;
        let rows = st.query_map([conversation_id], |r| Ok(Node { id: r.get(0)?, parent: r.get(1)?, picked: r.get(2)?, seq: r.get(3)? })).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    /// The stored head: `None` when there is none (chats from before schema 4), `Some("")`
    /// before the first message.
    fn stored_head(&self, conversation_id: &str) -> Result<Option<String>> {
        self.db.query_row("SELECT head FROM message_heads WHERE conversation_id = ?1", [conversation_id], |r| r.get(0)).optional().map_err(db_err)
    }

    fn set_head(&self, conversation_id: &str, head: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO message_heads (conversation_id, head) VALUES (?1, ?2) ON CONFLICT(conversation_id) DO UPDATE SET head = excluded.head",
                params![conversation_id, head],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// The shown branch of a conversation, oldest first.
    pub fn messages(&self, conversation_id: &str) -> Result<Vec<Message>> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.messages(conversation_id);
        }
        let rows = self.rows(conversation_id)?;
        let head = self.stored_head(conversation_id)?;
        let nodes: Vec<&Node> = rows.iter().map(|(n, _)| n).collect();
        let path = active_path(&nodes, head.as_deref());
        let mut msgs: Vec<Option<Message>> = rows.into_iter().map(|(_, m)| Some(m)).collect();
        Ok(path.into_iter().filter_map(|i| msgs.get_mut(i).and_then(Option::take)).collect())
    }

    /// For each message of the shown branch that has other versions: which one it is (from 1)
    /// and how many there are.
    pub fn variants(&self, conversation_id: &str) -> Result<BTreeMap<String, Variant>> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.variants(conversation_id);
        }
        let nodes = self.nodes(conversation_id)?;
        let refs: Vec<&Node> = nodes.iter().collect();
        let head = self.stored_head(conversation_id)?;
        let mut out = BTreeMap::new();
        for i in active_path(&refs, head.as_deref()) {
            let Some(n) = nodes.get(i) else { continue };
            let siblings: Vec<&Node> = nodes.iter().filter(|s| s.parent == n.parent).collect();
            if siblings.len() > 1
                && let Some(pos) = siblings.iter().position(|s| s.id == n.id)
            {
                out.insert(n.id.clone(), Variant { index: pos + 1, count: siblings.len() });
            }
        }
        Ok(out)
    }

    /// Mark the shown branch and index only it for search.
    fn relayout(&self, conversation_id: &str) -> Result<()> {
        let nodes = self.nodes(conversation_id)?;
        let refs: Vec<&Node> = nodes.iter().collect();
        let head = self.stored_head(conversation_id)?;
        let path = active_path(&refs, head.as_deref());
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute("UPDATE messages SET active = 0 WHERE conversation_id = ?1", [conversation_id]).map_err(db_err)?;
        for i in path {
            if let Some(n) = nodes.get(i) {
                tx.execute("UPDATE messages SET active = 1 WHERE id = ?1", [&n.id]).map_err(db_err)?;
            }
        }
        tx.execute("DELETE FROM messages_fts WHERE conversation_id = ?1", [conversation_id]).map_err(db_err)?;
        tx.execute(
            "INSERT INTO messages_fts (content, message_id, conversation_id) SELECT content, id, conversation_id FROM messages
             WHERE conversation_id = ?1 AND active = 1 AND trim(content, ' ' || char(9, 10, 13)) != ''",
            [conversation_id],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// Insert or replace a message (keeps its place) and its search entry. A new message goes
    /// after the head and becomes the head.
    pub fn save_message(&self, m: &Message) -> Result<()> {
        if self.is_temporary(&m.conversation_id)
            && let Some(t) = &self.temp
        {
            return t.save_message(m);
        }
        let extra = Extra {
            model: m.model.clone(),
            attachments: m.attachments.iter().map(|a| aiscraft_core::Attachment { data_base64: None, ..a.clone() }).collect(),
            activity: m.activity.clone(),
            error: m.error.clone(),
            usage: m.usage,
            tool_calls: m.tool_calls.clone(),
            tool_result: m.tool_result.clone(),
            thinking: m.thinking.clone(),
            citations: m.citations.clone(),
        };
        let exists: Option<bool> =
            self.db.query_row("SELECT active FROM messages WHERE id = ?1", [&m.id], |r| r.get::<_, i64>(0).map(|a| a != 0)).optional().map_err(db_err)?;
        let parent = match exists {
            Some(_) => String::new(),
            None => self.tip(&m.conversation_id)?,
        };
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        let (seq, picked): (i64, i64) = tx
            .query_row(
                "SELECT COALESCE((SELECT seq FROM messages WHERE id = ?1), (SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE conversation_id = ?2)),
                        (SELECT COALESCE(MAX(picked), 0) + 1 FROM messages WHERE conversation_id = ?2)",
                params![m.id, m.conversation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_err)?;
        tx.execute(
            "INSERT INTO messages (id, conversation_id, seq, role, content, created_at, extra, parent_id, picked, active)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
             ON CONFLICT(id) DO UPDATE SET content = excluded.content, extra = excluded.extra",
            params![m.id, m.conversation_id, seq, m.role.as_str(), m.content, m.created_at, serde_json::to_string(&extra)?, parent, picked],
        )
        .map_err(db_err)?;
        if exists.is_none() {
            tx.execute(
                "INSERT INTO message_heads (conversation_id, head) VALUES (?1, ?2) ON CONFLICT(conversation_id) DO UPDATE SET head = excluded.head",
                params![m.conversation_id, m.id],
            )
            .map_err(db_err)?;
        }
        tx.execute("DELETE FROM messages_fts WHERE message_id = ?1", [&m.id]).map_err(db_err)?;
        if exists.unwrap_or(true) && !m.content.trim().is_empty() {
            tx.execute("INSERT INTO messages_fts (content, message_id, conversation_id) VALUES (?1, ?2, ?3)", params![m.content, m.id, m.conversation_id])
                .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    /// The message a new one goes after: the head, or the end of the shown branch.
    fn tip(&self, conversation_id: &str) -> Result<String> {
        if let Some(h) = self.stored_head(conversation_id)? {
            return Ok(h);
        }
        let nodes = self.nodes(conversation_id)?;
        let refs: Vec<&Node> = nodes.iter().collect();
        Ok(active_path(&refs, None).last().and_then(|&i| nodes.get(i)).map(|n| n.id.clone()).unwrap_or_default())
    }

    /// Give a message the id the server assigned to it.
    pub fn rename_message(&self, old: &str, new: &str) -> Result<()> {
        if old == new {
            return Ok(());
        }
        if let Some(t) = &self.temp {
            t.rename_message(old, new)?;
        }
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute("UPDATE messages SET id = ?2 WHERE id = ?1", params![old, new]).map_err(db_err)?;
        tx.execute("UPDATE messages SET parent_id = ?2 WHERE parent_id = ?1", params![old, new]).map_err(db_err)?;
        tx.execute("UPDATE message_heads SET head = ?2 WHERE head = ?1", params![old, new]).map_err(db_err)?;
        tx.execute("UPDATE messages_fts SET message_id = ?2 WHERE message_id = ?1", params![old, new]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// The next message saved becomes a new version of `message_id` (for edit and regenerate):
    /// the shown branch ends just before it. The old version and what followed it stay.
    pub fn fork_at(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.fork_at(conversation_id, message_id);
        }
        let parent: Option<String> = self
            .db
            .query_row("SELECT parent_id FROM messages WHERE id = ?1 AND conversation_id = ?2", params![message_id, conversation_id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let Some(parent) = parent else { return Err(Error::not_found("Nachricht nicht gefunden")) };
        self.set_head(conversation_id, &parent)?;
        self.relayout(conversation_id)
    }

    /// Undo [`Store::fork_at`] when nothing new was saved since (the edit or regeneration
    /// failed before it began): `message_id`'s branch is shown again.
    pub fn unfork(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.unfork(conversation_id, message_id);
        }
        let parent: Option<String> = self
            .db
            .query_row("SELECT parent_id FROM messages WHERE id = ?1 AND conversation_id = ?2", params![message_id, conversation_id], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        match parent {
            Some(p) if self.stored_head(conversation_id)?.as_deref() == Some(p.as_str()) => self.show_branch(conversation_id, message_id),
            _ => Ok(()),
        }
    }

    /// Show the version `index` (from 1) among `message_id` and its siblings, with the branch
    /// that was last shown below it. Returns how many versions there are.
    pub fn select_variant(&self, conversation_id: &str, message_id: &str, index: usize) -> Result<usize> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.select_variant(conversation_id, message_id, index);
        }
        let nodes = self.nodes(conversation_id)?;
        let Some(me) = nodes.iter().find(|n| n.id == message_id) else { return Err(Error::not_found("Nachricht nicht gefunden")) };
        let siblings: Vec<&Node> = nodes.iter().filter(|n| n.parent == me.parent).collect();
        let Some(target) = index.checked_sub(1).and_then(|i| siblings.get(i)) else {
            return Err(Error::invalid(format!("Fassung {index} gibt es nicht (1 bis {})", siblings.len())));
        };
        let id = target.id.clone();
        self.show_branch(conversation_id, &id)?;
        Ok(siblings.len())
    }

    /// Make `message_id` the shown child of its parent, and the head the end of its branch.
    fn show_branch(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE messages SET picked = (SELECT COALESCE(MAX(picked), 0) + 1 FROM messages WHERE conversation_id = ?1) WHERE id = ?2 AND conversation_id = ?1",
                params![conversation_id, message_id],
            )
            .map_err(db_err)?;
        let nodes = self.nodes(conversation_id)?;
        let refs: Vec<&Node> = nodes.iter().collect();
        let leaf = descend(&refs, message_id);
        self.set_head(conversation_id, &leaf)?;
        self.relayout(conversation_id)
    }

    /// Delete a message and everything that follows it on any branch (a schedule run without news).
    pub fn truncate_from(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        if self.is_temporary(conversation_id)
            && let Some(t) = &self.temp
        {
            return t.truncate_from(conversation_id, message_id);
        }
        let nodes = self.nodes(conversation_id)?;
        let Some(me) = nodes.iter().find(|n| n.id == message_id) else { return Ok(()) };
        let parent = me.parent.clone();
        let mut doomed = vec![message_id.to_string()];
        let mut i = 0;
        while let Some(id) = doomed.get(i).cloned() {
            doomed.extend(nodes.iter().filter(|n| n.parent == id).map(|n| n.id.clone()));
            i += 1;
            if i > nodes.len() {
                break;
            }
        }
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        for id in &doomed {
            tx.execute("DELETE FROM messages_fts WHERE message_id = ?1", [id]).map_err(db_err)?;
            tx.execute("DELETE FROM messages WHERE id = ?1 AND conversation_id = ?2", params![id, conversation_id]).map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        self.set_head(conversation_id, &parent)?;
        self.relayout(conversation_id)
    }

    /// Full-text search over every message. Each word matches as a prefix; all must occur.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = fts_query(query) else { return Ok(Vec::new()) };
        let mut st = self
            .db
            .prepare(
                "SELECT f.conversation_id, f.message_id, COALESCE(c.title, ''), snippet(messages_fts, 0, '«', '»', '…', 12), COALESCE(c.backend_id, '')
                 FROM messages_fts f LEFT JOIN conversations c ON c.id = f.conversation_id
                 WHERE messages_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![fts, limit as i64], |r| {
                Ok(SearchHit { conversation_id: r.get(0)?, message_id: r.get(1)?, title: r.get(2)?, snippet: r.get(3)?, backend_id: r.get(4)? })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    // ---- projects -------------------------------------------------------------------------

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut st = self.db.prepare("SELECT id, name, instructions, created_at FROM projects ORDER BY name COLLATE NOCASE").map_err(db_err)?;
        let rows = st.query_map([], |r| Ok(Project { id: r.get(0)?, name: r.get(1)?, instructions: r.get(2)?, created_at: r.get(3)? })).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    pub fn save_project(&self, p: &Project) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO projects (id, name, instructions, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, instructions = excluded.instructions",
                params![p.id, p.name, p.instructions, p.created_at],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Deleting a project keeps its chats; they just leave the project. Its files go (#32).
    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM project_chunks WHERE project_id = ?1", [id]).map_err(db_err)?;
        self.db.execute("DELETE FROM project_files WHERE project_id = ?1", [id]).map_err(db_err)?;
        self.db.execute("UPDATE conversations SET project_id = NULL WHERE project_id = ?1", [id]).map_err(db_err)?;
        self.db.execute("DELETE FROM projects WHERE id = ?1", [id]).map_err(db_err)?;
        Ok(())
    }

    // ---- memory ---------------------------------------------------------------------------

    pub fn memory(&self) -> Result<Vec<MemoryNote>> {
        let mut st =
            self.db.prepare("SELECT id, area, text, source_message_id, source_quote, created_at, archived FROM memory ORDER BY created_at").map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, area, text, source_message_id, source_quote, created_at, archived) = row.map_err(db_err)?;
            out.push(MemoryNote {
                id,
                area: MemoryArea::parse(&area).unwrap_or(MemoryArea::Lessons),
                text,
                source_message_id,
                source_quote,
                created_at,
                archived: archived != 0,
            });
        }
        Ok(out)
    }

    pub fn save_memory(&self, n: &MemoryNote) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO memory (id, area, text, source_message_id, source_quote, created_at, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET area = excluded.area, text = excluded.text, archived = excluded.archived",
                params![n.id, n.area.as_str(), n.text, n.source_message_id, n.source_quote, n.created_at, n.archived as i64],
            )
            .map_err(db_err)?;
        Ok(())
    }

    // ---- artifacts ------------------------------------------------------------------------

    pub fn artifacts(&self) -> Result<Vec<Artifact>> {
        let mut st = self
            .db
            .prepare("SELECT id, kind, title, conversation_id, project_id, created_at, updated_at FROM artifacts ORDER BY updated_at DESC")
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, kind, title, conversation_id, project_id, created_at, updated_at) = row.map_err(db_err)?;
            let Some(kind) = ArtifactKind::parse(&kind) else { continue };
            out.push(Artifact { id, kind, title, conversation_id, project_id, created_at, updated_at });
        }
        Ok(out)
    }

    /// Insert or update an artifact; `state` replaces the stored content when given.
    pub fn save_artifact(&self, a: &Artifact, state: Option<&[u8]>) -> Result<()> {
        match state {
            Some(st) => self.db.execute(
                "INSERT INTO artifacts (id, kind, title, conversation_id, project_id, created_at, updated_at, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET title = excluded.title, conversation_id = excluded.conversation_id, project_id = excluded.project_id,
                   updated_at = excluded.updated_at, state = excluded.state",
                params![a.id, a.kind.as_str(), a.title, a.conversation_id, a.project_id, a.created_at, a.updated_at, st],
            ),
            None => self.db.execute(
                "UPDATE artifacts SET title = ?2, conversation_id = ?3, project_id = ?4, updated_at = ?5 WHERE id = ?1",
                params![a.id, a.title, a.conversation_id, a.project_id, a.updated_at],
            ),
        }
        .map_err(db_err)?;
        Ok(())
    }

    pub fn artifact_state(&self, id: &str) -> Result<Option<Vec<u8>>> {
        self.db.query_row("SELECT state FROM artifacts WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(db_err)
    }

    pub fn delete_artifact(&self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM artifact_versions WHERE artifact_id = ?1", [id]).map_err(db_err)?;
        self.db.execute("DELETE FROM artifacts WHERE id = ?1", [id]).map_err(db_err)?;
        Ok(())
    }

    /// Keep a snapshot; returns its version number. At most 50 are kept per artifact.
    pub fn add_artifact_version(&self, id: &str, note: &str, state: &[u8], at: i64) -> Result<i64> {
        let v: i64 =
            self.db.query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM artifact_versions WHERE artifact_id = ?1", [id], |r| r.get(0)).map_err(db_err)?;
        self.db
            .execute(
                "INSERT INTO artifact_versions (artifact_id, version, created_at, note, state) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, v, at, note, state],
            )
            .map_err(db_err)?;
        self.db.execute("DELETE FROM artifact_versions WHERE artifact_id = ?1 AND version <= ?2", params![id, v - 50]).map_err(db_err)?;
        Ok(v)
    }

    pub fn artifact_versions(&self, id: &str) -> Result<Vec<ArtifactVersion>> {
        let mut st = self.db.prepare("SELECT version, created_at, note FROM artifact_versions WHERE artifact_id = ?1 ORDER BY version DESC").map_err(db_err)?;
        let rows = st
            .query_map([id], |r| Ok(ArtifactVersion { artifact_id: id.to_string(), version: r.get(0)?, created_at: r.get(1)?, note: r.get(2)? }))
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    pub fn artifact_version_state(&self, id: &str, version: i64) -> Result<Option<Vec<u8>>> {
        self.db
            .query_row("SELECT state FROM artifact_versions WHERE artifact_id = ?1 AND version = ?2", params![id, version], |r| r.get(0))
            .optional()
            .map_err(db_err)
    }

    // ---- usage ----------------------------------------------------------------------------

    /// Tokens per month (local time), backend and model, newest month first.
    pub fn usage_by_month(&self, months: u32) -> Result<Vec<UsageRow>> {
        let mut st = self
            .db
            .prepare(
                "SELECT strftime('%Y-%m', m.created_at / 1000, 'unixepoch', 'localtime') AS month, COALESCE(c.backend_id, ''),
                        COALESCE(json_extract(m.extra, '$.model'), ''), SUM(COALESCE(json_extract(m.extra, '$.usage.input_tokens'), 0)),
                        SUM(COALESCE(json_extract(m.extra, '$.usage.output_tokens'), 0)), COUNT(*)
                 FROM messages m LEFT JOIN conversations c ON c.id = m.conversation_id
                 WHERE m.role = 'assistant' AND json_extract(m.extra, '$.usage') IS NOT NULL
                 GROUP BY month, c.backend_id, json_extract(m.extra, '$.model') ORDER BY month DESC",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok(UsageRow {
                    month: r.get(0)?,
                    backend_id: r.get(1)?,
                    model: r.get(2)?,
                    input_tokens: r.get(3)?,
                    output_tokens: r.get(4)?,
                    answers: r.get(5)?,
                })
            })
            .map_err(db_err)?;
        let mut out: Vec<UsageRow> = rows.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut months_seen: Vec<String> = out.iter().map(|r| r.month.clone()).collect();
        months_seen.dedup();
        let keep: Vec<String> = months_seen.into_iter().take(months as usize).collect();
        out.retain(|r| keep.contains(&r.month));
        Ok(out)
    }

    // ---- action log (#23) -----------------------------------------------------------------

    /// Add an entry; the log keeps the newest [`MAX_LOG`] entries.
    pub fn log_action(&self, e: &LogEntry) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO action_log (at, conversation_id, kind, name, detail, reason, outcome, input_tokens, output_tokens, usd)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![e.at, e.conversation_id, e.kind, e.name, e.detail, e.reason, e.outcome, e.input_tokens, e.output_tokens, e.usd],
            )
            .map_err(db_err)?;
        self.db.execute("DELETE FROM action_log WHERE seq <= (SELECT MAX(seq) FROM action_log) - ?1", [MAX_LOG]).map_err(db_err)?;
        Ok(())
    }

    /// The newest entries first, optionally of one chat.
    pub fn action_log(&self, limit: usize, conversation_id: Option<&str>) -> Result<Vec<LogEntry>> {
        let mut st = self
            .db
            .prepare(
                "SELECT at, conversation_id, kind, name, detail, reason, outcome, input_tokens, output_tokens, usd FROM action_log
                 WHERE ?1 IS NULL OR conversation_id = ?1 ORDER BY seq DESC LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![conversation_id, limit.min(MAX_LOG as usize) as i64], |r| {
                Ok(LogEntry {
                    at: r.get(0)?,
                    conversation_id: r.get(1)?,
                    kind: r.get(2)?,
                    name: r.get(3)?,
                    detail: r.get(4)?,
                    reason: r.get(5)?,
                    outcome: r.get(6)?,
                    input_tokens: r.get(7)?,
                    output_tokens: r.get(8)?,
                    usd: r.get(9)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    // ---- claims (#24: one approval, one result) -------------------------------------------

    /// Claim an action before it runs. `false` when it was claimed before (by a double click,
    /// another window or an earlier run): then it must not run again.
    pub fn claim(&self, id: &str, conversation_id: &str, tool: &str, args_hash: &str, at: i64) -> Result<bool> {
        let n = self
            .db
            .execute(
                "INSERT OR IGNORE INTO tool_claims (id, conversation_id, tool, args_hash, state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?5)",
                params![id, conversation_id, tool, args_hash, at],
            )
            .map_err(db_err)?;
        Ok(n == 1)
    }

    /// The end of a claimed action: `done`, `failed` (it never reached the connector) or
    /// `uncertain` (it may have run).
    pub fn finish_claim(&self, id: &str, state: &str, at: i64) -> Result<()> {
        self.db.execute("UPDATE tool_claims SET state = ?2, updated_at = ?3 WHERE id = ?1", params![id, state, at]).map_err(db_err)?;
        Ok(())
    }

    /// Whether the same action (chat, tool, arguments) ended uncertain before.
    pub fn uncertain_claim(&self, conversation_id: &str, tool: &str, args_hash: &str) -> Result<bool> {
        self.db
            .prepare("SELECT 1 FROM tool_claims WHERE conversation_id = ?1 AND tool = ?2 AND args_hash = ?3 AND state = 'uncertain'")
            .and_then(|mut st| st.exists(params![conversation_id, tool, args_hash]))
            .map_err(db_err)
    }

    pub fn claim_state(&self, id: &str) -> Result<Option<String>> {
        self.db.query_row("SELECT state FROM tool_claims WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(db_err)
    }

    /// After a restart: everything still running may have run. It becomes uncertain and is
    /// returned (conversation, tool) so the person can check; nothing runs again.
    pub fn settle_running_claims(&self, at: i64) -> Result<Vec<(String, String)>> {
        let mut st = self.db.prepare("SELECT conversation_id, tool FROM tool_claims WHERE state = 'running'").map_err(db_err)?;
        let rows: Vec<(String, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(db_err)?.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        self.db.execute("UPDATE tool_claims SET state = 'uncertain', updated_at = ?1 WHERE state = 'running'", [at]).map_err(db_err)?;
        // Keep the table small: claims older than 90 days have done their job.
        self.db.execute("DELETE FROM tool_claims WHERE updated_at < ?1", [at - 90 * 24 * 3600 * 1000]).map_err(db_err)?;
        Ok(rows)
    }

    // ---- settings -------------------------------------------------------------------------

    pub fn setting(&self, key: &str) -> Result<Option<Value>> {
        let v: Option<String> = self.db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional().map_err(db_err)?;
        Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        self.db
            .execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value.to_string()])
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.db.execute("DELETE FROM settings WHERE key = ?1", params![key]).map_err(db_err)?;
        Ok(())
    }
}

/// A chunk of project knowledge found for a request: file, position, text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeHit {
    pub file_id: String,
    pub idx: i64,
    pub text: String,
}

impl Store {
    // ---- project knowledge (#32) ---------------------------------------------------------

    /// Add a project file with its extracted text and the chunks the search finds.
    pub fn add_project_file(&self, f: &ProjectFile, text: &str, chunks: &[String]) -> Result<()> {
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute(
            "INSERT OR REPLACE INTO project_files (id, project_id, name, mime, size, chars, path, text, added_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![f.id, f.project_id, f.name, f.mime, f.size as i64, f.chars as i64, f.path, text, f.added_at],
        )
        .map_err(db_err)?;
        tx.execute("DELETE FROM project_chunks WHERE file_id = ?1", [&f.id]).map_err(db_err)?;
        for (i, c) in chunks.iter().enumerate() {
            tx.execute("INSERT INTO project_chunks (text, file_id, project_id, idx) VALUES (?1, ?2, ?3, ?4)", params![c, f.id, f.project_id, i as i64])
                .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    /// The files of every project (without their text), oldest first.
    pub fn project_files(&self) -> Result<Vec<ProjectFile>> {
        let mut st =
            self.db.prepare("SELECT id, project_id, name, mime, size, chars, path, added_at FROM project_files ORDER BY added_at, name").map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok(ProjectFile {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    name: r.get(2)?,
                    mime: r.get(3)?,
                    size: r.get::<_, i64>(4)?.max(0) as u64,
                    chars: r.get::<_, i64>(5)?.max(0) as u64,
                    path: r.get(6)?,
                    added_at: r.get(7)?,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    /// The whole extracted text of a project file.
    pub fn project_file_text(&self, id: &str) -> Result<Option<String>> {
        self.db.query_row("SELECT text FROM project_files WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(db_err)
    }

    pub fn delete_project_file(&self, id: &str) -> Result<bool> {
        self.db.execute("DELETE FROM project_chunks WHERE file_id = ?1", [id]).map_err(db_err)?;
        Ok(self.db.execute("DELETE FROM project_files WHERE id = ?1", [id]).map_err(db_err)? > 0)
    }

    /// Chunks of a project's files where any of the words occurs, best first.
    pub fn search_knowledge(&self, project_id: &str, words: &[String], limit: usize) -> Result<Vec<KnowledgeHit>> {
        let terms: Vec<String> = words
            .iter()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
            .filter(|w| w.chars().count() >= 3)
            .take(16)
            .map(|w| format!("\"{w}\"*"))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self
            .db
            .prepare("SELECT file_id, idx, text FROM project_chunks WHERE project_chunks MATCH ?1 AND project_id = ?2 ORDER BY rank LIMIT ?3")
            .map_err(db_err)?;
        let rows = st
            .query_map(params![terms.join(" OR "), project_id, limit as i64], |r| Ok(KnowledgeHit { file_id: r.get(0)?, idx: r.get(1)?, text: r.get(2)? }))
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

impl Store {
    /// Full-text search where any of the words may match (for recall), best first.
    pub fn search_any(&self, words: &[String], exclude_conversation: Option<&str>, limit: usize) -> Result<Vec<SearchHit>> {
        let terms: Vec<String> = words
            .iter()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
            .filter(|w| w.chars().count() >= 3)
            .take(8)
            .map(|w| format!("\"{w}\"*"))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self
            .db
            .prepare(
                "SELECT f.conversation_id, f.message_id, COALESCE(c.title, ''), snippet(messages_fts, 0, '«', '»', '…', 14), COALESCE(c.backend_id, '')
                 FROM messages_fts f LEFT JOIN conversations c ON c.id = f.conversation_id
                 WHERE messages_fts MATCH ?1 AND f.conversation_id != ?2 ORDER BY rank LIMIT ?3",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![terms.join(" OR "), exclude_conversation.unwrap_or(""), limit as i64], |r| {
                Ok(SearchHit { conversation_id: r.get(0)?, message_id: r.get(1)?, title: r.get(2)?, snippet: r.get(3)?, backend_id: r.get(4)? })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

impl Store {
    /// The owner's messages since a time, newest first: (conversation, title, text, at).
    pub fn user_messages_since(&self, since: i64, limit: usize) -> Result<Vec<(String, String, String, i64)>> {
        let mut st = self
            .db
            .prepare(
                "SELECT m.conversation_id, COALESCE(c.title, ''), m.content, m.created_at FROM messages m LEFT JOIN conversations c ON c.id = m.conversation_id
                 WHERE m.role = 'user' AND m.active = 1 AND m.created_at >= ?1 ORDER BY m.created_at DESC LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st.query_map(params![since, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

/// A message's place in the tree.
#[derive(Debug, Clone)]
struct Node {
    id: String,
    parent: String,
    picked: i64,
    seq: i64,
}

/// Which version a message is among its siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    /// From 1.
    pub index: usize,
    pub count: usize,
}

/// The shown child of `parent`: the one picked last, the newest on a tie.
fn shown_child<'a>(nodes: &[&'a Node], parent: &str) -> Option<(usize, &'a Node)> {
    nodes.iter().copied().enumerate().filter(|(_, n)| n.parent == parent).max_by_key(|(_, n)| (n.picked, n.seq))
}

/// Indices (into `nodes`) of the shown branch from the first message, up to `head` when given.
fn active_path(nodes: &[&Node], head: Option<&str>) -> Vec<usize> {
    let mut path = Vec::new();
    if head == Some("") {
        return path;
    }
    let mut cur = String::new();
    // At most one step per message: a damaged tree with a cycle still ends.
    while path.len() < nodes.len() {
        let Some((i, n)) = shown_child(nodes, &cur) else { break };
        path.push(i);
        if head == Some(n.id.as_str()) {
            break;
        }
        cur = n.id.clone();
    }
    path
}

/// The end of the shown branch below (and including) `id`.
fn descend(nodes: &[&Node], id: &str) -> String {
    let mut cur = id.to_string();
    for _ in 0..nodes.len() {
        match shown_child(nodes, &cur) {
            Some((_, n)) => cur = n.id.clone(),
            None => break,
        }
    }
    cur
}

/// Turn what a person types into an FTS5 query: every word quoted, as a prefix, all required.
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> =
        q.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).take(12).map(|t| format!("\"{}\"*", t.replace('"', ""))).collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiscraft_core::{Attachment, BackendKind, DISK_FULL, ErrorKind};

    #[test]
    fn sqlite_full_is_named_as_a_full_disk() {
        let full = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), Some("database or disk is full".into()));
        let e = db_err(full);
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(e.message.starts_with(DISK_FULL), "{}", e.message);
        assert!(e.message.contains("database or disk is full"), "details kept: {}", e.message);
        let busy = db_err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None));
        assert!(!busy.message.contains(DISK_FULL), "{}", busy.message);
    }

    /// A real SQLITE_FULL from the store: the database may not grow past a few pages.
    #[test]
    fn a_store_that_cannot_grow_says_the_disk_is_full() {
        let st = Store::open_in_memory().unwrap();
        let pages: i64 = st.db.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap();
        st.db.execute_batch(&format!("PRAGMA max_page_count = {}", pages + 1)).unwrap();
        let big = Value::String("x".repeat(256 * 1024));
        let e = st.set_setting("gross", &big).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
        assert!(e.message.starts_with(DISK_FULL), "{}", e.message);
    }

    fn conv(id: &str, updated: i64) -> Conversation {
        Conversation {
            id: id.into(),
            title: format!("Chat {id}"),
            backend_id: "b".into(),
            model_id: "m".into(),
            project_id: None,
            created_at: 0,
            updated_at: updated,
            pinned: false,
            sources: Default::default(),
            persona: None,
            kind: Default::default(),
            remote_id: None,
            temporary: false,
        }
    }

    fn msg(conv: &str, id: &str, role: Role, text: &str) -> Message {
        Message {
            id: id.into(),
            conversation_id: conv.into(),
            role,
            content: text.into(),
            created_at: 1,
            model: None,
            attachments: vec![],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
            thinking: String::new(),
            citations: Vec::new(),
        }
    }

    /// A database from schema 2 (no `extra` column) opens, keeps its chats and stores personas.
    #[test]
    fn migrates_schema_2_and_keeps_the_persona() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE conversations (id TEXT PRIMARY KEY, title TEXT NOT NULL, backend_id TEXT NOT NULL, model_id TEXT NOT NULL,
               project_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, sources TEXT NOT NULL DEFAULT '{}');
             INSERT INTO conversations VALUES ('old', 'Alt', 'b', 'm', NULL, 0, 5, 0, '{}');
             PRAGMA user_version = 2;",
        )
        .unwrap();
        let s = Store::init(db).unwrap();
        let old = s.conversation("old").unwrap().unwrap();
        assert_eq!((old.title.as_str(), old.persona.clone()), ("Alt", None));
        let mut c = conv("p", 9);
        c.persona = Some(aiscraft_core::Persona { kind: aiscraft_core::PersonaKind::Character, id: "c1".into(), name: "Goethe".into() });
        s.save_conversation(&c).unwrap();
        assert_eq!(s.conversation("p").unwrap().unwrap().persona, c.persona);
        let v: i64 = s.db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn conversations_messages_search_and_truncate() {
        let s = Store::open_in_memory().unwrap();
        s.save_conversation(&conv("a", 1)).unwrap();
        let mut c = conv("b", 2);
        s.save_conversation(&c).unwrap();
        c.pinned = true;
        c.updated_at = 0;
        s.save_conversation(&c).unwrap();
        assert_eq!(s.conversations(10).unwrap().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);

        s.save_message(&msg("a", "1", Role::User, "Erkläre die Photosynthese für Klasse 7")).unwrap();
        let mut answer = msg("a", "2", Role::Assistant, "");
        s.save_message(&answer).unwrap();
        answer.content = "Pflanzen wandeln Licht in Zucker um.".into();
        answer.attachments.push(Attachment { name: "x.png".into(), data_base64: Some("big".into()), ..Default::default() });
        s.save_message(&answer).unwrap();
        let ms = s.messages("a").unwrap();
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[1].content, "Pflanzen wandeln Licht in Zucker um.");
        assert_eq!(ms[1].attachments[0].data_base64, None, "image bytes are not kept in the database");

        let hits = s.search("photosynth klasse", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains('«'));
        assert_eq!(hits[0].backend_id, conv("a", 1).backend_id, "hits name their chat's service");
        let any = s.search_any(&["photosynthese".into()], Some("b"), 3).unwrap();
        assert_eq!(any[0].backend_id, conv("a", 1).backend_id);
        assert_eq!(s.search("zucker", 10).unwrap()[0].message_id, "2");
        assert!(s.search("\"); DROP TABLE x; --", 10).unwrap().is_empty());
        assert!(s.search("", 10).unwrap().is_empty());

        s.truncate_from("a", "2").unwrap();
        assert_eq!(s.messages("a").unwrap().len(), 1);
        assert!(s.search("zucker", 10).unwrap().is_empty());

        s.delete_conversation("a").unwrap();
        assert!(s.messages("a").unwrap().is_empty());
        assert!(s.search("photosynthese", 10).unwrap().is_empty());
    }

    fn ids(s: &Store, conv: &str) -> Vec<String> {
        s.messages(conv).unwrap().into_iter().map(|m| m.id).collect()
    }

    /// Edit and regenerate keep the old version; switching shows its branch again; search
    /// and the owner's words follow the shown branch.
    #[test]
    fn variants_after_edit_and_regenerate() {
        let s = Store::open_in_memory().unwrap();
        s.save_conversation(&conv("a", 1)).unwrap();
        s.save_message(&msg("a", "u1", Role::User, "Frage eins")).unwrap();
        s.save_message(&msg("a", "a1", Role::Assistant, "Antwort Apfel")).unwrap();
        s.save_message(&msg("a", "u2", Role::User, "Frage zwei")).unwrap();
        s.save_message(&msg("a", "a2", Role::Assistant, "Antwort Birne")).unwrap();
        assert!(s.variants("a").unwrap().is_empty());

        // Regenerate the last answer.
        s.fork_at("a", "a2").unwrap();
        assert_eq!(ids(&s, "a"), ["u1", "a1", "u2"]);
        assert!(s.search("birne", 10).unwrap().is_empty(), "the hidden version is not searched");
        s.save_message(&msg("a", "a2b", Role::Assistant, "Antwort Kirsche")).unwrap();
        assert_eq!(ids(&s, "a"), ["u1", "a1", "u2", "a2b"]);
        assert_eq!(s.variants("a").unwrap().get("a2b"), Some(&Variant { index: 2, count: 2 }));

        // Edit the first question: a new branch from the start.
        s.fork_at("a", "u1").unwrap();
        assert!(ids(&s, "a").is_empty());
        s.save_message(&msg("a", "u1b", Role::User, "Frage eins anders")).unwrap();
        s.save_message(&msg("a", "a1b", Role::Assistant, "Antwort Pflaume")).unwrap();
        assert_eq!(ids(&s, "a"), ["u1b", "a1b"]);
        assert_eq!(s.variants("a").unwrap().get("u1b"), Some(&Variant { index: 2, count: 2 }));
        assert_eq!(s.user_messages_since(0, 10).unwrap().len(), 1, "only the shown branch counts as the owner's words");

        // Back to version 1: its branch comes back as it was last shown (with Kirsche).
        assert_eq!(s.select_variant("a", "u1b", 1).unwrap(), 2);
        assert_eq!(ids(&s, "a"), ["u1", "a1", "u2", "a2b"]);
        assert_eq!(s.search("kirsche", 10).unwrap().len(), 1);
        assert!(s.search("pflaume", 10).unwrap().is_empty());
        s.select_variant("a", "a2b", 1).unwrap();
        assert_eq!(ids(&s, "a"), ["u1", "a1", "u2", "a2"]);
        assert_eq!(s.search("birne", 10).unwrap()[0].message_id, "a2");
        assert!(s.select_variant("a", "a2", 3).is_err());
        assert!(s.select_variant("a", "a2", 0).is_err());
        assert!(s.select_variant("a", "nope", 1).is_err());
        assert!(s.fork_at("a", "nope").is_err());

        // A new message goes after the shown branch.
        s.save_message(&msg("a", "u3", Role::User, "Frage drei")).unwrap();
        assert_eq!(ids(&s, "a").last().map(String::as_str), Some("u3"));

        // A failed edit puts the old branch back; after a saved message it does nothing.
        s.fork_at("a", "u3").unwrap();
        s.unfork("a", "u3").unwrap();
        assert_eq!(ids(&s, "a").last().map(String::as_str), Some("u3"));
        s.fork_at("a", "u3").unwrap();
        s.save_message(&msg("a", "u3b", Role::User, "Frage drei anders")).unwrap();
        s.unfork("a", "u3").unwrap();
        assert_eq!(ids(&s, "a").last().map(String::as_str), Some("u3b"));

        // Server ids keep the tree together.
        s.save_message(&msg("a", "tmp", Role::Assistant, "")).unwrap();
        s.rename_message("tmp", "srv").unwrap();
        s.save_message(&msg("a", "u4", Role::User, "Frage vier")).unwrap();
        assert_eq!(ids(&s, "a").iter().rev().take(2).collect::<Vec<_>>(), ["u4", "srv"]);

        // Deleting a run removes it on every branch.
        s.truncate_from("a", "u3b").unwrap();
        assert_eq!(ids(&s, "a"), ["u1", "a1", "u2", "a2"]);
        assert!(s.search("vier", 10).unwrap().is_empty());
        s.delete_conversation("a").unwrap();
        assert_eq!(s.stored_head("a").unwrap(), None);
    }

    /// A database from schema 3 (messages without a tree) opens; its chats become one branch
    /// and take new versions.
    #[test]
    fn migrates_schema_3_messages_into_one_branch() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE conversations (id TEXT PRIMARY KEY, title TEXT NOT NULL, backend_id TEXT NOT NULL, model_id TEXT NOT NULL,
               project_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0,
               sources TEXT NOT NULL DEFAULT '{}', extra TEXT NOT NULL DEFAULT '{}');
             CREATE TABLE messages (id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
               seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at INTEGER NOT NULL, extra TEXT NOT NULL);
             INSERT INTO conversations VALUES ('old', 'Alt', 'b', 'm', NULL, 0, 5, 0, '{}', '{}');
             INSERT INTO messages VALUES ('m1', 'old', 1, 'user', 'Hallo', 1, '{}'), ('m2', 'old', 2, 'assistant', 'Tag', 2, '{}'),
               ('m3', 'old', 3, 'user', 'Noch was', 3, '{}'), ('m4', 'old', 4, 'assistant', 'Gern', 4, '{}');
             PRAGMA user_version = 3;",
        )
        .unwrap();
        let s = Store::init(db).unwrap();
        assert_eq!(ids(&s, "old"), ["m1", "m2", "m3", "m4"]);
        assert!(s.variants("old").unwrap().is_empty());
        s.fork_at("old", "m3").unwrap();
        s.save_message(&msg("old", "m3b", Role::User, "Anders")).unwrap();
        assert_eq!(ids(&s, "old"), ["m1", "m2", "m3b"]);
        s.select_variant("old", "m3b", 1).unwrap();
        assert_eq!(ids(&s, "old"), ["m1", "m2", "m3", "m4"]);
        let v: i64 = s.db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn migrates_schema_1_and_keeps_sources() {
        let path = std::env::temp_dir().join(format!("aiscraft-migrate-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            // A version-1 database without the sources column.
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE conversations (id TEXT PRIMARY KEY, title TEXT NOT NULL, backend_id TEXT NOT NULL, model_id TEXT NOT NULL, project_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, pinned INTEGER NOT NULL DEFAULT 0); INSERT INTO conversations VALUES ('old','Alt','b','m',NULL,0,0,0); PRAGMA user_version = 1;").unwrap();
        }
        let s = Store::open(&path).unwrap();
        let old = s.conversation("old").unwrap().unwrap();
        assert_eq!(old.sources, aiscraft_core::Sources::default());
        let mut c = old.clone();
        c.sources.memory = false;
        c.sources.connectors = Some(vec!["k1".into()]);
        s.save_conversation(&c).unwrap();
        assert_eq!(s.conversation("old").unwrap().unwrap().sources, c.sources);
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn backends_projects_memory_settings() {
        let s = Store::open_in_memory().unwrap();
        let b =
            BackendConfig { id: "x".into(), name: "X".into(), kind: BackendKind::OpenAi, base_url: "http://l".into(), default_model: None, send_context: true };
        s.save_backend(&b).unwrap();
        s.save_backend(&BackendConfig { name: "Y".into(), ..b.clone() }).unwrap();
        assert_eq!(s.backends().unwrap()[0].name, "Y");

        s.save_project(&Project { id: "p".into(), name: "Mathe 7b".into(), instructions: "Du bist…".into(), created_at: 0 }).unwrap();
        let mut c = conv("c", 1);
        c.project_id = Some("p".into());
        s.save_conversation(&c).unwrap();
        s.delete_project("p").unwrap();
        assert_eq!(s.conversation("c").unwrap().unwrap().project_id, None);

        let mut n = MemoryNote {
            id: "n".into(),
            area: MemoryArea::About,
            text: "Lehrerin".into(),
            source_message_id: Some("1".into()),
            source_quote: "merke dir: Lehrerin".into(),
            created_at: 0,
            archived: false,
        };
        s.save_memory(&n).unwrap();
        n.archived = true;
        s.save_memory(&n).unwrap();
        assert!(s.memory().unwrap()[0].archived);

        s.set_setting("theme", &serde_json::json!("dark")).unwrap();
        assert_eq!(s.setting("theme").unwrap(), Some(serde_json::json!("dark")));
        assert_eq!(s.setting("nope").unwrap(), None);
    }

    #[test]
    fn a_claim_is_taken_once_and_running_claims_become_uncertain() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.claim("approval:1", "c", "k__send", "h", 1).unwrap());
        assert!(!s.claim("approval:1", "c", "k__send", "h", 2).unwrap(), "a second claim fails");
        assert!(s.claim("approval:2", "c", "k__send", "h2", 3).unwrap());
        s.finish_claim("approval:2", "done", 4).unwrap();
        assert!(!s.uncertain_claim("c", "k__send", "h").unwrap());
        let settled = s.settle_running_claims(5).unwrap();
        assert_eq!(settled, [("c".to_string(), "k__send".to_string())]);
        assert_eq!(s.claim_state("approval:1").unwrap().as_deref(), Some("uncertain"));
        assert_eq!(s.claim_state("approval:2").unwrap().as_deref(), Some("done"));
        assert!(s.uncertain_claim("c", "k__send", "h").unwrap());
        assert!(s.settle_running_claims(6).unwrap().is_empty());
    }

    #[test]
    fn action_log_keeps_the_newest_entries() {
        let s = Store::open_in_memory().unwrap();
        let e = |i: i64, conv: &str| LogEntry {
            at: i,
            conversation_id: conv.into(),
            kind: "tool".into(),
            name: format!("Kalender · {i}"),
            detail: String::new(),
            reason: "Nur lesen".into(),
            outcome: "ok".into(),
            input_tokens: None,
            output_tokens: None,
            usd: None,
        };
        for i in 0..(MAX_LOG + 20) {
            s.log_action(&e(i, if i % 2 == 0 { "a" } else { "b" })).unwrap();
        }
        let all = s.action_log(usize::MAX, None).unwrap();
        assert_eq!(all.len() as i64, MAX_LOG, "capped");
        assert_eq!(all[0].at, MAX_LOG + 19, "newest first");
        let a = s.action_log(10, Some("a")).unwrap();
        assert_eq!(a.len(), 10);
        assert!(a.iter().all(|x| x.conversation_id == "a"));
    }

    /// #34: a temporary chat lives only in the database in memory: never listed, never in the
    /// file, never found; deleting it leaves nothing.
    #[test]
    fn temporary_chats_stay_out_of_the_file_and_the_search() {
        let dir = std::env::temp_dir().join(format!("aiscraft-store-temp-{}", std::process::id()));
        let path = dir.join("t.db");
        let s = Store::open(&path).unwrap();
        let t = Conversation { temporary: true, ..conv("t", 5) };
        s.save_conversation(&t).unwrap();
        s.save_message(&msg("t", "m1", Role::User, "Vertraulicher Elternbrief")).unwrap();
        s.save_message(&msg("t", "m2", Role::Assistant, "Entwurf")).unwrap();
        assert!(s.is_temporary("t"));
        assert_eq!(s.messages("t").unwrap().len(), 2, "readable while it is open");
        assert!(s.conversation("t").unwrap().unwrap().temporary);
        assert!(s.conversations(10).unwrap().is_empty(), "never listed");
        assert!(s.search("Elternbrief", 10).unwrap().is_empty() && s.search_any(&["elternbrief".into()], None, 10).unwrap().is_empty(), "never found");
        assert!(s.user_messages_since(0, 10).unwrap().is_empty(), "never tidied into notes");
        let on_disk: i64 = s.db.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(on_disk, 0, "nothing in the file");
        s.fork_at("t", "m2").unwrap();
        s.save_message(&msg("t", "m3", Role::Assistant, "Zweiter Entwurf")).unwrap();
        assert_eq!(s.variants("t").unwrap().len(), 1, "versions work as in any chat");
        s.delete_conversation("t").unwrap();
        assert!(!s.is_temporary("t") && s.messages("t").unwrap().is_empty() && s.conversation("t").unwrap().is_none());
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
