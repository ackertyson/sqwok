use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{
    app::{AppState, RenderRow},
    pane::Pane,
    style as s,
};

pub fn draw(frame: &mut Frame, area: Rect, app: &AppState, pane: &Pane) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // chat topic
            Constraint::Min(1),    // messages
            Constraint::Length(1), // status
        ])
        .split(area);

    // Topic bar
    let topic = app.current_chat.as_deref().unwrap_or("(no chat)");
    let keys_indicator = if app.has_keys { "[enc]" } else { "[no keys]" };
    let topic_line = Line::from(vec![
        Span::styled(
            topic,
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} members", app.members.len()),
            Style::default().fg(s::dim()),
        ),
        Span::raw("  "),
        Span::styled(
            keys_indicator,
            Style::default().fg(if app.has_keys {
                s::success_color()
            } else {
                s::error_color()
            }),
        ),
    ]);
    frame.render_widget(Paragraph::new(topic_line), chunks[0]);

    // Message area
    draw_messages(frame, chunks[1], app, pane);

    // Status bar — show screenname and editing indicator
    let editing_hint = if pane.current_input().is_empty() {
        ""
    } else {
        " [typing...]"
    };
    let status = Line::from(vec![
        Span::styled(&app.my_screenname, Style::default().fg(s::accent())),
        Span::styled(editing_hint, Style::default().fg(s::dim())),
        Span::raw("  "),
        Span::styled(
            "↑↓ nav  Enter edit  →/← expand/collapse  / cmd  Alt+N split", // TODO colorize the shortcut keys
            Style::default().fg(s::dim()),
        ),
    ]);
    frame.render_widget(Paragraph::new(status), chunks[2]);
}

fn draw_messages(frame: &mut Frame, area: Rect, app: &AppState, pane: &Pane) {
    let rows = app.build_render_rows();
    let total_rows = rows.len();

    if total_rows == 0 {
        let empty = Paragraph::new("No messages yet. Press Enter to start typing.")
            .style(Style::default().fg(s::dim()));
        frame.render_widget(empty, area);
        return;
    }

    // Show "scroll up for older messages" hint when there's more history
    if app.msg_store.has_more_above && pane.scroll_offset == 0 && pane.selected == 0 {
        let hint =
            Paragraph::new("↑ scroll for older messages").style(Style::default().fg(s::dim()));
        let hint_area = Rect::new(area.x, area.y, area.width, 1);
        frame.render_widget(hint, hint_area);
    }

    let visible_height = area.height as usize;

    // Ensure scroll_offset keeps selection visible
    let selected = pane.selected.min(total_rows.saturating_sub(1));
    let scroll_offset = {
        let offset = pane.scroll_offset;
        if selected < offset {
            selected
        } else if selected >= offset + visible_height {
            selected.saturating_sub(visible_height.saturating_sub(1))
        } else {
            offset
        }
    };

    let visible_rows: Vec<&RenderRow> = rows
        .iter()
        .skip(scroll_offset)
        .take(visible_height)
        .collect();

    for (i, row) in visible_rows.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }
        let row_area = Rect::new(area.x, y, area.width, 1);
        let abs_idx = scroll_offset + i;
        let is_selected = abs_idx == selected;

        draw_row(frame, row_area, row, is_selected);
    }

    // New messages below indicator
    let visible_bottom = scroll_offset + visible_height;
    if visible_bottom < total_rows {
        let new_count = total_rows - visible_bottom;
        let indicator = Paragraph::new(format!("v {} new", new_count)).style(
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        );
        let ind_area = Rect::new(
            area.x + area.width.saturating_sub(12),
            area.y + area.height.saturating_sub(1),
            11,
            1,
        );
        frame.render_widget(indicator, ind_area);
    }
}

fn draw_row(frame: &mut Frame, area: Rect, row: &RenderRow, is_selected: bool) {
    let bg = if is_selected {
        s::selection_bg()
    } else {
        s::BG
    };

    let line = match row {
        RenderRow::Message {
            author,
            author_uuid,
            body,
            timestamp,
            indent,
            is_mine,
            is_pending,
            highlight_age,
            reply_to_uuid,
            ..
        } => {
            let actual_bg = if is_selected {
                s::selection_bg()
            } else if let Some(age) = highlight_age {
                if age.as_millis() < 1000 {
                    s::highlight_bg()
                } else {
                    s::BG
                }
            } else {
                s::BG
            };

            let indent_str = build_indent(*indent);

            // Show reply indicator if this is a reply-to a specific message
            let reply_prefix = if reply_to_uuid.is_some() { "↩ " } else { "" };

            // Style own messages with a subtle indicator
            let name_style = if *is_mine {
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(s::username_color(author_uuid))
                    .add_modifier(Modifier::BOLD)
            };

            let mut spans = vec![
                Span::styled(indent_str, Style::default().fg(s::dim())),
                Span::styled(reply_prefix, Style::default().fg(s::dim())),
                Span::styled(author.clone(), name_style),
                Span::raw("  "),
                Span::raw(body.clone()),
                Span::raw("  "),
                Span::styled(timestamp.clone(), Style::default().fg(s::dim())),
            ];
            if *is_pending {
                spans.push(Span::styled(
                    " sending...",
                    Style::default()
                        .fg(s::dim())
                        .add_modifier(Modifier::ITALIC),
                ));
            }
            Line::from(spans).style(Style::default().bg(actual_bg))
        }
        RenderRow::CollapsedThread {
            author,
            author_uuid,
            preview,
            reply_count,
            timestamp,
            ..
        } => Line::from(vec![
            Span::styled(
                author.clone(),
                Style::default()
                    .fg(s::username_color(author_uuid))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(preview.clone()),
            Span::raw("  "),
            Span::styled(
                format!("[{} replies]", reply_count),
                Style::default().fg(s::accent()),
            ),
            Span::raw("  "),
            Span::styled(timestamp.clone(), Style::default().fg(s::dim())),
        ]),
        RenderRow::Input {
            thread_uuid,
            indent,
            is_active,
            content,
        } => {
            let indent_str = build_indent(*indent);
            let prompt_color = if *is_active { s::accent() } else { s::dim() };
            let mut spans = vec![
                Span::styled(indent_str, Style::default().fg(s::dim())),
                Span::styled("> ", Style::default().fg(prompt_color)),
            ];
            if *is_active {
                spans.push(Span::raw(content.clone()));
                spans.push(Span::styled("_", Style::default().fg(s::accent())));
            } else if thread_uuid.is_some() {
                spans.push(Span::styled("reply...", Style::default().fg(s::dim())));
            } else {
                spans.push(Span::styled("message...", Style::default().fg(s::dim())));
            }
            Line::from(spans)
        }
    };

    frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
}

fn build_indent(indent: u8) -> String {
    match indent {
        0 => String::new(),
        1 => "  +- ".to_string(),
        _ => "  |  +- ".to_string(),
    }
}
