use std::collections::HashMap;

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
    /// Scroll offset (first visible row index)
    pub scroll_offset: usize,
    /// Currently editing an input (None = navigation mode)
    pub editing: Option<InputTarget>,
    /// Input field contents by target
    pub inputs: HashMap<InputTarget, String>,
}

impl Pane {
    pub fn new() -> Self {
        Pane {
            selected: 0,
            scroll_offset: 0,
            editing: None,
            inputs: HashMap::new(),
        }
    }

    pub fn current_input(&self) -> &str {
        if let Some(ref target) = self.editing {
            self.inputs.get(target).map(|s| s.as_str()).unwrap_or("")
        } else {
            ""
        }
    }

    pub fn push_char(&mut self, c: char) {
        if let Some(target) = self.editing.clone() {
            self.inputs.entry(target).or_default().push(c);
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
