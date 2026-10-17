# Changelog

All notable changes to grokbot-tui are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
No version has been tagged yet; everything below ships in the first release
(the workspace version is currently 1.0.0).

## [Unreleased]

### Changed

- **`grokbot_api` now comes from crates.io.** gb depends on
  [`grokbot_api` 1.1](https://crates.io/crates/grokbot_api) (with the `desktop-session`
  feature). The in-repo `crates/grokbot_api` copy was removed; the library now lives at
  [WeirdBrains/grokbot_api](https://github.com/WeirdBrains/grokbot_api). Building gb now
  requires Rust 1.89 or newer.
- **Renamed environment variables** (read by the library):
  - `GB_ALLOW_LOCAL_GATEWAY` → `GROKBOT_ALLOW_LOCAL_GATEWAY`
  - `GB_GROKBOT_APP_DIR` → `GROKBOT_APP_DIR`

  The old names still work: at startup gb copies an old name's value to the new name when
  the new one isn't set.
- When `/login desktop` finds several saved sessions or Cursor accounts, gb now adds its own
  instructions (`/login desktop <name|number>`, `/account add <alias>`) to the library's
  error message, which no longer names them.

### Added

- **Terminal UI.** A ratatui app with a sidebar of bots, a chat body, an input bar and
  a registry of slash commands. Includes an offline mock backend (`--mock`).
- **Real Grok Bot gateway.** New `grokbot_api` crate (Cargo workspace) with an
  async client for the unofficial gateway API.
  - Logins: `/login gateway` (URL + token) and `/login cursor` (Cursor access token
    exchanged via `EnsureSandBox`).
  - The UI stays responsive while API calls run.
- **`/login desktop`** (opt-in): reuses the Grok Bot desktop app's saved session.
  - Supports Linux secret-tool, the macOS Keychain and Windows DPAPI.
  - Falls back to the desktop app's Cursor token plus `EnsureSandBox`.
  - Detects the desktop app's version for client headers.
- **Multi-account support.** Accounts are kept separate in storage. Manage them with
  `/account`, or choose one at startup with `--account` / `GB_ACCOUNT`.
- **SQLite storage** at `~/.gb/context.db` for chats, bots and settings, with schema
  migrations. Adds the `/prefs` and `/memory` commands.
- **Persistent sessions.** Saved logins (file mode 0600) are restored on startup and
  removed only by `/logout`.
  - An expired desktop session is re-read automatically.
  - Network errors are retried with backoff.
- **In-TUI help.**
  - A topic modal with related links you can follow, plus a keys overlay (F1, `?`).
  - Keyword search across help.
  - Guidance when the chat is empty, hints on errors, and "did you mean" suggestions.
  - `/whoami` shows where the client version came from.
- **Delivery tracking.** Sends whose outcome is unknown are resolved with
  `promptAcceptanceStatus` (accepted / rejected / not found / pending).
- **Reusable selection modal** (keyboard and mouse), used for:
  - bot switching (F2, bare `/bot`) and account switching (F3, `/account use`,
    clicking the account header);
  - confirming account removal;
  - choosing between several desktop accounts.
- **`/bot new <name>` and `/bot delete [name]`** via the gateway's `createAgent` and
  `deleteAgent`. Deleting asks for confirmation first, with Cancel as the default.
- **`/shell <command>`** (alias `/sh`) runs a local command off the UI thread.
  - Output, stderr and the exit code appear in a scrollable modal.
  - Capped at 500 lines / 64 KB, with a 120 s timeout.
  - Never sent to a bot and never saved as chat.
- **Multi-line input.**
  - Shift-Enter (kitty keyboard protocol / modifyOtherKeys), Alt-Enter or Ctrl-J
    insert a newline.
  - Cursor movement works across lines, and long lines wrap.
  - The input box grows up to 10 rows, then scrolls.
- **Text selection and copy.** F6 or `/mouse [on|off]` turns mouse capture off so the
  terminal can select natively (Shift-drag also works in most terminals). `/copy` copies
  the last bot message via OSC 52.
- **Team bots.** Bots shared with your team (e.g. "Chomp") now appear in the sidebar,
  marked `team`.
  - They are listed via the Cursor backend's `ListGrokBotTeamAgents`.
  - Calls for them carry their viewer session.
- **Release packaging.**
  - A cargo-dist GitHub Actions workflow builds Linux (x86_64, aarch64), macOS
    (x86_64, arm64) and Windows (x64) on tag push.
  - It produces archives, shell/PowerShell installers, a Homebrew formula, a WiX MSI,
    an unsigned macOS `.pkg`, and `.deb`/`.rpm` packages.
  - AUR PKGBUILDs and winget manifests are rendered from the release's real checksums.
  - See `PACKAGING.md`.
- **License.** The project is licensed under BSD-3-Clause (`LICENSE` at the repo root).
  The license metadata is set in Cargo and every package format, and the license
  text ships in the archives, `.deb`, `.rpm` and AUR packages.

### Changed

- The installed binary is named `grokbot-tui` (the Cargo package is still `gb`, so a
  shell alias `gb=git branch` no longer shadows it).
- The token environment variable was renamed from `GROKBOT_TOKEN` to `GB_GROKBOT_TOKEN`.
- **The chat feed is rendered only from SQLite**, which is the single source of truth.
  Notices are transient, shown in their own section and never saved. `/clear` was
  removed.
- **Message order follows the gateway transcript's order** (schema v5 `ord` column), not
  insertion order. Pending sends take their real place once confirmed.
- **Timing.** The UI redraw loop is separate from gateway polling: local DB refresh
  every 100 ms, active gateway poll every 1 s, with backoff when idle.
- **Chat layout.**
  - Padding and a blank line between messages.
  - Your own messages are right-aligned as a clean block.
  - Message bubbles wrap at no more than 2/3 of the chat width.

### Fixed

- **Chat sync.** Selecting a bot (or `/history`) now loads the full transcript instead
  of only the last 50 entries, so messages from the desktop app show up.
- **Newest and own messages missing.** Scrolling to the bottom now uses the exact
  wrapped height, and rows without an order sort last (schema v6 repair).
- **"Delivery unknown" on every send.** The gateway's real success response
  (`{accepted: true}`) is now recognised, and refusals are reported as rejected.
- **Endless HTTP 417 errors.** The gateway answers an expired session with 417; it is now
  handled like 401, re-reading or re-exchanging the credential.
- A build break caused by quotes inside the delivery help text.
