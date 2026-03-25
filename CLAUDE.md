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

CLI usage: `sqwok` (interactive), `sqwok new "Topic"` (create chat), `sqwok join CODE-1234` (join via invite).

## Core principles

1. self reliance: prefer simple self-rolled implementations over 3rd-party solutions
2. simplicity: don't reach for complex solutions when a succinct approach will do the job
3. UX is king: our UI should be elegant, intuitive and powerful

## Architecture

### Event Loop

The app is event-driven with three async event sources merged into a single stream (`tui/event.rs`):
- Terminal input (crossterm EventStream)
- WebSocket frames from server
- 100ms tick timer

Each event mutates `AppState` (`tui/app.rs`, the central state object) and optionally spawns async tasks. The main loop lives in `tui/mod.rs::run`.

### Module Responsibilities

- **`auth/`** — RSA-signed token generation for WebSocket authentication. Token format: `base64(message).base64(signature).base64(cert)`, 30s TTL.
- **`channel/`** — Phoenix WebSocket protocol. `protocol.rs` handles JSON frame encoding/decoding. `chat.rs` manages per-chat state and message sending.
- **`crypto/`** — E2E encryption stack:
  - Ed25519 signing + X25519 key exchange (derived from same seed)
  - AES-256-GCM message encryption with wire format: `[epoch:4B][nonce:12B][ciphertext+tag]`
  - Epoch-based group key management (`KeyChain`) with HKDF key derivation
  - Key bundles encrypted per-recipient via X25519 DH
- **`identity/`** — Registration flow (email verification → RSA keypair → server-signed X.509 cert).
- **`net/`** — WebSocket client with exponential backoff reconnection (1s→60s max), 30s heartbeat. Also HTTP endpoints for invites and user search.
- **`storage/`** — SQLite persistence. Per-chat message DB at `~/.sqwok/chats/{uuid}/messages.db`. Global contact cache at `~/.sqwok/contacts.db`.
- **`tui/`** — Ratatui-based terminal UI. `app.rs` (~1500 lines) holds all state. `input.rs` handles keyboard dispatch per mode/context. `views/` has individual view components.

### State Modes

AppState operates in two modes: `ChatPicker` (chat list) and `Chat` (active conversation). Modals (member list, settings, invite, search, contacts) overlay on top.

### Phoenix Channel Protocol

Server communication uses Phoenix channel frames: `{"topic", "event", "ref", "join_ref", "payload"}`. Key events: `msg:new`, `msg:updated`, `member:joined`, `key:request`, `key:distribute`.

## Environment Variables

- `SQWOK_SERVER` — Server URL (default: `http://localhost:4000`)

## File Paths at Runtime

- `~/.sqwok/identity/` — E2E keys (Ed25519 private key)
- `~/.local/share/sqwok/` — Registration credentials (RSA key, X.509 cert)
- `~/.sqwok/chats/{uuid}/messages.db` — Per-chat SQLite
- `~/.sqwok/contacts.db` — Contact cache
- `~/.sqwok/debug.log` — Debug output (written by `dlog!` macro)

## Patterns

- Long-running operations (HTTP calls, key exchange) are spawned as tokio tasks that send results back via `AppEvent`
- MPSC channels connect the WebSocket reader/writer to the app
- `anyhow::Result` used throughout for error handling
- Tests are inline `#[cfg(test)]` modules (primarily in `crypto/identity.rs` and `channel/protocol.rs`)
