# sqwok

Rust TUI chat client. Terminal-based, end-to-end encrypted.

![sqwok crow logo](logo.png)

## Install

Download from our [Releases](https://github.com/ackertyson/sqwok/releases) page, or...

### Homebrew (macOS)

```shell
brew tap ackertyson/sqwok
brew install sqwok
```

To upgrade later:

```shell
brew update
brew upgrade sqwok
```

### From source

Install [Rust](https://rust-lang.org/tools/install/), then...

```bash
cargo install --path .
```

## Onboarding

On first launch the client walks you through registration:

1. Enter your email and screenname
2. Check your email and click the verification link
3. The client polls until verified, then generates a keypair and gets a cert signed by the server's CA

Credentials are stored in `~/.sqwok/identity/`:
- `private_key.pem` (mode `0600`)
- `cert.pem` — signed X.509 client cert
- `ca.pem` — server's CA root cert
- `user_uuid` — your permanent identity
- `screenname` — your display name

**Chat history lives in `~/.sqwok/chats/<chat-uuid>/`** as SQLite files. The server does not retain messages — your local store is the only copy.

## Account Recovery

If you lose your device and need to re-register on a new one, run `sqwok` and choose "I have an account" at the prompt. The server issues a new cert with your original UUID; your chat history from the old device is not recoverable.

## Navigation

| Key | Action |
|---|---|
| `↑` / `↓` | Move selection between messages and input prompts |
| `→` | Expand a collapsed thread |
| `←` | Collapse an expanded thread |
| `Enter` | Focus a message's thread input; send from an active input |
| `Alt+←` / `Alt+→` | Switch pane focus |
| `Alt+N` | Open a new pane (horizontal split) |
| `Alt+W` | Close the current pane |
| `Tab` | Cycle between input fields within a pane |
| `/` | Open command bar |
| `Esc` | Dismiss command bar or modal |
| `G` / `End` | Jump to latest messages |

Max thread depth is 2 (message → reply → sub-reply).

## Command Bar

Press `/` to open the command bar. Use it to switch chats, search users, manage members, create invite codes, and access settings.

## Invite Codes

Generate an invite from the command bar or member list modal and share it out-of-band. Codes have a TTL (1h / 24h / 7d) and optional single-use enforcement.
