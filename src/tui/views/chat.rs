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

    // Topic bar — look up friendly name from chat_list, fall back to uuid
    let chat_summary = app
        .current_chat
        .as_deref()
        .and_then(|uuid| app.chat_list.iter().find(|c| c.uuid == uuid));
    let topic = chat_summary
        .map(|c| c.topic.as_str())
        .or(app.current_chat.as_deref())
        .unwrap_or("(no chat)");
    let description = chat_summary.and_then(|c| c.description.as_deref());
    let keys_indicator = if app.has_keys {
        "[secure]"
    } else {
        "[no keys]"
    };
    let mut topic_spans = vec![Span::styled(
        topic,
        Style::default()
            .fg(s::accent())
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(desc) = description {
        topic_spans.push(Span::raw("  "));
        topic_spans.push(Span::styled(desc, Style::default().fg(s::dim())));
    }
    topic_spans.push(Span::raw("  "));
    let topic_line = Line::from({
        let mut spans = topic_spans;
        spans.extend(vec![
            Span::styled(
                {
                    let total = app.members.len();
                    let online = app.members.iter().filter(|m| m.online).count();
                    if online < total {
                        format!("{} members ({} online)", total, online)
                    } else {
                        format!("{} members", total)
                    }
                },
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
        spans
    });
    frame.render_widget(Paragraph::new(topic_line), chunks[0]);

    // Message area
    draw_messages(frame, chunks[1], app, pane);

    // Status bar — show screenname and editing indicator
    let editing_hint = if pane.current_input().is_empty() {
        ""
    } else {
        " [typing...]"
    };
    let mut status_spans = vec![
        Span::styled(app.my_screenname.clone(), Style::default().fg(s::accent())),
        Span::styled(editing_hint, Style::default().fg(s::dim())),
        Span::raw("  "),
    ];
    let hint = s::hint_line(&[
        ("↑↓", "nav"),
        ("Enter", "edit"),
        ("→/←", "expand/collapse"),
        ("/", "cmd"),
        ("Alt+\\", "vpane"),
        ("Alt+-", "hpane"),
    ]);
    status_spans.extend(hint.spans);
    frame.render_widget(Paragraph::new(Line::from(status_spans)), chunks[2]);
}

fn draw_messages(frame: &mut Frame, area: Rect, app: &AppState, pane: &Pane) {
    let rows = app.build_render_rows_for_pane(pane);
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

    // Ensure scroll_offset keeps selection visible (in row-count units)
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

    // Render rows from scroll_offset, tracking current y position.
    // Each row may occupy multiple lines depending on message length.
    let mut y = area.y;
    let area_bottom = area.y + area.height;
    let mut last_rendered_row = scroll_offset;

    for (abs_idx, row) in rows.iter().enumerate().skip(scroll_offset) {
        if y >= area_bottom {
            break;
        }
        let is_selected = abs_idx == selected;
        let row_height = row_visual_height(row, area.width);
        let available = area_bottom.saturating_sub(y);
        let render_height = row_height.min(available);
        let row_area = Rect::new(area.x, y, area.width, render_height);
        draw_row(frame, row_area, row, is_selected, area.width);
        y += row_height;
        last_rendered_row = abs_idx;
    }

    // New messages below indicator
    if last_rendered_row + 1 < total_rows {
        let new_count = total_rows - last_rendered_row - 1;
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

/// Compute how many terminal lines a row will occupy at the given width.
fn row_visual_height(row: &RenderRow, width: u16) -> u16 {
    let RenderRow::Message {
        indent,
        author,
        body,
        timestamp,
        reply_to_uuid,
        is_pending,
        ..
    } = row
    else {
        return 1;
    };
    if width == 0 {
        return 1;
    }
    let w = width as usize;
    let indent_len = match indent {
        0 => 0usize,
        1 => 5,
        _ => 9,
    };
    let reply_len = if reply_to_uuid.is_some() { 2usize } else { 0 };
    let pending_len: usize = if *is_pending { " sending...".len() } else { 0 };
    // prefix: indent + reply + author + "  "
    let prefix_len = indent_len + reply_len + author.chars().count() + 2;
    // suffix on first line: "  " + timestamp
    let ts_len = 2 + timestamp.chars().count();
    let body_len = body.chars().count() + pending_len;

    // First line fits prefix + body_first + timestamp
    let first_avail = w.saturating_sub(prefix_len + ts_len);
    if body_len <= first_avail || first_avail == 0 {
        return 1;
    }
    // Remaining body wraps with prefix padding
    let remaining = body_len - first_avail;
    let cont_avail = w.saturating_sub(prefix_len).max(1);
    let cont_lines = remaining.div_ceil(cont_avail);
    (1 + cont_lines).min(area_height_cap()) as u16
}

/// Cap maximum row height to avoid pathological cases.
#[inline]
fn area_height_cap() -> usize {
    20
}

fn draw_row(frame: &mut Frame, area: Rect, row: &RenderRow, is_selected: bool, avail_width: u16) {
    let bg = if is_selected {
        s::selection_bg()
    } else {
        s::BG
    };

    match row {
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
            let reply_prefix = if reply_to_uuid.is_some() { "↩ " } else { "" };
            let name_style = if *is_mine {
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(s::username_color(author_uuid))
                    .add_modifier(Modifier::BOLD)
            };

            let pending_suffix = if *is_pending { " sending..." } else { "" };
            let full_body = format!("{}{}", body, pending_suffix);

            // Compute layout widths
            let w = avail_width as usize;
            let indent_chars = indent_str.chars().count();
            let reply_chars = reply_prefix.chars().count();
            let author_chars = author.chars().count();
            let prefix_len = indent_chars + reply_chars + author_chars + 2; // +2 for "  "
            let ts_suffix = format!("  {}", timestamp);
            let ts_len = ts_suffix.chars().count();
            let body_chars: Vec<char> = full_body.chars().collect();
            let first_avail = w.saturating_sub(prefix_len + ts_len);
            let cont_avail = w.saturating_sub(prefix_len).max(1);

            // Build lines: first line has prefix + body_chunk + timestamp,
            // continuation lines have padding + body_chunk (aligned under body).
            let padding = " ".repeat(prefix_len);
            let mut lines: Vec<Line> = Vec::new();

            if body_chars.len() <= first_avail || first_avail == 0 {
                // Single line
                let spans = vec![
                    Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                    Span::styled(reply_prefix, Style::default().fg(s::dim())),
                    Span::styled(author.clone(), name_style),
                    Span::raw("  "),
                    Span::raw(full_body.clone()),
                    Span::styled(ts_suffix.clone(), Style::default().fg(s::dim())),
                ];
                lines.push(Line::from(spans).style(Style::default().bg(actual_bg)));
            } else {
                // Multi-line: first line
                let first_chunk: String = body_chars[..first_avail].iter().collect();
                let first_spans = vec![
                    Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                    Span::styled(reply_prefix, Style::default().fg(s::dim())),
                    Span::styled(author.clone(), name_style),
                    Span::raw("  "),
                    Span::raw(first_chunk),
                ];
                lines.push(Line::from(first_spans).style(Style::default().bg(actual_bg)));

                // Continuation lines
                let mut pos = first_avail;
                while pos < body_chars.len() {
                    let end = (pos + cont_avail).min(body_chars.len());
                    let chunk: String = body_chars[pos..end].iter().collect();
                    let is_last = end >= body_chars.len();
                    let line = if is_last {
                        Line::from(vec![
                            Span::raw(padding.clone()),
                            Span::raw(chunk),
                            Span::styled(ts_suffix.clone(), Style::default().fg(s::dim())),
                        ])
                    } else {
                        Line::from(vec![Span::raw(padding.clone()), Span::raw(chunk)])
                    };
                    lines.push(line.style(Style::default().bg(actual_bg)));
                    pos = end;
                }
            }

            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(actual_bg)),
                area,
            );
        }
        RenderRow::CollapsedThread {
            author,
            author_uuid,
            is_mine,
            preview,
            reply_count,
            timestamp,
            ..
        } => {
            let author_color = if *is_mine {
                s::accent()
            } else {
                s::username_color(author_uuid)
            };
            let line = Line::from(vec![
                Span::styled(
                    author.clone(),
                    Style::default()
                        .fg(author_color)
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
            ]);
            frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
        }
        RenderRow::Input {
            thread_uuid,
            reply_to_uuid: _,
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
            let line = Line::from(spans);
            frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
        }
    }
}

fn build_indent(indent: u8) -> String {
    match indent {
        0 => String::new(),
        1 => "  +- ".to_string(),
        _ => "  |  +- ".to_string(),
    }
}
