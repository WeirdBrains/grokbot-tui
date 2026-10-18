# Changelog

All notable changes to grokbot-tui are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
## [Unreleased]

## [1.7.0] - 2026-10-17

The theme system now matches acp-rs-scraper's.

### Added

- **`--theme <name>`** uses a theme for this run only; nothing is saved. A bad name, or a theme
  that is missing or can't be used, stops with an error before the UI starts. The error lists the
  themes you have. `default` may be missing from an empty themes folder, because the first-run
  download fetches it. Picking a theme in the app saves it as usual and ends the override.
- **`--list-themes`** prints the theme files and exits. `●` marks the one in use; each line shows
  the name, the description and any warnings, and broken files show why. It also says when the
  selected theme is missing. It never downloads.
- **`/theme export <name> [path]`** copies a theme file, with its exact text, out of the themes
  folder. By default it goes to `<data dir>/exports/<name>-<UTC time>.toml`; a folder argument
  gets `<name>.toml`, and `~/` is expanded. Only usable themes are exported, and an existing
  file is never overwritten.
- **Live preview in the theme pick list (`F5`):** moving through the list applies each theme as
  you go. `Enter` keeps it and `Esc` puts the old one back.
- **Theme editor:**
  - `[` and `]` step to the previous or next theme file. The dark/light tab is kept, and the
    step is refused while there are unsaved changes.
  - `Shift-F5` (or `F17`) closes the editor.
  - `Ctrl-D`, `Ctrl-N` and `Ctrl-X` work like `d`, `n` and `x`.
  - `⚠` in the list marks colors and parts below 3:1 contrast.

### Changed

- **Theme files are written atomically and privately.** The temp file is named with the process
  ID and created `0600`, then renamed into place; it is removed if the write fails. A new themes
  folder is created `0700`, and an existing folder's mode is left alone. Windows keeps its
  default permissions. Downloads and `/theme init` still never replace an existing file.
- **Theme files are size-checked before reading.** A file over 64 KiB is refused with its size,
  without being read into memory.
- **A theme file without a `schema` line now gets a warning.** It is still read as schema 1.
- **`/theme list`** shows each theme's description, uses `●` for the one in use, shows
  `BROKEN: <reason>` for broken files and says when the selected theme is missing.


## [1.6.0] - 2026-10-17

### Added

- **Theme editor: the selected color or part is highlighted in the live preview** (reverse
  video; `h` toggles). The preview title says when the selected part isn't on screen (the
  compact header, mouse capture off, a focused sidebar). The preview also shows sample chat
  entries and shell modal frames, so every part can be inspected even in an empty chat.
- **The default theme is downloaded on first run.** When `~/.grokbot-tui/themes` (or
  `<data dir>/themes`) has no themes, grokbot-tui fetches `themes/default.toml` from the git
  tag of the running version
  (`https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v<version>/themes/default.toml`)
  in the background. It checks that the file is a valid theme, saves it and shows a notice.
  Startup never waits (5 s timeout). Offline or on failure, plain terminal colors and a notice
  are kept. This works for every install method, including the macOS `.pkg`, the shell and
  PowerShell installers, and `cargo install`.
- **Opt-out:** `GROKBOT_TUI_NO_THEME_DOWNLOAD=1` or the new global pref `ui.theme_download`
  (`true` by default).
- **`/theme starter [name]`** writes the minimal starter theme (what `/theme init` used to do).

### Changed

- **Themes are user-level only:** they are read from `<data dir>/themes` and nowhere else.
- **`/theme init [name]`** now downloads the default theme for this version as `<name>.toml`.
  If downloading is off or fails, it writes the minimal starter instead.

### Removed

- **System-wide theme folders.** grokbot-tui no longer reads `/usr/share/grokbot-tui/themes`,
  `/usr/local/share/…`, Homebrew's `share/grokbot-tui/themes`, `$XDG_DATA_DIRS`, or a `themes/`
  folder next to the binary. It no longer copies themes from them on first run.
- The deb, rpm, AUR, MSI and Homebrew packages and the release archives no longer install
  `themes/default.toml`. The repo file stays as the download source. Copies already in your
  themes folder are kept, and files that 1.5.0 packages installed under
  `/usr/share/grokbot-tui/themes` are removed when the package is upgraded.

## [1.5.0] - 2026-10-17

### Added

- **Theme files.** Every theme, including `default`, is now a TOML file in
  `~/.grokbot-tui/themes/<name>.toml` (it follows `--data-dir`), with a dark and a light
  variant: 14 named palette colors plus a style (fg, bg, border, bold, italic, underline,
  reverse, dim) for each of 38 parts of the screen. Styles can point at palette colors
  (`"@accent"`). Nothing is built into the binary.
- **Shipped `themes/default.toml`**, the exact 1.4.0 look (including the logo colors and the
  non-bold title). The deb, rpm and AUR packages install it to `/usr/share/grokbot-tui/themes`,
  the release archives carry it in `themes/` (Homebrew installs it under
  `share/grokbot-tui/themes`), and the MSI puts it in `share\grokbot-tui\themes` next to `bin`.
  On startup, when your themes folder has no themes, the shipped ones are copied into it. Your
  files are never overwritten.
- **Theme switcher in the header** (`‹default› ▾`) and **F5**: a pick list of your themes, ending
  with *Edit themes…*. The choice is saved in the new global pref `ui.theme` (default `default`).
- **Theme editor** (**Shift-F5**, `/theme edit [name]`): the palette and every part on the left, a
  live preview of the app on the right, drawn by the real UI code with the unsaved edits. Tab flips
  the dark/light variant. Colors can be typed (hex, 0-255, names) or picked from a 256-color
  palette. Includes a WCAG contrast warning, per-item and whole-theme revert, save (applied right
  away to the theme in use), duplicate, new, and delete (asks first). It also asks before closing
  with unsaved changes, before replacing a broken file, and before overwriting a file that changed
  on disk. Works with the mouse too.
- **`/theme` subcommands:** `use [name]` (or `/theme <name>`), `list`, `edit [name]`,
  `init [name]` (writes a minimal starter, never overwrites), `delete <name>` (asks first) and
  `reload`. `/theme dark|light|system`, bare `/theme` and F4 still switch the mode.
- Help topics `theme` (rewritten) and `theme-files`. The keys and prefs pages and `/help theme` are updated.

### Changed

- When the selected theme is missing or broken, grokbot-tui uses the terminal's own colors
  (highlights stay visible via reverse video). A notice explains why and how to fix it, and the
  header shows `‹name: missing›` / `‹name: broken›`. Partial theme files load, with warnings:
  missing colors use the terminal default, missing parts use the starter rule, and a missing
  variant copies the other.

## [1.4.0] - 2026-10-17

### Added

- **Title style picked per terminal.** The big header title uses quadrant glyphs (4 columns
  per letter) again by default, and switches to the 1.3.2 half-height blocks in macOS
  Terminal (detected from `TERM_PROGRAM=Apple_Terminal`), whose fonts mangle quadrants.
- **`ui.title_glyphs` pref** (global): `auto` (default), `quadrant` or `half`. Set it with
  `/prefs set ui.title_glyphs half`, for example over ssh, where the terminal can't be
  detected and `auto` falls back to quadrants.

## [1.3.2] - 2026-10-17

### Fixed

- **Header title looked broken in macOS Terminal.** The big "Grok Bot" title now uses
  half-height pixels (only `▀ ▄ █`) instead of quadrant glyphs, which some terminal fonts
  draw with gaps and odd shapes. It is also no longer bold, since bold block glyphs can come
  from a different font. The title is 8 columns per letter now, so it shows when the
  terminal is at least 128 columns wide; narrower tall headers keep the logo and info rows.
- **Header logo looked like a blob.** It is now traced at 8 × 8 square pixels (it was
  stretched to 12 × 8) and its dark parts are near-black, so they stand out from the header
  background.

## [1.3.1] - 2026-10-17

### Fixed

- `/login desktop` (and any login) failing with "listAgents: gateway response too large
  (over 2097152 bytes)" on accounts with many bots. The TUI now accepts gateway replies up
  to 64 MiB, and `GROKBOT_TUI_MAX_RESPONSE_BYTES` overrides the cap.

## [1.3.0] - 2026-10-17

### Added

- **Tall header** (4 rows instead of 1): the Grok Bot logo as half-block art (made from the
  desktop app's icon), a big "Grok Bot" title (tui-big-text, quadrant pixels), and info
  rows for the account, theme, login, bot and mouse state.
- **Account switcher in the header**: `@alias ▾`. Click it or press F3 to open the account
  pick list.
- **Light theme and a dark / light / system switcher in the header.** Click a choice, press
  **F4** to cycle, or use `/theme [dark|light|system]`. `system` follows the terminal
  background (OSC 11 query at startup, else `$COLORFGBG`, else dark). The choice is saved
  in the global pref `ui.theme_mode` (default `system`). New `/help theme` page.

### Changed

- Terminals smaller than 80 × 24 get a compact one-row header with the same switchers.
- The sidebar's `@account` header is gone (the switcher lives in the top bar now), so the
  bot list starts right under the header.

## [1.2.4] - 2026-10-17

### Changed

- **Automatic Homebrew publishing is switched off for now**, until the
  `WeirdBrains/homebrew-tap` repo and the `HOMEBREW_TAP_TOKEN` secret exist. `"homebrew"` is
  commented out of `publish-jobs` in `dist-workspace.toml` (re-enabling is a one-line
  change plus `dist generate`; see PACKAGING.md). The formula `grokbot-tui.rb` is still
  built and attached to every release, and the `tap` setting is kept.

## [1.2.3] - 2026-10-17

### Added

- **Automatic Homebrew publishing.** Each release now pushes `Formula/grokbot-tui.rb` to
  `WeirdBrains/homebrew-tap` (cargo-dist `publish-jobs = ["homebrew", ...]`), including
  releases started by auto-release. It needs the `WeirdBrains/homebrew-tap` repo and a
  `HOMEBREW_TAP_TOKEN` secret; until then only that job fails and the GitHub Release is
  still published. See PACKAGING.md.

## [1.2.2] - 2026-10-17

### Changed

- **Package identifiers moved to WeirdBrains:**
  - Homebrew tap `m3talsmith/homebrew-tap` → `WeirdBrains/homebrew-tap`; install with
    `brew install WeirdBrains/tap/grokbot-tui`.
  - winget `PackageIdentifier` `m3talsmith.grokbot-tui` → `WeirdBrains.grokbot-tui`
    (manifests now render under `manifests/w/WeirdBrains/grokbot-tui/<version>/`),
    `Publisher` `WeirdBrains`, `PublisherUrl` <https://github.com/WeirdBrains>.
  - macOS `.pkg` bundle identifier `io.github.m3talsmith.grokbot-tui` →
    `io.github.weirdbrains.grokbot-tui`.

  The maintainer/author email and the MSI upgrade GUID are unchanged.

### Fixed

- The winget templates now quote `PackageVersion` and `InstallerSha256`, so a hash made
  only of digits (or a version like `1.3`) can't be read as a number and fail winget's
  schema validation.

## [1.2.1] - 2026-10-17

### Changed

- **Shorter environment variable names** (the 1.2.0 names repeated "GROKBOT"):

  | 1.2.0 | 1.2.1 |
  |---|---|
  | `GROKBOT_TUI_GROKBOT_TOKEN` | `GROKBOT_TUI_TOKEN` |
  | `GROKBOT_TUI_GROKBOT_GATEWAY_URL` | `GROKBOT_TUI_GATEWAY_URL` |
  | `GROKBOT_TUI_GROKBOT_GATEWAY_HEADERS` | `GROKBOT_TUI_GATEWAY_HEADERS` |
  | `GROKBOT_TUI_GROKBOT_APP_DIR` | `GROKBOT_TUI_APP_DIR` |

  1.2.0 was never published, so the long forms are not read. The pre-1.2 `GB_*` names
  still work as fallbacks (`GB_GROKBOT_TOKEN` → `GROKBOT_TUI_TOKEN`, and so on), and
  `GROKBOT_TUI_APP_DIR` / `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` are still forwarded to the
  library's `GROKBOT_APP_DIR` / `GROKBOT_ALLOW_LOCAL_GATEWAY`.
- **Repository URLs** now point at <https://github.com/WeirdBrains/grokbot-tui> (was
  `github.com/m3talsmith/gb`): `Cargo.toml` homepage/repository, the AUR PKGBUILD and
  winget templates (download, license and release-notes links), the Debian copyright
  file, the MSI help link and the README.

## [1.2.0] - 2026-10-17

### Changed

- **The Cargo package is renamed from `gb` to `grokbot-tui`**, matching the binary.
- **The project is now a single crate instead of a Cargo workspace.** `crates/gb` was the
  only member once `grokbot_api` moved to crates.io, so `src/`, `wix/` and the package
  manifest now live at the repository root (no `[workspace]`, no `crates/` directory).
  Build with `cargo build` and install from source with `cargo install --path .` from the
  repository root. The cargo-dist, cargo-deb, cargo-generate-rpm and WiX settings are in
  the root `Cargo.toml` / `dist-workspace.toml` with updated paths.
- **Release artifacts are renamed** from `gb-<target>.*` to `grokbot-tui-<target>.*`
  (e.g. `grokbot-tui-x86_64-unknown-linux-gnu.tar.xz`, `grokbot-tui-installer.sh`,
  `grokbot-tui-x86_64-pc-windows-msvc.msi`). The AUR, winget and Linux-package templates
  and workflows use the new names. The `.deb`/`.rpm` names and the MSI upgrade GUID are
  unchanged.
- **Environment variables now start with `GROKBOT_TUI_` instead of `GB_`.** The old names
  still work as fallbacks: at startup each `GB_*` value is copied to its `GROKBOT_TUI_*`
  name when that name isn't set, so the new name wins when both are set.

  | Old | New |
  |---|---|
  | `GB_GROKBOT_TOKEN` | `GROKBOT_TUI_GROKBOT_TOKEN` |
  | `GB_GROKBOT_GATEWAY_URL` | `GROKBOT_TUI_GROKBOT_GATEWAY_URL` |
  | `GB_GROKBOT_GATEWAY_HEADERS` | `GROKBOT_TUI_GROKBOT_GATEWAY_HEADERS` |
  | `GB_CURSOR_API_BASE_URL` | `GROKBOT_TUI_CURSOR_API_BASE_URL` |
  | `GB_CURSOR_CLIENT_VERSION` | `GROKBOT_TUI_CURSOR_CLIENT_VERSION` |
  | `GB_CURSOR_CLIENT_TYPE` | `GROKBOT_TUI_CURSOR_CLIENT_TYPE` |
  | `GB_SAND_BOX_NAMESPACE` | `GROKBOT_TUI_SAND_BOX_NAMESPACE` |
  | `GB_ACCOUNT` | `GROKBOT_TUI_ACCOUNT` |
  | `GB_ALLOW_LOCAL_GATEWAY` | `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` |
  | `GB_GROKBOT_APP_DIR` | `GROKBOT_TUI_GROKBOT_APP_DIR` |

  `GROKBOT_TUI_ALLOW_LOCAL_GATEWAY` and `GROKBOT_TUI_GROKBOT_APP_DIR` (or their `GB_`
  names) are still forwarded to `GROKBOT_ALLOW_LOCAL_GATEWAY` and `GROKBOT_APP_DIR`, the
  names `grokbot_api` reads; a library name that is already set wins.
- **The default data directory is now `~/.grokbot-tui`** (was `~/.gb`). `context.db`
  stays inside it, with the directory `0700` and the file `0600`. On startup, unless
  `--data-dir` is given:
  - if `~/.grokbot-tui` doesn't exist and `~/.gb` does, `~/.gb` is renamed to
    `~/.grokbot-tui`;
  - if the rename fails (e.g. across filesystems), the folder is copied and every file
    verified, `~/.gb` is left in place, and a notice is shown; if the copy also fails,
    grokbot-tui keeps using `~/.gb` and says so;
  - if both exist, `~/.grokbot-tui` is used and a one-time notice says `~/.gb` was
    ignored.

  User data is never deleted; remove `~/.gb` yourself once you no longer need it.
  `--data-dir` still overrides everything.
- Help text and docs call the app `grokbot-tui` and list the new names.

### Added

- **Automatic releases.** Pushing or merging to `main` with a bumped version in
  `Cargo.toml` builds a release: `.github/workflows/auto-release.yml` runs CI, creates and
  pushes the `v<version>` tag if it's missing (a tag you pushed yourself is used as is),
  and starts the cargo-dist release (archives, installers, Homebrew formula, `.pkg`,
  `.msi`, `.deb`/`.rpm`, AUR/winget manifests). It does nothing when that version already
  has a release or a release run in progress. `release.yml` is now started with
  `workflow_dispatch` (`dispatch-releases = true`) instead of a tag push, so the default
  `GITHUB_TOKEN` is enough; no personal access token is needed. See PACKAGING.md.
- **CI** (`.github/workflows/ci.yml`): `cargo fmt --check`, clippy with `-D warnings`,
  tests and a release build on pull requests and branch pushes.

## [1.1.0] - 2026-10-17

### Changed

- **All TUI styling now uses [`ratatui-lipgloss`](https://crates.io/crates/ratatui-lipgloss)
  1.1.** Colors, borders and padding for the top bar, sidebar, chat body, user bubbles,
  notices, input bar and the help, picker and `/shell` modals now live in one theme
  module (`crates/gb/src/theme.rs`); `ui.rs` only handles layout. Output looks the
  same as before.
- Upgraded `ratatui` 0.29 → 0.30 and `crossterm` 0.28 → 0.29 (required by
  `ratatui-lipgloss`).

## [1.0.0] - 2026-10-17

First release.

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

[Unreleased]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.7.0...HEAD
[1.7.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.6.0...v1.7.0
[1.6.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.5.0...v1.6.0
[1.5.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.4.0...v1.5.0
[1.4.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.3.2...v1.4.0
[1.3.2]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.3.1...v1.3.2
[1.3.1]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.3.0...v1.3.1
[1.3.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.2.4...v1.3.0
[1.2.4]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.2.3...v1.2.4
[1.2.3]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.2.2...v1.2.3
[1.2.2]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.2.1...v1.2.2
[1.2.1]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.2.0...v1.2.1
[1.2.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/WeirdBrains/grokbot-tui/releases/tag/v1.0.0
