use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::Value;

pub struct MessageStore {
    conn: Connection,
}

fn row_to_json(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "uuid": row.get::<_, String>(0)?,
        "sender_uuid": row.get::<_, String>(1)?,
        "thread_uuid": row.get::<_, Option<String>>(2)?,
        "reply_to_uuid": row.get::<_, Option<String>>(3)?,
        "global_seq": row.get::<_, i64>(4)?,
        "key_epoch": row.get::<_, i64>(5)?,
        "ciphertext": row.get::<_, String>(6)?,
        "ts": row.get::<_, String>(7)?,
        "server_ts": row.get::<_, String>(8)?,
        "read": row.get::<_, i64>(9)?,
    }))
}

impl MessageStore {
    #[cfg(test)]
    pub fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                uuid TEXT PRIMARY KEY,
                sender_uuid TEXT NOT NULL,
                thread_uuid TEXT,
                reply_to_uuid TEXT,
                global_seq INTEGER NOT NULL,
                key_epoch INTEGER NOT NULL DEFAULT 0,
                ciphertext TEXT NOT NULL,
                client_ts TEXT NOT NULL,
                server_ts TEXT NOT NULL,
                read INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX idx_messages_global_seq ON messages(global_seq);
            CREATE INDEX idx_messages_thread ON messages(thread_uuid);
            CREATE TABLE peer_keys (
                user_uuid TEXT PRIMARY KEY,
                ed25519_public BLOB NOT NULL,
                x25519_public BLOB,
                fetched_at TEXT NOT NULL
            );",
        )
        .unwrap();
        MessageStore { conn }
    }

    pub fn open(chat_uuid: &str) -> Result<Self> {
        let dir = crate::config::chat_dir(chat_uuid)?;
        std::fs::create_dir_all(&dir)?;

        let db_path = dir.join("messages.db");
        let conn = Connection::open(&db_path)?;
        crate::storage::restrict_file_permissions(&db_path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS messages (
                uuid TEXT PRIMARY KEY,
                sender_uuid TEXT NOT NULL,
                thread_uuid TEXT,
                reply_to_uuid TEXT,
                global_seq INTEGER NOT NULL,
                key_epoch INTEGER NOT NULL DEFAULT 0,
                ciphertext TEXT NOT NULL,
                client_ts TEXT NOT NULL,
                server_ts TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_messages_global_seq ON messages(global_seq);
            CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_uuid);
            CREATE TABLE IF NOT EXISTS peer_keys (
                user_uuid TEXT PRIMARY KEY,
                ed25519_public BLOB NOT NULL,
                x25519_public BLOB,
                fetched_at TEXT NOT NULL
            );
        ",
        )?;

        // Migration: add `read` column if this is an existing DB without it.
        let _ = conn.execute(
            "ALTER TABLE messages ADD COLUMN read INTEGER NOT NULL DEFAULT 0",
            [],
        );
        // Migration: add `x25519_public` column if this is an existing DB without it.
        // NULL entries are treated as cache misses and re-fetched from the server.
        let _ = conn.execute("ALTER TABLE peer_keys ADD COLUMN x25519_public BLOB", []);

        Ok(MessageStore { conn })
    }

    pub fn insert_message(&self, msg: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO messages
             (uuid, sender_uuid, thread_uuid, reply_to_uuid, global_seq,
              key_epoch, ciphertext, client_ts, server_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                msg["uuid"].as_str(),
                msg["sender_uuid"].as_str(),
                msg["thread_uuid"].as_str(),
                msg["reply_to_uuid"].as_str(),
                msg["global_seq"].as_i64(),
                msg["key_epoch"].as_i64(),
                msg["ciphertext"].as_str(),
                msg["ts"].as_str().or(msg["client_ts"].as_str()),
                msg["server_ts"].as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn mark_read(&self, uuid: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE messages SET read = 1 WHERE uuid = ?1",
            rusqlite::params![uuid],
        )?;
        Ok(())
    }

    pub fn get_high_water(&self) -> Result<i64> {
        let hw: i64 = self.conn.query_row(
            "SELECT COALESCE(MAX(global_seq), 0) FROM messages",
            [],
            |row| row.get(0),
        )?;
        Ok(hw)
    }

    pub fn get_range(&self, from_seq: i64, to_seq: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, sender_uuid, thread_uuid, reply_to_uuid, global_seq,
                    key_epoch, ciphertext, client_ts, server_ts, read
             FROM messages
             WHERE global_seq >= ?1 AND global_seq <= ?2
             ORDER BY global_seq",
        )?;

        let rows = stmt.query_map(params![from_seq, to_seq], row_to_json)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Returns `(ed25519_public, x25519_public)` if both are cached, or `None` if either is missing.
    pub fn get_peer_keys(&self, user_uuid: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let result = self.conn.query_row(
            "SELECT ed25519_public, x25519_public FROM peer_keys WHERE user_uuid = ?1",
            rusqlite::params![user_uuid],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        );
        match result {
            Ok((ed25519, Some(x25519))) => Ok(Some((ed25519, x25519))),
            Ok((_, None)) => Ok(None), // incomplete cache entry — treat as miss
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn store_peer_keys(&self, user_uuid: &str, ed25519: &[u8], x25519: &[u8]) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT OR REPLACE INTO peer_keys (user_uuid, ed25519_public, x25519_public, fetched_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![user_uuid, ed25519, x25519, now],
        )?;
        Ok(())
    }

    /// Fetch the most recent top-level messages and their thread replies.
    pub fn get_recent(&self, limit: i64) -> Result<Vec<Value>> {
        // First get recent top-level messages
        let mut stmt = self.conn.prepare(
            "SELECT uuid, sender_uuid, thread_uuid, reply_to_uuid, global_seq,
                    key_epoch, ciphertext, client_ts, server_ts, read
             FROM messages
             WHERE thread_uuid IS NULL
             ORDER BY global_seq DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit], row_to_json)?;

        let mut messages: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        messages.reverse();

        // Collect UUIDs of top-level messages to fetch their thread replies
        let top_uuids: Vec<String> = messages
            .iter()
            .filter_map(|m| m["uuid"].as_str().map(|s| s.to_string()))
            .collect();

        // Fetch thread replies for these top-level messages
        let replies = self.get_thread_replies(&top_uuids)?;
        messages.extend(replies);

        Ok(messages)
    }

    /// Fetch messages with global_seq < before_seq, plus their thread replies.
    pub fn get_before(&self, before_seq: i64, limit: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, sender_uuid, thread_uuid, reply_to_uuid, global_seq,
                    key_epoch, ciphertext, client_ts, server_ts, read
             FROM messages
             WHERE thread_uuid IS NULL AND global_seq < ?1
             ORDER BY global_seq DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![before_seq, limit], row_to_json)?;

        let mut messages: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        messages.reverse();

        let top_uuids: Vec<String> = messages
            .iter()
            .filter_map(|m| m["uuid"].as_str().map(|s| s.to_string()))
            .collect();

        let replies = self.get_thread_replies(&top_uuids)?;
        messages.extend(replies);

        Ok(messages)
    }

    /// Return contiguous sequence ranges held in [from_seq, to_seq], capped at `cap` pairs.
    /// If more gaps exist than the cap allows, the final segment extends to the local maximum
    /// (a slight over-claim that is acceptable for best-effort sync).
    pub fn get_segments_in_range(
        &self,
        from_seq: i64,
        to_seq: i64,
        cap: usize,
    ) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT global_seq FROM messages
             WHERE global_seq >= ?1 AND global_seq <= ?2
             ORDER BY global_seq",
        )?;

        let seqs: Vec<i64> = stmt
            .query_map(params![from_seq, to_seq], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        if seqs.is_empty() {
            return Ok(Vec::new());
        }

        let mut segments: Vec<(i64, i64)> = Vec::new();
        let mut seg_start = seqs[0];
        let mut seg_end = seqs[0];

        for &seq in &seqs[1..] {
            if seq == seg_end + 1 {
                seg_end = seq;
            } else {
                segments.push((seg_start, seg_end));
                if segments.len() >= cap {
                    // Over-claim: extend last segment to the local max in range
                    let last = segments.last_mut().unwrap();
                    last.1 = *seqs.last().unwrap();
                    return Ok(segments);
                }
                seg_start = seq;
                seg_end = seq;
            }
        }
        segments.push((seg_start, seg_end));
        Ok(segments)
    }

    /// Fetch all thread replies for a set of top-level message UUIDs.
    fn get_thread_replies(&self, parent_uuids: &[String]) -> Result<Vec<Value>> {
        if parent_uuids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = (1..=parent_uuids.len())
            .map(|i| format!("?{}", i))
            .collect();
        let sql = format!(
            "SELECT uuid, sender_uuid, thread_uuid, reply_to_uuid, global_seq,
                    key_epoch, ciphertext, client_ts, server_ts, read
             FROM messages
             WHERE thread_uuid IN ({})
             ORDER BY global_seq",
            placeholders.join(", ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = parent_uuids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_json)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn open_mem_store() -> MessageStore {
        MessageStore::open_in_memory()
    }

    fn make_msg(seq: i64) -> Value {
        serde_json::json!({
            "uuid": format!("00000000-0000-0000-0000-{:012}", seq),
            "sender_uuid": "sender-uuid",
            "thread_uuid": null,
            "reply_to_uuid": null,
            "global_seq": seq,
            "key_epoch": 0,
            "ciphertext": base64::engine::general_purpose::STANDARD.encode(format!("msg {seq}")),
            "ts": "2026-03-22T00:00:00Z",
            "server_ts": "2026-03-22T00:00:01Z",
        })
    }

    #[test]
    fn test_insert_and_high_water() {
        let store = open_mem_store();
        assert_eq!(store.get_high_water().unwrap(), 0);

        store.insert_message(&make_msg(1)).unwrap();
        store.insert_message(&make_msg(3)).unwrap();

        assert_eq!(store.get_high_water().unwrap(), 3);
    }

    #[test]
    fn test_insert_or_ignore_deduplication() {
        let store = open_mem_store();
        let msg = make_msg(1);
        store.insert_message(&msg).unwrap();
        store.insert_message(&msg).unwrap();
        assert_eq!(store.get_high_water().unwrap(), 1);
    }

    #[test]
    fn test_get_range() {
        let store = open_mem_store();
        for i in 1..=5 {
            store.insert_message(&make_msg(i)).unwrap();
        }

        let range = store.get_range(2, 4).unwrap();
        assert_eq!(range.len(), 3);
        assert_eq!(range[0]["global_seq"], 2);
        assert_eq!(range[2]["global_seq"], 4);
    }

    #[test]
    fn test_mark_read() {
        let store = open_mem_store();
        store.insert_message(&make_msg(1)).unwrap();

        let before = store.get_range(1, 1).unwrap();
        assert_eq!(before[0]["read"], 0);

        store
            .mark_read(&format!("00000000-0000-0000-0000-{:012}", 1))
            .unwrap();

        let after = store.get_range(1, 1).unwrap();
        assert_eq!(after[0]["read"], 1);
    }

    #[test]
    fn test_peer_keys_roundtrip() {
        let store = open_mem_store();
        let user = "user-uuid-1";
        let ed25519 = vec![0xABu8; 32];
        let x25519 = vec![0xCDu8; 32];

        assert!(store.get_peer_keys(user).unwrap().is_none());
        store.store_peer_keys(user, &ed25519, &x25519).unwrap();
        assert_eq!(store.get_peer_keys(user).unwrap(), Some((ed25519, x25519)));
    }

    #[test]
    fn test_peer_keys_replace() {
        let store = open_mem_store();
        let user = "user-uuid-1";
        let ed1 = vec![0x01u8; 32];
        let x1 = vec![0x02u8; 32];
        let ed2 = vec![0x03u8; 32];
        let x2 = vec![0x04u8; 32];
        store.store_peer_keys(user, &ed1, &x1).unwrap();
        store.store_peer_keys(user, &ed2, &x2).unwrap();
        assert_eq!(store.get_peer_keys(user).unwrap(), Some((ed2, x2)));
    }

    #[test]
    fn test_peer_keys_missing_x25519_returns_none() {
        let store = open_mem_store();
        let user = "user-uuid-missing-x25519";
        // Manually insert a row with NULL x25519_public (simulates old cache entry)
        store
            .conn
            .execute(
                "INSERT INTO peer_keys (user_uuid, ed25519_public, x25519_public, fetched_at)
             VALUES (?1, ?2, NULL, ?3)",
                rusqlite::params![user, vec![0xAAu8; 32], "2026-01-01T00:00:00Z"],
            )
            .unwrap();
        assert!(store.get_peer_keys(user).unwrap().is_none());
    }

    #[test]
    fn test_get_segments_contiguous() {
        let store = open_mem_store();
        for i in 1..=5 {
            store.insert_message(&make_msg(i)).unwrap();
        }
        let segs = store.get_segments_in_range(1, 5, 10).unwrap();
        assert_eq!(segs, vec![(1, 5)]);
    }

    #[test]
    fn test_get_segments_with_gaps() {
        let store = open_mem_store();
        for i in [1i64, 2, 4, 5, 8] {
            store.insert_message(&make_msg(i)).unwrap();
        }
        let segs = store.get_segments_in_range(1, 10, 10).unwrap();
        assert_eq!(segs, vec![(1, 2), (4, 5), (8, 8)]);
    }

    #[test]
    fn test_get_segments_scoped_to_range() {
        let store = open_mem_store();
        for i in 1..=10 {
            store.insert_message(&make_msg(i)).unwrap();
        }
        let segs = store.get_segments_in_range(3, 7, 10).unwrap();
        assert_eq!(segs, vec![(3, 7)]);
    }

    #[test]
    fn test_get_segments_empty() {
        let store = open_mem_store();
        let segs = store.get_segments_in_range(1, 10, 10).unwrap();
        assert!(segs.is_empty());
    }

    #[test]
    fn test_get_segments_cap_extends_last() {
        let store = open_mem_store();
        // Seqs: 1, 3, 5, 7, 9 — each is its own segment (gap of 1 between each)
        for i in [1i64, 3, 5, 7, 9] {
            store.insert_message(&make_msg(i)).unwrap();
        }
        // Cap at 2 — should return [(1,1), (3,9)] where last extends to max
        let segs = store.get_segments_in_range(1, 10, 2).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], (1, 1));
        // Last segment's end extended to 9 (the max seq in range)
        assert_eq!(segs[1].1, 9);
    }

    fn make_thread_reply(seq: i64, thread_uuid: &str) -> Value {
        let mut msg = make_msg(seq);
        msg["thread_uuid"] = serde_json::Value::String(thread_uuid.to_string());
        msg
    }

    #[test]
    fn test_get_recent_includes_thread_replies() {
        let store = open_mem_store();
        // Top-level message at seq 1
        let top = make_msg(1);
        let top_uuid = top["uuid"].as_str().unwrap().to_string();
        store.insert_message(&top).unwrap();
        // Thread reply at seq 2
        store
            .insert_message(&make_thread_reply(2, &top_uuid))
            .unwrap();

        let recent = store.get_recent(50).unwrap();
        assert_eq!(recent.len(), 2);
        // Top-level comes first (reversed from DESC), reply follows
        assert_eq!(recent[0]["global_seq"], 1);
        assert_eq!(recent[1]["global_seq"], 2);
    }

    #[test]
    fn test_get_before_paginates() {
        let store = open_mem_store();
        for i in 1..=5 {
            store.insert_message(&make_msg(i)).unwrap();
        }
        // Fetch top-level messages before seq 4, limit 2
        let page = store.get_before(4, 2).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0]["global_seq"], 2);
        assert_eq!(page[1]["global_seq"], 3);
    }
}
