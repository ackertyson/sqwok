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
    }

    pub fn push_char(&mut self, c: char) {
        // Reject control characters (except common whitespace) and enforce a
        // reasonable message length cap to prevent runaway input.
        if c.is_control() && c != '\t' {
            return;
        }
        if let Some(target) = self.editing.clone() {
            let s = self.inputs.entry(target).or_default();
            if s.len() < 4096 {
                s.push(c);
            }
        }
    }

    pub fn pop_char(&mut self) {
        if let Some(target) = self.editing.clone() {
            if let Some(s) = self.inputs.get_mut(&target) {
                s.pop();
            }
        }
    }

    pub fn take_input(&mut self) -> Option<String> {
        let target = self.editing.take()?;
        let text = self.inputs.remove(&target).unwrap_or_default();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}
