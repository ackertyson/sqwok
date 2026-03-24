use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{app::SearchModalState, style as s};

use super::modal::draw_modal_frame;

pub fn draw(frame: &mut Frame, state: &SearchModalState) {
    draw_modal_frame(frame, "Find Users", |frame, area| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // query input
                Constraint::Length(1), // separator hint
                Constraint::Min(1),    // results
            ])
            .split(area);

        // Query input field
        let input_line = Line::from(vec![
            Span::styled("Search: ", Style::default().fg(s::dim())),
            Span::styled(&state.query, Style::default().fg(s::fg())),
            Span::styled("█", Style::default().fg(s::accent())),
        ]);
        frame.render_widget(Paragraph::new(input_line), chunks[0]);

        // Hint
        frame.render_widget(
            Paragraph::new("↑↓ navigate   Enter add to chat   Esc close")
                .style(Style::default().fg(s::dim()))
                .alignment(Alignment::Center),
            chunks[1],
        );

        // Results list
        if state.results.is_empty() {
            let placeholder = if state.query.is_empty() {
                "Type to search users..."
            } else if state.query != state.last_searched {
                "Searching..."
            } else {
                "No results."
            };
            frame.render_widget(
                Paragraph::new(placeholder).style(Style::default().fg(s::dim())),
                chunks[2],
            );
        } else {
            let result_lines: Vec<Line> = state
                .results
                .iter()
                .enumerate()
                .map(|(i, r)| {
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
                    Line::from(vec![
                        Span::styled(marker, style),
                        Span::styled(r.screenname.clone(), style),
                        Span::styled(
                            format!("  {}", &r.uuid.to_string()[..8]),
                            Style::default().fg(s::dim()),
                        ),
                    ])
                })
                .collect();
            frame.render_widget(Paragraph::new(result_lines), chunks[2]);
        }
    });
}

/// Returns the UUID of the selected result if Enter was pressed, or None.
pub fn handle_input(key: KeyEvent, state: &mut SearchModalState) -> SearchAction {
    match key.code {
        KeyCode::Esc => SearchAction::Close,
        KeyCode::Enter => {
            if let Some(r) = state.results.get(state.selected) {
                SearchAction::SelectUser(r.uuid)
            } else {
                SearchAction::Close
            }
        }
        KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            SearchAction::Continue
        }
        KeyCode::Down => {
            if state.selected + 1 < state.results.len() {
                state.selected += 1;
            }
            SearchAction::Continue
        }
        KeyCode::Char(c)
            if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
        {
            state.query.push(c);
            state.selected = 0;
            SearchAction::QueryChanged
        }
        KeyCode::Backspace => {
            state.query.pop();
            state.selected = 0;
            SearchAction::QueryChanged
        }
        _ => SearchAction::Continue,
    }
}

pub enum SearchAction {
    Continue,
    Close,
    QueryChanged,
    SelectUser(uuid::Uuid),
}
