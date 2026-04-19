# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

sqwok is a terminal-based group chat client written in Rust with end-to-end encryption. It connects to a Phoenix (Elixir) server over WebSockets using the Phoenix channel protocol.

## Build & Run Commands

```bash
cargo fmt                      # Format all files (run after changes)
cargo clippy                   # Lint all files (run after changes)
cargo build                    # Debug build
cargo build --release          # Release build
cargo install --path .         # Install binary as `sqwok`
cargo test                     # Run all tests
cargo test <test_name>         # Run a single test
docker-compose up              # Run via Docker
```

CLI usage: `sqwok` (interactive), `sqwok new "Topic" [--description "Desc"]` (create chat), `sqwok join CODE-1234` (join via invite). Global flags: `--server <URL>`, `--identity <DIR>`.

## Core Principles

1. **Self-reliance**: prefer self-rolled implementations over 3rd-party solutions.
2. **Simplicity**: don't reach for complex solutions when a succinct approach will do the job *unless the additional complexity adds real value*.
3. **UX is king**: our UI should be snappy, elegant, intuitive and powerful.

## Design Philosophy

**TUI is the identity.** The terminal client is the product — sqwok isn't competing with Slack/Teams/Discord GUI apps and isn't trying to. Building a new kind of TUI chat app was the founding impetus.

**Message sovereignty.** The server never decodes or stores message content. Chat history lives entirely on client devices. When a message is sent, the server assigns it a sequence number and relays it to online peers, but keeps no copy. This is core to the entire app design — there is no central message DB.

**Scale ambition.** Threading + independent pane views are designed so the chat UX can scale from 2 users to 200,000.

## Security Model

sqwok is **secure but not anonymous**. The server cannot read message content — that's guaranteed by E2E encryption. What it *can* see is metadata: who is in a group, who sent each message, and when. The E2E posture is philosophical (protecting user communication content as a principle), not aimed at a specific adversary.

The crypto stack was designed by an LLM with the owner's guidance. Contributors are encouraged to scrutinize it.

### Authentication

RSA keypair + server-signed X.509 certificate. The server acts as a CA — during registration, the client generates an RSA keypair, sends a CSR, and receives back a signed cert with the user's UUID as the CN. Every WebSocket connection authenticates via a short-lived token: `base64url(timestamp|host).base64url(RSA_signature).base64url(cert_PEM)`, 30s TTL. The server validates the cert chain, checks signature freshness, rejects replays, and verifies the cert serial matches the user's current cert (so old/rotated certs are rejected).

### E2E Encryption

Two independent key pairs per user (separate random seeds, not derived from each other):
- **Ed25519** (`e2e_private.key`) — signing key bundles during key exchange
- **X25519** (`x25519_private.key`) — Diffie-Hellman for encrypting key bundles per-recipient

**Message encryption**: AES-256-GCM with epoch-based group keys. Wire format: `[epoch:4B LE][nonce:12B][ciphertext+GCM_tag]`. AAD binds both the epoch and sender UUID, preventing epoch-swap and sender-spoofing attacks.

**Group key management**: `KeyChain` holds all epoch keys for a chat. The group creator generates epoch 0. Key rotation (new epoch with fresh random key) happens on member removal. Old epoch keys are retained so historical messages remain decryptable.

**Key exchange**: Epoch keys are encrypted per-recipient using X25519 DH shared secrets, wrapped with HKDF-SHA256 (salt = sorted concatenation of both public keys, info = `sqwok-key-wrap-v1`), then AES-256-GCM with epoch number as AAD. The entire bundle is Ed25519-signed by the sender. On the receiving end, signature is verified before any decryption.

**Key buffering**: When keys are distributed to an offline recipient, the server buffers the encrypted bundle (ETS-backed, 30-day TTL, most-recent-only since each bundle contains the full epoch chain) and delivers it on reconnect.

### Account Recovery

When a user loses `~/.sqwok/identity/`, they re-register with the same email + existing TOTP code (do *not* re-scan QR). The server issues a new cert with the original UUID and disconnects old sessions. On rejoining a chat, online peers automatically re-distribute encryption keys and sync message history. Recovery completeness depends on what peers still have locally.

## Peer Sync Protocol

Since the server stores no messages, clients sync history peer-to-peer. The protocol is server-coordinated (the server tracks sequence numbers and online presence) but message content flows directly between peers as encrypted blobs.

**Two-phase sync** (on join/reconnect):
1. **Probe**: Client sends `sync:catchup` with its known segments. Server computes missing ranges against `global_seq`, broadcasts `sync:query` to online peers asking what they have.
2. **Assign**: Peers respond with `sync:offer` (their local segment ranges). Server assigns non-overlapping ranges across peers that actually hold the needed data, sends `sync:assign` to each. Peers then `sync:push` the messages.

**Scrollback**: `sync:scrollback` requests older history (before the client's earliest known seq) using the same two-phase probe-and-assign mechanism.

**Orphan recovery**: If an assigned peer disconnects mid-sync, the server detects this via presence_diff and reassigns their ranges to remaining peers.

**Trust model**: Synced messages are encrypted blobs — the server never sees plaintext. Peers trust each other's sequence claims. There is no signature verification on synced messages beyond what AES-256-GCM provides (the message will fail to decrypt if the key/epoch/sender doesn't match).

## Architecture

### Event Loop

The app is event-driven with three async event sources merged into a single stream (`tui/event.rs`):
- Terminal input (crossterm EventStream)
- WebSocket frames from server
- 100ms tick timer

Each event mutates `AppState` (`tui/app.rs`, the central state object) and optionally spawns async tasks. The main loop lives in `tui/mod.rs::run`.

### Module Responsibilities

- **`auth/`** — RSA-signed token generation for WebSocket authentication.
- **`channel/`** — Phoenix WebSocket protocol. `protocol.rs` handles JSON frame encoding/decoding. `chat.rs` manages per-chat state and message sending. `sync.rs` builds `sync:push` response frames for peer-to-peer message history sync.
- **`config.rs`** — Path helpers (`identity_dir`, `server_url`, `chat_dir`, `home_dir`). Handles migration from old platform-specific data dirs.
- **`crypto/`** — E2E encryption stack. See Security Model above.
- **`identity/`** — Registration flow: email verification → TOTP setup → RSA keypair + E2E keypairs → server-signed X.509 cert. Account recovery follows the same TOTP-gated path.
- **`net/`** — WebSocket client (`ws.rs`) with exponential backoff reconnection (1s→60s max), 30s WS-level ping/pong (10s pong timeout). HTTP endpoints for invites and user search. Phoenix-level heartbeat is sent from `main.rs`.
- **`storage/`** — SQLite persistence. Per-chat message DB at `~/.sqwok/chats/{uuid}/messages.db`. Global contact cache at `~/.sqwok/contacts.db`.
- **`tui/`** — Ratatui-based terminal UI. See TUI section below.

### TUI Structure

- **`app.rs`** — `AppState`: central state object, all inbound event handlers, business logic.
- **`input.rs`** — Keyboard dispatch by mode/context.
- **`render.rs`** — Top-level frame layout and pane borders.
- **`render_rows.rs`** — Converts message store into flat `Vec<RenderRow>` for rendering. `RenderRow` is depth-aware (indent 0/1/2 for message/thread/sub-reply).
- **`pane.rs`** — Per-pane viewport state. `InputTarget` enum encodes which input field is active.
- **`store.rs`** — `TuiMessageStore`: in-memory message index with unread/mention tracking.
- **`mention.rs`** — `@mention` parsing and rendering.
- **`style.rs`** — Color palette. All colors go through functions here; never hardcode colors elsewhere.
- **`views/`** — Individual view components.

### Key TUI Invariants

- All depth levels share `RenderRow::Message` / `RenderRow::Input`, differentiated only by `indent`. Adding a depth level means: push rows with the right indent, call `push_depth_footer`, extend `build_indent`/`indent_width` in `views/chat.rs`.
- `mentions_me` on `RenderRow` reflects raw mention status (aggregated over collapsed sub-replies). Visual read-gating (`is_unread && mentions_me`) is applied in `views/chat.rs`.
- `Alt+m` and `Alt+n` both use the `jump_to_matching` helper in `app.rs`.
- Blocking is local-only — no server notification. First attempt, not battle-tested.
- Thread depth (3 levels) is an open design question without empirical data yet.

### State Modes

`AppState` operates in two modes: `ChatPicker` (chat list) and `Chat` (active conversation). Modals overlay on top. `Chat` mode supports multiple simultaneous panes (vertical or horizontal split) via `Vec<Pane>` with an `active_pane` index.

## Environment Variables

- `SQWOK_SERVER` — Server URL (default: `https://sqwok.fixbase.io`)

## File Paths at Runtime

Identity files under `~/.sqwok/identity/`:
- `private_key.pem` — RSA private key (auth)
- `cert.pem`, `ca.pem` — Server-signed X.509 cert and CA cert (auth)
- `user_uuid` — Registered user UUID
- `screenname` — Display name
- `e2e_private.key` — Ed25519 signing key (32-byte seed)
- `x25519_private.key` — X25519 DH key (32-byte seed, independent from Ed25519)

Per-chat data under `~/.sqwok/chats/{uuid}/`:
- `messages.db` — SQLite message store
- `keychain.bin` — Epoch keys, binary format: `[epoch:4B LE][key:32B]` repeated, file mode 0o600

Other:
- `~/.sqwok/contacts.db` — Contact/screenname cache + blocked users table

## Patterns

- Long-running operations (HTTP calls, key exchange) are spawned as tokio tasks that send results back via `AppEvent`
- MPSC channels connect the WebSocket reader/writer to the app
- `anyhow::Result` used throughout for error handling
- Tests are inline `#[cfg(test)]` modules spread across the codebase
