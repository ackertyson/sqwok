pub mod app;
pub mod event;
pub mod input;
pub mod pane;
pub mod render;
pub mod render_rows;
pub mod store;
pub mod style;
pub mod views;

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use tokio::sync::mpsc;
use uuid::Uuid;

use self::app::{AppState, InviteStep, ModalState};
use self::event::{AppEvent, EventCollector};
use self::input::Action;

pub struct Tui {
    pub terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
}

impl Tui {
    pub fn new() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn restore(&mut self) -> anyhow::Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub async fn run(
    app: &mut AppState,
    tui: &mut Tui,
    events: &mut EventCollector,
    http: reqwest::Client,
) -> anyhow::Result<()> {
    // Select first chat in picker if list is non-empty
    if !app.chat_list.is_empty() {
        app.picker_state.select(Some(0));
    }

    let event_tx = events.sender();

    loop {
        tui.terminal.draw(|frame| render::draw(frame, app))?;

        match events.next().await {
            Some(AppEvent::Input(evt)) => {
                if matches!(input::handle(app, evt), Action::Quit) {
                    app.save_scroll_position();
                    return Ok(());
                }
                // After handling input, check for pending async operations
                maybe_spawn_invite_create(app, &event_tx, &http);
                maybe_spawn_redeem(app, &event_tx, &http);
                maybe_spawn_leave_chat(app, &event_tx, &http);
                maybe_spawn_search(app, &event_tx, &http);
                maybe_spawn_add_member(app, &event_tx, &http);
                maybe_spawn_list_invites(app, &event_tx, &http);
                maybe_spawn_revoke_invite(app, &event_tx, &http);
            }
            Some(AppEvent::Frame(frame)) => {
                if frame.topic.starts_with("user:") && frame.event == "chat:added" {
                    let chat_uuid = frame.payload["chat_uuid"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let topic = frame.payload["topic"].as_str().unwrap_or("").to_string();
                    let invited_by = frame.payload["invited_by"].as_str().map(|s| s.to_string());
                    if !chat_uuid.is_empty() {
                        app.handle_chat_added(chat_uuid, topic, invited_by);
                    }
                } else if frame.topic.starts_with("user:") && frame.event == "chat:removed" {
                    let chat_uuid = frame.payload["chat_uuid"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    // If we're currently viewing this chat, exit to picker
                    if app.current_chat.as_deref() == Some(&chat_uuid) {
                        app.current_chat = None;
                        app.chat_channel = None;
                        app.clear_chat_state();
                        app.mode = app::Mode::ChatPicker;
                    }
                    app.chat_list.retain(|c| c.uuid != chat_uuid);
                    app.toast = Some((
                        "You were removed from a chat".to_string(),
                        std::time::Instant::now() + std::time::Duration::from_secs(4),
                    ));
                } else {
                    app.handle_frame(&frame);
                }
            }
            Some(AppEvent::Tick) => {
                app.tick();
            }
            Some(AppEvent::ConnectionLost(reason)) => {
                app.connection_status = app::ConnStatus::Disconnected {
                    reason,
                    since: std::time::Instant::now(),
                };
            }
            Some(AppEvent::Reconnected) => {
                app.connection_status = app::ConnStatus::Connected;
                app.toast = Some((
                    "Reconnected".to_string(),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
                // Re-join personal notification channel
                let join_frame = crate::channel::protocol::Frame::join(
                    &format!("user:{}", app.my_uuid),
                    serde_json::json!({}),
                );
                let _ = app.ws_tx.send(join_frame.encode());
                // Re-join current chat channel if active
                if let Some(ref mut ch) = app.chat_channel {
                    let join_frame = ch.join_frame();
                    let _ = app.ws_tx.send(join_frame.encode());
                }
                // Fire any pending invite redeem (set via `sqwok join <code>`)
                maybe_spawn_redeem(app, &event_tx, &http);
            }
            Some(AppEvent::InviteCreated(info)) => {
                if let Some(ModalState::InviteCreate(ref mut inv_modal)) = app.modal {
                    inv_modal.created_code = Some(format!("sqwok-{}", info.display_code));
                    inv_modal.step = InviteStep::Display;
                }
                app.toast = Some((
                    format!("Invite created: sqwok-{}", info.display_code),
                    std::time::Instant::now() + std::time::Duration::from_secs(5),
                ));
            }
            Some(AppEvent::InviteError(msg)) => {
                if let Some(ModalState::InviteCreate(ref mut inv_modal)) = app.modal {
                    inv_modal.error = Some(msg.clone());
                    inv_modal.step = InviteStep::Configure;
                    inv_modal.creating_spawned = false;
                }
                app.toast = Some((
                    format!("Invite error: {}", msg),
                    std::time::Instant::now() + std::time::Duration::from_secs(4),
                ));
            }
            Some(AppEvent::SearchResults { query, results }) => {
                if let Some(ModalState::Search(ref mut search)) = app.modal {
                    if search.query == query {
                        // Merge with existing results, deduplicating by UUID
                        let existing_uuids: std::collections::HashSet<Uuid> =
                            search.results.iter().map(|r| r.uuid).collect();
                        for r in results {
                            if !existing_uuids.contains(&r.uuid) {
                                search.results.push(r);
                            }
                        }
                        search.last_searched = query;
                    }
                }
            }
            Some(AppEvent::RedeemOk { chat_uuid, topic }) => {
                // Add to chat list if not already present, then auto-join
                if !app.chat_list.iter().any(|c| c.uuid == chat_uuid) {
                    app.chat_list.push(app::ChatSummary {
                        uuid: chat_uuid.clone(),
                        topic: if topic.is_empty() {
                            "Chat".to_string()
                        } else {
                            topic
                        },
                        description: None,
                        member_count: 0,
                    });
                }
                // Auto-join immediately — no need to "select from list"
                app.join_chat(chat_uuid);
            }
            Some(AppEvent::RedeemError(msg)) => {
                app.toast = Some((
                    format!("Join failed: {}", msg),
                    std::time::Instant::now() + std::time::Duration::from_secs(4),
                ));
            }
            Some(AppEvent::LeaveChatOk) => {
                if let Some(ref uuid) = app.current_chat.clone() {
                    app.chat_list.retain(|c| &c.uuid != uuid);
                }
                app.current_chat = None;
                app.chat_channel = None;
                app.clear_chat_state();
                app.mode = app::Mode::ChatPicker;
                app.pending_leave_chat = false;
                app.toast = Some((
                    "Left chat".to_string(),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
            Some(AppEvent::LeaveChatError(msg)) => {
                app.pending_leave_chat = false;
                app.toast = Some((
                    format!("Leave failed: {}", msg),
                    std::time::Instant::now() + std::time::Duration::from_secs(4),
                ));
            }
            Some(AppEvent::AddMemberOk {
                screenname,
                user_uuid,
                e2e_public_key,
            }) => {
                // Distribute keys to the newly added member
                if let Some(ref e2e_bytes) = e2e_public_key {
                    distribute_keys_to_member(app, &user_uuid, e2e_bytes);
                }
                app.toast = Some((
                    format!("Added {} to chat", screenname),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
            Some(AppEvent::AddMemberError(msg)) => {
                app.toast = Some((
                    format!("Add member failed: {}", msg),
                    std::time::Instant::now() + std::time::Duration::from_secs(4),
                ));
            }
            Some(AppEvent::InviteList(invites)) => {
                if let Some(ModalState::InviteCreate(ref mut inv_modal)) = app.modal {
                    inv_modal.active_invites = invites;
                }
            }
            Some(AppEvent::InviteRevoked(code)) => {
                if let Some(ModalState::InviteCreate(ref mut inv_modal)) = app.modal {
                    inv_modal.active_invites.retain(|i| i.code != code);
                }
                app.toast = Some((
                    format!("Invite {} revoked", code),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
            Some(AppEvent::InviteRevokeError(msg)) => {
                app.toast = Some((
                    format!("Revoke failed: {}", msg),
                    std::time::Instant::now() + std::time::Duration::from_secs(4),
                ));
            }
            None => return Ok(()),
        }
    }
}

// Spawn the invite-create HTTP task when the modal transitions to Creating.
fn maybe_spawn_invite_create(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    let (ttl, use_limit) = if let Some(ModalState::InviteCreate(ref mut modal)) = app.modal {
        if modal.step != InviteStep::Creating || modal.creating_spawned {
            return;
        }
        modal.creating_spawned = true;
        let ttl = views::invite::TTL_OPTIONS[modal.ttl_selection]
            .0
            .to_string();
        (ttl, modal.use_limit)
    } else {
        return;
    };

    {
        let chat_uuid_str = app.current_chat.clone().unwrap_or_default();
        let server_url = app.server_url.clone();
        let identity_dir = app.identity_dir.clone();
        let http = http.clone();
        let tx = event_tx.clone();

        tokio::spawn(async move {
            let chat_uuid = match Uuid::parse_str(&chat_uuid_str) {
                Ok(u) => u,
                Err(e) => {
                    let _ = tx.send(AppEvent::InviteError(format!("invalid chat UUID: {}", e)));
                    return;
                }
            };
            let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(AppEvent::InviteError(e.to_string()));
                    return;
                }
            };
            match crate::net::invites::create_invite(
                &http,
                &server_url,
                &token,
                chat_uuid,
                &ttl,
                use_limit,
            )
            .await
            {
                Ok(info) => {
                    let _ = tx.send(AppEvent::InviteCreated(info));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::InviteError(e.to_string()));
                }
            }
        });
    }
}

// Spawn the redeem HTTP task when pending_redeem is set.
fn maybe_spawn_redeem(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    let code = match app.pending_redeem.take() {
        Some(c) => c,
        None => return,
    };

    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(e) => {
                let _ = tx.send(AppEvent::RedeemError(e.to_string()));
                return;
            }
        };
        match crate::net::invites::redeem_invite(&http, &server_url, &token, &code).await {
            Ok((chat_uuid, topic)) => {
                let _ = tx.send(AppEvent::RedeemOk {
                    chat_uuid: chat_uuid.to_string(),
                    topic,
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::RedeemError(e.to_string()));
            }
        }
    });
}

// Spawn the leave-chat HTTP task when pending_leave_chat is set.
fn maybe_spawn_leave_chat(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    if !app.pending_leave_chat {
        return;
    }
    // We only spawn once; the flag is cleared on LeaveChatOk/Error
    let chat_uuid = match &app.current_chat {
        Some(u) => u.clone(),
        None => {
            app.pending_leave_chat = false;
            return;
        }
    };
    let my_uuid = app.my_uuid.clone();
    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();
    // Clear flag to avoid re-spawning while task is in flight
    app.pending_leave_chat = false;

    tokio::spawn(async move {
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(e) => {
                let _ = tx.send(AppEvent::LeaveChatError(e.to_string()));
                return;
            }
        };
        let url = format!("{}/api/chats/{}/members/{}", server_url, chat_uuid, my_uuid);
        match http
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let _ = tx.send(AppEvent::LeaveChatOk);
            }
            Ok(resp) => {
                let _ = tx.send(AppEvent::LeaveChatError(format!(
                    "server error: {}",
                    resp.status()
                )));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::LeaveChatError(e.to_string()));
            }
        }
    });
}

// Spawn a server search when the search modal query has changed.
fn maybe_spawn_search(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    // Extract query — releases borrow so we can access other fields below.
    let query = match &app.modal {
        Some(ModalState::Search(s)) if !s.query.is_empty() && s.query != s.last_searched => {
            s.query.clone()
        }
        _ => return,
    };

    // Mark as searched and do a synchronous local search.
    if let Some(ModalState::Search(ref mut search)) = app.modal {
        search.last_searched = query.clone();
    }
    let local_results = app
        .contact_store
        .as_ref()
        .and_then(|cs| cs.search(&query, 10).ok())
        .map(|hits| {
            hits.into_iter()
                .map(|c| crate::net::search::SearchResult {
                    uuid: c.uuid,
                    screenname: c.screenname,
                })
                .collect::<Vec<_>>()
        });
    if let Some(ModalState::Search(ref mut search)) = app.modal {
        if let Some(results) = local_results {
            search.results = results;
        }
    }

    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(_) => return,
        };
        if let Ok(results) =
            crate::net::search::search_users(&http, &server_url, &token, &query).await
        {
            let _ = tx.send(AppEvent::SearchResults { query, results });
        }
    });
}

// Spawn the add-member HTTP task when pending_add_member is set.
fn maybe_spawn_add_member(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    let (chat_uuid, user_uuid) = match app.pending_add_member.take() {
        Some(pair) => pair,
        None => return,
    };

    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(e) => {
                let _ = tx.send(AppEvent::AddMemberError(e.to_string()));
                return;
            }
        };
        let url = format!("{}/api/chats/{}/members", server_url, chat_uuid);
        match http
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({"user_uuid": user_uuid}))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                // Fetch the new member's e2e public key so we can distribute keys
                let e2e_key = fetch_e2e_key(&http, &server_url, &token, &user_uuid).await;
                let _ = tx.send(AppEvent::AddMemberOk {
                    screenname: user_uuid.clone(),
                    user_uuid,
                    e2e_public_key: e2e_key,
                });
            }
            Ok(resp) => {
                let _ = tx.send(AppEvent::AddMemberError(format!(
                    "server error: {}",
                    resp.status()
                )));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::AddMemberError(e.to_string()));
            }
        }
    });
}

// Spawn the invite-list HTTP task when pending_list_invites is set.
fn maybe_spawn_list_invites(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    if !app.pending_list_invites {
        return;
    }
    app.pending_list_invites = false;

    let chat_uuid_str = match &app.current_chat {
        Some(u) => u.clone(),
        None => return,
    };

    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let chat_uuid = match uuid::Uuid::parse_str(&chat_uuid_str) {
            Ok(u) => u,
            Err(_) => return,
        };
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(_) => return,
        };
        if let Ok(invites) =
            crate::net::invites::list_invites(&http, &server_url, &token, chat_uuid).await
        {
            let _ = tx.send(AppEvent::InviteList(invites));
        }
    });
}

// Spawn the revoke-invite HTTP task when invite modal has a pending_revoke.
fn maybe_spawn_revoke_invite(
    app: &mut AppState,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    http: &reqwest::Client,
) {
    let code = match app.modal.as_mut().and_then(|m| {
        if let ModalState::InviteCreate(ref mut inv) = m {
            inv.pending_revoke.take()
        } else {
            None
        }
    }) {
        Some(c) => c,
        None => return,
    };

    let server_url = app.server_url.clone();
    let identity_dir = app.identity_dir.clone();
    let http = http.clone();
    let tx = event_tx.clone();

    tokio::spawn(async move {
        let token = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(t) => t,
            Err(e) => {
                let _ = tx.send(AppEvent::InviteRevokeError(e.to_string()));
                return;
            }
        };
        match crate::net::invites::revoke_invite(&http, &server_url, &token, &code).await {
            Ok(()) => {
                let _ = tx.send(AppEvent::InviteRevoked(code));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::InviteRevokeError(e.to_string()));
            }
        }
    });
}

/// Fetch a user's e2e public key from the server. Returns raw key bytes or None.
async fn fetch_e2e_key(
    http: &reqwest::Client,
    server_url: &str,
    token: &str,
    user_uuid: &str,
) -> Option<Vec<u8>> {
    let url = format!("{}/api/users/{}/e2e_key", server_url, user_uuid);
    let resp: serde_json::Value = http
        .get(&url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let key_b64 = resp["e2e_public_key"].as_str()?;
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key_b64).ok()
}

/// Distribute current epoch keys to a newly added member.
fn distribute_keys_to_member(app: &mut AppState, user_uuid: &str, e2e_public_bytes: &[u8]) {
    let chat = match &app.chat_channel {
        Some(c) => c,
        None => return,
    };
    let crypto = match &chat.crypto {
        Some(c) => c,
        None => return,
    };

    // Parse the member's Ed25519 key and convert to X25519
    let arr: [u8; 32] = match e2e_public_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return,
    };
    let peer_ed25519 = match ed25519_dalek::VerifyingKey::from_bytes(&arr) {
        Ok(k) => k,
        Err(_) => return,
    };
    let peer_x25519 = match crate::crypto::identity::ed25519_to_x25519_public(&peer_ed25519) {
        Some(k) => k,
        None => return,
    };

    // Store the peer key for future use
    let _ = chat.store.store_peer_key(user_uuid, e2e_public_bytes);

    // Prepare and send key bundle (all epochs for new member)
    if let Ok(bundle) = crypto.prepare_key_bundle(&peer_x25519, true) {
        let wire = crate::crypto::bundle_to_wire_payload(&bundle, user_uuid);
        let frame = chat.frame("key:distribute", wire);
        let _ = app.ws_tx.send(frame.encode());
    }
}
