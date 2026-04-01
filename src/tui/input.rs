use crossterm::event::{Event as CtEvent, KeyCode, KeyModifiers};

use super::app::{AppState, ContactsModalState, ModalState, Mode, SearchModalState};
use super::views::command_bar::{CommandAction, CommandBarState};

/// Extract the mention query currently being typed: the text between the last
/// unmatched `@` before the cursor and the cursor itself.  Returns `None` when
/// the cursor is not inside an `@…` sequence.
fn current_mention_query(content: &str, cursor: usize) -> Option<&str> {
    let chars: Vec<char> = content.chars().collect();
    let at_pos = chars[..cursor].iter().rposition(|&c| c == '@')?;
    // If there's a space between the @ and cursor the user has moved on.
    if chars[at_pos + 1..cursor].contains(&' ') {
        return None;
    }
    // Return the byte slice corresponding to the query characters.
    let byte_at: usize = chars[..at_pos + 1].iter().map(|c| c.len_utf8()).sum();
    let byte_cursor: usize = chars[..cursor].iter().map(|c| c.len_utf8()).sum();
    Some(&content[byte_at..byte_cursor])
}

pub enum Action {
    Continue,
    Quit,
}

pub fn handle(app: &mut AppState, event: CtEvent) -> Action {
    // Command bar intercepts all input
    if app.command_bar.is_some() {
        return handle_command_bar(app, event);
    }

    // Modal intercepts when active
    if app.modal.is_some() {
        return handle_modal(app, event);
    }

    match app.mode {
        Mode::ChatPicker => handle_picker(app, event),
        Mode::Chat => handle_chat(app, event),
    }
}

fn handle_picker(app: &mut AppState, event: CtEvent) -> Action {
    match event {
        CtEvent::Key(key) => match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => Action::Quit,
            (KeyModifiers::NONE, KeyCode::Up) => {
                app.picker_select_prev();
                Action::Continue
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                app.picker_select_next();
                Action::Continue
            }
            (KeyModifiers::NONE, KeyCode::Enter) => {
                app.picker_join_selected();
                Action::Continue
            }
            // Accept first pending invitation with 'i'
            (KeyModifiers::NONE, KeyCode::Char('i')) => {
                if !app.invitations.is_empty() {
                    app.accept_invitation(0);
                }
                Action::Continue
            }
            (KeyModifiers::NONE, KeyCode::Char('/')) => {
                let mut bar = CommandBarState::new();
                bar.update_suggestions(app);
                app.command_bar = Some(bar);
                Action::Continue
            }
            _ => Action::Continue,
        },
        _ => Action::Continue,
    }
}

fn handle_chat(app: &mut AppState, event: CtEvent) -> Action {
    // Check if we're in editing mode
    if app.active_pane().editing.is_some() {
        return handle_editing(app, event);
    }

    match event {
        CtEvent::Key(key) => match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => Action::Quit,

            (KeyModifiers::NONE, KeyCode::Up) => {
                // Load older messages when at the top of the list
                if app.active_pane().selected == 0 && app.msg_store.has_more_above {
                    let prev_count = app.render_row_count();
                    app.load_scrollback();
                    // Adjust selection to keep position stable
                    let new_count = app.render_row_count();
                    let added = new_count.saturating_sub(prev_count);
                    app.panes[app.active_pane].selected += added;
                }
                app.move_selection(-1);
                Action::Continue
            }
            (KeyModifiers::NONE, KeyCode::Down) => {
                app.move_selection(1);
                Action::Continue
            }

            (KeyModifiers::NONE, KeyCode::Right) => {
                app.expand_thread();
                Action::Continue
            }
            (KeyModifiers::NONE, KeyCode::Left) => {
                app.collapse_thread();
                Action::Continue
            }

            (KeyModifiers::NONE, KeyCode::Enter) => {
                app.activate();
                Action::Continue
            }

            (KeyModifiers::ALT, KeyCode::Char('\\')) => {
                app.split_pane_vertical();
                Action::Continue
            }
            (KeyModifiers::ALT, KeyCode::Char('-')) => {
                app.split_pane_horizontal();
                Action::Continue
            }
            (KeyModifiers::ALT, KeyCode::Char('w')) => {
                app.close_pane();
                Action::Continue
            }
            (KeyModifiers::ALT, KeyCode::Left) => {
                app.focus_pane(-1);
                Action::Continue
            }
            (KeyModifiers::ALT, KeyCode::Right) => {
                app.focus_pane(1);
                Action::Continue
            }

            (KeyModifiers::NONE, KeyCode::Char('/')) => {
                let mut bar = CommandBarState::new();
                bar.update_suggestions(app);
                app.command_bar = Some(bar);
                Action::Continue
            }

            (KeyModifiers::NONE, KeyCode::End) => {
                app.jump_to_latest();
                Action::Continue
            }

            (KeyModifiers::NONE, KeyCode::Esc) => {
                app.modal = None;
                Action::Continue
            }

            _ => Action::Continue,
        },
        CtEvent::Resize(_, _) => Action::Continue,
        _ => Action::Continue,
    }
}

fn handle_editing(app: &mut AppState, event: CtEvent) -> Action {
    match event {
        CtEvent::Key(key) => match key.code {
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => Action::Quit,

            // --- Mention popup: navigate and complete ---
            KeyCode::Up if app.mention_popup.is_some() => {
                if let Some(ref mut popup) = app.mention_popup {
                    if popup.selected > 0 {
                        popup.selected -= 1;
                    }
                }
                Action::Continue
            }
            KeyCode::Down if app.mention_popup.is_some() => {
                if let Some(ref mut popup) = app.mention_popup {
                    let max = popup.matches.len().saturating_sub(1);
                    if popup.selected < max {
                        popup.selected += 1;
                    }
                }
                Action::Continue
            }
            KeyCode::Enter if app.mention_popup.is_some() => {
                app.complete_mention();
                Action::Continue
            }
            KeyCode::Tab if app.mention_popup.is_some() => {
                app.complete_mention();
                Action::Continue
            }
            KeyCode::Esc if app.mention_popup.is_some() => {
                app.mention_popup = None;
                Action::Continue
            }

            // --- Normal editing ---
            KeyCode::Enter => {
                app.send_current_input();
                Action::Continue
            }
            KeyCode::Esc => {
                app.mention_popup = None;
                app.active_pane_mut().editing = None;
                Action::Continue
            }
            KeyCode::Up => {
                app.move_selection(-1);
                Action::Continue
            }
            KeyCode::Down => {
                app.move_selection(1);
                Action::Continue
            }
            KeyCode::End => {
                app.mention_popup = None;
                app.active_pane_mut().editing = None;
                app.jump_to_latest();
                Action::Continue
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                app.mention_popup = None;
                app.active_pane_mut().move_cursor_to_start();
                Action::Continue
            }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                app.mention_popup = None;
                app.active_pane_mut().move_cursor_to_end();
                Action::Continue
            }
            KeyCode::Char('b') if key.modifiers == KeyModifiers::ALT => {
                app.mention_popup = None;
                app.active_pane_mut().move_cursor_word_back();
                Action::Continue
            }
            KeyCode::Char('f') if key.modifiers == KeyModifiers::ALT => {
                app.mention_popup = None;
                app.active_pane_mut().move_cursor_word_forward();
                Action::Continue
            }
            KeyCode::Backspace if key.modifiers == KeyModifiers::ALT => {
                app.mention_popup = None;
                app.active_pane_mut().delete_word_back();
                Action::Continue
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                app.active_pane_mut().pop_char_forward();
                Action::Continue
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::ALT => {
                app.mention_popup = None;
                app.active_pane_mut().delete_word_forward();
                Action::Continue
            }
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                app.active_pane_mut().push_char(c);
                app.maybe_send_typing_notify();

                // Manage the mention popup.
                if c == '@' {
                    app.open_mention_popup();
                } else if let Some(ref mut popup) = app.mention_popup {
                    // Append to the query and re-filter.
                    let mut q = popup.query.clone();
                    q.push(c);
                    let q_lower = q.to_lowercase();
                    popup.query = q;
                    popup.matches = app
                        .members
                        .iter()
                        .filter(|m| {
                            m.uuid != app.my_uuid
                                && m.screenname.to_lowercase().starts_with(&q_lower)
                        })
                        .map(|m| (m.uuid.clone(), m.screenname.clone()))
                        .collect();
                    popup.selected = 0;
                }
                Action::Continue
            }
            KeyCode::Backspace => {
                app.active_pane_mut().pop_char();

                // Keep popup in sync: recompute query from buffer state.
                if app.mention_popup.is_some() {
                    let (content, cursor) = {
                        let pane = app.active_pane();
                        let target = match &pane.editing {
                            Some(t) => t.clone(),
                            None => {
                                app.mention_popup = None;
                                return Action::Continue;
                            }
                        };
                        let content = pane.inputs.get(&target).cloned().unwrap_or_default();
                        let cursor = pane
                            .cursor_positions
                            .get(&target)
                            .copied()
                            .unwrap_or(content.chars().count());
                        (content, cursor)
                    };
                    match current_mention_query(&content, cursor) {
                        Some(q) => app.update_mention_query(q),
                        None => app.mention_popup = None,
                    }
                }
                Action::Continue
            }
            KeyCode::Left => {
                app.active_pane_mut().move_cursor_left();
                // Moving left might exit the @… region — close popup if so.
                if app.mention_popup.is_some() {
                    let (content, cursor) = {
                        let pane = app.active_pane();
                        if let Some(target) = &pane.editing {
                            let c = pane.inputs.get(target).cloned().unwrap_or_default();
                            let cur = pane
                                .cursor_positions
                                .get(target)
                                .copied()
                                .unwrap_or(c.chars().count());
                            (c, cur)
                        } else {
                            (String::new(), 0)
                        }
                    };
                    if current_mention_query(&content, cursor).is_none() {
                        app.mention_popup = None;
                    }
                }
                Action::Continue
            }
            KeyCode::Right => {
                app.active_pane_mut().move_cursor_right();
                Action::Continue
            }
            _ => Action::Continue,
        },
        _ => Action::Continue,
    }
}

fn handle_command_bar(app: &mut AppState, event: CtEvent) -> Action {
    match event {
        CtEvent::Key(key) => match key.code {
            KeyCode::Esc => {
                app.command_bar = None;
                Action::Continue
            }
            KeyCode::Enter => {
                // Take the bar out, execute, then handle result
                let action = app.command_bar.as_mut().and_then(|b| b.execute());
                app.command_bar = None;
                if let Some(action) = action {
                    execute_command(app, action)
                } else {
                    Action::Continue
                }
            }
            KeyCode::Up => {
                if let Some(ref mut bar) = app.command_bar {
                    bar.select_prev();
                }
                Action::Continue
            }
            KeyCode::Down => {
                if let Some(ref mut bar) = app.command_bar {
                    bar.select_next();
                }
                Action::Continue
            }
            KeyCode::Tab => {
                if let Some(ref mut bar) = app.command_bar {
                    bar.select_next();
                    // Complete the input text to the selected suggestion so
                    // the user can see exactly what Enter will run.
                    if let Some(suggestion) = bar.suggestions.get(bar.selected_suggestion) {
                        bar.input = suggestion.label.trim_start_matches('/').to_string();
                    }
                }
                Action::Continue
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                // Take the bar out temporarily to avoid borrow conflict with update_suggestions
                if let Some(mut bar) = app.command_bar.take() {
                    bar.input.push(c);
                    bar.update_suggestions(app);
                    app.command_bar = Some(bar);
                }
                Action::Continue
            }
            KeyCode::Backspace => {
                if let Some(mut bar) = app.command_bar.take() {
                    bar.input.pop();
                    bar.update_suggestions(app);
                    app.command_bar = Some(bar);
                }
                Action::Continue
            }
            _ => Action::Continue,
        },
        _ => Action::Continue,
    }
}

fn handle_modal(app: &mut AppState, event: CtEvent) -> Action {
    match event {
        CtEvent::Key(key) => {
            // Delegate to invite modal
            if matches!(&app.modal, Some(ModalState::InviteCreate(_))) {
                let close = if let Some(ModalState::InviteCreate(ref mut s)) = app.modal {
                    super::views::invite::handle_input(key, s)
                } else {
                    false
                };
                if close {
                    app.modal = None;
                }
                return Action::Continue;
            }

            // Delegate to search modal
            if matches!(&app.modal, Some(ModalState::Search(_))) {
                use super::views::search::{handle_input as search_input, SearchAction};
                let action = if let Some(ModalState::Search(ref mut s)) = app.modal {
                    Some(search_input(key, s))
                } else {
                    None
                };
                match action {
                    Some(SearchAction::Close) => app.modal = None,
                    Some(SearchAction::SelectUser(uuid)) => {
                        let chat_uuid = app.current_chat.clone();
                        if let Some(ref cu) = chat_uuid {
                            app.pending_add_member = Some((cu.clone(), uuid.to_string()));
                        }
                        app.modal = None;
                    }
                    Some(SearchAction::QueryChanged) | Some(SearchAction::Continue) | None => {}
                }
                return Action::Continue;
            }

            // Delegate to contacts modal
            if matches!(&app.modal, Some(ModalState::Contacts(_))) {
                use super::views::contacts::{handle_input as contacts_input, ContactsAction};
                let close = if let Some(ModalState::Contacts(ref mut s)) = app.modal {
                    matches!(contacts_input(key, s), ContactsAction::Close)
                } else {
                    false
                };
                if close {
                    app.modal = None;
                }
                return Action::Continue;
            }

            // Group settings modal: handle [L] for leave, [R] for key rotation
            if matches!(&app.modal, Some(ModalState::GroupSettings)) {
                match key.code {
                    KeyCode::Char('l') | KeyCode::Char('L') => {
                        app.pending_leave_chat = true;
                        app.modal = None;
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        app.modal = None;
                        app.rotate_and_distribute_keys();
                    }
                    KeyCode::Esc => {
                        app.modal = None;
                    }
                    _ => {}
                }
                return Action::Continue;
            }

            match key.code {
                KeyCode::Esc => {
                    app.modal = None;
                    Action::Continue
                }
                _ => Action::Continue,
            }
        }
        _ => Action::Continue,
    }
}

fn execute_command(app: &mut AppState, action: CommandAction) -> Action {
    match action {
        CommandAction::Quit => Action::Quit,
        CommandAction::MemberList => {
            app.modal = Some(ModalState::MemberList);
            Action::Continue
        }
        CommandAction::GroupSettings => {
            app.modal = Some(ModalState::GroupSettings);
            Action::Continue
        }
        CommandAction::InviteCreate => {
            use crate::tui::app::InviteModalState;
            let mut s = InviteModalState::new();
            s.creating_spawned = false;
            app.modal = Some(ModalState::InviteCreate(s));
            // Fetch existing invites for this chat
            app.pending_list_invites = true;
            Action::Continue
        }
        CommandAction::SwitchChat(uuid) => {
            app.join_chat(uuid);
            Action::Continue
        }
        CommandAction::Search => {
            app.modal = Some(ModalState::Search(SearchModalState::new()));
            Action::Continue
        }
        CommandAction::Contacts => {
            let contacts = app
                .contact_store
                .as_ref()
                .and_then(|cs| cs.all(100).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.uuid.to_string() != app.my_uuid)
                .collect();
            let mut modal = ContactsModalState::new(contacts);
            modal.chat_names = app
                .chat_list
                .iter()
                .map(|c| (c.uuid.clone(), c.topic.clone()))
                .collect();
            app.modal = Some(ModalState::Contacts(modal));
            Action::Continue
        }
        CommandAction::JoinByCode(code) => {
            if !code.is_empty() {
                let c = code.to_uppercase();
                let c = c.strip_prefix("SQWOK-").unwrap_or(&c);
                app.pending_redeem = Some(c.replace('-', ""));
            }
            Action::Continue
        }
        CommandAction::RotateKeys => {
            app.rotate_and_distribute_keys();
            Action::Continue
        }
    }
}
