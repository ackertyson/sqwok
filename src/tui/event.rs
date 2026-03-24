use crossterm::event::{Event as CtEvent, EventStream};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::channel::protocol::Frame;
use crate::net::search::SearchResult;

pub enum AppEvent {
    Input(CtEvent),
    Frame(Frame),
    Tick,
    ConnectionLost(String),
    Reconnected,
    InviteCreated(crate::net::invites::InviteInfo),
    InviteError(String),
    SearchResults {
        query: String,
        results: Vec<SearchResult>,
    },
    RedeemOk { chat_uuid: String, topic: String },
    RedeemError(String),
    LeaveChatOk,
    LeaveChatError(String),
    AddMemberOk {
        screenname: String,
        user_uuid: String,
        e2e_public_key: Option<Vec<u8>>,
    },
    AddMemberError(String),
    InviteList(Vec<crate::net::invites::InviteInfo>),
    InviteRevoked(String),
    InviteRevokeError(String),
}

pub struct EventCollector {
    rx: mpsc::UnboundedReceiver<AppEvent>,
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl EventCollector {
    pub fn new(ws_rx: mpsc::UnboundedReceiver<String>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        // Spawn terminal input reader
        let tx_input = tx.clone();
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            while let Some(Ok(evt)) = reader.next().await {
                if tx_input.send(AppEvent::Input(evt)).is_err() {
                    break;
                }
            }
        });

        // Spawn tick generator (100ms interval)
        let tx_tick = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
            loop {
                interval.tick().await;
                if tx_tick.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        });

        // Forward WebSocket messages; handle reconnect/disconnect markers
        let tx_ws = tx.clone();
        tokio::spawn(async move {
            let mut ws_rx = ws_rx;
            while let Some(text) = ws_rx.recv().await {
                let event = if let Some(reason) = text.strip_prefix("__connection_lost__") {
                    let reason = reason.trim_start_matches(':');
                    AppEvent::ConnectionLost(if reason.is_empty() {
                        "Reconnecting...".to_string()
                    } else {
                        reason.to_string()
                    })
                } else if text == "__reconnected__" {
                    AppEvent::Reconnected
                } else if let Some(frame) = Frame::decode(&text) {
                    AppEvent::Frame(frame)
                } else {
                    continue;
                };
                if tx_ws.send(event).is_err() {
                    break;
                }
            }
            let _ = tx_ws.send(AppEvent::ConnectionLost("WebSocket closed".to_string()));
        });

        Self { rx, tx }
    }

    /// Get a sender for injecting events (e.g., from async tasks).
    pub fn sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.tx.clone()
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}
