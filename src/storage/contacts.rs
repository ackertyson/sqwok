use anyhow::Result;
use rusqlite::{params, Connection};
use uuid::Uuid;

pub struct ContactStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Contact {
    pub uuid: Uuid,
    pub screenname: String,
    pub last_seen_chat: Option<Uuid>,
    pub updated_at: i64,
}

fn contact_from_row(row: &rusqlite::Row) -> rusqlite::Result<Contact> {
    Ok(Contact {
        uuid: row
            .get::<_, String>(0)?
            .parse()
            .unwrap_or_else(|_| Uuid::nil()),
        screenname: row.get(1)?,
        last_seen_chat: row
            .get::<_, Option<String>>(2)?
            .and_then(|s| s.parse().ok()),
        updated_at: row.get(3)?,
    })
}

impl ContactStore {
    #[cfg(test)]
    fn open_in_memory() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                uuid TEXT PRIMARY KEY,
                screenname TEXT NOT NULL,
                last_seen_chat TEXT,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_contacts_screenname
            ON contacts(screenname COLLATE NOCASE);
            CREATE TABLE IF NOT EXISTS chat_scroll (
                chat_uuid TEXT PRIMARY KEY,
                msg_uuid  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS blocked_users (
                uuid TEXT PRIMARY KEY,
                blocked_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );",
        )
        .unwrap();
        Self { conn }
    }

    pub fn open() -> Result<Self> {
        let path = crate::config::home_dir()?
            .join(".sqwok")
            .join("contacts.db");
        std::fs::create_dir_all(path.parent().unwrap())?;
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                uuid TEXT PRIMARY KEY,
                screenname TEXT NOT NULL,
                last_seen_chat TEXT,
                updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            CREATE INDEX IF NOT EXISTS idx_contacts_screenname
            ON contacts(screenname COLLATE NOCASE);
            CREATE TABLE IF NOT EXISTS chat_scroll (
                chat_uuid TEXT PRIMARY KEY,
                msg_uuid  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS blocked_users (
                uuid TEXT PRIMARY KEY,
                blocked_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );",
        )?;
        Ok(Self { conn })
    }

    /// Upsert a contact. Called whenever we see a user in a chat.
    pub fn upsert(&self, uuid: Uuid, screenname: &str, chat_uuid: Option<Uuid>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO contacts (uuid, screenname, last_seen_chat, updated_at)
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))
             ON CONFLICT(uuid) DO UPDATE SET
                screenname = excluded.screenname,
                last_seen_chat = excluded.last_seen_chat,
                updated_at = excluded.updated_at",
            params![
                uuid.to_string(),
                screenname,
                chat_uuid.map(|u| u.to_string())
            ],
        )?;
        Ok(())
    }

    /// Search contacts locally by screenname substring.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Contact>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT uuid, screenname, last_seen_chat, updated_at
             FROM contacts
             WHERE screenname LIKE ?1 COLLATE NOCASE
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pattern, limit as i64], contact_from_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Persist the last-selected message UUID for a chat (scroll position).
    pub fn save_scroll(&self, chat_uuid: &str, msg_uuid: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chat_scroll (chat_uuid, msg_uuid) VALUES (?1, ?2)
             ON CONFLICT(chat_uuid) DO UPDATE SET msg_uuid = excluded.msg_uuid",
            params![chat_uuid, msg_uuid],
        )?;
        Ok(())
    }

    /// Load the last-selected message UUID for a chat, if any.
    pub fn load_scroll(&self, chat_uuid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT msg_uuid FROM chat_scroll WHERE chat_uuid = ?1")?;
        let mut rows = stmt.query(params![chat_uuid])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Add a UUID to the blocked list.
    pub fn block(&self, uuid: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO blocked_users (uuid) VALUES (?1)",
            params![uuid],
        )?;
        Ok(())
    }

    /// Remove a UUID from the blocked list.
    pub fn unblock(&self, uuid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM blocked_users WHERE uuid = ?1", params![uuid])?;
        Ok(())
    }

    /// Look up a contact's screenname by UUID.
    pub fn screenname_for(&self, uuid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT screenname FROM contacts WHERE uuid = ?1")?;
        let mut rows = stmt.query(params![uuid])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Return all blocked UUIDs.
    pub fn blocked_uuids(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT uuid FROM blocked_users ORDER BY blocked_at ASC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get all contacts, most recently seen first.
    pub fn all(&self, limit: usize) -> Result<Vec<Contact>> {
        let mut stmt = self.conn.prepare(
            "SELECT uuid, screenname, last_seen_chat, updated_at
             FROM contacts
             ORDER BY updated_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], contact_from_row)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_and_screenname_for() {
        let store = ContactStore::open_in_memory();
        let uuid = Uuid::new_v4();
        store.upsert(uuid, "alice", None).unwrap();
        let name = store.screenname_for(&uuid.to_string()).unwrap();
        assert_eq!(name.as_deref(), Some("alice"));
    }

    #[test]
    fn test_upsert_updates_screenname() {
        let store = ContactStore::open_in_memory();
        let uuid = Uuid::new_v4();
        store.upsert(uuid, "alice", None).unwrap();
        store.upsert(uuid, "alice2", None).unwrap();
        let name = store.screenname_for(&uuid.to_string()).unwrap();
        assert_eq!(name.as_deref(), Some("alice2"));
    }

    #[test]
    fn test_screenname_for_missing_returns_none() {
        let store = ContactStore::open_in_memory();
        let result = store.screenname_for(&Uuid::new_v4().to_string()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_search_case_insensitive() {
        let store = ContactStore::open_in_memory();
        store.upsert(Uuid::new_v4(), "AliceSmith", None).unwrap();
        store.upsert(Uuid::new_v4(), "BobAlice", None).unwrap();
        store.upsert(Uuid::new_v4(), "Charlie", None).unwrap();

        let results = store.search("alice", 10).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|c| c.screenname.to_lowercase().contains("alice")));
    }

    #[test]
    fn test_search_respects_limit() {
        let store = ContactStore::open_in_memory();
        for i in 0..5 {
            store
                .upsert(Uuid::new_v4(), &format!("user{}", i), None)
                .unwrap();
        }
        let results = store.search("user", 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_scroll_roundtrip() {
        let store = ContactStore::open_in_memory();
        let chat = Uuid::new_v4().to_string();
        let msg = Uuid::new_v4().to_string();

        assert!(store.load_scroll(&chat).unwrap().is_none());
        store.save_scroll(&chat, &msg).unwrap();
        assert_eq!(
            store.load_scroll(&chat).unwrap().as_deref(),
            Some(msg.as_str())
        );
    }

    #[test]
    fn test_scroll_update() {
        let store = ContactStore::open_in_memory();
        let chat = Uuid::new_v4().to_string();
        let msg1 = Uuid::new_v4().to_string();
        let msg2 = Uuid::new_v4().to_string();

        store.save_scroll(&chat, &msg1).unwrap();
        store.save_scroll(&chat, &msg2).unwrap();
        assert_eq!(
            store.load_scroll(&chat).unwrap().as_deref(),
            Some(msg2.as_str())
        );
    }

    #[test]
    fn test_block_unblock() {
        let store = ContactStore::open_in_memory();
        let uuid = Uuid::new_v4().to_string();

        assert!(store.blocked_uuids().unwrap().is_empty());
        store.block(&uuid).unwrap();
        assert_eq!(store.blocked_uuids().unwrap(), vec![uuid.clone()]);
        store.unblock(&uuid).unwrap();
        assert!(store.blocked_uuids().unwrap().is_empty());
    }

    #[test]
    fn test_block_is_idempotent() {
        let store = ContactStore::open_in_memory();
        let uuid = Uuid::new_v4().to_string();
        store.block(&uuid).unwrap();
        store.block(&uuid).unwrap(); // INSERT OR IGNORE — must not error
        assert_eq!(store.blocked_uuids().unwrap().len(), 1);
    }
}
