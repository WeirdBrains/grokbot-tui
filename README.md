# gb — Grok Bot TUI

A terminal UI for Grok Bot, written in Rust (2024 edition) with ratatui + crossterm.

> **Status:** There is no confirmed public Grok Bot API spec yet. The HTTP client's
> base URL and endpoint paths are **placeholders**. Use `gb --mock` to try the full UI offline.

## Run

```sh
cargo run -- --mock          # offline: fake bots, echo replies; any token except "bad" logs in
cargo run                    # real HTTP client (set real endpoints with /prefs)
cargo install --path .       # installs the `gb` binary
gb --data-dir /tmp/gbtest    # use a different data directory (holds context.db)
gb --account work            # start in an account (or GB_ACCOUNT=work gb)
```

## Layout

```
┌ gb  account: default  user: mock-user  bot: Helper [MOCK] ┐  top bar
│ @default ⇄    ┌ Chat: Helper ───────────────────────────┐│  account header (click to cycle)
│┌ Bots ──────┐│                                         ││
││ Helper     ││ you: hi                                   ││  body: command output
││ Coder      ││ Helper: echo: hi                          ││  and chat transcript
││ Scribe     ││                                           ││  (scrollable)
│└────────────┘└───────────────────────────────────────────┘│
│┌ /command or message ────────────────────────────────────┐│  bottom input bar
││ > _                                                     ││
│└─────────────────────────────────────────────────────────┘│
```

- **Top bar:** active account alias, logged-in user, and active bot.
- **Account header** (top of the sidebar): the active account; click it to cycle to the next account.
- **Left sidebar:** the active account's bots; the active one is highlighted. Click one to switch the body to its chat. The chat is restored from the local database (also across restarts), falling back to server history. The last active bot is reselected on start.
- **Body:** each bot has its own log; command output goes into the current one.
- **Bottom bar:** `/`-prefixed input runs a command; plain text is sent to the active bot.

Keys: `Enter` submit · `Tab` toggle focus input/sidebar · `↑/↓` (or `j/k`) + `Enter` in sidebar to pick a bot ·
`↑/↓`, `PgUp/PgDn`, mouse wheel scroll body · `Esc` cancel token entry · `Ctrl-C`/`Ctrl-D` quit.

## Commands

| Command | Description |
|---|---|
| `/help [cmd]` (`/h`, `/?`) | List commands or show one |
| `/login [token]` | Log in to the active account. With no argument, prompts for a masked token |
| `/logout` | Log out of the active account. Revokes if configured and deletes its credential; chats and memory are kept |
| `/whoami` | Show the logged-in user |
| `/bots` | Refresh and list bots |
| `/bot <name\|id>` (`/use`) | Switch to a bot's chat |
| `/history` | Reload the active bot's history |
| `/account` (`/acct`) | List accounts (`*` = active) |
| `/account add <alias> [api_base_url]` | Create an account, optionally on its own server |
| `/account use <alias>` | Switch account: reloads client, login, bots, chats, top bar |
| `/account rename <old> <new>` | Rename an account |
| `/account remove <alias>` | Delete an account and all its data; you confirm by typing the alias (can't remove the active one) |
| `/prefs` (`/pref`) | List preferences; `/prefs get <key>`, `/prefs set <key> <value>` |
| `/memory` (`/mem`) | List the active bot's memory notes; `/memory add <key> <value>`, `/memory clear [key]` |
| `/clear [saved]` | Clear the body; with `saved`, also delete the bot's stored transcript |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `src/commands.rs`. To add one, write a `fn(&mut App, &str) -> Result<()>` and add a `Command` entry to `registry()`.

## Accounts

gb supports several **strictly segregated accounts**, for example `personal` plus several work accounts.
Each account has its own alias, credential, API base URL and endpoint paths (so work accounts can point
at different servers), bot list, chat transcripts, per-bot memory, and account preferences.
Nothing is shared between accounts except a few global preferences.

- Start-up account: `--account <alias>`, then `GB_ACCOUNT`, then the last active account, then `default`.
- Switch with `/account use <alias>` or click the `@alias` header in the sidebar.
- Removing an account deletes all of its rows (FK `ON DELETE CASCADE`).

## Storage and configuration

Everything lives in a SQLite database at `~/.gb/context.db` (directory `0700`, file `0600`),
via `rusqlite` with bundled SQLite. Override the directory with `--data-dir`. All access goes through
`storage::Store` (`src/storage.rs`). A `Store` is bound to one active account, and every account-scoped query filters on its `account_id`.

Schema v2 (version tracked in `PRAGMA user_version`; `MIGRATIONS` run in order on open):

| Table | Key | Columns | Notes |
|---|---|---|---|
| `accounts` | `id` | `alias` (unique, case-insensitive), `created_at` | |
| `credentials` | `account_id` → accounts | `token`, `user_id`, `user_name`, `api_base`, `created_at`, `updated_at` | One per account |
| `bots` | `(account_id, bot_id)`; `account_id` → accounts | `name`, `description` | Cached bot list |
| `messages` | `id`; `(account_id, bot_id)` → bots | `role` (`user`/`bot`), `text`, `created_at` | Index `(account_id, bot_id, id)` |
| `memory` | `(account_id, bot_id, key)`; `(account_id, bot_id)` → bots | `value`, `updated_at` | Per-bot notes |
| `account_prefs` | `(account_id, key)`; `account_id` → accounts | `value` | Account preferences |
| `global_prefs` | `key` | `value` | `active_account`, `ui.sidebar_width`, `legacy_imported` |

All foreign keys use `ON DELETE CASCADE`, and `PRAGMA foreign_keys=ON` is set on every connection.

**Migration v1 → v2:** creates account `default` (id 1) and moves the existing credential, messages,
memory, and prefs into it. `ui.sidebar_width` moves to the global prefs.

Account preference keys (seeded per account; `/prefs set` writes to the right scope automatically):

| Key | Default |
|---|---|
| `api_base_url` | `https://api.grokbot.example` (PLACEHOLDER), or the URL given to `/account add` |
| `request_timeout_secs` | `30` |
| `paths.whoami` / `paths.bots` | `/v1/me` / `/v1/bots` (PLACEHOLDERS) |
| `paths.history` / `paths.send` | `/v1/bots/{bot}/messages` (PLACEHOLDERS, `{bot}` = bot id) |
| `paths.revoke` | empty (no revoke call) |
| `default_bot` | empty |
| `active_bot` | updated automatically when you switch bots |

Global keys: `ui.sidebar_width` (default `24`, 10 to 80); `active_account` (set it with `/account use`).
API changes take effect immediately: the client is rebuilt.

**Legacy import:** on first run with the default data dir, if `~/.grokbot-tui/` exists, its `config.toml`
and `credentials` are imported once into the active account on a best-effort basis. The old directory is left alone.

## Authentication

- **Login:** `/login` asks for a token (shown as `*`), validates it with `whoami`, then saves token,
  user id/name, and API base to the `credentials` table. Bad tokens are never saved. On startup the stored token is checked the same way.
- **`GB_GROKBOT_TOKEN`** env var overrides the stored credential, but **only for the account selected at startup**
  (`--account` / `GB_ACCOUNT` / last active). After `/account use` to another account, that account's own stored credential is used.
  This stops one token from being sent to another account's server.
- **Logout:** `/logout` calls the revoke endpoint if `paths.revoke` is set, deletes the `credentials` row,
  for the active account only, and clears user, bots, and chats from memory. Stored transcripts and memory notes are kept. If `GB_GROKBOT_TOKEN` is set it warns that you'll be logged back in on the next start.
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
