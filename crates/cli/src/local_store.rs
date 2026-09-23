//! The local SQLite store shared by the desktop CLI and the embedded phone core: account
//! metadata, sync bookkeeping, messages, and the relay outbox.

use std::path::Path;
use std::sync::Mutex;

use anyhow::Context;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;

use crate::app::{OutboxItem, SentJob, Slot, State};
use crate::model::{Author, Body, LiveTurn, Message};

pub struct LocalStore {
    connection: Mutex<Connection>,
}

pub struct Upsert {
    pub previous: Option<Message>,
    pub changed: bool,
}

pub struct MessageSearchHit {
    pub chat_id: String,
    pub message_id: String,
    pub snippet: String,
    pub author: Author,
    pub created_at: f64,
}

impl LocalStore {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection =
            Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA journal_size_limit = 16777216;
             CREATE TABLE IF NOT EXISTS metadata (
                 id                       INTEGER PRIMARY KEY CHECK (id = 1),
                 auto_review_json         TEXT NOT NULL,
                 last_seq                 INTEGER NOT NULL,
                 machine_blob_hash        TEXT,
                 credentials_uploaded     INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS devices (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bots (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chats (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS routines (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL,
                 json     TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS group_deletes (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS blob_deletes (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS device_seen (
                 id      TEXT PRIMARY KEY NOT NULL,
                 seen_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS applied_blobs (
                 id       TEXT PRIMARY KEY NOT NULL,
                 position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS chat_history (
                 chat_id      TEXT PRIMARY KEY NOT NULL,
                 before_place INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id            TEXT PRIMARY KEY NOT NULL,
                 chat_id       TEXT NOT NULL,
                 position      INTEGER NOT NULL,
                 sort_at       REAL NOT NULL,
                 created_at    REAL NOT NULL,
                 author_kind   TEXT NOT NULL,
                 author_bot_id TEXT,
                 body_kind     TEXT NOT NULL,
                 is_complete   INTEGER NOT NULL,
                 text_nonempty INTEGER NOT NULL,
                 message_json  TEXT NOT NULL,
                 UNIQUE(chat_id, position)
             );
             CREATE INDEX IF NOT EXISTS messages_chat_position
                 ON messages(chat_id, position);
             CREATE INDEX IF NOT EXISTS messages_chat_order
                 ON messages(chat_id, sort_at, position);
             CREATE INDEX IF NOT EXISTS messages_author
                 ON messages(author_kind, author_bot_id, created_at);
             CREATE TABLE IF NOT EXISTS outbox (
                 position        INTEGER PRIMARY KEY AUTOINCREMENT,
                 id              TEXT UNIQUE NOT NULL,
                 kind            TEXT NOT NULL,
                 recipient       TEXT,
                 ciphertext      BLOB NOT NULL,
                 slot_name       TEXT,
                 slot_keep_first INTEGER NOT NULL DEFAULT 0,
                 group_name      TEXT
             );
             CREATE UNIQUE INDEX IF NOT EXISTS outbox_slot
                 ON outbox(slot_name) WHERE slot_name IS NOT NULL;
             CREATE TABLE IF NOT EXISTS device_turns (
                 id   TEXT PRIMARY KEY NOT NULL,
                 json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sent_jobs (
                 id         TEXT PRIMARY KEY NOT NULL,
                 chat_id    TEXT NOT NULL,
                 bot_id     TEXT NOT NULL,
                 routine_id TEXT,
                 runner_id  TEXT NOT NULL,
                 sent_at    REAL NOT NULL
             );
             PRAGMA user_version = 1;",
        )?;
        crate::config::set_private(path)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn load_state(&self) -> anyhow::Result<State> {
        let connection = self.connection.lock().unwrap();
        let metadata: Option<(String, i64, Option<String>, bool)> = connection
            .query_row(
                "SELECT auto_review_json, last_seq, machine_blob_hash, credentials_uploaded
                 FROM metadata WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let (auto_review, last_seq, machine_blob_hash, credentials_uploaded) = match metadata {
            Some((json, last_seq, machine_blob_hash, credentials_uploaded)) => (
                serde_json::from_str(&json).context("decoding Auto-review state")?,
                last_seq,
                machine_blob_hash,
                credentials_uploaded,
            ),
            None => (Default::default(), 0, None, false),
        };
        Ok(State {
            devices: load_json_table(&connection, "devices")?,
            bots: load_json_table(&connection, "bots")?,
            chats: load_json_table(&connection, "chats")?,
            routines: load_json_table(&connection, "routines")?,
            auto_review,
            last_seq,
            group_deletes: load_ordered_ids(&connection, "group_deletes")?,
            blob_deletes: load_ordered_ids(&connection, "blob_deletes")?,
            machine_blob_hash,
            credentials_uploaded,
            device_seen: load_device_seen(&connection)?,
            device_online: Default::default(),
            turns_online: Default::default(),
            device_turns: load_device_turns(&connection)?,
            applied_blob_ids: load_ordered_ids(&connection, "applied_blobs")?,
        })
    }

    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_state_tx(&tx, state)?;
        tx.commit()?;
        Ok(())
    }

    pub fn save_state_deleting_chats(
        &self,
        state: &State,
        chat_ids: &[String],
    ) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_state_tx(&tx, state)?;
        for chat_id in chat_ids {
            tx.execute("DELETE FROM messages WHERE chat_id = ?1", [chat_id])?;
            tx.execute("DELETE FROM chat_history WHERE chat_id = ?1", [chat_id])?;
            tx.execute(
                "DELETE FROM outbox WHERE group_name = ?1",
                [crate::model::relay_name(chat_id)],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert(&self, message: &Message) -> anyhow::Result<Upsert> {
        let json = serde_json::to_string(message)?;
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let existing: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT message_json, chat_id, position FROM messages WHERE id = ?1",
                [&message.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let previous = existing
            .as_ref()
            .map(|(stored, _, _)| serde_json::from_str(stored))
            .transpose()
            .context("decoding stored message")?;
        if existing
            .as_ref()
            .is_some_and(|(stored, _, _)| stored == &json)
        {
            tx.commit()?;
            return Ok(Upsert {
                previous,
                changed: false,
            });
        }

        let position = match existing.as_ref() {
            Some((_, old_chat_id, position)) if old_chat_id == &message.chat_id => *position,
            _ => tx.query_row(
                "SELECT COALESCE(MAX(position), 0) + 1 FROM messages WHERE chat_id = ?1",
                [&message.chat_id],
                |row| row.get(0),
            )?,
        };
        let (author_kind, author_bot_id) = author_columns(&message.author);
        let body_kind = body_kind(&message.body);
        let text_nonempty =
            matches!(&message.body, Body::Text { text, .. } if !text.trim().is_empty());
        tx.execute(
            "INSERT INTO messages (
                 id, chat_id, position, sort_at, created_at, author_kind, author_bot_id,
                 body_kind, is_complete, text_nonempty, message_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 chat_id = excluded.chat_id,
                 position = excluded.position,
                 sort_at = excluded.sort_at,
                 created_at = excluded.created_at,
                 author_kind = excluded.author_kind,
                 author_bot_id = excluded.author_bot_id,
                 body_kind = excluded.body_kind,
                 is_complete = excluded.is_complete,
                 text_nonempty = excluded.text_nonempty,
                 message_json = excluded.message_json",
            params![
                message.id,
                message.chat_id,
                position,
                message.promoted_at.unwrap_or(message.created_at),
                message.created_at,
                author_kind,
                author_bot_id,
                body_kind,
                message.is_complete(),
                text_nonempty,
                json,
            ],
        )?;
        tx.commit()?;
        Ok(Upsert {
            previous,
            changed: true,
        })
    }

    /// Where this Device's copy of the chat begins in the relay's log, when the relay has
    /// older messages than the ones here. `None` for a chat that is here whole.
    pub fn history_before(&self, chat_id: &str) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row("SELECT before_place FROM chat_history WHERE chat_id = ?1", [chat_id], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    /// `Some(place)`: the relay has messages placed below it that are not here. `None`: the
    /// chat is here whole.
    pub fn set_history_before(&self, chat_id: &str, before: Option<i64>) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        match before {
            Some(place) => connection.execute(
                "INSERT INTO chat_history (chat_id, before_place) VALUES (?1, ?2)
                 ON CONFLICT(chat_id) DO UPDATE SET before_place = excluded.before_place",
                params![chat_id, place],
            )?,
            None => connection.execute("DELETE FROM chat_history WHERE chat_id = ?1", [chat_id])?,
        };
        Ok(())
    }

    /// Puts messages read backwards from the relay ahead of everything the chat has here.
    /// `newest_first` is the page in that order, so each lands before the one after it. One
    /// that is here already (a late edit arrived through the log and went to the end) moves
    /// to its place.
    pub fn insert_older(&self, newest_first: &[Message]) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for message in newest_first {
            let position: i64 = tx.query_row(
                "SELECT COALESCE(MIN(position), 1) - 1 FROM messages WHERE chat_id = ?1",
                [&message.chat_id],
                |row| row.get(0),
            )?;
            let (author_kind, author_bot_id) = author_columns(&message.author);
            let text_nonempty = matches!(&message.body, Body::Text { text, .. } if !text.trim().is_empty());
            tx.execute(
                "INSERT INTO messages (
                     id, chat_id, position, sort_at, created_at, author_kind, author_bot_id,
                     body_kind, is_complete, text_nonempty, message_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET chat_id = excluded.chat_id, position = excluded.position",
                params![
                    message.id,
                    message.chat_id,
                    position,
                    message.promoted_at.unwrap_or(message.created_at),
                    message.created_at,
                    author_kind,
                    author_bot_id,
                    body_kind(&message.body),
                    message.is_complete(),
                    text_nonempty,
                    serde_json::to_string(message)?,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn message(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    /// Every message in insertion order, for assertions over the durable store.
    #[cfg(test)]
    pub fn all(&self, chat_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT message_json FROM messages WHERE chat_id = ?1 ORDER BY position")?;
        let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    /// The model-visible ordering, with a valid compaction cursor taking precedence over the
    /// fallback limit. Without a valid cursor only the newest `limit` rows are materialized.
    pub fn context(
        &self,
        chat_id: &str,
        after: Option<&str>,
        limit: Option<usize>,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let cursor: Option<(f64, i64)> = match after {
            Some(id) => connection
                .query_row(
                    "SELECT sort_at, position FROM messages WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?,
            None => None,
        };
        let (mut messages, cursor_found) = if let Some((sort_at, position)) = cursor {
            let mut statement = connection.prepare(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND (sort_at > ?2 OR (sort_at = ?2 AND position > ?3))
                 ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map(params![chat_id, sort_at, position], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, true)
        } else if let Some(limit) = limit {
            let mut statement = connection.prepare(
                "SELECT message_json FROM (
                     SELECT message_json, sort_at, position FROM messages
                     WHERE chat_id = ?1 ORDER BY sort_at DESC, position DESC LIMIT ?2
                 ) ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map(params![chat_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?;
            let messages = collect_messages(rows)?;
            (messages, false)
        } else {
            let mut statement = connection.prepare(
                "SELECT message_json FROM messages WHERE chat_id = ?1 ORDER BY sort_at, position",
            )?;
            let rows = statement.query_map([chat_id], |row| row.get::<_, String>(0))?;
            let messages = collect_messages(rows)?;
            (messages, false)
        };
        // Defend against an old malformed row without changing the ordering of healthy rows.
        messages.retain(|message| message.chat_id == chat_id);
        Ok((messages, cursor_found))
    }

    /// A page in insertion order, oldest first. `before` is exclusive; an unknown cursor means
    /// the end of the chat, matching the previous in-memory API.
    pub fn page(
        &self,
        chat_id: &str,
        before: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<(Vec<Message>, bool)> {
        let connection = self.connection.lock().unwrap();
        let end = match before {
            Some(id) => connection
                .query_row(
                    "SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2",
                    params![chat_id, id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(i64::MAX),
            None => i64::MAX,
        };
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE chat_id = ?1 AND position < ?2
             ORDER BY position DESC LIMIT ?3",
        )?;
        let mut messages = collect_messages(
            statement.query_map(params![chat_id, end, (limit + 1) as i64], |row| {
                row.get::<_, String>(0)
            })?,
        )?;
        let has_more = messages.len() > limit;
        if has_more {
            messages.pop();
        }
        messages.reverse();
        Ok((messages, has_more))
    }

    pub fn messages_after(&self, chat_id: &str, message_id: &str) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let Some(position) = connection
            .query_row(
                "SELECT position FROM messages WHERE chat_id = ?1 AND id = ?2",
                params![chat_id, message_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(Vec::new());
        };
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages WHERE chat_id = ?1 AND position > ?2 ORDER BY position",
        )?;
        let rows =
            statement.query_map(params![chat_id, position], |row| row.get::<_, String>(0))?;
        collect_messages(rows)
    }

    pub fn count_after(&self, chat_id: &str, message_id: Option<&str>) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = match message_id {
            Some(id) => {
                let cursor: Option<(f64, i64)> = connection
                    .query_row(
                        "SELECT sort_at, position FROM messages WHERE chat_id = ?1 AND id = ?2",
                        params![chat_id, id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                match cursor {
                    Some((sort_at, position)) => connection.query_row(
                        "SELECT COUNT(*) FROM messages
                         WHERE chat_id = ?1 AND (sort_at > ?2 OR (sort_at = ?2 AND position > ?3))",
                        params![chat_id, sort_at, position],
                        |row| row.get::<_, i64>(0),
                    )?,
                    None => connection.query_row(
                        "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                        [chat_id],
                        |row| row.get::<_, i64>(0),
                    )?,
                }
            }
            None => connection.query_row(
                "SELECT COUNT(*) FROM messages WHERE chat_id = ?1",
                [chat_id],
                |row| row.get::<_, i64>(0),
            )?,
        };
        Ok(count as usize)
    }

    pub fn last_at_or_before(
        &self,
        chat_id: &str,
        timestamp_ms: u64,
    ) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id FROM messages WHERE chat_id = ?1 AND sort_at * 1000.0 <= ?2
                 ORDER BY sort_at DESC, position DESC LIMIT 1",
                params![chat_id, timestamp_ms as f64],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn last_user_at(&self) -> anyhow::Result<Option<i64>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT CAST(MAX(created_at) AS INTEGER) FROM messages WHERE author_kind = 'you'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn last_user_text(&self, chat_id: &str) -> anyhow::Result<Option<String>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'you' AND body_kind = 'text' AND text_nonempty = 1
                 ORDER BY position DESC LIMIT 1",
                [chat_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .map(|json| serde_json::from_str::<Message>(&json).context("decoding stored message"))
            .transpose()?
            .and_then(|message| match message.body {
                Body::Text { text, .. } if !text.trim().is_empty() => Some(text.trim().to_string()),
                _ => None,
            }))
    }

    pub fn last_bot_text(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<Option<Message>> {
        let connection = self.connection.lock().unwrap();
        let json: Option<String> = connection
            .query_row(
                "SELECT message_json FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2
                   AND body_kind = 'text' AND is_complete = 1
                 ORDER BY position DESC LIMIT 1",
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("decoding stored message"))
            .transpose()
    }

    pub fn text_messages(
        &self,
        chat_id: &str,
        since: Option<i64>,
        until: Option<i64>,
    ) -> anyhow::Result<Vec<Message>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE chat_id = ?1 AND body_kind = 'text' AND is_complete = 1
               AND (?2 IS NULL OR created_at >= ?2)
               AND (?3 IS NULL OR created_at <= ?3)
             ORDER BY position",
        )?;
        let rows = statement.query_map(params![chat_id, since, until], |row| {
            row.get::<_, String>(0)
        })?;
        collect_messages(rows)
    }

    pub fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<MessageSearchHit>> {
        let terms = search_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT message_json FROM messages
             WHERE body_kind IN ('text', 'handoff', 'notice', 'permission')
             ORDER BY created_at DESC, position DESC",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut hits = Vec::new();
        for row in rows {
            let message: Message = serde_json::from_str(&row?).context("decoding search result")?;
            let Some(text) = message_search_text(&message) else {
                continue;
            };
            if !search_matches(&text, &terms) {
                continue;
            }
            hits.push(MessageSearchHit {
                message_id: message.id,
                chat_id: message.chat_id,
                snippet: search_snippet(&text, &terms),
                author: message.author,
                created_at: message.created_at,
            });
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }

    pub fn heard_count(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let count = connection.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND is_complete = 1 AND body_kind IN ('text', 'handoff')
               AND NOT (author_kind = 'bot' AND author_bot_id = ?2)",
            params![chat_id, bot_id],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    pub fn new_count_since_bot_spoke(&self, chat_id: &str, bot_id: &str) -> anyhow::Result<usize> {
        let connection = self.connection.lock().unwrap();
        let last: Option<i64> = connection
            .query_row(
                "SELECT position FROM messages
                 WHERE chat_id = ?1 AND author_kind = 'bot' AND author_bot_id = ?2 AND body_kind = 'text'
                 ORDER BY position DESC LIMIT 1",
                params![chat_id, bot_id],
                |row| row.get(0),
            )
            .optional()?;
        let count = connection.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND position > ?2 AND is_complete = 1
               AND body_kind IN ('text', 'handoff')",
            params![chat_id, last.unwrap_or(0)],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as usize)
    }

    pub fn remove(&self, chat_id: &str, message_id: &str) -> anyhow::Result<bool> {
        let connection = self.connection.lock().unwrap();
        let removed = connection.execute(
            "DELETE FROM messages WHERE chat_id = ?1 AND id = ?2",
            params![chat_id, message_id],
        )? > 0;
        Ok(removed)
    }

    /// Remove rows whose chat metadata no longer exists. The chat table is authoritative;
    /// this catches interrupted or concurrent cleanup before the app serves a snapshot.
    pub fn retain_chats(&self, chat_ids: &[String]) -> anyhow::Result<()> {
        let valid: std::collections::HashSet<&str> = chat_ids.iter().map(String::as_str).collect();
        let valid_groups: std::collections::HashSet<String> = chat_ids
            .iter()
            .map(|id| crate::model::relay_name(id))
            .collect();
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        let stored_chats: Vec<String> = {
            let mut statement = tx.prepare("SELECT DISTINCT chat_id FROM messages")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for chat_id in stored_chats {
            if !valid.contains(chat_id.as_str()) {
                tx.execute("DELETE FROM messages WHERE chat_id = ?1", [&chat_id])?;
            }
        }
        let queued_groups: Vec<String> = {
            let mut statement =
                tx.prepare("SELECT DISTINCT group_name FROM outbox WHERE group_name IS NOT NULL")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for group in queued_groups {
            if !valid_groups.contains(&group) {
                tx.execute("DELETE FROM outbox WHERE group_name = ?1", [&group])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Queue a relay blob. A newer version of a slot replaces the waiting one without moving
    /// its queue position, so reconnecting still uploads messages in transcript order.
    #[cfg(test)]
    pub fn queue_outbox(&self, item: &OutboxItem) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        queue_outbox_tx(&tx, item)?;
        tx.commit()?;
        Ok(())
    }

    pub fn queue_outbox_with_state(&self, item: &OutboxItem, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        sync_json_table(
            &tx,
            "devices",
            state
                .devices
                .iter()
                .map(|device| (device.id.clone(), serde_json::to_string(device)))
                .collect::<Vec<_>>(),
        )?;
        sync_chats(&tx, state)?;
        append_applied_blob_tx(&tx, &item.id)?;
        queue_outbox_tx(&tx, item)?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_outbox_with_state(&self, id: &str, state: &State) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        save_metadata_tx(&tx, state)?;
        tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    pub fn first_outbox(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
                 FROM outbox ORDER BY position LIMIT 1",
                [],
                outbox_row,
            )
            .optional()
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub fn last_outbox(&self) -> anyhow::Result<Option<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        connection
            .query_row(
                "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
                 FROM outbox ORDER BY position DESC LIMIT 1",
                [],
                outbox_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// The turns another Device's latest machine blob lists; none drops its row.
    pub fn set_device_turns(&self, device_id: &str, turns: &[LiveTurn]) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        if turns.is_empty() {
            connection.execute("DELETE FROM device_turns WHERE id = ?1", [device_id])?;
        } else {
            connection.execute(
                "INSERT INTO device_turns (id, json) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                params![device_id, serde_json::to_string(turns)?],
            )?;
        }
        Ok(())
    }

    /// Keeps a job sealed to another Runner until its wait ends.
    pub fn insert_sent_job(&self, job: &SentJob) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute(
            "INSERT OR REPLACE INTO sent_jobs (id, chat_id, bot_id, routine_id, runner_id, sent_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![job.id, job.chat_id, job.bot_id, job.routine_id, job.runner_id, job.sent_at],
        )?;
        Ok(())
    }

    pub fn remove_sent_job(&self, id: &str) -> anyhow::Result<()> {
        let connection = self.connection.lock().unwrap();
        connection.execute("DELETE FROM sent_jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The jobs sealed to other Runners that this Device still waits on, oldest first.
    pub fn sent_jobs(&self) -> anyhow::Result<Vec<SentJob>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, chat_id, bot_id, routine_id, runner_id, sent_at FROM sent_jobs ORDER BY sent_at",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SentJob {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                bot_id: row.get(2)?,
                routine_id: row.get(3)?,
                runner_id: row.get(4)?,
                sent_at: row.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
    }

    #[cfg(test)]
    pub fn outbox(&self) -> anyhow::Result<Vec<OutboxItem>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection.prepare(
            "SELECT id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name
             FROM outbox ORDER BY position",
        )?;
        let rows = statement.query_map([], outbox_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        let mut connection = self.connection.lock().unwrap();
        let tx = connection.transaction()?;
        for table in [
            "metadata",
            "devices",
            "bots",
            "chats",
            "routines",
            "group_deletes",
            "blob_deletes",
            "device_seen",
            "applied_blobs",
            "messages",
            "chat_history",
            "outbox",
            "sent_jobs",
            "device_turns",
        ] {
            tx.execute(&format!("DELETE FROM {table}"), [])?;
        }
        tx.commit()?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }
}

fn queue_outbox_tx(tx: &Transaction<'_>, item: &OutboxItem) -> anyhow::Result<()> {
    let waiting: Option<i64> = match item.slot.as_ref() {
        Some(slot) => tx
            .query_row(
                "SELECT position FROM outbox WHERE slot_name = ?1",
                [&slot.name],
                |row| row.get(0),
            )
            .optional()?,
        None => None,
    };
    let slot_name = item.slot.as_ref().map(|slot| slot.name.as_str());
    let keep_first = item.slot.as_ref().is_some_and(|slot| slot.keep_first);
    match waiting {
        Some(position) => {
            tx.execute(
                "UPDATE outbox SET id = ?1, kind = ?2, recipient = ?3, ciphertext = ?4,
                     slot_name = ?5, slot_keep_first = ?6, group_name = ?7 WHERE position = ?8",
                params![
                    item.id,
                    item.kind,
                    item.recipient,
                    item.ciphertext,
                    slot_name,
                    keep_first,
                    item.group,
                    position
                ],
            )?;
        }
        None => {
            tx.execute(
                    "INSERT INTO outbox (id, kind, recipient, ciphertext, slot_name, slot_keep_first, group_name)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![item.id, item.kind, item.recipient, item.ciphertext, slot_name, keep_first, item.group],
                )?;
        }
    }
    Ok(())
}

fn save_state_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    save_metadata_tx(tx, state)?;
    sync_json_table(
        tx,
        "devices",
        state
            .devices
            .iter()
            .map(|device| (device.id.clone(), serde_json::to_string(device)))
            .collect::<Vec<_>>(),
    )?;
    sync_json_table(
        tx,
        "bots",
        state
            .bots
            .iter()
            .map(|bot| (bot.id.clone(), serde_json::to_string(bot)))
            .collect::<Vec<_>>(),
    )?;
    sync_chats(tx, state)?;
    sync_json_table(
        tx,
        "routines",
        state
            .routines
            .iter()
            .map(|routine| (routine.id.clone(), serde_json::to_string(routine)))
            .collect::<Vec<_>>(),
    )?;
    sync_ordered_ids(tx, "group_deletes", &state.group_deletes)?;
    sync_ordered_ids(tx, "blob_deletes", &state.blob_deletes)?;
    sync_ordered_ids(tx, "applied_blobs", &state.applied_blob_ids)?;
    sync_device_seen(tx, &state.device_seen)?;
    Ok(())
}

fn save_metadata_tx(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO metadata (id, auto_review_json, last_seq, machine_blob_hash, credentials_uploaded)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
             auto_review_json = excluded.auto_review_json,
             last_seq = excluded.last_seq,
             machine_blob_hash = excluded.machine_blob_hash,
             credentials_uploaded = excluded.credentials_uploaded",
        params![
            serde_json::to_string(&state.auto_review)?,
            state.last_seq,
            state.machine_blob_hash,
            state.credentials_uploaded,
        ],
    )?;
    Ok(())
}

fn sync_chats(tx: &Transaction<'_>, state: &State) -> anyhow::Result<()> {
    sync_json_table(
        tx,
        "chats",
        state
            .chats
            .iter()
            .map(|chat| (chat.meta.id.clone(), serde_json::to_string(chat)))
            .collect::<Vec<_>>(),
    )
}

fn message_search_text(message: &Message) -> Option<String> {
    let text = match &message.body {
        Body::Text { text, attachments } => {
            let names = attachments
                .iter()
                .map(|attachment| attachment.name.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            format!("{text} {names}")
        }
        Body::Handoff { reason, .. } => reason.clone(),
        Body::Notice { text, .. } => text.clone(),
        Body::Permission { summary, .. } => summary.clone(),
        Body::Tool { .. } => return None,
    };
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub(crate) fn search_terms(input: &str) -> Vec<String> {
    input
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub(crate) fn search_matches(text: &str, terms: &[String]) -> bool {
    let text = text.to_lowercase();
    terms.iter().all(|term| text.contains(term))
}

pub(crate) fn search_snippet(text: &str, terms: &[String]) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let matched = words
        .iter()
        .position(|word| {
            let word = word.to_lowercase();
            terms.iter().any(|term| word.contains(term))
        })
        .unwrap_or(0);
    let start = matched.saturating_sub(8);
    let end = (matched + 16).min(words.len());
    format!(
        "{}{}{}",
        if start > 0 { "… " } else { "" },
        words[start..end].join(" "),
        if end < words.len() { " …" } else { "" }
    )
}

fn append_applied_blob_tx(tx: &Transaction<'_>, id: &str) -> anyhow::Result<()> {
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), 0) + 1 FROM applied_blobs",
        [],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT INTO applied_blobs (id, position) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position",
        params![id, position],
    )?;
    tx.execute(
        "DELETE FROM applied_blobs
         WHERE id NOT IN (SELECT id FROM applied_blobs ORDER BY position DESC LIMIT 2000)",
        [],
    )?;
    Ok(())
}

fn sync_json_table(
    tx: &Transaction<'_>,
    table: &str,
    rows: Vec<(String, serde_json::Result<String>)>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, (i64, String)> = {
        let mut statement = tx.prepare(&format!("SELECT id, position, json FROM {table}"))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?))))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    let upsert = format!(
        "INSERT INTO {table} (id, position, json) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET position = excluded.position, json = excluded.json"
    );
    for (position, (id, json)) in rows.into_iter().enumerate() {
        let json = json?;
        let position = position as i64;
        if existing
            .get(&id)
            .is_none_or(|stored| stored.0 != position || stored.1 != json)
        {
            tx.execute(&upsert, params![id, position, json])?;
        }
        retained.insert(id);
    }
    delete_missing_from(tx, table, existing.keys(), &retained)
}

fn sync_ordered_ids(tx: &Transaction<'_>, table: &str, rows: &[String]) -> anyhow::Result<()> {
    let existing: Vec<(String, i64)> = {
        let mut statement = tx.prepare(&format!(
            "SELECT id, position FROM {table} ORDER BY position"
        ))?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let existing_ids: std::collections::HashSet<&str> =
        existing.iter().map(|(id, _)| id.as_str()).collect();
    let retained: std::collections::HashSet<String> = rows.iter().cloned().collect();
    delete_missing_from(tx, table, existing.iter().map(|(id, _)| id), &retained)?;
    let mut position = existing
        .iter()
        .map(|(_, position)| *position)
        .max()
        .unwrap_or(0);
    let insert = format!("INSERT INTO {table} (id, position) VALUES (?1, ?2)");
    for id in rows {
        if !existing_ids.contains(id.as_str()) {
            position += 1;
            tx.execute(&insert, params![id, position])?;
        }
    }
    Ok(())
}

fn sync_device_seen(
    tx: &Transaction<'_>,
    rows: &std::collections::HashMap<String, i64>,
) -> anyhow::Result<()> {
    let existing: std::collections::HashMap<String, i64> = {
        let mut statement = tx.prepare("SELECT id, seen_at FROM device_seen")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut retained = std::collections::HashSet::new();
    for (id, seen_at) in rows {
        if existing.get(id) != Some(seen_at) {
            tx.execute(
                "INSERT INTO device_seen (id, seen_at) VALUES (?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET seen_at = excluded.seen_at",
                params![id, seen_at],
            )?;
        }
        retained.insert(id.clone());
    }
    delete_missing_from(tx, "device_seen", existing.keys(), &retained)
}

fn delete_missing_from<'a>(
    tx: &Transaction<'_>,
    table: &str,
    existing: impl Iterator<Item = &'a String>,
    retained: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let delete = format!("DELETE FROM {table} WHERE id = ?1");
    for id in existing {
        if !retained.contains(id) {
            tx.execute(&delete, [id])?;
        }
    }
    Ok(())
}

fn load_json_table<T: DeserializeOwned>(
    connection: &Connection,
    table: &str,
) -> anyhow::Result<Vec<T>> {
    let values: Vec<String> = {
        let mut statement =
            connection.prepare(&format!("SELECT json FROM {table} ORDER BY position"))?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    values
        .into_iter()
        .map(|json| serde_json::from_str(&json).with_context(|| format!("decoding {table} row")))
        .collect()
}

fn load_ordered_ids(connection: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("SELECT id FROM {table} ORDER BY position"))?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn load_device_turns(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, Vec<LiveTurn>>> {
    let mut statement = connection.prepare("SELECT id, json FROM device_turns")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    rows.map(|row| {
        let (id, json) = row?;
        Ok((id, serde_json::from_str(&json).context("decoding a Device's turns")?))
    })
    .collect()
}

fn load_device_seen(
    connection: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut statement = connection.prepare("SELECT id, seen_at FROM device_seen")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<_>>().map_err(Into::into)
}

fn collect_messages(
    rows: impl Iterator<Item = rusqlite::Result<String>>,
) -> anyhow::Result<Vec<Message>> {
    rows.map(|row| {
        let json = row?;
        serde_json::from_str(&json).context("decoding stored message")
    })
    .collect()
}

fn author_columns(author: &Author) -> (&'static str, Option<&str>) {
    match author {
        Author::You => ("you", None),
        Author::Bot { bot_id } => ("bot", Some(bot_id)),
        Author::System => ("system", None),
    }
}

fn body_kind(body: &Body) -> &'static str {
    match body {
        Body::Text { .. } => "text",
        Body::Tool { .. } => "tool",
        Body::Handoff { .. } => "handoff",
        Body::Notice { .. } => "notice",
        Body::Permission { .. } => "permission",
    }
}

fn outbox_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxItem> {
    let slot_name: Option<String> = row.get(4)?;
    let keep_first = row.get::<_, i64>(5)? != 0;
    Ok(OutboxItem {
        id: row.get(0)?,
        kind: row.get(1)?,
        recipient: row.get(2)?,
        ciphertext: row.get(3)?,
        slot: slot_name.map(|name| Slot { name, keep_first }),
        group: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MessageState, SNAPSHOT_MESSAGES};

    struct Scratch(LocalStore, std::path::PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch() -> Scratch {
        let home = std::env::temp_dir().join(format!("lorca-transcript-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        Scratch(LocalStore::open(&home.join("lorca.sqlite3")).unwrap(), home)
    }

    fn message(id: &str, at: f64) -> Message {
        let mut message = Message::new("chat", Author::You, Body::text(id));
        message.id = id.into();
        message.created_at = at;
        message
    }

    #[test]
    fn older_pages_land_before_what_is_here() {
        let scratch = scratch();
        let store = &scratch.0;
        let said = |id: &str| message(id, 0.0);
        for id in ["m5", "m6"] {
            store.upsert(&said(id)).unwrap();
        }
        // An edit of an old message came through the log and went to the end.
        store.upsert(&said("m3")).unwrap();
        assert_eq!(store.history_before("chat").unwrap(), None);
        store.set_history_before("chat", Some(40)).unwrap();
        assert_eq!(store.history_before("chat").unwrap(), Some(40));

        store.insert_older(&[said("m4"), said("m3")]).unwrap();
        store.insert_older(&[said("m2"), said("m1")]).unwrap();
        store.set_history_before("chat", None).unwrap();
        let ids = |messages: Vec<Message>| messages.into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(store.all("chat").unwrap()), ["m1", "m2", "m3", "m4", "m5", "m6"]);
        let (page, more) = store.page("chat", Some("m5"), 2).unwrap();
        assert_eq!((ids(page), more), (vec!["m3".to_string(), "m4".into()], true));
        assert_eq!(store.history_before("chat").unwrap(), None);
    }

    #[test]
    fn upserts_keep_position_and_pages_are_stable() {
        let scratch = scratch();
        for i in 0..5 {
            scratch
                .0
                .upsert(&message(&format!("m{i}"), i as f64))
                .unwrap();
        }
        let (page, more) = scratch.0.page("chat", None, 2).unwrap();
        assert_eq!(
            page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m3", "m4"]
        );
        assert!(more);
        let mut changed = message("m3", 3.0);
        changed.state = MessageState::Streaming;
        scratch.0.upsert(&changed).unwrap();
        let (page, _) = scratch
            .0
            .page("chat", Some("m4"), SNAPSHOT_MESSAGES)
            .unwrap();
        assert_eq!(
            page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["m0", "m1", "m2", "m3"]
        );
    }

    #[test]
    fn context_orders_promoted_messages_and_honors_a_compaction_cursor() {
        let scratch = scratch();
        scratch.0.upsert(&message("first", 1.0)).unwrap();
        let mut promoted = message("steer", 2.0);
        promoted.promoted_at = Some(4.0);
        scratch.0.upsert(&promoted).unwrap();
        scratch.0.upsert(&message("settled", 3.0)).unwrap();
        let (messages, found) = scratch.0.context("chat", None, Some(10)).unwrap();
        assert!(!found);
        assert_eq!(
            messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "settled", "steer"]
        );
        let (messages, found) = scratch.0.context("chat", Some("settled"), Some(1)).unwrap();
        assert!(found);
        assert_eq!(
            messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["steer"]
        );
    }

    #[test]
    fn indexed_activity_queries_do_not_load_tool_payloads() {
        let scratch = scratch();
        let mut user = message("user", 1.0);
        user.body = Body::text("hello");
        let mut own = message("own", 2.0);
        own.author = Author::Bot {
            bot_id: "chef".into(),
        };
        own.body = Body::text("done");
        let mut other = message("other", 3.0);
        other.author = Author::Bot {
            bot_id: "scout".into(),
        };
        other.body = Body::text("found it");
        let mut handoff = message("handoff", 4.0);
        handoff.author = Author::Bot {
            bot_id: "scout".into(),
        };
        handoff.body = Body::Handoff {
            from: "scout".into(),
            to: "chef".into(),
            reason: "take over".into(),
        };
        let mut empty = message("empty", 5.0);
        empty.body = Body::text("  ");
        for row in [user, own, other, handoff, empty] {
            scratch.0.upsert(&row).unwrap();
        }

        assert_eq!(
            scratch.0.last_user_text("chat").unwrap().as_deref(),
            Some("hello")
        );
        assert_eq!(
            scratch.0.last_bot_text("chat", "chef").unwrap().unwrap().id,
            "own"
        );
        assert_eq!(scratch.0.heard_count("chat", "chef").unwrap(), 4);
        assert_eq!(
            scratch.0.new_count_since_bot_spoke("chat", "chef").unwrap(),
            3
        );
        assert_eq!(
            scratch
                .0
                .text_messages("chat", Some(2), Some(3))
                .unwrap()
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["own", "other"]
        );
    }

    #[test]
    fn plain_text_search_scans_visible_message_text() {
        let scratch = scratch();
        let mut deployed = message("deployed", 2.0);
        deployed.body = Body::text("Deployed the resume service successfully");
        scratch.0.upsert(&deployed).unwrap();

        let message_hits = scratch.0.search_messages("deploy resume", 10).unwrap();
        assert_eq!(message_hits[0].message_id, "deployed");
        assert!(message_hits[0].snippet.contains("Deployed"));
        assert!(scratch.0.search_messages("\" OR *", 10).unwrap().is_empty());
        let terms = search_terms("release infra");
        assert!(search_matches("Release Room Infrastructure", &terms));
        assert!(search_snippet("Release Room Infrastructure", &terms).contains("Release"));

        deployed.body = Body::text("Finished something else");
        scratch.0.upsert(&deployed).unwrap();
        assert!(scratch.0.search_messages("deploy", 10).unwrap().is_empty());
        scratch.0.remove("chat", "deployed").unwrap();
        assert!(scratch
            .0
            .search_messages("finished", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn queued_ciphertext_is_binary_and_survives_reopening() {
        let scratch = scratch();
        let ciphertext = vec![0, 255, 128, 13, 10, 34];
        let item = OutboxItem {
            id: "att-file".into(), kind: "file".into(), recipient: None,
            ciphertext: ciphertext.clone(), slot: None, group: Some("chat".into()),
        };
        scratch.0.queue_outbox(&item).unwrap();
        let connection = scratch.0.connection.lock().unwrap();
        let (kind, length): (String, usize) = connection.query_row(
            "SELECT typeof(ciphertext), length(ciphertext) FROM outbox WHERE id = 'att-file'",
            [], |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(kind, "blob");
        assert_eq!(length, ciphertext.len());
        drop(connection);
        let reopened = LocalStore::open(&scratch.1.join("lorca.sqlite3")).unwrap();
        let queued = reopened.first_outbox().unwrap().unwrap();
        assert_eq!(queued.ciphertext, ciphertext);
        assert_eq!(queued.group.as_deref(), Some("chat"));
    }

    #[test]
    fn a_waiting_slot_is_replaced_without_moving_in_the_outbox() {
        let scratch = scratch();
        let item = |id: &str, slot: Option<&str>| OutboxItem {
            id: id.into(),
            kind: "chat".into(),
            recipient: None,
            ciphertext: id.as_bytes().to_vec(),
            slot: slot.map(|name| Slot {
                name: name.into(),
                keep_first: true,
            }),
            group: Some("chat".into()),
        };
        scratch
            .0
            .queue_outbox(&item("first", Some("message")))
            .unwrap();
        scratch.0.queue_outbox(&item("second", None)).unwrap();
        scratch
            .0
            .queue_outbox(&item("latest", Some("message")))
            .unwrap();

        let outbox = scratch.0.outbox().unwrap();
        assert_eq!(
            outbox
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["latest", "second"]
        );
        assert_eq!(
            scratch.0.first_outbox().unwrap().unwrap().ciphertext,
            b"latest"
        );
    }

    #[test]
    fn metadata_prunes_orphaned_messages_and_grouped_uploads() {
        let scratch = scratch();
        let mut kept = message("kept", 1.0);
        kept.chat_id = "kept-chat".into();
        let mut orphan = message("orphan", 2.0);
        orphan.chat_id = "orphan-chat".into();
        scratch.0.upsert(&kept).unwrap();
        scratch.0.upsert(&orphan).unwrap();
        for (id, group) in [
            ("kept-upload", "kept-chat"),
            ("orphan-upload", "orphan-chat"),
        ] {
            scratch
                .0
                .queue_outbox(&OutboxItem {
                    id: id.into(),
                    kind: "chat".into(),
                    recipient: None,
                    ciphertext: Vec::new(),
                    slot: None,
                    group: Some(group.into()),
                })
                .unwrap();
        }

        scratch.0.retain_chats(&["kept-chat".into()]).unwrap();

        assert!(scratch.0.message("kept-chat", "kept").unwrap().is_some());
        assert!(scratch
            .0
            .message("orphan-chat", "orphan")
            .unwrap()
            .is_none());
        assert_eq!(
            scratch
                .0
                .outbox()
                .unwrap()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["kept-upload"]
        );
    }
}
