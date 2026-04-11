use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::tui::{app::AppState, style as s};

#[derive(Clone, Debug)]
pub enum CommandAction {
    SwitchChat(String),
    MemberList,
    GroupSettings,
    InviteCreate,
    Search,
    Contacts,
    JoinByCode(String),
    RotateKeys,
    BlockUsers,
    Quit,
}

#[derive(Clone, Debug)]
pub struct CommandSuggestion {
    pub label: String,
    pub description: String,
    pub action: CommandAction,
}

#[derive(Clone, Debug)]
pub struct CommandBarState {
    pub input: String,
    pub suggestions: Vec<CommandSuggestion>,
    pub selected_suggestion: usize,
    pub scroll_offset: usize,
}

impl CommandBarState {
    pub fn new() -> Self {
        CommandBarState {
            input: String::new(),
            suggestions: Vec::new(),
            selected_suggestion: 0,
            scroll_offset: 0,
        }
    }

    pub fn update_suggestions(&mut self, app: &AppState) {
        let input = self.input.to_lowercase();
        let mut suggestions = Vec::new();

        // Fixed commands
        let fixed: &[(&str, &str, CommandAction)] = &[
            ("members", "m", CommandAction::MemberList),
            ("settings", "set", CommandAction::GroupSettings),
            ("invite", "inv", CommandAction::InviteCreate),
            ("search", "sr", CommandAction::Search),
            ("contacts", "co", CommandAction::Contacts),
            ("join", "j", CommandAction::JoinByCode(String::new())),
            ("rotate", "rot", CommandAction::RotateKeys),
            ("block", "bl", CommandAction::BlockUsers),
            ("quit", "q", CommandAction::Quit),
        ];

        let descs = &[
            "Show member list",
            "Group settings",
            "Create invite code",
            "Find users by screenname",
            "Local contacts",
            "Join by invite code",
            "Rotate encryption keys",
            "Manage blocked users",
            "Exit sqwok",
        ];

        for (i, (full, short, action)) in fixed.iter().enumerate() {
            if full.starts_with(&input) || short.starts_with(&input) || input.is_empty() {
                suggestions.push(CommandSuggestion {
                    label: format!("/{}", full),
                    description: descs[i].to_string(),
                    action: action.clone(),
                });
            }
        }

        // /join <code> — if user typed "join " with a code
        if input.starts_with("join ") {
            let code = input.trim_start_matches("join ").trim().to_string();
            if !code.is_empty() {
                suggestions.clear();
                suggestions.push(CommandSuggestion {
                    label: format!("/join {}", code),
                    description: "Redeem invite code".to_string(),
                    action: CommandAction::JoinByCode(code),
                });
            }
        }

        // Switch chat suggestions
        if "switch".starts_with(&input) || input.is_empty() {
            for chat in &app.chat_list {
                suggestions.push(CommandSuggestion {
                    label: format!("/switch {}", chat.topic),
                    description: chat.uuid.chars().take(8).collect::<String>(),
                    action: CommandAction::SwitchChat(chat.uuid.clone()),
                });
            }
        }

        // Sort alphabetically by label, except keep /join <code> at top when active
        if !input.starts_with("join ") || suggestions.is_empty() {
            suggestions.sort_by(|a, b| a.label.cmp(&b.label));
        }

        self.suggestions = suggestions;
        self.selected_suggestion = 0;
        self.scroll_offset = 0;
    }

    fn adjust_scroll(&mut self, visible: usize) {
        if self.selected_suggestion < self.scroll_offset {
            self.scroll_offset = self.selected_suggestion;
        } else if self.selected_suggestion >= self.scroll_offset + visible {
            self.scroll_offset = self.selected_suggestion + 1 - visible;
        }
    }

    pub fn select_prev(&mut self) {
        if !self.suggestions.is_empty() {
            if self.selected_suggestion == 0 {
                self.selected_suggestion = self.suggestions.len() - 1;
            } else {
                self.selected_suggestion -= 1;
            }
            self.adjust_scroll(MAX_VISIBLE_SUGGESTIONS);
        }
    }

    pub fn select_next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected_suggestion = (self.selected_suggestion + 1) % self.suggestions.len();
            self.adjust_scroll(MAX_VISIBLE_SUGGESTIONS);
        }
    }

    pub fn execute(&mut self) -> Option<CommandAction> {
        self.suggestions
            .get(self.selected_suggestion)
            .map(|s| s.action.clone())
    }
}

const MAX_VISIBLE_SUGGESTIONS: usize = 7;

pub fn draw(frame: &mut Frame, state: &CommandBarState) {
    let area = frame.area();

    let visible_count = state.suggestions.len().min(MAX_VISIBLE_SUGGESTIONS);
    let bar_height = (visible_count as u16) + 1; // +1 for input row
    let bar_y = area.height.saturating_sub(bar_height);

    // Clear the entire command bar area so the footer is fully covered
    frame.render_widget(Clear, Rect::new(0, bar_y, area.width, bar_height));

    // Suggestion list
    let overlay_bg = s::overlay_bg();
    for (row, suggestion) in state
        .suggestions
        .iter()
        .enumerate()
        .skip(state.scroll_offset)
        .take(visible_count)
    {
        let y = bar_y + (row - state.scroll_offset) as u16;
        let selected = row == state.selected_suggestion;
        let style = if selected {
            Style::default().bg(s::selection_bg()).fg(s::fg())
        } else {
            Style::default().bg(overlay_bg).fg(s::dim())
        };
        let line = Line::from(vec![
            Span::styled(" ", style),
            Span::styled(suggestion.label.clone(), style.add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", suggestion.description), style),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(style),
            Rect::new(0, y, area.width, 1),
        );
    }

    // Input row
    let input_area = Rect::new(0, area.height.saturating_sub(1), area.width, 1);
    let input_style = Style::default().bg(overlay_bg);
    let input_line = Line::from(vec![
        Span::styled(
            "/",
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state
                .input
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string(),
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            state
                .input
                .find(' ')
                .map(|i| &state.input[i..])
                .unwrap_or("")
                .to_string(),
            Style::default().fg(s::fg()),
        ),
        Span::styled("█", Style::default().fg(s::accent())),
    ]);
    frame.render_widget(Paragraph::new(input_line).style(input_style), input_area);
}
