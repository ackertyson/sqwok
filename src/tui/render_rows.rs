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
        /// True when this message (or any hidden reply within a collapsed sub) mentions the current user.
        mentions_me: bool,
        /// Resolved screennames of everyone mentioned in this message body.
        mentioned_names: Vec<String>,
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
        /// True when the header or any reply in this collapsed thread mentions the current user.
        mentions_me: bool,
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
#[allow(clippy::too_many_arguments)]
pub fn build(
    pane: &Pane,
    msg_store: &TuiMessageStore,
    my_uuid: &str,
    member_order: &[&str],
    name_cache: &HashMap<String, String>,
    highlights: &HashMap<String, Instant>,
    typing_indicators: &HashSet<(Option<String>, Option<String>)>,
    blocked_uuids: &HashSet<String>,
) -> Vec<RenderRow> {
    let mut rows = Vec::new();

    let display_name = |sender_uuid: &str, fallback: &str| -> String {
        name_cache
            .get(sender_uuid)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
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

        // Skip entire thread when the top-level author is blocked.
        if blocked_uuids.contains(&msg.sender_uuid) {
            continue;
        }

        let reply_count = msg_store.reply_count(top_uuid) as u32;
        let is_expanded = pane.expanded.contains(top_uuid.as_str());

        if reply_count > 0 && !is_expanded {
            // Collapsed thread
            let preview: String = msg.body.clone();
            let typing_active = typing_indicators
                .iter()
                .any(|(t, _)| t.as_deref() == Some(top_uuid.as_str()));
            let thread_replies = msg_store.threads.get(top_uuid);
            let (replies_unread, replies_mention) =
                msg_store.unread_status(thread_replies.into_iter().flatten().map(String::as_str));
            rows.push(RenderRow::CollapsedThread {
                uuid: top_uuid.clone(),
                author: display_name(&msg.sender_uuid, &msg.sender_name),
                author_color: author_color(&msg.sender_uuid),
                preview,
                reply_count,
                timestamp: format_timestamp(&msg.timestamp),
                typing_active,
                is_unread: !msg.read || replies_unread,
                mentions_me: msg.mentions_me || replies_mention,
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
                mentions_me: msg.mentions_me,
                mentioned_names: msg.mentioned_names.clone(),
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

                    if blocked_uuids.contains(&reply.sender_uuid) {
                        continue;
                    }

                    // Collect d2 replies belonging to this d1 message once; reuse below.
                    let d2_for_reply: Vec<&String> = d2_uuids
                        .iter()
                        .copied()
                        .filter(|u| {
                            msg_store
                                .by_uuid
                                .get(u.as_str())
                                .map(|m| m.reply_to_uuid.as_deref() == Some(reply_uuid.as_str()))
                                .unwrap_or(false)
                        })
                        .collect();

                    let is_sub_collapsed = pane.collapsed_subs.contains(reply_uuid.as_str());
                    let collapsed_sub_count = is_sub_collapsed.then_some(d2_for_reply.len());
                    let sub_typing_active = typing_indicators
                        .iter()
                        .any(|(_, r)| r.as_deref() == Some(reply_uuid.as_str()));
                    let d1_gutter = if is_sub_collapsed {
                        Gutter::Collapsed
                    } else if !d2_for_reply.is_empty() {
                        Gutter::Expanded
                    } else {
                        Gutter::None
                    };
                    let (subs_unread, subs_mention) = if is_sub_collapsed {
                        msg_store.unread_status(d2_for_reply.iter().map(|u| u.as_str()))
                    } else {
                        (false, false)
                    };
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
                        is_unread: !reply.read || subs_unread,
                        mentions_me: reply.mentions_me || subs_mention,
                        mentioned_names: reply.mentioned_names.clone(),
                    });

                    // Depth-2 replies — only shown when the subthread is not collapsed.
                    if !is_sub_collapsed {
                        for sub_uuid in d2_for_reply.iter() {
                            let sub = match msg_store.by_uuid.get(sub_uuid.as_str()) {
                                Some(m) => m,
                                None => continue,
                            };

                            if blocked_uuids.contains(&sub.sender_uuid) {
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
                                highlight_age: highlights
                                    .get(sub_uuid.as_str())
                                    .map(|i| i.elapsed()),
                                collapsed_sub_count: None,
                                sub_typing_active: false,
                                gutter: Gutter::None,
                                is_unread: !sub.read,
                                mentions_me: sub.mentions_me,
                                mentioned_names: sub.mentioned_names.clone(),
                            });
                        }

                        // Depth-2 footer: typing indicator + reply input (active-only)
                        push_depth_footer(
                            &mut rows,
                            pane,
                            InputTarget::Reply(reply_uuid.to_string(), top_uuid.clone()),
                            &(Some(top_uuid.clone()), Some(reply_uuid.to_string())),
                            typing_indicators,
                            true,
                        );
                    }
                }

                // Depth-1 footer: typing indicator + thread input
                push_depth_footer(
                    &mut rows,
                    pane,
                    InputTarget::Thread(top_uuid.clone()),
                    &(Some(top_uuid.clone()), None),
                    typing_indicators,
                    false,
                );
            }
        }
    }

    // Depth-0 footer: typing indicator + main chat input
    push_depth_footer(
        &mut rows,
        pane,
        InputTarget::MainChat,
        &(None, None),
        typing_indicators,
        false,
    );

    rows
}

/// Append an Input row for `target` to `rows`. The thread/reply UUIDs and
/// indent are derived from the target itself. When `only_when_active` is true
/// the row is only appended if the pane is currently editing that target
/// (used for depth-2 inline reply inputs).
fn push_input_row(
    rows: &mut Vec<RenderRow>,
    pane: &Pane,
    target: InputTarget,
    only_when_active: bool,
) {
    let is_active = pane.editing.as_ref() == Some(&target);
    if only_when_active && !is_active {
        return;
    }
    let indent = target.indent();
    let (thread_uuid, reply_to_uuid) = target.to_wire_uuids();
    let content = pane.inputs.get(&target).cloned().unwrap_or_default();
    let cursor = pane
        .cursor_positions
        .get(&target)
        .copied()
        .unwrap_or(content.chars().count());
    rows.push(RenderRow::Input {
        thread_uuid,
        reply_to_uuid,
        indent,
        is_active,
        content,
        cursor,
    });
}

/// Append a TypingIndicator (if active) followed by an Input row for `target`.
/// This is the standard footer for every depth level.
fn push_depth_footer(
    rows: &mut Vec<RenderRow>,
    pane: &Pane,
    target: InputTarget,
    typing_key: &(Option<String>, Option<String>),
    typing_indicators: &HashSet<(Option<String>, Option<String>)>,
    only_input_when_active: bool,
) {
    if typing_indicators.contains(typing_key) {
        rows.push(RenderRow::TypingIndicator {
            indent: target.indent(),
        });
    }
    push_input_row(rows, pane, target, only_input_when_active);
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
