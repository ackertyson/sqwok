use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::pane::{InputTarget, Pane};
use super::store::TuiMessageStore;

/// Indicates whether a message row has a thread and its expansion state,
/// used to render the gutter triangle indicator.
#[derive(Debug, Clone, PartialEq)]
pub enum Gutter {
    None,
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone)]
pub enum RenderRow {
    Message {
        uuid: String,
        author: String,
        author_color: ratatui::style::Color,
        body: String,
        timestamp: String,
        indent: u8,
        thread_uuid: Option<String>,
        reply_to_uuid: Option<String>,
        is_pending: bool,
        highlight_age: Option<std::time::Duration>,
        /// For depth-1 messages with a collapsed subthread, the number of hidden depth-2 replies.
        collapsed_sub_count: Option<usize>,
        /// True when peers are typing in this depth-1 message's collapsed subthread.
        sub_typing_active: bool,
        /// Thread indicator for the left-margin gutter.
        gutter: Gutter,
        /// True when this message (or any hidden reply within a collapsed sub) is unread.
        is_unread: bool,
    },
    CollapsedThread {
        uuid: String,
        author: String,
        author_color: ratatui::style::Color,
        preview: String,
        reply_count: u32,
        timestamp: String,
        /// True when peers are typing anywhere in this collapsed thread.
        typing_active: bool,
        /// True when the header or any hidden reply in this thread is unread.
        is_unread: bool,
    },
    Input {
        thread_uuid: Option<String>,
        reply_to_uuid: Option<String>,
        indent: u8,
        is_active: bool,
        content: String,
        /// Cursor position as a char index within `content`.
        cursor: usize,
    },
    TypingIndicator {
        indent: u8,
    },
}

/// Build the flat render row list for a pane.
///
/// `member_order` is a slice of member UUIDs in join order, used to assign
/// deterministic per-session colors. Pass `&[]` if members aren't yet known.
pub fn build(
    pane: &Pane,
    msg_store: &TuiMessageStore,
    my_uuid: &str,
    member_order: &[&str],
    name_cache: &HashMap<String, String>,
    highlights: &HashMap<String, Instant>,
    typing_indicators: &HashSet<(Option<String>, Option<String>)>,
) -> Vec<RenderRow> {
    let mut rows = Vec::new();

    let display_name = |sender_uuid: &str, fallback: &str| -> String {
        if sender_uuid == my_uuid {
            "me".to_string()
        } else {
            name_cache
                .get(sender_uuid)
                .cloned()
                .unwrap_or_else(|| fallback.to_string())
        }
    };

    let author_color = |sender_uuid: &str| -> ratatui::style::Color {
        if sender_uuid == my_uuid {
            super::style::accent()
        } else {
            let idx = member_order
                .iter()
                .position(|&uid| uid == sender_uuid)
                .unwrap_or(0);
            super::style::username_color_by_index(idx)
        }
    };

    for top_uuid in &msg_store.top_level {
        let msg = match msg_store.by_uuid.get(top_uuid) {
            Some(m) => m,
            None => continue,
        };

        let reply_count = msg_store.reply_count(top_uuid) as u32;
        let is_expanded = pane.expanded.contains(top_uuid.as_str());

        if reply_count > 0 && !is_expanded {
            // Collapsed thread
            let preview: String = msg.body.chars().take(40).collect();
            let typing_active = typing_indicators
                .iter()
                .any(|(t, _)| t.as_deref() == Some(top_uuid.as_str()));
            let has_unread_replies = msg_store
                .threads
                .get(top_uuid)
                .map(|replies| {
                    replies
                        .iter()
                        .any(|r| msg_store.by_uuid.get(r).map(|m| !m.read).unwrap_or(false))
                })
                .unwrap_or(false);
            rows.push(RenderRow::CollapsedThread {
                uuid: top_uuid.clone(),
                author: display_name(&msg.sender_uuid, &msg.sender_name),
                author_color: author_color(&msg.sender_uuid),
                preview,
                reply_count,
                timestamp: format_timestamp(&msg.timestamp),
                typing_active,
                is_unread: !msg.read || has_unread_replies,
            });
        } else {
            let top_gutter = if reply_count > 0 {
                Gutter::Expanded
            } else {
                Gutter::None
            };
            rows.push(RenderRow::Message {
                uuid: top_uuid.clone(),
                author: display_name(&msg.sender_uuid, &msg.sender_name),
                author_color: author_color(&msg.sender_uuid),
                body: msg.body.clone(),
                timestamp: format_timestamp(&msg.timestamp),
                indent: 0,
                thread_uuid: None,
                reply_to_uuid: msg.reply_to_uuid.clone(),
                is_pending: msg.pending,
                highlight_age: highlights.get(top_uuid).map(|i| i.elapsed()),
                collapsed_sub_count: None,
                sub_typing_active: false,
                gutter: top_gutter,
                is_unread: !msg.read,
            });

            // Thread replies if expanded
            if is_expanded {
                let replies = msg_store.threads.get(top_uuid).cloned().unwrap_or_default();

                // Partition into depth-1 (no reply_to) and depth-2 (has reply_to)
                let (d1_uuids, d2_uuids): (Vec<_>, Vec<_>) = replies.iter().partition(|u| {
                    msg_store
                        .by_uuid
                        .get(*u)
                        .map(|m| m.reply_to_uuid.is_none())
                        .unwrap_or(true)
                });

                for reply_uuid in d1_uuids.iter() {
                    let reply = match msg_store.by_uuid.get(*reply_uuid) {
                        Some(m) => m,
                        None => continue,
                    };
                    let collapsed_sub_count = if pane.collapsed_subs.contains(reply_uuid.as_str()) {
                        Some(
                            d2_uuids
                                .iter()
                                .filter(|u| {
                                    msg_store
                                        .by_uuid
                                        .get(u.as_str())
                                        .map(|m| {
                                            m.reply_to_uuid.as_deref() == Some(reply_uuid.as_str())
                                        })
                                        .unwrap_or(false)
                                })
                                .count(),
                        )
                    } else {
                        None
                    };
                    let sub_typing_active = typing_indicators
                        .iter()
                        .any(|(_, r)| r.as_deref() == Some(reply_uuid.as_str()));
                    let has_sub_replies = d2_uuids.iter().any(|u| {
                        msg_store
                            .by_uuid
                            .get(u.as_str())
                            .map(|m| m.reply_to_uuid.as_deref() == Some(reply_uuid.as_str()))
                            .unwrap_or(false)
                    });
                    let d1_gutter = if collapsed_sub_count.is_some() {
                        Gutter::Collapsed
                    } else if has_sub_replies {
                        Gutter::Expanded
                    } else {
                        Gutter::None
                    };
                    let has_unread_collapsed_subs = collapsed_sub_count.is_some()
                        && d2_uuids.iter().any(|u| {
                            msg_store
                                .by_uuid
                                .get(u.as_str())
                                .map(|m| {
                                    m.reply_to_uuid.as_deref() == Some(reply_uuid.as_str())
                                        && !m.read
                                })
                                .unwrap_or(false)
                        });
                    rows.push(RenderRow::Message {
                        uuid: reply_uuid.to_string(),
                        author: display_name(&reply.sender_uuid, &reply.sender_name),
                        author_color: author_color(&reply.sender_uuid),
                        body: reply.body.clone(),
                        timestamp: format_timestamp(&reply.timestamp),
                        indent: 1,
                        thread_uuid: Some(top_uuid.clone()),
                        reply_to_uuid: None,
                        is_pending: reply.pending,
                        highlight_age: highlights.get(*reply_uuid).map(|i| i.elapsed()),
                        collapsed_sub_count,
                        sub_typing_active,
                        gutter: d1_gutter,
                        is_unread: !reply.read || has_unread_collapsed_subs,
                    });

                    // Depth-2 replies — only shown when the subthread is not collapsed.
                    if !pane.collapsed_subs.contains(reply_uuid.as_str()) {
                        for sub_uuid in d2_uuids.iter() {
                            let sub = match msg_store.by_uuid.get(*sub_uuid) {
                                Some(m) => m,
                                None => continue,
                            };
                            if sub.reply_to_uuid.as_deref() != Some(reply_uuid.as_str()) {
                                continue;
                            }
                            rows.push(RenderRow::Message {
                                uuid: sub_uuid.to_string(),
                                author: display_name(&sub.sender_uuid, &sub.sender_name),
                                author_color: author_color(&sub.sender_uuid),
                                body: sub.body.clone(),
                                timestamp: format_timestamp(&sub.timestamp),
                                indent: 2,
                                thread_uuid: Some(top_uuid.clone()),
                                reply_to_uuid: sub.reply_to_uuid.clone(),
                                is_pending: sub.pending,
                                highlight_age: highlights.get(*sub_uuid).map(|i| i.elapsed()),
                                collapsed_sub_count: None,
                                sub_typing_active: false,
                                gutter: Gutter::None,
                                is_unread: !sub.read,
                            });
                        }

                        // Depth-2 typing indicator (before the reply input)
                        if typing_indicators
                            .contains(&(Some(top_uuid.clone()), Some(reply_uuid.to_string())))
                        {
                            rows.push(RenderRow::TypingIndicator { indent: 2 });
                        }

                        // Inline depth-2 reply input if active for this depth-1 message
                        let reply_target =
                            InputTarget::Reply(reply_uuid.to_string(), top_uuid.clone());
                        if pane.editing.as_ref() == Some(&reply_target) {
                            let content =
                                pane.inputs.get(&reply_target).cloned().unwrap_or_default();
                            let cursor = pane
                                .cursor_positions
                                .get(&reply_target)
                                .copied()
                                .unwrap_or(content.chars().count());
                            rows.push(RenderRow::Input {
                                thread_uuid: Some(top_uuid.clone()),
                                reply_to_uuid: Some(reply_uuid.to_string()),
                                indent: 2,
                                is_active: true,
                                content,
                                cursor,
                            });
                        }
                    }
                }

                // Depth-1 typing indicator (before the thread input)
                if typing_indicators.contains(&(Some(top_uuid.clone()), None)) {
                    rows.push(RenderRow::TypingIndicator { indent: 1 });
                }

                // Thread input prompt at depth-1
                let thread_input_target = InputTarget::Thread(top_uuid.clone());
                let is_editing = pane.editing.as_ref() == Some(&thread_input_target);
                let content = pane
                    .inputs
                    .get(&thread_input_target)
                    .cloned()
                    .unwrap_or_default();
                let cursor = pane
                    .cursor_positions
                    .get(&thread_input_target)
                    .copied()
                    .unwrap_or(content.chars().count());
                rows.push(RenderRow::Input {
                    thread_uuid: Some(top_uuid.clone()),
                    reply_to_uuid: None,
                    indent: 1,
                    is_active: is_editing,
                    content,
                    cursor,
                });
            }
        }
    }

    // Top-level typing indicator (before main chat input)
    if typing_indicators.contains(&(None, None)) {
        rows.push(RenderRow::TypingIndicator { indent: 0 });
    }

    // Main chat input at bottom
    let is_editing_main = pane.editing.as_ref() == Some(&InputTarget::MainChat);
    let main_content = pane
        .inputs
        .get(&InputTarget::MainChat)
        .cloned()
        .unwrap_or_default();
    let main_cursor = pane
        .cursor_positions
        .get(&InputTarget::MainChat)
        .copied()
        .unwrap_or(main_content.chars().count());
    rows.push(RenderRow::Input {
        thread_uuid: None,
        reply_to_uuid: None,
        indent: 0,
        is_active: is_editing_main,
        content: main_content,
        cursor: main_cursor,
    });

    rows
}

fn format_timestamp(ts: &str) -> String {
    use chrono::{DateTime, Local};
    if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
        let local = dt.with_timezone(&Local);
        let today = Local::now().date_naive();
        if local.date_naive() == today {
            return local.format("%H:%M").to_string();
        } else {
            return local.format("%b %d %H:%M").to_string();
        }
    }
    // Fallback: extract HH:MM directly from ISO 8601 string (UTC)
    if ts.len() >= 16 {
        ts[11..16].to_string()
    } else {
        ts.to_string()
    }
}
