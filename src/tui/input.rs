use crossterm::event::{Event as CtEvent, KeyCode, KeyModifiers};

use super::app::{AppState, ContactsModalState, ModalKind, Mode, SearchModalState};
use super::views::command_bar::{CommandAction, CommandBarState};

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
                    app.panes[app.active_pane].scroll_offset += added;
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

            (KeyModifiers::NONE, KeyCode::Char('r')) => {
                app.reply_to_selected();
                Action::Continue
            }

            (KeyModifiers::SHIFT, KeyCode::Char('G')) | (KeyModifiers::NONE, KeyCode::End) => {
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
            KeyCode::Enter => {
                app.send_current_input();
                Action::Continue
            }
            KeyCode::Esc => {
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
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                app.active_pane_mut().push_char(c);
                Action::Continue
            }
            KeyCode::Backspace => {
                app.active_pane_mut().pop_char();
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
            if let Some(ModalKind::InviteCreate) = &app.modal {
                if let Some(ref mut inv_state) = app.invite_modal {
                    let close = super::views::invite::handle_input(key, inv_state);
                    if close {
                        app.modal = None;
                        app.invite_modal = None;
                    }
                }
                return Action::Continue;
            }

            // Delegate to search modal
            if let Some(ModalKind::Search) = &app.modal {
                if let Some(ref mut search_state) = app.search_modal {
                    use super::views::search::{handle_input as search_input, SearchAction};
                    match search_input(key, search_state) {
                        SearchAction::Close => {
                            app.modal = None;
                            app.search_modal = None;
                        }
                        SearchAction::SelectUser(uuid) => {
                            // Add user to current chat via member API
                            if let Some(ref chat_uuid) = app.current_chat {
                                app.pending_add_member =
                                    Some((chat_uuid.clone(), uuid.to_string()));
                            }
                            app.modal = None;
                            app.search_modal = None;
                        }
                        SearchAction::QueryChanged | SearchAction::Continue => {}
                    }
                }
                return Action::Continue;
            }

            // Delegate to contacts modal
            if let Some(ModalKind::Contacts) = &app.modal {
                if let Some(ref mut contacts_state) = app.contacts_modal {
                    use super::views::contacts::{handle_input as contacts_input, ContactsAction};
                    if matches!(contacts_input(key, contacts_state), ContactsAction::Close) {
                        app.modal = None;
                        app.contacts_modal = None;
                    }
                }
                return Action::Continue;
            }

            // Group settings modal: handle [L] for leave, [R] for key rotation
            if let Some(ModalKind::GroupSettings) = &app.modal {
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
            app.modal = Some(ModalKind::MemberList);
            Action::Continue
        }
        CommandAction::GroupSettings => {
            app.modal = Some(ModalKind::GroupSettings);
            Action::Continue
        }
        CommandAction::InviteCreate => {
            use crate::tui::app::InviteModalState;
            app.invite_modal = Some(InviteModalState::new());
            app.modal = Some(ModalKind::InviteCreate);
            // Fetch existing invites for this chat
            app.pending_list_invites = true;
            Action::Continue
        }
        CommandAction::SwitchChat(uuid) => {
            app.join_chat(uuid);
            Action::Continue
        }
        CommandAction::Search => {
            app.search_modal = Some(SearchModalState::new());
            app.modal = Some(ModalKind::Search);
            Action::Continue
        }
        CommandAction::Contacts => {
            let contacts = app
                .contact_store
                .as_ref()
                .and_then(|cs| cs.all(100).ok())
                .unwrap_or_default();
            let mut modal = ContactsModalState::new(contacts);
            modal.chat_names = app
                .chat_list
                .iter()
                .map(|c| (c.uuid.clone(), c.topic.clone()))
                .collect();
            app.contacts_modal = Some(modal);
            app.modal = Some(ModalKind::Contacts);
            Action::Continue
        }
        CommandAction::JoinByCode(code) => {
            if !code.is_empty() {
                app.pending_redeem = Some(code.to_uppercase().replace('-', ""));
            }
            Action::Continue
        }
        CommandAction::RotateKeys => {
            app.rotate_and_distribute_keys();
            Action::Continue
        }
    }
}
