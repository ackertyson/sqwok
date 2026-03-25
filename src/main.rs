mod auth;
mod channel;
mod config;
mod crypto;
pub mod debug_log;
mod identity;
mod net;
mod storage;
mod tui;

use anyhow::Result;
use tokio::sync::mpsc;

use tui::app::{AppState, ChatSummary, Invitation};

#[derive(clap::Parser)]
#[command(name = "sqwok", about = "Terminal group chat with E2E encryption")]
struct Cli {
    /// Server URL (overrides SQWOK_SERVER env var)
    #[arg(long, global = true)]
    server: Option<String>,

    /// Identity directory (overrides default ~/.sqwok/identity)
    #[arg(long, global = true)]
    identity: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<SubCommand>,
}

#[derive(clap::Subcommand)]
enum SubCommand {
    /// Create a new chat and open it
    New {
        /// Chat topic
        topic: String,
        /// Optional description shown in the chat header and group list
        #[arg(long)]
        description: Option<String>,
    },
    /// Join a chat using an invite code
    Join {
        /// Invite code (e.g. ABCD-EF12)
        code: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    use clap::Parser;
    let cli = Cli::parse();

    let identity_dir = cli.identity.unwrap_or_else(config::identity_dir);
    let server_url = cli.server.unwrap_or_else(config::server_url);

    debug_log::init();
    dlog!("sqwok starting — server={}", server_url);

    // Registration if needed
    if !identity::credentials::is_registered(&identity_dir) {
        identity::registration::run_registration(&server_url, &identity_dir).await?;
    }

    // Read our user UUID and screenname
    let user_uuid_str = std::fs::read_to_string(identity_dir.join("user_uuid"))
        .map_err(|_| anyhow::anyhow!("user_uuid file missing — try re-registering"))?
        .trim()
        .to_string();

    let my_screenname = std::fs::read_to_string(identity_dir.join("screenname"))
        .unwrap_or_else(|_| user_uuid_str.chars().take(8).collect());

    // Build HTTP client (reused throughout)
    let http = reqwest::Client::new();

    // Channel for sending frames out over WS
    let (ws_out_tx, ws_out_rx) = mpsc::unbounded_channel::<String>();

    // Channel for incoming WS frames -> event system
    let (ws_in_tx, ws_in_rx) = mpsc::unbounded_channel::<String>();

    // Spawn WS supervisor (handles connect + reconnect with backoff)
    {
        let sv_server_url = server_url.clone();
        let sv_identity_dir = identity_dir.clone();
        let sv_ws_in_tx = ws_in_tx.clone();
        tokio::spawn(async move {
            net::ws::run_with_reconnect(sv_server_url, sv_identity_dir, ws_out_rx, sv_ws_in_tx)
                .await;
        });
    }

    // Spawn heartbeat task
    let hb_tx = ws_out_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let frame = channel::protocol::Frame::heartbeat();
            if hb_tx.send(frame.encode()).is_err() {
                break;
            }
        }
    });

    // Open contact store (best-effort)
    let contact_store = storage::contacts::ContactStore::open().ok();

    // Fetch server chat list (includes chats joined while offline)
    let (server_chat_list, fetch_error) =
        match fetch_chat_list(&server_url, &identity_dir, &http).await {
            Ok(list) => (list, None),
            Err(e) => {
                eprintln!("Warning: failed to fetch chat list: {}", e);
                (Vec::new(), Some(format!("Failed to fetch chats: {}", e)))
            }
        };

    // Detect new invitations: chats on server not yet in local SQLite
    let local_chat_uuids: std::collections::HashSet<String> = {
        let chats_dir = config::chats_dir()?;
        if chats_dir.exists() {
            std::fs::read_dir(&chats_dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        } else {
            std::collections::HashSet::new()
        }
    };

    let mut known_chats = Vec::new();
    let mut pending_invitations: Vec<Invitation> = Vec::new();
    for chat in &server_chat_list {
        if local_chat_uuids.contains(&chat.uuid) {
            known_chats.push(chat.clone());
        } else {
            pending_invitations.push(Invitation {
                chat_uuid: chat.uuid.clone(),
                topic: chat.topic.clone(),
                invited_by: None,
                received_at: chrono::Utc::now().timestamp(),
            });
        }
    }
    // If no local chats yet, treat all as known (first run)
    if local_chat_uuids.is_empty() {
        known_chats = server_chat_list;
        pending_invitations.clear();
    }

    // Build TUI app state
    let mut app = AppState::new(
        user_uuid_str.clone(),
        my_screenname.trim().to_string(),
        identity_dir.clone(),
        server_url.clone(),
        ws_out_tx.clone(),
    );
    app.chat_list = known_chats;
    app.invitations = pending_invitations;
    // Pre-populate name cache from local contact store so historical messages
    // show screennames even for offline senders.
    if let Some(ref cs) = contact_store {
        if let Ok(contacts) = cs.all(10_000) {
            for c in contacts {
                app.name_cache.insert(c.uuid.to_string(), c.screenname);
            }
        }
    }
    // Always map our own UUID to our current screenname (overrides any stale contact entry).
    app.name_cache
        .insert(user_uuid_str.clone(), app.my_screenname.clone());
    app.contact_store = contact_store;
    if let Some(err_msg) = fetch_error {
        app.toast = Some((
            err_msg,
            std::time::Instant::now() + std::time::Duration::from_secs(5),
        ));
    }

    // Handle CLI subcommands
    match cli.command {
        Some(SubCommand::New { topic, description }) => {
            let token = auth::token::build_token(&identity_dir, &server_url)?;
            match create_chat(&server_url, &token, &topic, description.as_deref(), &http).await {
                Ok(chat) => {
                    let uuid = chat.uuid.clone();

                    // Initialize E2E encryption keys for the new chat (we're the creator)
                    let chat_dir = config::chat_dir(&uuid)?;
                    let _ = std::fs::create_dir_all(&chat_dir);
                    let _ = crypto::ChatCrypto::create_new(&identity_dir, &chat_dir);

                    app.chat_list.push(chat);
                    app.pending_redeem = None;
                    app.picker_state.select(Some(app.chat_list.len() - 1));
                    app.join_chat(uuid);
                }
                Err(e) => {
                    eprintln!("Failed to create chat: {}", e);
                    return Err(e);
                }
            }
        }
        Some(SubCommand::Join { code }) => {
            // Let the run loop handle redeeming after WS connects
            app.pending_redeem = Some(code.to_uppercase().replace('-', ""));
        }
        None => {}
    }

    let mut tui_instance = tui::Tui::new()?;
    let mut events = tui::event::EventCollector::new(ws_in_rx);

    let result = tui::run(&mut app, &mut tui_instance, &mut events, http).await;
    tui_instance.restore()?;

    result
}

async fn fetch_chat_list(
    server_url: &str,
    identity_dir: &std::path::Path,
    http: &reqwest::Client,
) -> Result<Vec<ChatSummary>> {
    let token = auth::token::build_token(identity_dir, server_url)?;
    let resp: serde_json::Value = http
        .get(format!("{}/api/chats", server_url))
        .header("Authorization", &token)
        .send()
        .await?
        .json()
        .await?;

    let arr = resp["chats"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("server response missing 'chats' array"))?;

    let mut chats = Vec::new();
    for item in arr {
        let uuid = item["uuid"].as_str().unwrap_or("").to_string();
        let topic = item["topic"].as_str().unwrap_or("untitled").to_string();
        let description = item["description"].as_str().map(|s| s.to_string());
        let member_count = item["member_count"].as_u64().unwrap_or(0) as usize;
        if !uuid.is_empty() {
            chats.push(ChatSummary {
                uuid,
                topic,
                description,
                member_count,
            });
        }
    }
    Ok(chats)
}

async fn create_chat(
    server_url: &str,
    token: &str,
    topic: &str,
    description: Option<&str>,
    http: &reqwest::Client,
) -> Result<ChatSummary> {
    let mut body = serde_json::json!({"topic": topic});
    if let Some(desc) = description {
        body["description"] = serde_json::json!(desc);
    }
    let resp: serde_json::Value = http
        .post(format!("{}/api/chats", server_url))
        .header("Authorization", token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let uuid = resp["uuid"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing uuid in create chat response"))?
        .to_string();
    let topic = resp["topic"].as_str().unwrap_or(topic).to_string();
    let description = resp["description"].as_str().map(|s| s.to_string());

    Ok(ChatSummary {
        uuid,
        topic,
        description,
        member_count: 1,
    })
}
