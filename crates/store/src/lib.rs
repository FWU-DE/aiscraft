//! Local storage (layer 1): one SQLite file with conversations, messages, projects, memory,
//! backends (without secrets) and settings, plus an FTS5 index over every message.

use std::path::Path;

use aiscraft_core::{Artifact, ArtifactKind, ArtifactVersion, BackendConfig, Conversation, Error, MemoryArea, MemoryNote, Message, Project, Result, Role};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SCHEMA_VERSION: i64 = 3;

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
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT NOT NULL, conversation_id TEXT, project_id TEXT,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, state BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact_versions (
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE, version INTEGER NOT NULL,
    created_at INTEGER NOT NULL, note TEXT NOT NULL, state BLOB NOT NULL, PRIMARY KEY (artifact_id, version)
);
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub conversation_id: String,
    pub message_id: String,
    pub title: String,
    /// Text around the match, with `«` `»` around matched words.
    pub snippet: String,
}

/// Conversation fields that live in one JSON column (added in schema 3).
#[derive(Default, Serialize, Deserialize)]
struct ConvExtra {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    persona: Option<aiscraft_core::Persona>,
}

pub struct Store {
    db: Connection,
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::storage(format!("Datenbank: {e}"))
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::storage(format!("Ordner {}: {e}", dir.display())))?;
        }
        Self::init(Connection::open(path).map_err(db_err)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;").map_err(db_err)?;
        let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(db_err)?;
        if version > SCHEMA_VERSION {
            return Err(Error::storage("Die Datenbank stammt aus einer neueren AIS.ChatCraft-Version"));
        }
        db.execute_batch(SCHEMA).map_err(db_err)?;
        // 1 → 2: per-chat sources.
        let has_sources: bool = db.prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'sources'").and_then(|mut st| st.exists([])).map_err(db_err)?;
        if !has_sources {
            db.execute_batch("ALTER TABLE conversations ADD COLUMN sources TEXT NOT NULL DEFAULT '{}'").map_err(db_err)?;
        }
        // 2 → 3: a JSON column for later fields (first: the persona a chat talks to).
        let has_extra: bool = db.prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'extra'").and_then(|mut st| st.exists([])).map_err(db_err)?;
        if !has_extra {
            db.execute_batch("ALTER TABLE conversations ADD COLUMN extra TEXT NOT NULL DEFAULT '{}'").map_err(db_err)?;
        }
        db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}")).map_err(db_err)?;
        Ok(Self { db })
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
        let sql = format!("SELECT {} FROM conversations WHERE id = ?1", Self::CONV_COLS);
        self.db.query_row(&sql, [id], Self::conv_row).optional().map_err(db_err)
    }

    pub fn save_conversation(&self, c: &Conversation) -> Result<()> {
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
                    serde_json::to_string(&ConvExtra { persona: c.persona.clone() })?
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute("DELETE FROM messages_fts WHERE conversation_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM messages WHERE conversation_id = ?1", [id]).map_err(db_err)?;
        tx.execute("DELETE FROM conversations WHERE id = ?1", [id]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    // ---- messages -------------------------------------------------------------------------

    pub fn messages(&self, conversation_id: &str) -> Result<Vec<Message>> {
        let mut st = self.db.prepare("SELECT id, role, content, created_at, extra FROM messages WHERE conversation_id = ?1 ORDER BY seq").map_err(db_err)?;
        let rows = st
            .query_map([conversation_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, String>(4)?))
            })
            .map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, role, content, created_at, extra) = row.map_err(db_err)?;
            let Some(role) = Role::parse(&role) else { continue };
            let extra: Extra = serde_json::from_str(&extra).unwrap_or_default();
            out.push(Message {
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
            });
        }
        Ok(out)
    }

    /// Insert or replace a message (keeps its position) and its search entry.
    pub fn save_message(&self, m: &Message) -> Result<()> {
        let extra = Extra {
            model: m.model.clone(),
            attachments: m.attachments.iter().map(|a| aiscraft_core::Attachment { data_base64: None, ..a.clone() }).collect(),
            activity: m.activity.clone(),
            error: m.error.clone(),
            usage: m.usage,
            tool_calls: m.tool_calls.clone(),
            tool_result: m.tool_result.clone(),
        };
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE((SELECT seq FROM messages WHERE id = ?1), (SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE conversation_id = ?2))",
                params![m.id, m.conversation_id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        tx.execute(
            "INSERT INTO messages (id, conversation_id, seq, role, content, created_at, extra) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET content = excluded.content, extra = excluded.extra",
            params![m.id, m.conversation_id, seq, m.role.as_str(), m.content, m.created_at, serde_json::to_string(&extra)?],
        )
        .map_err(db_err)?;
        tx.execute("DELETE FROM messages_fts WHERE message_id = ?1", [&m.id]).map_err(db_err)?;
        if !m.content.trim().is_empty() {
            tx.execute("INSERT INTO messages_fts (content, message_id, conversation_id) VALUES (?1, ?2, ?3)", params![m.content, m.id, m.conversation_id])
                .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)
    }

    /// Give a message the id the server assigned to it.
    pub fn rename_message(&self, old: &str, new: &str) -> Result<()> {
        if old == new {
            return Ok(());
        }
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        tx.execute("UPDATE messages SET id = ?2 WHERE id = ?1", params![old, new]).map_err(db_err)?;
        tx.execute("UPDATE messages_fts SET message_id = ?2 WHERE message_id = ?1", params![old, new]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// Delete a message and every message after it (for regenerate and edit).
    pub fn truncate_from(&self, conversation_id: &str, message_id: &str) -> Result<()> {
        let tx = self.db.unchecked_transaction().map_err(db_err)?;
        let Some(seq): Option<i64> = tx.query_row("SELECT seq FROM messages WHERE id = ?1", [message_id], |r| r.get(0)).optional().map_err(db_err)? else {
            return Ok(());
        };
        tx.execute(
            "DELETE FROM messages_fts WHERE message_id IN (SELECT id FROM messages WHERE conversation_id = ?1 AND seq >= ?2)",
            params![conversation_id, seq],
        )
        .map_err(db_err)?;
        tx.execute("DELETE FROM messages WHERE conversation_id = ?1 AND seq >= ?2", params![conversation_id, seq]).map_err(db_err)?;
        tx.commit().map_err(db_err)
    }

    /// Full-text search over every message. Each word matches as a prefix; all must occur.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(fts) = fts_query(query) else { return Ok(Vec::new()) };
        let mut st = self
            .db
            .prepare(
                "SELECT f.conversation_id, f.message_id, COALESCE(c.title, ''), snippet(messages_fts, 0, '«', '»', '…', 12)
                 FROM messages_fts f LEFT JOIN conversations c ON c.id = f.conversation_id
                 WHERE messages_fts MATCH ?1 ORDER BY rank LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![fts, limit as i64], |r| Ok(SearchHit { conversation_id: r.get(0)?, message_id: r.get(1)?, title: r.get(2)?, snippet: r.get(3)? }))
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

    /// Deleting a project keeps its chats; they just leave the project.
    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.db.execute("UPDATE conversations SET project_id = NULL WHERE project_id = ?1", [id]).map_err(db_err)?;
        self.db.execute("DELETE FROM projects WHERE id = ?1", [id]).map_err(db_err)?;
        Ok(())
    }

    // ---- memory ---------------------------------------------------------------------------

    pub fn memory(&self) -> Result<Vec<MemoryNote>> {
        let mut st = self
            .db
            .prepare("SELECT id, area, text, source_message_id, source_quote, created_at, archived FROM memory ORDER BY created_at")
            .map_err(db_err)?;
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
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })
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
        let v: i64 = self.db.query_row("SELECT COALESCE(MAX(version), 0) + 1 FROM artifact_versions WHERE artifact_id = ?1", [id], |r| r.get(0)).map_err(db_err)?;
        self.db
            .execute("INSERT INTO artifact_versions (artifact_id, version, created_at, note, state) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, v, at, note, state])
            .map_err(db_err)?;
        self.db.execute("DELETE FROM artifact_versions WHERE artifact_id = ?1 AND version <= ?2", params![id, v - 50]).map_err(db_err)?;
        Ok(v)
    }

    pub fn artifact_versions(&self, id: &str) -> Result<Vec<ArtifactVersion>> {
        let mut st = self.db.prepare("SELECT version, created_at, note FROM artifact_versions WHERE artifact_id = ?1 ORDER BY version DESC").map_err(db_err)?;
        let rows = st.query_map([id], |r| Ok(ArtifactVersion { artifact_id: id.to_string(), version: r.get(0)?, created_at: r.get(1)?, note: r.get(2)? })).map_err(db_err)?;
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
            .query_map([], |r| Ok(UsageRow { month: r.get(0)?, backend_id: r.get(1)?, model: r.get(2)?, input_tokens: r.get(3)?, output_tokens: r.get(4)?, answers: r.get(5)? }))
            .map_err(db_err)?;
        let mut out: Vec<UsageRow> = rows.collect::<rusqlite::Result<_>>().map_err(db_err)?;
        let mut months_seen: Vec<String> = out.iter().map(|r| r.month.clone()).collect();
        months_seen.dedup();
        let keep: Vec<String> = months_seen.into_iter().take(months as usize).collect();
        out.retain(|r| keep.contains(&r.month));
        Ok(out)
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
}

impl Store {
    /// Full-text search where any of the words may match (for recall), best first.
    pub fn search_any(&self, words: &[String], exclude_conversation: Option<&str>, limit: usize) -> Result<Vec<SearchHit>> {
        let terms: Vec<String> = words.iter().map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>()).filter(|w| w.chars().count() >= 3).take(8).map(|w| format!("\"{w}\"*")).collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut st = self
            .db
            .prepare(
                "SELECT f.conversation_id, f.message_id, COALESCE(c.title, ''), snippet(messages_fts, 0, '«', '»', '…', 14)
                 FROM messages_fts f LEFT JOIN conversations c ON c.id = f.conversation_id
                 WHERE messages_fts MATCH ?1 AND f.conversation_id != ?2 ORDER BY rank LIMIT ?3",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![terms.join(" OR "), exclude_conversation.unwrap_or(""), limit as i64], |r| {
                Ok(SearchHit { conversation_id: r.get(0)?, message_id: r.get(1)?, title: r.get(2)?, snippet: r.get(3)? })
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
                 WHERE m.role = 'user' AND m.created_at >= ?1 ORDER BY m.created_at DESC LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = st.query_map(params![since, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

/// Turn what a person types into an FTS5 query: every word quoted, as a prefix, all required.
fn fts_query(q: &str) -> Option<String> {
    let terms: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(12)
        .map(|t| format!("\"{}\"*", t.replace('"', "")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiscraft_core::{Attachment, BackendKind};

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
            usage: None, tool_calls: vec![], tool_result: None,
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
        let b = BackendConfig { id: "x".into(), name: "X".into(), kind: BackendKind::OpenAi, base_url: "http://l".into(), default_model: None, send_context: true };
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
}
