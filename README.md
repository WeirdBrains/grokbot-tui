# gb — Grok Bot TUI

A terminal UI for Grok Bot, written in Rust (2024 edition) with ratatui + crossterm.

> **Status:** There is no confirmed public Grok Bot API spec yet. The HTTP client's
> base URL and endpoint paths are **placeholders**. Use `gb --mock` to try the full UI offline.

## Run

```sh
cargo run -- --mock          # offline: fake bots, echo replies; any token except "bad" logs in
cargo run                    # real HTTP client (needs real endpoints in config.toml)
cargo install --path .       # installs the `gb` binary
gb --config-dir /tmp/gbtest  # use a different config directory
```

## Layout

```
┌ gb  user: mock-user   bot: Helper   [MOCK] ───────────────┐  top bar
│┌ Bots ──────┐┌ Chat: Helper ─────────────────────────────┐│
││ Helper     ││ you: hi                                   ││  body: command output
││ Coder      ││ Helper: echo: hi                          ││  and chat transcript
││ Scribe     ││                                           ││  (scrollable)
│└────────────┘└───────────────────────────────────────────┘│
│┌ /command or message ────────────────────────────────────┐│  bottom input bar
││ > _                                                     ││
│└─────────────────────────────────────────────────────────┘│
```

- **Top bar:** logged-in user and active bot.
- **Left sidebar:** bots; the active one is highlighted. Click one to switch the body to its chat (history loads on first visit).
- **Body:** each bot has its own log; command output goes into the current one.
- **Bottom bar:** `/`-prefixed input runs a command; plain text is sent to the active bot.

Keys: `Enter` submit · `Tab` toggle focus input/sidebar · `↑/↓` (or `j/k`) + `Enter` in sidebar to pick a bot ·
`↑/↓`, `PgUp/PgDn`, mouse wheel scroll body · `Esc` cancel token entry · `Ctrl-C`/`Ctrl-D` quit.

## Commands

| Command | Description |
|---|---|
| `/help [cmd]` (`/h`, `/?`) | List commands or show one |
| `/login [token]` | Log in. With no argument, prompts for a masked token |
| `/logout` | Revoke (if configured), delete stored credential, clear state |
| `/whoami` | Show the logged-in user |
| `/bots` | Refresh and list bots |
| `/bot <name\|id>` (`/use`) | Switch to a bot's chat |
| `/history` | Reload the active bot's history |
| `/clear` | Clear the body (local only) |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `src/commands.rs`. To add one, write a `fn(&mut App, &str) -> Result<()>` and add a `Command` entry to `registry()`.

## Configuration

Everything is in `~/.grokbot-tui/` (directory is `0700`):

- `config.toml` — created with defaults on first run:

  ```toml
  api_base_url = "https://api.grokbot.example"   # PLACEHOLDER
  request_timeout_secs = 30
  # default_bot = "helper"

  [paths]                       # PLACEHOLDERS, `{bot}` = bot id
  whoami = "/v1/me"
  bots = "/v1/bots"
  history = "/v1/bots/{bot}/messages"
  send = "/v1/bots/{bot}/messages"
  # revoke = "/v1/tokens/revoke"
  ```
- `credentials` — the API token, file mode `0600`.

## Authentication

- **Login:** `/login` asks for a token (shown as `*`), validates it with `whoami`, then saves it
  to `credentials`. Bad tokens are never saved. On startup the stored token is checked the same way.
- **`GROKBOT_TOKEN`** env var overrides the stored credential.
- **Logout:** `/logout` calls the revoke endpoint if `paths.revoke` is set, deletes `credentials`,
  and clears user, bots, and chats from memory. If `GROKBOT_TOKEN` is set it warns that you'll be logged back in on the next start.
- **Future:** OAuth device-code flow. `commands::finish_login` takes a token from anywhere, so a
  device flow (show code, poll token endpoint) can drop in there.

## Wiring the real API

Networking sits behind the `ApiClient` trait in `src/api.rs` (`whoami`, `list_bots`, `history`, `send`, `revoke`).
`HttpClient` currently assumes:

- Bearer token auth (`Authorization: Bearer <token>`)
- `GET whoami` → `{"id","name"}`
- `GET bots` → `[{"id","name","description"?}]`
- `GET history` → `[{"role":"user"|"bot","text"}]`
- `POST send` with `{"text"}` → `{"role":"bot","text"}`

To connect it, we need from Grok Bot: the base URL, how tokens are issued (personal tokens and/or OAuth device flow),
the auth header scheme, the real paths and JSON shapes (adjust the serde structs or map them),
whether there's a revoke endpoint, and whether replies stream (SSE/WebSocket). Streaming would need a background task in place of the current blocking call.

## Development

```sh
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
```
