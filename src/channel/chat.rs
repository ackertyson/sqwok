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

    fn topic(&self) -> String {
        format!("chat:{}", self.chat_uuid)
    }

    pub fn join_frame(&mut self) -> Frame {
        // Report actual segment coverage so the server knows exactly what we hold.
        // Full range [1, i64::MAX] gives a complete picture; cap at 10 pairs to keep
        // the frame small. An empty list signals a fresh client with no history.
        let segments = self
            .store
            .get_segments_in_range(1, i64::MAX, 10)
            .unwrap_or_default();
        let segments_json: Vec<[i64; 2]> = segments.iter().map(|&(s, e)| [s, e]).collect();

        let frame = Frame::join(
            &self.topic(),
            serde_json::json!({
                "segments": segments_json
            }),
        );
        self.join_ref = frame.join_ref.clone();
        frame
    }

    /// Create a channel frame with the correct join_ref for this channel.
    /// Use this for all outgoing messages after joining.
    pub fn frame(&self, event: &str, payload: serde_json::Value) -> Frame {
        Frame {
            join_ref: self.join_ref.clone(),
            ref_id: Some(crate::channel::protocol::next_ref()),
            topic: self.topic(),
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
        let segments = self
            .store
            .get_segments_in_range(1, i64::MAX, 10)
            .unwrap_or_default();
        let segments_json: Vec<[i64; 2]> = segments.iter().map(|&(s, e)| [s, e]).collect();
        self.frame(
            "sync:catchup",
            serde_json::json!({
                "segments": segments_json
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
            "member_list" => {
                self.handle_member_list(&frame.payload)?;
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

    fn handle_member_list(&self, payload: &Value) -> Result<()> {
        if let Some(members) = payload["members"].as_array() {
            println!("Members:");
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
        let requester = payload["requester"].as_str().unwrap_or("");
        let from_seq = payload["from_seq"].as_i64().unwrap_or(1);
        let to_seq = payload["to_seq"].as_i64().unwrap_or(i64::MAX);

        // Report actual segments within the requested range so the server can
        // assign only ranges we genuinely hold.
        let segments = self
            .store
            .get_segments_in_range(from_seq, to_seq, 10)
            .unwrap_or_default();
        let segments_json: Vec<[i64; 2]> = segments.iter().map(|&(s, e)| [s, e]).collect();

        Ok(Some(self.frame(
            "sync:offer",
            serde_json::json!({
                "requester": requester,
                "segments": segments_json
            }),
        )))
    }

    fn handle_sync_push(&mut self, payload: &Value) -> Result<()> {
        if let Some(messages) = payload["messages"].as_array() {
            for msg in messages {
                self.store.insert_message(msg)?;
                let seq = msg["global_seq"].as_i64().unwrap_or(0);
                if seq > self.high_water {
                    self.high_water = seq;
                }
            }
        }
        Ok(())
    }

    fn handle_key_distribute(&mut self, payload: &Value) -> Result<()> {
        let sender_id = payload["sender_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("key:distribute missing sender_id"))?
            .to_string();

        let bundle = parse_key_bundle_from_wire(payload)?;
        let sender_ed25519 = self.get_peer_ed25519(&sender_id, false)?;
        let sender_x25519 = ed25519_to_x25519_public(&sender_ed25519)
            .ok_or_else(|| anyhow::anyhow!("invalid sender Ed25519 key"))?;

        if let Some(crypto) = &mut self.crypto {
            crypto.receive_key_bundle(&sender_x25519, &sender_ed25519, &bundle)?;
        } else {
            let identity_dir = self.identity_dir.clone();
            let chat_dir = self.chat_dir.clone();
            let mut crypto = ChatCrypto::load(&identity_dir, &chat_dir)?
                .unwrap_or_else(|| ChatCrypto::from_empty(&identity_dir, &chat_dir).unwrap());
            crypto.receive_key_bundle(&sender_x25519, &sender_ed25519, &bundle)?;
            self.crypto = Some(crypto);
        }

        Ok(())
    }

    fn handle_key_request(&self, payload: &Value) -> Result<Option<Frame>> {
        let requester_id = payload["requester_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("key:request missing requester_id"))?;

        if requester_id == self.user_uuid.to_string() {
            return Ok(None);
        }

        let crypto = match &self.crypto {
            Some(c) => c,
            None => return Ok(None),
        };

        let requester_ed25519 = self.get_peer_ed25519(requester_id, true)?;
        let requester_x25519 = ed25519_to_x25519_public(&requester_ed25519)
            .ok_or_else(|| anyhow::anyhow!("invalid requester Ed25519 key"))?;

        let bundle = crypto.prepare_key_bundle(&requester_x25519, true)?;
        let wire_payload = bundle_to_wire_payload(&bundle, requester_id);

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
                let arr: [u8; 32] = key_bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("cached key wrong length for {}", user_uuid))?;
                return Ok(VerifyingKey::from_bytes(&arr)?);
            }
        }

        let url = format!("{}/api/users/{}/e2e_key", self.server_url, user_uuid);
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

        let key_b64 = resp["e2e_public_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("no e2e key for user {}", user_uuid))?;
        let key_bytes = base64::engine::general_purpose::STANDARD.decode(key_b64)?;

        self.store.store_peer_key(user_uuid, &key_bytes)?;

        let arr: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("server key wrong length for {}", user_uuid))?;
        Ok(VerifyingKey::from_bytes(&arr)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_store() -> MessageStore {
        MessageStore::open_in_memory()
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

    // ── segment wire format ────────────────────────────────────────────────

    fn insert_seq(store: &MessageStore, seq: i64) {
        store
            .insert_message(&serde_json::json!({
                "uuid": format!("00000000-0000-0000-0000-{:012}", seq),
                "sender_uuid": "sender",
                "thread_uuid": null,
                "reply_to_uuid": null,
                "global_seq": seq,
                "key_epoch": 0,
                "ciphertext": base64::engine::general_purpose::STANDARD.encode(format!("msg {seq}")),
                "ts": "2026-01-01T00:00:00Z",
                "server_ts": "2026-01-01T00:00:01Z",
            }))
            .unwrap();
    }

    #[test]
    fn test_join_frame_uses_segments_not_high_water() {
        let mut chat = make_test_channel();
        let frame = chat.join_frame();
        assert_eq!(frame.event, "phx_join");
        // Must have "segments" key
        assert!(
            frame.payload.get("segments").is_some(),
            "join frame must include segments"
        );
        // Must NOT have old "high_water" key
        assert!(
            frame.payload.get("high_water").is_none(),
            "join frame must not include high_water"
        );
    }

    #[test]
    fn test_join_frame_empty_segments_for_fresh_store() {
        let mut chat = make_test_channel();
        let frame = chat.join_frame();
        let segs = frame.payload["segments"].as_array().unwrap();
        assert!(segs.is_empty(), "fresh store should report empty segments");
    }

    #[test]
    fn test_join_frame_reports_correct_segments() {
        let store = MessageStore::open_in_memory();
        // Store has seqs 1–3 and 6–8 (gap at 4–5)
        for seq in [1i64, 2, 3, 6, 7, 8] {
            insert_seq(&store, seq);
        }
        let user_uuid = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("sqwok-seg-test-{}", user_uuid));
        std::fs::create_dir_all(&dir).unwrap();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        std::fs::write(dir.join("e2e_private.key"), signing_key.to_bytes()).unwrap();
        let chat_dir = std::env::temp_dir().join(format!("sqwok-seg-chat-{}", user_uuid));
        std::fs::create_dir_all(&chat_dir).unwrap();
        let crypto = crate::crypto::ChatCrypto::create_new(&dir, &chat_dir).unwrap();

        let mut chat = ChatChannel::new(
            "test-chat",
            user_uuid,
            "http://localhost:4000".to_string(),
            dir,
            chat_dir,
            store,
            Some(crypto),
        );
        let frame = chat.join_frame();
        let segs = frame.payload["segments"].as_array().unwrap();
        assert_eq!(segs.len(), 2, "should report two contiguous segments");
        assert_eq!(segs[0][0], 1);
        assert_eq!(segs[0][1], 3);
        assert_eq!(segs[1][0], 6);
        assert_eq!(segs[1][1], 8);
    }

    #[test]
    fn test_sync_catchup_frame_uses_segments() {
        let mut chat = make_test_channel();
        chat.join_frame(); // set join_ref
        let frame = chat.sync_catchup_frame();
        assert_eq!(frame.event, "sync:catchup");
        assert!(frame.payload.get("segments").is_some());
        assert!(frame.payload.get("high_water").is_none());
    }

    #[test]
    fn test_handle_sync_query_returns_segments_for_range() {
        let store = MessageStore::open_in_memory();
        for seq in 1i64..=10 {
            insert_seq(&store, seq);
        }
        let user_uuid = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("sqwok-qtest-{}", user_uuid));
        std::fs::create_dir_all(&dir).unwrap();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        std::fs::write(dir.join("e2e_private.key"), signing_key.to_bytes()).unwrap();
        let chat_dir = std::env::temp_dir().join(format!("sqwok-qchat-{}", user_uuid));
        std::fs::create_dir_all(&chat_dir).unwrap();
        let crypto = crate::crypto::ChatCrypto::create_new(&dir, &chat_dir).unwrap();

        let mut chat = ChatChannel::new(
            "test-chat",
            user_uuid,
            "http://localhost:4000".to_string(),
            dir,
            chat_dir,
            store,
            Some(crypto),
        );
        chat.join_frame();

        // Query only asks about seqs 3–7
        let query_frame = Frame::new(
            "chat:test-chat",
            "sync:query",
            serde_json::json!({
                "requester": "peer-uuid",
                "from_seq": 3,
                "to_seq": 7,
            }),
        );
        let response = chat.handle_incoming(&query_frame).unwrap().unwrap();
        assert_eq!(response.event, "sync:offer");

        let segs = response.payload["segments"].as_array().unwrap();
        // All of 3–7 are in the store, so expect a single contiguous segment
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0][0], 3);
        assert_eq!(segs[0][1], 7);

        // Must not include seqs outside the query range
        assert!(response.payload.get("high_water").is_none());
        assert!(response.payload.get("low_water").is_none());
    }

    #[test]
    fn test_handle_sync_query_scopes_to_range_with_gap() {
        let store = MessageStore::open_in_memory();
        // Store has 1–5 and 8–10; query asks for 1–10
        for seq in [1i64, 2, 3, 4, 5, 8, 9, 10] {
            insert_seq(&store, seq);
        }
        let user_uuid = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("sqwok-gap-{}", user_uuid));
        std::fs::create_dir_all(&dir).unwrap();
        let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        std::fs::write(dir.join("e2e_private.key"), signing_key.to_bytes()).unwrap();
        let chat_dir = std::env::temp_dir().join(format!("sqwok-gapchat-{}", user_uuid));
        std::fs::create_dir_all(&chat_dir).unwrap();
        let crypto = crate::crypto::ChatCrypto::create_new(&dir, &chat_dir).unwrap();

        let mut chat = ChatChannel::new(
            "test-chat",
            user_uuid,
            "http://localhost:4000".to_string(),
            dir,
            chat_dir,
            store,
            Some(crypto),
        );
        chat.join_frame();

        let query_frame = Frame::new(
            "chat:test-chat",
            "sync:query",
            serde_json::json!({
                "requester": "peer-uuid",
                "from_seq": 1,
                "to_seq": 10,
            }),
        );
        let response = chat.handle_incoming(&query_frame).unwrap().unwrap();
        let segs = response.payload["segments"].as_array().unwrap();

        assert_eq!(segs.len(), 2, "should report two segments with gap at 6–7");
        assert_eq!(segs[0][0], 1);
        assert_eq!(segs[0][1], 5);
        assert_eq!(segs[1][0], 8);
        assert_eq!(segs[1][1], 10);
    }
}
