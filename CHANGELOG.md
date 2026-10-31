# Changelog

All notable changes to grokbot-tui are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project aims to follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
## [Unreleased]

## [1.34.0] - 2026-10-31

### Added
- Grok Bot notifications: a header **🔔** bell with the unread count, a notifications page
  (bell, **Alt-b**, `/notifications`, vim `N` / keymap `notifications.open`) and small
  toasts stacked in the top-right corner.
- Sources (the gateway has no notifications API, so these are what it really returns):
  transcript `kind: notice` entries (Grok Bot system notices), `listAgents`
  `unreadCount`/`hasUnread` and `awaitingUserResponse` for bots you aren't viewing, and
  pending cards. **Usage / low credits** is derived: a notice or message preview that
  mentions running out of usage or credits becomes a *Usage* notification that opens
  Settings → Usage & Billing (there is no usage API).
- Read state: clicking a toast marks it read, removes it and opens its context (the bot's
  chat, the card, or Settings). Closing with **[x]** or the timeout only removes the toast;
  it stays unread on the page. The bell counts unread only. Read items stay on the page,
  dimmed and marked *read*, until cleared.
- Page: newest first; Enter open, m toggle read, a mark all read, c / [Clear all] clear
  (asks y/n); `/notifications read|clear`. Clear and read state are local (per account,
  `notifications-<account>.json` next to `context.db`); cleared items don't come back.
- Pref `notifications.toast_timeout_secs` (default 20, 1–600). Theme parts `bell`,
  `notification_toast`, `notification_unread`, `notification_read`.

## [1.33.0] - 2026-10-31

### Added
- Settings page for the selected account and bot: header **[⚙ Settings]** button (`[⚙]` in
  the compact header), **Alt-,** (Ctrl-, with the kitty protocol), `/settings [tab]`, vim
  `s` / keymap action `settings.open`. Tabs follow the Grok Bot app (General, Computer,
  Vault, Usage & Billing, Updates) plus **Bot** and **Local (grokbot-tui)**. Esc goes back.
- Bot tab reads the bot's `listAgents` record live. Editable: name, title, description,
  avatar shape and color (`updateAgent`), notify on updates (`setAgentNotifyOnUpdates`),
  hidden from sidebar (`setAgentHiddenFromSidebar`). Read-only: notifications, voice,
  harness. Team bots are read-only (their settings belong to the owner).
- Account/app settings have no gateway API, so their rows show *Open in Grok Bot →* and
  open the app's `grokbot://app/v1/settings?id=<row>` deep link. Reset computer, cancel
  trial and sign out ask y/n first; reset/cancel still happen in the app, never from the TUI.
- Local tab: links.open_with, appearance, compact header, collapsed sidebar, files panel,
  mouse capture, spinner (editable); theme opens the picker; key scheme, SSH link rule and
  refresh interval shown.
- Theme parts `settings_frame`, `settings_tab`, `settings_button`.

### Changed
- Attachment and file links now follow `links.open_with` too (system browser selected →
  system opener), like chat links and Settings links. Non-text files still use the system
  opener because the built-in browser shows text only.

## [1.32.0] - 2026-10-31

### Added
- Loading overlay with an animated braille spinner and a stage message at startup, on
  account switch (`/account use`) and during sign-in (`/login gateway`, `/login cursor`'s
  EnsureSandBox exchange, `/login desktop`, saved-login restore): *Signing in to <account>…*,
  *Checking gateway <host>…*, *Exchanging Cursor token…*, *Reading the desktop app
  session…*, *Loading bots…*, *Loading chat with <bot>…*. Non-blocking: input and quit keys
  keep working.
- Errors replace the spinner with a retry hint (Esc dismisses) instead of hanging.
- Pref `ui.spinner` (default on; `off` = static glyph). Theme parts `loading` and `spinner`.

## [1.31.0] - 2026-10-31

### Added
- MCP server: `grokbot-tui mcp serve [--stdio | --http [ADDR]] [--read-only]`, built on
  the official `rmcp` SDK. Tools `list_bots`, `get_bot`, `send_message`, `read_messages`,
  `wait_for_reply`, `list_pending_cards`, `answer_card` (all card kinds: questions,
  approvals, local tool permissions, secrets, forms, drafts), `list_attachments`,
  `download_attachment`; resources `grokbot://bots` and `grokbot://bots/{id}/transcript`.
- Streamable HTTP transport on loopback only, behind a bearer token kept in
  `<data dir>/mcp-http-token` (0600). `grokbot-tui mcp token [--rotate]` prints it;
  `grokbot-tui mcp config` shows the settings in use.
- `<data dir>/mcp.toml` `[mcp.server]`: `allowed_bots`, `read_only`, `http_addr`,
  `audit`, `download_dir`. Audit log at `~/.cache/grokbot-tui/mcp-audit.log` (no message
  bodies, answers, secrets, tokens or URLs).
- Example client configs for Claude Code and Cursor in `docs/mcp/`.

## [1.30.1] - 2026-10-30

### Changed
- CI: build macOS on macos-15. The Apple Silicon builds (dist's aarch64-apple-darwin job
  and the -ffmpeg variant) moved off the retiring macos-14 image (`github-custom-runners`
  in dist-workspace.toml; Intel stays on macos-15-intel). No app changes.

## [1.30.0] - 2026-10-30

### Added
- [⧉] copy button on every chat bubble (yours and the bot's, left of [☺+]): copies the
  message's raw text / markdown source, not the wrapped screen lines. Also in the
  click-to-react popup ([⧉ copy], Ctrl-Y, `y` in vim list mode) and keymap action
  `message.copy` (vim normal `Y`, the last message).
- [⧉] copy button at the top-right of every code block: copies just the code (no fences,
  no wrapping, original indentation). Code blocks always get a header row for it.
- Copies also go to the system clipboard when running locally (wl-copy, xclip, xsel or
  pbcopy), alongside OSC 52 (kept for SSH), with a notice like "Copied 12 lines.".
- Theme part `copy_button`.

### Fixed
- Drag selection no longer picks up TUI chrome: selected cells map back to each
  message's source text, leaving out bubble padding, the name header, button/status/card
  rows, code-block headers and borders, the sidebar and the right column. Soft-wrapped
  lines join back into one line, the selection is clipped to the chat pane, and only the
  selected message text is highlighted.

## [1.29.0] - 2026-10-30

### Added
- Grok Bot cards in the chat: you can now see and act on the interactive prompts a bot
  sends, as in the desktop app. Each card's bubble shows its state (pending, answered,
  dismissed, expired), its details and a button; click it, press F11 (keymap `card.open`,
  vim normal `c`) or run `/card` (newest pending card) to open it.
  - Question widgets: single choice (1-9 or ↑↓ Enter), multi-select (Space toggles),
    type-your-own answers (Tab) and multi-step questions; `x` dismisses.
  - Auto-review approvals and local tool permissions: `a` approve / allow once, `d` deny.
  - Secret requests: masked input, sent only through `submitSecret`; never logged,
    echoed, stored or shown, and cleared after sending or closing.
  - Forms (`user-form`): Tab between fields, password/OTP/secret fields masked, required
    fields checked; Ctrl-S submits, Ctrl-D dismisses.
  - Message drafts (email, Slack, meeting chat): `s` send, `e` edit the body, `x` discard.
  - Other cards (purchase/virtual-card approvals, connector grants, cookie, publish and
    similar confirmations) are shown read-only with a note to act in the desktop app.
- Theme parts `card_frame`, `card_pending`, `card_done` and `card_button`.

## [1.28.0] - 2026-10-30

### Added
- Right column header: the open chat's bot name (bold) and description, wrapped, cut after
  3 rows with a clickable **▾ more** / **▴ less**. It sits above the attachments panel and
  follows the bot-list refresh. Theme parts `bot_header_name` and `bot_header_desc`.

### Changed
- The sidebar bot list shows names only. A name with the description typed into it after
  " · " (e.g. "Lippy · Very ratatui-lipgloss focused rust developer") is split: the sidebar
  shows "Lippy" and the rest becomes the description in the header.
- Bot replies now look like your prompt bubbles: the bot's name is the bubble header on its
  own row, and the reply sits directly under it at the same edge and 2/3 width, left-aligned
  as yours are right-aligned. (Before, the name was a `Name: ` prefix with the reply indented
  after it.) Inline media, images and download chips line up under the name too.
- The right column (`ui.attachments_panel`) is now shown by default, since the header is
  useful in every chat. `hide`, F9, `/attachments` and the «/» buttons still collapse the
  whole column to the strip, and it still hides when the chat body is under 70 columns.
  A saved `hide` is kept.

## [1.27.0] - 2026-10-30

### Added
- Refresh the bot list without restarting: `/refresh`, F12 (keymap action
  `agents.refresh`, also in the vim keymap) and a ↻ button in the sidebar header and in the
  collapsed sidebar strip.
- Automatic refresh every `agents.refresh_interval_secs` seconds (pref, default 60, `0` = off),
  and after a re-login (reconnect). The gateway has no agent create/delete/rename events, so
  this polls `listAgents` (plus team bots), the same call as at startup.
- The new list is merged in place: the open chat stays selected, with no transcript reload,
  scroll jump or media stop, and the sidebar cursor stays on the same bot. New bots are
  added, deleted bots removed (if the open one is gone you get a notice and the next bot is
  shown), and names, descriptions and hidden/group/team flags are updated. A status notice
  says what changed ("Bot list refreshed: 1 new (ratatoskn)"); background refreshes stay
  quiet when nothing changed, and their errors are not reported while a list is showing.

### Notes
- `/reload` was not used as an alias because it already reloads the open chat's transcript.
- Bot avatars are not shown in the TUI, so there is nothing to update for them.

## [1.26.0] - 2026-10-29

### Added
- Install from a local checkout without `cargo install`, crates.io installs or downloaded
  binaries: `make`, `make install`, `make uninstall` (PREFIX, default `~/.local` or
  `/usr/local` as root; DESTDIR; `FFMPEG=1` for the -ffmpeg variant; `OFFLINE=1`;
  `make vendor` for a fully offline build), backed by `install-local.sh`, plus
  `install-local.ps1` for Windows. They install the binary, README/CHANGELOG/THIRD_PARTY,
  the licence and the example themes, keymap and notify rules (and a man page or shell
  completions when those exist). Building still needs a Rust toolchain.

### Changed
- `install.sh` now runs `install-local.sh` (default prefix `~/.local`, was `~/.cargo/bin`
  via `cargo install`).
- The AUR source package installs with `make install NOBUILD=1 PREFIX=/usr DESTDIR=$pkgdir`,
  so it uses the same layout. The .deb and .rpm keep their cargo-deb / cargo-generate-rpm
  asset lists (those tools package an already-built binary; same paths).

## [1.25.0] - 2026-10-29

### Added
- kitty keyboard protocol: where the terminal supports it (kitty, Ghostty, foot, WezTerm,
  recent Alacritty and iTerm2), grokbot-tui now pushes disambiguate, report event types
  and report alternate keys. That makes Alt, Ctrl-Enter and Shift-Enter reliable, and a
  plain Esc acts at once with no ESC-prefix delay. Other terminals keep the ESC-prefix Alt
  handling (`keys.esc_alt_timeout_ms`).
  - Pref `keys.kitty_protocol = auto|on|off`, default `auto`, which uses the protocol
    when the terminal answers the query.
  - `/kitty [auto|on|off]` (alias `/keyboard`) shows the state or changes it live.
  - Held keys repeat (repeat events count as presses; releases are ignored). With
    alternate keys a shifted symbol (e.g. `?` + Shift) matches the same bindings as before.
  - The flags are popped on exit, on a panic, and when the process is suspended (SIGTSTP),
    and pushed again on resume, which also pauses media and redraws.

## [1.24.0] - 2026-10-29

### Changed
- **Licence: grokbot-tui is now GPL-3.0-or-later** (was BSD-3-Clause). Releases 1.23.0 and
  earlier stay available under BSD-3-Clause. The `grokbot_api` crate is unchanged.
  LICENSE, Cargo metadata, the .deb copyright, the AUR/winget manifests (from Cargo.toml)
  and the README are updated, and `THIRD_PARTY.md` lists every bundled component.

### Added
- Inline video, animated GIFs and music in chat bubbles, for received links and attachments:
  - Scaled to the bubble like images (row cap `images.inline_max_rows`), with the poster
    frame shown while stopped.
  - A one-row bar under each: [▶]/[⏸] toggles playback, clicking the bar seeks, and the
    label shows elapsed/total time. GIFs show their frame position instead.
  - Clicking the picture opens a full-size player modal (Space, ←/→ ±5 s, Home, Esc/q, and
    the clickable bar).
  - Only one video or track plays at a time, and it pauses when scrolled out of view. GIFs
    loop on their own (`media.gif_autoplay`).
  - Decoders: GIF (image crate), H.264 MP4 through OpenH264 compiled from source into
    every build, and MP3/AAC/M4A/FLAC/Vorbis/WAV through rodio/Symphonia. Sound plays
    in sync with MP4 video. Other codecs use FFmpeg in the new `-ffmpeg` builds, else a
    system `ffmpeg` on PATH if there is one.
  - Prefs `media.inline`, `media.fps`, `media.decoder` (auto|builtin|ffmpeg|system) and
    `media.gif_autoplay`. Commands `/media pause|stop|decoders` (alias `/play`).
  - The attachments panel's [view] opens the player for video, GIFs and audio.
- Two builds per target: standard, and `-ffmpeg` (`grokbot-tui-ffmpeg-<target>` archives,
  FFmpeg linked statically). New `grokbot-tui-setup.sh` / `grokbot-tui-setup.ps1` wrapper
  installers ask which one to install.
- `/license` (aliases `/licence`, `/third-party`): the licence notice, the decoders in this
  build, and THIRD_PARTY.md, including the H.264 patent caveat. `/license full` shows the
  GPL text.
- Theme part `media_bar`.

### Notes
- On Linux, audio uses ALSA (libasound2). The .deb/.rpm/AUR packages depend on it.

## [1.23.0] - 2026-10-29

### Added
- Attachments panel: a collapsible bar on the right of the chat listing every attachment
  in it, sent and received (images, files and file links in replies), with its type icon,
  name, size, sender and time, and [view] and [⤓] buttons.
  - [view] opens images in the image viewer, text/HTML/Markdown/JSON in the built-in
    browser, and anything else in the system app (asks first over SSH).
  - [⤓] opens the download modal. Clicking a row scrolls the chat to that message.
  - A search box and type filters (All, Img, Vid, Aud, Doc); keyboard: j/k, Enter (jump),
    v (view), d (download), / (filter), t (type), Esc (back to the input).
  - Collapses to a 1-column strip with the attachment count; toggle with the «/» buttons,
    View ▸ attachments, F9, `/attachments show|hide|toggle` (alias `/files`) or keymap
    `view.toggle_attachments`. Persisted as `ui.attachments_panel` (default hide).
  - Hidden when the chat body is narrower than 70 columns; works with the collapsed or
    expanded sidebar and the compact header.
- Theme parts `attach_panel_frame`, `attach_panel_filter`, `attach_panel_count`.

### Fixed
- The built-in browser's URL bar, edit field and copy no longer expose presigned URL
  signatures (host only).

## [1.22.0] - 2026-10-29

### Added
- Clickable menus: click a menu title to open it (again to close), hover highlights items,
  click runs one, click outside closes. `/menu [name]` opens the menu bar without Alt
  (as do F10 and Alt-M). Alt+<underlined letter> opens that menu (Alt-C Chat, Alt-V View, …).
- **View** menu, `/header [compact|full|toggle]` (F7, keymap `view.toggle_header`, ▾/▴ button):
  a one-row header with the bot, account, login status, SSH and 🔕. Pref `ui.header`.
- `/sidebar [collapse|expand|toggle]` (F8, keymap `view.toggle_sidebar`, «/» buttons):
  collapses the bot list to a 1-column strip (● active, ◆ keyboard cursor); click it to
  expand. Pref `ui.sidebar_collapsed`.
- Click an empty part of a message bubble to open the reaction picker for it (pref
  `message.click_to_react`, default on). Press and release must be on the same cell, so
  scrolling and drag selection are unchanged; links, images, [☺+] and [⤓] keep their actions.
- `/status [extra text]` (menu **Chat**): sends "Where are we now? Give me a brief status of
  your current tasks." plus any extra text to the current bot as a normal prompt (chat and
  history). The text is pref `commands.status_prompt`. Tab suggests a few add-ons.
- Inline images: received images and your image attachments are drawn in the bubble with
  ratatui-image (kitty / sixel / iTerm2, halfblocks fallback), scaled to the bubble width
  with the aspect kept and at most `images.inline_max_rows` rows (default 12). `images.inline =
  on|off` (default on). Loaded lazily (the `[🖼 …]` placeholder stays until ready, and while
  the image is only partly on screen); resized protocols are cached per size; kitty/sixel
  graphics are cleared when what's shown changes, so scrolling leaves no artifacts. Click an
  image for the full-size viewer.
- Pref `keys.esc_alt_timeout_ms` (default 50, 0 disables).
- Theme parts `collapse_button`, `sidebar_strip` in all shipped themes.

### Fixed
- Long completion lists were cut off at 8 rows with the selection running off the end. The
  shared completion popup now sizes to its items up to the room above (or below) the prompt,
  scrolls to keep the selection visible, shows a scrollbar and `n/total`, and takes
  PgUp/PgDn and the mouse wheel.
- Paths with spaces in completion: the token under the cursor is found shell-style (`"…"`,
  `'…'` and `\ ` escapes), so completing inside `My\ Dir/` or `"My Dir/` keeps working.
  Completed names with spaces are inserted backslash-escaped (always); `/attach` parses the
  same forms. The 1.16.1 token-span replacement is unchanged.
- Alt shortcuts over SSH (reported from Windows Terminal). Terminals send Alt+x as ESC x; over
  SSH the two bytes can arrive in separate reads, and crossterm then reports a lone Esc followed
  by x. Only ESC+Enter was folded back before; now any letter, Enter, arrow or Backspace within
  the window becomes Alt+key, and Esc Esc gives one Esc. Also, Alt+<menu letter> was never bound
  (only Alt-M and F10 opened the menu), so Alt-C did nothing even locally.

## [1.21.0] - 2026-10-29

### Added
- Presence muting: with `notifications.claim_presence = true` (per account, default off;
  `/mute [on|off|toggle]`, alias `/presence`, or **Notify → mute**) grokbot-tui reports
  `ReportGrokBotDesktopActivity {installId, active: true}` while the terminal is focused and
  you've used it within `notifications.idle_minutes` (default 10), so the server can hold
  Grok Bot pushes to your other devices, as the desktop app does. Heartbeats at half the
  returned lease (150 s live, so every 75 s). Released (`active: false`) on focus loss, idle,
  logout, account switch and quit (waits up to 2 s). If the process is suspended or the
  machine sleeps, heartbeats stop and the lease lapses on its own.
- A per-account install id (`notifications.install_id`, a UUID generated once).
- 🔕 in the header while the claim is held; `/whoami` shows the claim, lease and time left.
- Theme part `presence_indicator` in all shipped themes.
- Terminal focus reporting is turned on (FocusGained/FocusLost).

### Verified
- One live round trip on the default account: `active: true` → lease 150000 ms,
  `active: false` → 0. Whether pushes are actually held depends on the server-side
  `grok_bot_hold_push_until_desktop_inactive` gate.

## [1.20.0] - 2026-10-29

### Added
- Built-in text browser (`/browse [url]`, alias `/web`, vim NORMAL `b`, menu **Web**): an
  in-process modal with no JavaScript and no cookies (reqwest, 10 redirects, 20 s timeout,
  5 MiB cap, `grokbot-tui/<version> (text browser)` User-Agent). HTML renders through
  html2text's rich output (headings, bold/italic, code, lists, quotes) with numbered `[n]`
  links. Images become `[🖼 alt]` placeholders that open the image viewer. Text, markdown and
  JSON render as text; other types offer download or the system opener.
- Browser keys: editable URL bar (`e`, Ctrl-L, `:open <url>`), j/k, arrows, PgUp/PgDn, g/G,
  Ctrl-d/u and wheel scrolling, Tab/Shift-Tab link cycling, Enter/click to follow, number then
  Enter to jump, H/L and Alt-←/→ history, `r` reload, `/` `n` `N` search, `o` system browser,
  `y` copy URL, `d` download, Esc/`q` close. Per-session history; `/browse` Tab completes
  recent URLs.
- Pref `links.open_with = builtin|system` (global, default **builtin**), `/links
  builtin|system|toggle` and the Web menu. Ctrl/Alt-click and vim NORMAL `x`
  (`message.open_other`) use the other opener. Chat link clicks and `/open` follow the pref.
- SSH detection (`SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`, parent `sshd` on Linux;
  `GROKBOT_TUI_FORCE_SSH=0|1`). In SSH the header shows **SSH**, links use the built-in browser
  (pref `links.ssh_behavior = builtin|respect_pref`, default builtin) and anything that would
  open an app on the remote machine warns and needs repeating within 5 s.
- Keymap actions `browser.open` (`b`) and `message.open_other` (`x`).
- Theme parts `browser_frame`, `browser_url`, `browser_link_number`, `browser_heading`,
  `browser_status`, `ssh_indicator` in all shipped themes (with built-in fallbacks).

### Changed
- The image viewer's external open goes through the same opener (SSH confirmation; presigned
  URLs show the host only).

## [1.19.0] - 2026-10-29

### Added
- `/attach <path…>` (alias `/file`): attach up to 6 files of any type to the next prompt. Tab
  path completion uses the shared completer (cycling replaces the path token). Quoted paths
  and `\ ` escapes work. Pasting a file path attaches it instead of typing it.
- Sizes are checked before uploading: images 25 MiB, video 1 GiB, other files 250 MiB.
  Empty files, duplicates and a seventh file are refused.
- Uploads start as soon as a file is staged (grokbot_api 1.2.0 `upload_attachment`: begin,
  presigned PUT, complete). Chips above the input show the icon, name, size, % progress and
  ✓/✗. Clicking a chip previews a staged image, ✕ removes it and aborts the upload, and
  `/attach remove <n>`, `/attach clear` and `/detach [n|all]` do the same from the keyboard.
- Sending waits for pending uploads, then sends `attachmentPaths`/`attachmentNames`. A failed
  upload blocks the send and keeps the prompt in the input.
- Clear messages for refusals (too large, storage full, unsupported type…), desktop-only bots
  (`USE_BOX`) and logins without a Cursor session.
- Sent prompts show `[icon name size ⤓]` chips. Clicking the name resolves the attachment
  (`ResolveGrokBotAttachmentReads`) and opens images in the image viewer. ⤓ opens the download
  modal with the real file name. Resolved images also join the viewer's n/N list and `/download`.
  Presigned links are never shown: the modal and notices name only the host.
- Message attachments are stored in `context.db` (schema v11, `messages.attachments`).
- Theme parts `attach_chip` and `attach_chip_error` in every shipped theme.

### Notes
- Verified live once against the real service: one 64×64 PNG to one bot. The upload came back
  `SINGLE_PUT` to S3 with `Content-Length` and `x-amz-checksum-sha256` headers. Complete returned
  a path-like alias, `sendPrompt` accepted `attachmentPaths`, and the resolved read returned the
  same bytes.
- The gateway transcript does not include attachments, so the chips only show for prompts sent
  from this install.
- Built against grokbot_api 1.2.0.

## [1.18.0] - 2026-10-28

### Added
- Download buttons: bot replies with images or file links get a row of `[⤓] name` buttons
  (theme part `download_button`). Clicking one, `/download [n|list]` (alias `/save`), or vim
  `d` (keymap action `message.download`) opens the download modal.
- Download modal: a path field prefilled with `~/Downloads/<name>` (falling back to `~`), Tab
  and Shift-Tab path completion (fuzzy; Esc restores what you typed), and a folder browser
  (Down to enter; arrows/hjkl; Enter/l opens a folder; h/← parent; click works too).
  Overwrites and missing parent folders are confirmed (y/n). `~` is expanded.
- Remote files (http/https) stream in the background to `<file>.part` and are renamed when
  complete, so a failed download leaves nothing behind. Local files are copied. Success or
  failure shows as a notification.
- Pref `downloads.max_mb` (global, 1–4096, default 25) caps download size.
- Theme parts `download_button`, `download_path`, `download_dir` in every shipped theme.

### Not added: attachments
- `/attach` was not implemented because no upload mechanism was found. grokbot_api 1.1.0
  `sendPrompt` takes only `agentId`, `prompt`, `clientNonce`, `replyToId` and `sessionId`.
  None of the crate's methods or guides, nor the reference gateway client, mentions an
  upload, attachment, file, media or blob call.

## [1.17.0] - 2026-10-28

### Added
- **`/react` / `/unreact` Tab completion** for the emoji: with no argument it lists the emoji
  already on the target message (marked `· on message`), then recently used reactions
  (`· recent`), then 👍 ❤️ 😂 🎉 🔥 👀 ✅ 🙏 😮 😢, then every emoji by category. Shortcodes
  match with or without the colon (fuzzy); the popup shows `:code: glyph` and inserts the
  glyph. `/unreact` offers only what's on the message. The `[n]` index completes too.
- **React button and emoji picker:** every chat message gets a `[☺+]` button (after its
  reaction pills, or in the blank line below it, so layout, scroll math and the jump-to-new
  badge are unchanged). Clicking it, bare `/react`, or vim NORMAL `R` (keymap action
  `message.react_picker`) opens a searchable emoji grid for that message: fuzzy search by
  shortcodes and CLDR name; on-message and recent emoji first, then common, then by category;
  arrows / hjkl / mouse wheel move (vim keys start in list mode, `/` searches); Enter or a click
  toggles the reaction and closes; Esc, `q` or a click outside closes. The footer shows the
  selection's shortcode, name and category.
- Theme part **`react_button`** (Chat). The picker reuses `help_frame`, `help_title`,
  `selected` and `reaction_pill` (on-message emoji).

## [1.16.1] - 2026-10-28

### Fixed
- **Tab completion cycling appended instead of replacing** (e.g. `/` then Tab Tab Tab gave
  `/mousemousemouse`, notably in vim `:` command mode). The popup now remembers the token's
  span and the text you typed when the cycle starts, and every Tab, Shift-Tab, ↑ or ↓
  swaps only that span. One step past the last candidate restores what you typed (so
  Shift-Tab from the first candidate gets it back) and the cycle wraps. Typing any other key
  ends the cycle; the next Tab starts from what's there. Enter accepts.
- Tab with the popup open now cycles (it used to close the popup and re-open it on the same
  first candidate). Shared code, so this covers command names, arguments (`/switch`,
  `/theme`, `/history`, `/keymap`, `/react` …) and `:shortcode:` emoji.
- `:shortcode` + Tab in a plain message did nothing in the app (only `/` lines reached the
  completer); it now opens the emoji popup.
- Regression tests: bare `/` cycled 12 times is always exactly one command, prefix cycling with
  wrap-around and Shift-Tab back to the original, argument completion, emoji cycling, and vim
  command-mode cycling.

## [1.16.0] - 2026-10-28

### Added
- **Image viewer.** Click an image placeholder in the chat, or use `/view [n]` (alias `/image`,
  vim NORMAL `O`, keymap action `image.view`, nearest on-screen image first), to open a modal
  that shows the image as large as the terminal allows with the aspect ratio kept
  (ratatui-image 11: kitty, sixel or iTerm2 detected at start-up, halfblocks fallback; the
  Windows console and Alacritty get halfblocks). The title shows the alt text or file name,
  pixel dimensions, file size, `n/N` and the protocol.
- Viewer keys: `Esc` / `q` or a click outside close; `o` / `Enter` open in the system viewer;
  `←`/`→`, `n`/`p`, `h`/`l` step to the previous / next image in the chat.
- Images load on demand (local paths, `~/`, `file://`, or http/https downloads up to 25 MB)
  in the background; a missing or undecodable image shows an error with the `o` fallback.
- Closing the viewer, stepping, and terminal resizes clear the screen before the next draw
  (plus a kitty delete-all) so no kitty or sixel image ghosts are left.
- `GROKBOT_TUI_IMAGES=halfblocks` skips the terminal graphics query.
- Theme part **`image_viewer_info`** (Modals). The viewer frame and title reuse `help_frame`
  and `help_title`.

### Changed
- Image placeholders use no-break spaces, so they wrap as one unit and stay one click target;
  the `<path>` after them is clickable too.

## [1.15.0] - 2026-10-28

### Added
- **Clickable links.** Markdown link labels, their `<url>` and bare `http(s)://` URLs in the
  chat open with the system opener on a mouse click (xdg-open / open / the Windows URL
  handler, no shell). Bare URLs are now styled with the `md_link` theme part.
- **OSC 8 hyperlinks:** after each frame the visible links are re-printed from the frame
  buffer (same characters and colors) inside OSC 8 sequences, so Ghostty, kitty, WezTerm,
  Windows Terminal, foot, iTerm2 and Alacritty 0.13+ make them natively clickable. ratatui's
  buffer and width math never see the escapes, and the cursor is saved and restored. Opt out
  with `GROKBOT_TUI_NO_HYPERLINKS=1`. Control characters in URLs are stripped.
- Keyboard path: `/open list` numbers every image and link in the replies, `/open <n>` opens
  one, and vim NORMAL `o` opens the newest.

## [1.14.0] - 2026-10-28

### Added
- **Images in replies:** `![alt](url)` renders as a themed placeholder,
  `[🖼 alt · cat.png · 12.0 KB]` (size for local files), followed by the URL. One row like
  any other line, so scroll math and the jump-to-new badge are unchanged.
- **`/open [n|list]`** (alias `/img`, vim NORMAL `o`, keymap action `message.open`): open the
  newest image in the bot's replies, then links, with the system handler (`xdg-open` on
  Linux, `open` on macOS, `rundll32 url.dll,FileProtocolHandler` on Windows; spawned without a
  shell). Bare image URLs and paths count too. `list` numbers them; missing local files are
  reported instead of launched.
- Theme part **`image_placeholder`** (Chat).

### Not supported (API)
- **Image attachments on prompts:** grokbot_api 1.1 `SendPrompt` has only `agent_id`,
  `prompt`, `client_nonce` and `reply_to_id`, and the client has no upload method, so there
  is no way to send an image. No `/attach` was added rather than fake it.
- Inline terminal graphics (kitty / sixel / iTerm2 via ratatui-image) are not drawn yet; the
  placeholder plus `/open` is the fallback for every terminal.

## [1.13.0] - 2026-10-28

### Added
- **Markdown in bot replies** (pulldown-cmark, CommonMark + GFM tables, strikethrough and
  task lists): headings, bold, italic, strikethrough, inline code, fenced code blocks (shaded
  block with a language label), bullet and numbered lists with nesting and hanging indents,
  `☑`/`☐` task items, `│` blockquotes, links shown as `label <url>` (autolinks once),
  box-drawn tables that shrink and truncate with `…` to fit, `---` rules, and image syntax
  shown as `🖼 alt <url>`. Before this, replies were shown as raw text with the markup.
- Replies are rendered already wrapped to the chat width (every line one row; wide emoji and
  long words are split between graphemes), so scroll math and exact-bottom follow are
  unchanged. Your messages and notices stay plain.
- Theme parts (Chat): **`md_heading`**, **`md_code`**, **`md_code_block`**, **`md_quote`**,
  **`md_link`**, **`md_list_marker`**, **`md_rule`** (rules and link URLs),
  **`md_table_border`**. Shipped themes set them; older files fall back to built-in defaults.
  Bold/italic/strikethrough are modifiers on the message style.

### Notes
- No syntax highlighting in code blocks yet (syntect would add a large dependency and its own
  theme set); blocks use the `md_code_block` style instead.

## [1.12.0] - 2026-10-28

### Added
- **Emoji shortcodes:** type `:` plus part of a name and press Tab for the same completion
  popup slash commands use (`:thumbsu` → 👍). Complete `:name:` shortcodes in a message are
  turned into emoji when sent (unknown names stay as typed; `12:30` and URLs are untouched).
- **Reactions:** `/react <emoji|:code:> [n]` (alias `/unreact`) toggles a reaction on the
  newest bot message (`n` = 2 for the one before). Reactions show as pills under the message
  (right-aligned under yours). Vim NORMAL `r` (keymap action `message.react`) opens `:react :`.
  **Local only:** grokbot_api 1.1 has no reaction endpoint and transcript entries carry no
  reaction data, so reactions are stored in `context.db` (schema v10, per account and bot)
  and never sent to the gateway.
- Theme part **`reaction_pill`** (Chat). Shipped themes set it; older theme files fall back to
  text on the header color.

### Fixed
- Emoji and other wide or multi-codepoint characters in the prompt: cursor moves, Backspace and
  Delete step over a whole grapheme (ZWJ families, flags, skin tones, VS16), and wrapping and
  the cursor column use display width, so the cursor no longer drifts after an emoji.
- Chat wrapping never splits a grapheme and measures emoji sequences as one wide cell pair.

## [1.11.1] - 2026-10-28

### Fixed
- **Ctrl-Enter** is now listed everywhere newline keys are shown (input placeholder, `/help
  keys`, README). It already inserted a newline; on Windows it is the one modifier-Enter that
  conhost and Windows Terminal always report, so the hint now says so. Linux terminals that
  send Ctrl-Enter as Ctrl-J also get a newline; ones that send plain Enter still send.
- Tests pin the Ctrl-Enter variants (Windows `Enter+CONTROL`, kitty/modifyOtherKeys CSI-u,
  raw LF as Ctrl-J) and that plain Enter still submits.

## [1.11.0] - 2026-10-23

### Added
- **`/history [query]`** (alias `/prompts`): a scrollable browser of every prompt you've
  sent, newest first, with its age and the bot it went to. Type to fuzzy-filter;
  ↑/↓, PgUp/PgDn, Home/End and the mouse wheel move; Enter (or clicking the selected row)
  puts the prompt in the input without sending; Esc closes and keeps your draft. With
  `ui.keys=vim` it opens in list mode (`j`/`k`, `g`/`G`, Ctrl-d/Ctrl-u, `/` to filter,
  `q` to close). Tab completes `clear` and words from recent prompts. Also in the Chat menu.
- **`/history clear`**: forget the active account's prompt history after a confirmation
  (Cancel is the default).
- **Pref `history.max_entries`** (global, 100–1000000, default 10000): how many prompts are
  kept per account. Replaces the fixed 500 cap.
- Prompt history remembers the bot each prompt went to (schema v9 adds a nullable `bot`
  column; existing history is kept).

### Changed
- The transcript reload that used to be `/history` is now **`/reload`** (alias
  `/transcript`).
- Up/Down and Ctrl-R re-read history from `context.db` when you start browsing, so prompts
  sent from another grokbot-tui on the same data dir show up without a restart.
- A failure to load prompt history is now reported instead of silently showing an empty
  history.

### Security
- `/login …` lines are stored in history without their arguments, so a token pasted on
  the same line never lands in `context.db`.

## [1.10.2] - 2026-10-23

### Fixed
- Apply rustfmt so the CI format check passes (no behavior changes). First release
  containing the 1.8.0–1.10.1 changes.

## [1.10.1] - 2026-10-22

### Fixed
- **Team bots (e.g. Chomp) showed in the sidebar but send/history did nothing.** Shared
  agents need the viewer `sessionId` from `ListGrokBotTeamAgents` on every
  `sendPrompt` / `getAgentTranscriptTail` / `promptAcceptanceStatus`. The TUI now keeps
  `viewer_session_id` on each bot (persisted), re-binds it before those calls, and errors
  clearly when a team bot has no session. Soft-fails of the team list are no longer
  silent. Send failures that are not gateway-rejected are shown in the chat. Desktop
  stored logins re-read the app session so a Cursor token is available again for the
  team list after restart.

## [1.10.0] - 2026-10-22

### Added
- **Notifications** (from acp-rs-scraper `src/notify.rs` + rules gist @ `a60c82a`):
  toast stack with TTL, do-not-disturb, per-kind switches (`error`/`warning`/`info`/
  `message`), and `~/.grokbot-tui/notify-rules.toml` (toast/mute/drop). `/notify`.
  Toasts never move the chat scroll (1.9.1 rule). Theme parts `toast`, `toast_warn`,
  `toast_error`.
- **Menu bar** (from `src/tui/widgets/menu.rs` gist): Chat / Account / Theme / Notify /
  Help mapping onto slash commands. F10 / Alt+M, arrows, Enter, click. Theme parts
  `menu_bar`, `menu_active`.
- **Hover tooltips** (from `src/tui/tooltip.rs` gist): 400 ms delay, theme part
  `tooltip`, deep-link topic. Motion stays coalesced — redraw only when the tip
  changes (no all-motion flood).
- **`docs/notifications-menu-tooltip-plan.md`**: source paths/commits and deviations.

### Fixed
- **macOS Terminal.app: Shift/Ctrl/Alt-Enter submitted instead of inserting a newline.**
  Push keyboard enhancement when `supports_keyboard_enhancement()` is true (pop on
  exit/panic); treat ESC+CR as Alt-Enter; Ctrl-J and Ctrl-Enter insert a newline;
  a trailing `\` then Enter becomes a newline; bracketed paste keeps multi-line
  pastes as text. One-time hint when enhancement is unsupported and
  `TERM_PROGRAM=Apple_Terminal`. Remappable as `input.newline` in `keymap.toml`
  (vim insert). See `/help keys`.

## [1.9.1] - 2026-10-22

### Fixed
- **Chat jumps to the bottom while reading.** Notices (`info`/`error`) and feed
  reloads forced `scroll = 0`, and distance-from-bottom scrolling kept a constant
  distance from the newest line — so any new content moved the text under the
  eyes. Now a follow/stick-to-bottom flag auto-scrolls only when the viewport is
  exactly at the bottom (distance-from-bottom scroll == 0); when scrolled up, the absolute viewport offset is
  pinned so appends and rewraps leave the reading position alone. Unread items
  show an unobtrusive `↓ N new` badge (theme part `unread_badge`, starter falls
  back to the warn badge). Click the badge, End (while scrolled up), vim `G`, or
  scroll to the bottom to follow again. Sending a message, switching bots, or
  switching accounts jumps to the bottom and re-enables follow.

## [1.9.0] - 2026-10-21

### Added
- **Opt-in vim-like modes** (`ui.keys = default|vim`, `/keymap vim|default`). Modes:
  INSERT / NORMAL / COMMAND / VISUAL with a styled indicator on the input title
  (theme parts `mode_normal`, `mode_insert`, `mode_command`, `mode_visual` with
  starter fallbacks so existing themes keep working).
- **TOML keymap** at `~/.grokbot-tui/keymap.toml` (shipped default in
  `keymap/default.toml`). Validation, conflict detection, graceful fallback + notice.
  `/keymap` (`/keys`) lists or reloads. `:` maps onto slash commands and reuses Tab
  fuzzy completion from 1.8.0. Visual mode integrates with in-app selection and the
  Ctrl-C resolver.
- **`docs/vim-mode-plan.md`** citing Lenin's `architecture/PLAN-MODES-KEYMAP.md` at
  commit `3c84eb2` and documenting deviations.

## [1.8.0] - 2026-10-21

### Added
- **Shipped themes** in `themes/`: Dracula (+ Alucard light), Nord, Gruvbox, Solarized,
  Catppuccin (Mocha/Latte), Tokyo Night/Day, One Dark/Light, Rosé Pine/Dawn. First-run and
  `/theme init all` fetch every shipped theme; `default` stays selected on first run.
- **Prompt history** (Up/Down from the first/last input line; per-account in `context.db`,
  deduped, capped at 500). Draft is restored when browsing back. Slash commands are stored.
- **Ctrl-R** fuzzy history search (Ctrl-S reverse; Esc / Ctrl-G restore the draft). Shares
  the fuzzy matcher with Tab completion. Ctrl-R in the theme editor still reverts all.
- **`/switch [alias]`** — bare opens the account picker; an argument matches by exact name,
  unique prefix, or unique fuzzy hit.
- **Tab completion** for slash commands and arguments (Shift-Tab cycles; Esc restores).
  Theme editor / picker Tab behaviour is unchanged.
- **In-app chat selection**: drag to select (button-event drag kept; moves still coalesce),
  copy on mouse-up via OSC 52, Esc clears, Ctrl-A selects the whole feed when the chat has
  focus. New theme part `selection` (starter falls back to the selected style) so existing
  themes keep working. F6 / `/mouse` persists as `ui.mouse_capture`.
- **`install.sh`** and README troubleshooting for `command not found` / zsh `rehash`.

### Changed
- Shared `fuzzy` matcher ranks Tab completion, Ctrl-R, and `/switch`.
- Mouse coalesce keeps the last drag (for selection) instead of dropping every drag.

## [1.7.2] - 2026-10-21

### Fixed

- **Clicks responded slowly**, mostly while the mouse was moving. Mouse capture reports every
  pointer move (xterm mode 1003). The event loop handled one event per pass and redrew the
  whole screen after each, so a click waited behind one full redraw per queued move.
  - With a 400-message chat at 200×60, a redraw took about 17 ms in a release build (about
    207 ms in a debug build). A burst of a few hundred moves therefore held a click back for
    seconds.
  - Every queued event is now handled before the next draw. Only the last pointer move of a
    batch is kept (pick-list hover), and drags are dropped.
  - Pointer moves outside pick lists, button releases and key releases no longer trigger a
    redraw.
  - A click now takes effect on the next frame.
- **Ctrl-Shift-C quit the program.** The quit handler fired on any key with Ctrl held whose
  letter was `c` or `d`. Terminals deliver Ctrl-Shift-C as Ctrl-C (or as Ctrl+Shift+C), so the
  copy shortcut quit, as did Ctrl-D, even in the theme editor with unsaved changes. Now a
  single resolver (`keyctx`) decides from focus, the pane under the mouse, selection and
  in-flight state, in this order:
  1. A selection in the input is copied (chat selection lands in 1.8.0).
  2. A pending reply is cancelled when the input or chat has focus.
  3. Text in the focused input: Ctrl-C clears it (Ctrl-Z restores it); Ctrl-Shift-C copies it.
  4. An open overlay (picker, theme editor, help, shell) closes like Esc.
  5. Over the chat with nothing selected: a hint, no quit.
  6. Otherwise "Press Ctrl-C again to quit"; a second press within 1.5 s with nothing done
     in between quits.
  - Ctrl-Shift-C never quits, closes or cancels. Ctrl-Q and `/quit` quit at once. Ctrl-D
    deletes forward (or, in the theme editor, duplicates); on an empty input it counts like
    Ctrl-C. Shift+arrows select in the input; Ctrl-A selects all of it.
- **Redraws no longer re-wrap the whole transcript.** The chat feed's lines and wrapped heights
  are cached until the feed, width, bot name or entry styles change, and only the lines in view
  are rendered. A redraw of a 400-message chat dropped from about 17 ms to about 0.5 ms
  (release build).

## [1.7.1] - 2026-10-18

Completes the 1.7.0 sync with acp-rs-scraper (its 2.14.0). These changes missed the 1.7.0 tag,
which was already published.

### Added

- **`/theme init [name|all]`** downloads that theme's file from the release's `themes/` folder.
  `all` downloads every shipped theme, which for now is just `default`. Existing files are kept
  and nothing is downloaded for them; several downloads run one after another.
- **Tabbed color picker in the theme editor (`p`):**
  - tabs: 16 colors, the 216-color cube, 24 grays, and `@ palette`
  - `@ palette` offers the palette colors as references for parts, plus `reset`
  - Tab and Shift-Tab switch tabs, and the tabs can be clicked
  - the picker opens on the tab and cell of the current value
- **`--list-themes` and `--theme` download the default theme first** when the themes folder is
  empty, waiting up to 3 s. A notice on stderr says what happened. There is no download with
  `--mock` for `--theme`, or when downloads are turned off.
- **`GROKBOT_TUI_THEME_URL`** replaces the theme download address: a URL base that
  `<name>.toml` is appended to. The tests use it with a local mock server.

### Changed

- **`/theme init <name>`** no longer saves a renamed copy of the default theme; use the editor's
  `d` (duplicate) for that. If the download fails, it now says why and points to
  `/theme starter <name>`. The starter is still written automatically when downloads are turned
  off.
- **The theme editor's 256-color palette grid** is replaced by the tabbed picker.

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

[Unreleased]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.7.2...HEAD
[1.7.2]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.7.1...v1.7.2
[1.7.1]: https://github.com/WeirdBrains/grokbot-tui/compare/v1.7.0...v1.7.1
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
