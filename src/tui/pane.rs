use std::collections::{HashMap, HashSet};

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum InputTarget {
    MainChat,
    Thread(String),        // thread root uuid
    Reply(String, String), // (reply_to_uuid, thread_root_uuid)
}

impl InputTarget {
    /// Convert a (thread_uuid, reply_to_uuid) pair from a render row into an InputTarget.
    pub fn from_uuids(thread_uuid: Option<&String>, reply_to_uuid: Option<&String>) -> Self {
        match (thread_uuid, reply_to_uuid) {
            (Some(thread), Some(reply)) => InputTarget::Reply(reply.clone(), thread.clone()),
            (Some(t), None) => InputTarget::Thread(t.clone()),
            _ => InputTarget::MainChat,
        }
    }

    /// Check whether this target corresponds to the given Input row's uuid fields.
    pub fn matches_input_row(
        &self,
        thread_uuid: &Option<String>,
        reply_to_uuid: &Option<String>,
    ) -> bool {
        match self {
            InputTarget::MainChat => thread_uuid.is_none() && reply_to_uuid.is_none(),
            InputTarget::Thread(root) => {
                thread_uuid.as_deref() == Some(root.as_str()) && reply_to_uuid.is_none()
            }
            InputTarget::Reply(reply_uuid, thread) => {
                thread_uuid.as_deref() == Some(thread.as_str())
                    && reply_to_uuid.as_deref() == Some(reply_uuid.as_str())
            }
        }
    }

    /// Extract the (thread_uuid, reply_to_uuid) wire fields for this target.
    pub fn to_wire_uuids(&self) -> (Option<String>, Option<String>) {
        match self {
            InputTarget::MainChat => (None, None),
            InputTarget::Thread(root) => (Some(root.clone()), None),
            InputTarget::Reply(reply_uuid, thread) => {
                (Some(thread.clone()), Some(reply_uuid.clone()))
            }
        }
    }

    /// The visual indent depth for the input row at this target.
    pub fn indent(&self) -> u8 {
        match self {
            InputTarget::MainChat => 0,
            InputTarget::Thread(_) => 1,
            InputTarget::Reply(_, _) => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Pane {
    /// Index of selected render row
    pub selected: usize,
    /// Currently editing an input (None = navigation mode)
    pub editing: Option<InputTarget>,
    /// Input field contents by target
    pub inputs: HashMap<InputTarget, String>,
    /// Cursor char-index within each input field
    pub cursor_positions: HashMap<InputTarget, usize>,
    /// Which threads are expanded in this pane
    pub expanded: HashSet<String>,
    /// Which depth-1 messages have their depth-2 replies collapsed in this pane
    pub collapsed_subs: HashSet<String>,
}

impl Pane {
    pub fn new() -> Self {
        Pane {
            selected: 0,
            editing: None,
            inputs: HashMap::new(),
            cursor_positions: HashMap::new(),
            expanded: HashSet::new(),
            collapsed_subs: HashSet::new(),
        }
    }

    pub fn clear_view_state(&mut self) {
        self.expanded.clear();
        self.collapsed_subs.clear();
        self.selected = 0;
        self.editing = None;
        self.inputs.clear();
        self.cursor_positions.clear();
    }

    pub fn push_char(&mut self, c: char) {
        // Reject control characters (except common whitespace) and enforce a
        // reasonable message length cap to prevent runaway input.
        if c.is_control() && c != '\t' {
            return;
        }
        if let Some(target) = self.editing.clone() {
            let s = self.inputs.entry(target.clone()).or_default();
            if s.len() < 4096 {
                let cursor = self
                    .cursor_positions
                    .entry(target)
                    .or_insert(s.chars().count());
                let byte_idx = s
                    .char_indices()
                    .nth(*cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                s.insert(byte_idx, c);
                *cursor += 1;
            }
        }
    }

    pub fn pop_char(&mut self) {
        if let Some(target) = self.editing.clone() {
            if let Some(s) = self.inputs.get_mut(&target) {
                let cursor = self
                    .cursor_positions
                    .entry(target)
                    .or_insert(s.chars().count());
                if *cursor > 0 {
                    let byte_idx = s
                        .char_indices()
                        .nth(*cursor - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(s.len());
                    s.remove(byte_idx);
                    *cursor -= 1;
                }
            }
        }
    }

    pub fn move_cursor_to_start(&mut self) {
        if let Some(target) = self.editing.as_ref() {
            let pos = self.cursor_positions.entry(target.clone()).or_insert(0);
            *pos = 0;
        }
    }

    pub fn move_cursor_to_end(&mut self) {
        if let Some(target) = self.editing.as_ref() {
            let len = self
                .inputs
                .get(target)
                .map(|s| s.chars().count())
                .unwrap_or(0);
            let pos = self.cursor_positions.entry(target.clone()).or_insert(len);
            *pos = len;
        }
    }

    pub fn move_cursor_word_back(&mut self) {
        let Some(target) = self.editing.clone() else {
            return;
        };
        let chars: Vec<char> = self
            .inputs
            .get(&target)
            .map(|s| s.chars().collect())
            .unwrap_or_default();
        let len = chars.len();
        let cursor = self.cursor_positions.entry(target).or_insert(len);
        while *cursor > 0 && !chars[*cursor - 1].is_alphanumeric() {
            *cursor -= 1;
        }
        while *cursor > 0 && chars[*cursor - 1].is_alphanumeric() {
            *cursor -= 1;
        }
    }

    pub fn move_cursor_word_forward(&mut self) {
        let Some(target) = self.editing.clone() else {
            return;
        };
        let chars: Vec<char> = self
            .inputs
            .get(&target)
            .map(|s| s.chars().collect())
            .unwrap_or_default();
        let len = chars.len();
        let cursor = self.cursor_positions.entry(target).or_insert(len);
        while *cursor < len && !chars[*cursor].is_alphanumeric() {
            *cursor += 1;
        }
        while *cursor < len && chars[*cursor].is_alphanumeric() {
            *cursor += 1;
        }
    }

    pub fn delete_word_back(&mut self) {
        let Some(target) = self.editing.clone() else {
            return;
        };
        let chars: Vec<char> = self
            .inputs
            .get(&target)
            .map(|s| s.chars().collect())
            .unwrap_or_default();
        let len = chars.len();
        let end = self.cursor_positions.get(&target).copied().unwrap_or(len);
        let mut new_pos = end;
        while new_pos > 0 && !chars[new_pos - 1].is_alphanumeric() {
            new_pos -= 1;
        }
        while new_pos > 0 && chars[new_pos - 1].is_alphanumeric() {
            new_pos -= 1;
        }
        // Also eat any whitespace before the word so it travels with the word.
        while new_pos > 0 && !chars[new_pos - 1].is_alphanumeric() {
            new_pos -= 1;
        }
        if new_pos == end {
            return;
        }
        let byte_start: usize = chars[..new_pos].iter().map(|c| c.len_utf8()).sum();
        let byte_end: usize = chars[..end].iter().map(|c| c.len_utf8()).sum();
        if let Some(s) = self.inputs.get_mut(&target) {
            s.drain(byte_start..byte_end);
        }
        self.cursor_positions.insert(target, new_pos);
    }

    pub fn delete_word_forward(&mut self) {
        let Some(target) = self.editing.clone() else {
            return;
        };
        let chars: Vec<char> = self
            .inputs
            .get(&target)
            .map(|s| s.chars().collect())
            .unwrap_or_default();
        let len = chars.len();
        let start = self.cursor_positions.get(&target).copied().unwrap_or(len);
        let mut end = start;
        while end < len && !chars[end].is_alphanumeric() {
            end += 1;
        }
        while end < len && chars[end].is_alphanumeric() {
            end += 1;
        }
        if end == start {
            return;
        }
        let byte_start: usize = chars[..start].iter().map(|c| c.len_utf8()).sum();
        let byte_end: usize = chars[..end].iter().map(|c| c.len_utf8()).sum();
        if let Some(s) = self.inputs.get_mut(&target) {
            s.drain(byte_start..byte_end);
        }
    }

    pub fn pop_char_forward(&mut self) {
        if let Some(target) = self.editing.clone() {
            if let Some(s) = self.inputs.get_mut(&target) {
                let len = s.chars().count();
                let cursor = self.cursor_positions.entry(target).or_insert(len);
                if *cursor < len {
                    let byte_idx = s
                        .char_indices()
                        .nth(*cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(s.len());
                    s.remove(byte_idx);
                }
            }
        }
    }

    pub fn move_cursor_left(&mut self) {
        if let Some(target) = self.editing.as_ref() {
            let len = self
                .inputs
                .get(target)
                .map(|s| s.chars().count())
                .unwrap_or(0);
            let cursor = self.cursor_positions.entry(target.clone()).or_insert(len);
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
    }

    pub fn move_cursor_right(&mut self) {
        if let Some(target) = self.editing.as_ref() {
            let len = self
                .inputs
                .get(target)
                .map(|s| s.chars().count())
                .unwrap_or(0);
            let cursor = self.cursor_positions.entry(target.clone()).or_insert(len);
            if *cursor < len {
                *cursor += 1;
            }
        }
    }

    /// Insert `text` at the current cursor position and advance the cursor.
    pub fn insert_str_at_cursor(&mut self, text: &str) {
        if let Some(target) = self.editing.clone() {
            let s = self.inputs.entry(target.clone()).or_default();
            let cursor = self
                .cursor_positions
                .entry(target)
                .or_insert(s.chars().count());
            let byte_idx = s
                .char_indices()
                .nth(*cursor)
                .map(|(i, _)| i)
                .unwrap_or(s.len());
            s.insert_str(byte_idx, text);
            *cursor += text.chars().count();
        }
    }

    /// Delete from the cursor backwards to (and including) the nearest `@`
    /// character. Returns `(at_char_index, deleted_query)` on success, or
    /// `None` if no `@` is found between the start and cursor.
    pub fn delete_back_to_at(&mut self) -> Option<(usize, String)> {
        let target = self.editing.clone()?;
        let s = self.inputs.get_mut(&target)?;
        let cursor = *self
            .cursor_positions
            .get(&target)
            .unwrap_or(&s.chars().count());
        let chars: Vec<char> = s.chars().collect();
        // Find the rightmost `@` before the cursor.
        let at_pos = chars[..cursor].iter().rposition(|&c| c == '@')?;
        let query: String = chars[at_pos + 1..cursor].iter().collect();

        // Compute byte range [byte_start, byte_end) for `@query`.
        let byte_start: usize = chars[..at_pos].iter().map(|c| c.len_utf8()).sum();
        let byte_end: usize = chars[..cursor].iter().map(|c| c.len_utf8()).sum();
        s.drain(byte_start..byte_end);

        if let Some(pos) = self.cursor_positions.get_mut(&target) {
            *pos = at_pos;
        }
        Some((at_pos, query))
    }

    pub fn take_input(&mut self) -> Option<String> {
        let target = self.editing.take()?;
        self.cursor_positions.remove(&target);
        let text = self.inputs.remove(&target).unwrap_or_default();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
