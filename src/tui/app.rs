use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::dlog;
use ratatui::widgets::ListState;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::channel::chat::ChatChannel;
use crate::channel::protocol::Frame;

use crate::storage::contacts::ContactStore;
use crate::storage::messages::MessageStore as SqlMessageStore;

use super::pane::{InputTarget, Pane};
use super::views::command_bar::CommandBarState;

#[derive(Debug, Clone)]
pub struct DisplayMessage {
    pub uuid: String,
    pub sender_uuid: String,
    pub sender_name: String,
    pub body: String,
    pub timestamp: String,
    pub global_seq: i64,
    pub thread_uuid: Option<String>,
    pub reply_to_uuid: Option<String>,
    /// True for optimistically-displayed messages awaiting server ack.
    pub pending: bool,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub uuid: String,
    pub screenname: String,
    pub online: bool,
}

#[derive(Debug, Clone)]
pub struct ChatSummary {
    pub uuid: String,
    pub topic: String,
    pub member_count: usize,
}

#[derive(Debug, Clone)]
pub struct Invitation {
    pub chat_uuid: String,
    pub topic: String,
    pub invited_by: Option<String>,
    pub received_at: i64,
}

/// State for the invite creation modal.
#[derive(Debug, Clone)]
pub struct InviteModalState {
    pub step: InviteStep,
    pub ttl_selection: usize,
    pub use_limit: Option<u32>,
    pub created_code: Option<String>,
    pub error: Option<String>,
    pub active_invites: Vec<crate::net::invites::InviteInfo>,
    /// True once we've spawned the HTTP task so we don't double-fire.
    pub creating_spawned: bool,
    /// Selected invite index in the display list (for revoke)
    pub selected_invite: usize,
    /// Code to revoke (consumed by run loop)
    pub pending_revoke: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InviteStep {
    Configure,
    Creating,
    Display,
}

impl InviteModalState {
    pub fn new() -> Self {
        InviteModalState {
            step: InviteStep::Configure,
            ttl_selection: 1, // default to 24h
            use_limit: None,
            created_code: None,
            error: None,
            active_invites: Vec::new(),
            creating_spawned: false,
            selected_invite: 0,
            pending_revoke: None,
        }
    }
}

/// State for the user search overlay.
#[derive(Debug)]
pub struct SearchModalState {
    pub query: String,
    pub results: Vec<crate::net::search::SearchResult>,
    pub selected: usize,
    /// Last query that was sent to the server (used to debounce).
    pub last_searched: String,
}

impl SearchModalState {
    pub fn new() -> Self {
        SearchModalState {
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            last_searched: String::new(),
        }
    }
}

/// State for the contacts overlay.
#[derive(Debug)]
pub struct ContactsModalState {
    pub contacts: Vec<crate::storage::contacts::Contact>,
    pub selected: usize,
    pub filter: String,
}

impl ContactsModalState {
    pub fn new(contacts: Vec<crate::storage::contacts::Contact>) -> Self {
        ContactsModalState {
            contacts,
            selected: 0,
            filter: String::new(),
        }
    }

    pub fn filtered(&self) -> Vec<&crate::storage::contacts::Contact> {
        if self.filter.is_empty() {
            self.contacts.iter().collect()
        } else {
            let f = self.filter.to_lowercase();
            self.contacts
                .iter()
                .filter(|c| c.screenname.to_lowercase().contains(&f))
                .collect()
        }
    }
}

pub enum Mode {
    ChatPicker,
    Chat,
}

pub enum ModalKind {
    MemberList,
    GroupSettings,
    InviteCreate,
    Search,
    Contacts,
}

pub enum ConnStatus {
    Connected,
    Connecting,
    Disconnected { reason: String, since: Instant },
}

/// How multiple panes are laid out on screen.
#[derive(Clone, Copy)]
pub enum PaneSplit {
    /// Vertical divider — panes side by side (left/right)
    Vertical,
    /// Horizontal divider — panes stacked (upper/lower)
    Horizontal,
}

/// In-memory display store for the TUI
pub struct TuiMessageStore {
    /// Top-level messages ordered by global_seq
    pub top_level: Vec<String>, // uuids
    /// All messages by UUID
    pub by_uuid: HashMap<String, DisplayMessage>,
    /// Thread children: thread_root_uuid -> [child uuids in order]
    pub threads: HashMap<String, Vec<String>>,
    /// Which threads are expanded
    pub expanded: HashSet<String>,
    /// Lowest global_seq currently loaded (for scrollback pagination)
    pub low_water: i64,
    /// Whether there are more messages in SQLite above the current window
    pub has_more_above: bool,
}

impl TuiMessageStore {
    pub fn new() -> Self {
        TuiMessageStore {
            top_level: Vec::new(),
            by_uuid: HashMap::new(),
            threads: HashMap::new(),
            expanded: HashSet::new(),
            low_water: i64::MAX,
            has_more_above: true,
        }
    }

    pub fn clear(&mut self) {
        self.top_level.clear();
        self.by_uuid.clear();
        self.threads.clear();
        self.expanded.clear();
        self.low_water = i64::MAX;
        self.has_more_above = true;
    }

    pub fn insert(&mut self, msg: DisplayMessage) {
        let uuid = msg.uuid.clone();
        let seq = msg.global_seq;

        if let Some(thread_root) = &msg.thread_uuid.clone() {
            // It's a reply
            let replies = self.threads.entry(thread_root.clone()).or_default();
            if !replies.contains(&uuid) {
                replies.push(uuid.clone());
            }
        } else {
            // Top-level message - insert in global_seq order
            if !self.top_level.contains(&uuid) {
                let pos = self.top_level.iter().position(|u| {
                    self.by_uuid
                        .get(u)
                        .map(|m| m.global_seq > seq)
                        .unwrap_or(false)
                });
                match pos {
                    Some(i) => self.top_level.insert(i, uuid.clone()),
                    None => self.top_level.push(uuid.clone()),
                }
            }
        }

        // Track the lowest seq we've seen
        if seq > 0 && seq < self.low_water {
            self.low_water = seq;
        }

        self.by_uuid.insert(uuid, msg);
    }

    pub fn reply_count(&self, uuid: &str) -> usize {
        self.threads.get(uuid).map(|v| v.len()).unwrap_or(0)
    }
}

pub struct AppState {
    pub mode: Mode,
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    /// Direction of the multi-pane split layout
    pub pane_split: PaneSplit,
    pub command_bar: Option<CommandBarState>,
    pub modal: Option<ModalKind>,
    pub invite_modal: Option<InviteModalState>,
    pub search_modal: Option<SearchModalState>,
    pub contacts_modal: Option<ContactsModalState>,
    pub current_chat: Option<String>,
    pub chat_list: Vec<ChatSummary>,
    pub invitations: Vec<Invitation>,
    pub connection_status: ConnStatus,
    pub members: Vec<Member>,
    pub msg_store: TuiMessageStore,
    pub highlights: HashMap<String, Instant>,
    pub ws_tx: mpsc::UnboundedSender<String>,
    pub my_uuid: String,
    pub my_screenname: String,
    pub identity_dir: PathBuf,
    pub server_url: String,
    pub picker_state: ListState,
    /// Name cache: uuid -> screenname
    pub name_cache: HashMap<String, String>,
    /// Whether we have encryption keys for current chat
    pub has_keys: bool,
    /// Chat channel for current chat (handles crypto/storage)
    pub chat_channel: Option<ChatChannel>,
    /// Last acked seq
    pub last_acked: i64,
    /// Toast notification (message, expiry)
    pub toast: Option<(String, Instant)>,
    /// Local contacts database
    pub contact_store: Option<ContactStore>,
    /// Invite code to redeem (set by /join command, consumed by run loop)
    pub pending_redeem: Option<String>,
    /// When true, run loop spawns an HTTP leave-chat request
    pub pending_leave_chat: bool,
    /// (chat_uuid, user_uuid) to add as member
    pub pending_add_member: Option<(String, String)>,
    /// When true, run loop spawns an invite-list fetch
    pub pending_list_invites: bool,
}

impl AppState {
    pub fn new(
        my_uuid: String,
        my_screenname: String,
        identity_dir: PathBuf,
        server_url: String,
        ws_tx: mpsc::UnboundedSender<String>,
    ) -> Self {
        AppState {
            mode: Mode::ChatPicker,
            panes: vec![Pane::new()],
            active_pane: 0,
            pane_split: PaneSplit::Vertical,
            command_bar: None,
            modal: None,
            invite_modal: None,
            search_modal: None,
            contacts_modal: None,
            current_chat: None,
            chat_list: Vec::new(),
            invitations: Vec::new(),
            connection_status: ConnStatus::Connecting,
            members: Vec::new(),
            msg_store: TuiMessageStore::new(),
            highlights: HashMap::new(),
            ws_tx,
            my_uuid,
            my_screenname,
            identity_dir,
            server_url,
            picker_state: ListState::default(),
            name_cache: HashMap::new(),
            has_keys: false,
            chat_channel: None,
            last_acked: 0,
            toast: None,
            contact_store: None,
            pending_redeem: None,
            pending_leave_chat: false,
            pending_add_member: None,
            pending_list_invites: false,
        }
    }

    pub fn active_pane(&self) -> &Pane {
        &self.panes[self.active_pane]
    }

    pub fn active_pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active_pane]
    }

    pub fn move_selection(&mut self, delta: i32) {
        let rows = self.build_render_rows();
        let row_count = rows.len();
        if row_count == 0 {
            return;
        }
        let new_sel = (self.panes[self.active_pane].selected as i32 + delta)
            .clamp(0, row_count as i32 - 1) as usize;
        self.panes[self.active_pane].selected = new_sel;

        // Auto-focus input rows when navigated to
        if let Some(RenderRow::Input { thread_uuid, .. }) = rows.get(new_sel) {
            let target = match thread_uuid {
                Some(t) => InputTarget::Thread(t.clone()),
                None => InputTarget::MainChat,
            };
            self.panes[self.active_pane].editing = Some(target);
        } else {
            // Clear editing state when navigating away from an input row
            self.panes[self.active_pane].editing = None;
        }
    }

    pub fn expand_thread(&mut self) {
        let selected = self.panes[self.active_pane].selected;
        if let Some(uuid) = self.selected_collapsed_thread_uuid(selected) {
            self.msg_store.expanded.insert(uuid);
        }
    }

    pub fn collapse_thread(&mut self) {
        let selected = self.panes[self.active_pane].selected;
        if let Some(uuid) = self.selected_thread_root(selected) {
            self.msg_store.expanded.remove(&uuid);
        }
    }

    fn selected_collapsed_thread_uuid(&self, selected: usize) -> Option<String> {
        let rows = self.build_render_rows();
        rows.get(selected).and_then(|r| {
            if let RenderRow::CollapsedThread { uuid, .. } = r {
                Some(uuid.clone())
            } else {
                None
            }
        })
    }

    fn selected_thread_root(&self, selected: usize) -> Option<String> {
        let rows = self.build_render_rows();
        rows.get(selected).and_then(|r| match r {
            RenderRow::Message {
                thread_uuid: Some(root),
                ..
            } => Some(root.clone()),
            RenderRow::Message { uuid, .. } => {
                if self.msg_store.expanded.contains(uuid) {
                    Some(uuid.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    pub fn activate(&mut self) {
        let rows = self.build_render_rows();
        let selected = self.panes[self.active_pane].selected;
        match rows.get(selected) {
            Some(RenderRow::CollapsedThread { uuid, .. }) => {
                let uuid = uuid.clone();
                self.msg_store.expanded.insert(uuid);
            }
            Some(RenderRow::Message { uuid, thread_uuid, .. }) => {
                // Use the thread root uuid as the input target.
                // For top-level messages, that's the message itself.
                // For replies, use their thread_uuid parent.
                let root = thread_uuid.clone().unwrap_or_else(|| uuid.clone());
                self.msg_store.expanded.insert(root.clone());
                self.panes[self.active_pane].editing = Some(InputTarget::Thread(root));
            }
            Some(RenderRow::Input { thread_uuid, .. }) => {
                let target = match thread_uuid {
                    Some(t) => InputTarget::Thread(t.clone()),
                    None => InputTarget::MainChat,
                };
                self.panes[self.active_pane].editing = Some(target);
            }
            None => {}
        }
    }

    pub fn split_pane_vertical(&mut self) {
        self.pane_split = PaneSplit::Vertical;
        self.panes.push(Pane::new());
        self.active_pane = self.panes.len() - 1;
    }

    pub fn split_pane_horizontal(&mut self) {
        self.pane_split = PaneSplit::Horizontal;
        self.panes.push(Pane::new());
        self.active_pane = self.panes.len() - 1;
    }

    pub fn close_pane(&mut self) {
        if self.panes.len() > 1 {
            self.panes.remove(self.active_pane);
            self.active_pane = self.active_pane.min(self.panes.len() - 1);
        }
    }

    pub fn focus_pane(&mut self, delta: i32) {
        let len = self.panes.len() as i32;
        self.active_pane = ((self.active_pane as i32 + delta).rem_euclid(len)) as usize;
    }

    pub fn jump_to_latest(&mut self) {
        let row_count = self.render_row_count();
        if row_count > 0 {
            self.panes[self.active_pane].selected = row_count - 1;
        }
    }

    pub fn send_current_input(&mut self) {
        // Block sends while disconnected
        if !matches!(self.connection_status, ConnStatus::Connected) {
            self.toast = Some((
                "Cannot send: not connected".to_string(),
                Instant::now() + std::time::Duration::from_secs(3),
            ));
            return;
        }

        // Determine thread context before taking input
        let editing_target = self.active_pane().editing.clone();

        let pane = self.active_pane_mut();
        let text = match pane.take_input() {
            Some(t) => t,
            None => return,
        };

        let (thread_uuid, reply_to_uuid): (Option<String>, Option<String>) = match &editing_target {
            Some(InputTarget::Thread(root)) => (Some(root.clone()), None),
            _ => (None, None),
        };

        let chat = match &mut self.chat_channel {
            Some(c) => c,
            None => return,
        };

        let thread_ref = thread_uuid.as_deref();
        let reply_ref = reply_to_uuid.as_deref();

        match chat.send_message(&text, thread_ref, reply_ref) {
            Ok(frame) => {
                // Extract UUID and timestamp for optimistic display
                let msg_uuid = frame.payload["uuid"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let ts = frame.payload["ts"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();

                let _ = self.ws_tx.send(frame.encode());

                // Optimistically display the sent message immediately
                if !msg_uuid.is_empty() {
                    let msg = DisplayMessage {
                        uuid: msg_uuid.clone(),
                        sender_uuid: self.my_uuid.clone(),
                        sender_name: self.my_screenname.clone(),
                        body: text,
                        timestamp: ts,
                        global_seq: i64::MAX, // sort to end until server assigns real seq
                        thread_uuid: thread_uuid.clone(),
                        reply_to_uuid: reply_to_uuid.clone(),
                        pending: true,
                    };
                    self.highlights.insert(msg_uuid, Instant::now());
                    self.msg_store.insert(msg);
                    // Auto-scroll to the new message
                    let row_count = self.render_row_count();
                    let pane = &mut self.panes[self.active_pane];
                    pane.selected = row_count.saturating_sub(1);
                }
            }
            Err(e) => {
                eprintln!("send error: {}", e);
            }
        }
    }

    /// Rotate the group encryption key and distribute to all online members.
    /// Called after member removal to ensure forward secrecy.
    pub fn rotate_and_distribute_keys(&mut self) {
        let chat = match &mut self.chat_channel {
            Some(c) => c,
            None => return,
        };
        let crypto = match &mut chat.crypto {
            Some(c) => c,
            None => {
                self.toast = Some((
                    "Cannot rotate: no encryption keys".to_string(),
                    Instant::now() + std::time::Duration::from_secs(3),
                ));
                return;
            }
        };

        let new_epoch = match crypto.rotate_key() {
            Ok(e) => e,
            Err(e) => {
                self.toast = Some((
                    format!("Key rotation failed: {}", e),
                    Instant::now() + std::time::Duration::from_secs(4),
                ));
                return;
            }
        };

        // Distribute new key to all online members (except ourselves)
        let my_uuid = self.my_uuid.clone();
        let topic = format!("chat:{}", chat.chat_uuid);
        let mut distributed = 0u32;

        dlog!("rotate_and_distribute: {} total members", self.members.len());
        for member in &self.members {
            if member.uuid == my_uuid {
                continue;
            }
            dlog!("rotate_and_distribute: trying member {}", member.uuid);
            // Fetch peer Ed25519 key from local cache or server
            let peer_ed25519 = match chat.get_peer_ed25519(&member.uuid, true) {
                Ok(k) => k,
                Err(e) => {
                    dlog!("rotate_and_distribute: get_peer_ed25519({}) FAILED: {}", member.uuid, e);
                    continue;
                }
            };
            let peer_x25519 = match crate::crypto::identity::ed25519_to_x25519_public(&peer_ed25519)
            {
                Some(k) => k,
                None => continue,
            };

            if let Some(ref crypto) = chat.crypto {
                if let Ok(bundle) = crypto.prepare_key_bundle(&peer_x25519, false) {
                    let wire = crate::crypto::bundle_to_wire_payload(&bundle, &member.uuid);
                    let frame =
                        crate::channel::protocol::Frame::new(&topic, "key:distribute", wire);
                    let _ = self.ws_tx.send(frame.encode());
                    distributed += 1;
                }
            }
        }

        self.toast = Some((
            format!(
                "Keys rotated to epoch {} — distributed to {} member(s)",
                new_epoch, distributed
            ),
            Instant::now() + std::time::Duration::from_secs(4),
        ));
    }

    /// Returns our E2E key fingerprint (hex of Ed25519 + X25519 public key prefixes + epoch).
    pub fn my_key_fingerprint(&self) -> String {
        if let Some(ref ch) = self.chat_channel {
            if let Some(ref crypto) = ch.crypto {
                // Access identity through the ChatCrypto facade
                let id = crypto.identity();
                let ed_bytes = id.verifying_key().to_bytes();
                let x_bytes = id.x25519_public().to_bytes();
                let ed_hex: String = ed_bytes
                    .iter()
                    .take(4)
                    .map(|b| format!("{:02x}", b))
                    .collect();
                let x_hex: String = x_bytes
                    .iter()
                    .take(4)
                    .map(|b| format!("{:02x}", b))
                    .collect();
                return format!("ed:{} x:{} epoch:{}", ed_hex, x_hex, crypto.current_epoch());
            }
        }
        "none".to_string()
    }

    /// Re-decrypt all stored messages after receiving keys.
    /// Called when key:distribute arrives and we transition from no-keys to having keys.
    pub fn redecrypt_stored_messages(&mut self) {
        let chat = match &self.chat_channel {
            Some(c) => c,
            None => return,
        };
        let crypto = match &chat.crypto {
            Some(c) => c,
            None => return,
        };

        // Collect all stored ciphertext messages from the SQLite store
        let stored = chat.store.get_range(0, i64::MAX).unwrap_or_default();
        let decrypted = crypto.decrypt_stored_messages(&stored);

        // Update display messages with decrypted text
        for (sender_str, plaintext) in decrypted {
            for msg in self.msg_store.by_uuid.values_mut() {
                if msg.sender_uuid == sender_str && msg.body.starts_with('<') {
                    msg.body = plaintext.clone();
                }
            }
        }
    }

    /// Load older messages from SQLite into the in-memory store (scrollback).
    pub fn load_scrollback(&mut self) {
        if !self.msg_store.has_more_above {
            return;
        }

        let chat = match &self.chat_channel {
            Some(c) => c,
            None => return,
        };

        let before_seq = self.msg_store.low_water;
        let older = match chat.store.get_before(before_seq, 50) {
            Ok(msgs) => msgs,
            Err(_) => return,
        };

        if older.is_empty() {
            self.msg_store.has_more_above = false;
            return;
        }

        for payload in &older {
            let uuid = payload["uuid"].as_str().unwrap_or("").to_string();
            let sender_uuid = payload["sender_uuid"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let global_seq = payload["global_seq"].as_i64().unwrap_or(0);
            let thread_uuid = payload["thread_uuid"].as_str().map(|s| s.to_string());
            let reply_to_uuid = payload["reply_to_uuid"].as_str().map(|s| s.to_string());
            let timestamp = payload["ts"]
                .as_str()
                .or(payload["client_ts"].as_str())
                .unwrap_or("")
                .to_string();

            // Decrypt
            let body = if let Some(ref ch) = self.chat_channel {
                if let Some(ref crypto) = ch.crypto {
                    if let Ok(sender_id) = Uuid::parse_str(&sender_uuid) {
                        let ciphertext = payload["ciphertext"].as_str().unwrap_or("");
                        crypto
                            .decrypt(&sender_id, ciphertext)
                            .unwrap_or_else(|_| "<decrypt failed>".to_string())
                    } else {
                        "<invalid sender>".to_string()
                    }
                } else {
                    "<awaiting keys>".to_string()
                }
            } else {
                "<no channel>".to_string()
            };

            let sender_name = self
                .name_cache
                .get(&sender_uuid)
                .cloned()
                .unwrap_or_else(|| sender_uuid.chars().take(8).collect());

            let msg = DisplayMessage {
                uuid,
                sender_uuid,
                sender_name,
                body,
                timestamp,
                global_seq,
                thread_uuid,
                reply_to_uuid,
                pending: false,
            };

            self.msg_store.insert(msg);
        }
    }

    /// Seed in-memory store with recent messages from SQLite on chat join.
    fn seed_from_sqlite(&mut self) {
        let chat = match &self.chat_channel {
            Some(c) => c,
            None => return,
        };

        let recent = match chat.store.get_recent(100) {
            Ok(msgs) => msgs,
            Err(_) => return,
        };

        if recent.is_empty() {
            self.msg_store.has_more_above = false;
            return;
        }

        for payload in &recent {
            let uuid = payload["uuid"].as_str().unwrap_or("").to_string();
            let sender_uuid = payload["sender_uuid"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let global_seq = payload["global_seq"].as_i64().unwrap_or(0);
            let thread_uuid = payload["thread_uuid"].as_str().map(|s| s.to_string());
            let reply_to_uuid = payload["reply_to_uuid"].as_str().map(|s| s.to_string());
            let timestamp = payload["ts"]
                .as_str()
                .or(payload["client_ts"].as_str())
                .unwrap_or("")
                .to_string();

            let body = if let Some(ref ch) = self.chat_channel {
                if let Some(ref crypto) = ch.crypto {
                    if let Ok(sender_id) = Uuid::parse_str(&sender_uuid) {
                        let ciphertext = payload["ciphertext"].as_str().unwrap_or("");
                        crypto
                            .decrypt(&sender_id, ciphertext)
                            .unwrap_or_else(|_| "<decrypt failed>".to_string())
                    } else {
                        "<invalid sender>".to_string()
                    }
                } else {
                    "<awaiting keys>".to_string()
                }
            } else {
                "<no channel>".to_string()
            };

            let sender_name = self
                .name_cache
                .get(&sender_uuid)
                .cloned()
                .unwrap_or_else(|| sender_uuid.chars().take(8).collect());

            let msg = DisplayMessage {
                uuid,
                sender_uuid,
                sender_name,
                body,
                timestamp,
                global_seq,
                thread_uuid,
                reply_to_uuid,
                pending: false,
            };

            self.msg_store.insert(msg);
        }
    }

    pub fn join_chat(&mut self, chat_uuid: String) {
        let chat_dir = match crate::config::chat_dir(&chat_uuid) {
            Ok(d) => d,
            Err(e) => {
                self.toast = Some((
                    format!("Cannot determine data dir: {}", e),
                    Instant::now() + std::time::Duration::from_secs(4),
                ));
                return;
            }
        };
        let _ = std::fs::create_dir_all(&chat_dir);

        let store = match SqlMessageStore::open(&chat_uuid) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Cannot open message store: {}", e);
                return;
            }
        };

        let crypto = crate::crypto::ChatCrypto::load(&self.identity_dir, &chat_dir)
            .ok()
            .flatten();
        self.has_keys = crypto.is_some();

        let user_uuid = Uuid::parse_str(&self.my_uuid).unwrap_or_else(|_| Uuid::new_v4());

        let mut channel = ChatChannel::new(
            &chat_uuid,
            user_uuid,
            self.server_url.clone(),
            self.identity_dir.clone(),
            chat_dir,
            store,
            crypto,
        );

        let join_frame = channel.join_frame();
        let _ = self.ws_tx.send(join_frame.encode());

        self.current_chat = Some(chat_uuid.clone());
        self.chat_channel = Some(channel);
        self.msg_store.clear();
        self.members.clear();
        self.mode = Mode::Chat;
        self.last_acked = 0;

        // Seed display from local SQLite history
        self.seed_from_sqlite();

        // Request keys if we don't have them
        if !self.has_keys {
            dlog!("join_chat({}) — no keys, sending key:request", chat_uuid);
            if let Some(ref ch) = self.chat_channel {
                let req = ch.key_request_frame();
                let _ = self.ws_tx.send(req.encode());
            }
        } else {
            dlog!("join_chat({}) — already have keys", chat_uuid);
        }

        // Request message backfill from peers
        if let Some(ref ch) = self.chat_channel {
            let sync = ch.sync_request_frame();
            let _ = self.ws_tx.send(sync.encode());
        }
    }

    pub fn handle_frame(&mut self, frame: &Frame) {
        dlog!("frame: event={} topic={}", frame.event, frame.topic);
        match frame.event.as_str() {
            "msg:new" => self.handle_msg_new(&frame.payload),
            "msg:buffered" => {
                if let Some(msgs) = frame.payload["messages"].as_array() {
                    let msgs: Vec<_> = msgs.clone();
                    for msg in msgs {
                        let pseudo_frame = Frame::new(&frame.topic, "msg:new", msg);
                        self.handle_msg_new(&pseudo_frame.payload);
                    }
                }
            }
            "presence_state" => self.handle_presence(&frame.payload),
            "presence_diff" => self.handle_presence_diff(&frame.payload),
            "sync:push" => {
                if let Some(msgs) = frame.payload["messages"].as_array() {
                    let msgs: Vec<_> = msgs.clone();
                    for msg in msgs {
                        let pseudo_frame = Frame::new(&frame.topic, "msg:new", msg);
                        self.handle_msg_new(&pseudo_frame.payload);
                    }
                }
            }
            "member:removed" => {
                let removed_uuid = frame.payload["user_uuid"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if removed_uuid == self.my_uuid {
                    // We were removed from this chat
                    self.toast = Some((
                        "You were removed from this chat".to_string(),
                        Instant::now() + std::time::Duration::from_secs(5),
                    ));
                    self.current_chat = None;
                    self.chat_channel = None;
                    self.msg_store.clear();
                    self.members.clear();
                    self.mode = Mode::ChatPicker;
                } else {
                    // Another member was removed — rotate keys for forward secrecy
                    self.members.retain(|m| m.uuid != removed_uuid);
                    let name = self
                        .name_cache
                        .get(&removed_uuid)
                        .cloned()
                        .unwrap_or_else(|| removed_uuid.chars().take(8).collect());
                    self.toast = Some((
                        format!("{} was removed — rotating keys", name),
                        Instant::now() + std::time::Duration::from_secs(4),
                    ));
                    self.rotate_and_distribute_keys();
                }
            }
            "key:distribute" | "key:request" | "phx_reply" | "phx_error" | "sync:assign" => {
                // Delegate to ChatChannel for crypto/sync handling
                if let Some(ref mut ch) = self.chat_channel {
                    match ch.handle_incoming(frame) {
                        Ok(Some(reply)) => {
                            dlog!("handle_incoming({}) → sending reply frame", frame.event);
                            let _ = self.ws_tx.send(reply.encode());
                        }
                        Ok(None) => {
                            dlog!("handle_incoming({}) → no reply", frame.event);
                        }
                        Err(e) => {
                            dlog!("handle_incoming({}) → ERROR: {}", frame.event, e);
                            eprintln!("channel error: {}", e);
                        }
                    }
                    // Update has_keys; if we just got keys, re-decrypt stored messages
                    let had_keys = self.has_keys;
                    self.has_keys = ch.crypto.is_some();
                    if !had_keys && self.has_keys {
                        dlog!("has_keys changed false→true — redecrypting stored messages");
                        self.redecrypt_stored_messages();
                    }
                } else {
                    dlog!("handle_frame({}) — no chat_channel to delegate to!", frame.event);
                }
                // For sync:assign, build sync responses
                if frame.event == "sync:assign" {
                    if let Some(ref ch) = self.chat_channel {
                        let requester = frame.payload["requester"].as_str().unwrap_or("");
                        let from = frame.payload["from_seq"].as_i64().unwrap_or(0);
                        let to = frame.payload["to_seq"].as_i64().unwrap_or(0);
                        let topic = &frame.topic;
                        match crate::channel::sync::build_sync_responses(
                            &ch.store, requester, from, to, topic,
                        ) {
                            Ok(frames) => {
                                for f in frames {
                                    let _ = self.ws_tx.send(f.encode());
                                }
                            }
                            Err(e) => eprintln!("sync error: {}", e),
                        }
                    }
                }
            }
            _ => {}
        }

        // Send periodic ack
        if let Some(ref ch) = self.chat_channel {
            if ch.high_water > self.last_acked {
                let ack = ch.ack_frame();
                let _ = self.ws_tx.send(ack.encode());
                self.last_acked = ch.high_water;
            }
        }
    }

    fn handle_msg_new(&mut self, payload: &serde_json::Value) {
        let uuid = payload["uuid"].as_str().unwrap_or("").to_string();
        let sender_uuid = payload["sender_uuid"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let global_seq = payload["global_seq"].as_i64().unwrap_or(0);
        let thread_uuid = payload["thread_uuid"].as_str().map(|s| s.to_string());
        let reply_to_uuid = payload["reply_to_uuid"].as_str().map(|s| s.to_string());
        let timestamp = payload["ts"]
            .as_str()
            .or(payload["client_ts"].as_str())
            .unwrap_or("")
            .to_string();

        // First store in SQLite via ChatChannel
        if let Some(ref mut ch) = self.chat_channel {
            let _ = ch.store.insert_message(payload);
            if global_seq > ch.high_water {
                ch.high_water = global_seq;
            }
        }

        // Decrypt
        let body = if let Some(ref ch) = self.chat_channel {
            if let Some(ref crypto) = ch.crypto {
                if let Ok(sender_id) = Uuid::parse_str(&sender_uuid) {
                    let ciphertext = payload["ciphertext"].as_str().unwrap_or("");
                    crypto
                        .decrypt(&sender_id, ciphertext)
                        .unwrap_or_else(|_| "<decrypt failed>".to_string())
                } else {
                    "<invalid sender>".to_string()
                }
            } else {
                "<awaiting keys>".to_string()
            }
        } else {
            "<no channel>".to_string()
        };

        let sender_name = self
            .name_cache
            .get(&sender_uuid)
            .cloned()
            .unwrap_or_else(|| sender_uuid.chars().take(8).collect());

        // If this UUID was optimistically inserted as pending, confirm it
        if self.msg_store.by_uuid.contains_key(&uuid) {
            if let Some(existing) = self.msg_store.by_uuid.get_mut(&uuid) {
                existing.pending = false;
                existing.global_seq = global_seq;
                existing.body = body;
            }
            // Update position in top_level now that we have the real global_seq
            if let Some(thread_root) = &thread_uuid {
                // Reply — already in threads, just update seq
                let _ = thread_root;
            } else {
                self.msg_store.top_level.retain(|u| u != &uuid);
                let pos = self.msg_store.top_level.iter().position(|u| {
                    self.msg_store
                        .by_uuid
                        .get(u)
                        .map(|m| m.global_seq > global_seq)
                        .unwrap_or(false)
                });
                match pos {
                    Some(i) => self.msg_store.top_level.insert(i, uuid.clone()),
                    None => self.msg_store.top_level.push(uuid.clone()),
                }
            }
            if global_seq > 0 && global_seq < self.msg_store.low_water {
                self.msg_store.low_water = global_seq;
            }
            self.highlights.insert(uuid, Instant::now());
            return;
        }

        let msg = DisplayMessage {
            uuid: uuid.clone(),
            sender_uuid,
            sender_name,
            body,
            timestamp,
            global_seq,
            thread_uuid,
            reply_to_uuid,
            pending: false,
        };

        self.highlights.insert(uuid, Instant::now());
        self.msg_store.insert(msg);
    }

    fn handle_presence(&mut self, payload: &serde_json::Value) {
        self.members.clear();
        let chat_uuid = self
            .current_chat
            .as_ref()
            .and_then(|s| s.parse::<Uuid>().ok());
        if let Some(members) = payload["members"].as_array() {
            for m in members {
                let uuid_str = m["user_uuid"].as_str().unwrap_or("").to_string();
                let screenname = m["screenname"].as_str().unwrap_or("?").to_string();
                self.name_cache.insert(uuid_str.clone(), screenname.clone());
                if let (Ok(uuid), Some(ref cs)) = (uuid_str.parse::<Uuid>(), &self.contact_store) {
                    let _ = cs.upsert(uuid, &screenname, chat_uuid);
                }
                self.members.push(Member {
                    uuid: uuid_str,
                    screenname,
                    online: true,
                });
            }
        }
    }

    fn handle_presence_diff(&mut self, payload: &serde_json::Value) {
        let mut new_joins = Vec::new();

        // Collect leaving UUIDs first. When a user updates presence metadata,
        // Phoenix fires a diff with the same UUID in both leaves (old meta) and
        // joins (new meta).  Those are NOT new online arrivals and should not
        // trigger key distribution.
        let leaving_uuids: std::collections::HashSet<String> = payload["leaves"]
            .as_object()
            .map(|l| l.keys().cloned().collect())
            .unwrap_or_default();

        // Process leaves: mark as offline rather than removing entirely so the
        // member count stays accurate.  A metadata-update diff has the same UUID
        // in both leaves and joins — the join half will flip online back to true.
        if !leaving_uuids.is_empty() {
            for m in self.members.iter_mut() {
                if leaving_uuids.contains(&m.uuid) {
                    m.online = false;
                }
            }
        }

        // Handle joins
        if let Some(joins) = payload["joins"].as_object() {
            for (uuid, meta) in joins {
                let screenname = meta["metas"][0]["screenname"]
                    .as_str()
                    .or(meta["screenname"].as_str())
                    .unwrap_or("?")
                    .to_string();
                self.name_cache.insert(uuid.clone(), screenname.clone());
                if let Some(existing) = self.members.iter_mut().find(|m| &m.uuid == uuid) {
                    existing.online = true;
                    existing.screenname = screenname;
                } else {
                    self.members.push(Member {
                        uuid: uuid.clone(),
                        screenname,
                        online: true,
                    });
                }
                // Distribute keys to any peer newly arriving online.
                // Skip if it's ourselves, or if this is just a metadata update
                // (UUID also appears in leaves — same connection, different meta).
                if uuid != &self.my_uuid && !leaving_uuids.contains(uuid) {
                    new_joins.push(uuid.clone());
                }
            }
        }

        // Proactively distribute keys to newly joined members
        if !new_joins.is_empty() {
            dlog!("presence_diff: {} new join(s): {:?}", new_joins.len(), new_joins);
            if let Some(ref chat) = self.chat_channel {
                if chat.crypto.is_some() {
                    for uuid in &new_joins {
                        dlog!("presence_diff: distributing keys to new member {}", uuid);
                        match chat.get_peer_ed25519(uuid, true) {
                            Ok(peer_ed25519) => {
                                match crate::crypto::identity::ed25519_to_x25519_public(&peer_ed25519) {
                                    Some(peer_x25519) => {
                                        if let Some(ref crypto) = chat.crypto {
                                            match crypto.prepare_key_bundle(&peer_x25519, true) {
                                                Ok(bundle) => {
                                                    let wire = crate::crypto::bundle_to_wire_payload(
                                                        &bundle, uuid,
                                                    );
                                                    let frame = chat.frame("key:distribute", wire);
                                                    dlog!("presence_diff: sending key:distribute to {} (topic={})", uuid, chat.chat_uuid);
                                                    let _ = self.ws_tx.send(frame.encode());
                                                }
                                                Err(e) => dlog!("presence_diff: prepare_key_bundle FAILED for {}: {}", uuid, e),
                                            }
                                        } else {
                                            dlog!("presence_diff: crypto vanished mid-loop?!");
                                        }
                                    }
                                    None => dlog!("presence_diff: ed25519_to_x25519 returned None for {}", uuid),
                                }
                            }
                            Err(e) => dlog!("presence_diff: get_peer_ed25519({}) FAILED: {}", uuid, e),
                        }
                    }
                } else {
                    dlog!("presence_diff: we have no crypto, skipping key distribution");
                }
            } else {
                dlog!("presence_diff: no chat_channel, skipping key distribution");
            }
        }
    }

    pub fn tick(&mut self) {
        self.highlights
            .retain(|_, instant| instant.elapsed().as_millis() < 1000);
        if let Some((_, expires)) = &self.toast {
            if expires.elapsed().as_secs() > 0 {
                self.toast = None;
            }
        }
    }

    pub fn handle_chat_added(
        &mut self,
        chat_uuid: String,
        topic: String,
        invited_by: Option<String>,
    ) {
        // Only add if we don't already know about this chat
        if !self.chat_list.iter().any(|c| c.uuid == chat_uuid)
            && !self.invitations.iter().any(|i| i.chat_uuid == chat_uuid)
        {
            self.invitations.push(Invitation {
                chat_uuid,
                topic: topic.clone(),
                invited_by,
                received_at: chrono::Utc::now().timestamp(),
            });
            self.toast = Some((
                format!("New invitation: {}", topic),
                Instant::now() + std::time::Duration::from_secs(5),
            ));
        }
    }

    pub fn accept_invitation(&mut self, idx: usize) {
        if let Some(inv) = self.invitations.get(idx).cloned() {
            self.invitations.remove(idx);
            self.chat_list.push(ChatSummary {
                uuid: inv.chat_uuid.clone(),
                topic: inv.topic,
                member_count: 0,
            });
            self.join_chat(inv.chat_uuid);
        }
    }

    pub fn picker_select_prev(&mut self) {
        let len = self.chat_list.len();
        if len == 0 {
            return;
        }
        let selected = self.picker_state.selected().unwrap_or(0);
        self.picker_state
            .select(Some(if selected == 0 { len - 1 } else { selected - 1 }));
    }

    pub fn picker_select_next(&mut self) {
        let len = self.chat_list.len();
        if len == 0 {
            return;
        }
        let selected = self.picker_state.selected().unwrap_or(0);
        self.picker_state.select(Some((selected + 1) % len));
    }

    pub fn picker_join_selected(&mut self) {
        if let Some(idx) = self.picker_state.selected() {
            if let Some(chat) = self.chat_list.get(idx) {
                let uuid = chat.uuid.clone();
                self.join_chat(uuid);
            }
        }
    }

    pub fn render_row_count(&self) -> usize {
        self.build_render_rows().len()
    }

    /// Build the flat render row list from the current message store
    pub fn build_render_rows(&self) -> Vec<RenderRow> {
        let mut rows = Vec::new();

        for top_uuid in &self.msg_store.top_level {
            let msg = match self.msg_store.by_uuid.get(top_uuid) {
                Some(m) => m,
                None => continue,
            };

            let reply_count = self.msg_store.reply_count(top_uuid) as u32;
            let is_expanded = self.msg_store.expanded.contains(top_uuid);

            if reply_count > 0 && !is_expanded {
                // Collapsed thread
                let preview: String = msg.body.chars().take(40).collect();
                rows.push(RenderRow::CollapsedThread {
                    uuid: top_uuid.clone(),
                    author: msg.sender_name.clone(),
                    author_uuid: msg.sender_uuid.clone(),
                    preview,
                    reply_count,
                    timestamp: format_timestamp(&msg.timestamp),
                });
            } else {
                rows.push(RenderRow::Message {
                    uuid: top_uuid.clone(),
                    author: msg.sender_name.clone(),
                    author_uuid: msg.sender_uuid.clone(),
                    body: msg.body.clone(),
                    timestamp: format_timestamp(&msg.timestamp),
                    indent: 0,
                    thread_uuid: None,
                    reply_to_uuid: msg.reply_to_uuid.clone(),
                    is_mine: msg.sender_uuid == self.my_uuid,
                    is_pending: msg.pending,
                    highlight_age: self.highlights.get(top_uuid).map(|i| i.elapsed()),
                });

                // Thread replies if expanded
                if is_expanded {
                    let replies = self
                        .msg_store
                        .threads
                        .get(top_uuid)
                        .cloned()
                        .unwrap_or_default();
                    for reply_uuid in replies.iter() {
                        let reply = match self.msg_store.by_uuid.get(reply_uuid) {
                            Some(m) => m,
                            None => continue,
                        };
                        rows.push(RenderRow::Message {
                            uuid: reply_uuid.clone(),
                            author: reply.sender_name.clone(),
                            author_uuid: reply.sender_uuid.clone(),
                            body: reply.body.clone(),
                            timestamp: format_timestamp(&reply.timestamp),
                            indent: 1,
                            thread_uuid: Some(top_uuid.clone()),
                            reply_to_uuid: reply.reply_to_uuid.clone(),
                            is_mine: reply.sender_uuid == self.my_uuid,
                            is_pending: reply.pending,
                            highlight_age: self.highlights.get(reply_uuid).map(|i| i.elapsed()),
                        });
                    }
                    // Thread input prompt
                    let pane = self.active_pane();
                    let thread_input_target = InputTarget::Thread(top_uuid.clone());
                    let is_editing = pane.editing.as_ref() == Some(&thread_input_target);
                    rows.push(RenderRow::Input {
                        thread_uuid: Some(top_uuid.clone()),
                        indent: 1,
                        is_active: is_editing,
                        content: pane
                            .inputs
                            .get(&thread_input_target)
                            .cloned()
                            .unwrap_or_default(),
                    });
                }
            }
        }

        // Main chat input at bottom
        let is_editing_main = self.active_pane().editing.as_ref() == Some(&InputTarget::MainChat);
        rows.push(RenderRow::Input {
            thread_uuid: None,
            indent: 0,
            is_active: is_editing_main,
            content: self
                .active_pane()
                .inputs
                .get(&InputTarget::MainChat)
                .cloned()
                .unwrap_or_default(),
        });

        rows
    }
}

fn format_timestamp(ts: &str) -> String {
    if ts.len() >= 16 {
        ts[11..16].to_string() // HH:MM from ISO 8601
    } else {
        ts.to_string()
    }
}

#[derive(Debug, Clone)]
pub enum RenderRow {
    Message {
        uuid: String,
        author: String,
        author_uuid: String,
        body: String,
        timestamp: String,
        indent: u8,
        thread_uuid: Option<String>,
        reply_to_uuid: Option<String>,
        is_mine: bool,
        is_pending: bool,
        highlight_age: Option<std::time::Duration>,
    },
    CollapsedThread {
        uuid: String,
        author: String,
        author_uuid: String,
        preview: String,
        reply_count: u32,
        timestamp: String,
    },
    Input {
        thread_uuid: Option<String>,
        indent: u8,
        is_active: bool,
        content: String,
    },
}
