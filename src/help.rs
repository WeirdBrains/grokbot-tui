//! In-app help: topics, per-command details, error hints, and "did you mean".
//!
//! Help is shown in a modal (see [`HelpModal`]) and is never written to chat history.
//! Help text is static and never includes credentials.
use crate::commands::Registry;

/// A help topic. `id` is what `/help <id>` takes (spaces map to `-`).
pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub related: &'static [&'static str],
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "overview",
        title: "grokbot-tui help",
        body: "\
grokbot-tui is a terminal client for Grok Bot (unofficial, uses an undocumented gateway API).

Getting started
  1. Sign in to the Grok Bot desktop app once.
  2. Run /login desktop  (recommended — no token copying).
  3. Pick a bot in the left sidebar and type a message.

Plain text is sent to the active bot. Lines starting with / are commands.

Help topics (Enter on a link below opens it):
  login · login desktop · login cursor · login gateway
  keys · theme · theme-files · accounts · prefs · env · sync · delivery · troubleshooting

Search: /help <keyword> (e.g. /help keyring), or press / in this window.

Commands: /help <command> shows usage, examples and related topics.
  help login logout whoami bots bot history reload account prefs memory theme quit",
        related: &[
            "login",
            "login-desktop",
            "keys",
            "accounts",
            "sync",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login",
        title: "Logging in",
        body: "\
Each grokbot-tui account logs in once; the login is stored in ~/.grokbot-tui/context.db for that account only.

Which method?
  ★ /login desktop  RECOMMENDED. Uses the Grok Bot desktop app's saved session.
                    Requires the app installed and signed in on this computer.
    /login cursor   Paste a Cursor access token; grokbot-tui exchanges it for a gateway session.
                    Getting this token by hand is not documented — use /login desktop.
    /login gateway  Paste a gateway URL + gateway token directly.
                    Getting these by hand is not documented — use /login desktop.

Credentials are only sent to allowed hosts (see /help env, GROKBOT_ALLOW_LOCAL_GATEWAY).
/logout removes this account's stored login and keeps chats and memory.
/whoami shows how the account is logged in (never the token).",
        related: &[
            "login-desktop",
            "login-cursor",
            "login-gateway",
            "accounts",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login-desktop",
        title: "/login desktop (recommended)",
        body: "\
Usage: /login desktop [entry]

Steps
  1. Install the Grok Bot desktop app and sign in once.
  2. Linux only: install secret-tool (package libsecret-tools) and keep your keyring unlocked.
  3. Run /login desktop. grokbot-tui reads the app's saved session, checks it by listing bots,
     and saves it to the active grokbot-tui account.

What grokbot-tui reads (only when you run this command, or on a 401 for a desktop-login account)
  App data folder:
    Linux    $XDG_CONFIG_HOME/Grok Bot  or  ~/.config/Grok Bot
    macOS    ~/Library/Application Support/Grok Bot
    Windows  %APPDATA%\\Grok Bot
  1st choice: gateway-descriptor.json (a ready gateway session).
  Fallback:   sand-secrets.json → cursor-accounts → Cursor access token, exchanged via
              EnsureSandBox like /login cursor. That token is not stored by grokbot-tui.
  Decryption uses the OS keychain: secret-tool on Linux, the Keychain item
  'Grok Bot Safe Storage' on macOS, DPAPI on Windows. Nothing is printed or logged.

Several saved sessions/accounts
  grokbot-tui opens a pick list (e.g. '1) 3f2a9c1e… (active in app)'); choose with
  ↑↓/Enter or the mouse. Or name it directly: /login desktop <number>.
  Tip: /account add <alias> first so each desktop account maps to its own grokbot-tui account.

Client version
  grokbot-tui detects the installed app version for x-cursor-client-version (see /help env).

Errors
  APP_NOT_FOUND        App data folder missing → install the app / check the path above.
  NOT_SIGNED_IN        No saved session → open the app and sign in.
  SECRET_TOOL_MISSING  Install libsecret-tools (provides secret-tool).
  KEYCHAIN_LOCKED      Unlock your keyring / log in to your desktop session.
  AMBIGUOUS_ENTRIES    Several sessions → /login desktop <number>.
  NO_SUCH_ENTRY        That number/name doesn't exist → run /login desktop to list.
  DECRYPT_FAILED       Wrong keychain password or corrupt data → sign out/in in the app.
  'version no longer supported' → update the app, or set GROKBOT_TUI_CURSOR_CLIENT_VERSION.",
        related: &[
            "login",
            "login-cursor",
            "accounts",
            "env",
            "troubleshooting",
        ],
    },
    Topic {
        id: "login-cursor",
        title: "/login cursor",
        body: "\
Usage: /login cursor   (then paste the token; input is masked, Esc cancels)

What it does
  grokbot-tui sends the token to https://api2.cursor.sh/aiserver.v1.GrokBotService/EnsureSandBox
  and gets back a gateway URL, gateway token and routing headers. The Cursor token is
  stored for this account so grokbot-tui can re-exchange it when the gateway answers 401.

Getting a Cursor access token
  Not documented — use /login desktop. The desktop app keeps this token encrypted in
  sand-secrets.json; /login desktop reads and exchanges it for you automatically.
  A Cursor dashboard API key does NOT work here.

Headers sent: x-cursor-client-type (sand), x-cursor-client-version (detected app version),
x-sand-box-namespace (prod). Override with GROKBOT_TUI_CURSOR_CLIENT_* / prefs (see /help env).

Errors
  401 'version no longer supported' → update the desktop app or set GROKBOT_TUI_CURSOR_CLIENT_VERSION.
  417 (empty body) → the gateway session expired; grokbot-tui re-authenticates like a 401
    (desktop re-read / Cursor re-exchange). If it persists, run /login desktop.
  'no gatewayUrl/gatewayToken' → the token was rejected or is not an access token.",
        related: &["login", "login-desktop", "env", "troubleshooting"],
    },
    Topic {
        id: "login-gateway",
        title: "/login gateway",
        body: "\
Usage: /login gateway [url]   (then paste the gateway token; masked, Esc cancels)

What it does
  Uses a gateway session directly: a URL like https://<box>.cursorvm.com plus its token.
  grokbot-tui checks the URL against the host allowlist, validates by listing bots, then saves it.

Getting a gateway URL + token
  Not documented — use /login desktop. These are issued by EnsureSandBox (what
  /login cursor and /login desktop do for you) and are not shown in the desktop app.

Non-interactive use (startup account only, never saved)
  GROKBOT_TUI_TOKEN=<token> GROKBOT_TUI_GATEWAY_URL=<url> grokbot-tui
  Optional GROKBOT_TUI_GATEWAY_HEADERS='{\"x-anyrun-network-token\":\"…\"}'

Allowed hosts: https on cursor.sh, cursor.com, *.cursor.sh, *.cursor.com, *.cursorvm.com.
Loopback only with GROKBOT_ALLOW_LOCAL_GATEWAY=1. URLs with user:pass@ are rejected.",
        related: &["login", "login-desktop", "env", "troubleshooting"],
    },
    Topic {
        id: "keys",
        title: "Keys",
        body: "\
Anywhere
  F1, or ? on an empty input   Open this help
  F2                           Pick a bot (list popup; same as /bot)
  F3                           Pick an account (same as /account use)
  F4                           Theme mode: dark → light → system (same as /theme)
  F5                           Pick a theme (same as /theme use)
  Shift-F5                     Theme editor (same as /theme edit; some terminals send F17)
  F6                           Mouse capture on/off (same as /mouse)
  Ctrl-C / Ctrl-Shift-C        Context-aware (see below). Ctrl-Shift-C never quits.
  Ctrl-A                       Select all of the input (or the chat, when it has focus)
  Ctrl-Z                       Restore the input a Ctrl-C cleared
  Ctrl-D                       Delete forward; on an empty input like Ctrl-C
  Ctrl-Q                       Quit at once (also /quit)
  Shift+←/→/Home/End           Select text in the input

Ctrl-C / Ctrl-Shift-C, in order: (1) a selection in the input or the chat is
copied; (2) a pending reply is cancelled when the input or chat has focus; (3) text in
the focused input: Ctrl-C clears it (Ctrl-Z brings it back), Ctrl-Shift-C copies it;
(4) an open overlay closes like Esc; (5) over the chat with nothing selected: a hint;
(6) otherwise 'press again to quit' — a second Ctrl-C within 1.5 s with nothing done
in between quits.
  Tab                          After `/`: fuzzy-complete a command or argument (Shift-Tab cycles).
                               Otherwise switch focus between input and sidebar
  Ctrl-R                       Fuzzy search prompt history (Ctrl-S reverse; Esc keeps the draft)
  /history [query]             Browse all past prompts (persisted; newest first, age + bot)
  :name then Tab               Emoji shortcode completion (:thumbsup: → 👍); complete :name:
                               shortcodes are also turned into emoji when you send
  Click a link                 Opens it in the built-in browser (links.open_with = builtin,
                               the default) or the system browser (= system; /links toggles,
                               menu Web). Ctrl/Alt-click or vim NORMAL x uses the other one.
                               Terminal OSC 8 hyperlinks (Ghostty, kitty, WezTerm, Windows
                               Terminal, foot, Alacritty 0.13+; Ctrl/Cmd-click in the
                               terminal) always go to the system browser: the terminal opens
                               them, not grokbot-tui. GROKBOT_TUI_NO_HYPERLINKS=1 turns them off.
  /mute [on|off|toggle]        Claim presence while you use grokbot-tui (focused, input in the
                               last notifications.idle_minutes, default 10) so Grok Bot pushes
                               to your other devices are held; 🔕 while held, /whoami shows
                               the lease. Released on blur, idle, logout, account switch, quit.
  /status [extra text]         Ask the bot “Where are we now? Give me a brief status of your
                               current tasks.” (+ extra text) as a normal prompt; change the
                               text with pref commands.status_prompt. Also menu Chat.
  /browse [url] (/web)         Built-in text browser: no JavaScript, no cookies, 5 MiB / 20 s
                               cap. Tab completes recent URLs; vim NORMAL b. In it: e, Ctrl-L
                               or : edit the URL; j/k ↑/↓ PgUp/PgDn g/G Ctrl-d/u wheel scroll;
                               Tab/Shift-Tab pick a link, Enter or click follows, a number then
                               Enter jumps to [n]; H/L or Alt-←/→ back/forward; r reload;
                               / n N search; o system browser; y copy URL; d download;
                               Esc/q close. [🖼 …] images open the image viewer.
  SSH                          Over SSH (SSH_CONNECTION/SSH_CLIENT/SSH_TTY or a parent sshd;
                               GROKBOT_TUI_FORCE_SSH=0/1 overrides) the header shows SSH and
                               links always use the built-in browser (links.ssh_behavior =
                               builtin; respect_pref follows links.open_with). Anything that
                               would open an app on the remote machine (o, /open, Ctrl-click,
                               the viewer's o) warns and needs repeating within 5 s.
  Inline images                Replies' images and your image attachments draw in the bubble
                               (images.inline = on|off, images.inline_max_rows = 12); click
                               one for the viewer. Shown once fully on screen.
  Tab completion               Lists scroll (PgUp/PgDn, wheel, n/total shown). Paths with
                               spaces: quotes or backslash escapes; completions insert
                               backslash-escaped names.
  /view [n]                    Image viewer: the image nearest the bottom of the chat (or the
                               n-th newest), as large as the terminal allows (kitty / sixel /
                               iTerm2, else halfblocks). Also: click a [🖼 …] placeholder,
                               vim NORMAL O. In the viewer: Esc/q or click outside close,
                               o/Enter open externally, ←/→ (n/p, h/l) previous/next image
  /open [n|list]               Open the newest image (then link) in replies: images in the
                               viewer, links per links.open_with (system: xdg-open / open /
                               Windows handler); vim NORMAL: o (x: the other opener).
                               Images show as [🖼 alt · file] placeholders
  /react <emoji|:code:> [n]    Toggle a reaction pill on the newest bot message (n=2 the one
                               before; vim NORMAL: r). Tab completes the emoji (on the
                               message first, then recent, then common; :code or code) and n.
                               Bare /react, vim R or a click on a message's [☺+] opens the
                               emoji picker: type to search (vim: / first), arrows/hjkl or
                               wheel move, Enter or click toggles, Esc/click outside closes. Stored locally: the Grok Bot API has
                               no reaction endpoint, so bots and other clients don't see them
  /attach <path…>              Attach files to your next prompt (alias /file; up to 6). Tab
                               completes paths; paste a file path to attach it. Uploads
                               start right away; chips above the input show progress (✓/✗),
                               click a chip to preview, ✕ removes it. Sending waits for
                               uploads. Limits: images 25 MiB, video 1 GiB, others 250 MiB
                               (the server is final). Needs /login desktop or /login cursor.
                               /attach remove <n>, /attach clear, /detach [n|all]
  /download [n|list]           Save an image or file from the bot's replies (newest; n=2 the
                               one before; vim NORMAL: d; alias /save). Replies with images or
                               file links get [⤓] name buttons; click one to open the modal:
                               path prefilled ~/Downloads/<name>, Tab completes paths, Down
                               browses folders (arrows/hjkl, Enter/l opens, h parent).
                               Overwrites and new folders are confirmed. Remote files stream
                               in the background (cap: downloads.max_mb); local files copy
  PgUp / PgDn, mouse wheel     Scroll the chat (not the prompt history).
                               Only the exact bottom follows new content; scroll up
                               pins the view. '↓ N new' while unread (click / End /
                               vim G to jump and follow). Send or switch bot/account
                               follows again.
  Esc                          Clear an in-app selection; cancel a prompt; return focus to input
  /switch [alias]              Switch account (bare opens the picker)
  /keymap [vim|default|reload|list]  Opt-in vim keys (ui.keys); list/reload keymap.toml

Input focused (multi-line)
  Enter                     Send message or run /command
  Ctrl-Enter                New line. On Windows (conhost / Windows Terminal) this is the
                            one that always works; on Linux it needs a terminal that reports
                            modifiers (kitty, Ghostty, foot, WezTerm, xterm modifyOtherKeys).
                            Terminals that send Ctrl-Enter as Ctrl-J still get a newline;
                            ones that send plain Enter (e.g. Alacritty by default) send.
  Shift/Alt-Enter           New line when the terminal reports modifiers (kitty protocol
                            / modifyOtherKeys). Remap via keymap `input.newline`.
  keys.kitty_protocol       auto (default) | on | off. With the kitty keyboard protocol
                            (kitty, Ghostty, foot, WezTerm, recent Alacritty/iTerm2) Alt,
                            Ctrl-Enter and Shift-Enter are reliable and Esc has no delay;
                            elsewhere the ESC-prefix Alt handling is used. /kitty shows or sets it
  Ctrl-J                    New line (works everywhere, including macOS Terminal.app)
  \\ then Enter             New line (strip the backslash) when modifiers are unavailable
  Alt-Enter                 New line (Option-as-Meta / ESC+CR). macOS Terminal.app:
                            enable “Use Option as Meta key”, or use Ctrl-J / \\+Enter.
  F10 / Alt-M / /menu [name] Menu bar (Chat, Account, View, Web, Theme, Notify, Help).
                            Alt+<underlined letter> opens that menu (Alt-C Chat, Alt-V
                            View, …). Or click a title; hover highlights, click runs,
                            click outside closes (needs mouse capture: F6).
  Alt over SSH              Terminals send Alt+x as ESC x. Over SSH (e.g. Windows Terminal
                            → ssh) the ESC can arrive alone, so ESC then a key within
                            keys.esc_alt_timeout_ms (default 50; 0 = off; raise it on slow
                            links) counts as Alt+key. A plain Esc still works (it fires
                            after that delay). Windows Terminal keeps some Alt chords
                            itself (Alt-Enter fullscreen, Alt-Space, Alt-Shift-+/-, Alt-F4):
                            use F10 or /menu instead.
  F7 / /header              Header: one row (compact) or full; also the ▾/▴ button and
                            View menu (pref ui.header; keymap view.toggle_header)
  F8 / /sidebar             Collapse the bot list to a 1-column strip (● active); click
                            the strip or » to expand, « to collapse (pref
                            ui.sidebar_collapsed; keymap view.toggle_sidebar)
  F12 / /refresh            Re-fetch the bot list now (also the ↻ in the sidebar header
                            or strip). New bots appear, deleted ones go, renames show;
                            the open chat stays. Automatic every
                            agents.refresh_interval_secs (60; 0 = off) and on re-login
                            (keymap agents.refresh)
  F9 / /attachments (/files) Right column (shown by default): the bot's name and
                            description (▾ more for long ones), then the panel listing
                            this chat's attachments (sent and
                            received) with [view] and [⤓]; click a row to jump to its
                            message. Collapses to a 1-column strip with the count (pref
                            ui.attachments_panel; keymap view.toggle_attachments).
                            In the panel: j/k move, Enter jump, v view, d download,
                            / filter, t type (All/Img/Vid/Aud/Doc), Esc back to input
  Video, GIFs, music        Play inline in the bubble: [▶]/[⏸] toggles, click the bar to
                            seek, click the picture for the full-size player (Space,
                            ←/→ 5 s, Home, Esc). One video/track plays at a time and
                            pauses when scrolled away; GIFs loop (media.gif_autoplay).
                            Prefs: media.inline on|off, media.fps (10), media.decoder
                            auto|builtin|ffmpeg|system. /media pause|stop|decoders
  /license                  Licence (GPL-3.0-or-later) and third-party notices
  Click a message           Opens the reaction picker for it (pref message.click_to_react,
                            default on). Links, images, [☺+] and [⤓] keep their actions;
                            dragging still selects text
  /notify                   Toasts: dnd, on/off <kind>, reload rules
  ← → / Home End            Move the cursor / to the start or end of the line
  ↑ / ↓                     Previous / next line; on the first / last line, prompt history
  Backspace / Delete        Delete before / at the cursor
  Long lines wrap; the box grows up to 10 rows, then scrolls to keep the cursor
  visible (row/total shown bottom-right). Pasted text keeps its line breaks.

Sidebar focused
  ↑ ↓ / k j  Move      Enter  Open bot

Mouse
  Click a bot to open it. Click '@account ▾' in the header to pick an account.
  Click '‹theme› ▾' in the header to pick a theme, dark / light / system to switch mode.
  The header shows 'mouse: on/OFF'.

Selecting and copying text
  With mouse capture on (default; F6 / /mouse; persisted as ui.mouse_capture):
  • Drag in the chat to select. The selection is copied on mouse-up (OSC 52).
    Esc clears it. Ctrl-C / Ctrl-Shift-C copy a selection; Ctrl-A selects the whole feed
    when the chat has focus.
  • Hold Shift (or Option) while dragging to use the terminal's own selection instead.
  • F6 or /mouse off: release the mouse for terminal selection (clicks/wheel go off).
  • /copy: copy the last bot message via OSC 52 (tmux: `set -g set-clipboard on`).

In a pick list (F2, F3, F5, /bot, /account use)
  ↑ ↓ / j k, PgUp PgDn, Home End  Move      Enter  Pick      Esc / q  Cancel
  Mouse: hover highlights, click picks, click outside cancels, wheel moves

In this help window
  ↑ ↓ / j k, PgUp PgDn  Scroll      Tab / ← →  Select a related link
  Enter  Open link      Backspace  Back      Esc / q  Close
  /      Search all help by keyword (type, Enter to run, Esc to cancel)

In the theme editor (Shift-F5): see /help theme",
        related: &["overview", "theme", "accounts"],
    },
    Topic {
        id: "theme",
        title: "Header and theme",
        body: "\
The header shows the Grok Bot logo and title, the account switcher, the theme
switcher, the login, the active bot and the mouse state. In terminals smaller than
80 × 24 it shrinks to one row with the same switchers.

Account switcher
  '@alias ▾' — click it or press F3 to pick an account (same as /account use).

Theme switcher
  '‹default› ▾' — the theme in use. Click it or press F5 to pick another theme
  (or /theme use <name>). Moving through the list previews each theme; Enter keeps
  it, Esc goes back to the one you had. The list ends with 'Edit themes…'. A theme that can't be
  loaded shows as '‹name: missing›' or '‹name: broken›' (see below).

Mode: dark · light · system
  Every theme has a dark and a light variant. Click one, press F4 to cycle, or
  /theme [dark|light|system]. system follows the terminal background: grokbot-tui
  asks the terminal at startup (OSC 11), else reads $COLORFGBG, else uses dark. The
  header shows what system resolved to, e.g. 'system (→ light)'. Restart after
  changing the terminal's colors.

Both choices are global prefs (ui.theme, ui.theme_mode), used for every account.
grokbot-tui --theme <name> uses a theme for that run only (nothing is saved; picking
a theme in the app saves it as usual). grokbot-tui --list-themes lists them and exits.

Commands
  /theme use <name>     Switch theme (also /theme <name>; no name: pick list)
  /theme list           List theme files (● = in use; broken files say why)
  /theme edit [name]    Open the editor (default: the theme in use)
  /theme init [name|all]
                        Download a shipped theme from the release (default: default; all:
                        every shipped one; existing files are kept)
  /theme starter [name] Write a minimal starter theme (no download; never overwrites)
  /theme delete <name>  Delete a theme file (asks first)
  /theme export <name> [path]
                        Copy a theme file out (default <data dir>/exports/<name>-<UTC time>.toml;
                        a folder gets <name>.toml; never overwrites)
  /theme reload         Re-read theme files after editing them by hand

Theme editor (Shift-F5)
  Left: every palette color and every part of the screen. Right: a live preview of
  the app in the edited theme, before you save, with the selected item highlighted
  and samples of the chat entries and modals.
  ↑ ↓ / j k, PgUp PgDn, Home End  Select      Tab  Flip dark / light variant
  Enter / e   Edit: a color takes hex (#ff8800), 0-255 or a name (cyan); a part
              opens its fields (fg, bg, border for boxes, bold italic underline
              reverse dim): ← → choose, Enter edit/toggle, Space toggle, Backspace clear
  p           Color picker with tabs: 16 colors · 216 cube · 24 grays · @ palette
              (Tab / Shift-Tab switch, arrows, Enter or click; @ palette offers the
              palette colors for parts, and reset)
  r / R       Revert this item / everything since the last save
  s, Ctrl-S   Save (applied right away if it's the theme in use)
  d / n       Duplicate this theme / new starter theme (asks for a name)
  d n x with Ctrl work too (Ctrl-D, Ctrl-N, Ctrl-X)
  x, Delete   Delete this theme (asks first)
  t           Edit another theme      [ / ]  Previous / next theme file
  Esc / q     Close (asks if there are unsaved changes; also Shift-F5 / F17)
  h           Highlight on/off: the selected color or part is marked in reverse video
              in the preview (the preview title says when it isn't on screen, e.g. the
              compact header or a focused sidebar)
  Parts' colors can point at palette colors (@accent), so a palette change restyles
  every part that uses it. A low-contrast warning appears when text and its
  background are hard to tell apart (WCAG ratio below 3:1), and ⚠ marks such items
  in the list.

Theme files: see /help theme-files.",
        related: &["theme-files", "keys", "prefs", "accounts"],
    },
    Topic {
        id: "theme-files",
        title: "Theme files",
        body: "\
Every theme, including default, is a TOML file in ~/.grokbot-tui/themes/<name>.toml
(or <data dir>/themes with --data-dir). Themes are read only from there: nothing is
built into the program and no system folder is searched. Names: lowercase letters,
digits, - and _ (up to 32).

Where default.toml comes from
  On first run, when your themes folder has no themes, grokbot-tui downloads the
  default theme for its own version in the background:
    https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v<version>/themes/default.toml
  checks that it's a valid theme, saves it as default.toml and says so. Startup never
  waits for it (plain colors until it arrives; it gives up after 5 s). Offline or
  failed: plain terminal colors and a notice; run /theme init later to try again.
  Existing files are never overwritten.
  Turn the download off with GROKBOT_TUI_NO_THEME_DOWNLOAD=1 or
  /prefs set ui.theme_download false (then use /theme starter, or save the file
  yourself and /theme reload).

When a theme can't be used
  If the selected file is missing or broken (invalid TOML, newer schema, over 64 KiB:
  the size is checked before reading),
  grokbot-tui uses your terminal's own colors and says why. Highlights still show
  (reverse video). Fix the file and /theme reload, pick another theme (F5), or
  /theme init. A broken file is never overwritten without asking.

/theme init [name|all] downloads <name>.toml from the release's themes/ folder (default:
default; all: every shipped theme, which is just default for now). Existing files are
kept. If downloading is off it writes the minimal starter instead; if it fails it says why.
--list-themes and --theme also download the default first when the folder is empty
(waiting up to 3 s). GROKBOT_TUI_THEME_URL replaces the download address (a URL that
<name>.toml is appended to, e.g. a fork or a local server).
/theme starter [name] always writes the minimal starter: every part mapped to a palette
role (title → @accent, errors → @danger, …) but every color set to your terminal's
default. Set the colors in the editor to make it yours.

Format
  schema = 1
  name = \"Default\"
  description = \"…\"

  [dark.colors]             # the palette: 14 named colors
  text = 252                # 0-255, \"#rrggbb\", \"#rgb\", a name (cyan, darkgray…), \"reset\"
  accent = \"cyan\"
  …
  [dark.styles]             # parts of the screen
  brand = { fg = \"@on_accent\", bg = \"@accent\", bold = true }
  panel_focused = { fg = \"@text\", border = \"@accent\" }
  …
  [light.colors] / [light.styles]   the same for the light variant

  Style fields: fg, bg, border (boxes only), bold, italic, underline, reverse, dim.
  Colors: a literal or @palette-name. Missing colors use the terminal default,
  missing parts use the starter rule, a missing variant copies the other one;
  each is reported once as a warning, as are unknown keys and a missing schema line.
  Layout (borders, padding) isn't themeable.

Saving
  Theme files are written atomically (a temp file in the same folder, then renamed)
  and readable only by you (0600; the themes folder is created 0700). Windows keeps
  its default permissions.

Command line
  grokbot-tui --theme <name>   use <name> for this run only (not saved); a missing or
                               broken theme stops before the UI starts
  grokbot-tui --list-themes    list the themes (● = in use) and exit
  /theme export <name> [path]  copy a theme file out (never overwrites)

Palette colors
  text background accent subtle muted header warn danger ok on_accent
  modal_text modal_background logo_dark logo_light
  The editor shows what each one is used for.",
        related: &["theme", "prefs"],
    },
    Topic {
        id: "accounts",
        title: "Accounts",
        body: "\
grokbot-tui keeps several accounts fully separate: logins, bots, chats, memory and prefs.

  /account                  List accounts (* = active)
  /account add <alias>      Create an account
  /account use [alias]      Switch (no alias, F3 or clicking @account ▾: pick list)
  /account rename <a> <b>   Rename
  /account remove <alias>   Delete (confirm in a pick list; Cancel is the default)

Start in an account: grokbot-tui --account work  or  GROKBOT_TUI_ACCOUNT=work grokbot-tui
GROKBOT_TUI_TOKEN only applies to the account selected at startup.
Several desktop Cursor accounts? Add one grokbot-tui account each, then /login desktop <number>.",
        related: &["login", "login-desktop", "prefs", "env"],
    },
    Topic {
        id: "prefs",
        title: "Preferences",
        body: "\
/prefs                 List preferences
/prefs get <key>       Show one
/prefs set <key> <v>   Set (empty value resets to default)

Per-account keys
  default_bot            Bot id/name to open when nothing else is selected. Default: unset.
  active_bot             Last opened bot (set automatically).
  show_hidden_bots       true/false — show bots hidden from the sidebar. Default: false.
  cursor.client_version  Override x-cursor-client-version for EnsureSandBox.
                         Default: detected desktop app version, else 0.70.2.
  cursor.client_type     Override x-cursor-client-type. Default: sand.
  sand.box_namespace     Override x-sand-box-namespace. Default: prod.

Global keys
  ui.sidebar_width       Sidebar width, 10–80. Default: 24.
  ui.theme               Theme name (a file in <data dir>/themes). Default: default.
  ui.theme_download      true/false — download the default theme when the themes folder
                         is empty, and for /theme init. Default: true.
                         Also F5, /theme use, or clicking the header (see /help theme).
  ui.theme_mode          dark, light or system (follow the terminal). Default: system.
                         Also F4, /theme, or clicking the header (see /help theme).
  ui.title_glyphs        Big header title glyphs: auto, quadrant or half. Default: auto
                         (half in macOS Terminal, whose fonts mangle quadrant glyphs;
                         quadrant elsewhere). Over ssh the terminal is unknown, so set
                         half if the title looks broken. Half needs 128+ columns.
  history.max_entries    Prompts kept per account (100–1000000). Default: 10000.
  downloads.max_mb       Largest /download in MB (1–4096). Default: 25.
  active_account         Account opened at startup (set automatically).

Environment variables override the cursor.* / sand.* prefs (see /help env).",
        related: &["env", "accounts", "login-cursor"],
    },
    Topic {
        id: "env",
        title: "Environment variables",
        body: "\
Login (startup account only, never saved)
  GROKBOT_TUI_TOKEN
      Gateway token override.
  GROKBOT_TUI_GATEWAY_URL
      Gateway URL for that token (else the stored URL).
  GROKBOT_TUI_GATEWAY_HEADERS
      JSON object of extra gateway headers.

EnsureSandBox (Cursor token exchange)
  GROKBOT_TUI_CURSOR_API_BASE_URL
      Backend base (must be *.cursor.sh / *.cursor.com).
  GROKBOT_TUI_CURSOR_CLIENT_VERSION
      x-cursor-client-version. Priority:
      env > pref cursor.client_version > detected app > 0.70.2
  GROKBOT_TUI_CURSOR_CLIENT_TYPE
      x-cursor-client-type (default sand).
  GROKBOT_TUI_SAND_BOX_NAMESPACE
      x-sand-box-namespace (default prod).

Other
  GROKBOT_TUI_ACCOUNT
      Starting account alias (same as --account).
  GROKBOT_TUI_ALLOW_LOCAL_GATEWAY=1
      Allow http(s) loopback gateways (development).
  GROKBOT_TUI_APP_DIR
      Grok Bot install dir for version detection.
  GROKBOT_TUI_NO_THEME_DOWNLOAD=1
      Never download the default theme (see /help theme-files).
  GROKBOT_TUI_THEME_URL
      Download themes from here instead of the release tag (<url><name>.toml).
  The library's own names GROKBOT_ALLOW_LOCAL_GATEWAY and GROKBOT_APP_DIR work too.

Older names: every GB_* variable (e.g. GB_GROKBOT_TOKEN) still works when its
GROKBOT_TUI_* name is not set; the new name wins when both are set.

Data lives in ~/.grokbot-tui/context.db (override with --data-dir). On first start,
an old ~/.gb folder is moved there. /whoami shows the client version in use
and where it came from.",
        related: &["prefs", "login-cursor", "login-gateway", "troubleshooting"],
    },
    Topic {
        id: "sync",
        title: "Chat sync",
        body: "\
Bots come from the gateway's listAgents. Messages come from getAgentTranscriptTail
(there is no separate chat list).

  • Opening a bot (or /reload) pages the whole transcript (up to 1000 entries),
    merges it into ~/.grokbot-tui/context.db and redraws the chat from there — so messages
    from the desktop app appear too.
  • While a bot is open grokbot-tui polls for new messages every ~1 s, backing off to 15 s
    when idle, and every 1 s right after you send.
  • The chat you see is always rebuilt from that database. Your sent message is saved
    there first (as pending) and shows immediately; notices and errors appear in a
    separate section below the chat and are not saved.
  • /bots refreshes the bot list.",
        related: &["delivery", "troubleshooting"],
    },
    Topic {
        id: "delivery",
        title: "Sending and delivery",
        body: "\
Every message you send shows as pending until the transcript confirms it.

  Accepted  The gateway returned a receipt. The message is confirmed when it appears.
  Rejected  The gateway refused it (4xx, policy). It was NOT delivered; fix and resend.
  Unknown   5xx, network error, timeout or no receipt. grokbot-tui then asks the gateway
            (promptAcceptanceStatus, by the message's nonce) whether it landed:
              accepted  → nothing to do; it will appear in the chat.
              rejected  → 'Not delivered (rejected by the gateway: <code>)'.
              not found → 'Not delivered … Safe to resend' (it never arrived).
              pending   → asked again a few times.
            Only if the gateway can't tell does grokbot-tui show 'Delivery unknown'; then
            wait for the message to appear (or not) before resending. grokbot-tui never
            resends itself.",
        related: &["sync", "troubleshooting"],
    },
    Topic {
        id: "troubleshooting",
        title: "Troubleshooting",
        body: "\
'Gateway rejected the credential (401)'
  desktop login: grokbot-tui re-reads the app session once automatically; if it still fails,
  open the app, make sure you're signed in, then /login desktop.
  cursor login: grokbot-tui re-exchanges once; otherwise /login cursor with a fresh token.
  gateway login: /login gateway with a new token.

'This version of Grok Bot is no longer supported'
  Update the desktop app, or set GROKBOT_TUI_CURSOR_CLIENT_VERSION / pref cursor.client_version.
  /whoami shows the version in use.

Desktop errors (APP_NOT_FOUND, NOT_SIGNED_IN, SECRET_TOOL_MISSING, KEYCHAIN_LOCKED, …)
  See /help login desktop for each fix.

'host not allowed' — the URL isn't on the allowlist (see /help login gateway).

Chat looks out of date — /reload reloads the full transcript.
Team bot in the list but empty chat / send fails — re-login with /login cursor
  or /login desktop (needs a Cursor token) so shared agents get a viewer sessionId.
No bots — create one in the desktop app, then /bots.
Running gb prints git errors — gb is often a git alias; the command is grokbot-tui.",
        related: &[
            "login-desktop",
            "login-cursor",
            "login-gateway",
            "sync",
            "env",
        ],
    },
];

/// Normalize `/help` arguments to a topic id: "login desktop" → "login-desktop".
pub fn normalize(arg: &str) -> String {
    arg.trim()
        .trim_start_matches('/')
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

pub fn topic(id: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.id == id)
}

/// Extra details for `/help <command>`: (description, examples, related topics).
fn command_extra(
    name: &str,
) -> (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
) {
    match name {
        "help" => (
            "Open help. With a topic or command name, open that page; any other words search all help pages.",
            &[
                "/help",
                "/help login desktop",
                "/help keys",
                "/help bot",
                "/help keyring",
                "/help search 401",
            ],
            &["overview", "keys"],
        ),
        "login" => (
            "Log in the active account. /login desktop is recommended. Bare /login opens login help.",
            &[
                "/login desktop",
                "/login desktop 2",
                "/login cursor",
                "/login gateway https://box.cursorvm.com",
            ],
            &["login", "login-desktop", "login-cursor", "login-gateway"],
        ),
        "logout" => (
            "Delete the active account's stored login. Chats, memory and prefs are kept.",
            &["/logout"],
            &["login", "accounts"],
        ),
        "whoami" => (
            "Show the account, login method, gateway host and the client version in use (never tokens).",
            &["/whoami"],
            &["login", "env"],
        ),
        "bots" => (
            "Refresh the bot list from the gateway and print it.",
            &["/bots"],
            &["sync", "prefs"],
        ),
        "bot" => (
            "Open a bot by name or id (also: click it, F2, or bare /bot for the pick list).\n\
             /bot new <name> creates a bot on the gateway (createAgent) and opens it.\n\
             /bot delete [name] deletes the named (or current) bot (deleteAgent) after a\n\
             confirmation pick list where Cancel is the default. Deleting is permanent.",
            &[
                "/bot Helper",
                "/use Helper",
                "/bot new Researcher",
                "/bot delete Researcher",
            ],
            &["sync", "keys"],
        ),
        "shell" => (
            "Run a command on THIS machine with $SHELL -c (sh if unset). Purely local:\n\
             nothing is sent to the bot/gateway and nothing is saved to chat history.\n\
             Output opens in a popup: the command line, stdout then stderr, and the exit\n\
             code. It shows 'running…' until the command finishes (the UI stays usable).\n\
             Scroll with ↑/↓, PgUp/PgDn, Home/End or the mouse wheel; Esc or q closes.\n\
             No stdin (interactive programs won't work); killed after 120s; output capped\n\
             at 500 lines / 64 KB. Red border = nonzero exit; exit 127 usually means\n\
             command not found.",
            &["/shell ls -la", "/shell git status", "/sh uptime"],
            &["keys"],
        ),
        "history" => (
            "Browse and search every prompt you've sent (kept in context.db per account, so\n\
             it survives restarts; newest history.max_entries kept, default 10000).\n\
             Newest first, with age and bot. Type to fuzzy-filter; ↑/↓, PgUp/PgDn and the\n\
             mouse wheel move; Enter (or clicking the selected row) puts the prompt in the\n\
             input without sending; Esc closes and keeps your draft. With ui.keys = vim the\n\
             list opens in list mode: j/k, g/G, Ctrl-d/Ctrl-u move, / filters, q closes.\n\
             /history clear forgets the account's history after a confirmation.\n\
             ↑ in an empty input and Ctrl-R search the same history. /login lines are\n\
             stored without their arguments.",
            &["/history", "/history deploy", "/history clear", "/prompts"],
            &["keys", "prefs"],
        ),
        "reload" => (
            "Reload the active bot's full transcript from the gateway.",
            &["/reload", "/transcript"],
            &["sync"],
        ),
        "account" => (
            "Manage separate accounts.",
            &[
                "/account",
                "/account add work",
                "/account use work",
                "/account rename work job",
                "/account remove job",
            ],
            &["accounts", "login"],
        ),
        "prefs" => (
            "List, read or change preferences.",
            &[
                "/prefs",
                "/prefs set show_hidden_bots true",
                "/prefs set cursor.client_version 0.70.2",
            ],
            &["prefs", "env"],
        ),
        "memory" => (
            "Local notes per bot (stored only in ~/.grokbot-tui, never sent).",
            &["/memory", "/memory add lang rust", "/memory clear lang"],
            &["accounts"],
        ),
        "theme" => (
            "Switch, list and edit themes, or the dark/light mode. Bare /theme cycles dark → light → system (also F4); system follows the terminal background, else dark. /theme use <name> (or F5) picks a theme file, /theme edit (or Shift-F5) opens the editor, /theme list lists them, /theme init [name|all] downloads a shipped theme from the release (else writes the minimal starter), /theme starter [name] always writes the starter, /theme delete <name> deletes one (asks first), /theme export <name> [path] copies one out, /theme reload re-reads the files. Saved in the global prefs ui.theme and ui.theme_mode.",
            &[
                "/theme",
                "/theme light",
                "/theme use default",
                "/theme edit",
                "/theme init all",
                "/theme list",
            ],
            &["theme", "theme-files", "keys", "prefs"],
        ),
        "quit" => ("Exit grokbot-tui.", &["/quit"], &["keys"]),
        _ => ("", &[], &["overview"]),
    }
}

/// A rendered help page: title, body, related topic ids.
pub struct Page {
    pub id: String,
    pub title: String,
    pub body: String,
    pub related: Vec<String>,
}

/// Resolve `/help <arg>` to a page: a topic, a command, `search <words>`, or —
/// when nothing matches exactly — keyword search results (if any).
pub fn page(registry: &Registry, arg: &str) -> Option<Page> {
    let raw = arg.trim();
    if let Some(q) = raw
        .strip_prefix("search:")
        .or_else(|| raw.strip_prefix("search "))
    {
        return Some(search_page(registry, q.trim()));
    }
    if let Some(p) = exact_page(registry, raw) {
        return Some(p);
    }
    (!search(registry, raw).is_empty()).then(|| search_page(registry, raw))
}

/// One keyword-search hit.
pub struct Hit {
    pub id: String,
    pub title: String,
    pub snippet: String,
}

/// Search all topics and command pages. Every query word must appear (case-
/// insensitive) in the id, title or body. Title/id matches rank first.
pub fn search(registry: &Registry, query: &str) -> Vec<Hit> {
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.trim_start_matches('/').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return vec![];
    }
    let mut hits: Vec<(usize, Hit)> = vec![];
    for name in all_names(registry) {
        let Some(p) = exact_page(registry, &name) else {
            continue;
        };
        let head = format!("{} {}", p.id, p.title).to_lowercase();
        let body = p.body.to_lowercase();
        if !words.iter().all(|w| head.contains(w) || body.contains(w)) {
            continue;
        }
        let score = words.iter().filter(|w| head.contains(*w)).count();
        let snippet = p
            .body
            .lines()
            .find(|l| words.iter().any(|w| l.to_lowercase().contains(w)))
            .unwrap_or_else(|| p.body.lines().next().unwrap_or(""))
            .trim();
        let snippet: String = if snippet.chars().count() > 90 {
            format!("{}…", snippet.chars().take(89).collect::<String>())
        } else {
            snippet.to_string()
        };
        hits.push((
            score,
            Hit {
                id: p.id,
                title: p.title,
                snippet,
            },
        ));
    }
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits.into_iter().map(|(_, h)| h).collect()
}

fn search_page(registry: &Registry, q: &str) -> Page {
    let hits = search(registry, q);
    let mut body = if hits.is_empty() {
        format!("No help pages match '{q}'.\n\nTry fewer or different words, or browse /help.\n")
    } else {
        format!(
            "{} result(s) for '{q}'. Tab/←→ to pick one below, Enter to open.\n\n",
            hits.len()
        )
    };
    for h in &hits {
        body.push_str(&format!(
            "{}  ({})\n    {}\n\n",
            h.title,
            link_label(&h.id),
            h.snippet
        ));
    }
    body.push_str("Press / to search again.");
    let mut related: Vec<String> = hits.into_iter().map(|h| h.id).collect();
    if related.is_empty() {
        related.push("overview".into());
    }
    Page {
        id: format!("search:{q}"),
        title: format!("search \"{q}\""),
        body,
        related,
    }
}

fn exact_page(registry: &Registry, arg: &str) -> Option<Page> {
    let id = normalize(arg);
    let id = if id.is_empty() {
        "overview".to_string()
    } else {
        id
    };
    if let Some(t) = topic(&id) {
        return Some(Page {
            id: t.id.into(),
            title: t.title.into(),
            body: t.body.into(),
            related: t.related.iter().map(|s| s.to_string()).collect(),
        });
    }
    let c = registry.find(id.strip_prefix("cmd-").unwrap_or(&id))?;
    let (desc, examples, related) = command_extra(c.name);
    let mut body = format!("Usage: {}\n\n{}\n", c.usage, c.help);
    if !desc.is_empty() {
        body.push_str(&format!("\n{desc}\n"));
    }
    if !c.aliases.is_empty() {
        body.push_str(&format!(
            "\nAliases: {}\n",
            c.aliases
                .iter()
                .map(|a| format!("/{a}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !examples.is_empty() {
        body.push_str("\nExamples:\n");
        for e in examples {
            body.push_str(&format!("  {e}\n"));
        }
    }
    Some(Page {
        id: format!("cmd-{}", c.name),
        title: format!("/{}", c.name),
        body,
        related: related.iter().map(|s| s.to_string()).collect(),
    })
}

/// How a related id reads in the modal ("/help login desktop").
pub fn link_label(id: &str) -> String {
    format!(
        "/help {}",
        id.strip_prefix("cmd-").unwrap_or(id).replace('-', " ")
    )
}

/// Every name `/help` accepts (topics + commands), for "did you mean".
pub fn all_names(registry: &Registry) -> Vec<String> {
    let mut v: Vec<String> = TOPICS.iter().map(|t| t.id.to_string()).collect();
    v.extend(registry.commands.iter().map(|c| c.name.to_string()));
    v
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within a small edit distance (or a unique prefix match).
pub fn suggest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let input = input.to_lowercase();
    let cands: Vec<&str> = candidates.into_iter().collect();
    let prefixed: Vec<&&str> = cands.iter().filter(|c| c.starts_with(&input)).collect();
    if !input.is_empty() && prefixed.len() == 1 {
        return Some(prefixed[0].to_string());
    }
    cands
        .iter()
        .map(|c| (levenshtein(&input, c), *c))
        .filter(|(d, c)| *d <= 2.max(c.len() / 3) && *d < c.len())
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c.to_string())
}

/// A fix / pointer for an error message, if we recognise it.
pub fn hint_for_error(msg: &str) -> Option<&'static str> {
    let m = msg.to_lowercase();
    let rules: &[(&str, &str)] = &[
        (
            "no longer supported",
            "Update the Grok Bot app or set GROKBOT_TUI_CURSOR_CLIENT_VERSION. See /help troubleshooting.",
        ),
        (
            "app data was not found",
            "Install the Grok Bot desktop app. See /help login desktop.",
        ),
        (
            "no saved session",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "no saved cursor accounts",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "no cursor-accounts",
            "Open the Grok Bot app and sign in, then /login desktop.",
        ),
        (
            "secret-tool",
            "Install libsecret-tools (provides secret-tool). See /help login desktop.",
        ),
        (
            "keychain did not return",
            "Unlock your keyring, then /login desktop. See /help login desktop.",
        ),
        ("several cursor accounts", "See /help login desktop."),
        ("several saved sessions", "See /help login desktop."),
        (
            "could not decrypt",
            "Sign out and back in to the Grok Bot app. See /help login desktop.",
        ),
        (
            "not allowed",
            "Only Cursor hosts are allowed. See /help login gateway.",
        ),
        ("401", "See /help troubleshooting (401)."),
        (
            "dashboard api key",
            "Use /login desktop instead. See /help login cursor.",
        ),
    ];
    rules.iter().find(|(k, _)| m.contains(k)).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::registry;

    #[test]
    fn every_topic_and_command_renders_with_resolvable_related() {
        let r = registry();
        for name in all_names(&r) {
            let p = page(&r, &name).unwrap_or_else(|| panic!("no page for {name}"));
            assert!(!p.body.trim().is_empty(), "{name}");
            assert!(!p.related.is_empty(), "{name} has no related links");
            for rel in &p.related {
                assert!(page(&r, rel).is_some(), "{name}: dangling related {rel}");
            }
        }
    }

    #[test]
    fn normalize_and_lookup() {
        let r = registry();
        assert_eq!(page(&r, "login desktop").unwrap().id, "login-desktop");
        assert_eq!(page(&r, "theme files").unwrap().id, "theme-files");
        assert!(
            page(&r, "theme-files")
                .unwrap()
                .body
                .contains("GROKBOT_TUI_NO_THEME_DOWNLOAD")
        );
        assert_eq!(page(&r, "/bot").unwrap().id, "cmd-bot");
        assert_eq!(page(&r, "").unwrap().id, "overview");
        assert!(page(&r, "nope").is_none());
        assert_eq!(link_label("login-desktop"), "/help login desktop");
        assert_eq!(link_label("cmd-bot"), "/help bot");
    }

    #[test]
    fn suggestions_and_hints() {
        assert_eq!(
            suggest("lgoin", ["login", "logout", "help"]).as_deref(),
            Some("login")
        );
        assert_eq!(
            suggest("hist", ["history", "help"]).as_deref(),
            Some("history")
        );
        assert_eq!(suggest("zzzzzz", ["login", "help"]), None);
        assert!(
            hint_for_error(
                "EnsureSandBox failed: 401 This version of Grok Bot is no longer supported"
            )
            .unwrap()
            .contains("GROKBOT_TUI_CURSOR_CLIENT_VERSION")
        );
        assert!(
            hint_for_error("Could not run secret-tool")
                .unwrap()
                .contains("libsecret")
        );
        assert!(hint_for_error("all good").is_none());
    }

    #[test]
    fn keyword_search_matches_topics_and_commands() {
        let r = registry();
        let ids = |q: &str| search(&r, q).into_iter().map(|h| h.id).collect::<Vec<_>>();
        assert!(ids("keyring").contains(&"login-desktop".to_string()));
        assert!(ids("GROKBOT_TUI_CURSOR_CLIENT_VERSION").contains(&"env".to_string()));
        assert!(ids("examples rename").contains(&"cmd-account".to_string()));
        assert_eq!(ids("prefs")[0], "prefs", "title match ranks first");
        assert!(ids("secret-tool libsecret").iter().all(|i| i != "keys"));
        assert!(ids("zzqqxx").is_empty());
        assert!(ids("").is_empty());
        let hit = &search(&r, "keyring")[0];
        assert!(hit.snippet.to_lowercase().contains("keyring"));
        // page(): exact first, else search with results, explicit search always.
        assert_eq!(page(&r, "keys").unwrap().id, "keys");
        let p = page(&r, "keyring").unwrap();
        assert_eq!(p.id, "search:keyring");
        assert!(p.related.iter().all(|id| page(&r, id).is_some()));
        assert!(page(&r, "zzqqxx").is_none());
        assert!(
            page(&r, "search zzqqxx")
                .unwrap()
                .body
                .contains("No help pages")
        );
        assert_eq!(page(&r, "search:keyring").unwrap().id, "search:keyring");
    }

    #[test]
    fn desktop_is_recommended() {
        let p = topic("login").unwrap();
        assert!(p.body.contains("RECOMMENDED"));
        assert!(topic("overview").unwrap().body.contains("/login desktop"));
    }
}
