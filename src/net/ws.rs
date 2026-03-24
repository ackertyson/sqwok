use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// Runs the WebSocket connection with automatic reconnect and exponential backoff.
///
/// - Forwards outgoing messages from `ws_out_rx` to the active WS connection.
/// - Forwards incoming text frames to `ws_in_tx`.
/// - Sends WebSocket pings every 30s and expects pong within 10s.
/// - Responds to server-initiated pings with pongs.
/// - Sends `"__reconnected__"` to `ws_in_tx` after each successful connection.
/// - Sends `"__connection_lost__"` to `ws_in_tx` when a connection drops.
///
/// Rebuilds the auth token on each reconnect attempt (tokens have a 30-second TTL).
pub async fn run_with_reconnect(
    server_url: String,
    identity_dir: std::path::PathBuf,
    mut ws_out_rx: mpsc::UnboundedReceiver<String>,
    ws_in_tx: mpsc::UnboundedSender<String>,
) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        // Build a fresh token for this connection attempt
        let ws_url = match crate::auth::token::build_token(&identity_dir, &server_url) {
            Ok(token) => format!(
                "{}/socket/websocket?token={}&vsn=1.0.0",
                server_url.replace("http", "ws"),
                token
            ),
            Err(e) => {
                eprintln!("ws: failed to build auth token: {}", e);
                let _ = ws_in_tx.send("__connection_lost__".to_string());
                sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
                continue;
            }
        };

        match tokio_tungstenite::connect_async(&ws_url).await {
            Ok((stream, _)) => {
                backoff = Duration::from_secs(1); // reset on success
                let (mut write, mut read) = stream.split();

                // Notify the event loop that we're connected
                if ws_in_tx.send("__reconnected__".to_string()).is_err() {
                    return; // TUI shut down
                }

                let ping_interval = Duration::from_secs(30);
                let pong_timeout = Duration::from_secs(10);
                let mut next_ping = Instant::now() + ping_interval;
                let mut awaiting_pong = false;
                let mut pong_deadline = Instant::now(); // only meaningful when awaiting_pong

                loop {
                    let timeout = if awaiting_pong {
                        pong_deadline
                    } else {
                        next_ping
                    };

                    tokio::select! {
                        msg = ws_out_rx.recv() => {
                            match msg {
                                Some(text) => {
                                    if write.send(Message::Text(text)).await.is_err() {
                                        break;
                                    }
                                }
                                None => return, // ws_out_tx dropped → app shut down
                            }
                        }
                        result = read.next() => {
                            match result {
                                Some(Ok(Message::Text(text))) => {
                                    if ws_in_tx.send(text.to_string()).is_err() {
                                        return; // TUI shut down
                                    }
                                }
                                Some(Ok(Message::Pong(_))) => {
                                    awaiting_pong = false;
                                }
                                Some(Ok(Message::Ping(data))) => {
                                    if write.send(Message::Pong(data)).await.is_err() {
                                        break;
                                    }
                                }
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                                Some(Ok(_)) => {} // binary — ignore
                            }
                        }
                        _ = sleep_until(timeout) => {
                            if awaiting_pong {
                                // Pong never arrived — connection is dead
                                break;
                            }
                            // Send a WebSocket ping
                            if write.send(Message::Ping(vec![].into())).await.is_err() {
                                break;
                            }
                            awaiting_pong = true;
                            pong_deadline = Instant::now() + pong_timeout;
                            next_ping = Instant::now() + ping_interval;
                        }
                    }
                }

                let _ = ws_in_tx.send("__connection_lost__".to_string());
            }
            Err(e) => {
                eprintln!("ws: connection failed ({}), retrying in {:?}", e, backoff);
                let _ = ws_in_tx.send("__connection_lost__".to_string());
            }
        }

        sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

async fn sleep_until(deadline: Instant) {
    tokio::time::sleep_until(deadline).await;
}
