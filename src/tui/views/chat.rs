use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::{
    app::{AppState, Gutter, MentionState, RenderRow},
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

    // Build the right-side status string first so we can reserve space for it.
    let total = app.members.len();
    let online = app.online_count();
    let members_text = format!("{} members", total);
    let online_text = format!("  {} online", online);
    let right_text = format!("{}{}  {}", members_text, online_text, keys_indicator);
    let right_width = (right_text.len() as u16).min(area.width);
    let left_width = area.width.saturating_sub(right_width);

    // Left area: topic + optional description (clipped by ratatui if too long).
    let mut left_spans = vec![Span::styled(
        topic,
        Style::default()
            .fg(s::accent())
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(desc) = description {
        left_spans.push(Span::raw("  "));
        left_spans.push(Span::styled(desc, Style::default().fg(s::dim())));
    }
    let left_area = ratatui::layout::Rect::new(area.x, area.y, left_width, area.height);
    frame.render_widget(Paragraph::new(Line::from(left_spans)), left_area);

    // Right area: member count + online count + keys indicator — always visible.
    let right_spans = vec![
        Span::styled(members_text, Style::default().fg(s::dim())),
        Span::styled(online_text, Style::default().fg(s::fg())),
        Span::raw("  "),
        Span::styled(
            keys_indicator,
            Style::default().fg(if app.has_keys {
                s::success_color()
            } else {
                s::error_color()
            }),
        ),
    ];
    let right_area =
        ratatui::layout::Rect::new(area.x + left_width, area.y, right_width, area.height);
    frame.render_widget(Paragraph::new(Line::from(right_spans)), right_area);
}

pub fn draw_bottom_bar(frame: &mut Frame, area: Rect) {
    let mut status_spans = vec![
        Span::styled(
            format!("sqwok v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(s::dim()),
        ),
        Span::raw("  "),
    ];
    let hint1 = s::hint_line(&[
        ("↑↓", "nav"),
        ("Enter", "new msg"),
        ("Esc", "cancel input"),
        ("→/←", "thread show/hide"),
        ("/", "cmd"),
    ]);
    status_spans.extend(hint1.spans);

    let icon_style = Style::default().fg(s::pane_icon_fg()).bg(s::pane_icon_bg());
    for (key, icon) in [("Alt+\\", "▕▏"), ("Alt+-", "──")] {
        status_spans.push(Span::raw("  "));
        status_spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(s::shortcut_key_color()),
        ));
        status_spans.push(Span::raw(" "));
        status_spans.push(Span::styled(icon.to_string(), icon_style));
    }

    status_spans.push(Span::raw("  "));
    let hint2 = s::hint_line(&[("Alt+w", "close pane"), ("Ctrl+c", "quit")]);
    status_spans.extend(hint2.spans);
    frame.render_widget(Paragraph::new(Line::from(status_spans)), area);
}

/// Width of the left-margin gutter (symbol + space).
const GUTTER_WIDTH: u16 = 2;

pub fn draw_messages(frame: &mut Frame, area: Rect, app: &AppState, pane: &Pane, is_active: bool) {
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

    let visible_height = area.height as usize;
    let mid = visible_height / 2;

    // Clamp selected to valid row range.
    let selected = pane.selected.min(total_rows.saturating_sub(1));
    // Compute scroll_offset using actual visual line heights rather than a
    // simple row-count offset.  The naive `selected - mid` formula assumes
    // every row is one terminal line tall; wrapped/multi-line messages break
    // that assumption — the rows between scroll_offset and selected can
    // collectively exceed `area.height`, pushing the selected row (often the
    // Input prompt) past `area_bottom` so it is never rendered.
    //
    // Instead, walk backwards from `selected`, accumulating visual lines until
    // we reach ~half the viewport.  For single-line messages this produces the
    // same result as the old formula; for multi-line messages it ensures the
    // selected row is always visible.
    let scroll_offset = {
        let mut offset = selected;
        let mut lines_above = 0u16;
        let mid_u16 = mid as u16;
        while offset > 0 {
            let h = row_visual_height(&rows[offset - 1], content_width);
            if lines_above + h > mid_u16 {
                break;
            }
            lines_above += h;
            offset -= 1;
        }
        offset
    };

    // Show "scroll up for older messages" hint when there's more history and
    // the cursor is at the first row. Rendered in row 0; messages start below.
    let hint_shown = app.msg_store.has_more_above && selected == 0;
    if hint_shown {
        let hint =
            Paragraph::new("↑ scroll for older messages").style(Style::default().fg(s::dim()));
        let hint_area = Rect::new(content_x, area.y, content_width, 1);
        frame.render_widget(hint, hint_area);
    }

    // Render rows from scroll_offset, tracking current y position.
    // Each row may occupy multiple lines depending on message length.
    // If the scrollback hint is shown, it occupies the first row.
    let mut y = if hint_shown { area.y + 1 } else { area.y };
    let area_bottom = area.y + area.height;
    let mut last_rendered_row = scroll_offset;
    // Screen position of the cursor in the active input row, for popup placement.
    let mut input_cursor_screen: Option<(u16, u16)> = None;

    for (abs_idx, row) in rows.iter().enumerate().skip(scroll_offset) {
        if y >= area_bottom {
            break;
        }
        let is_selected = abs_idx == selected;
        let row_height = row_visual_height(row, content_width);
        let available = area_bottom.saturating_sub(y);
        let render_height = row_height.min(available);

        // Track cursor screen position for the active (is_active=true) input.
        if is_active && input_cursor_screen.is_none() {
            if let RenderRow::Input {
                is_active: true,
                content,
                cursor,
                indent,
                ..
            } = row
            {
                let prefix_len = indent_width(*indent) + 2; // "❯ "
                let avail = (content_width as usize).saturating_sub(prefix_len).max(1);
                let cursor_content = insert_cursor_marker(content, *cursor);
                let wrapped_c = wrap_words(&cursor_content, avail, avail);
                for (line_idx, wline) in wrapped_c.iter().enumerate() {
                    if let Some(marker_pos) = wline.find('\x01') {
                        use unicode_width::UnicodeWidthStr;
                        let col = wline[..marker_pos].width() as u16;
                        let cx = content_x + prefix_len as u16 + col;
                        let cy = y + line_idx as u16;
                        input_cursor_screen = Some((cx, cy));
                        break;
                    }
                }
            }
        }

        // Draw gutter symbol (▶/▼) then message content alongside it.
        let gutter_area = Rect::new(area.x, y, GUTTER_WIDTH, render_height);
        draw_gutter(frame, gutter_area, row, is_selected);
        let row_area = Rect::new(content_x, y, content_width, render_height);
        draw_row(frame, row_area, row, is_selected, content_width);

        y += row_height;
        last_rendered_row = abs_idx;
    }

    // Unread-above indicator: count unread rows hidden above the viewport.
    let (unread_above, mention_above) = count_unread_and_mentions(&rows[..scroll_offset]);
    if unread_above > 0 {
        let text = if mention_above {
            format!("↑ {} new*", unread_above)
        } else {
            format!("↑ {} new", unread_above)
        };
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
        let (unread_below, mention_below) =
            count_unread_and_mentions(&rows[last_rendered_row + 1..]);
        if unread_below > 0 {
            let text = if mention_below {
                format!("↓ {} new*", unread_below)
            } else {
                format!("↓ {} new", unread_below)
            };
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

    // Mention autocomplete popup — only for the active pane.
    if is_active {
        if let Some(ref popup) = app.mention_popup {
            draw_mention_popup(frame, area, popup, input_cursor_screen);
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
        cursor,
        ..
    } = row
    {
        if !is_active || width == 0 {
            return 1;
        }
        let prefix_len = indent_width(*indent) + 2;
        let avail = (width as usize).saturating_sub(prefix_len).max(1);
        // Include cursor char in wrap calculation at the correct position
        let cursor_content = insert_cursor_marker(content, *cursor);
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
        reply_to_uuid: _,
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
    // prefix: indent + author + "  "
    let prefix_len = indent_len + author.chars().count() + 2;
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

/// Insert a `\x01` marker at char position `cursor` in `s`.
/// The marker is used during wrapping so the cursor position influences line breaks,
/// then located in the wrapped output to render the block cursor.
fn insert_cursor_marker(s: &str, cursor: usize) -> String {
    let char_count = s.chars().count();
    let cursor = cursor.min(char_count);
    let byte_idx = s
        .char_indices()
        .nth(cursor)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let mut out = String::with_capacity(s.len() + 1);
    out.push_str(&s[..byte_idx]);
    out.push('\x01');
    out.push_str(&s[byte_idx..]);
    out
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

    // Hard-break a word that is wider than the available column width.
    // Appends completed lines to `lines`, leaves the last partial in `current`/`col`.
    let hard_break = |word: &str,
                      lines: &mut Vec<String>,
                      current: &mut String,
                      col: &mut usize,
                      avail: &dyn Fn(usize) -> usize| {
        for ch in word.chars() {
            let ch_w = ch.width().unwrap_or(0);
            let w = avail(lines.len());
            if *col + ch_w > w && *col > 0 {
                lines.push(std::mem::take(current));
                *col = 0;
            }
            current.push(ch);
            *col += ch_w;
        }
    };

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_col = 0usize; // display columns used on the current line

    for word in text.split_whitespace() {
        let word_w = word.width(); // display columns

        if current_col == 0 {
            let w = avail(lines.len());
            if word_w <= w {
                current.push_str(word);
                current_col = word_w;
            } else {
                hard_break(word, &mut lines, &mut current, &mut current_col, &avail);
            }
        } else {
            let w = avail(lines.len());
            if current_col + 1 + word_w <= w {
                current.push(' ');
                current.push_str(word);
                current_col += 1 + word_w;
            } else {
                lines.push(std::mem::take(&mut current));
                current_col = 0;
                let w = avail(lines.len());
                if word_w <= w {
                    current.push_str(word);
                    current_col = word_w;
                } else {
                    hard_break(word, &mut lines, &mut current, &mut current_col, &avail);
                }
            }
        }
    }

    lines.push(current);
    lines
}

/// Count unread rows and whether any of them mention the current user.
fn count_unread_and_mentions(rows: &[RenderRow]) -> (usize, bool) {
    rows.iter().fold((0usize, false), |(count, mention), r| {
        let (unread, is_mention) = match r {
            RenderRow::Message {
                is_unread,
                mentions_me,
                ..
            } => (*is_unread, *is_unread && *mentions_me),
            RenderRow::CollapsedThread {
                is_unread,
                mentions_me,
                ..
            } => (*is_unread, *is_unread && *mentions_me),
            _ => (false, false),
        };
        (count + unread as usize, mention || is_mention)
    })
}

/// Compute the primary background color for a row.
fn row_bg(
    is_selected: bool,
    is_unread: bool,
    mentions_me: bool,
    highlighted: bool,
) -> ratatui::style::Color {
    if is_selected {
        s::selection_bg()
    } else if highlighted {
        s::highlight_bg()
    } else if is_unread && mentions_me {
        s::mention_bg()
    } else if is_unread {
        s::unread_bg()
    } else {
        s::BG
    }
}

/// Compute the trailing/fade background color for a row.
fn row_trail_bg(
    is_selected: bool,
    show_unread_trail: bool,
    mentions_me: bool,
) -> ratatui::style::Color {
    if is_selected {
        s::selection_trail_bg()
    } else if show_unread_trail && mentions_me {
        s::mention_trail_bg()
    } else if show_unread_trail {
        s::unread_trail_bg()
    } else {
        s::BG
    }
}

fn draw_row(frame: &mut Frame, area: Rect, row: &RenderRow, is_selected: bool, avail_width: u16) {
    let bg = if is_selected {
        s::selection_bg()
    } else {
        s::BG
    };

    // Pre-extract unread + mention flags (with highlight priority) for use after the match.
    let (row_is_unread, row_mentions_me) = match row {
        RenderRow::Message {
            is_unread,
            mentions_me,
            highlight_age,
            ..
        } => {
            let unread =
                *is_unread && !highlight_age.map(|a| a.as_millis() < 1000).unwrap_or(false);
            (unread, *mentions_me)
        }
        RenderRow::CollapsedThread {
            is_unread,
            mentions_me,
            ..
        } => (*is_unread, *mentions_me),
        _ => (false, false),
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
            reply_to_uuid: _,
            collapsed_sub_count,
            sub_typing_active,
            is_unread,
            mentions_me,
            mentioned_names,
            ..
        } => {
            let highlighted = highlight_age.map(|a| a.as_millis() < 1000).unwrap_or(false);
            let actual_bg = row_bg(is_selected, *is_unread, *mentions_me, highlighted);
            let show_unread_trail = *is_unread && !is_selected && !highlighted;

            let indent_str = build_indent(*indent);
            let name_style = Style::default()
                .fg(*author_color)
                .add_modifier(Modifier::BOLD);

            let pending_suffix = if *is_pending { " sending..." } else { "" };
            let full_body = format!("{}{}", body, pending_suffix);

            // Compute layout widths (in terminal columns, not char count)
            let w = avail_width as usize;
            let indent_chars = indent_str.width();
            let author_chars = author.width();
            let prefix_len = indent_chars + author_chars + 2; // +2 for "  "
            let replies_tag = collapsed_sub_count.map(|n| format!("[{} replies]", n));
            let replies_tag_len = replies_tag.as_ref().map(|s| 2 + s.width()).unwrap_or(0);
            let ts_suffix = format!("  {}", timestamp);
            let ts_len = replies_tag_len + ts_suffix.width();
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
                    &full_body.chars().take(40).collect::<String>()
                );
            }
            let mention_style = Style::default()
                .fg(s::mention_inline_fg())
                .bg(s::mention_inline_bg());
            // Split `text` into body spans, highlighting any `@name` tokens.
            let body_line_spans = |text: String| -> Vec<Span<'static>> {
                let body_style = Style::default().fg(s::fg());
                crate::tui::mention::split_body_spans(&text, mentioned_names)
                    .into_iter()
                    .map(|(seg, is_mention)| {
                        if is_mention {
                            Span::styled(seg, mention_style)
                        } else {
                            Span::styled(seg, body_style)
                        }
                    })
                    .collect()
            };
            for (i, line_text) in wrapped.into_iter().enumerate() {
                let is_first = i == 0;
                let is_last = i + 1 == n;
                let body_style = Style::default().fg(s::fg());
                let line = if is_first && is_last {
                    let mut spans = vec![
                        Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                        Span::styled(author.clone(), name_style),
                        Span::styled("  ", body_style),
                    ];
                    spans.extend(body_line_spans(line_text));
                    spans.extend(trailing_spans(vec![]));
                    Line::from(spans)
                } else if is_first {
                    let mut spans = vec![
                        Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                        Span::styled(author.clone(), name_style),
                        Span::styled("  ", body_style),
                    ];
                    spans.extend(body_line_spans(line_text));
                    Line::from(spans)
                } else if is_last {
                    let mut spans = vec![Span::raw(padding.clone())];
                    spans.extend(body_line_spans(line_text));
                    spans.extend(trailing_spans(vec![]));
                    Line::from(spans)
                } else {
                    let mut spans = vec![Span::raw(padding.clone())];
                    spans.extend(body_line_spans(line_text));
                    Line::from(spans)
                };
                // For rows with a trail (selected or unread): pin bg on every text
                // span so the text area keeps actual_bg while the paragraph's bg
                // (para_bg) fills the empty trailing space with the vivid color.
                let line = if is_selected || show_unread_trail {
                    let spans = line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.bg(actual_bg)))
                        .collect::<Vec<_>>();
                    Line::from(spans)
                } else {
                    line
                };
                lines.push(line.style(Style::default().bg(actual_bg)));
            }

            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(row_trail_bg(
                    is_selected,
                    show_unread_trail,
                    *mentions_me,
                ))),
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
            mentions_me,
            ..
        } => {
            let actual_bg = row_bg(is_selected, *is_unread, *mentions_me, false);
            let show_unread_trail = *is_unread && !is_selected;
            let author_color = *author_color;
            let replies_tag = format!("[{} replies]", reply_count);
            // Compute how many columns are reserved for non-preview content so we
            // only truncate the preview when it would actually crowd out the
            // replies count or timestamp.
            let reserved = author.width()
                + 2  // "  " after author
                + 2  // "  " before replies tag
                + replies_tag.width()
                + 2  // "  " before timestamp
                + timestamp.width()
                + if *typing_active { 4 } else { 0 } // " ..."
                + 8; // leave some space in right margin for selected row highlight
            let preview_avail = (avail_width as usize).saturating_sub(reserved);
            let preview_display_width = preview.width();
            let truncated_preview: String = if preview_display_width <= preview_avail {
                preview.clone()
            } else {
                // Truncate to fit within preview_avail - 1 columns (reserving 1 for "…")
                let mut col = 0usize;
                let mut s = String::new();
                for ch in preview.chars() {
                    let ch_w = ch.width().unwrap_or(0);
                    if col + ch_w > preview_avail.saturating_sub(1) {
                        break;
                    }
                    s.push(ch);
                    col += ch_w;
                }
                s + "…"
            };
            let mut spans = vec![
                Span::styled(
                    author.clone(),
                    Style::default()
                        .fg(author_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::raw(truncated_preview),
                Span::raw("  "),
                Span::styled(replies_tag, Style::default().fg(s::accent())),
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
            let line = if is_selected || show_unread_trail {
                let spans = spans
                    .into_iter()
                    .map(|s| Span::styled(s.content, s.style.bg(actual_bg)))
                    .collect::<Vec<_>>();
                Line::from(spans)
            } else {
                Line::from(spans)
            };
            frame.render_widget(
                Paragraph::new(line).style(Style::default().bg(row_trail_bg(
                    is_selected,
                    show_unread_trail,
                    *mentions_me,
                ))),
                area,
            );
        }
        RenderRow::Input {
            thread_uuid: _,
            reply_to_uuid: _,
            indent,
            is_active,
            content,
            cursor,
        } => {
            let indent_str = build_indent(*indent);
            let prompt_color = if *is_active { s::accent() } else { s::dim() };
            if !is_active {
                let spans = vec![
                    Span::styled(indent_str, Style::default().fg(s::dim())),
                    Span::styled("❯ ", Style::default().fg(prompt_color)),
                    Span::styled("message...", Style::default().fg(s::dim())),
                ];
                frame.render_widget(
                    Paragraph::new(Line::from(spans)).style(Style::default().bg(bg)),
                    area,
                );
            } else {
                let prefix_len = indent_str.chars().count() + 2;
                let avail = (avail_width as usize).saturating_sub(prefix_len).max(1);
                let padding = " ".repeat(prefix_len);

                // Insert \x01 marker at cursor position so wrapping accounts for it,
                // then locate the marker in wrapped output to split before/after.
                let cursor_content = insert_cursor_marker(content, *cursor);
                let wrapped = wrap_words(&cursor_content, avail, avail);
                let mut lines: Vec<Line> = Vec::new();
                for (i, line_text) in wrapped.into_iter().enumerate() {
                    let is_first = i == 0;
                    // Split on the \x01 marker if present in this line.
                    // The first char of `remaining` is the cursor char (or space at EOL).
                    let mut spans: Vec<Span> = if is_first {
                        vec![
                            Span::styled(indent_str.clone(), Style::default().fg(s::dim())),
                            Span::styled("❯ ", Style::default().fg(prompt_color)),
                        ]
                    } else {
                        vec![Span::raw(padding.clone())]
                    };
                    if let Some(marker_pos) = line_text.find('\x01') {
                        let before = &line_text[..marker_pos];
                        let remaining = &line_text[marker_pos + 1..];
                        let (cursor_ch, after) = match remaining.chars().next() {
                            Some(ch) => (ch.to_string(), remaining[ch.len_utf8()..].to_string()),
                            None => (" ".to_string(), String::new()),
                        };
                        spans.push(Span::raw(before.to_string()));
                        spans.push(Span::styled(
                            cursor_ch,
                            Style::default().fg(s::BG).bg(s::accent()),
                        ));
                        if !after.is_empty() {
                            spans.push(Span::raw(after));
                        }
                    } else {
                        spans.push(Span::raw(line_text));
                    }
                    lines.push(Line::from(spans));
                }
                frame.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), area);
            }
        }
    }
    if is_selected {
        apply_selection_fade(frame, area);
    } else if row_is_unread && row_mentions_me {
        apply_mention_fade(frame, area);
    } else if row_is_unread {
        apply_unread_fade(frame, area);
    }
}

/// Generic trail-fade helper. After a paragraph is rendered with text spans
/// pinned to one bg and the paragraph bg set to `trail_bg`, this scans each
/// row right-to-left to find where the trailing `trail_bg` block begins, then
/// blends the first `fade_steps` cells using `fade_fn`.
///
/// Wide characters leave a continuation cell pre-filled with the paragraph bg;
/// the right-to-left scan ensures we only touch the true trailing block.
fn apply_trail_fade<F>(
    frame: &mut Frame,
    area: Rect,
    trail_bg: ratatui::style::Color,
    fade_steps: u16,
    fade_fn: F,
) where
    F: Fn(u16, u16) -> ratatui::style::Color,
{
    let trail_xs: Vec<Option<u16>> = (area.y..area.y + area.height)
        .map(|y| {
            let last_text_x = (area.x..area.x + area.width).rev().find(|&x| {
                frame
                    .buffer_mut()
                    .cell(Position { x, y })
                    .map(|c| c.bg != trail_bg)
                    .unwrap_or(false)
            });
            last_text_x
                .map(|x| x + 1)
                .filter(|&x| x < area.x + area.width)
        })
        .collect();

    for (row_idx, trail_x) in trail_xs.into_iter().enumerate() {
        let Some(tx) = trail_x else { continue };
        let y = area.y + row_idx as u16;
        let available = (area.x + area.width).saturating_sub(tx);
        let steps = fade_steps.min(available);
        for step in 0..steps {
            if let Some(cell) = frame.buffer_mut().cell_mut(Position { x: tx + step, y }) {
                cell.set_bg(fade_fn(step, fade_steps));
            }
        }
    }
}

fn apply_selection_fade(frame: &mut Frame, area: Rect) {
    apply_trail_fade(
        frame,
        area,
        s::selection_trail_bg(),
        s::selection_fade_steps(),
        s::selection_bg_fade,
    );
}

fn apply_unread_fade(frame: &mut Frame, area: Rect) {
    apply_trail_fade(
        frame,
        area,
        s::unread_trail_bg(),
        s::selection_fade_steps(),
        s::unread_bg_fade,
    );
}

fn apply_mention_fade(frame: &mut Frame, area: Rect) {
    apply_trail_fade(
        frame,
        area,
        s::mention_trail_bg(),
        s::selection_fade_steps(),
        s::mention_bg_fade,
    );
}

/// Draw the `@mention` autocomplete popup one line below the cursor.
///
/// `cursor_screen` is the `(x, y)` terminal position of the cursor in the
/// active input row (from tracking during the main render loop).  When absent
/// (cursor not on screen), the popup falls back to just above the bottom edge.
fn draw_mention_popup(
    frame: &mut Frame,
    area: Rect,
    popup: &MentionState,
    cursor_screen: Option<(u16, u16)>,
) {
    if popup.matches.is_empty() {
        return;
    }

    let display: Vec<_> = popup.matches.iter().take(5).collect();
    let popup_h = display.len() as u16;

    // Width: widest "@name" + 1 space padding on each side.
    let name_width = display
        .iter()
        .map(|(_, name)| name.chars().count() + 1) // 1 for '@'
        .max()
        .unwrap_or(8);
    let popup_w = (name_width as u16 + 2).min(area.width);

    // Anchor the popup's left edge at the cursor x, one row below the cursor.
    // Clamp so it stays inside the pane and doesn't overflow the bottom.
    let (anchor_x, anchor_y) =
        cursor_screen.unwrap_or((area.x + 2, area.y + area.height.saturating_sub(popup_h + 1)));
    // Popup appears one line BELOW the input line that contains the cursor.
    let popup_y = (anchor_y + 1).min(area.y + area.height.saturating_sub(popup_h));
    let popup_x = anchor_x.min(area.x + area.width.saturating_sub(popup_w));

    let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);
    frame.render_widget(Clear, popup_area);

    for (i, (_, name)) in display.iter().enumerate() {
        let y = popup_area.y + i as u16;
        let is_sel = i == popup.selected;
        let (fg, bg) = if is_sel {
            (s::BG, s::accent())
        } else {
            (s::fg(), s::selection_bg())
        };
        let text = format!("@{}", name);
        let row_area = Rect::new(popup_area.x, y, popup_area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text,
                Style::default().fg(fg).bg(bg),
            )))
            .style(Style::default().bg(bg)),
            row_area,
        );
    }
}

fn indent_width(indent: u8) -> usize {
    match indent {
        0 => 0,
        1 => 5, // "  └─ "
        _ => 8, // "  │  └─ "
    }
}

fn build_indent(indent: u8) -> String {
    match indent {
        0 => String::new(),
        1 => "  └─ ".to_string(),
        _ => "  │  └─ ".to_string(),
    }
}
