use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::{app::AppState, style as s};

pub fn draw(frame: &mut Frame, app: &mut AppState) {
    let area = frame.area();

    let has_invitations = !app.invitations.is_empty();
    let inv_height = if has_invitations {
        (app.invitations.len() as u16 + 2).min(6)
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(inv_height),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    let title = Paragraph::new("sqwok")
        .style(
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(s::dim())),
        );

    frame.render_widget(title, chunks[0]);

    // Invitations section
    if has_invitations {
        let inv_items: Vec<ListItem> = app
            .invitations
            .iter()
            .map(|inv| {
                let from = inv
                    .invited_by
                    .as_deref()
                    .map(|s| format!("  from {}", s))
                    .unwrap_or_default();
                let ago = s::format_time_ago(inv.received_at);
                let content = Line::from(vec![
                    Span::styled("● ", Style::default().fg(s::warning_color())),
                    Span::styled(
                        inv.topic.clone(),
                        Style::default()
                            .fg(s::warning_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(from, Style::default().fg(s::dim())),
                    Span::styled(format!("  {}", ago), Style::default().fg(s::dim())),
                ]);
                ListItem::new(content)
            })
            .collect();

        let inv_list = List::new(inv_items).block(
            Block::default()
                .title(format!(
                    " {} pending invitation(s) — Enter to accept ",
                    app.invitations.len()
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(s::warning_color())),
        );
        frame.render_widget(inv_list, chunks[1]);
    }

    let selected_idx = app.picker_state.selected();

    let items: Vec<ListItem> = app
        .chat_list
        .iter()
        .enumerate()
        .map(|(i, chat)| {
            let is_selected = selected_idx == Some(i);

            let mut spans: Vec<Span> = if is_selected {
                vec![
                    Span::styled(
                        "\u{2590}",
                        Style::default().fg(s::pill_color()).bg(s::selection_bg()),
                    ),
                    Span::styled(
                        format!(" {} ", chat.topic),
                        Style::default()
                            .fg(s::pill_fg())
                            .bg(s::pill_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "\u{258C}",
                        Style::default().fg(s::pill_color()).bg(s::selection_bg()),
                    ),
                ]
            } else {
                vec![Span::styled(
                    chat.topic.clone(),
                    Style::default().fg(s::fg()).add_modifier(Modifier::BOLD),
                )]
            };

            if let Some(desc) = &chat.description {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(desc.clone(), Style::default().fg(s::dim())));
            }
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!("{} members", chat.member_count),
                Style::default().fg(s::dim()),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().bg(s::selection_bg()))
        .highlight_symbol("");

    frame.render_stateful_widget(list, chunks[2], &mut app.picker_state);

    let help_line = if has_invitations {
        s::hint_line(&[
            ("↑↓", "navigate"),
            ("Enter", "accept/join"),
            ("I", "accept invitation"),
            ("Ctrl-C", "quit"),
        ])
    } else {
        s::hint_line(&[("↑↓", "navigate"), ("Enter", "join"), ("Ctrl-C", "quit")])
    };
    let help = Paragraph::new(help_line).alignment(Alignment::Center);
    frame.render_widget(help, chunks[3]);
}
