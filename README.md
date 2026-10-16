# gb — Grok Bot TUI

A terminal UI for Grok Bot, written in Rust (2024 edition) with ratatui + crossterm. It's a Cargo workspace:

| Crate | What |
|---|---|
| [`crates/grokbot_api`](crates/grokbot_api) | Library: async client for every known Grok Bot gateway call, the Cursor EnsureSandBox exchange, and the credential host allowlist |
| [`crates/gb`](crates/gb) | The `gb` binary (TUI). All networking goes through `grokbot_api` |

> ⚠️ **Unofficial and undocumented.** The Grok Bot gateway is a private API with no public spec.
> Its protocol was reverse-engineered from [grok-bot-cli](https://github.com/ScriptedAlchemy/grok-bot-cli)
> and **can change or break at any time**. Use `gb --mock` to try the UI without a server.

## Run

```sh
cargo run -p gb -- --mock        # offline: fake bots, delayed echo replies
cargo run -p gb                  # real gateway
cargo install --path crates/gb   # installs the `gb` binary
gb --data-dir /tmp/gbtest        # different data directory (holds context.db)
gb --account work                # start in an account (or GB_ACCOUNT=work gb)
```

In mock mode any token logs in except `bad`. Sending `reject` simulates a 4xx and `unknown` simulates a 5xx after the prompt has landed.

## Layout

```
┌ gb  account: default  login: cursor · box.cursorvm.com  bot: Helper ┐  top bar
│ @default ⇄    ┌ Chat: Helper · Helper is replying ⠹ ─────────────────┐│  account header (click to cycle)
│┌ Bots ──────┐│ you: hi                                               ││
││ Helper     ││ Helper: hello!                                        ││  body: command output
││ Coder      ││                                                       ││  and chat transcript
│└────────────┘└───────────────────────────────────────────────────────┘│
│┌ /command or message ─────────────────────────────────────────────────┐│  bottom input bar
││ > _                                                                  ││
│└──────────────────────────────────────────────────────────────────────┘│
```

- **Top bar:** account alias, login (method and gateway host), and active bot.
- **Account header:** the active account; click it to cycle through accounts.
- **Sidebar:** the account's bots (agents flagged hidden-from-sidebar are left out unless `show_hidden_bots` is on). Click one to switch to its chat.
- **Body:** each bot has its own log. Chats are restored from the local database, then kept current by polling.
- **Bottom bar:** `/`-prefixed input runs a command; plain text is sent to the active bot.

Keys: `Enter` submit · `Tab` toggle focus input/sidebar · `↑/↓` (or `j/k`) + `Enter` in sidebar to pick a bot ·
`↑/↓`, `PgUp/PgDn`, mouse wheel scroll body · `Esc` cancel a prompt · `Ctrl-C`/`Ctrl-D` quit.

### Responsiveness

The screen never waits on the network. Every gateway call runs on a tokio task and posts its result back over a channel.
Results that arrive after an account switch or logout are discarded.

Only the **active bot** is polled (`getAgentTranscriptTail`, last 50 entries):
- after a send: about every 1.5 s, backing off to 8 s while waiting for the reply ("… is replying" spinner in the body title, up to 5 min);
- otherwise: every 10 s, backing off to 60 s when nothing changes or on errors.

## Commands

| Command | Description |
|---|---|
| `/help [cmd]` (`/h`, `/?`) | List commands or show one |
| `/login gateway [url]` | Log in the active account with a gateway URL plus gateway token (token prompt is masked) |
| `/login cursor` | Log in with a Cursor access token (masked), exchanged via EnsureSandBox |
| `/login desktop [entry]` | Opt-in: log in with the Grok Bot desktop app's saved session (see Authentication) |
| `/logout` | Delete this account's stored credential and clear the session. Chats and memory are kept. (The gateway has no revoke call.) |
| `/whoami` | Show the account's login details (method, gateway, header names, whether a Cursor token is stored, env override) |
| `/bots` | Refresh and list all bots, including hidden ones and groups |
| `/bot <name\|id>` (`/use`) | Switch to a bot's chat |
| `/history` | Fetch the last 200 transcript entries for the active bot |
| `/account` (`/acct`) | List accounts (`*` = active) |
| `/account add <alias>` | Create an account |
| `/account use <alias>` | Switch account: reloads login, bots, chats, top bar |
| `/account rename <old> <new>` | Rename an account |
| `/account remove <alias>` | Delete an account and all its data; you confirm by typing the alias (the active account can't be removed) |
| `/prefs` (`/pref`) | List preferences; `/prefs get <key>`, `/prefs set <key> <value>` |
| `/memory` (`/mem`) | The active bot's local memory notes; `/memory add <key> <value>`, `/memory clear [key]` |
| `/clear [saved]` | Clear the body; with `saved`, also delete the bot's stored transcript |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `crates/gb/src/commands.rs`.

## Authentication

All methods are per account and stored in SQLite (see below).

1. **Gateway token** — `/login gateway <url>`, then paste the token. gb checks the URL against the allowlist,
   then validates the credential with `listAgents` (the gateway has no "who am I" call). It is saved only if that succeeds.
2. **Cursor access token** — `/login cursor`, then paste the token. gb calls
   `POST https://api2.cursor.sh/aiserver.v1.GrokBotService/EnsureSandBox` to get a gateway URL, token and
   extra headers (e.g. `x-anyrun-network-token`). The Cursor token is stored too, so when the gateway answers
   **401** gb re-exchanges once, retries, and saves the new session. A Cursor dashboard API key does not work here.

3. **Desktop app session (opt-in)** — `/login desktop [entry]`. gb reads the desktop app's saved session only when
   you run this command, mirroring grok-bot-cli's `app-session.js`. It reads `gateway-descriptor.json` from the app data dir:
   - Linux: `$XDG_CONFIG_HOME/Grok Bot` or `~/.config/Grok Bot`;
   - macOS: `~/Library/Application Support/Grok Bot`;
   - Windows: `%APPDATA%\Grok Bot`.

   Both descriptor formats are supported: v1 `{version, encrypted}` and v2 `{version: 2, entries}`. When a v2 file has
   several entries, gb lists them and you choose one with `/login desktop <entry>`. The payload is Chromium Safe Storage:
   - Linux `v11`: the password comes from `secret-tool lookup xdg:schema chrome_libsecret_os_crypt_password_v2 application 'Grok Bot'`;
   - Linux `v10`: the fixed `peanuts` password;
   - macOS: Keychain item `Grok Bot Safe Storage`, PBKDF2-SHA1 (`saltysalt`, 1003 rounds) and AES-128-CBC;
   - Windows: the DPAPI-protected key in `Local State` and AES-256-GCM.

   The decrypted `{baseUrl, token, headers?}` must pass the host allowlist and is validated with `listAgents`.
   It is then saved to the active account with method `desktop`. If the gateway later answers **401**, gb re-reads the
   desktop session once automatically, for that account only. gb never reads the desktop session in any other code path.
   You get clear errors when the app isn't installed, you aren't signed in, or `secret-tool` is missing. Tokens are never printed or logged.

   **Fallback when there is no `gateway-descriptor.json`:** some desktop builds only write `sand-secrets.json`.
   In that case `/login desktop` decrypts the `cursor-accounts` field (Chromium Safe Storage, same scheme as above),
   takes the Cursor access token, and exchanges it via EnsureSandBox exactly like `/login cursor`. The Cursor token
   itself is **not** stored — method stays `desktop` and a 401 re-reads `sand-secrets.json`. With several Cursor
   accounts, gb lists them as `1) <keyprefix>…` and you pick with `/login desktop <number>` (tip: `/account add`
   first so each maps to its own gb account). `local-exec-file-key`, `cursor-machine-id`, Cookies and `~/.grokbot/`
   are never read.

**Host allowlist.** Credentials are only sent to:
- gateway: `https` on `cursor.sh`, `cursor.com`, `*.cursor.sh`, `*.cursor.com`, `*.cursorvm.com`;
- EnsureSandBox backend (gets the Cursor token): `https` on `*.cursor.sh` / `*.cursor.com` only, never `*.cursorvm.com`.

Loopback gateways (`localhost`, `127.0.0.1`, `::1`) are allowed only with `GB_ALLOW_LOCAL_GATEWAY=1`. URLs containing userinfo are always rejected.
Redirects are never followed, requests time out after 30 s, responses are capped at 2 MiB, and tokens are redacted from error messages.

**Environment** (each applies only to the account selected at startup: `--account` / `GB_ACCOUNT` / last active):

| Variable | Meaning |
|---|---|
| `GB_GROKBOT_TOKEN` | **Gateway token** override; never saved. After `/account use` switches away, that account's stored login is used |
| `GB_GROKBOT_GATEWAY_URL` | Gateway URL for the token above (else the account's stored gateway URL) |
| `GB_GROKBOT_GATEWAY_HEADERS` | Optional JSON object of extra gateway headers |
| `GB_CURSOR_API_BASE_URL` | Override the EnsureSandBox backend (still must be `*.cursor.sh` / `*.cursor.com`) |
| `GB_ALLOW_LOCAL_GATEWAY` | `1` to permit loopback gateways (development) |
| `GB_ACCOUNT` | Starting account alias |

## Sending and delivery

`sendPrompt` returns only a receipt (`messageId`). Replies are picked up by polling the transcript.
- **Accepted** (receipt) → "replying" indicator until a new bot entry shows up.
- **Rejected** (4xx, or a disallowed URL) → shown as not delivered, and the local copy is removed.
- **Unknown** (5xx, network error, timeout, or no receipt) → gb **does not resend**. It checks the transcript right away, and the message shows as confirmed if it appears there. Check before resending yourself.

Your message is stored locally as pending and takes the transcript entry id once the gateway echoes it, so it is never duplicated.
Transcript text is parsed like grok-bot-cli's `transcript.js` (`text` / `prompt` / `message` / `content` parts / `message.content` / `preview`).
Terminal control sequences are stripped before display. Whether an entry is yours is a heuristic: its role/kind contains `user`, `human` or `prompt`.

## Accounts

Several **strictly separate accounts** (e.g. `personal` plus work accounts). Each has its own alias, login
(gateway URL, token, headers, method, Cursor token), bot list, chat transcripts, per-bot memory, and account preferences.

- Start-up account: `--account <alias>`, then `GB_ACCOUNT`, then the last active account, then `default`.
- Switch with `/account use <alias>` or click the `@alias` header.
- Removing an account deletes all of its rows (FK `ON DELETE CASCADE`).

## Storage

SQLite at `~/.gb/context.db` (directory `0700`, file `0600`), via `rusqlite` with bundled SQLite. Schema version is tracked in `PRAGMA user_version`.

| Table | Key | Columns |
|---|---|---|
| `accounts` | `id` | `alias` (unique, case-insensitive), `created_at` |
| `credentials` | `account_id` → accounts | `method` (`gateway`/`cursor`/`desktop`), `gateway_url`, `gateway_token`, `gateway_headers` (JSON), `cursor_token`, `desktop_entry`, `created_at`, `updated_at` |
| `bots` | `(account_id, bot_id)` → accounts | `name`, `description`, `hidden`, `is_group` |
| `messages` | `id`; `(account_id, bot_id)` → bots | `role`, `text`, `remote_id` (transcript id or `pending:<nonce>`, unique per account+bot), `created_at` |
| `memory` | `(account_id, bot_id, key)` → bots | `value`, `updated_at` |
| `account_prefs` | `(account_id, key)` → accounts | `value` |
| `global_prefs` | `key` | `value` |

**Migrations:** v1→v2 moved single-account data into account `default`. v2→v3 replaces the credentials table with the gateway schema.
The old placeholder-API tokens are **dropped** because they never pointed at a real server, so log in again. v2→v3 also adds `messages.remote_id` and the bot flags, and deletes the dead `api_base_url`, `request_timeout_secs` and `paths.*` prefs.
v3→v4 allows method `desktop` and adds `credentials.desktop_entry`, keeping existing logins.
The earlier one-time import from `~/.grokbot-tui` was removed, since everything it imported is now dead.

**Preferences:** account-scoped `default_bot`, `active_bot` (automatic), `show_hidden_bots` (`false`); global `ui.sidebar_width` (`24`), `active_account`.

## grokbot_api

See [`crates/grokbot_api/README.md`](crates/grokbot_api/README.md). It covers every gateway method grok-bot-cli uses: listAgents, sendPrompt,
getAgentTranscriptTail, getAgentThread, createAgent, updateAgent, deleteAgent, setAgentNotifyOnUpdates,
setAgentHiddenFromSidebar, createGroup, setGroupMembers, getAgentWorkflows, importAgentWorkflowText,
deleteAgentWorkflow, resolveLocalToolPermission and resolveAutoReviewApproval. It also covers EnsureSandBox and the URL policy.

## Development

```sh
cargo build
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests never contact real servers. They use temp directories and local `wiremock` servers, and loopback is enabled only through `Policy::local_testing()`.
