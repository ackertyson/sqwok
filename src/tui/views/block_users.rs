use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{
    app::{BlockAction, BlockFocus, BlockUsersModalState},
    style as s,
};

use super::modal::draw_modal_frame;

pub enum BlockUsersAction {
    Continue,
    Block(String, String),   // (uuid, screenname) — confirmed block
    Unblock(String, String), // (uuid, screenname) — confirmed unblock
    Close,
}

pub fn draw(frame: &mut Frame, state: &BlockUsersModalState) {
    draw_modal_frame(frame, "Block Users", |frame, area| {
        // When a confirmation is pending, take over the entire body.
        if let Some((_, name, action)) = &state.confirm {
            let verb = match action {
                BlockAction::Block => "block",
                BlockAction::Unblock => "unblock",
            };
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)])
                .split(area);

            let prompt = Line::from(vec![
                Span::styled(format!("Really {} ", verb), Style::default().fg(s::dim())),
                Span::styled(name.clone(), Style::default().fg(s::accent())),
                Span::styled("?", Style::default().fg(s::dim())),
            ]);
            let keys = Line::from(vec![Span::styled(
                "  [y] confirm    [n] cancel  ",
                Style::default().fg(s::accent()),
            )]);
            frame.render_widget(
                Paragraph::new(vec![Line::raw(""), prompt, keys]).alignment(Alignment::Center),
                chunks[1],
            );
            return;
        }

        // Normal two-section layout.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // search + suggestions
                Constraint::Length(1), // spacer
                Constraint::Min(1),    // blocked list
                Constraint::Length(1), // hint
            ])
            .split(area);

        // --- Search section ---
        let search_focused = state.focus == BlockFocus::Search;
        let search_label_style = if search_focused {
            Style::default().fg(s::accent())
        } else {
            Style::default().fg(s::dim())
        };

        let search_section = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // header + input
                Constraint::Min(0),    // suggestion list
            ])
            .split(chunks[0]);

        let input_line = Line::from(vec![
            Span::styled("Search  ", search_label_style),
            Span::styled("> ", Style::default().fg(s::accent())),
            Span::styled(state.search_input.clone(), Style::default().fg(s::fg())),
            Span::styled("█", Style::default().fg(s::accent())),
        ]);
        frame.render_widget(Paragraph::new(input_line), search_section[0]);

        if !state.suggestions.is_empty() {
            let lines: Vec<Line> = state
                .suggestions
                .iter()
                .enumerate()
                .map(|(i, (_, name))| {
                    let selected = search_focused && i == state.selected_suggestion;
                    let style = s::selected_style(selected);
                    let marker = if selected { "► " } else { "  " };
                    Line::from(vec![
                        Span::styled(marker, style),
                        Span::styled(name.clone(), style),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), search_section[1]);
        }

        // --- Blocked section ---
        let blocked_focused = state.focus == BlockFocus::Blocked;
        let blocked_label_style = if blocked_focused {
            Style::default().fg(s::accent())
        } else {
            Style::default().fg(s::dim())
        };

        let blocked_section = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // "Blocked (N)" header
                Constraint::Min(0),    // blocked list
            ])
            .split(chunks[2]);

        let header = format!("Blocked ({})", state.blocked_list.len());
        frame.render_widget(
            Paragraph::new(header).style(blocked_label_style),
            blocked_section[0],
        );

        if state.blocked_list.is_empty() {
            frame.render_widget(
                Paragraph::new("  No blocked users.").style(Style::default().fg(s::dim())),
                blocked_section[1],
            );
        } else {
            let lines: Vec<Line> = state
                .blocked_list
                .iter()
                .enumerate()
                .map(|(i, (_, name))| {
                    let selected = blocked_focused && i == state.selected_blocked;
                    let style = s::selected_style(selected);
                    let marker = if selected { "► " } else { "  " };
                    Line::from(vec![
                        Span::styled(marker, style),
                        Span::styled("⊘ ", Style::default().fg(s::dim())),
                        Span::styled(name.clone(), style),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), blocked_section[1]);
        }

        // --- Hint ---
        frame.render_widget(
            Paragraph::new("Enter block/unblock  ·  Tab switch  ·  Esc close")
                .style(Style::default().fg(s::dim()))
                .alignment(Alignment::Center),
            chunks[3],
        );
    });
}

pub fn handle_input(key: KeyEvent, state: &mut BlockUsersModalState) -> BlockUsersAction {
    // Confirmation pending — Enter also acts as 'y' so the user can press Enter twice.
    if state.confirm.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some((uuid, name, action)) = state.confirm.take() {
                    return match action {
                        BlockAction::Block => BlockUsersAction::Block(uuid, name),
                        BlockAction::Unblock => BlockUsersAction::Unblock(uuid, name),
                    };
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                state.confirm = None;
            }
            _ => {}
        }
        return BlockUsersAction::Continue;
    }

    match key.code {
        KeyCode::Esc => BlockUsersAction::Close,

        KeyCode::Tab => {
            state.focus = match state.focus {
                BlockFocus::Search => BlockFocus::Blocked,
                BlockFocus::Blocked => BlockFocus::Search,
            };
            BlockUsersAction::Continue
        }

        KeyCode::Up => {
            match state.focus {
                BlockFocus::Search => {
                    state.selected_suggestion = state.selected_suggestion.saturating_sub(1);
                }
                BlockFocus::Blocked => {
                    state.selected_blocked = state.selected_blocked.saturating_sub(1);
                }
            }
            BlockUsersAction::Continue
        }

        KeyCode::Down => {
            match state.focus {
                BlockFocus::Search => {
                    let max = state.suggestions.len().saturating_sub(1);
                    if state.selected_suggestion < max {
                        state.selected_suggestion += 1;
                    }
                }
                BlockFocus::Blocked => {
                    let max = state.blocked_list.len().saturating_sub(1);
                    if state.selected_blocked < max {
                        state.selected_blocked += 1;
                    }
                }
            }
            BlockUsersAction::Continue
        }

        KeyCode::Enter => {
            match state.focus {
                BlockFocus::Search => {
                    if let Some((uuid, name)) = state.suggestions.get(state.selected_suggestion) {
                        state.confirm = Some((uuid.clone(), name.clone(), BlockAction::Block));
                    }
                }
                BlockFocus::Blocked => {
                    if let Some((uuid, name)) = state.blocked_list.get(state.selected_blocked) {
                        state.confirm = Some((uuid.clone(), name.clone(), BlockAction::Unblock));
                    }
                }
            }
            BlockUsersAction::Continue
        }

        KeyCode::Char(c)
            if state.focus == BlockFocus::Search
                && (key.modifiers == KeyModifiers::NONE
                    || key.modifiers == KeyModifiers::SHIFT) =>
        {
            state.search_input.push(c);
            state.selected_suggestion = 0;
            BlockUsersAction::Continue
        }

        KeyCode::Backspace if state.focus == BlockFocus::Search => {
            state.search_input.pop();
            state.selected_suggestion = 0;
            BlockUsersAction::Continue
        }

        _ => BlockUsersAction::Continue,
    }
}
