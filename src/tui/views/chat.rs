use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use crate::tui::{
    app::{AppState, Gutter, RenderRow},
    pane::Pane,
    style as s,
};

pub fn draw_top_bar(frame: &mut Frame, area: Rect, app: &AppState) {
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
    frame.render_widget(Paragraph::new(topic_line), area);
}

pub fn draw_bottom_bar(frame: &mut Frame, area: Rect) {
    let mut status_spans = vec![
        Span::styled(
            format!("sqwok v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(s::dim()),
        ),
        Span::raw("  "),
    ];
    let hint = s::hint_line(&[
        ("↑↓", "nav"),
        ("Enter", "new msg"),
        ("Esc", "cancel input"),
        ("→/←", "thread show/hide"),
        ("/", "cmd"),
        ("Alt+\\", "vpane"),
        ("Alt+-", "hpane"),
        ("Ctrl+c", "quit"),
    ]);
    status_spans.extend(hint.spans);
    frame.render_widget(Paragraph::new(Line::from(status_spans)), area);
}

/// Width of the left-margin gutter (symbol + space).
const GUTTER_WIDTH: u16 = 2;

pub fn draw_messages(frame: &mut Frame, area: Rect, app: &AppState, pane: &Pane) {
    // Clear the entire area first so that stale characters from previous renders
    // (e.g. "No messages yet..." or the scrollback hint) don't bleed through when
    // new content is shorter. Ratatui's buf.set_style only updates styles, not cell
    // content, so without this old characters persist beyond what new spans overwrite.
    frame.render_widget(Clear, area);

    let rows = app.build_render_rows_for_pane(pane);
    let total_rows = rows.len();

    if total_rows == 0 {
        let empty = Paragraph::new("No messages yet. Press Enter to start typing.")
            .style(Style::default().fg(s::dim()));
        frame.render_widget(empty, area);
        return;
    }

    // Reserve left gutter; content area is inset by GUTTER_WIDTH.
    let content_width = area.width.saturating_sub(GUTTER_WIDTH);
    let content_x = area.x + GUTTER_WIDTH;

    // Show "scroll up for older messages" hint when there's more history.
    // Rendered in the first row; message rendering starts on the row below.
    let hint_shown = app.msg_store.has_more_above && pane.scroll_offset == 0 && pane.selected == 0;
    if hint_shown {
        let hint =
            Paragraph::new("↑ scroll for older messages").style(Style::default().fg(s::dim()));
        let hint_area = Rect::new(content_x, area.y, content_width, 1);
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
    // If the scrollback hint is shown, it occupies the first row.
    let mut y = if hint_shown { area.y + 1 } else { area.y };
    let area_bottom = area.y + area.height;
    let mut last_rendered_row = scroll_offset;

    for (abs_idx, row) in rows.iter().enumerate().skip(scroll_offset) {
        if y >= area_bottom {
            break;
        }
        let is_selected = abs_idx == selected;
        let row_height = row_visual_height(row, content_width);
        let available = area_bottom.saturating_sub(y);
        let render_height = row_height.min(available);

        // Draw gutter symbol (▶/▼) then message content alongside it.
        let gutter_area = Rect::new(area.x, y, GUTTER_WIDTH, render_height);
        draw_gutter(frame, gutter_area, row, is_selected);
        let row_area = Rect::new(content_x, y, content_width, render_height);
        draw_row(frame, row_area, row, is_selected, content_width);

        y += row_height;
        last_rendered_row = abs_idx;
    }

    // Unread-above indicator: count unread rows hidden above the viewport.
    let unread_above = rows[..scroll_offset]
        .iter()
        .filter(|r| match r {
            RenderRow::Message { is_unread, .. } => *is_unread,
            RenderRow::CollapsedThread { is_unread, .. } => *is_unread,
            _ => false,
        })
        .count();
    if unread_above > 0 {
        let text = format!("↑ {} new", unread_above);
        let w = text.chars().count() as u16 + 1;
        let indicator = Paragraph::new(text).style(
            Style::default()
                .fg(s::accent())
                .add_modifier(Modifier::BOLD),
        );
        let ind_area = Rect::new(area.x + area.width.saturating_sub(w), area.y, w, 1);
        frame.render_widget(indicator, ind_area);
    }

    // Unread-below indicator: count unread rows hidden below the viewport.
    if last_rendered_row + 1 < total_rows {
        let unread_below = rows[last_rendered_row + 1..]
            .iter()
            .filter(|r| match r {
                RenderRow::Message { is_unread, .. } => *is_unread,
                RenderRow::CollapsedThread { is_unread, .. } => *is_unread,
                _ => false,
            })
            .count();
        if unread_below > 0 {
            let text = format!("↓ {} new", unread_below);
            let w = text.chars().count() as u16 + 1;
            let indicator = Paragraph::new(text).style(
                Style::default()
                    .fg(s::accent())
                    .add_modifier(Modifier::BOLD),
            );
            let ind_area = Rect::new(
                area.x + area.width.saturating_sub(w),
                area.y + area.height.saturating_sub(1),
                w,
                1,
            );
            frame.render_widget(indicator, ind_area);
        }
    }
}

/// Render the left-margin gutter triangle for a row.
fn draw_gutter(frame: &mut Frame, area: Rect, row: &RenderRow, is_selected: bool) {
    let symbol = match row {
        RenderRow::CollapsedThread { .. } => "▶",
        RenderRow::Message {
            gutter: Gutter::Collapsed,
            ..
        } => "▶",
        RenderRow::Message {
            gutter: Gutter::Expanded,
            ..
        } => "▼",
        _ => " ",
    };
    let color = if is_selected { s::accent() } else { s::dim() };
    let line = Line::from(Span::styled(symbol, Style::default().fg(color)));
    frame.render_widget(Paragraph::new(line), area);
}

/// Compute how many terminal lines a row will occupy at the given width.
fn row_visual_height(row: &RenderRow, width: u16) -> u16 {
    if let RenderRow::Input {
        indent,
        is_active,
        content,
        ..
    } = row
    {
        if !is_active || width == 0 {
            return 1;
        }
        let prefix_len = indent_width(*indent) + 2; // "> "
        let avail = (width as usize).saturating_sub(prefix_len).max(1);
        // Include cursor char in wrap calculation
        let cursor_content = format!("{}_", content);
        return wrap_words(&cursor_content, avail, avail)
            .len()
            .max(1)
            .min(area_height_cap()) as u16;
    }

    let RenderRow::Message {
        indent,
        author,
        body,
        timestamp,
        reply_to_uuid,
        is_pending,
        collapsed_sub_count,
        ..
    } = row
    else {
        return 1;
    };
    if width == 0 {
        return 1;
    }
    let w = width as usize;
    let indent_len = indent_width(*indent);
    let reply_len = if reply_to_uuid.is_some() { 2usize } else { 0 };
    // prefix: indent + reply + author + "  "
    let prefix_len = indent_len + reply_len + author.chars().count() + 2;
    // suffix on last line: optional "[N replies]  " + "  " + timestamp
    let replies_tag_len = collapsed_sub_count
        .map(|n| format!("  [{} replies]", n).chars().count())
        .unwrap_or(0);
    let ts_len = replies_tag_len + 2 + timestamp.chars().count();
    let first_avail = w.saturating_sub(prefix_len + ts_len);
    let cont_avail = w.saturating_sub(prefix_len).max(1);
    if first_avail == 0 {
        return 1;
    }
    let pending_suffix = if *is_pending { " sending..." } else { "" };
    let full_body = format!("{}{}", body, pending_suffix);
    wrap_words(&full_body, first_avail, cont_avail)
        .len()
        .min(area_height_cap()) as u16
}

/// Cap maximum row height to avoid pathological cases.
#[inline]
fn area_height_cap() -> usize {
    20
}

/// Word-wrap `text` into lines. The first line has at most `first_width` chars,
/// continuation lines have at most `cont_width` chars. Words longer than the
/// available width are hard-broken at the character level.
fn wrap_words(text: &str, first_width: usize, cont_width: usize) -> Vec<String> {
    let avail = |line_idx: usize| -> usize {
        (if line_idx == 0 {
            first_width
        } else {
            cont_width
        })
        .max(1)
    };

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in text.split_whitespace() {
        let word_chars: Vec<char> = word.chars().collect();
        let word_len = word_chars.len();

        if current_len == 0 {
            // At the start of a line — place word or hard-break it
            let w = avail(lines.len());
            if word_len <= w {
                current.push_str(word);
                current_len = word_len;
            } else {
                let mut pos = 0;
                while pos < word_len {
                    let w = avail(lines.len());
                    let end = (pos + w).min(word_len);
                    let chunk: String = word_chars[pos..end].iter().collect();
                    current.push_str(&chunk);
                    current_len += end - pos;
                    if end < word_len {
                        lines.push(std::mem::take(&mut current));
                        current_len = 0;
                    }
                    pos = end;
                }
            }
        } else {
            let w = avail(lines.len());
            if current_len + 1 + word_len <= w {
                // Word fits on current line with a space
                current.push(' ');
                current.push_str(word);
                current_len += 1 + word_len;
            } else {
                // Flush current line and start a new one
                lines.push(std::mem::take(&mut current));
                current_len = 0;
                let w = avail(lines.len());
                if word_len <= w {
                    current.push_str(word);
                    current_len = word_len;
                } else {
                    // Hard-break the oversized word
                    let mut pos = 0;
                    while pos < word_len {
                        let w = avail(lines.len());
                        let end = (pos + w).min(word_len);
                        let chunk: String = word_chars[pos..end].iter().collect();
                        current.push_str(&chunk);
                        current_len += end - pos;
                        if end < word_len {
                            lines.push(std::mem::take(&mut current));
                            current_len = 0;
                        }
                        pos = end;
                    }
                }
            }
        }
    }

    lines.push(current);
    lines
}

fn draw_row(frame: &mut Frame, area: Rect, row: &RenderRow, is_selected: bool, avail_width: u16) {
    let bg = if is_selected {
        s::selection_bg()
    } else {
        s::BG
    };

    match row {
        RenderRow::TypingIndicator { indent } => {
            let indent_str = build_indent(*indent);
            let line = Line::from(vec![
                Span::styled(indent_str, Style::default().fg(s::dim())),
                Span::styled(
                    "...",
                    Style::default()
                        .fg(s::typing_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(Paragraph::new(line).style(Style::default().bg(bg)), area);
        }
        RenderRow::Message {
            author,
            author_color,
            body,
            timestamp,
            indent,
            is_pending,
            highlight_age,
            reply_to_uuid,
            collapsed_sub_count,
            sub_typing_active,
            is_unread,
            ..
        } => {
            let actual_bg = if is_selected {
                s::selection_bg()
            } else if highlight_age.map(|a| a.as_millis() < 1000).unwrap_or(false) {
                s::highlight_bg()
            } else if *is_unread {
                s::unread_bg()
            } else {
                s::BG
            };

            let indent_str = build_indent(*indent);
            let reply_prefix = if reply_to_uuid.is_some() { "↩ " } else { "" };
            let name_style = Style::default()
                .fg(*author_color)
                .add_modifier(Modifier::BOLD);

            let pending_suffix = if *is_pending { " sending..." } else { "" };
            let full_body = format!("{}{}", body, pending_suffix);

            // Compute layout widths
            let w = avail_width as usize;
            let indent_chars = indent_str.chars().count();
            let reply_chars = reply_prefix.chars().count();
            let author_chars = author.chars().count();
            let prefix_len = indent_chars + reply_chars + author_chars + 2; // +2 for "  "
            let replies_tag = collapsed_sub_count.map(|n| format!("[{} replies]", n));
            let replies_tag_len = replies_tag
                .as_ref()
                .map(|s| 2 + s.chars().count())
                .unwrap_or(0);
            let ts_suffix = format!("  {}", timestamp);
            let ts_len = replies_tag_len + ts_suffix.chars().count();
            let first_avail = w.saturating_sub(prefix_len + ts_len);
            let cont_avail = w.saturating_sub(prefix_len).max(1);

            // Trailing spans appended after the body on the last line.
            let trailing_spans = |extra: Vec<Span<'static>>| -> Vec<Span<'static>> {
                let mut v = extra;
                if let Some(ref tag) = replies_tag {
                    v.push(Span::raw("  "));
                    v.push(Span::styled(tag.clone(), Style::default().fg(s::accent())));
                    if *sub_typing_active {
                        v.push(Span::styled(
                            " ...",
                            Style::default()
                                .fg(s::typing_color())
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                }
                v.push(Span::styled(
                    ts_suffix.clone(),
                    Style::default().fg(s::dim()),
                ));
                v
            };

            // Build lines: first line has prefix + body_chunk + trailing spans,
            // continuation lines have padding + body_chunk (aligned under body).
            let padding = " ".repeat(prefix_len);
            let mut lines: Vec<Line> = Vec::new();
            let wrapped = wrap_words(&full_body, first_avail, cont_avail);
            let n = wrapped.len();
            // Detect height mismatches: if the computed line count doesn't match the
            // allocated area height, stale buffer content can bleed through.
            if n as u16 != area.height {
                crate::dlog!(
                    "MSG HEIGHT MISMATCH: area.h={} lines={} w={} prefix={} ts={} body={:?}",
                    area.height,
                    n,
                    avail_width,
                    prefix_len,
                    ts_len,
                    &full_body[..full_body.len().min(40)]
                );
            }
            for (i, line_text) in wrapped.into_iter().enumerate() {
                let is_first = i == 0;
                let is_last = i + 1 == n;
                let body_style = Style::default().fg(s::fg());
                let line = if is_first && is_last {
                    let mut spans = vec![
                        Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                        Span::styled(reply_prefix.to_string(), Style::default().fg(s::dim())),
                        Span::styled(author.clone(), name_style),
                        Span::styled("  ", body_style),
                        Span::styled(line_text, body_style),
                    ];
                    spans.extend(trailing_spans(vec![]));
                    Line::from(spans)
                } else if is_first {
                    Line::from(vec![
                        Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                        Span::styled(reply_prefix.to_string(), Style::default().fg(s::dim())),
                        Span::styled(author.clone(), name_style),
                        Span::styled("  ", body_style),
                        Span::styled(line_text, body_style),
                    ])
                } else if is_last {
                    let mut spans = vec![
                        Span::raw(padding.clone()),
                        Span::styled(line_text, body_style),
                    ];
                    spans.extend(trailing_spans(vec![]));
                    Line::from(spans)
                } else {
                    Line::from(vec![
                        Span::raw(padding.clone()),
                        Span::styled(line_text, body_style),
                    ])
                };
                lines.push(line.style(Style::default().bg(actual_bg)));
            }

            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(actual_bg)),
                area,
            );
        }
        RenderRow::CollapsedThread {
            author,
            author_color,
            preview,
            reply_count,
            timestamp,
            typing_active,
            is_unread,
            ..
        } => {
            let actual_bg = if is_selected {
                s::selection_bg()
            } else if *is_unread {
                s::unread_bg()
            } else {
                s::BG
            };
            let author_color = *author_color;
            let mut spans = vec![
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
            ];
            if *typing_active {
                spans.push(Span::styled(
                    " ...",
                    Style::default()
                        .fg(s::typing_color())
                        .add_modifier(Modifier::BOLD),
                ));
            }
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                timestamp.clone(),
                Style::default().fg(s::dim()),
            ));
            let line = Line::from(spans);
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(actual_bg)),
                area,
            );
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
            if !is_active {
                let mut spans = vec![
                    Span::styled(indent_str, Style::default().fg(s::dim())),
                    Span::styled("> ", Style::default().fg(prompt_color)),
                ];
                if thread_uuid.is_some() {
                    spans.push(Span::styled("reply...", Style::default().fg(s::dim())));
                } else {
                    spans.push(Span::styled("message...", Style::default().fg(s::dim())));
                }
                frame.render_widget(
                    Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
                    area,
                );
            } else {
                let prefix_len = indent_str.chars().count() + 2; // "> "
                let avail = (avail_width as usize).saturating_sub(prefix_len).max(1);
                let padding = " ".repeat(prefix_len);

                // Wrap content+cursor together so the cursor drives line breaks
                let cursor_content = format!("{}_", content);
                let wrapped = wrap_words(&cursor_content, avail, avail);
                let n = wrapped.len();
                let mut lines: Vec<Line> = Vec::new();
                for (i, line_text) in wrapped.into_iter().enumerate() {
                    let is_first = i == 0;
                    let is_last = i + 1 == n;
                    // On the last line, strip the trailing "_" and re-add it as a styled span
                    let (body_text, cursor_span): (String, Option<Span>) = if is_last {
                        let mut chars = line_text.chars();
                        chars.next_back(); // remove "_"
                        (
                            chars.collect(),
                            Some(Span::styled("_", Style::default().fg(s::accent()))),
                        )
                    } else {
                        (line_text, None)
                    };
                    let mut spans: Vec<Span> = if is_first {
                        vec![
                            Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                            Span::styled("> ", Style::default().fg(prompt_color)),
                            Span::raw(body_text),
                        ]
                    } else {
                        vec![Span::raw(padding.clone()), Span::raw(body_text)]
                    };
                    if let Some(cur) = cursor_span {
                        spans.push(cur);
                    }
                    lines.push(Line::from(spans));
                }
                frame.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), area);
            }
        }
    }
}

fn indent_width(indent: u8) -> usize {
    match indent {
        0 => 0,
        1 => 5, // "  +- "
        _ => 8, // "  |  +- "
    }
}

fn build_indent(indent: u8) -> String {
    match indent {
        0 => String::new(),
        1 => "  +- ".to_string(),
        _ => "  |  +- ".to_string(),
    }
}
