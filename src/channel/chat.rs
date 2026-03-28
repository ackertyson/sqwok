use crate::dlog;
use anyhow::Result;
use base64::Engine;
use ed25519_dalek::VerifyingKey;
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

use crate::channel::protocol::Frame;
use crate::crypto::identity::ed25519_to_x25519_public;
use crate::crypto::{bundle_to_wire_payload, parse_key_bundle_from_wire, ChatCrypto};
use crate::storage::messages::MessageStore;

pub struct ChatChannel {
    pub chat_uuid: String,
    pub high_water: i64,
    local_seq: i64,
    pub store: MessageStore,
    user_uuid: Uuid,
    server_url: String,
    identity_dir: PathBuf,
    chat_dir: PathBuf,
    pub crypto: Option<ChatCrypto>,
    /// join_ref from the phx_join frame — must be included in all outgoing
    /// channel messages so Phoenix can route them to the right channel process.
    pub join_ref: Option<String>,
}

impl ChatChannel {
    pub fn new(
        chat_uuid: &str,
        user_uuid: Uuid,
        server_url: String,
        identity_dir: PathBuf,
        chat_dir: PathBuf,
        store: MessageStore,
        crypto: Option<ChatCrypto>,
    ) -> Self {
        let high_water = store.get_high_water().unwrap_or(0);
        ChatChannel {
            chat_uuid: chat_uuid.to_string(),
            high_water,
            local_seq: 0,
            store,
            user_uuid,
            server_url,
            identity_dir,
            chat_dir,
            crypto,
            join_ref: None,
        }
    }

    pub fn join_frame(&mut self) -> Frame {
        let topic = format!("chat:{}", self.chat_uuid);
        let frame = Frame::join(
            &topic,
            serde_json::json!({
                "high_water": self.high_water
            }),
        );
        self.join_ref = frame.join_ref.clone();
        frame
    }

    /// Create a channel frame with the correct join_ref for this channel.
    /// Use this for all outgoing messages after joining.
    pub fn frame(&self, event: &str, payload: serde_json::Value) -> Frame {
        let topic = format!("chat:{}", self.chat_uuid);
        Frame {
            join_ref: self.join_ref.clone(),
            ref_id: Some(crate::channel::protocol::next_ref()),
            topic,
            event: event.to_string(),
            payload,
        }
    }

    pub fn key_request_frame(&self) -> Frame {
        self.frame("key:request", serde_json::json!({}))
    }

    pub fn typing_notify_frame(
        &self,
        thread_uuid: Option<&str>,
        reply_to_uuid: Option<&str>,
    ) -> Frame {
        self.frame(
            "typing:notify",
            serde_json::json!({
                "thread_uuid": thread_uuid,
                "reply_to_uuid": reply_to_uuid,
            }),
        )
    }

    pub fn send_message(
        &mut self,
        text: &str,
        thread_uuid: Option<&str>,
        reply_to_uuid: Option<&str>,
    ) -> Result<Frame> {
        let crypto = self
            .crypto
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot send: no encryption keys yet"))?;

        self.local_seq += 1;
        let uuid = uuid::Uuid::new_v4().to_string();
        let ts = chrono::Utc::now().to_rfc3339();

        let (ciphertext, key_epoch) = crypto.encrypt(&self.user_uuid, text)?;

        Ok(self.frame(
            "msg:new",
            serde_json::json!({
                "uuid": uuid,
                "thread_uuid": thread_uuid,
                "reply_to_uuid": reply_to_uuid,
                "seq": self.local_seq,
                "key_epoch": key_epoch,
                "ciphertext": ciphertext,
                "ts": ts
            }),
        ))
    }

    pub fn ack_frame(&self) -> Frame {
        self.frame(
            "msg:ack",
            serde_json::json!({"through_seq": self.high_water}),
        )
    }

    pub fn sync_catchup_frame(&self) -> Frame {
        self.frame(
            "sync:catchup",
            serde_json::json!({
                "high_water": self.high_water
            }),
        )
    }

    pub fn sync_scrollback_frame(&self, before_seq: i64, limit: i64) -> Frame {
        self.frame(
            "sync:scrollback",
            serde_json::json!({
                "before_seq": before_seq,
                "limit": limit
            }),
        )
    }

    /// Returns Some(frame) when a response should be sent back (e.g. for key:request, sync:query).
    pub fn handle_incoming(&mut self, frame: &Frame) -> Result<Option<Frame>> {
        match frame.event.as_str() {
            "msg:new" => {
                self.handle_msg_new(&frame.payload)?;
                Ok(None)
            }
            "presence_state" => {
                self.handle_presence_state(&frame.payload)?;
                Ok(None)
            }
            "sync:push" => {
                self.handle_sync_push(&frame.payload)?;
                Ok(None)
            }
            "sync:query" => self.handle_sync_query(&frame.payload),
            "key:distribute" => {
                self.handle_key_distribute(&frame.payload)?;
                Ok(None)
            }
            "key:request" => self.handle_key_request(&frame.payload),
            "phx_reply" => {
                self.handle_reply(&frame.payload)?;
                Ok(None)
            }
            "phx_error" => {
                eprintln!("Channel error: {:?}", frame.payload);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn handle_msg_new(&mut self, payload: &Value) -> Result<()> {
        let global_seq = payload["global_seq"].as_i64().unwrap_or(0);

        self.store.insert_message(payload)?;

        if global_seq > self.high_water {
            self.high_water = global_seq;
        }

        let ciphertext = payload["ciphertext"].as_str().unwrap_or("");
        let sender_str = payload["sender_uuid"].as_str().unwrap_or("unknown");
        let short_sender = &sender_str[..sender_str.len().min(8)];

        if let Some(crypto) = &self.crypto {
            if let Ok(sender_uuid) = Uuid::parse_str(sender_str) {
                match crypto.decrypt(&sender_uuid, ciphertext) {
                    Ok(text) => println!("[{}] {}", short_sender, text),
                    Err(e) => eprintln!("[{}] <decrypt failed: {}>", short_sender, e),
                }
            }
        } else {
            println!("[{}] <encrypted — awaiting keys>", short_sender);
        }

        Ok(())
    }

    fn handle_presence_state(&self, payload: &Value) -> Result<()> {
        if let Some(members) = payload["members"].as_array() {
            println!("Online members:");
            for m in members {
                println!(
                    "  {} ({})",
                    m["screenname"].as_str().unwrap_or("?"),
                    m["role"].as_str().unwrap_or("member")
                );
            }
        }
        Ok(())
    }

    fn handle_sync_query(&self, payload: &Value) -> Result<Option<Frame>> {
        let high = self.store.get_high_water().unwrap_or(0);
        let low = self.store.get_low_water().unwrap_or(0);
        let requester = payload["requester"].as_str().unwrap_or("");
        let from_seq = payload["from_seq"].as_i64();
        let to_seq = payload["to_seq"].as_i64();
        dlog!(
            "[SYNC] handle_sync_query: requester={} from_seq={:?} to_seq={:?} → responding low_water={} high_water={}",
            requester, from_seq, to_seq, low, high
        );

        Ok(Some(self.frame(
            "sync:offer",
            serde_json::json!({
                "requester": requester,
                "low_water": low,
                "high_water": high
            }),
        )))
    }

    fn handle_sync_push(&mut self, payload: &Value) -> Result<()> {
        if let Some(messages) = payload["messages"].as_array() {
            dlog!(
                "[SYNC] handle_sync_push: received {} messages, current high_water={}",
                messages.len(), self.high_water
            );
            for msg in messages {
                self.store.insert_message(msg)?;
                let seq = msg["global_seq"].as_i64().unwrap_or(0);
                if seq > self.high_water {
                    self.high_water = seq;
                }
            }
            dlog!("[SYNC] handle_sync_push: new high_water={}", self.high_water);
        }
        Ok(())
    }

    fn handle_key_distribute(&mut self, payload: &Value) -> Result<()> {
        let sender_id = payload["sender_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("key:distribute missing sender_id"))?
            .to_string();

        dlog!("key:distribute received from sender={}", sender_id);

        let bundle = match parse_key_bundle_from_wire(payload) {
            Ok(b) => b,
            Err(e) => {
                dlog!("key:distribute parse_bundle FAILED: {}", e);
                return Err(e);
            }
        };
        let sender_ed25519 = match self.get_peer_ed25519(&sender_id, false) {
            Ok(k) => k,
            Err(e) => {
                dlog!(
                    "key:distribute get_peer_ed25519({}) FAILED: {}",
                    sender_id,
                    e
                );
                return Err(e);
            }
        };
        let sender_x25519 = ed25519_to_x25519_public(&sender_ed25519)
            .ok_or_else(|| anyhow::anyhow!("invalid sender Ed25519 key"))?;

        if let Some(crypto) = &mut self.crypto {
            crypto.receive_key_bundle(&sender_x25519, &sender_ed25519, &bundle)?;
            dlog!("key:distribute applied (already had keys)");
        } else {
            let identity_dir = self.identity_dir.clone();
            let chat_dir = self.chat_dir.clone();
            let mut crypto = ChatCrypto::load(&identity_dir, &chat_dir)?
                .unwrap_or_else(|| ChatCrypto::from_empty(&identity_dir, &chat_dir).unwrap());
            crypto.receive_key_bundle(&sender_x25519, &sender_ed25519, &bundle)?;
            self.crypto = Some(crypto);
            dlog!("key:distribute applied — first keys received!");
        }

        Ok(())
    }

    fn handle_key_request(&self, payload: &Value) -> Result<Option<Frame>> {
        let requester_id = payload["requester_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("key:request missing requester_id"))?;

        dlog!("key:request received from requester={}", requester_id);

        if requester_id == self.user_uuid.to_string() {
            dlog!("key:request — ignoring (is ourselves)");
            return Ok(None);
        }

        let crypto = match &self.crypto {
            Some(c) => c,
            None => {
                dlog!("key:request — we have NO crypto, cannot respond");
                return Ok(None);
            }
        };

        let requester_ed25519 = match self.get_peer_ed25519(requester_id, true) {
            Ok(k) => {
                dlog!("key:request — fetched peer ed25519 for {}", requester_id);
                k
            }
            Err(e) => {
                dlog!(
                    "key:request — get_peer_ed25519({}) FAILED: {}",
                    requester_id,
                    e
                );
                return Err(e);
            }
        };
        let requester_x25519 = ed25519_to_x25519_public(&requester_ed25519)
            .ok_or_else(|| anyhow::anyhow!("invalid requester Ed25519 key"))?;

        let bundle = crypto.prepare_key_bundle(&requester_x25519, true)?;
        let wire_payload = bundle_to_wire_payload(&bundle, requester_id);

        dlog!(
            "key:request — responding with key:distribute to {}",
            requester_id
        );
        Ok(Some(self.frame("key:distribute", wire_payload)))
    }

    fn handle_reply(&self, payload: &Value) -> Result<()> {
        if let Some(status) = payload["status"].as_str() {
            if status == "error" {
                eprintln!("Server error: {:?}", payload["response"]);
            }
        }
        Ok(())
    }

    /// Fetch a peer's Ed25519 public key from local cache or server.
    /// Pass `force_refresh = true` when about to encrypt key material for this peer —
    /// ensures we use their current key even if they regenerated it (e.g. account recovery).
    pub fn get_peer_ed25519(&self, user_uuid: &str, force_refresh: bool) -> Result<VerifyingKey> {
        if !force_refresh {
            if let Some(key_bytes) = self.store.get_peer_key(user_uuid)? {
                dlog!("get_peer_ed25519({}) — found in local cache", user_uuid);
                let arr: [u8; 32] = key_bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("cached key wrong length for {}", user_uuid))?;
                return Ok(VerifyingKey::from_bytes(&arr)?);
            }
        }

        let url = format!("{}/api/users/{}/e2e_key", self.server_url, user_uuid);
        dlog!(
            "get_peer_ed25519({}) — fetching from server (force_refresh={})",
            user_uuid,
            force_refresh
        );
        let token = crate::auth::token::build_token(&self.identity_dir, &self.server_url)?;

        let resp: serde_json::Value = tokio::task::block_in_place(|| {
            reqwest::blocking::Client::new()
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .map_err(|e| anyhow::anyhow!("HTTP fetch failed: {}", e))?
                .json()
                .map_err(|e| anyhow::anyhow!("JSON parse failed: {}", e))
        })?;

        dlog!(
            "get_peer_ed25519({}) — server response: {}",
            user_uuid,
            resp
        );

        let key_b64 = resp["e2e_public_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no e2e key for user {}", user_uuid))?;
        let key_bytes = base64::engine::general_purpose::STANDARD.decode(key_b64)?;

        self.store.store_peer_key(user_uuid, &key_bytes)?;
        dlog!(
            "get_peer_ed25519({}) — stored in cache, {} bytes",
            user_uuid,
            key_bytes.len()
        );

        let arr: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("server key wrong length for {}", user_uuid))?;
        Ok(VerifyingKey::from_bytes(&arr)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn open_temp_store() -> MessageStore {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "
            CREATE TABLE messages (
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
            CREATE INDEX idx_messages_global_seq ON messages(global_seq);
            CREATE INDEX idx_messages_thread ON messages(thread_uuid);
            CREATE TABLE peer_keys (
                user_uuid TEXT PRIMARY KEY,
                ed25519_public BLOB NOT NULL,
                fetched_at TEXT NOT NULL
            );
            ",
        )
        .unwrap();
        MessageStore::from_connection(conn)
    }

    fn make_test_channel() -> ChatChannel {
        use std::env;

        // Write a test Ed25519 key to a temp dir.
        let dir = env::temp_dir().join(format!("sqwok-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        std::fs::write(dir.join("e2e_private.key"), signing_key.to_bytes()).unwrap();

        let chat_dir = env::temp_dir().join(format!("sqwok-chat-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&chat_dir).unwrap();

        let crypto = ChatCrypto::create_new(&dir, &chat_dir).unwrap();

        let user_uuid = Uuid::new_v4();
        let store = open_temp_store();
        ChatChannel::new(
            "test-chat",
            user_uuid,
            "http://localhost:4000".to_string(),
            dir,
            chat_dir,
            store,
            Some(crypto),
        )
    }

    #[test]
    fn test_send_message_payload() {
        let mut chat = make_test_channel();
        let frame = chat.send_message("hello world", None, None).unwrap();
        assert_eq!(frame.event, "msg:new");
        assert_eq!(frame.topic, "chat:test-chat");

        let payload = &frame.payload;
        assert!(payload["uuid"].is_string());
        assert_eq!(payload["seq"], 1);
        assert!(payload["key_epoch"].is_number());
        // Ciphertext must be valid base64 and longer than the plaintext
        let ct = payload["ciphertext"].as_str().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(ct)
            .unwrap();
        assert!(decoded.len() > "hello world".len()); // has encryption overhead
    }

    #[test]
    fn test_local_seq_increments() {
        let mut chat = make_test_channel();
        let f1 = chat.send_message("a", None, None).unwrap();
        let f2 = chat.send_message("b", None, None).unwrap();
        assert_eq!(f1.payload["seq"], 1);
        assert_eq!(f2.payload["seq"], 2);
    }

    #[test]
    fn test_send_message_requires_crypto() {
        let store = open_temp_store();
        let mut chat = ChatChannel::new(
            "test-chat",
            Uuid::new_v4(),
            "http://localhost:4000".to_string(),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
            store,
            None, // No crypto
        );
        let result = chat.send_message("hello", None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_ack_frame() {
        let store = open_temp_store();
        let mut chat = ChatChannel::new(
            "test-chat",
            Uuid::new_v4(),
            "http://localhost:4000".to_string(),
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp"),
            store,
            None,
        );
        chat.high_water = 42;
        let frame = chat.ack_frame();
        assert_eq!(frame.event, "msg:ack");
        assert_eq!(frame.payload["through_seq"], 42);
    }
}
