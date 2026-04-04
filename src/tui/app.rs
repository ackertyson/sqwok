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

// Re-export so existing callers (e.g. views/chat.rs) don't need updating.
pub use super::render_rows::{Gutter, RenderRow};
pub use super::store::{DisplayMessage, TuiMessageStore};

// --- Member / ChatSummary / Invitation ---

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
    pub description: Option<String>,
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
    /// Maps chat UUID -> topic name for display
    pub chat_names: HashMap<String, String>,
}

impl ContactsModalState {
    pub fn new(contacts: Vec<crate::storage::contacts::Contact>) -> Self {
        ContactsModalState {
            contacts,
            selected: 0,
            filter: String::new(),
            chat_names: HashMap::new(),
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

pub enum ModalState {
    MemberList,
    GroupSettings,
    InviteCreate(InviteModalState),
    Search(SearchModalState),
    Contacts(ContactsModalState),
}

pub enum ConnStatus {
    Connected,
    Connecting,
    Disconnected { reason: String, since: Instant },
}

/// State for the `@mention` autocomplete popup.
#[derive(Debug)]
pub struct MentionState {
    /// What the user has typed after the `@` trigger.
    pub query: String,
    /// Members whose screenname starts with `query` (uuid, screenname).
    pub matches: Vec<(String, String)>,
    /// Index of the currently highlighted entry.
    pub selected: usize,
}

/// A mention that has been committed to an input buffer, tracking where
/// `@screenname` lives so we can substitute the wire tag on send.
#[derive(Debug, Clone)]
pub struct PendingMention {
    pub target: InputTarget,
    /// Char-index of the `@` in the input buffer at the time of insertion.
    pub at_char: usize,
    pub uuid: String,
    pub screenname: String,
}

/// How multiple panes are laid out on screen.
#[derive(Clone, Copy)]
pub enum PaneSplit {
    /// Vertical divider — panes side by side (left/right)
    Vertical,
    /// Horizontal divider — panes stacked (upper/lower)
    Horizontal,
}

pub struct AppState {
    pub mode: Mode,
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    /// Direction of the multi-pane split layout
    pub pane_split: PaneSplit,
    pub command_bar: Option<CommandBarState>,
    pub modal: Option<ModalState>,
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
    /// Debounce: when we last sent a typing:notify
    pub last_typing_notify: Option<Instant>,
    /// The editing target we last sent a typing:notify for (used to detect context switches)
    pub last_typing_target: Option<InputTarget>,
    /// Active typing indicators from peers: (thread_uuid, reply_to_uuid)
    pub typing_indicators: HashSet<(Option<String>, Option<String>)>,
    /// When the last typing:active was received (for expiry)
    pub typing_indicators_updated: Option<Instant>,
    /// When true, run loop spawns an invite-list fetch
    pub pending_list_invites: bool,
    /// Whether a peer scrollback request is in-flight
    pub scrollback_pending: bool,
    /// Rate-limit: last time a scrollback sync was requested
    pub last_scrollback_request: Option<Instant>,
    /// Active `@mention` autocomplete popup, if any.
    pub mention_popup: Option<MentionState>,
    /// Pending mentions committed to input buffers, consumed on send.
    pub pending_mentions: Vec<PendingMention>,
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
            scrollback_pending: false,
            last_scrollback_request: None,
            last_typing_notify: None,
            last_typing_target: None,
            typing_indicators: HashSet::new(),
            typing_indicators_updated: None,
            mention_popup: None,
            pending_mentions: Vec::new(),
        }
    }

    pub fn active_pane(&self) -> &Pane {
        &self.panes[self.active_pane]
    }

    pub fn active_pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active_pane]
    }

    /// Open the `@mention` autocomplete popup with all non-self members.
    pub fn open_mention_popup(&mut self) {
        let matches = self
            .members
            .iter()
            .filter(|m| m.uuid != self.my_uuid)
            .map(|m| (m.uuid.clone(), m.screenname.clone()))
            .collect();
        self.mention_popup = Some(MentionState {
            query: String::new(),
            matches,
            selected: 0,
        });
    }

    /// Filter the popup matches to members whose screenname starts with `query`.
    pub fn update_mention_query(&mut self, query: &str) {
        if let Some(ref mut popup) = self.mention_popup {
            popup.query = query.to_string();
            let q = query.to_lowercase();
            popup.matches = self
                .members
                .iter()
                .filter(|m| m.uuid != self.my_uuid && m.screenname.to_lowercase().starts_with(&q))
                .map(|m| (m.uuid.clone(), m.screenname.clone()))
                .collect();
            popup.selected = 0;
        }
    }

    /// Complete the selected mention: replace `@query` in the input with
    /// `@screenname ` and record a `PendingMention` for wire-format substitution
    /// on send.
    pub fn complete_mention(&mut self) {
        let popup = match self.mention_popup.take() {
            Some(p) => p,
            None => return,
        };
        let (uuid, screenname) = match popup.matches.get(popup.selected) {
            Some(m) => m.clone(),
            None => return,
        };

        // Delete `@query` back to (and including) the triggering `@`.
        let at_char = match self.active_pane_mut().delete_back_to_at() {
            Some((pos, _)) => pos,
            None => return,
        };

        // Insert the display form `@screenname ` at that position.
        let insertion = format!("@{} ", screenname);
        self.active_pane_mut().insert_str_at_cursor(&insertion);

        // Record the pending mention for wire-format substitution on send.
        let target = match self.active_pane().editing.clone() {
            Some(t) => t,
            None => return,
        };
        self.pending_mentions.push(PendingMention {
            target,
            at_char,
            uuid,
            screenname,
        });
    }

    /// Apply pending mentions for `target` to `text`, replacing `@screenname`
    /// spans with `<@uuid>screenname</@>` wire-format tags. Consumed mentions
    /// are removed from `self.pending_mentions`.
    fn apply_pending_mentions(&mut self, mut text: String, target: &InputTarget) -> String {
        // Partition: collect matching mentions, keep others in place.
        let mut remaining = Vec::new();
        let mut mentions = Vec::new();
        for pm in self.pending_mentions.drain(..) {
            if &pm.target == target {
                mentions.push(pm);
            } else {
                remaining.push(pm);
            }
        }
        self.pending_mentions = remaining;

        // Sort descending by position so earlier replacements don't shift later ones.
        mentions.sort_by(|a, b| b.at_char.cmp(&a.at_char));

        for pm in &mentions {
            let expected = format!("@{}", pm.screenname);
            let expected_len = expected.chars().count();
            let chars: Vec<char> = text.chars().collect();
            if pm.at_char + expected_len > chars.len() {
                continue;
            }
            let slice: String = chars[pm.at_char..pm.at_char + expected_len]
                .iter()
                .collect();
            if slice != expected {
                continue;
            }
            let byte_start: usize = chars[..pm.at_char].iter().map(|c| c.len_utf8()).sum();
            let byte_end: usize = chars[..pm.at_char + expected_len]
                .iter()
                .map(|c| c.len_utf8())
                .sum();
            let tag = format!("<@{}>{}</@>", pm.uuid, pm.screenname);
            text.replace_range(byte_start..byte_end, &tag);
        }
        text
    }

    fn show_toast(&mut self, msg: impl Into<String>, secs: u64) {
        self.toast = Some((
            msg.into(),
            Instant::now() + std::time::Duration::from_secs(secs),
        ));
    }

    pub fn maybe_send_typing_notify(&mut self) {
        let editing = match self.active_pane().editing.clone() {
            Some(e) => e,
            None => return,
        };

        // Bypass cooldown when this is a fresh or changed editing context so the
        // indicator appears immediately. Apply the 5s cooldown only within the
        // same continuous typing session.
        let same_target = self.last_typing_target.as_ref() == Some(&editing);
        if same_target {
            if let Some(last) = self.last_typing_notify {
                if last.elapsed().as_secs() < 5 {
                    return;
                }
            }
        }

        let (thread_uuid, reply_to_uuid) = editing.to_wire_uuids();

        let frame = match &self.chat_channel {
            Some(ch) => ch.typing_notify_frame(thread_uuid.as_deref(), reply_to_uuid.as_deref()),
            None => return,
        };

        let _ = self.ws_tx.send(frame.encode());
        self.last_typing_notify = Some(Instant::now());
        self.last_typing_target = Some(editing);
        dlog!(
            "typing:notify sent thread={:?} reply_to={:?}",
            thread_uuid,
            reply_to_uuid
        );
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

        // Mark message as read when the cursor lands on it
        let uuid_to_mark = if let Some(RenderRow::Message {
            uuid, is_unread, ..
        }) = rows.get(new_sel)
        {
            if *is_unread {
                Some(uuid.clone())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(ref uuid) = uuid_to_mark {
            if let Some(msg) = self.msg_store.by_uuid.get_mut(uuid) {
                msg.read = true;
            }
            if let Some(ref ch) = self.chat_channel {
                let _ = ch.store.mark_read(uuid);
            }
            self.update_title();
        }

        // Auto-focus input rows when navigated to
        if let Some(RenderRow::Input {
            thread_uuid,
            reply_to_uuid,
            ..
        }) = rows.get(new_sel)
        {
            let target = InputTarget::from_uuids(thread_uuid.as_ref(), reply_to_uuid.as_ref());
            self.panes[self.active_pane].editing = Some(target);
        } else {
            // Clear editing state when navigating away from an input row
            self.panes[self.active_pane].editing = None;
        }
    }

    pub fn expand_thread(&mut self) {
        let rows = self.build_render_rows();
        let selected = self.panes[self.active_pane].selected;

        // Find a collapsed top-level thread or a depth-1 message with a collapsed subthread.
        let expand_top = rows.get(selected).and_then(|r| {
            if let RenderRow::CollapsedThread { uuid, .. } = r {
                Some(uuid.clone())
            } else {
                None
            }
        });
        let expand_sub = rows.get(selected).and_then(|r| {
            if let RenderRow::Message {
                uuid,
                thread_uuid,
                reply_to_uuid,
                ..
            } = r
            {
                if thread_uuid.is_some()
                    && reply_to_uuid.is_none()
                    && self.panes[self.active_pane]
                        .collapsed_subs
                        .contains(uuid.as_str())
                {
                    return Some(uuid.clone());
                }
            }
            None
        });
        // Top-level message (no thread_uuid): right arrow starts/enters the thread.
        let start_thread = rows.get(selected).and_then(|r| {
            if let RenderRow::Message {
                uuid,
                thread_uuid: None,
                ..
            } = r
            {
                Some(uuid.clone())
            } else {
                None
            }
        });
        // Depth-2 message: right arrow acts like Enter (focus the depth-2 reply input).
        let start_reply = rows.get(selected).and_then(|r| {
            if let RenderRow::Message {
                thread_uuid: Some(root),
                reply_to_uuid: Some(parent),
                ..
            } = r
            {
                Some((parent.clone(), root.clone()))
            } else {
                None
            }
        });
        drop(rows);

        if let Some(uuid) = expand_top {
            // Has existing replies: just expand, let the user navigate in.
            self.panes[self.active_pane].expanded.insert(uuid);
        } else if let Some(uuid) = expand_sub {
            // Re-expand a collapsed subthread.
            self.panes[self.active_pane].collapsed_subs.remove(&uuid);
        } else if let Some(uuid) = start_thread {
            // No thread yet (or thread already expanded): expand and focus the thread input.
            self.panes[self.active_pane].expanded.insert(uuid.clone());
            self.panes[self.active_pane].editing = Some(InputTarget::Thread(uuid));
            self.scroll_to_input();
        } else if let Some((parent, root)) = start_reply {
            // Depth-2: mirror Enter behavior — focus the depth-2 reply input.
            self.panes[self.active_pane].expanded.insert(root.clone());
            self.panes[self.active_pane].editing = Some(InputTarget::Reply(parent, root));
            self.scroll_to_input();
        } else {
            // Depth-1 message with expanded subthread: initiate a depth-2 reply.
            self.reply_to_selected();
            self.scroll_to_input();
        }
    }

    pub fn collapse_thread(&mut self) {
        let rows = self.build_render_rows();
        let selected = self.panes[self.active_pane].selected;

        // Extract context from the selected row.
        let (uuid, thread_uuid, reply_to_uuid) = match rows.get(selected) {
            Some(RenderRow::Message {
                uuid,
                thread_uuid,
                reply_to_uuid,
                ..
            }) => (
                Some(uuid.clone()),
                thread_uuid.clone(),
                reply_to_uuid.clone(),
            ),
            Some(RenderRow::Input {
                thread_uuid,
                reply_to_uuid,
                ..
            }) => (None, thread_uuid.clone(), reply_to_uuid.clone()),
            _ => (None, None, None),
        };

        if let Some(parent_uuid) = reply_to_uuid {
            // Depth-2: collapse this subthread and navigate focus to the depth-1 parent.
            self.panes[self.active_pane]
                .collapsed_subs
                .insert(parent_uuid.clone());
            self.panes[self.active_pane].editing = None;
            if let Some(idx) = rows
                .iter()
                .position(|r| matches!(r, RenderRow::Message { uuid, .. } if uuid == &parent_uuid))
            {
                self.panes[self.active_pane].selected = idx;
            }
        } else if let Some(msg_uuid) = uuid {
            if self.panes[self.active_pane]
                .collapsed_subs
                .contains(msg_uuid.as_str())
            {
                // Depth-1 subthread already collapsed: collapse the entire top-level thread.
                if let Some(root) = thread_uuid {
                    self.panes[self.active_pane].expanded.remove(&root);
                }
            } else if thread_uuid.is_some() {
                // Depth-1 subthread is expanded: collapse just this subthread.
                self.panes[self.active_pane].collapsed_subs.insert(msg_uuid);
            } else if self.panes[self.active_pane]
                .expanded
                .contains(msg_uuid.as_str())
            {
                // Depth-0 expanded thread: collapse it and clear any thread editing state.
                self.panes[self.active_pane]
                    .expanded
                    .remove(msg_uuid.as_str());
                self.panes[self.active_pane].editing = None;
            }
        } else if let Some(root) = thread_uuid {
            // Thread input row: collapse the top-level thread.
            self.panes[self.active_pane].expanded.remove(&root);
            self.panes[self.active_pane].editing = None;
        }
    }

    pub fn activate(&mut self) {
        let rows = self.build_render_rows();
        let selected = self.panes[self.active_pane].selected;
        let changed = match rows.get(selected) {
            Some(RenderRow::CollapsedThread { .. }) => {
                // Depth-0: stay at depth-0, focus the main chat input.
                self.panes[self.active_pane].editing = Some(InputTarget::MainChat);
                true
            }
            Some(RenderRow::Message {
                uuid,
                thread_uuid,
                reply_to_uuid,
                ..
            }) => {
                let uuid = uuid.clone();
                let thread_uuid = thread_uuid.clone();
                let reply_to_uuid = reply_to_uuid.clone();
                if let Some(root) = &thread_uuid {
                    if let Some(parent) = reply_to_uuid {
                        // Depth-2 message: focus the depth-2 reply input for this parent.
                        let root = root.clone();
                        self.panes[self.active_pane].expanded.insert(root.clone());
                        self.panes[self.active_pane].editing =
                            Some(InputTarget::Reply(parent, root));
                    } else {
                        // Depth-1 message: focus the thread input at the bottom.
                        self.panes[self.active_pane].editing =
                            Some(InputTarget::Thread(root.clone()));
                    }
                } else {
                    // Depth-0 message: focus the main chat input.
                    let _ = uuid;
                    self.panes[self.active_pane].editing = Some(InputTarget::MainChat);
                }
                true
            }
            Some(RenderRow::Input {
                thread_uuid,
                reply_to_uuid,
                ..
            }) => {
                let target = match (thread_uuid, reply_to_uuid) {
                    (Some(thread), Some(reply)) => {
                        InputTarget::Reply(reply.clone(), thread.clone())
                    }
                    (Some(t), None) => InputTarget::Thread(t.clone()),
                    _ => InputTarget::MainChat,
                };
                self.panes[self.active_pane].editing = Some(target);
                true
            }
            Some(RenderRow::TypingIndicator { .. }) | None => false,
        };
        if changed {
            self.scroll_to_input();
        }
    }

    /// Scroll the viewport so the input row matching the current editing target is visible.
    fn scroll_to_input(&mut self) {
        let editing = match self.panes[self.active_pane].editing.clone() {
            Some(e) => e,
            None => return,
        };
        let rows = self.build_render_rows();
        let target_idx = rows.iter().position(|r| {
            if let RenderRow::Input {
                thread_uuid,
                reply_to_uuid,
                ..
            } = r
            {
                editing.matches_input_row(thread_uuid, reply_to_uuid)
            } else {
                false
            }
        });
        if let Some(idx) = target_idx {
            self.panes[self.active_pane].selected = idx;
        }
    }

    /// Initiate a depth-2 reply to the currently selected depth-1 message.
    pub fn reply_to_selected(&mut self) {
        let rows = self.build_render_rows();
        let selected = self.panes[self.active_pane].selected;
        if let Some(RenderRow::Message {
            uuid,
            thread_uuid: Some(root),
            ..
        }) = rows.get(selected)
        {
            if let Some(msg) = self.msg_store.by_uuid.get(uuid.as_str()) {
                if msg.reply_to_uuid.is_none() {
                    let root = root.clone();
                    let uuid = uuid.clone();
                    self.panes[self.active_pane].expanded.insert(root.clone());
                    self.panes[self.active_pane].editing = Some(InputTarget::Reply(uuid, root));
                }
            }
        }
    }

    pub fn split_pane_vertical(&mut self) {
        self.pane_split = PaneSplit::Vertical;
        let mut new_pane = self.panes[self.active_pane].clone();
        new_pane.editing = None;
        self.panes.push(new_pane);
        self.active_pane = self.panes.len() - 1;
    }

    pub fn split_pane_horizontal(&mut self) {
        self.pane_split = PaneSplit::Horizontal;
        let mut new_pane = self.panes[self.active_pane].clone();
        new_pane.editing = None;
        self.panes.push(new_pane);
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

    /// Save the UUID of the currently-selected message to disk so it can be
    /// restored the next time this chat is opened.
    pub fn save_scroll_position(&self) {
        let chat_uuid = match &self.current_chat {
            Some(u) => u.clone(),
            None => return,
        };
        let cs = match &self.contact_store {
            Some(cs) => cs,
            None => return,
        };
        let rows = self.build_render_rows();
        let sel = self.panes[self.active_pane]
            .selected
            .min(rows.len().saturating_sub(1));
        if let Some(RenderRow::Message { uuid, .. }) = rows.get(sel) {
            let _ = cs.save_scroll(&chat_uuid, uuid);
        }
    }

    /// Restore scroll position after loading messages for a chat. Finds the
    /// saved message UUID in the current render rows and sets `selected`; falls
    /// back to `jump_to_latest` if the message isn't in the loaded window.
    fn restore_scroll_position(&mut self) {
        let chat_uuid = match &self.current_chat {
            Some(u) => u.clone(),
            None => return,
        };
        let saved_uuid = match &self.contact_store {
            Some(cs) => cs.load_scroll(&chat_uuid).ok().flatten(),
            None => None,
        };
        let saved_uuid = match saved_uuid {
            Some(u) => u,
            None => {
                self.jump_to_latest();
                return;
            }
        };
        let rows = self.build_render_rows();
        let idx = rows
            .iter()
            .position(|r| matches!(r, RenderRow::Message { uuid, .. } if uuid == &saved_uuid));
        match idx {
            Some(i) => self.panes[self.active_pane].selected = i,
            None => self.jump_to_latest(),
        }
    }

    pub fn send_current_input(&mut self) {
        // Block sends while disconnected
        if !matches!(self.connection_status, ConnStatus::Connected) {
            self.show_toast("Cannot send: not connected", 3);
            return;
        }

        // Determine thread context before taking input
        let editing_target = self.active_pane().editing.clone();

        let pane = self.active_pane_mut();
        let display_text = match pane.take_input() {
            Some(t) => t,
            None => return,
        };

        // Close any open mention popup and substitute wire-format tags.
        self.mention_popup = None;
        let target_for_mentions = editing_target.clone().unwrap_or(InputTarget::MainChat);
        let wire_text = self.apply_pending_mentions(display_text, &target_for_mentions);

        let (thread_uuid, reply_to_uuid) = editing_target
            .as_ref()
            .map(|t| t.to_wire_uuids())
            .unwrap_or((None, None));

        let chat = match &mut self.chat_channel {
            Some(c) => c,
            None => return,
        };

        let thread_ref = thread_uuid.as_deref();
        let reply_ref = reply_to_uuid.as_deref();

        match chat.send_message(&wire_text, thread_ref, reply_ref) {
            Ok(frame) => {
                // Extract UUID and timestamp for optimistic display
                let msg_uuid = frame.payload["uuid"].as_str().unwrap_or("").to_string();
                let ts = frame.payload["ts"].as_str().unwrap_or("").to_string();

                let _ = self.ws_tx.send(frame.encode());

                // Optimistically display the sent message immediately (render tags → @name).
                let mentioned_names_opt =
                    super::mention::extract_mentioned_names(&wire_text, &self.name_cache);
                let display_body = super::mention::render_body(&wire_text, &self.name_cache);
                if !msg_uuid.is_empty() {
                    let msg = DisplayMessage {
                        uuid: msg_uuid.clone(),
                        sender_uuid: self.my_uuid.clone(),
                        sender_name: self.my_screenname.clone(),
                        body: display_body,
                        timestamp: ts,
                        global_seq: i64::MAX, // sort to end until server assigns real seq
                        thread_uuid: thread_uuid.clone(),
                        reply_to_uuid: reply_to_uuid.clone(),
                        pending: true,
                        read: true,
                        mentions_me: false, // own messages are never self-mentions
                        mentioned_names: mentioned_names_opt,
                    };
                    self.highlights.insert(msg_uuid, Instant::now());
                    self.msg_store.insert(msg);
                    // Auto-scroll: stay in the current thread context after sending.
                    // Depth-2 reply inputs are only rendered while editing, so we must
                    // restore the editing target before rebuilding rows; otherwise the
                    // Input row won't appear in the list and the lookup falls back to the
                    // bottom of the main chat. Depth-1 thread inputs always render, so
                    // this is a no-op for that case (editing was already cleared by take_input).
                    if matches!(&editing_target, Some(InputTarget::Reply(..))) {
                        self.panes[self.active_pane].editing = editing_target.clone();
                    }
                    // Find the input row matching the context we just sent from.
                    let rows = self.build_render_rows();
                    let effective_target = editing_target
                        .as_ref()
                        .cloned()
                        .unwrap_or(InputTarget::MainChat);
                    let target_idx = rows.iter().position(|r| {
                        if let RenderRow::Input {
                            thread_uuid: row_thread,
                            reply_to_uuid: row_reply,
                            ..
                        } = r
                        {
                            effective_target.matches_input_row(row_thread, row_reply)
                        } else {
                            false
                        }
                    });
                    let pane = &mut self.panes[self.active_pane];
                    pane.selected = target_idx.unwrap_or_else(|| rows.len().saturating_sub(1));
                }
            }
            Err(e) => {
                dlog!("send error: {}", e);
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
                self.show_toast("Cannot rotate: no encryption keys", 3);
                return;
            }
        };

        let new_epoch = match crypto.rotate_key() {
            Ok(e) => e,
            Err(e) => {
                self.show_toast(format!("Key rotation failed: {}", e), 4);
                return;
            }
        };

        // Distribute new key to all online members (except ourselves)
        let my_uuid = self.my_uuid.clone();
        let topic = format!("chat:{}", chat.chat_uuid);
        let mut distributed = 0u32;

        dlog!(
            "rotate_and_distribute: {} total members",
            self.members.len()
        );
        for member in &self.members {
            if member.uuid == my_uuid {
                continue;
            }
            dlog!("rotate_and_distribute: trying member {}", member.uuid);
            // Fetch peer Ed25519 key from local cache or server
            let peer_ed25519 = match chat.get_peer_ed25519(&member.uuid, true) {
                Ok(k) => k,
                Err(e) => {
                    dlog!(
                        "rotate_and_distribute: get_peer_ed25519({}) FAILED: {}",
                        member.uuid,
                        e
                    );
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

        self.show_toast(
            format!(
                "Keys rotated to epoch {} — distributed to {} member(s)",
                new_epoch, distributed
            ),
            4,
        );
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
                let epoch = crypto
                    .current_epoch()
                    .map_or("none".into(), |e| e.to_string());
                return format!("ed:{} x:{} epoch:{}", ed_hex, x_hex, epoch);
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

        // Collect all stored ciphertext messages from SQLite.
        let stored = chat.store.get_range(0, i64::MAX).unwrap_or_default();

        // Build a uuid -> plaintext map so each message is decrypted individually.
        let mut decrypted_by_uuid: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in &stored {
            let uuid = match msg["uuid"].as_str() {
                Some(u) => u.to_string(),
                None => continue,
            };
            let sender_str = match msg["sender_uuid"].as_str() {
                Some(s) => s,
                None => continue,
            };
            let ct = match msg["ciphertext"].as_str() {
                Some(c) => c,
                None => continue,
            };
            if let Ok(sender_id) = Uuid::parse_str(sender_str) {
                if let Ok(plaintext) = crypto.decrypt(&sender_id, ct) {
                    decrypted_by_uuid.insert(uuid, plaintext);
                }
            }
        }

        // Update in-memory display messages by UUID (not by sender).
        let my_uuid = self.my_uuid.clone();
        let name_cache = self.name_cache.clone();
        for (uuid, plaintext) in decrypted_by_uuid {
            if let Some(msg) = self.msg_store.by_uuid.get_mut(&uuid) {
                msg.mentions_me = super::mention::mentions_user(&plaintext, &my_uuid);
                msg.mentioned_names =
                    super::mention::extract_mentioned_names(&plaintext, &name_cache);
                msg.body = super::mention::render_body(&plaintext, &name_cache);
            }
        }
    }

    /// Parse a JSON message payload into a DisplayMessage, handling decryption
    /// and sender name resolution.
    fn parse_message_payload(&self, payload: &serde_json::Value) -> DisplayMessage {
        let uuid = payload["uuid"].as_str().unwrap_or("").to_string();
        let sender_uuid = payload["sender_uuid"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let global_seq = payload["global_seq"].as_i64().unwrap_or(0);
        // Client-side thread validation: discard refs that don't resolve locally
        let thread_uuid = payload["thread_uuid"]
            .as_str()
            .filter(|u| self.msg_store.by_uuid.contains_key(*u))
            .map(|s| s.to_string());
        let reply_to_uuid = payload["reply_to_uuid"]
            .as_str()
            .filter(|u| thread_uuid.is_some() && self.msg_store.by_uuid.contains_key(*u))
            .map(|s| s.to_string());
        let timestamp = payload["ts"]
            .as_str()
            .or(payload["client_ts"].as_str())
            .unwrap_or("")
            .to_string();

        let raw_body = if let Some(ref ch) = self.chat_channel {
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

        let mentions_me = super::mention::mentions_user(&raw_body, &self.my_uuid);
        let mentioned_names = super::mention::extract_mentioned_names(&raw_body, &self.name_cache);
        let body = super::mention::render_body(&raw_body, &self.name_cache);

        let sender_name = self
            .name_cache
            .get(&sender_uuid)
            .cloned()
            .unwrap_or_else(|| sender_uuid.chars().take(8).collect());

        let read = payload["read"]
            .as_i64()
            .map(|v| v != 0)
            .unwrap_or_else(|| sender_uuid == self.my_uuid);

        DisplayMessage {
            uuid,
            sender_uuid,
            sender_name,
            body,
            timestamp,
            global_seq,
            thread_uuid,
            reply_to_uuid,
            pending: false,
            read,
            mentions_me,
            mentioned_names,
        }
    }

    /// Load older messages from SQLite into the in-memory store (scrollback).
    /// When local storage is exhausted, requests older messages from peers.
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
            // Local SQLite has no older messages — request from peers
            if before_seq > 1 && !self.scrollback_pending {
                let rate_ok = self
                    .last_scrollback_request
                    .map(|t| t.elapsed() >= std::time::Duration::from_secs(2))
                    .unwrap_or(true);
                if rate_ok {
                    if let Some(ref ch) = self.chat_channel {
                        let frame = ch.sync_scrollback_frame(before_seq, 50);
                        let _ = self.ws_tx.send(frame.encode());
                        self.scrollback_pending = true;
                        self.last_scrollback_request = Some(Instant::now());
                    }
                }
            }
            return;
        }

        for payload in &older {
            let msg = self.parse_message_payload(payload);
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
            let msg = self.parse_message_payload(payload);
            self.msg_store.insert(msg);
        }
    }

    pub fn clear_chat_state(&mut self) {
        self.msg_store.clear();
        self.members.clear();
        self.scrollback_pending = false;
        self.last_scrollback_request = None;
        for pane in &mut self.panes {
            pane.clear_view_state();
        }
    }

    pub fn join_chat(&mut self, chat_uuid: String) {
        let chat_dir = match crate::config::chat_dir(&chat_uuid) {
            Ok(d) => d,
            Err(e) => {
                self.show_toast(format!("Cannot determine data dir: {}", e), 4);
                return;
            }
        };
        let _ = std::fs::create_dir_all(&chat_dir);

        let store = match SqlMessageStore::open(&chat_uuid) {
            Ok(s) => s,
            Err(e) => {
                dlog!("Cannot open message store: {}", e);
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

        self.save_scroll_position();
        self.current_chat = Some(chat_uuid.clone());
        self.chat_channel = Some(channel);
        self.clear_chat_state();
        self.mode = Mode::Chat;
        self.last_acked = 0;
        self.typing_indicators.clear();
        self.typing_indicators_updated = None;
        self.last_typing_notify = None;
        self.last_typing_target = None;

        // Seed display from local SQLite history and restore last scroll position
        self.seed_from_sqlite();
        self.restore_scroll_position();

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

        // Request message catchup from peers
        if let Some(ref ch) = self.chat_channel {
            dlog!(
                "[SYNC] requesting sync:catchup with high_water={}",
                ch.high_water
            );
            let sync = ch.sync_catchup_frame();
            let _ = self.ws_tx.send(sync.encode());
        }
    }

    pub fn handle_frame(&mut self, frame: &Frame) {
        dlog!("frame: event={} topic={}", frame.event, frame.topic);
        match frame.event.as_str() {
            "msg:new" => self.handle_msg_new(&frame.payload),
            "member_list" => self.handle_member_list(&frame.payload),
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
                self.scrollback_pending = false;
            }
            "member:removed" => {
                let removed_uuid = frame.payload["user_uuid"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                if removed_uuid == self.my_uuid {
                    // We were removed from this chat
                    self.show_toast("You were removed from this chat", 5);
                    self.current_chat = None;
                    self.chat_channel = None;
                    self.clear_chat_state();
                    self.mode = Mode::ChatPicker;
                } else {
                    // Another member was removed — rotate keys for forward secrecy
                    self.members.retain(|m| m.uuid != removed_uuid);
                    let name = self
                        .name_cache
                        .get(&removed_uuid)
                        .cloned()
                        .unwrap_or_else(|| removed_uuid.chars().take(8).collect());
                    self.show_toast(format!("{} was removed — rotating keys", name), 4);
                    self.rotate_and_distribute_keys();
                }
            }
            "key:distribute" | "key:request" | "phx_reply" | "phx_error" | "sync:assign"
            | "sync:query" => {
                if frame.event == "sync:query" {
                    dlog!(
                        "[SYNC] sync:query arrived in frame dispatch, payload={}",
                        frame.payload
                    );
                }
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
                    dlog!(
                        "handle_frame({}) — no chat_channel to delegate to!",
                        frame.event
                    );
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
                                dlog!(
                                    "[SYNC] sync:assign: requester={} from={} to={} → built {} frames",
                                    requester, from, to, frames.len()
                                );
                                for mut f in frames {
                                    f.join_ref = ch.join_ref.clone();
                                    let _ = self.ws_tx.send(f.encode());
                                }
                            }
                            Err(e) => dlog!("sync error: {}", e),
                        }
                    }
                }
            }
            "typing:active" => {
                if let Some(entries) = frame.payload["entries"].as_array() {
                    let my_uuid = self.my_uuid.clone();
                    self.typing_indicators = entries
                        .iter()
                        .filter(|e| e["user_uuid"].as_str().unwrap_or("") != my_uuid.as_str())
                        .map(|e| {
                            let thread = e["thread_uuid"].as_str().map(|s| s.to_string());
                            let reply = e["reply_to_uuid"].as_str().map(|s| s.to_string());
                            (thread, reply)
                        })
                        .collect();
                    self.typing_indicators_updated = Some(Instant::now());
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
        let global_seq = payload["global_seq"].as_i64().unwrap_or(0);

        // First store in SQLite via ChatChannel
        if let Some(ref mut ch) = self.chat_channel {
            let _ = ch.store.insert_message(payload);
            if global_seq > ch.high_water {
                ch.high_water = global_seq;
            }
        }

        let msg = self.parse_message_payload(payload);
        let uuid = msg.uuid.clone();
        let is_own = msg.read;

        // If this UUID was optimistically inserted as pending, confirm it
        if let Some(existing) = self.msg_store.by_uuid.get_mut(&uuid) {
            existing.pending = false;
            existing.global_seq = msg.global_seq;
            existing.body = msg.body;

            // Persist read=1 for own messages (SQLite defaults to 0 on insert)
            if is_own {
                if let Some(ref ch) = self.chat_channel {
                    let _ = ch.store.mark_read(&uuid);
                }
            }

            // Update position in top_level now that we have the real global_seq
            if msg.thread_uuid.is_none() {
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

        // Persist read=1 for own messages arriving from another session
        if is_own {
            if let Some(ref ch) = self.chat_channel {
                let _ = ch.store.mark_read(&uuid);
            }
        }
        self.highlights.insert(uuid, Instant::now());
        self.msg_store.insert(msg);
        self.update_title();
    }

    /// Handle `member_list` — the full chat roster from the database.
    /// All members start as offline; `presence_state` and `presence_diff`
    /// flip the actually-online ones.
    fn handle_member_list(&mut self, payload: &serde_json::Value) {
        self.members.clear();
        let chat_uuid = self
            .current_chat
            .as_ref()
            .and_then(|s| s.parse::<Uuid>().ok());
        if let Some(members) = payload["members"].as_array() {
            for m in members {
                let uuid_str = m["user_uuid"].as_str().unwrap_or("").to_string();
                let screenname = if uuid_str == self.my_uuid {
                    self.my_screenname.clone()
                } else {
                    m["screenname"].as_str().unwrap_or("?").to_string()
                };
                self.name_cache.insert(uuid_str.clone(), screenname.clone());
                if uuid_str != self.my_uuid {
                    if let (Ok(uuid), Some(ref cs)) =
                        (uuid_str.parse::<Uuid>(), &self.contact_store)
                    {
                        let _ = cs.upsert(uuid, &screenname, chat_uuid);
                    }
                }
                self.members.push(Member {
                    uuid: uuid_str,
                    screenname,
                    online: false,
                });
            }
        }
        self.update_title();
    }

    /// Handle `presence_state` — Phoenix Presence list of currently-online users.
    /// Payload is a map of `{uuid: {metas: [{screenname, ...}]}}`.
    fn handle_presence(&mut self, payload: &serde_json::Value) {
        if let Some(obj) = payload.as_object() {
            for (uuid, meta) in obj {
                // Skip keys that aren't UUIDs (e.g. stale server sending
                // the old {members: [...]} format under this event name).
                if Uuid::parse_str(uuid).is_err() {
                    continue;
                }
                if let Some(existing) = self.members.iter_mut().find(|m| &m.uuid == uuid) {
                    existing.online = true;
                } else {
                    let screenname = if uuid == &self.my_uuid {
                        self.my_screenname.clone()
                    } else {
                        meta["metas"][0]["screenname"]
                            .as_str()
                            .unwrap_or("?")
                            .to_string()
                    };
                    self.members.push(Member {
                        uuid: uuid.clone(),
                        screenname,
                        online: true,
                    });
                }
            }
        }
        self.update_title();
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
                let screenname = if uuid == &self.my_uuid {
                    self.my_screenname.clone()
                } else {
                    meta["metas"][0]["screenname"]
                        .as_str()
                        .or(meta["screenname"].as_str())
                        .unwrap_or("?")
                        .to_string()
                };
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
            dlog!(
                "presence_diff: {} new join(s): {:?}",
                new_joins.len(),
                new_joins
            );
            if let Some(ref chat) = self.chat_channel {
                if chat.crypto.is_some() {
                    for uuid in &new_joins {
                        dlog!("presence_diff: distributing keys to new member {}", uuid);
                        match chat.get_peer_ed25519(uuid, true) {
                            Ok(peer_ed25519) => {
                                match crate::crypto::identity::ed25519_to_x25519_public(
                                    &peer_ed25519,
                                ) {
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
                                    None => dlog!(
                                        "presence_diff: ed25519_to_x25519 returned None for {}",
                                        uuid
                                    ),
                                }
                            }
                            Err(e) => {
                                dlog!("presence_diff: get_peer_ed25519({}) FAILED: {}", uuid, e)
                            }
                        }
                    }
                } else {
                    dlog!("presence_diff: we have no crypto, skipping key distribution");
                }
            } else {
                dlog!("presence_diff: no chat_channel, skipping key distribution");
            }
        }
        self.update_title();
    }

    pub fn tick(&mut self) {
        self.highlights
            .retain(|_, instant| instant.elapsed().as_millis() < 1000);
        if let Some((_, expires)) = &self.toast {
            if expires.elapsed().as_secs() > 0 {
                self.toast = None;
            }
        }
        if let Some(updated) = self.typing_indicators_updated {
            if updated.elapsed().as_secs() >= 7 {
                self.typing_indicators.clear();
                self.typing_indicators_updated = None;
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
            self.show_toast(format!("New invitation: {}", topic), 5);
        }
    }

    pub fn accept_invitation(&mut self, idx: usize) {
        if let Some(inv) = self.invitations.get(idx).cloned() {
            self.invitations.remove(idx);
            self.chat_list.push(ChatSummary {
                uuid: inv.chat_uuid.clone(),
                topic: inv.topic,
                description: None,
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

    /// Build the flat render row list from the current message store using the active pane.
    pub fn build_render_rows(&self) -> Vec<RenderRow> {
        let pane = self.active_pane().clone();
        self.build_render_rows_for_pane(&pane)
    }

    /// Build the flat render row list using a specific pane's editing/input state.
    /// This allows each pane to render its own independent editing state.
    pub fn build_render_rows_for_pane(&self, pane: &Pane) -> Vec<RenderRow> {
        let member_order: Vec<&str> = self.members.iter().map(|m| m.uuid.as_str()).collect();
        super::render_rows::build(
            pane,
            &self.msg_store,
            &self.my_uuid,
            &member_order,
            &self.name_cache,
            &self.highlights,
            &self.typing_indicators,
        )
    }

    /// Number of online peers (excludes self).
    pub fn online_count(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.online && m.uuid != self.my_uuid)
            .count()
    }

    /// Compute and set the terminal title reflecting unread count and online members.
    pub fn update_title(&self) {
        let unread = self.msg_store.by_uuid.values().filter(|m| !m.read).count();
        let online = self.online_count();
        let mut title = String::new();
        if unread > 0 {
            title.push_str(&format!("[{} new]", unread));
        }
        if online > 0 {
            title.push_str(&format!("[{} online]", online));
        }
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str("sqwok");
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(title));
    }
}
