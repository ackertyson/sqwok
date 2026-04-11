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

impl ContactStore {
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
        let rows = stmt.query_map(params![pattern, limit as i64], |row| {
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
        })?;
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
        let rows = stmt.query_map(params![limit as i64], |row| {
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
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
