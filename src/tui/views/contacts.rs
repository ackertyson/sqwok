use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{app::ContactsModalState, style as s};

use super::modal::draw_modal_frame;

pub fn draw(frame: &mut Frame, state: &ContactsModalState) {
    draw_modal_frame(frame, "Contacts", |frame, area| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // filter input
                Constraint::Length(1), // hint
                Constraint::Min(1),    // list
            ])
            .split(area);

        // Filter input
        let input_line = Line::from(vec![
            Span::styled("Filter: ", Style::default().fg(s::dim())),
            Span::styled(&state.filter, Style::default().fg(s::fg())),
            Span::styled("█", Style::default().fg(s::accent())),
        ]);
        frame.render_widget(Paragraph::new(input_line), chunks[0]);

        // Hint
        frame.render_widget(
            Paragraph::new("↑↓ navigate   Esc close")
                .style(Style::default().fg(s::dim()))
                .alignment(Alignment::Center),
            chunks[1],
        );

        let filtered = state.filtered();

        if filtered.is_empty() {
            let msg = if state.contacts.is_empty() {
                "No contacts yet. Contacts are added automatically from chat members."
            } else {
                "No contacts match your filter."
            };
            frame.render_widget(
                Paragraph::new(msg).style(Style::default().fg(s::dim())),
                chunks[2],
            );
        } else {
            let lines: Vec<Line> = filtered
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let selected = i == state.selected;
                    let style = if selected {
                        Style::default()
                            .bg(s::selection_bg())
                            .fg(s::fg())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(s::fg())
                    };
                    let marker = if selected { "► " } else { "  " };
                    let ago = format_time_ago(c.updated_at);
                    let chat_hint = c
                        .last_seen_chat
                        .map(|u| format!("  in {}", &u.to_string()[..8]))
                        .unwrap_or_default();
                    Line::from(vec![
                        Span::styled(marker, style),
                        Span::styled(c.screenname.clone(), style),
                        Span::styled(
                            format!("  {}{}", ago, chat_hint),
                            Style::default().fg(s::dim()),
                        ),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), chunks[2]);
        }
    });
}

fn format_time_ago(unix_ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - unix_ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

pub enum ContactsAction {
    Continue,
    Close,
}

pub fn handle_input(key: KeyEvent, state: &mut ContactsModalState) -> ContactsAction {
    match key.code {
        KeyCode::Esc => ContactsAction::Close,
        KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            ContactsAction::Continue
        }
        KeyCode::Down => {
            let max = state.filtered().len().saturating_sub(1);
            if state.selected < max {
                state.selected += 1;
            }
            ContactsAction::Continue
        }
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            state.filter.push(c);
            state.selected = 0;
            ContactsAction::Continue
        }
        KeyCode::Backspace => {
            state.filter.pop();
            state.selected = 0;
            ContactsAction::Continue
        }
        _ => ContactsAction::Continue,
    }
}
