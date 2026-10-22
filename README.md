# grokbot-tui — Grok Bot TUI

A terminal UI for Grok Bot, written in Rust (2024 edition, Rust 1.89 or newer) with ratatui + crossterm.
The `grokbot-tui` binary is this repository's single crate ([`src/`](src)). All networking goes through the
[`grokbot_api`](https://crates.io/crates/grokbot_api) crate from crates.io: an async client for every known
Grok Bot gateway call, the Cursor EnsureSandBox exchange, and the credential host allowlist.

> ⚠️ **Unofficial and undocumented.** The Grok Bot gateway is a private API with no public spec.
> Its protocol was reverse-engineered from [grok-bot-cli](https://github.com/ScriptedAlchemy/grok-bot-cli)
> and **can change or break at any time**. Use `grokbot-tui --mock` to try the UI without a server.

## Install

Prebuilt packages are attached to each [GitHub Release](https://github.com/WeirdBrains/grokbot-tui/releases)
(see [PACKAGING.md](PACKAGING.md) for how releases are made and what is published where):

- **Shell installer (macOS/Linux):** the `grokbot-tui-installer.sh` command shown in the release notes
- **PowerShell (Windows):** the `grokbot-tui-installer.ps1` command shown in the release notes
- **Windows:** `grokbot-tui-x86_64-pc-windows-msvc.msi` · **macOS:** `grokbot-tui-<arch>.pkg` (unsigned)
- **Debian/Ubuntu:** `sudo apt install ./grokbot-tui_<ver>-1_<arch>.deb`
- **Fedora:** `sudo dnf install ./grokbot-tui-<ver>-1.<arch>.rpm`
- **Homebrew, AUR, winget:** once published (see PACKAGING.md), `brew install WeirdBrains/tap/grokbot-tui`,
  `yay -S grokbot-tui-bin`, `winget install WeirdBrains.grokbot-tui`
- **From source:** `cargo install --path .` (from the repository root)

## Run

```sh
cargo run -- --mock                        # offline: fake bots, delayed echo replies
cargo run                                  # real gateway
cargo install --path .                     # installs the `grokbot-tui` binary
grokbot-tui --data-dir /tmp/gbtest         # different data directory (holds context.db)
grokbot-tui --account work                 # start in an account (or GROKBOT_TUI_ACCOUNT=work)
grokbot-tui --theme solar                  # use a theme for this run only (not saved)
grokbot-tui --list-themes                  # list the theme files (● = in use) and exit
```

In mock mode any token logs in except `bad`. Sending `reject` simulates a 4xx and `unknown` simulates a 5xx after the prompt has landed.


## Layout

```
 ▄▀▀▀▀▀▀▀▀▀▀▄  ▗▛▜▖        ▜▌      ▜▛▜▖     ▟     account  @default ▾  F3                       header
 ▀▀▀▀▀▀▀▀▀▀▀▀  █   ▜▟▜▖▟▀▙ ▐▌▟▘    ▐▙▟▘▟▀▙ ▝█▀    theme  ‹default› ▾  dark  light  system  F4   (logo, title,
 ▀▀▀▀▀▀▀▀▀▀▀▀  ▜▖▜▌▐▌▝▘█ █ ▐▛▙     ▐▌▐▌█ █  █▗    login cursor · box.cursorvm.com               switchers)
 ▀▀▀▀▀▀▀▀▀▀▀▀   ▀▀▘▀▀  ▝▀▘ ▀▘▝▘    ▀▀▀ ▝▀▘  ▝▘    mouse: on (F6)
┌ Bots ──────┐┌ Chat: Helper · Helper is replying ⠹ ──────────────────┐
│ Helper     ││ you: hi                                               │  body: command output
│ Coder      ││ Helper: hello!                                        │  and chat transcript
└────────────┘└───────────────────────────────────────────────────────┘
┌ /command or message ──────────────────────────────────────────────────┐  bottom input bar
│ > _                                                                   │
└───────────────────────────────────────────────────────────────────────┘
```

- **Header:** the Grok Bot logo and title, then the account switcher (`@alias ▾`: click it or press
  `F3` to pick an account), the theme switcher, the login (method and gateway host), the active bot and
  the mouse state. Terminals smaller than 80 × 24 get a one-row header with the same switchers.
- **Theme:** `‹default› ▾` is the theme in use. Click it or press `F5` to pick another one; the list
  ends with *Edit themes…* (also `Shift-F5`). See [Themes](#themes).
- **Mode:** `dark`, `light` or `system` picks the theme's variant. Click one in the header, press `F4` to cycle, or run
  `/theme [dark|light|system]`. `system` follows the terminal background (OSC 11 query at startup,
  else `$COLORFGBG`, else dark). Saved in the global pref `ui.theme_mode` (default `system`).
- **Sidebar:** the account's bots (agents flagged hidden-from-sidebar are left out unless `show_hidden_bots` is on). Click one to switch to its chat.
- **Body:** each bot has its own log. Chats are restored from the local database, then kept current by polling.
- **Bottom bar:** `/`-prefixed input runs a command; plain text is sent to the active bot.

Keys: `Enter` submit · `Tab` toggle focus input/sidebar · `↑/↓` (or `j/k`) + `Enter` in sidebar to pick a bot ·
`↑/↓`, `PgUp/PgDn`, mouse wheel scroll body · `Esc` cancel a prompt · `Ctrl-C` / `Ctrl-Shift-C` are context-aware (copy a selection, cancel a reply, clear or copy
the input, close an overlay, or ask before quitting; `Ctrl-Shift-C` never quits) ·
`Ctrl-Q` or `/quit` quit at once ·
`F1` help · `F2` pick a bot · `F3` pick an account · `F4` cycle dark/light/system · `F5` pick a theme ·
`Shift-F5` theme editor · `F6` mouse capture on/off.

### Vim-like keys (opt-in)

`/keymap vim` (or pref `ui.keys=vim`) enables NORMAL / INSERT / COMMAND / VISUAL modes
with a mode badge on the input. Bindings live in `~/.grokbot-tui/keymap.toml`
(`/keymap reload` after editing). Default stays classic. See `docs/vim-mode-plan.md`.

## Prompt history and completion

Up/Down in the input walk lines, then the last ~500 prompts for the active account
(stored in `context.db`). Ctrl-R opens a fuzzy history search (Ctrl-S reverse; Esc keeps
the draft). Tab after `/` fuzzy-completes slash commands and arguments; Shift-Tab cycles.

## Selecting text

With mouse capture on (the default; F6 / `/mouse` toggles and persists as `ui.mouse_capture`),
drag in the chat to select. The selection is copied on mouse-up (OSC 52). Ctrl-C /
Ctrl-Shift-C also copy a selection. Esc clears it. Shift-drag or Option-drag usually
bypasses mouse capture so the terminal can select instead.

## Themes

Every theme, including `default`, is a TOML file in `~/.grokbot-tui/themes/<name>.toml` (it follows
`--data-dir`), and themes are read only from that folder: nothing is built into the binary and no
system folder is searched. Each file has a `[dark]` and a `[light]` variant:
a palette of 14 named colors and a style for each part of the screen.

```toml
schema = 1
name = "Default"
description = "The original grokbot-tui look"

[dark.colors]
text = 252                 # 0-255, "#rrggbb", "#rgb", a color name, or "reset" (terminal default)
background = 234
accent = "cyan"
# … subtle, muted, header, warn, danger, ok, on_accent, modal_text, modal_background,
#   logo_dark, logo_light

[dark.styles]              # fg, bg, border (boxes only), bold, italic, underline, reverse, dim
brand = { fg = "@on_accent", bg = "@accent", bold = true }   # "@name" = a palette color
panel_focused = { fg = "@text", border = "@accent" }
# …

[light.colors]
# …
```

- **Where `default.toml` comes from:** on first run, when your themes folder has no themes,
  grokbot-tui downloads the repo's [`themes/default.toml`](themes/default.toml) from the git tag
  matching its version
  (`https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v<version>/themes/default.toml`),
  checks that it's a valid theme, saves it there and shows a notice. This works the same for every
  install method. Startup never waits for it: plain terminal colors until it arrives, and it gives
  up after 5 seconds. Offline or failed: plain colors plus a notice; `/theme init` tries again. Your
  files are never overwritten. **Opt out** with `GROKBOT_TUI_NO_THEME_DOWNLOAD=1` or
  `/prefs set ui.theme_download false`.
- **Missing or broken theme:** grokbot-tui uses your terminal's own colors (highlights stay visible
  via reverse video), shows a notice saying why, and marks the header `‹name: missing›` /
  `‹name: broken›`. A broken file is never overwritten without asking. Partial files are fine:
  missing colors use the terminal default, missing parts use the starter rule, a missing variant
  copies the other, and each problem is listed once (as are unknown keys and a missing `schema`
  line). Files over 64 KiB are refused before they're read, and a newer `schema` is refused.
- **Saving** is atomic (a temp file in the same folder, then a rename) and private: theme files are
  `0600` and a new themes folder is `0700` (Windows keeps its default permissions).
- **Picking (`F5`):** moving through the list previews each theme live; `Enter` keeps it, `Esc`
  goes back. `--theme <name>` picks one for a single run without saving it (a missing or broken
  theme stops before the UI starts); `--list-themes` prints the list and exits.
- **`/theme export <name> [path]`** copies a theme file (exact text) to
  `<data dir>/exports/<name>-<UTC time>.toml`, or to the path given (a folder gets `<name>.toml`).
  It never overwrites.
- **`/theme init [name|all]`** downloads `<name>.toml` from the release's `themes/` folder the same
  way (default `default`; `all` = every shipped theme, currently just `default`). Existing files
  are kept. With downloading off it writes the minimal starter instead. `--list-themes` and
  `--theme` also fetch the default first when the folder is empty (waiting up to 3 s).
  `GROKBOT_TUI_THEME_URL` replaces the download address (`<url><name>.toml`).
- **`/theme starter [name]`** always writes the minimal starter: every part is mapped to a palette
  role (title → `@accent`, errors → `@danger`, …) but every color is the terminal default. Set the
  colors in the editor.
- **Editor (`Shift-F5` or `/theme edit [name]`):** the palette and every part on the left, a live
  preview of the app on the right (drawn by the real UI code with your unsaved changes, plus sample
  chat entries and modals). The selected color or part is marked in reverse video in the preview
  (`h` toggles; the preview title says when the part isn't on screen). `Tab` flips
  dark/light, `Enter`/`e` edits (hex, 0-255, a name or `@palette`), `p` opens the color picker (tabs: 16 colors, the 216-color cube, 24 grays, `@palette` refs),
  `r`/`R` revert the item/everything, `s` saves (applied immediately if it's the theme in use),
  `d` duplicates, `n` creates a starter theme, `x` deletes (asks first; `Ctrl-D`/`Ctrl-N`/`Ctrl-X`
  work too), `t` opens another theme and `[`/`]` step to the previous/next one, `Esc` (or
  `Shift-F5`) closes (asks about unsaved changes). Text/background pairs below a 3:1 WCAG contrast
  ratio get a warning, and `⚠` marks them in the list. Mouse clicks work on the list, tabs, fields and palette.

### Responsiveness

The screen never waits on the network. Every gateway call runs on a tokio task and posts its result back over a channel.
Results that arrive after an account switch or logout are discarded.

Only the **active bot** is polled (`getAgentTranscriptTail`, last 100 entries):
- after a send: about every 1 s, backing off to 8 s while waiting for the reply ("… is replying" spinner in the body title, up to 5 min);
- otherwise: every 1 s, backing off to 15 s when nothing changes or on errors. The visible feed is re-read from local SQLite every 100 ms (no network).

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
| `/theme [dark\|light\|system]` (`/mode`) | Switch dark/light mode; bare `/theme` cycles (also `F4` or a header click) |
| `/theme use [name]` | Switch theme (also `/theme <name>`); no name: pick list (also `F5`) |
| `/theme list` · `/theme reload` | List theme files (`●` = in use; broken files say why) · re-read them after editing by hand |
| `/theme edit [name]` | Open the theme editor (also `Shift-F5`) |
| `/theme init [name\|all]` | Download a shipped theme from the release (else write the minimal starter; never overwrites) |
| `/theme starter [name]` | Write a minimal starter theme (no download; never overwrites) |
| `/theme delete <name>` | Delete a theme file (confirm in a pick list) |
| `/theme export <name> [path]` | Copy a theme file out (default `<data dir>/exports/<name>-<UTC time>.toml`; never overwrites) |
| `/memory` (`/mem`) | The active bot's local memory notes; `/memory add <key> <value>`, `/memory clear [key]` |
| `/quit` (`/q`, `/exit`) | Exit |

Commands live in a registry in `src/commands.rs`.

## Authentication

All methods are per account and stored in SQLite (see below).

1. **Gateway token** — `/login gateway <url>`, then paste the token. grokbot-tui checks the URL against the allowlist,
   then validates the credential with `listAgents` (the gateway has no "who am I" call). It is saved only if that succeeds.
2. **Cursor access token** — `/login cursor`, then paste the token. grokbot-tui calls
   `POST https://api2.cursor.sh/aiserver.v1.GrokBotService/EnsureSandBox` to get a gateway URL, token and
   extra headers (e.g. `x-anyrun-network-token`). The Cursor token is stored too, so when the gateway answers
   **401** grokbot-tui re-exchanges once, retries, and saves the new session. A Cursor dashboard API key does not work here.

3. **Desktop app session (opt-in)** — `/login desktop [entry]`. grokbot-tui reads the desktop app's saved session only when
   you run this command, mirroring grok-bot-cli's `app-session.js`. It reads `gateway-descriptor.json` from the app data dir:
   - Linux: `$XDG_CONFIG_HOME/Grok Bot` or `~/.config/Grok Bot`;
   - macOS: `~/Library/Application Support/Grok Bot`;
   - Windows: `%APPDATA%\Grok Bot`.

   Both descriptor formats are supported: v1 `{version, encrypted}` and v2 `{version: 2, entries}`. When a v2 file has
   several entries, grokbot-tui lists them and you choose one with `/login desktop <entry>`. The payload is Chromium Safe Storage:
   - Linux `v11`: the password comes from `secret-tool lookup xdg:schema chrome_libsecret_os_crypt_password_v2 application 'Grok Bot'`;
   - Linux `v10`: the fixed `peanuts` password;
   - macOS: Keychain item `Grok Bot Safe Storage`, PBKDF2-SHA1 (`saltysalt`, 1003 rounds) and AES-128-CBC;
   - Windows: the DPAPI-protected key in `Local State` and AES-256-GCM.

   The decrypted `{baseUrl, token, headers?}` must pass the host allowlist and is validated with `listAgents`.
   It is then saved to the active account with method `desktop`. If the gateway later answers **401**, grokbot-tui re-reads the
   desktop session once automatically, for that account only. grokbot-tui never reads the desktop session in any other code path.
   You get clear errors when the app isn't installed, you aren't signed in, or `secret-tool` is missing. Tokens are never printed or logged.

   **Fallback when there is no `gateway-descriptor.json`:** some desktop builds only write `sand-secrets.json`.
   In that case `/login desktop` decrypts the `cursor-accounts` field (Chromium Safe Storage, same scheme as above),
   takes the Cursor access token, and exchanges it via EnsureSandBox exactly like `/login cursor`. The Cursor token
   itself is **not** stored — method stays `desktop` and a 401 re-reads `sand-secrets.json`. With several Cursor
   accounts, grokbot-tui lists them as `1) <keyprefix>…` and you pick with `/login desktop <number>` (tip: `/account add`
   first so each maps to its own grokbot-tui account). `local-exec-file-key`, `cursor-machine-id`, Cookies and `~/.grokbot/`
   are never read.

**Host allowlist.** Credentials are only sent to:
- gateway: `https` on `cursor.sh`, `cursor.com`, `*.cursor.sh`, `*.cursor.com`, `*.cursorvm.com`;
- EnsureSandBox backend (gets the Cursor token): `https` on `*.cursor.sh` / `*.cursor.com` only, never `*.cursorvm.com`.

Loopback gateways (`localhost`, `127.0.0.1`, `::1`) are allowed only with `GROKBOT_ALLOW_LOCAL_GATEWAY=1`. URLs containing userinfo are always rejected.
Redirects are never followed, requests time out after 30 s, responses are capped at 2 MiB, and tokens are redacted from error messages.

**Environment** (each applies only to the account selected at startup: `--account` / `GROKBOT_TUI_ACCOUNT` / last active):

| Variable | Meaning |
|---|---|
| `GROKBOT_TUI_TOKEN` | **Gateway token** override; never saved. After `/account use` switches away, that account's stored login is used |
| `GROKBOT_TUI_GATEWAY_URL` | Gateway URL for the token above (else the account's stored gateway URL) |
| `GROKBOT_TUI_GATEWAY_HEADERS` | Optional JSON object of extra gateway headers |
| `GROKBOT_TUI_CURSOR_API_BASE_URL` | Override the EnsureSandBox backend (still must be `*.cursor.sh` / `*.cursor.com`) |
| `GROKBOT_TUI_CURSOR_CLIENT_VERSION` | Override `x-cursor-client-version` (else pref `cursor.client_version`, else detect installed Grok Bot, else `0.70.2`) |
| `GROKBOT_TUI_CURSOR_CLIENT_TYPE` | Override `x-cursor-client-type` (default `sand`) |
| `GROKBOT_TUI_SAND_BOX_NAMESPACE` | Override `x-sand-box-namespace` (default `prod`) |
| `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` | `1` to permit loopback gateways (development) |
| `GROKBOT_TUI_MAX_RESPONSE_BYTES` | Cap on one gateway response body in bytes (default 64 MiB) |
| `GROKBOT_TUI_APP_DIR` | Grok Bot desktop app install directory, for client version detection |
| `GROKBOT_TUI_NO_THEME_DOWNLOAD` | `1` = never download the default theme (see [Themes](#themes)) |
| `GROKBOT_TUI_THEME_URL` | Download themes from this URL base instead of the release tag (`<url><name>.toml`; used by the tests) |
| `GROKBOT_TUI_ACCOUNT` | Starting account alias |

`GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` and `GROKBOT_TUI_APP_DIR` are forwarded at startup to
`GROKBOT_ALLOW_LOCAL_GATEWAY` and `GROKBOT_APP_DIR`, the names the `grokbot_api` library reads (setting the
library names directly also works).

**Older names.** Before 1.2.0 these variables started with `GB_` (e.g. `GB_GROKBOT_TOKEN`,
`GB_ACCOUNT`). Every `GB_*` name still works as a fallback: at startup grokbot-tui copies its value to the
matching `GROKBOT_TUI_*` name when that one isn't set, so the new name wins when both are set.

## Chat sync

Bots come from `listAgents`. Chat text comes from `getAgentTranscriptTail` (there is no separate
listChats/getChat API). On bot select (and on `/history`), grokbot-tui pages that endpoint via
`beforeSeq`/`nextBeforeSeq` up to 1000 entries, merges into SQLite, and rebuilds the body from the
DB so messages from the desktop app show up. While a bot is active it also polls the newest page
every few seconds (backing off to 15s when idle).

## Sending and delivery

`sendPrompt` returns only a receipt (`messageId`). Replies are picked up by polling the transcript.
- **Accepted** (receipt) → "replying" indicator until a new bot entry shows up.
- **Rejected** (4xx, or a disallowed URL) → shown as not delivered, and the local copy is removed.
- **Unknown** (5xx, network error, timeout, or no receipt) → grokbot-tui **does not resend**. It checks the transcript right away, and the message shows as confirmed if it appears there. Check before resending yourself.

Your message is stored locally as pending and takes the transcript entry id once the gateway echoes it, so it is never duplicated.
Transcript text is parsed like grok-bot-cli's `transcript.js` (`text` / `prompt` / `message` / `content` parts / `message.content` / `preview`).
Terminal control sequences are stripped before display. Whether an entry is yours is a heuristic: its role/kind contains `user`, `human` or `prompt`.

## Accounts

Several **strictly separate accounts** (e.g. `personal` plus work accounts). Each has its own alias, login
(gateway URL, token, headers, method, Cursor token), bot list, chat transcripts, per-bot memory, and account preferences.

- Start-up account: `--account <alias>`, then `GROKBOT_TUI_ACCOUNT`, then the last active account, then `default`.
- Switch with `/account use <alias>` or click the `@alias` header.
- Removing an account deletes all of its rows (FK `ON DELETE CASCADE`).

## Storage

SQLite at `~/.grokbot-tui/context.db` (directory `0700`, file `0600`; override with `--data-dir`), via `rusqlite` with bundled SQLite. Schema version is tracked in `PRAGMA user_version`.

**Moving from `~/.gb`.** Before 1.2.0 the data lived in `~/.gb`. On startup (unless `--data-dir` is given):

- If `~/.grokbot-tui` doesn't exist and `~/.gb` does, `~/.gb` is renamed to `~/.grokbot-tui`.
- If that rename fails (e.g. the two are on different filesystems), the folder is copied, every file is
  verified, and `~/.gb` is left in place with a notice. If even the copy fails, grokbot-tui keeps using `~/.gb`.
- If both exist, `~/.grokbot-tui` is used and a one-time notice says `~/.gb` was ignored.

User data is never deleted; remove `~/.gb` yourself once you no longer need it.

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

**Preferences:** account-scoped `default_bot`, `active_bot` (automatic), `show_hidden_bots` (`false`); global `ui.sidebar_width` (`24`), `ui.theme` (`default`), `ui.theme_download` (`true`), `ui.theme_mode` (`dark`/`light`/`system`, default `system`), `active_account`.

## grokbot_api

The gateway client is published separately as [`grokbot_api`](https://crates.io/crates/grokbot_api)
([source](https://github.com/WeirdBrains/grokbot_api), [docs](https://docs.rs/grokbot_api)). It covers every gateway method grok-bot-cli uses: listAgents, sendPrompt,
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

## In-app help

Press **F1** (or **?** on an empty input) or type `/help`. Help opens in a scrollable
popup and is never written to chat history. Topics: `login`, `login desktop`
(recommended), `login cursor`, `login gateway`, `keys`, `accounts`, `prefs`, `env`,
`sync`, `delivery`, `troubleshooting`, plus `/help <command>`. Each page lists related
topics; Tab/←→ selects one, Enter opens it, Backspace goes back, Esc closes.
`/help <keyword>` (or `/` inside the help window) searches every topic and command
page and lists matches with a snippet; pick one with Tab/←→ and Enter. A bare
`/login` opens the login help. Mistyped commands get a "did you mean" suggestion.

## Chat feed and the database

SQLite (`~/.grokbot-tui/context.db`) is the single source of truth for the chat feed. The body
is always rebuilt from `store.messages(bot)`: polled messages and bot replies are merged
into the DB first, and your own message is written as a pending row and the feed is
rebuilt immediately (so it still appears instantly). Notices — command echoes, info,
errors, delivery warnings — are transient: they are kept in memory per bot, shown in a
separate "notices (not saved)" section below the chat, and never written to the DB.

## License

BSD 3-Clause. See [LICENSE](LICENSE).

## Troubleshooting

### `grokbot-tui: command not found` after install

The binary lands in `$CARGO_HOME/bin` (usually `~/.cargo/bin`). Put that directory on
`PATH`, then refresh the shell's command cache:

```bash
# zsh
rehash
# bash
hash -r
```

Optional (zsh): auto-rehash when a command is not found —

```zsh
zstyle ':completion:*' rehash true
```

Or run `./install.sh` from a checkout, which installs and reminds you to rehash.
