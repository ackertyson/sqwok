mod auth;
mod channel;
mod config;
mod crypto;
mod identity;
mod net;
mod storage;
mod tui;

use anyhow::Result;
use base64::Engine as _;
use tokio::sync::mpsc;

use tui::app::{AppState, ChatSummary, Invitation};

const LOGO_256: &str = include_str!("../assets/logo_256.txt");
const LOGO_TRUE: &str = include_str!("../assets/logo_truecolor.txt");

#[derive(clap::Parser)]
#[command(
    name = "sqwok",
    about = "Terminal group chat with E2E encryption",
    disable_help_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    /// Show help
    #[arg(short, long, action = clap::ArgAction::SetTrue)]
    help: bool,

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
    /// Show help
    Help,
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

    if cli.help || matches!(cli.command, Some(SubCommand::Help)) {
        print_help();
        return Ok(());
    }

    let identity_dir = cli.identity.unwrap_or_else(config::identity_dir);
    let server_url = cli.server.unwrap_or_else(config::server_url);

    // Restore terminal before printing panic so the message is readable.
    std::panic::set_hook(Box::new(|info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen);
        eprintln!("{info}");
    }));

    // Registration if needed
    if !identity::credentials::is_registered(&identity_dir) {
        identity::registration::run_registration(&server_url, &identity_dir).await?;
    }

    // Require local E2E keys — no silent generation. Registration and account
    // recovery are the only paths that create them (and upload them to the server).
    if !identity_dir.join("e2e_private.key").exists() {
        anyhow::bail!(
            "E2E encryption keys not found. \
             Please re-register or recover your account to set up your identity."
        );
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

    // Verify our public key is registered on the server; upload if missing or stale.
    // Runs before the WebSocket connects so peers can always verify our signatures.
    // Self-healing: catches failed uploads from registration, DB restores, or key rotation.
    ensure_e2e_key_registered(&identity_dir, &server_url, &user_uuid_str, &http).await;

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
    // Load persisted block list.
    if let Some(ref cs) = app.contact_store {
        if let Ok(uuids) = cs.blocked_uuids() {
            app.blocked_uuids = uuids.into_iter().collect();
        }
    }
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
            let c = code.to_uppercase();
            let c = c.strip_prefix("SQWOK-").unwrap_or(&c);
            app.pending_redeem = Some(c.replace('-', ""));
        }
        Some(SubCommand::Help) | None => {}
    }

    let mut tui_instance = tui::Tui::new()?;
    let mut events = tui::event::EventCollector::new(ws_in_rx);

    let result = tui::run(&mut app, &mut tui_instance, &mut events, http).await;
    tui_instance.restore()?;

    result
}

/// Verify our E2E public key is registered on the server and upload it if not.
///
/// Compares our local public key against what the server has stored. Uploads when
/// the server returns 404 (key was never uploaded) or when the keys don't match
/// (e.g. account recovery generated new keys but a previous upload failed).
///
/// Runs before the WebSocket connects so peers never see us online without being
/// able to fetch our public key. Failures are soft warnings — a transient network
/// issue shouldn't block the app, and the check will succeed on the next startup.
async fn ensure_e2e_key_registered(
    identity_dir: &std::path::Path,
    server_url: &str,
    user_uuid: &str,
    http: &reqwest::Client,
) {
    let (local_ed, local_x) = match identity::e2e_keys::load_public_keys(identity_dir) {
        Ok(k) => k,
        Err(e) => {
            eprintln!("Warning: could not load local E2E keys: {}", e);
            return;
        }
    };

    let token = match auth::token::build_token(identity_dir, server_url) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Warning: could not build auth token for E2E key check: {}",
                e
            );
            return;
        }
    };

    let server_resp = http
        .get(format!("{}/api/users/{}/e2e_key", server_url, user_uuid))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await;

    let needs_upload = match server_resp {
        Err(e) => {
            eprintln!("Warning: could not verify E2E key registration: {}", e);
            return;
        }
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            let server_ed = body["e2e_public_key"]
                .as_str()
                .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok());
            let server_x = body["x25519_public_key"]
                .as_str()
                .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok());
            server_ed.as_deref() != Some(local_ed.as_slice())
                || server_x.as_deref() != Some(local_x.as_slice())
        }
        Ok(_) => true, // 404 or other non-success: server doesn't have our key
    };

    if !needs_upload {
        return;
    }

    let token = match auth::token::build_token(identity_dir, server_url) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Warning: could not build auth token for E2E key upload: {}",
                e
            );
            return;
        }
    };

    match http
        .post(format!("{}/api/e2e_key", server_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "public_key": base64::engine::general_purpose::STANDARD.encode(&local_ed),
            "x25519_public_key": base64::engine::general_purpose::STANDARD.encode(&local_x),
        }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => eprintln!("Warning: E2E key upload failed: {}", r.status()),
        Err(e) => eprintln!("Warning: E2E key upload failed: {}", e),
    }
}

async fn fetch_chat_list(
    server_url: &str,
    identity_dir: &std::path::Path,
    http: &reqwest::Client,
) -> Result<Vec<ChatSummary>> {
    let token = auth::token::build_token(identity_dir, server_url)?;
    let resp: serde_json::Value = http
        .get(format!("{}/api/chats", server_url))
        .header("Authorization", format!("Bearer {}", token))
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
        if !uuid.is_empty() {
            chats.push(ChatSummary {
                uuid,
                topic,
                description,
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
        .header("Authorization", format!("Bearer {}", token))
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
    })
}

fn detect_truecolor() -> bool {
    if let Ok(ct) = std::env::var("COLORTERM") {
        ct == "truecolor" || ct == "24bit"
    } else {
        false
    }
}

fn visible_width(s: &str) -> usize {
    let mut w = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            w += 1;
        }
    }
    w
}

// Like visible_width but stops at the last non-whitespace character,
// so trailing spaces in logo lines don't inflate the column width check.
fn visible_content_width(s: &str) -> usize {
    let mut w = 0;
    let mut last_nonspace = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c.is_ascii_alphabetic() {
                in_esc = false;
            }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            w += 1;
            if !c.is_ascii_whitespace() {
                last_nonspace = w;
            }
        }
    }
    last_nonspace
}

fn print_help() {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const BOLD: &str = "\x1b[1m";
    const GREEN: &str = "\x1b[1;32m";
    const CYAN: &str = "\x1b[36m";
    const DIM: &str = "\x1b[2m";
    const R: &str = "\x1b[0m";

    let lines = vec![
        format!("{BOLD}sqwok{R} v{VERSION} — terminal chat app with E2E encryption"),
        String::new(),
        format!("Full docs: {CYAN}https://github.com/ackertyson/sqwok{R}"),
        String::new(),
        format!("{GREEN}Usage:{R}"),
        format!("  {BOLD}sqwok{R}                                    Run chat client"),
        format!(
            "  {BOLD}sqwok new{R} {BOLD}<TOPIC>{R} {DIM}[--description <DESC>]{R} Create a new chat with optional description"
        ),
        format!("  {BOLD}sqwok join{R} {BOLD}<CODE>{R}                        Join via invite code"),
        String::new(),
        format!("{GREEN}Options:{R}"),
        format!("  {BOLD}--server{R} {DIM}<URL>{R}      Server URL"),
        format!("  {BOLD}--identity{R} {DIM}<DIR>{R}    Identity directory"),
        format!("  {BOLD}-h{R}, {BOLD}--help{R}          Show this help"),
    ];

    let logo = if detect_truecolor() {
        LOGO_TRUE
    } else {
        LOGO_256
    };
    let logo_lines: Vec<&str> = logo.lines().collect();
    let logo_width = logo_lines
        .iter()
        .map(|l| visible_content_width(l))
        .max()
        .unwrap_or(0);

    let gap = 3;
    let help_width = lines.iter().map(|l| visible_width(l)).max().unwrap_or(0);
    let tw = terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80);

    if tw >= help_width + gap + logo_width && !logo_lines.is_empty() {
        let total = lines.len().max(logo_lines.len());
        let logo_offset = total.saturating_sub(logo_lines.len()) / 2;

        for i in 0..total {
            let help_part = lines.get(i).map(|s| s.as_str()).unwrap_or("");
            let logo_part = if i >= logo_offset && i - logo_offset < logo_lines.len() {
                logo_lines[i - logo_offset]
            } else {
                ""
            };
            let pad = help_width + gap - visible_width(help_part);
            println!("{}{:pad$}{}\x1b[0m", help_part, "", logo_part, pad = pad);
        }
    } else {
        for line in &lines {
            println!("{line}");
        }
    }
}
