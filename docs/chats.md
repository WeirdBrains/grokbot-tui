# Chats and bots

The layout, the bot list, message bubbles, reactions, cards, sync and delivery.

## Layout

```
 ▄▀▀▀▀▀▀▀▀▀▀▄  ▗▛▜▖        ▜▌      ▜▛▜▖     ▟     account  @default ▾  F3                       header
 ▀▀▀▀▀▀▀▀▀▀▀▀  █   ▜▟▜▖▟▀▙ ▐▌▟▘    ▐▙▟▘▟▀▙ ▝█▀    theme  ‹default› ▾  dark  light  system  F4   (logo, title,
 ▀▀▀▀▀▀▀▀▀▀▀▀  ▜▖▜▌▐▌▝▘█ █ ▐▛▙     ▐▌▐▌█ █  █▗    login cursor · box.cursorvm.com               switchers)
 ▀▀▀▀▀▀▀▀▀▀▀▀   ▀▀▘▀▀  ▝▀▘ ▀▘▝▘    ▀▀▀ ▝▀▘  ▝▘    mouse: on (F6)
╭ Bots ──────╮╭ Chat: Helper · Helper is replying ⠹ ──────────────────╮
│ Helper     ││                                               you     │  body: command output
│ Coder      ││                                            ╭──────╮   │  and chat transcript,
│            ││                                            │ hi   │   │  messages on rounded
│            ││  Helper                                    ╰──────╯   │  bubbles (reactions on
│            ││  ╭─────────── 👍 ─╮                                   │  the top-right corner)
│            ││  │ hello!         │                                   │
│            ││  ╰────────────────╯                                   │
╰────────────╯╰───────────────────────────────────────────────────────╯
╭ /command or message ──────────────────────────────────────────────────╮  bottom input bar
│ > _                                                                   │
└───────────────────────────────────────────────────────────────────────┘
```

- **Header:** the Grok Bot logo and title, then the account switcher (`@alias ▾`: click it or press
  `F3` to pick an account), the theme switcher, the login (method and gateway host), the active bot and
  the mouse state. Terminals smaller than 80 × 24 get a one-row header with the same switchers.
- **Theme:** `‹default› ▾` is the theme in use. Click it or press `F5` to pick another one; the list
  ends with *Edit themes…* (also `Shift-F5`). See [Themes](./themes.md#themes).
- **Mode:** `dark`, `light` or `system` picks the theme's variant. Click one in the header, press `F4` to cycle, or run
  `/theme [dark|light|system]`. `system` follows the terminal background (OSC 11 query at startup,
  else `$COLORFGBG`, else dark). Saved in the global pref `ui.theme_mode` (default `system`).
- **Sidebar:** the account's bots (agents flagged hidden-from-sidebar are left out unless `show_hidden_bots` is on). Click one to switch to its chat.
- **Body:** each bot has its own log. Chats are restored from the local database, then kept current by polling.
- **Bottom bar:** `/`-prefixed input runs a command; plain text is sent to the active bot.

Keys: `Enter` submit · `Tab` toggle focus input/sidebar · `↑/↓` (or `j/k`) + `Enter` in sidebar to pick a bot ·
`↑/↓`, `PgUp/PgDn`, mouse wheel scroll body · `Esc` cancel a prompt · `Ctrl-C` / `Ctrl-Shift-C` copy (the selection, else the input; never quit) ·
`Ctrl-D` (also `Ctrl-Q`, `/quit`) quits at once (in a card it dismisses the card) ·
`F1` help · `F2` pick a bot · `F3` pick an account · `F4` cycle dark/light/system · `F5` pick a theme ·
`Shift-F5` theme editor · `F6` mouse capture on/off.

### Markdown in replies

Bot replies render as markdown (CommonMark plus GitHub tables, ~~strikethrough~~ and task
lists): headings, **bold**, *italic*, `inline code`, fenced code blocks (a shaded block with
the language label; no syntax highlighting yet), bullet and numbered lists with nesting,
`☑`/`☐` task items, `│` blockquotes, links as `label <url>`, box-drawn tables (columns shrink
and cells end in `…` when the chat is narrow) and `---` rules. Lines are wrapped before
drawing, so scrolling and the bottom-follow stay exact. Your own messages and notices stay
plain text. Theme parts: `md_heading`, `md_code`, `md_code_block`, `md_quote`, `md_link`,
`md_list_marker`, `md_rule` (also link URLs), `md_table_border`.

### Links

Links in the chat are clickable: markdown link labels, their `<url>`, and bare `http(s)://`
URLs (styled with `md_link`). A mouse click (with mouse capture on) opens the link with the
system opener. Each frame also wraps the links in **OSC 8 hyperlinks**, so Ghostty, kitty,
WezTerm, Windows Terminal, foot, iTerm2 and Alacritty 0.13+ make them natively clickable
(e.g. Ctrl/Cmd-click, or with `F6` mouse off). Set `GROKBOT_TUI_NO_HYPERLINKS=1` to turn
OSC 8 off. Keyboard: `/open list` then `/open <n>`, or vim `o` for the newest.

### Emoji and reactions

Type `:` and part of a name, then **Tab**, to pick an emoji (`:tada` → 🎉), using the same
popup as slash-command completion. Complete `:name:` shortcodes are turned into emoji when you
send. Emoji (ZWJ families, flags, skin tones, `❤️` with VS16) count as one character and two
columns in the input and in chat wrapping. `/react 👍` (or `/react :thumbsup:`, vim `r`)
toggles a reaction pill on the newest bot message (top-right corner of its bubble). Tab completes the emoji (already on the
message, then recently used, then 👍 ❤️ 😂 🎉 🔥 👀 ✅…) and the message index; `/unreact` +
Tab offers what's on the message. Every message has a `[☺+]` button (theme part
`react_button`): click it, use bare `/react`, or vim `R` (`message.react_picker`) for a
searchable emoji grid (by shortcode and name; recent and on-message first, then by category). **Reactions are local**: grokbot_api 1.1
has no reaction endpoint and transcripts carry no reaction data, so they're kept in
`context.db` (per account and bot) and never sent. Theme part: `reaction_pill`.

**Blocks instead of emoji** mean the terminal's font has no emoji glyphs (common over SSH
into a terminal with a text-only font). `/emoji text` (pref `emoji.render = auto|glyph|text`,
default `auto` = glyph) shows reactions, the react button and the picker as `:shortcode:`
text instead; `/emoji glyph` switches back. No emoji images are bundled.

### Newlines in the prompt

Prefer **Ctrl-J** for a newline (works in every terminal). **Ctrl-Enter** inserts a newline
on Windows (conhost and Windows Terminal always report it) and in Linux terminals that report
modifiers (kitty, Ghostty, foot, WezTerm; Alacritty needs a binding such as
`{ key = "Enter", mods = "Control", chars = "\n" }`, which arrives as Ctrl-J). Shift/Alt-Enter
work when the terminal reports modifiers. A trailing `\` then Enter also inserts a newline.
**macOS Terminal.app** sends plain Enter for Shift/Ctrl-Enter; enable “Use Option as Meta
key” for Option-Enter, or use Ctrl-J / `\`+Enter.

## Right column: bot header and attachments

The right column starts with a header for the open chat's bot: its name in bold and its
description, wrapped (long ones are cut after 3 rows; click **▾ more** / **▴ less**). It
follows the bot-list refresh, so renames and new descriptions show at once. The sidebar
shows bot names only. A bot whose name has the description typed into it ("Lippy · Very
ratatui-focused …") shows as "Lippy", with the rest as its description.

Below the header, a collapsible bar on the right of each chat lists its attachments, sent and received, with
type, name, size, sender and time. **[view]** opens images in the viewer, text/HTML/Markdown
in the built-in browser and other files in the system app (with the SSH guard); **[⤓]** opens
the download modal; clicking a row jumps to its message. It has a search box and
All/Img/Vid/Aud/Doc filters, and keys j/k, Enter, v, d, /, t and Esc when focused.
Toggle with F9, the «/» buttons, View ▸ attachments, `/attachments show|hide|toggle`
(alias `/files`) or keymap `view.toggle_attachments`; pref `ui.attachments_panel`
(`show`/`hide`, default `show` since 1.28.0; `hide` = 1-column strip with the count). These
toggle the whole column, header included. Hidden when the chat body is under 70 columns.
Theme parts: `attach_panel_frame`, `attach_panel_filter`, `attach_panel_count`,
`bot_header_name`, `bot_header_desc`.

Bot replies are laid out like your messages: the bot's name on its own row, the reply
directly under it, the same 2/3 bubble width, on the left where yours are on the right.
Since 1.38.0 each message's text sits on a rounded bubble (`╭─╮ │ … │ ╰─╯`), accent-
coloured for yours and muted for the bot's, and its reaction pills ride the bubble's top
border at the top-right corner, like the desktop app. Every panel, dialog, popup and the
input box uses rounded corners too.

## Grok Bot cards: questions, approvals, secrets, forms and drafts (1.29.0)

When a bot asks you something, the chat shows a card with its state (● pending, answered,
dismissed or expired) and a button. Click the button, press **F11** (keymap `card.open`,
vim normal `c`) or run `/card` to open the newest pending one:

| Card | Keys |
|------|------|
| Question (single, multi-select, your own answer, multi-step) | `1`-`9` or `↑↓` `Enter`; `Space` toggles; `Tab` type your own; `x` dismiss |
| Auto-review approval / local tool permission | `a` approve (allow once), `d` deny |
| Secret request | type (masked) and `Enter`; `Ctrl-D` dismiss |
| Form | `Tab` between fields (secret fields masked), `Ctrl-S` submit, `Ctrl-D` dismiss |
| Email / Slack / meeting draft | `s` send, `e` edit the body, `x` discard |

Secrets are sent only through the gateway's `submitSecret` / `submitUserForm` calls; they
are never logged, stored or shown and are wiped after sending or closing. Purchase
approvals, connector grants and similar cards are shown read-only; act on them in the
desktop app. Theme parts: `card_frame`, `card_pending`, `card_done`, `card_button`.

## Chat sync

Bots come from `listAgents`. Chat text comes from `getAgentTranscriptTail` (there is no separate
listChats/getChat API). On bot select (and on `/reload`), grokbot-tui pages that endpoint via
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

## Selecting text

With mouse capture on (the default; F6 / `/mouse` toggles and persists as `ui.mouse_capture`),
drag in the chat to select. The selection is copied on mouse-up. Ctrl-C /
Ctrl-Shift-C also copy a selection. Esc clears it.

Since 1.30.0 a selection copies **only message text**: the selected cells are mapped back
to each message's source (your text, or the bot's markdown source). Bubble padding, the
name header, the [⧉]/[☺+] row, card and status rows, code-block headers, table borders,
the sidebar and the right column are left out, and lines that were only soft-wrapped on
screen join back into one line. The selection is clipped to the chat pane; a message
selected from start to end copies whole.

Copy buttons:

- **[⧉] on every bubble** (yours and the bot's, left of [☺+]) copies that message's raw
  text / markdown source. Also in the click-to-react popup (**[⧉ copy]**, Ctrl-Y, or `y`
  in vim list mode) and keymap `message.copy` (vim normal `Y`; the last message).
- **[⧉] at the top-right of every code block** copies just that block's code: no fences,
  no wrapping, original indentation.

Copying uses OSC 52 (works over SSH; in tmux `set -g set-clipboard on`) and, when running
locally, also the system clipboard (`wl-copy`, `xclip`, `xsel` or `pbcopy` if installed). A
notice says what was copied ("Copied 12 lines."). Theme part: `copy_button`.

Shift-drag or Option-drag usually bypasses mouse capture so the terminal can select
instead (that copies what's on screen, borders included).

## Prompt history and completion

Every prompt and slash command you submit is saved in `context.db` (per account, newest
first, with the bot it went to and when), so history survives restarts. The newest
`history.max_entries` are kept (default 10000; `/prefs set history.max_entries 50000`).
Consecutive duplicates are stored once, and `/login …` lines are saved without their
arguments so a pasted token never lands in history.

- **Up/Down** in the input walk lines, then your prompt history (also prompts sent from
  another grokbot-tui on the same data dir).
- **Ctrl-R** opens a fuzzy history search (Ctrl-S reverse; Esc keeps the draft).
- **Ctrl-F** searches your messages and attachments (see Searching messages).
- **`/history [query]`** (`/prompts`) opens a scrollable browser of every saved prompt with
  its age and bot. Type to fuzzy-filter; ↑/↓, PgUp/PgDn and the mouse wheel move; Enter or
  clicking the selected row puts the prompt in the input (it is not sent); Esc closes. With
  `ui.keys=vim` it opens in list mode: `j`/`k`, `g`/`G`, Ctrl-d/Ctrl-u move, `/` filters,
  `q` closes. `/history clear` forgets the account's history after a confirmation.

Tab after `/` fuzzy-completes slash commands and arguments; Shift-Tab cycles.

## Loading spinner

While grokbot-tui signs in, loads the bot list and opens the first chat (at startup, on
`/account use <name>`, and during `/login gateway|cursor|desktop`), a centered overlay shows a
braille spinner and the current stage: *Signing in to <account>…*, *Checking gateway <host>…*,
*Exchanging Cursor token…* (the EnsureSandBox exchange), *Reading the desktop app session…*,
*Loading bots…*, *Loading chat with <bot>…*. The work runs in the background, so input keeps
working (Ctrl-D / Ctrl-Q still quit). If a step fails, the error replaces the spinner with a
retry hint; **Esc** dismisses it. Pref `ui.spinner = off` shows a static glyph instead of the
animation (reduced motion). Theme parts: `loading` (frame and message) and `spinner` (glyph).

## Layout toggles, menus and Alt over SSH

- `/header compact|full|toggle` (F7, the ▾/▴ button, **View** menu; pref `ui.header`) switches
  to a one-row header that keeps the bot, account, login status, SSH and 🔕.
- `/sidebar collapse|expand|toggle` (F8, the «/» buttons; pref `ui.sidebar_collapsed`) shrinks
  the bot list to a 1-column strip; click it to expand.
- `/refresh` (F12, the ↻ button in the sidebar header or the collapsed strip; keymap
  `agents.refresh`) re-fetches the bot list, so a bot made in the desktop app shows up without
  a restart. The list also refreshes on its own every `agents.refresh_interval_secs` seconds
  (default 60, `0` = off) and after a re-login. The merge keeps the open chat, its scroll and
  the sidebar cursor; new bots are added, deleted ones dropped (with a notice if it was the
  open one), renames and descriptions updated. A status line says what changed, e.g.
  "Bot list refreshed: 1 new (ratatoskn)". (`/bots` still prints the list; `/reload` still
  reloads the open chat's transcript.)
- Menus are clickable (title opens, hover highlights, click runs, outside closes). Without the
  mouse: F10, Alt-M, Alt+<underlined letter>, or `/menu [name]`.
- Clicking an empty part of a message bubble opens its reaction picker
  (`message.click_to_react`, default on).
- Over SSH, Alt+key can arrive as a separate ESC and key. ESC then a key within
  `keys.esc_alt_timeout_ms` (default 50 ms; 0 disables) is read as Alt+key. Windows Terminal
  keeps a few Alt chords for itself (Alt-Enter, Alt-Space, Alt-Shift-+/-); use F10 or `/menu`.

Images in replies and your image attachments show inline in the bubble (scaled to fit, at
most `images.inline_max_rows` rows, default 12; `images.inline = off` turns it off). Click one
for the full-size viewer.

`/status [extra text]` asks the current bot "Where are we now? Give me a brief status of your
current tasks." (plus your text) as a normal prompt; change the text with
`commands.status_prompt`.

Theme parts: `collapse_button`, `sidebar_strip`.

### Video, GIFs and music (1.24.0)

Videos and animated GIFs play **inside the bubble**, scaled like images (capped at
`images.inline_max_rows`), with a one-row bar under them: **[▶]/[⏸]** toggles playback,
clicking the bar seeks, and the label shows elapsed/total time (GIFs: frame position).
Clicking the picture opens the full-size player (Space play/pause, ←/→ seek 5 s, Home,
Esc/q close). Music links and attachments get the same bar. Only one video or track plays
at a time, and it pauses when scrolled out of view; GIFs loop on their own.

| Build | Decodes | Download |
|---|---|---|
| standard | GIF; H.264 MP4 (OpenH264, compiled in); MP3, AAC/M4A, FLAC, Vorbis, WAV. Other video codecs use a system `ffmpeg` if one is on PATH | `grokbot-tui-installer.sh/.ps1`, .deb/.rpm/.msi/.pkg, Homebrew, AUR |
| -ffmpeg | the above, plus FFmpeg linked in statically: nearly every codec and container, nothing to install | `grokbot-tui-ffmpeg-<target>.tar.xz/.zip` |

`grokbot-tui-setup.sh` / `grokbot-tui-setup.ps1` (release assets) ask which build to
install (`--variant standard|ffmpeg` / `-Variant` to skip the question).

Prefs: `media.inline` (`on`), `media.fps` (`10`, 1–30), `media.decoder`
(`auto`|`builtin`|`ffmpeg`|`system`), `media.gif_autoplay` (`on`). `/media
pause|stop|decoders`. Theme part: `media_bar`. On Linux, sound needs ALSA (libasound2).

### Right column: bot header and attachments

The right column starts with a header for the open chat's bot: its name in bold and its
description, wrapped (long ones are cut after 3 rows; click **▾ more** / **▴ less**). It
follows the bot-list refresh, so renames and new descriptions show at once. The sidebar
shows bot names only. A bot whose name has the description typed into it ("Lippy · Very
ratatui-focused …") shows as "Lippy", with the rest as its description.

Below the header, a collapsible bar on the right of each chat lists its attachments, sent and received, with
type, name, size, sender and time. **[view]** opens images in the viewer, text/HTML/Markdown
in the built-in browser and other files in the system app (with the SSH guard); **[⤓]** opens
the download modal; clicking a row jumps to its message. It has a search box and
All/Img/Vid/Aud/Doc filters, and keys j/k, Enter, v, d, /, t and Esc when focused.
Toggle with F9, the «/» buttons, View ▸ attachments, `/attachments show|hide|toggle`
(alias `/files`) or keymap `view.toggle_attachments`; pref `ui.attachments_panel`
(`show`/`hide`, default `show` since 1.28.0; `hide` = 1-column strip with the count). These
toggle the whole column, header included. Hidden when the chat body is under 70 columns.
Theme parts: `attach_panel_frame`, `attach_panel_filter`, `attach_panel_count`,
`bot_header_name`, `bot_header_desc`.

Bot replies are laid out like your messages: the bot's name on its own row, the reply
directly under it, the same 2/3 bubble width, on the left where yours are on the right.
Since 1.38.0 each message's text sits on a rounded bubble (`╭─╮ │ … │ ╰─╯`), accent-
coloured for yours and muted for the bot's, and its reaction pills ride the bubble's top
border at the top-right corner, like the desktop app. Every panel, dialog, popup and the
input box uses rounded corners too.

### Grok Bot cards: questions, approvals, secrets, forms and drafts (1.29.0)

When a bot asks you something, the chat shows a card with its state (● pending, answered,
dismissed or expired) and a button. Click the button, press **F11** (keymap `card.open`,
vim normal `c`) or run `/card` to open the newest pending one:

| Card | Keys |
|------|------|
| Question (single, multi-select, your own answer, multi-step) | `1`-`9` or `↑↓` `Enter`; `Space` toggles; `Tab` type your own; `x` dismiss |
| Auto-review approval / local tool permission | `a` approve (allow once), `d` deny |
| Secret request | type (masked) and `Enter`; `Ctrl-D` dismiss |
| Form | `Tab` between fields (secret fields masked), `Ctrl-S` submit, `Ctrl-D` dismiss |
| Email / Slack / meeting draft | `s` send, `e` edit the body, `x` discard |

Secrets are sent only through the gateway's `submitSecret` / `submitUserForm` calls; they
are never logged, stored or shown and are wiped after sending or closing. Purchase
approvals, connector grants and similar cards are shown read-only; act on them in the
desktop app. Theme parts: `card_frame`, `card_pending`, `card_done`, `card_button`.

## Muting other devices while you're here

Set `notifications.claim_presence = true` (per account; `/mute`, or **Notify → mute**) and,
while grokbot-tui is focused and you've used it in the last `notifications.idle_minutes`
(default 10), it tells the Grok Bot service this install is active, the same report the desktop
app sends, so pushes to your other devices can be held. 🔕 in the header shows a live claim;
`/whoami` shows the lease. It's released when the terminal loses focus, you go idle, log out,
switch accounts or quit; if the process is suspended the lease simply expires. It needs a
Cursor session (`/login desktop` or `/login cursor`). Theme part: `presence_indicator`.
