# gb — Grok Bot TUI

A terminal UI for Grok Bot, written in Rust (2024 edition) with ratatui + crossterm.

> **Status:** There is no confirmed public Grok Bot API spec yet. The HTTP client's
> base URL and endpoint paths are **placeholders**. Use `gb --mock` to try the full UI offline.

## Run

```sh
cargo run -- --mock          # offline: fake bots, echo replies; any token except "bad" logs in
cargo run                    # real HTTP client (needs real endpoints in config.toml)
cargo install --path .       # installs the `gb` binary
gb --data-dir /tmp/gbtest    # use a different data directory (holds context.db)
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
- **Left sidebar:** bots; the active one is highlighted. Click one to switch the body to its chat. The chat is restored from the local database (also across restarts), falling back to server history. The last active bot is reselected on start.
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
| `/prefs` (`/pref`) | List preferences; `/prefs get <key>`, `/prefs set <key> <value>` |
| `/memory` (`/mem`) | List the active bot's memory notes; `/memory add <key> <value>`, `/memory clear [key]` |
| `/clear [saved]` | Clear the body; with `saved`, also delete the bot's stored transcript |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `src/commands.rs`. To add one, write a `fn(&mut App, &str) -> Result<()>` and add a `Command` entry to `registry()`.

## Storage and configuration

Everything lives in a SQLite database at `~/.gb/context.db` (directory `0700`, file `0600`),
via `rusqlite` with bundled SQLite. Override the directory with `--data-dir`. All access goes through
`storage::Store` (`src/storage.rs`).

Schema (version tracked in `PRAGMA user_version`; migrations in `MIGRATIONS` run in order on open):

| Table | Columns | Purpose |
|---|---|---|
| `credentials` | `id` (always 1), `token`, `user_id`, `user_name`, `api_base`, `created_at`, `updated_at` | Saved login |
| `messages` | `id`, `bot_id`, `role` (`user`/`bot`), `text`, `created_at` | Per-bot chat transcript |
| `memory` | `bot_id`, `key`, `value`, `updated_at` (PK `bot_id,key`) | Per-bot memory notes |
| `prefs` | `key`, `value` | Preferences |

Preference keys (seeded with defaults; edit with `/prefs set`):

| Key | Default |
|---|---|
| `api_base_url` | `https://api.grokbot.example` (PLACEHOLDER) |
| `request_timeout_secs` | `30` |
| `paths.whoami` / `paths.bots` | `/v1/me` / `/v1/bots` (PLACEHOLDERS) |
| `paths.history` / `paths.send` | `/v1/bots/{bot}/messages` (PLACEHOLDERS, `{bot}` = bot id) |
| `paths.revoke` | empty (no revoke call) |
| `default_bot` | empty |
| `active_bot` | updated automatically when you switch bots |
| `ui.sidebar_width` | `24` (10 to 80) |

Changes to API keys take effect after you restart gb.

**Legacy import:** on first run with the default data dir, if `~/.grokbot-tui/` exists, its `config.toml`
(flattened to keys like `paths.bots`) and `credentials` are imported once on a best-effort basis
(`legacy_imported` pref marks it done). The old directory is left alone, so delete it yourself.

## Authentication

- **Login:** `/login` asks for a token (shown as `*`), validates it with `whoami`, then saves token,
  user id/name, and API base to the `credentials` table. Bad tokens are never saved. On startup the stored token is checked the same way.
- **`GROKBOT_TOKEN`** env var overrides the stored credential.
- **Logout:** `/logout` calls the revoke endpoint if `paths.revoke` is set, deletes the `credentials` row,
  and clears user, bots, and chats from memory. Stored transcripts and memory notes are kept. If `GROKBOT_TOKEN` is set it warns that you'll be logged back in on the next start.
- **Future:** OAuth device-code flow. `commands::finish_login` takes a token from anywhere, so a
  device flow can drop in there.

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
