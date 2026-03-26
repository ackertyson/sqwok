use std::collections::HashMap;

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

/// In-memory display store for the TUI
pub struct TuiMessageStore {
    /// Top-level messages ordered by global_seq
    pub top_level: Vec<String>, // uuids
    /// All messages by UUID
    pub by_uuid: HashMap<String, DisplayMessage>,
    /// Thread children: thread_root_uuid -> [child uuids in order]
    pub threads: HashMap<String, Vec<String>>,
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
            low_water: i64::MAX,
            has_more_above: true,
        }
    }

    pub fn clear(&mut self) {
        self.top_level.clear();
        self.by_uuid.clear();
        self.threads.clear();
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
