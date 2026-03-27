use std::collections::{HashMap, HashSet};

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum InputTarget {
    MainChat,
    Thread(String),        // thread root uuid
    Reply(String, String), // (reply_to_uuid, thread_root_uuid)
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
