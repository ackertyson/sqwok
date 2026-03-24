use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
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
}

impl CommandBarState {
    pub fn new() -> Self {
        CommandBarState {
            input: String::new(),
            suggestions: Vec::new(),
            selected_suggestion: 0,
        }
    }

    /// Clone a snapshot for drawing (avoids borrow conflicts).
    pub fn clone_for_draw(&self) -> CommandBarState {
        self.clone()
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

        self.suggestions = suggestions;
        self.selected_suggestion = 0;
    }

    pub fn select_prev(&mut self) {
        if !self.suggestions.is_empty() {
            if self.selected_suggestion == 0 {
                self.selected_suggestion = self.suggestions.len() - 1;
            } else {
                self.selected_suggestion -= 1;
            }
        }
    }

    pub fn select_next(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected_suggestion = (self.selected_suggestion + 1) % self.suggestions.len();
        }
    }

    pub fn execute(&mut self) -> Option<CommandAction> {
        self.suggestions
            .get(self.selected_suggestion)
            .map(|s| s.action.clone())
    }
}

pub fn draw(frame: &mut Frame, state: &CommandBarState) {
    let area = frame.area();

    let bar_height = if state.suggestions.is_empty() {
        1u16
    } else {
        (state.suggestions.len() as u16 + 1).min(8)
    };

    let bar_y = area.height.saturating_sub(bar_height);

    // Suggestion list
    for (i, suggestion) in state
        .suggestions
        .iter()
        .enumerate()
        .take((bar_height - 1) as usize)
    {
        let y = bar_y + i as u16;
        if y >= area.height.saturating_sub(1) {
            break;
        }
        let style = if i == state.selected_suggestion {
            Style::default().bg(s::selection_bg()).fg(s::fg())
        } else {
            Style::default().fg(s::dim())
        };
        let line = Line::from(vec![
            Span::styled(suggestion.label.clone(), style.add_modifier(Modifier::BOLD)),
            Span::styled(format!("  {}", suggestion.description), style),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(1, y, area.width.saturating_sub(2), 1),
        );
    }

    // Input row
    let input_area = Rect::new(0, area.height.saturating_sub(1), area.width, 1);
    let input_line = Line::from(vec![
        Span::styled(
            "/",
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(state.input.clone()),
        Span::styled("█", Style::default().fg(s::accent())),
    ]);
    frame.render_widget(
        Paragraph::new(input_line)
            .style(Style::default().bg(ratatui::style::Color::Rgb(15, 15, 25))),
        input_area,
    );
}
