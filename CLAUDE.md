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

## Core principles

1. self reliance: prefer self-rolled implementations over 3rd-party solutions
2. simplicity: don't reach for complex solutions when a succinct approach will do the job *unless the additional complexity adds real value*
3. UX is king: our UI should be snappy, elegant, intuitive and powerful

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
- **`identity/`** — Registration flow: email verification → TOTP setup → RSA keypair → server-signed X.509 cert. Account recovery follows the same TOTP-gated path.
- **`net/`** — WebSocket client with exponential backoff reconnection (1s→60s max), 30s heartbeat. Also HTTP endpoints for invites and user search.
- **`storage/`** — SQLite persistence. Per-chat message DB at `~/.sqwok/chats/{uuid}/messages.db`. Global contact cache at `~/.sqwok/contacts.db`.
- **`tui/`** — Ratatui-based terminal UI. See TUI section below.

### TUI Structure

- **`app.rs`** (~1800 lines) — `AppState`: central state object, all inbound event handlers, business logic (send, navigate, expand/collapse threads, key exchange, etc.).
- **`input.rs`** — Keyboard dispatch. Routes `crossterm` key events to `AppState` methods based on current mode/context (chat vs picker, editing vs navigating, modal active, etc.).
- **`render.rs`** — Top-level frame layout: splits the terminal into top bar, pane area, bottom bar; handles multi-pane borders; dispatches to view components.
- **`render_rows.rs`** — Builds the flat `Vec<RenderRow>` for a pane. `RenderRow` is the core display abstraction: a depth-aware enum (`Message`, `CollapsedThread`, `Input`, `TypingIndicator`) with an `indent: u8` field that drives visual nesting. All collapsed-group unread/mention status flows through `TuiMessageStore::unread_status`.
- **`pane.rs`** — `Pane` struct (per-pane viewport state: selection, editing target, input buffers, expanded/collapsed sets). `InputTarget` enum (`MainChat`, `Thread(uuid)`, `Reply(reply_uuid, thread_uuid)`) with methods for converting to/from row fields.
- **`store.rs`** — `TuiMessageStore`: in-memory display store. Holds `top_level` order, `by_uuid` map, and `threads` map. `unread_status(uuids)` is the single source of truth for collapsed-group unread status. Note: `mentions_me` on `RenderRow` is **not** read-gated — it reflects raw mention status from the store. Visual read-gating (`is_unread && mentions_me`) is applied in `views/chat.rs`.
- **`mention.rs`** — `@mention` parsing: `mentions_user`, `render_body` (wire tags → `@name`), `split_body_spans` (inline highlight segments), autocomplete query extraction.
- **`style.rs`** — Color palette. All colors go through functions here; never hardcode colors elsewhere.
- **`views/`** — Individual view components: `chat` (message list, top/bottom bars), `chat_picker`, `command_bar`, `contacts`, `error_toast`, `group_settings`, `invite`, `member_list`, `modal`, `search`.

### State Modes

`AppState` operates in two modes: `ChatPicker` (chat list) and `Chat` (active conversation). Modals (`MemberList`, `GroupSettings`, `InviteCreate`, `Search`, `Contacts`) overlay on top. `Chat` mode supports multiple simultaneous panes (vertical or horizontal split) via `Vec<Pane>` with an `active_pane` index.

### Render Row Model

`render_rows::build()` converts the message store into a flat list for rendering. The key design principle: all depth levels share the same `RenderRow::Message` / `RenderRow::Input` variants, differentiated only by `indent` (0 = top-level, 1 = thread reply, 2 = sub-reply). Adding a new depth level requires only: pushing messages with the appropriate `indent`, calling `push_depth_footer` with the right `InputTarget`, and extending `build_indent`/`indent_width` in `views/chat.rs`.

`InputTarget` encodes which input field is active and can derive its own wire UUIDs (`to_wire_uuids`), indent depth (`indent`), and row-matching predicate (`matches_input_row`).

### Navigation Shortcuts (Chat mode, non-editing)

| Key | Action |
|-----|--------|
| `Alt+m` | Jump to next `@mention`, cycling; expands collapsed threads to land on the message |
| `Alt+n` | Jump to next unread message, cycling; expands collapsed threads to land on the message |
| `Alt+\` | Split pane vertically |
| `Alt+-` | Split pane horizontally |
| `Alt+w` | Close active pane |
| `Alt+←/→` | Focus previous/next pane |

Both `Alt+m` and `Alt+n` share the `jump_to_matching` helper in `app.rs`.

### Phoenix Channel Protocol

Server communication uses Phoenix channel frames: `{"topic", "event", "ref", "join_ref", "payload"}`. Key inbound events: `msg:new`, `msg:buffered`, `member_list`, `presence_state`, `presence_diff`, `member:removed`, `key:request`, `key:distribute`, `typing:active`, `sync:push`, `sync:assign`, `sync:query`, `chat:added`, `chat:removed`.

## Environment Variables

- `SQWOK_SERVER` — Server URL (default: `https://sqwok.fixbase.io`)

## File Paths at Runtime

All identity and credential files are co-located under `~/.sqwok/identity/`:
- `private_key.pem` — RSA private key (auth)
- `cert.pem`, `ca.pem` — Server-signed X.509 cert (auth)
- `user_uuid` — Registered user UUID
- `screenname` — Display name
- `e2e_private.key` — Ed25519 private key (E2E encryption)

Other paths:
- `~/.sqwok/chats/{uuid}/messages.db` — Per-chat SQLite message store
- `~/.sqwok/contacts.db` — Contact/screenname cache
- `~/.sqwok/debug.log` — Debug output (written by `dlog!` macro)

## Patterns

- Long-running operations (HTTP calls, key exchange) are spawned as tokio tasks that send results back via `AppEvent`
- MPSC channels connect the WebSocket reader/writer to the app
- `anyhow::Result` used throughout for error handling
- Tests are inline `#[cfg(test)]` modules (primarily in `crypto/identity.rs` and `channel/protocol.rs`)
