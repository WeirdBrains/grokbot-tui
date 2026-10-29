//! Application state and input handling.
//!
//! The UI thread never waits on the network: every gateway call runs on a tokio task and
//! reports back as an [`Event`] over a channel, drained by [`App::pump`] / [`App::tick`].
//! A generation counter discards results from before an account switch or logout.
use crate::api::{
    ApiClient, Bot, GatewayApi, Message, MockClient, Role, Session, mock_ensure_sandbox,
};
use crate::commands::{self, Registry};
use crate::config::{
    CLIENT_TYPE_ENV, CLIENT_VERSION_ENV, CURSOR_API_ENV, Config, GATEWAY_HEADERS_ENV,
    GATEWAY_URL_ENV, SANDBOX_NAMESPACE_ENV, TOKEN_ENV,
};
use crate::storage::{Account, LoginMethod, Store, StoredLogin};
use anyhow::Result;
use grokbot_api::{self as gw, ClientOptions, Delivery, Policy};
use ratatui::layout::Rect;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Max wait of the UI event loop between redraws. Purely local: gateway polling is
/// gated separately by `poll.next` (POLL_IDLE / POLL_FAST / backoff), so a faster UI
/// tick never causes extra API calls.
pub const UI_TICK: Duration = Duration::from_millis(100);
/// A second Ctrl-C within this long quits.
pub const QUIT_CONFIRM: Duration = Duration::from_millis(1500);
/// How often the visible feed is re-read from local SQLite (cheap, no network).
pub const DB_REFRESH: Duration = Duration::from_millis(100);
/// Retry delays for restoring a saved login after a non-auth failure (network etc.).
const LOGIN_RETRY_MIN: Duration = Duration::from_secs(5);
const LOGIN_RETRY_MAX: Duration = Duration::from_secs(60);
/// Gateway poll interval right after a send / while waiting for a reply.
pub const POLL_FAST: Duration = Duration::from_secs(1);
/// Ceiling while waiting for a reply.
pub const POLL_AWAIT_MAX: Duration = Duration::from_secs(8);
/// Gateway poll interval while a bot is open (backs off when nothing changes).
pub const POLL_IDLE: Duration = Duration::from_secs(1);
/// Ceiling when idle (also used after errors).
pub const POLL_IDLE_MAX: Duration = Duration::from_secs(15);
/// Stop showing "typing" after this long without a reply.
pub const AWAIT_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_LIMIT: u32 = 100;
const HISTORY_LIMIT: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    System,
    Error,
    User,
    Bot,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    /// `/login gateway`: plain gateway URL entry.
    GatewayUrl,
    /// `/login gateway`: masked gateway token entry.
    GatewayToken,
    /// `/login cursor`: masked Cursor access token entry.
    CursorToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Sidebar,
}

/// Where a login attempt came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginSource {
    /// Interactive `/login`; saved on success.
    Interactive,
    /// The account's stored credential.
    Stored,
    /// `GROKBOT_TUI_TOKEN` (never saved).
    Env,
}

type LoginOk = (Arc<dyn ApiClient>, Session, Vec<Bot>);

/// Outcome of an opt-in desktop read (descriptor session or sand Cursor token).
pub type DesktopRead = (gw::desktop::DesktopCredential, Option<String>);

/// Reads the desktop app's saved credential for an optional entry.
/// Only invoked by `/login desktop` and the 401 re-read for desktop-method accounts.
pub type DesktopReader =
    Arc<dyn Fn(Option<&str>, &Policy) -> gw::Result<DesktopRead> + Send + Sync>;

/// Lists the desktop app's saved sessions/accounts as (label, choice) pairs, for the
/// "pick one" modal when `/login desktop` finds several. Labels only; no secrets.
pub type DesktopLister = Arc<dyn Fn() -> gw::Result<Vec<(String, String)>> + Send + Sync>;

pub fn real_desktop_lister() -> DesktopLister {
    Arc::new(|| {
        let platform = gw::desktop::Platform::current().ok_or_else(|| gw::Error::Desktop {
            code: "UNSUPPORTED_PLATFORM".into(),
            message: "unsupported platform".into(),
        })?;
        let cfg = gw::desktop::DesktopConfig::new(platform);
        if let Ok(entries) = cfg.entries()
            && entries.len() > 1
        {
            return Ok(entries
                .into_iter()
                .map(|e| (e.key.clone(), e.key))
                .collect());
        }
        Ok(cfg
            .sand_accounts()?
            .into_iter()
            .map(|a| {
                let label = if a.active {
                    format!("{} (active in app)", a.label)
                } else {
                    a.label
                };
                (label, a.key)
            })
            .collect())
    })
}

/// Text for a desktop-session error. grokbot_api describes `AMBIGUOUS_ENTRIES` without naming
/// any app's commands, so point the user at grokbot-tui's own: `/login desktop <name|number>`, and
/// `/account add` when several desktop Cursor accounts exist.
fn desktop_error_text(e: &gw::Error) -> String {
    match e {
        gw::Error::Desktop { code, message } if code == "AMBIGUOUS_ENTRIES" => {
            let what = message
                .split(" Specify which")
                .next()
                .unwrap_or(message)
                .trim_end();
            if message.contains("Cursor accounts") {
                format!(
                    "{what} Choose one with /login desktop <number> (tip: /account add <alias> first so each gets its own grokbot-tui account)."
                )
            } else {
                format!("{what} Choose one with /login desktop <name>.")
            }
        }
        _ => e.to_string(),
    }
}

/// The real reader: the platform's app-data dir + OS keychain (see `grokbot_api::desktop`).
pub fn real_desktop_reader() -> DesktopReader {
    Arc::new(|entry, policy| {
        let platform = gw::desktop::Platform::current().ok_or_else(|| gw::Error::Desktop {
            code: "UNSUPPORTED_PLATFORM".into(),
            message: "Reading the desktop session is supported on Linux, macOS and Windows only."
                .into(),
        })?;
        let cfg = gw::desktop::DesktopConfig::new(platform);
        let cred = cfg.load_credential(entry, policy)?;
        // Prefer the stable sand account key (so a number/`prefix` choice re-reads the same account).
        let stored = match &cred {
            gw::desktop::DesktopCredential::CursorToken(_) => cfg.sand_key_for(entry)?,
            gw::desktop::DesktopCredential::Session(_) => entry.map(str::to_string),
        };
        Ok((cred, stored))
    })
}

/// Fill EnsureSandBox client identity on `opts`.
/// Priority: env → prefs → detected desktop app version (when `detect`) → crate defaults.
/// Returns where `client_version` came from (for /whoami).
pub fn apply_client_identity(
    opts: &mut ClientOptions,
    config: &crate::config::Config,
    detect: bool,
) -> &'static str {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
    let source;
    if let Some(v) = env(CLIENT_VERSION_ENV) {
        opts.client_version = v;
        source = "env GROKBOT_TUI_CURSOR_CLIENT_VERSION";
    } else if let Some(v) = config.client_version.clone() {
        opts.client_version = v;
        source = "pref cursor.client_version";
    } else if detect
        && let Some(platform) = gw::desktop::Platform::current()
        && let Some(v) = gw::desktop::DesktopConfig::new(platform).detect_app_version()
    {
        opts.client_version = v;
        source = "detected desktop app";
    } else {
        opts.client_version = gw::DEFAULT_CLIENT_VERSION.into();
        source = "built-in default";
    }
    if let Some(v) = env(CLIENT_TYPE_ENV) {
        opts.client_type = v;
    } else if let Some(v) = config.client_type.clone() {
        opts.client_type = v;
    }
    if let Some(v) = env(SANDBOX_NAMESPACE_ENV) {
        opts.sandbox_namespace = v;
    } else if let Some(v) = config.sandbox_namespace.clone() {
        opts.sandbox_namespace = v;
    }
    source
}

/// The help modal: one page, scroll offset, selected related link and back stack.
pub struct HelpModal {
    pub page: crate::help::Page,
    pub scroll: u16,
    pub link: usize,
    pub back: Vec<String>,
    /// Search box text while typing a query in the modal (`/` to start).
    pub query: Option<String>,
}

/// Result of a `/shell` command.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellOutput {
    /// Combined stdout then stderr, capped to [`SHELL_MAX_LINES`] / [`SHELL_MAX_BYTES`].
    pub text: String,
    pub truncated: bool,
    /// `None` if killed by a signal.
    pub code: Option<i32>,
}

/// The `/shell` output popup.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellModal {
    pub id: u64,
    pub cmd: String,
    /// `None` while running.
    pub result: Option<std::result::Result<ShellOutput, String>>,
    pub scroll: u16,
}

impl ShellModal {
    pub fn running(&self) -> bool {
        self.result.is_none()
    }
    /// Body text: command line, output, then exit status.
    pub fn body(&self) -> String {
        let mut t = format!("$ {}\n", self.cmd);
        match &self.result {
            None => t.push_str("\nrunning…"),
            Some(Err(e)) => t.push_str(&format!("\n[shell error: {e}]")),
            Some(Ok(o)) => {
                if !o.text.is_empty() {
                    t.push_str(&o.text);
                    t.push('\n');
                }
                if o.truncated {
                    t.push_str(&format!(
                        "… output truncated (first {SHELL_MAX_LINES} lines / {} KB)\n",
                        SHELL_MAX_BYTES / 1024
                    ));
                }
                t.push_str(&format!("\n[{}]", shell_status(o.code)));
            }
        }
        t
    }
    /// Whether the finished command failed (for styling).
    pub fn failed(&self) -> bool {
        !matches!(
            &self.result,
            None | Some(Ok(ShellOutput { code: Some(0), .. }))
        )
    }
}

pub fn shell_status(code: Option<i32>) -> String {
    match code {
        Some(0) => "exit 0".into(),
        Some(127) => "exit 127 (command not found?)".into(),
        Some(c) => format!("exit {c}"),
        None => "killed by signal".into(),
    }
}

pub const SHELL_MAX_LINES: usize = 500;
pub const SHELL_MAX_BYTES: usize = 64 * 1024;
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(120);

/// Run `cmd` with `$SHELL -c` (fallback `sh`), capturing stdout+stderr and exit code.
pub async fn run_shell(cmd: &str) -> std::result::Result<ShellOutput, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "sh".into());
    let child = tokio::process::Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not start {shell}: {e}"))?;
    let out = tokio::time::timeout(SHELL_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {}s (killed)", SHELL_TIMEOUT.as_secs()))?
        .map_err(|e| format!("failed: {e}"))?;
    let mut all = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.is_empty() {
        if !all.is_empty() && !all.ends_with('\n') {
            all.push('\n');
        }
        all.push_str(&err);
    }
    let (text, truncated) = cap_output(&grokbot_api::sanitize_text(&all));
    Ok(ShellOutput {
        text,
        truncated,
        code: out.status.code(),
    })
}

/// Keep the first [`SHELL_MAX_LINES`] lines and at most [`SHELL_MAX_BYTES`] bytes.
pub fn cap_output(s: &str) -> (String, bool) {
    let mut out = String::new();
    let mut truncated = false;
    for (n, line) in s.lines().enumerate() {
        if n >= SHELL_MAX_LINES || out.len() + line.len() + 1 > SHELL_MAX_BYTES {
            truncated = true;
            break;
        }
        if n > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    (out, truncated)
}

pub enum Event {
    Login {
        epoch: u64,
        method: LoginMethod,
        source: LoginSource,
        cursor_token: Option<String>,
        desktop_entry: Option<String>,
        result: std::result::Result<LoginOk, String>,
    },
    Bots {
        epoch: u64,
        announce: bool,
        result: gw::Result<Vec<Bot>>,
    },
    Transcript {
        epoch: u64,
        bot: String,
        /// True when this was a full-history fetch (bot select / /history).
        full: bool,
        result: gw::Result<Vec<Message>>,
    },
    Sent {
        epoch: u64,
        bot: String,
        nonce: String,
        result: gw::Result<gw::SendReceipt>,
    },
    /// `/login desktop` found several sessions/accounts: let the user pick one.
    DesktopChoices {
        epoch: u64,
        choices: Vec<(String, String)>,
    },
    /// A download of the shipped default theme finished (see `theme_editor`).
    ThemeDownload {
        purpose: crate::theme_editor::DownloadFor,
        url: String,
        result: std::result::Result<String, String>,
    },
    /// `/shell` finished (shown in the shell modal; never chat or gateway).
    Shell {
        id: u64,
        cmd: String,
        result: std::result::Result<ShellOutput, String>,
    },
    /// `/bot new` finished.
    BotCreated { epoch: u64, result: gw::Result<Bot> },
    /// `/bot delete` finished.
    BotDeleted {
        epoch: u64,
        id: String,
        name: String,
        result: gw::Result<()>,
    },
    /// The image viewer's image finished loading.
    ImageLoaded {
        id: u64,
        result: crate::imageview::Loaded,
    },
    /// A `/download` transfer finished.
    Downloaded {
        url: String,
        dest: std::path::PathBuf,
        result: std::result::Result<u64, String>,
    },
    /// An attachment upload finished.
    Uploaded {
        id: u64,
        result: gw::Result<gw::UploadedAttachment>,
    },
    /// Presigned reads for attachments came back.
    AttachResolved {
        paths: Vec<String>,
        then: crate::attach::Then,
        result: gw::Result<Vec<gw::AttachmentRead>>,
    },
    /// `promptAcceptanceStatus` answer for a send whose outcome was unknown.
    Acceptance {
        epoch: u64,
        bot: String,
        nonce: String,
        attempt: u32,
        result: gw::Result<gw::AcceptanceStatus>,
    },
}

/// Notice shown while asking the gateway whether an unconfirmed send landed.
const CHECKING_NOTICE: &str = "No receipt yet; checking delivery status with the gateway…";
/// How many times to re-ask while the gateway says "pending".
const ACCEPTANCE_MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone)]
pub struct PollState {
    pub next: Instant,
    pub interval: Duration,
    pub inflight: bool,
    pub limit: u32,
}

pub struct App {
    pub store: Store,
    pub config: Config,
    pub rt: tokio::runtime::Handle,
    pub registry: Registry,
    pub mock: bool,
    pub opts: ClientOptions,
    pub api: Option<Arc<dyn ApiClient>>,
    /// e.g. "cursor · box.us12.cursorvm.com"; None when logged out.
    pub user_label: Option<String>,
    pub login_method: Option<LoginMethod>,
    pub login_source: Option<LoginSource>,
    pub connecting: bool,
    /// Every bot from the gateway (including hidden ones).
    pub all_bots: Vec<Bot>,
    /// Bots shown in the sidebar.
    pub bots: Vec<Bot>,
    pub active: Option<usize>,
    pub sidebar_cursor: usize,
    /// Chat feed per bot id. Only ever rebuilt from SQLite by `reload_bot_log`;
    /// nothing else writes it.
    pub logs: HashMap<String, Vec<Entry>>,
    /// Local reactions of the active bot, cached: (bot id, [(msg_key, emoji)]).
    pub reaction_cache: Option<(String, Vec<(String, String)>)>,
    /// Transient notices (command echoes, info, errors, delivery warnings) keyed by
    /// bot id ("" when no bot is active). Never stored, never mixed into the feed;
    /// rendered in their own section below it.
    pub notices: HashMap<String, Vec<Entry>>,
    pub input: String,
    /// Byte offset of the cursor in `input`.
    pub input_cursor: usize,
    /// First visible row of the input box (when over 10 rows).
    pub input_scroll: usize,
    pub mode: InputMode,
    pub focus: Focus,
    pub scroll: u16,
    /// Stick the viewport to the newest content (distance-from-bottom scroll == 0).
    pub follow: bool,
    /// Absolute top-row offset to keep under the eyes when [`follow`] is false.
    pub pinned_off: Option<u16>,
    /// New feed/notice items arrived while scrolled up.
    pub unread: u32,
    /// Last laid-out body height in wrapped rows (for anchoring).
    pub body_rows: u16,
    /// Last body viewport height in rows.
    pub body_inner_h: u16,
    /// Clickable "↓ N new" badge area (set by the renderer).
    pub unread_area: Rect,
    /// Links on screen in the last frame (clicks, OSC 8 hyperlinks).
    pub link_hits: Vec<crate::ui::LinkHit>,
    /// React buttons on screen (url = message key).
    pub react_hits: Vec<crate::ui::LinkHit>,
    /// The emoji picker modal.
    pub react_picker: Option<crate::emojipicker::EmojiPicker>,
    /// The download modal (`[⤓]`, `/download`, vim `d`).
    pub download: Option<crate::download::DownloadModal>,
    /// Download buttons on screen (url = what to download).
    pub download_hits: Vec<crate::ui::LinkHit>,
    /// Staged attachments, the queued prompt, resolved reads.
    pub attach: crate::attach::Attachments,
    /// Chip hit areas above the input (`v<n>` preview, `x<n>` remove).
    pub chip_hits: Vec<crate::ui::LinkHit>,
    /// Per bot: (message key, attachments) for messages that carry files, in feed order.
    pub msg_attachments: HashMap<String, Vec<(String, Vec<crate::api::Attachment>)>>,
    /// The image viewer modal.
    pub image_view: Option<crate::imageview::ImageView>,
    pub image_load_seq: u64,
    /// Graphics protocol for images (halfblocks until the terminal is queried).
    pub image_picker: ratatui_image::picker::Picker,
    /// Clear the terminal before the next draw (image ghosts after the viewer closes).
    pub needs_clear: bool,
    /// Local notification toasts + settings (1.10.0).
    pub notices_board: crate::notify::NoticeBoard,
    /// Rules deciding toast / mute / drop.
    pub notify_rules: crate::notify_rules::Ruleset,
    /// Top menu bar.
    pub menu: crate::menubar::MenuBar,
    /// Hover tooltips.
    pub tooltip: crate::tooltip::TooltipState,
    /// Kitty-style keyboard enhancement is active.
    pub keyboard_enhancement: bool,
    /// One-time Apple Terminal newline hint already shown this process.
    pub apple_newline_hinted: bool,
    pub sidebar_area: Rect,
    /// The account switcher in the header (click opens the account picker).
    pub account_area: Rect,
    /// The dark / light / system choices in the header (click selects one).
    pub theme_areas: Vec<(Rect, crate::theme::Mode)>,
    /// Terminal background detected at startup, used by the "system" theme mode.
    pub detected_variant: Option<crate::theme::Variant>,
    /// The theme in use: loaded from `<data dir>/themes/<ui.theme>.toml`, or plain terminal
    /// colors when that file is missing or broken (see `theme_problem`).
    pub theme: Arc<crate::theme::Theme>,
    /// Why the selected theme couldn't be loaded ("missing" / "broken"), if it couldn't.
    pub theme_problem: Option<&'static str>,
    /// Where shipped themes are downloaded from (`<base><name>.toml`): the repo's `themes/`
    /// at this build's tag, or GROKBOT_TUI_THEME_URL. None = never (mock mode,
    /// GROKBOT_TUI_NO_THEME_DOWNLOAD).
    pub theme_download_base: Option<String>,
    /// A theme download is in flight.
    pub theme_downloading: bool,
    /// `--theme <name>`: the theme for this run only (never saved; survives config reloads
    /// until a theme is picked in the app).
    pub theme_override: Option<String>,
    /// When a lone Ctrl-C asked "press again to quit" (a second one before
    /// [`QUIT_CONFIRM`] quits).
    pub quit_armed: Option<Instant>,
    /// Input selection anchor (byte offset); the selection runs to the cursor.
    pub input_anchor: Option<usize>,
    /// What the last Ctrl-C cleared from the input (Ctrl-Z restores it).
    pub cleared_input: Option<(String, usize)>,
    /// Last mouse position reported by the terminal (for "the pane under the mouse").
    pub mouse_pos: Option<(u16, u16)>,
    /// Newest-first prompts for the active account (Up/Down, Ctrl-R).
    pub prompt_history: Vec<String>,
    /// Bot and time for each [`prompt_history`] entry (same order).
    pub prompt_meta: Vec<crate::storage::PromptEntry>,
    /// The last prompt-history load error, so it is reported once, not on every key.
    history_error: Option<String>,
    /// Position in [`prompt_history`] while browsing (`None` = at the draft).
    pub history_idx: Option<usize>,
    /// The input text before browsing history.
    pub history_draft: Option<String>,
    /// Ctrl-R fuzzy history search overlay.
    pub history_search: Option<crate::history::Search>,
    /// Tab-completion popup for slash commands.
    pub completion: Option<crate::complete::Popup>,
    /// In-app selection in the chat body.
    pub body_sel: Option<crate::select::Selection>,
    /// Plain text of the feed matching the current layout (for selection offsets).
    pub body_plain: String,
    /// Wrapped-row scroll offset of the chat body (for click→offset mapping).
    pub body_row_off: usize,
    pub body_wrap: usize,
    /// Vim-like editor mode (only when `ui.keys = vim`).
    pub vim_mode: crate::keymap::Mode,
    /// Loaded keymap (vim scheme).
    pub keymap: crate::keymap::Keymap,
    /// Pending `:` command line (without the leading colon).
    pub vim_command: Option<String>,
    /// The chat body and the input box, as last drawn.
    pub body_area: Rect,
    pub input_area: Rect,
    /// The chat feed's layout, reused while nothing it depends on changes.
    pub feed_cache: Option<crate::ui::FeedCache>,
    /// While the theme picker is open: the theme (and its problem) to put back on Esc.
    pub theme_preview: Option<(Arc<crate::theme::Theme>, Option<&'static str>)>,
    /// The `‹theme› ▾` switcher in the header (click opens the theme picker).
    pub theme_switch_area: Rect,
    /// The full-screen theme editor (Shift-F5), when open.
    pub editor: Option<crate::theme_editor::ThemeEditor>,
    pub pending_remove: Option<String>,
    pub login_url_draft: Option<String>,
    /// Bots with a sent prompt whose reply hasn't shown up yet.
    pub awaiting: HashMap<String, Instant>,
    pub poll: PollState,
    last_poll_error: Option<String>,
    pub epoch: u64,
    pub inflight: usize,
    pub desktop_reader: DesktopReader,
    pub desktop_lister: DesktopLister,
    /// Open help modal (never part of chat history).
    pub help: Option<HelpModal>,
    /// `/shell` output popup.
    pub shell: Option<ShellModal>,
    shell_seq: u64,
    /// Open selection modal (bots, accounts, …).
    pub picker: Option<crate::picker::Picker>,
    /// Where `opts.client_version` came from.
    pub client_version_source: &'static str,
    /// Set after one automatic desktop re-read on 401 so a bad session can't loop.
    desktop_reread_tried: bool,
    /// Saved login restore failed for a non-auth reason: retry at this time.
    login_retry: Option<(Instant, Duration)>,
    /// Last local DB refresh of the feed, and the (count, max id) stamp it saw.
    db_refresh: (Instant, Option<(i64, i64, i64)>),
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
    pub should_quit: bool,
    /// Whether the TUI captures the mouse. Off lets the terminal select/copy natively.
    pub mouse_capture: bool,
    /// Text the main loop should put on the clipboard (OSC 52), then clear.
    pub pending_clipboard: Option<String>,
}

pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Whether a login error string means the credential itself was rejected.
fn is_auth_error(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("401") || e.contains("403") || e.contains("417") || e.contains("unauthorized")
}

async fn login_task(
    mock: bool,
    opts: ClientOptions,
    session: Option<Session>,
    cursor_token: Option<String>,
) -> std::result::Result<LoginOk, String> {
    let es = |e: gw::Error| e.to_string();
    let (client, session): (Arc<dyn ApiClient>, Session) = if mock {
        let s = match (session, &cursor_token) {
            (Some(s), _) => s,
            (None, Some(t)) => mock_ensure_sandbox(t).map_err(es)?,
            (None, None) => return Err("no credentials".into()),
        };
        (Arc::new(MockClient::new(&s.gateway_token)), s)
    } else {
        let c = match (session, &cursor_token) {
            (Some(s), Some(t)) => gw::Client::with_cursor_token(s, t.clone(), opts),
            (Some(s), None) => gw::Client::new(s, opts),
            (None, Some(t)) => gw::Client::connect_cursor(t, opts).await,
            (None, None) => return Err("no credentials".into()),
        }
        .map_err(es)?;
        let s = c.session();
        (Arc::new(GatewayApi::new(c)), s)
    };
    // There is no whoami call; listAgents validates the credential.
    let bots = client.list_bots().await.map_err(es)?;
    let session = client.take_refreshed_session().unwrap_or(session);
    Ok((client, session, bots))
}

impl App {
    pub fn new(store: Store, rt: tokio::runtime::Handle, mock: bool) -> Result<Self> {
        let config = store.load_config()?;
        let backend_base = std::env::var(CURSOR_API_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| gw::DEFAULT_BACKEND_BASE.into());
        let mut opts = ClientOptions {
            policy: Policy::from_env(),
            backend_base,
            max_response_bytes: crate::config::max_response_bytes(),
            ..Default::default()
        };
        let client_version_source = apply_client_identity(&mut opts, &config, true);
        let (tx, rx) = mpsc::unbounded_channel();
        let mouse_capture = config.mouse_capture;
        let keymap_path = crate::keymap::user_path(&store.dir);
        let (keymap, _keymap_err) = crate::keymap::load(&keymap_path, None);
        Ok(Self {
            store,
            config,
            rt,
            registry: commands::registry(),
            mock,
            opts,
            api: None,
            user_label: None,
            login_method: None,
            login_source: None,
            connecting: false,
            all_bots: vec![],
            bots: vec![],
            active: None,
            sidebar_cursor: 0,
            logs: HashMap::new(),
            reaction_cache: None,
            notices: HashMap::new(),
            input: String::new(),
            input_cursor: 0,
            input_scroll: 0,
            mode: InputMode::Normal,
            focus: Focus::Input,
            scroll: 0,
            follow: true,
            pinned_off: None,
            unread: 0,
            body_rows: 0,
            body_inner_h: 0,
            unread_area: Rect::default(),
            link_hits: vec![],
            react_hits: vec![],
            react_picker: None,
            download: None,
            download_hits: vec![],
            attach: Default::default(),
            chip_hits: vec![],
            msg_attachments: HashMap::new(),
            image_view: None,
            image_load_seq: 0,
            image_picker: ratatui_image::picker::Picker::halfblocks(),
            needs_clear: false,
            notices_board: crate::notify::NoticeBoard::new(),
            notify_rules: crate::notify_rules::Ruleset::default(),
            menu: crate::menubar::MenuBar::default(),
            tooltip: crate::tooltip::TooltipState::default(),
            keyboard_enhancement: false,
            apple_newline_hinted: false,
            sidebar_area: Rect::default(),
            account_area: Rect::default(),
            theme_areas: Vec::new(),
            detected_variant: None,
            theme: Arc::new(crate::theme::Theme::unstyled(crate::themes::DEFAULT_ID)),
            theme_problem: None,
            theme_download_base: (!mock
                && !crate::themes::opted_out(
                    std::env::var(crate::themes::NO_DOWNLOAD_ENV)
                        .ok()
                        .as_deref(),
                ))
            .then(crate::themes::download_base_now),
            theme_downloading: false,
            theme_override: None,
            theme_preview: None,
            feed_cache: None,
            quit_armed: None,
            input_anchor: None,
            cleared_input: None,
            mouse_pos: None,
            prompt_history: Vec::new(),
            prompt_meta: Vec::new(),
            history_error: None,
            history_idx: None,
            history_draft: None,
            history_search: None,
            completion: None,
            body_sel: None,
            body_plain: String::new(),
            body_row_off: 0,
            body_wrap: 1,
            vim_mode: crate::keymap::Mode::Insert,
            keymap,
            vim_command: None,
            body_area: Rect::default(),
            input_area: Rect::default(),
            theme_switch_area: Rect::default(),
            editor: None,
            pending_remove: None,
            login_url_draft: None,
            awaiting: HashMap::new(),
            poll: PollState {
                next: Instant::now(),
                interval: POLL_IDLE,
                inflight: false,
                limit: POLL_LIMIT,
            },
            last_poll_error: None,
            epoch: 0,
            inflight: 0,
            desktop_reader: real_desktop_reader(),
            desktop_lister: real_desktop_lister(),
            help: None,
            shell: None,
            shell_seq: 0,
            picker: None,
            client_version_source,
            desktop_reread_tried: false,
            login_retry: None,
            db_refresh: (Instant::now(), None),
            tx,
            rx,
            should_quit: false,
            mouse_capture,
            pending_clipboard: None,
        })
    }

    pub fn account(&self) -> &Account {
        self.store.account()
    }

    // ------------------------------------------------------------ tasks & events

    /// A sender for tasks that report back themselves (they must count in `inflight`).
    pub(crate) fn event_sender(&self) -> mpsc::UnboundedSender<Event> {
        self.tx.clone()
    }

    pub(crate) fn spawn<F: Future<Output = Event> + Send + 'static>(&mut self, fut: F) {
        self.inflight += 1;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(fut.await);
        });
    }

    /// Apply all finished task results without blocking.
    pub fn pump(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.handle(ev);
        }
    }

    /// Called every UI loop iteration: apply results and poll the active bot when due.
    pub fn tick(&mut self) {
        self.notices_board.tick(std::time::Instant::now());
        self.pump();
        let now = Instant::now();
        if let Some((at, _)) = self.login_retry
            && now >= at
            && !self.connecting
            && self.api.is_none()
        {
            self.connect();
        }
        if now.duration_since(self.db_refresh.0) >= DB_REFRESH {
            self.refresh_feed_from_db();
        }
        self.awaiting
            .retain(|_, t| now.duration_since(*t) < AWAIT_TIMEOUT);
        let (Some(api), Some((bot, session, team))) = (self.api.clone(), self.active_bot_route())
        else {
            return;
        };
        if self.poll.inflight || now < self.poll.next {
            return;
        }
        if team && session.is_none() {
            // Avoid spamming: one error when selecting the bot is enough; skip polls.
            self.poll.next = now + POLL_IDLE_MAX;
            return;
        }
        self.poll.inflight = true;
        let (epoch, limit) = (self.epoch, self.poll.limit);
        self.poll.limit = POLL_LIMIT;
        self.spawn(async move {
            api.bind_viewer_session(&bot, session.as_deref());
            let result = api.transcript(&bot, limit).await;
            Event::Transcript {
                epoch,
                bot,
                full: false,
                result,
            }
        });
    }

    /// Re-read the active bot's feed from SQLite if it changed (local only, no network).
    pub fn refresh_feed_from_db(&mut self) {
        self.db_refresh.0 = Instant::now();
        let Some(bot) = self.active_bot().map(|b| b.id.clone()) else {
            return;
        };
        let stamp = self.store.message_stamp(&bot).ok();
        if stamp != self.db_refresh.1 {
            self.db_refresh.1 = stamp;
            self.reload_bot_log(&bot);
        }
    }

    /// Poll the active bot on the next tick.
    pub fn poll_now(&mut self) {
        self.poll.next = Instant::now();
    }

    /// Block (tests) until all in-flight tasks finish or `timeout` passes.
    #[cfg(test)]
    pub fn wait_idle(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.inflight > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let rt = self.rt.clone();
            match rt.block_on(async { tokio::time::timeout(left, self.rx.recv()).await }) {
                Ok(Some(ev)) => self.handle(ev),
                _ => break,
            }
        }
    }

    pub fn handle(&mut self, ev: Event) {
        self.inflight = self.inflight.saturating_sub(1);
        match ev {
            Event::Login {
                epoch,
                method,
                source,
                cursor_token,
                desktop_entry,
                result,
            } => {
                if epoch != self.epoch {
                    return;
                }
                self.connecting = false;
                match result {
                    Ok((client, session, bots)) => {
                        self.login_retry = None;
                        let label = match source {
                            LoginSource::Env => {
                                format!("{TOKEN_ENV} · {}", host_of(&session.gateway_url))
                            }
                            _ => format!("{} · {}", method.as_str(), host_of(&session.gateway_url)),
                        };
                        self.api = Some(client);
                        self.user_label = Some(label.clone());
                        self.login_method = Some(method);
                        self.login_source = Some(source);
                        let login = StoredLogin {
                            method,
                            session,
                            cursor_token,
                            desktop_entry,
                        };
                        match source {
                            LoginSource::Interactive => match self.store.save_login(&login) {
                                Ok(()) => self.info(format!(
                                    "Logged in ({label}); credential saved for account '{}'.",
                                    self.account().alias
                                )),
                                Err(e) => self.error(format!(
                                    "Logged in, but could not save credential: {e:#}"
                                )),
                            },
                            LoginSource::Stored => {
                                if self
                                    .store
                                    .load_login()
                                    .ok()
                                    .flatten()
                                    .is_some_and(|l| l != login)
                                {
                                    let _ = self.store.save_login(&login);
                                }
                                self.info(format!("Logged in ({label})."));
                            }
                            LoginSource::Env => self
                                .info(format!("Logged in with {TOKEN_ENV} ({label}); not saved.")),
                        }
                        self.apply_bots(bots);
                        if let Some(n) = self.api.as_ref().and_then(|a| a.take_list_notice()) {
                            self.error(n);
                        }
                    }
                    Err(e) => {
                        let hint = crate::help::hint_for_error(&e)
                            .map(|h| format!(" → {h}"))
                            .unwrap_or_else(|| " → /help login".into());
                        match source {
                            LoginSource::Interactive => {
                                self.error(format!("Login failed: {e}{hint}"))
                            }
                            // The saved login is never deleted on failure; only /logout removes it.
                            LoginSource::Stored
                                if method == LoginMethod::Desktop
                                    && is_auth_error(&e)
                                    && !self.desktop_reread_tried =>
                            {
                                // Desktop account opted in to re-reading the app session on 401.
                                self.desktop_reread_tried = true;
                                self.login_method = Some(method);
                                self.info(
                                    "Saved gateway session expired; re-reading the desktop app session…"
                                        .into(),
                                );
                                self.start_desktop_login(desktop_entry, LoginSource::Stored);
                            }
                            LoginSource::Stored if !is_auth_error(&e) => {
                                let delay = self
                                    .login_retry
                                    .map(|(_, d)| (d * 2).min(LOGIN_RETRY_MAX))
                                    .unwrap_or(LOGIN_RETRY_MIN);
                                self.login_retry = Some((Instant::now() + delay, delay));
                                self.error(format!(
                                    "Could not restore saved login: {e}. It is kept; retrying in {}s.{hint}",
                                    delay.as_secs()
                                ));
                            }
                            LoginSource::Stored => self.error(format!(
                                "Saved login was rejected: {e}. It is kept until /logout; log in again with /login desktop.{hint}"
                            )),
                            LoginSource::Env => {
                                self.error(format!("{TOKEN_ENV} login failed: {e}{hint}"))
                            }
                        }
                    }
                }
            }
            Event::ImageLoaded { id, result } => self.on_image_loaded(id, result),
            Event::Uploaded { id, result } => self.on_uploaded(id, result),
            Event::AttachResolved {
                paths,
                then,
                result,
            } => self.on_attach_resolved(paths, then, result),
            Event::Downloaded { url, dest, result } => self.on_downloaded(url, dest, result),
            Event::Shell { id, cmd, result } => match &mut self.shell {
                Some(m) if m.id == id => {
                    m.result = Some(result);
                }
                _ => {
                    // Modal was closed while running: just note the outcome.
                    let how = match &result {
                        Ok(o) => shell_status(o.code),
                        Err(e) => format!("error: {e}"),
                    };
                    self.info(format!("/shell {cmd} finished ({how}); output was closed."));
                }
            },
            Event::BotCreated { epoch, result } => {
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(b) => {
                        let mut all = self.all_bots.clone();
                        if !all.iter().any(|x| x.id == b.id) {
                            all.push(b.clone());
                        }
                        self.apply_bots(all);
                        if let Some(i) = self.bots.iter().position(|x| x.id == b.id) {
                            self.select_bot(i);
                        }
                        self.info(format!("Created bot {} ({}).", b.name, b.id));
                        self.refresh_bots(false);
                    }
                    Err(e) => self.error(format!("Could not create bot: {e}")),
                }
            }
            Event::BotDeleted {
                epoch,
                id,
                name,
                result,
            } => {
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(()) => {
                        let all = self
                            .all_bots
                            .iter()
                            .filter(|b| b.id != id)
                            .cloned()
                            .collect();
                        self.apply_bots(all);
                        self.info(format!("Deleted bot {name}."));
                        self.refresh_bots(false);
                    }
                    Err(e) => self.error(format!("Could not delete bot {name}: {e}")),
                }
            }
            Event::Bots {
                epoch,
                announce,
                result,
            } => {
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(b) => {
                        self.apply_bots(b);
                        if let Some(n) = self.api.as_ref().and_then(|a| a.take_list_notice()) {
                            self.error(n);
                        }
                        if announce {
                            let list = self
                                .all_bots
                                .iter()
                                .map(|b| {
                                    let mut flags = vec![];
                                    if b.is_group {
                                        flags.push("group");
                                    }
                                    if b.team {
                                        flags.push("team");
                                    }
                                    if b.hidden {
                                        flags.push("hidden");
                                    }
                                    let f = if flags.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" [{}]", flags.join(", "))
                                    };
                                    format!("  {:<20} {}{f}  {}", b.name, b.id, b.description)
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.info(format!("Bots:\n{list}"));
                        }
                    }
                    Err(e) => {
                        self.error(format!("Could not list bots: {e}"));
                        if self.bots.is_empty()
                            && let Ok(cached) = self.store.bots()
                            && !cached.is_empty()
                        {
                            self.all_bots = cached.clone();
                            self.bots = cached;
                            self.info("Showing cached bots.".into());
                        }
                    }
                }
            }
            Event::Transcript {
                epoch,
                bot,
                full,
                result,
            } => {
                // Always clear the poll gate — even for stale epochs — so a mid-flight
                // account switch cannot leave polling permanently stuck.
                self.poll.inflight = false;
                if epoch != self.epoch {
                    return;
                }
                let waiting = self.awaiting.contains_key(&bot);
                match result {
                    Ok(msgs) => {
                        self.last_poll_error = None;
                        self.desktop_reread_tried = false;
                        let added = match self.store.merge_remote(&bot, &msgs, full) {
                            Ok(a) => a,
                            Err(e) => {
                                self.error(format!("Could not store transcript: {e:#}"));
                                vec![]
                            }
                        };
                        let got_reply = added.iter().any(|m| m.role == Role::Bot);
                        // The chat feed is always rebuilt from SQLite (single source of truth).
                        self.reload_bot_log(&bot);
                        if got_reply {
                            self.awaiting.remove(&bot);
                        }
                        let waiting = self.awaiting.contains_key(&bot);
                        self.poll.interval = if !added.is_empty() {
                            if waiting { POLL_FAST } else { POLL_IDLE }
                        } else if waiting {
                            (self.poll.interval * 2).clamp(POLL_FAST, POLL_AWAIT_MAX)
                        } else {
                            (self.poll.interval * 2).clamp(POLL_IDLE, POLL_IDLE_MAX)
                        };
                    }
                    Err(e) => {
                        if e.is_unauthorized() && self.try_desktop_reread() {
                            self.poll.next = Instant::now() + POLL_IDLE;
                            return;
                        }
                        let msg = if e.is_unauthorized() {
                            "Gateway rejected the session (401/417, usually expired). Run /login desktop (or your login method) again → /help troubleshooting".to_string()
                        } else {
                            format!("Could not refresh chat: {e}")
                        };
                        if self.last_poll_error.as_deref() != Some(&msg) {
                            self.error(msg.clone());
                        }
                        self.last_poll_error = Some(msg);
                        let floor = if waiting { POLL_FAST } else { POLL_IDLE };
                        self.poll.interval = (self.poll.interval * 2).clamp(floor, POLL_IDLE_MAX);
                    }
                }
                self.poll.next = Instant::now() + self.poll.interval;
            }
            Event::Sent {
                epoch,
                bot,
                nonce,
                result,
            } => {
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(r) if r.delivery == Delivery::Accepted => {
                        self.awaiting.insert(bot.clone(), Instant::now());
                        self.schedule_fast(&bot, Duration::from_millis(500));
                    }
                    Ok(_) => self.check_acceptance(bot, nonce, 1, Duration::ZERO),
                    Err(e) if e.delivery() == Delivery::Rejected => {
                        let _ = self.store.delete_pending(&bot, &nonce);
                        self.reload_bot_log(&bot);
                        self.push_to(
                            &bot,
                            Entry {
                                kind: EntryKind::Error,
                                text: format!("Not delivered (rejected): {e}"),
                            },
                        );
                    }
                    // Network / 5xx / team-session errors: show the error, then ask whether it landed.
                    Err(e) => {
                        self.push_to(
                            &bot,
                            Entry {
                                kind: EntryKind::Error,
                                text: format!("Send failed: {e}"),
                            },
                        );
                        self.check_acceptance(bot, nonce, 1, Duration::ZERO);
                    }
                }
            }
            Event::ThemeDownload {
                purpose,
                url,
                result,
            } => self.on_theme_download(purpose, &url, result),
            Event::DesktopChoices { epoch, choices } => {
                if epoch != self.epoch {
                    return;
                }
                self.connecting = false;
                let items = choices
                    .into_iter()
                    .map(|(label, value)| crate::picker::PickItem::new(label, value))
                    .collect();
                self.open_picker(
                    crate::picker::Picker::new(
                        "Several desktop accounts: pick one to log in",
                        items,
                        |app, it| {
                            app.info("Reading the chosen desktop account…".into());
                            app.start_desktop_login(Some(it.value), LoginSource::Interactive);
                        },
                    )
                    .on_cancel(|app| app.info("Desktop login cancelled.".into())),
                );
            }
            Event::Acceptance {
                epoch,
                bot,
                nonce,
                attempt,
                result,
            } => {
                if epoch != self.epoch {
                    return;
                }
                use gw::AcceptanceStatus as A;
                match result {
                    Ok(A::Pending) if attempt < ACCEPTANCE_MAX_ATTEMPTS => {
                        self.check_acceptance(bot, nonce, attempt + 1, Duration::from_millis(1500));
                        return;
                    }
                    _ => {}
                }
                self.drop_checking_notice(&bot);
                match result {
                    Ok(A::Accepted) => {
                        // Confirmed: no warning; the transcript will replace the placeholder.
                        self.awaiting.insert(bot.clone(), Instant::now());
                        self.schedule_fast(&bot, Duration::ZERO);
                    }
                    Ok(A::Rejected(code)) => {
                        let _ = self.store.delete_pending(&bot, &nonce);
                        self.reload_bot_log(&bot);
                        let code = code.map(|c| format!(": {c}")).unwrap_or_default();
                        self.push_to(
                            &bot,
                            Entry {
                                kind: EntryKind::Error,
                                text: format!("Not delivered (rejected by the gateway{code})."),
                            },
                        );
                    }
                    Ok(A::NotFound) => {
                        let _ = self.store.delete_pending(&bot, &nonce);
                        self.reload_bot_log(&bot);
                        self.push_to(
                            &bot,
                            Entry {
                                kind: EntryKind::Error,
                                text: "Not delivered: the gateway has no record of it. Safe to resend."
                                    .into(),
                            },
                        );
                    }
                    Ok(A::Pending) | Ok(A::Unknown) | Err(_) => {
                        self.push_to(&bot, Entry {
                            kind: EntryKind::Error,
                            text: "Delivery unknown: the gateway couldn't confirm it. Watching the transcript; don't resend until it does or doesn't appear.".into(),
                        });
                        self.awaiting.insert(bot.clone(), Instant::now());
                        self.schedule_fast(&bot, Duration::ZERO);
                    }
                }
            }
        }
        // Persist a session refreshed by a 401 re-exchange.
        if let Some(s) = self.api.as_ref().and_then(|a| a.take_refreshed_session())
            && self.login_source != Some(LoginSource::Env)
            && let Ok(Some(mut l)) = self.store.load_login()
        {
            l.session = s;
            if self.store.save_login(&l).is_ok() {
                self.info("Gateway session refreshed via Cursor token.".into());
            }
        }
    }

    /// Ask `promptAcceptanceStatus` whether the send with `nonce` landed.
    fn check_acceptance(&mut self, bot: String, nonce: String, attempt: u32, after: Duration) {
        if attempt == 1 {
            self.push_to(
                &bot,
                Entry {
                    kind: EntryKind::System,
                    text: CHECKING_NOTICE.into(),
                },
            );
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let session = self
            .bots
            .iter()
            .chain(self.all_bots.iter())
            .find(|b| b.id == bot)
            .and_then(|b| b.viewer_session_id.clone());
        let epoch = self.epoch;
        self.spawn(async move {
            if !after.is_zero() {
                tokio::time::sleep(after).await;
            }
            api.bind_viewer_session(&bot, session.as_deref());
            let result = api.acceptance(&bot, &nonce).await;
            Event::Acceptance {
                epoch,
                bot,
                nonce,
                attempt,
                result,
            }
        });
    }

    fn drop_checking_notice(&mut self, bot: &str) {
        if let Some(n) = self.notices.get_mut(bot) {
            n.retain(|e| e.text != CHECKING_NOTICE);
        }
    }

    fn schedule_fast(&mut self, bot: &str, after: Duration) {
        if self.active_bot().is_some_and(|b| b.id == bot) {
            self.poll.interval = POLL_FAST;
            self.poll.next = Instant::now() + after;
        }
    }

    // ------------------------------------------------------------ session lifecycle

    fn reset_session(&mut self) {
        self.epoch += 1;
        self.api = None;
        self.user_label = None;
        self.login_method = None;
        self.login_source = None;
        self.desktop_reread_tried = false;
        self.login_retry = None;
        self.db_refresh.1 = None;
        self.connecting = false;
        self.all_bots.clear();
        self.bots.clear();
        self.active = None;
        self.sidebar_cursor = 0;
        self.logs.clear();
        self.notices.clear();
        self.awaiting.clear();
        self.follow_bottom();
        self.poll = PollState {
            next: Instant::now(),
            interval: POLL_IDLE,
            inflight: false,
            limit: POLL_LIMIT,
        };
        self.last_poll_error = None;
    }

    pub fn reload_config(&mut self) -> Result<()> {
        let old_theme = self.config.theme.clone();
        self.config = self.store.load_config()?;
        if let Some(t) = &self.theme_override {
            self.config.theme = t.clone();
        }
        if self.config.theme != old_theme {
            self.load_theme();
        }
        Ok(())
    }

    pub fn switch_account(&mut self, alias: &str) -> Result<()> {
        self.store.use_account(alias)?;
        self.reset_session();
        self.reload_config()?;
        self.reload_history();
        self.info(format!("Switched to account '{}'.", self.account().alias));
        self.connect();
        Ok(())
    }

    pub fn init(&mut self) {
        self.reload_history();
        self.info(format!(
            "Welcome to grokbot-tui{}. Type /help for commands.",
            if self.mock { " (mock mode)" } else { "" }
        ));
        {
            let path = crate::notify_rules::user_path(&self.store.dir);
            let (rs, err) = crate::notify_rules::Ruleset::load(&path, None);
            self.notify_rules = rs;
            if let Some(e) = err {
                self.error(format!("Notify rules: {e} (kept defaults)."));
            }
        }
        if self.config.keys == crate::config::KeysScheme::Vim {
            self.vim_mode = crate::keymap::Mode::Insert;
            self.info(
                "Keys: vim (ui.keys=vim). Esc → NORMAL, i → INSERT, : → command. /keymap".into(),
            );
        }
        let path = crate::keymap::user_path(&self.store.dir);
        let (km, err) = crate::keymap::load(&path, Some(&self.keymap));
        self.keymap = km;
        if let Some(e) = err {
            self.error(format!("Keymap: {e} (kept previous / default)."));
        }
        self.connect();
    }

    /// Log in with `GROKBOT_TUI_TOKEN` (startup account only) or the stored credential.
    pub fn connect(&mut self) {
        let stored = self.store.load_login().ok().flatten();
        if self.store.is_startup_account()
            && let Ok(tok) = std::env::var(TOKEN_ENV)
            && !tok.trim().is_empty()
        {
            let url = std::env::var(GATEWAY_URL_ENV)
                .ok()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| stored.as_ref().map(|l| l.session.gateway_url.clone()));
            let Some(url) = url else {
                self.error(format!("{TOKEN_ENV} is set but no gateway URL: set {GATEWAY_URL_ENV} or log in once with /login gateway."));
                return;
            };
            let headers: BTreeMap<String, String> = match std::env::var(GATEWAY_HEADERS_ENV) {
                Ok(h) if !h.trim().is_empty() => {
                    match serde_json::from_str::<BTreeMap<String, String>>(&h) {
                        Ok(m) => m,
                        Err(e) => {
                            self.error(format!(
                                "{GATEWAY_HEADERS_ENV} must be a JSON object of strings: {e}"
                            ));
                            return;
                        }
                    }
                }
                _ => BTreeMap::new(),
            };
            let session = Session::new(url.trim(), tok.trim()).with_headers(headers);
            self.start_login(LoginMethod::Gateway, LoginSource::Env, Some(session), None);
        } else if let Some(l) = stored {
            // Desktop login often yields a Cursor sand token (not persisted). Re-read the
            // app so ListGrokBotTeamAgents can run and shared bots get a viewer sessionId.
            if l.method == LoginMethod::Desktop && l.cursor_token.is_none() {
                self.start_desktop_login(l.desktop_entry, LoginSource::Stored);
            } else {
                self.spawn_login(
                    l.method,
                    LoginSource::Stored,
                    Some(l.session),
                    l.cursor_token,
                    l.desktop_entry,
                );
            }
        } else {
            self.info(format!(
                "Account '{}' is not logged in. Recommended: sign in to the Grok Bot desktop app, then /login desktop. Other options: /help login · keys: F1",
                self.account().alias
            ));
        }
    }

    pub fn start_login(
        &mut self,
        method: LoginMethod,
        source: LoginSource,
        session: Option<Session>,
        cursor_token: Option<String>,
    ) {
        self.spawn_login(method, source, session, cursor_token, None);
    }

    fn spawn_login(
        &mut self,
        method: LoginMethod,
        source: LoginSource,
        session: Option<Session>,
        cursor_token: Option<String>,
        desktop_entry: Option<String>,
    ) {
        if let Some(s) = &session
            && !self.mock
            && let Err(e) = self.opts.policy.check_gateway(&s.gateway_url)
        {
            self.error(e);
            return;
        }
        self.connecting = true;
        let (epoch, mock, opts) = (self.epoch, self.mock, self.opts.clone());
        self.spawn(async move {
            let result = login_task(mock, opts, session, cursor_token.clone()).await;
            Event::Login {
                epoch,
                method,
                source,
                cursor_token,
                desktop_entry,
                result,
            }
        });
    }

    /// `/login desktop [entry]`: read the desktop app's saved session (opt-in, user-initiated).
    /// Prefers `gateway-descriptor.json`; falls back to decrypting `sand-secrets.json`'s
    /// `cursor-accounts` and exchanging the Cursor access token via EnsureSandBox (like
    /// `/login cursor`). Never reads the desktop session in any other code path.
    pub fn start_desktop_login(&mut self, entry: Option<String>, source: LoginSource) {
        // Refresh detected desktop app version right before EnsureSandBox (env/prefs still win).
        self.client_version_source = apply_client_identity(&mut self.opts, &self.config, true);
        self.connecting = true;
        let (epoch, mock, opts) = (self.epoch, self.mock, self.opts.clone());
        let reader = self.desktop_reader.clone();
        let lister = self.desktop_lister.clone();
        let ask = source == LoginSource::Interactive && entry.is_none();
        self.spawn(async move {
            let e2 = entry.clone();
            let policy = opts.policy;
            let joined = tokio::task::spawn_blocking(move || {
                let r = reader(e2.as_deref(), &policy);
                let ambiguous = matches!(&r, Err(gw::Error::Desktop { code, .. }) if code == "AMBIGUOUS_ENTRIES");
                let choices = (ask && ambiguous).then(|| lister().ok()).flatten();
                (r, choices)
            })
            .await;
            if let Ok((_, Some(choices))) = &joined
                && choices.len() > 1
            {
                return Event::DesktopChoices {
                    epoch,
                    choices: choices.clone(),
                };
            }
            let read = joined
                .map_err(|e| format!("desktop session reader failed: {e}"))
                .and_then(|(r, _)| r.map_err(|e| desktop_error_text(&e)));
            let (result, desktop_entry) = match read {
                Ok((gw::desktop::DesktopCredential::Session(session), stored)) => (
                    login_task(mock, opts, Some(session), None).await,
                    stored.or(entry),
                ),
                Ok((gw::desktop::DesktopCredential::CursorToken(tok), stored)) => (
                    // Exchange like `/login cursor`, but do NOT persist the Cursor token:
                    // on 401 we re-read sand-secrets instead.
                    login_task(mock, opts, None, Some(tok)).await,
                    stored.or(entry),
                ),
                Err(e) => (Err(e), entry),
            };
            Event::Login {
                epoch,
                method: LoginMethod::Desktop,
                source,
                cursor_token: None,
                desktop_entry,
                result,
            }
        });
    }

    /// On 401 for an account that logged in with `/login desktop`, re-read the desktop
    /// session once (the user opted in for this account). Returns true if a re-read started.
    fn try_desktop_reread(&mut self) -> bool {
        if self.login_method != Some(LoginMethod::Desktop)
            || self.login_source == Some(LoginSource::Env)
            || self.desktop_reread_tried
            || self.connecting
        {
            return false;
        }
        self.desktop_reread_tried = true;
        let entry = self
            .store
            .load_login()
            .ok()
            .flatten()
            .and_then(|l| l.desktop_entry);
        self.info(
            "Gateway rejected the session (401/417, usually expired); re-reading the desktop app session…".into(),
        );
        self.start_desktop_login(entry, LoginSource::Stored);
        true
    }

    /// Delete this account's stored credential and clear session state. Chats and memory are kept.
    pub fn logout(&mut self) {
        let alias = self.account().alias.clone();
        self.reset_session();
        match self.store.delete_credentials() {
            Ok(true) => self.info(format!(
                "Logged out of '{alias}'; stored credential deleted. Chats and memory kept."
            )),
            Ok(false) => self.info(format!("Logged out of '{alias}' (no stored credential).")),
            Err(e) => self.error(format!("Could not delete credential: {e:#}")),
        }
        if self.store.is_startup_account()
            && std::env::var(TOKEN_ENV).is_ok_and(|t| !t.trim().is_empty())
        {
            self.info(format!(
                "Note: {TOKEN_ENV} is set and will log this account back in next start."
            ));
        }
    }

    // ------------------------------------------------------------ bots

    pub fn refresh_bots(&mut self, announce: bool) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in. Use /login gateway or /login cursor.".into());
            return;
        };
        let epoch = self.epoch;
        self.spawn(async move {
            let result = api.list_bots().await;
            Event::Bots {
                epoch,
                announce,
                result,
            }
        });
    }

    /// `/shell <cmd>`: run locally off the UI thread; output goes to the shell modal.
    /// Purely local: no gateway call, nothing stored as chat.
    pub fn run_shell_command(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        self.shell_seq += 1;
        let id = self.shell_seq;
        self.help = None;
        self.shell = Some(ShellModal {
            id,
            cmd: cmd.clone(),
            result: None,
            scroll: 0,
        });
        self.spawn(async move {
            let result = run_shell(&cmd).await;
            Event::Shell { id, cmd, result }
        });
    }

    pub fn close_shell(&mut self) {
        self.shell = None;
    }

    pub fn shell_scroll(&mut self, delta: i32) {
        if let Some(m) = &mut self.shell {
            m.scroll = (m.scroll as i32 + delta).max(0) as u16;
        }
    }

    /// `/bot new <name>`: create on the gateway, then select it.
    pub fn create_bot(&mut self, name: &str) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in. Use /login desktop.".into());
            return;
        };
        let name = name.trim().to_string();
        let epoch = self.epoch;
        self.info(format!("Creating bot {name}…"));
        self.spawn(async move {
            let result = api.create_bot(&name).await;
            Event::BotCreated { epoch, result }
        });
    }

    /// `/bot delete [name]`: confirm in a picker (Cancel is the default), then delete.
    pub fn confirm_delete_bot(&mut self, i: usize) {
        use crate::picker::{PickItem, Picker};
        let Some(b) = self.bots.get(i).cloned() else {
            return;
        };
        let items = vec![
            PickItem::new("Cancel", "cancel"),
            PickItem::new(format!("Delete {}", b.name), format!("delete:{}", b.id))
                .detail("permanent"),
        ];
        self.open_picker(
            Picker::new(format!("Delete bot {}?", b.name), items, |app, it| {
                if let Some(id) = it.value.strip_prefix("delete:") {
                    app.delete_bot(id);
                } else {
                    app.info("Delete cancelled.".into());
                }
            })
            .with_selected("cancel")
            .on_cancel(|app| app.info("Delete cancelled.".into())),
        );
    }

    fn delete_bot(&mut self, id: &str) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in. Use /login desktop.".into());
            return;
        };
        let name = self
            .all_bots
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| id.to_string());
        let id = id.to_string();
        let epoch = self.epoch;
        self.spawn(async move {
            let result = api.delete_bot(&id).await;
            Event::BotDeleted {
                epoch,
                id,
                name,
                result,
            }
        });
    }

    /// Store the bot list and rebuild the sidebar (hidden bots only with `show_hidden_bots`).
    pub fn apply_bots(&mut self, all: Vec<Bot>) {
        if let Err(e) = self.store.save_bots(&all) {
            self.error(format!("Could not save bots: {e:#}"));
        }
        let current = self.active_bot().map(|b| b.id.clone());
        self.all_bots = all;
        self.bots = self
            .all_bots
            .iter()
            .filter(|b| {
                self.config.show_hidden_bots
                    || !b.hidden
                    || current.as_deref() == Some(b.id.as_str())
            })
            .cloned()
            .collect();
        self.active = None;
        let want = current
            .or(self.config.active_bot.clone())
            .or(self.config.default_bot.clone());
        let idx = want
            .and_then(|w| self.find_bot(&w))
            .or(if self.bots.is_empty() { None } else { Some(0) });
        if let Some(i) = idx {
            self.select_bot(i);
        }
    }

    pub fn find_bot(&self, q: &str) -> Option<usize> {
        let q = q.trim().to_lowercase();
        self.bots
            .iter()
            .position(|b| b.id.to_lowercase() == q || b.name.to_lowercase() == q)
    }

    /// Id, viewer session, and team flag for the active bot (for gateway routing).
    pub(crate) fn active_bot_route(&self) -> Option<(String, Option<String>, bool)> {
        self.active_bot()
            .map(|b| (b.id.clone(), b.viewer_session_id.clone(), b.team))
    }

    /// Switch the body to bot `i`'s chat: restore from the DB, then poll its transcript.
    pub fn select_bot(&mut self, i: usize) {
        if i >= self.bots.len() {
            return;
        }
        let changed = self.active != Some(i);
        self.active = Some(i);
        self.sidebar_cursor = i;
        self.follow_bottom();
        let id = self.bots[i].id.clone();
        if self.config.active_bot.as_deref() != Some(id.as_str()) {
            self.config.active_bot = Some(id.clone());
            if let Err(e) = self.store.set_pref("active_bot", &id) {
                self.error(format!("Could not save active bot: {e:#}"));
            }
        }
        self.reload_bot_log(&id);
        if changed {
            self.poll.interval = if self.awaiting.contains_key(&id) {
                POLL_FAST
            } else {
                POLL_IDLE
            };
            if let Some(b) = self.active_bot()
                && b.team
                && b.viewer_session_id.is_none()
            {
                self.error(format!(
                    "Team bot '{}' has no viewer session — send/history will fail.                      Re-login with /login cursor or /login desktop (needs a Cursor token).",
                    b.name
                ));
            }
            // Pull the server transcript whenever the active bot changes so chats
            // from the desktop app show up without a manual /history.
            if self.api.is_some() {
                self.request_history();
            } else {
                self.poll_now();
            }
        }
    }

    /// Replace the in-memory log for `bot_id` with the merged SQLite transcript,
    /// keeping only trailing system/error lines (e.g. delivery notices) so polls
    /// don't wipe them and don't duplicate them.
    fn reload_bot_log(&mut self, bot_id: &str) {
        let old_len = self.logs.get(bot_id).map(|v| v.len()).unwrap_or(0);
        let msgs = self.store.messages(bot_id).unwrap_or_default();
        let atts: Vec<(String, Vec<crate::api::Attachment>)> = msgs
            .iter()
            .filter(|m| !m.attachments.is_empty())
            .map(|m| {
                let e = entry_from_msg(m);
                (
                    crate::emoji::message_key(e.kind as u8, &e.text),
                    m.attachments.clone(),
                )
            })
            .collect();
        if atts.is_empty() {
            self.msg_attachments.remove(bot_id);
        } else {
            self.msg_attachments.insert(bot_id.to_string(), atts);
        }
        let entries: Vec<Entry> = msgs.iter().map(entry_from_msg).collect();
        let new_len = entries.len();
        self.logs.insert(bot_id.to_string(), entries);
        if self.active_bot().is_some_and(|b| b.id == bot_id) && !self.follow && new_len > old_len {
            self.unread = self.unread.saturating_add((new_len - old_len) as u32);
        }
    }

    /// Fetch the full (paged) chat history for the active bot now.
    pub fn request_history(&mut self) {
        let (Some(api), Some((bot, session, team))) = (self.api.clone(), self.active_bot_route())
        else {
            self.poll.limit = HISTORY_LIMIT;
            self.poll_now();
            return;
        };
        if team && session.is_none() {
            self.error(
                "Cannot load history: team bot has no viewer session.                  Re-login with /login cursor or /login desktop."
                    .into(),
            );
            return;
        }
        if self.poll.inflight {
            // Queue a full fetch as soon as the in-flight poll finishes.
            self.poll.limit = HISTORY_LIMIT;
            return;
        }
        self.poll.inflight = true;
        let epoch = self.epoch;
        self.spawn(async move {
            api.bind_viewer_session(&bot, session.as_deref());
            let result = api.transcript_history(&bot, HISTORY_LIMIT).await;
            Event::Transcript {
                epoch,
                bot,
                full: true,
                result,
            }
        });
    }

    // ------------------------------------------------------------ selection modal

    /// Show a selection modal; `on_pick` gets the chosen item on confirm.
    pub fn open_picker(&mut self, picker: crate::picker::Picker) {
        self.picker = Some(picker);
    }

    /// Confirm the highlighted item: close the modal, then run its callback.
    pub fn picker_confirm(&mut self) {
        if let Some(p) = self.picker.take()
            && let Some(item) = p.selected().cloned()
        {
            (p.on_pick)(self, item);
        }
    }

    /// Call the open picker's `on_move` hook if its highlight left `before`.
    pub fn picker_moved(&mut self, before: usize) {
        if let Some(p) = &self.picker
            && p.cursor != before
            && let Some(f) = p.on_move
        {
            f(self);
        }
    }

    pub fn picker_cancel(&mut self) {
        if let Some(f) = self.picker.take().and_then(|p| p.on_cancel) {
            f(self);
        }
    }

    /// Bot switcher (`/bot`, F2).
    pub fn open_bot_picker(&mut self) {
        use crate::picker::{PickItem, Picker};
        if self.bots.is_empty() {
            self.error("No bots to pick from. Log in (/login desktop) or /bots.".into());
            return;
        }
        let current = self.active_bot().map(|b| b.id.clone()).unwrap_or_default();
        let items = self
            .bots
            .iter()
            .map(|b| {
                let it = PickItem::new(b.name.clone(), b.id.clone());
                if b.id == current {
                    it.detail("current")
                } else {
                    it
                }
            })
            .collect();
        self.open_picker(
            Picker::new("Switch bot", items, |app, it| {
                if let Some(i) = app.bots.iter().position(|b| b.id == it.value) {
                    app.select_bot(i);
                    app.focus = Focus::Input;
                }
            })
            .with_selected(&current),
        );
    }

    /// Account switcher (`/account use`, F3, click the @account header).
    pub fn open_account_picker(&mut self) {
        use crate::picker::{PickItem, Picker};
        let list = match self.store.accounts() {
            Ok(l) => l,
            Err(e) => {
                self.error(format!("{e:#}"));
                return;
            }
        };
        let current = self.account().alias.clone();
        let items = list
            .iter()
            .map(|a| {
                let it = PickItem::new(format!("@{}", a.alias), a.alias.clone());
                if a.alias == current {
                    it.detail("current")
                } else {
                    it
                }
            })
            .collect();
        self.open_picker(
            Picker::new("Switch account", items, |app, it| {
                if it.value != app.account().alias
                    && let Err(e) = app.switch_account(&it.value)
                {
                    app.error(format!("{e:#}"));
                }
            })
            .with_selected(&current),
        );
    }

    // ------------------------------------------------------------ help modal

    /// Open `/help <arg>` in the modal. Returns false if no such topic/command.
    pub fn open_help(&mut self, arg: &str) -> bool {
        let Some(page) = crate::help::page(&self.registry, arg) else {
            return false;
        };
        let back = match self.help.take() {
            Some(m) if m.page.id != page.id => {
                let mut b = m.back;
                b.push(m.page.id);
                b
            }
            Some(m) => m.back,
            None => vec![],
        };
        self.help = Some(HelpModal {
            page,
            scroll: 0,
            link: 0,
            back,
            query: None,
        });
        true
    }

    /// Start typing a search query inside the help modal.
    pub fn help_search_start(&mut self) {
        if let Some(m) = &mut self.help {
            m.query = Some(String::new());
        }
    }

    /// Run the typed query (empty cancels).
    pub fn help_search_submit(&mut self) {
        let Some(q) = self.help.as_mut().and_then(|m| m.query.take()) else {
            return;
        };
        if !q.trim().is_empty() {
            self.open_help(&format!("search {q}"));
        }
    }

    pub fn close_help(&mut self) {
        self.help = None;
    }

    pub fn help_scroll(&mut self, delta: i32) {
        if let Some(m) = &mut self.help {
            m.scroll = (m.scroll as i32 + delta).max(0) as u16;
        }
    }

    pub fn help_select(&mut self, delta: i32) {
        if let Some(m) = &mut self.help
            && !m.page.related.is_empty()
        {
            let n = m.page.related.len() as i32;
            m.link = ((m.link as i32 + delta).rem_euclid(n)) as usize;
        }
    }

    /// Follow the selected related link.
    pub fn help_follow(&mut self) {
        if let Some(id) = self
            .help
            .as_ref()
            .and_then(|m| m.page.related.get(m.link).cloned())
        {
            self.open_help(&id);
        }
    }

    pub fn help_back(&mut self) {
        let Some(m) = &mut self.help else { return };
        let Some(prev) = m.back.pop() else { return };
        let back = std::mem::take(&mut m.back);
        if let Some(page) = crate::help::page(&self.registry, &prev) {
            self.help = Some(HelpModal {
                page,
                scroll: 0,
                link: 0,
                back,
                query: None,
            });
        }
    }

    pub fn active_bot(&self) -> Option<&Bot> {
        self.active.and_then(|i| self.bots.get(i))
    }

    pub fn is_typing(&self) -> bool {
        self.active_bot()
            .is_some_and(|b| self.awaiting.contains_key(&b.id))
    }

    // ------------------------------------------------------------ body log

    pub(crate) fn key_of_active(&self) -> String {
        self.key()
    }

    fn key(&self) -> String {
        self.active_bot().map(|b| b.id.clone()).unwrap_or_default()
    }

    /// The active bot's chat feed (rebuilt from SQLite).
    pub fn feed(&self) -> &[Entry] {
        self.logs.get(&self.key()).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Local reactions for the active bot's messages, (msg_key, emoji).
    pub fn active_reactions(&mut self) -> &[(String, String)] {
        let bot = self.key();
        if self.reaction_cache.as_ref().is_none_or(|(b, _)| *b != bot) {
            let list = if bot.is_empty() {
                vec![]
            } else {
                self.store.reactions(&bot).unwrap_or_default()
            };
            self.reaction_cache = Some((bot, list));
        }
        &self.reaction_cache.as_ref().expect("set above").1
    }

    /// Toggle `emoji` on the `nth` (1 = newest) bot message in the active feed, or the
    /// newest message of any kind when the bot hasn't replied yet. Local only: the gateway
    /// API has no reaction endpoint.
    pub fn react(&mut self, emoji: &str, nth: usize) -> anyhow::Result<bool> {
        let bot = self.key();
        if bot.is_empty() {
            anyhow::bail!("No active bot");
        }
        let Some(key) = self.react_target(nth) else {
            anyhow::bail!("No message to react to")
        };
        let added = self.store.toggle_reaction(&bot, &key, emoji)?;
        self.reaction_cache = None;
        Ok(added)
    }

    /// The message key `/react … nth` targets: the nth newest bot message, or the nth
    /// newest message when the bot hasn't replied.
    pub fn react_target(&self, nth: usize) -> Option<String> {
        let feed = self.feed();
        let bots: Vec<&Entry> = feed
            .iter()
            .rev()
            .filter(|e| e.kind == EntryKind::Bot)
            .collect();
        let target = if bots.is_empty() {
            feed.iter().rev().nth(nth.saturating_sub(1))
        } else {
            bots.get(nth.saturating_sub(1)).copied()
        };
        target.map(|e| crate::emoji::message_key(e.kind as u8, &e.text))
    }

    /// Emoji already on the target message, in the order added.
    pub fn reactions_on(&self, nth: usize) -> Vec<String> {
        let (Some(key), bot) = (self.react_target(nth), self.key()) else {
            return vec![];
        };
        self.store
            .reactions(&bot)
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| *k == key)
            .map(|(_, e)| e)
            .collect()
    }

    /// Transient notices for the active view.
    pub fn notices(&self) -> &[Entry] {
        self.notices
            .get(&self.key())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Everything shown in the body, in render order: feed, then notices.
    #[cfg(test)]
    pub fn current_log(&self) -> Vec<Entry> {
        self.feed().iter().chain(self.notices()).cloned().collect()
    }

    fn push_to(&mut self, key: &str, e: Entry) {
        const MAX_NOTICES: usize = 50;
        let n = self.notices.entry(key.to_string()).or_default();
        n.push(e);
        if n.len() > MAX_NOTICES {
            n.drain(..n.len() - MAX_NOTICES);
        }
        if key == self.key() && !self.follow {
            self.unread = self.unread.saturating_add(1);
        }
    }

    pub fn push(&mut self, kind: EntryKind, text: String) {
        let k = self.key();
        self.push_to(&k, Entry { kind, text });
    }
    pub fn info(&mut self, t: String) {
        self.raise_notice(crate::notify::NoticeKind::Info, "Info", t.clone());
        self.push(EntryKind::System, t);
    }
    pub fn error(&mut self, t: String) {
        self.raise_notice(crate::notify::NoticeKind::Error, "Error", t.clone());
        self.push(EntryKind::Error, t);
    }

    /// Apply notify-rules then maybe toast. Never changes chat scroll (1.9.1).
    pub fn raise_notice(
        &mut self,
        kind: crate::notify::NoticeKind,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        let title = title.into();
        let body = body.into();
        match self.notify_rules.decide(kind, &title, &body) {
            crate::notify_rules::Action::Drop => {}
            crate::notify_rules::Action::Mute => {
                self.notices_board
                    .log
                    .push_front(crate::notify::Notice::new(kind, title, body));
            }
            crate::notify_rules::Action::Toast => {
                // Temporarily honour mute-via-settings by posting through the board.
                self.notices_board.post(
                    crate::notify::Notice::new(kind, title, body),
                    std::time::Instant::now(),
                );
            }
        }
    }

    pub fn maybe_apple_terminal_hint(&mut self) {
        if self.apple_newline_hinted {
            return;
        }
        let term = std::env::var("TERM_PROGRAM").ok();
        if crate::keys::apple_terminal_needs_hint(self.keyboard_enhancement, term.as_deref()) {
            self.apple_newline_hinted = true;
            self.info(crate::keys::APPLE_TERMINAL_HINT.into());
        }
    }

    pub fn reload_notify_rules(&mut self) {
        let path = crate::notify_rules::user_path(&self.store.dir);
        let (rs, err) = crate::notify_rules::Ruleset::load(&path, Some(&self.notify_rules));
        self.notify_rules = rs;
        if let Some(e) = err {
            self.error(format!("Notify rules reload failed: {e}"));
        } else {
            self.info(format!("Notify rules reloaded from {}.", path.display()));
        }
    }

    /// Run a menu-selected slash command by name.
    pub fn run_menu_command(&mut self, name: &str) {
        self.menu.close();
        self.input = format!("/{name}");
        self.input_cursor = self.input.len();
        self.submit();
    }

    // ------------------------------------------------------------ input

    pub fn input_insert(&mut self, t: &str) {
        self.input_delete_selection();
        crate::input::insert(&mut self.input, &mut self.input_cursor, t);
    }
    /// Newline in the message (Shift/Alt-Enter, Ctrl-J). Login prompts stay one line.
    pub fn input_newline(&mut self) {
        if self.mode == InputMode::Normal {
            self.input_insert("\n");
        }
    }
    pub fn input_backspace(&mut self) {
        if !self.input_delete_selection() {
            crate::input::backspace(&mut self.input, &mut self.input_cursor);
        }
    }
    pub fn input_delete(&mut self) {
        if !self.input_delete_selection() {
            crate::input::delete(&mut self.input, &mut self.input_cursor);
        }
    }
    pub fn input_left(&mut self) {
        crate::input::left(&self.input, &mut self.input_cursor);
    }
    pub fn input_right(&mut self) {
        crate::input::right(&self.input, &mut self.input_cursor);
    }
    pub fn input_home(&mut self) {
        crate::input::home(&self.input, &mut self.input_cursor);
    }
    pub fn input_end(&mut self) {
        crate::input::end(&self.input, &mut self.input_cursor);
    }
    /// The input's selected byte range, if any (anchor to cursor, ordered).
    pub fn input_selection(&self) -> Option<(usize, usize)> {
        let a = self.input_anchor?.min(self.input.len());
        let c = self.input_cursor.min(self.input.len());
        (a != c).then(|| (a.min(c), a.max(c)))
    }

    /// Start or extend the input selection before a Shift+movement.
    pub fn input_extend(&mut self) {
        if self.input_anchor.is_none() {
            self.input_anchor = Some(self.input_cursor);
        }
    }

    pub fn input_select_all(&mut self) {
        self.input_anchor = Some(0);
        self.input_cursor = self.input.len();
    }

    /// Delete the selected range. Returns whether there was one.
    fn input_delete_selection(&mut self) -> bool {
        let sel = self.input_selection();
        self.input_anchor = None;
        match sel {
            Some((a, b)) => {
                self.input.replace_range(a..b, "");
                self.input_cursor = a;
                true
            }
            None => false,
        }
    }

    /// Put `text` on the clipboard (OSC 52) and say how much.
    pub fn copy_text(&mut self, text: String) {
        let n = text.chars().count();
        self.pending_clipboard = Some(text);
        self.info(format!("Copied {n} chars."));
    }

    /// The pane at a screen position.
    pub fn pane_at(&self, col: u16, row: u16) -> crate::keyctx::Pane {
        use crate::keyctx::Pane;
        let at = ratatui::layout::Position::new(col, row);
        if self.body_area.contains(at) {
            Pane::Body
        } else if self.input_area.contains(at) {
            Pane::Input
        } else if self.sidebar_area.contains(at) {
            Pane::Sidebar
        } else if row < self.sidebar_area.y {
            Pane::Header
        } else {
            Pane::None
        }
    }

    pub fn overlay_open(&self) -> bool {
        self.picker.is_some()
            || self.editor.is_some()
            || self.help.is_some()
            || self.shell.is_some()
            || self.history_search.is_some()
            || self.completion.is_some()
            || self.menu.open.is_some()
            || self.image_view.is_some()
            || self.react_picker.is_some()
            || self.download.is_some()
    }

    /// Load prompt history for the active account from `context.db`. A failure is shown
    /// (once) instead of silently leaving history empty.
    pub fn reload_history(&mut self) {
        self.refresh_history();
        self.history_idx = None;
        self.history_draft = None;
    }

    /// Re-read history from the database without resetting browsing state. Picks up prompts
    /// sent from another grokbot-tui running on the same data dir.
    fn refresh_history(&mut self) {
        match self.store.prompt_entries(self.config.history_max) {
            Ok(entries) => {
                self.history_error = None;
                self.prompt_history = entries.iter().map(|e| e.text.clone()).collect();
                self.prompt_meta = entries;
            }
            Err(e) => {
                let msg = format!("Couldn't load prompt history: {e:#}");
                if self.history_error.as_deref() != Some(msg.as_str()) {
                    self.history_error = Some(msg.clone());
                    self.error(msg);
                }
            }
        }
    }

    /// Up in the input: previous line, else older prompt.
    pub fn history_up(&mut self) -> bool {
        if crate::input::up(&self.input, &mut self.input_cursor) {
            return true;
        }
        let next = match self.history_idx {
            None => {
                self.refresh_history();
                if self.prompt_history.is_empty() {
                    return false;
                }
                self.history_draft = Some(self.input.clone());
                0
            }
            Some(i) if i + 1 < self.prompt_history.len() => i + 1,
            Some(_) => return true,
        };
        self.history_idx = Some(next);
        self.input = self.prompt_history[next].clone();
        self.input_cursor = self.input.len();
        self.input_anchor = None;
        true
    }

    /// Down in the input: next line, else newer prompt / the draft.
    pub fn history_down(&mut self) -> bool {
        if crate::input::down(&self.input, &mut self.input_cursor) {
            return true;
        }
        match self.history_idx {
            None => false,
            Some(0) => {
                self.history_idx = None;
                self.input = self.history_draft.take().unwrap_or_default();
                self.input_cursor = self.input.len();
                self.input_anchor = None;
                true
            }
            Some(i) => {
                self.history_idx = Some(i - 1);
                self.input = self.prompt_history[i - 1].clone();
                self.input_cursor = self.input.len();
                self.input_anchor = None;
                true
            }
        }
    }

    pub fn open_history_search(&mut self) {
        self.completion = None;
        self.refresh_history();
        let mut search = crate::history::Search::new(
            &self.prompt_history,
            self.input.clone(),
            self.input_cursor,
        );
        search.vim = self.config.keys == crate::config::KeysScheme::Vim;
        self.history_search = Some(search);
    }

    /// `/history [query]`: browse every remembered prompt (newest first), filtered as you type.
    pub fn open_history_browser(&mut self, query: &str) {
        self.completion = None;
        self.refresh_history();
        self.history_search = Some(crate::history::Search::browser(
            &self.prompt_history,
            query.trim(),
            self.config.keys == crate::config::KeysScheme::Vim,
            self.input.clone(),
            self.input_cursor,
        ));
    }

    /// `/history clear`: confirm in a picker (Cancel is the default), then forget it all.
    pub fn confirm_clear_history(&mut self) {
        use crate::picker::{PickItem, Picker};
        let n = self.prompt_history.len();
        let items = vec![
            PickItem::new("Cancel", "cancel"),
            PickItem::new(format!("Forget {n} prompts"), "clear").detail("permanent"),
        ];
        self.open_picker(
            Picker::new(
                format!(
                    "Clear prompt history for account '{}'?",
                    self.account().alias
                ),
                items,
                |app, it| {
                    if it.value == "clear" {
                        match app.store.clear_prompt_history() {
                            Ok(n) => {
                                app.reload_history();
                                app.info(format!("Prompt history cleared ({n} removed)."));
                            }
                            Err(e) => app.error(format!("Couldn't clear prompt history: {e:#}")),
                        }
                    } else {
                        app.info("Prompt history kept.".into());
                    }
                },
            )
            .with_selected("cancel")
            .on_cancel(|app| app.info("Prompt history kept.".into())),
        );
    }

    /// Mouse in the history overlay: wheel scrolls, click selects / takes.
    pub fn history_search_mouse(&mut self, m: ratatui::crossterm::event::MouseEvent) {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let Some(s) = self.history_search.as_mut() else {
            return;
        };
        match m.kind {
            MouseEventKind::ScrollUp => s.wheel(-crate::history::WHEEL_STEP),
            MouseEventKind::ScrollDown => s.wheel(crate::history::WHEEL_STEP),
            MouseEventKind::Down(MouseButton::Left) => {
                if let crate::history::Outcome::Take(text) =
                    s.click(&self.prompt_history, m.column, m.row)
                {
                    self.history_search = None;
                    self.take_history(text);
                }
            }
            _ => {}
        }
    }

    fn take_history(&mut self, text: String) {
        self.input = text;
        self.input_cursor = self.input.len();
        self.input_anchor = None;
        self.history_idx = None;
        self.history_draft = None;
    }

    pub fn history_search_key(&mut self, k: ratatui::crossterm::event::KeyEvent) {
        let Some(mut search) = self.history_search.take() else {
            return;
        };
        match crate::history::key(&mut search, &self.prompt_history, k) {
            crate::history::Outcome::Take(text) => self.take_history(text),
            crate::history::Outcome::Cancel => {
                self.input = search.draft;
                self.input_cursor = search.draft_cursor.min(self.input.len());
            }
            crate::history::Outcome::None => self.history_search = Some(search),
        }
    }

    /// Remember a sent prompt (slash commands included).
    pub fn body_drag(&mut self, col: u16, row: u16) {
        let Some(sel) = self.body_sel.as_mut() else {
            return;
        };
        if let Some(off) = crate::select::offset_at(
            &self.body_plain,
            self.body_area,
            col,
            row,
            self.body_wrap,
            self.body_row_off,
        ) {
            sel.cursor = off;
            // Scroll when dragging past the edge.
            if row <= self.body_area.y {
                self.scroll_up(1);
            } else if row + 1 >= self.body_area.bottom() {
                self.scroll_down(1);
            }
        }
    }

    pub fn body_mouse_up(&mut self) {
        if let Some(sel) = &self.body_sel
            && !sel.is_empty()
        {
            let t = crate::select::selected_text(&self.body_plain, sel);
            if !t.is_empty() {
                self.copy_text(t);
            }
        }
    }

    pub fn keys_are_vim(&self) -> bool {
        self.config.keys == crate::config::KeysScheme::Vim
    }

    pub fn set_keys_scheme(&mut self, scheme: crate::config::KeysScheme) -> anyhow::Result<()> {
        self.store
            .set_pref(crate::config::PREF_KEYS, scheme.as_str())?;
        self.config.keys = scheme;
        if scheme == crate::config::KeysScheme::Vim {
            self.vim_mode = crate::keymap::Mode::Insert;
            self.info("Keys: vim. Esc → NORMAL, i → INSERT, : → command. See /keymap.".into());
        } else {
            self.vim_mode = crate::keymap::Mode::Insert;
            self.vim_command = None;
            self.info("Keys: default (classic bindings).".into());
        }
        Ok(())
    }

    pub fn reload_keymap(&mut self) {
        let path = crate::keymap::user_path(&self.store.dir);
        let (km, err) = crate::keymap::load(&path, Some(&self.keymap));
        self.keymap = km;
        if let Some(e) = err {
            self.error(format!("Keymap reload failed: {e} (kept previous)."));
        } else {
            self.info(format!("Keymap reloaded from {}.", path.display()));
        }
    }

    /// Run a keymap action id. Returns true if handled.
    pub fn vim_action(&mut self, action: &str) -> bool {
        use crate::keymap::Mode;
        match action {
            "app.quit" => {
                self.should_quit = true;
                true
            }
            "app.ctrl_c" => {
                self.ctrl_c(false, std::time::Instant::now());
                true
            }
            "app.ctrl_c_copy" => {
                self.ctrl_c(true, std::time::Instant::now());
                true
            }
            "help.open" => {
                self.open_help("keys");
                true
            }
            "mouse.toggle" => {
                self.toggle_mouse_capture();
                true
            }
            "mode.insert" => {
                self.vim_mode = Mode::Insert;
                self.focus = Focus::Input;
                self.vim_command = None;
                true
            }
            "mode.insert_end" => {
                self.vim_mode = Mode::Insert;
                self.focus = Focus::Input;
                self.input_cursor = self.input.len();
                self.vim_command = None;
                true
            }
            "mode.normal" => {
                self.vim_mode = Mode::Normal;
                self.vim_command = None;
                self.completion = None;
                true
            }
            "mode.visual" => {
                self.vim_mode = Mode::Visual;
                if self.body_sel.is_none() {
                    let off = self.body_plain.len().saturating_sub(1);
                    self.body_sel = Some(crate::select::Selection {
                        anchor: off,
                        cursor: off,
                    });
                }
                true
            }
            "mode.command" => {
                self.vim_mode = Mode::Command;
                self.vim_command = Some(String::new());
                self.focus = Focus::Input;
                true
            }
            "move.left" => {
                if self.vim_mode == Mode::Visual {
                    if let Some(sel) = self.body_sel.as_mut() {
                        sel.cursor = sel.cursor.saturating_sub(1);
                    }
                } else if self.focus == Focus::Input {
                    self.input_left();
                } else {
                    self.scroll_up(1);
                }
                true
            }
            "move.right" => {
                if self.vim_mode == Mode::Visual {
                    if let Some(sel) = self.body_sel.as_mut() {
                        sel.cursor = (sel.cursor + 1).min(self.body_plain.len());
                    }
                } else if self.focus == Focus::Input {
                    self.input_right();
                } else {
                    self.scroll_down(1);
                }
                true
            }
            "move.up" | "move.down" => {
                let up = action == "move.up";
                if self.vim_mode == Mode::Visual || self.focus == Focus::Sidebar {
                    if up {
                        self.scroll_up(1);
                    } else {
                        self.scroll_down(1);
                    }
                } else if up {
                    let _ = self.history_up();
                } else {
                    let _ = self.history_down();
                }
                true
            }
            "move.top" => {
                // Distance-from-bottom: large value = top of the chat.
                self.scroll = u16::MAX / 2;
                self.follow = false;
                self.pinned_off = Some(0);
                true
            }
            "move.bottom" => {
                self.follow_bottom();
                true
            }
            "focus.next" => {
                self.focus = if self.focus == Focus::Input {
                    Focus::Sidebar
                } else {
                    Focus::Input
                };
                true
            }
            "focus.prev" => {
                self.focus = if self.focus == Focus::Sidebar {
                    Focus::Input
                } else {
                    Focus::Sidebar
                };
                true
            }
            "input.history_search" => {
                self.open_history_search();
                true
            }
            "input.undo_clear" => {
                self.input_undo_clear();
                true
            }
            "selection.select_all" => {
                self.ctrl_a();
                true
            }
            "selection.copy" | "selection.copy_line" => {
                if let Some(sel) = &self.body_sel {
                    let t = crate::select::selected_text(&self.body_plain, sel);
                    if !t.is_empty() {
                        self.copy_text(t);
                    }
                }
                if action == "selection.copy" {
                    self.vim_mode = Mode::Normal;
                }
                true
            }
            "selection.clear" => {
                self.body_sel = None;
                self.vim_mode = Mode::Normal;
                true
            }
            "command.run" => {
                if let Some(cmd) = self.vim_command.take() {
                    self.vim_mode = Mode::Normal;
                    let line = if cmd.starts_with('/') {
                        cmd
                    } else {
                        format!("/{cmd}")
                    };
                    self.input = line;
                    self.input_cursor = self.input.len();
                    self.submit();
                }
                true
            }
            "command.complete" => {
                if let Some(cmd) = &self.vim_command {
                    self.input = if cmd.starts_with('/') {
                        cmd.clone()
                    } else {
                        format!("/{cmd}")
                    };
                    self.input_cursor = self.input.len();
                    crate::complete::tab(self, false);
                    let text = self.input.clone();
                    self.vim_command = Some(text.trim_start_matches('/').to_string());
                }
                true
            }
            "keymap.reload" => {
                self.reload_keymap();
                true
            }
            "input.newline" => {
                self.input_newline();
                true
            }
            "image.view" => {
                if !self.view_nearest_image() {
                    self.error("No images in this chat's replies".into());
                }
                true
            }
            "message.open" => {
                if let Err(e) = crate::commands::open_item(self, "") {
                    self.error(format!("{e:#}"));
                }
                true
            }
            "message.download" => {
                if let Err(e) = crate::commands::download(self, "") {
                    self.error(format!("{e:#}"));
                }
                true
            }
            "message.react_picker" => {
                if !self.open_react_picker(1) {
                    self.error("No message to react to".into());
                }
                true
            }
            "message.react" => {
                self.vim_mode = Mode::Command;
                self.vim_command = Some("react :".to_string());
                self.focus = Focus::Input;
                true
            }
            "input.paste" => true, // terminal paste still works via bracketed paste
            _ => false,
        }
    }

    /// Remember a submitted line in the persistent prompt history (secrets redacted).
    pub fn remember_prompt(&mut self, text: &str) {
        let text = crate::history::redact(text);
        // Opening the history browser isn't worth remembering; it would bury real prompts.
        let first = text.split_whitespace().next().unwrap_or("");
        if matches!(first, "/history" | "/prompts") {
            return;
        }
        let bot = self.active_bot().map(|b| b.name.clone());
        if let Err(e) = self
            .store
            .push_prompt(&text, bot.as_deref(), self.config.history_max)
        {
            self.error(format!("Couldn't save prompt history: {e:#}"));
            return;
        }
        self.reload_history();
    }

    /// What Ctrl-C / Ctrl-A look at (see [`crate::keyctx`]).
    pub fn key_ctx(&self, now: Instant) -> crate::keyctx::Ctx {
        use crate::keyctx::{Ctx, Pane};
        Ctx {
            focus: if self.overlay_open() {
                Pane::Overlay
            } else if self.focus == Focus::Sidebar {
                Pane::Sidebar
            } else {
                Pane::Input
            },
            hovered: self
                .mouse_capture
                .then_some(self.mouse_pos)
                .flatten()
                .map(|(c, r)| self.pane_at(c, r))
                .unwrap_or_default(),
            body_selection: self.body_sel.as_ref().is_some_and(|s| !s.is_empty()),
            input_selection: self.input_selection().is_some(),
            input_has_text: !self.input.is_empty(),
            in_flight: self.is_typing(),
            overlay_own_ctrl_c: false,
            quit_armed: self
                .quit_armed
                .is_some_and(|at| now.duration_since(at) <= QUIT_CONFIRM),
        }
    }

    /// Ctrl-C (`copy`: Ctrl-Shift-C), resolved by [`crate::keyctx::ctrl_c`]. Never quits
    /// at once: a lone Ctrl-C asks, a second within [`QUIT_CONFIRM`] (nothing done in
    /// between) quits.
    pub fn ctrl_c(&mut self, copy: bool, now: Instant) {
        use crate::keyctx::CtrlC;
        let act = crate::keyctx::ctrl_c(&self.key_ctx(now), copy);
        if act != CtrlC::ArmQuit {
            self.quit_armed = None;
        }
        match act {
            CtrlC::CopyBodySelection => {
                if let Some(sel) = &self.body_sel {
                    let t = crate::select::selected_text(&self.body_plain, sel);
                    if !t.is_empty() {
                        self.copy_text(t);
                    }
                }
            }
            CtrlC::CopyInputSelection => {
                let (a, b) = self.input_selection().unwrap_or_default();
                self.copy_text(self.input[a..b].to_string());
            }
            CtrlC::CancelRequest => {
                if let Some(bot) = self.active_bot().map(|b| b.id.clone()) {
                    self.awaiting.remove(&bot);
                }
                self.info(
                    "Stopped waiting for the reply (it still lands in the chat if it comes)."
                        .into(),
                );
            }
            CtrlC::CopyInput => self.copy_text(self.input.clone()),
            CtrlC::ClearInput => {
                self.cleared_input = Some((std::mem::take(&mut self.input), self.input_cursor));
                self.input_cursor = 0;
                self.input_scroll = 0;
                self.input_anchor = None;
                self.info("Input cleared (Ctrl-Z brings it back).".into());
            }
            CtrlC::CloseOverlay => self.close_overlay(),
            CtrlC::Overlay => {}
            CtrlC::BodyHint => self.info(
                "Nothing selected. Shift-drag (Option-drag in macOS terminals) selects text \
                 with your terminal, F6 turns mouse capture off, /copy copies the last reply. \
                 Ctrl-Q quits."
                    .into(),
            ),
            CtrlC::NothingToCopy => self.info(
                "Nothing selected to copy. Shift-drag (Option-drag in macOS terminals) selects \
                 with your terminal; /copy copies the last bot reply."
                    .into(),
            ),
            CtrlC::ArmQuit => {
                self.quit_armed = Some(now);
                self.info("Press Ctrl-C again to quit (or Ctrl-Q / /quit).".into());
            }
            CtrlC::Quit => self.should_quit = true,
        }
    }

    /// Close the open overlay the way Esc does (the theme editor asks about unsaved changes).
    pub fn close_overlay(&mut self) {
        if self.picker.is_some() {
            self.picker_cancel();
        } else if self.editor.is_some() {
            self.editor_key(ratatui::crossterm::event::KeyEvent::new(
                ratatui::crossterm::event::KeyCode::Esc,
                ratatui::crossterm::event::KeyModifiers::NONE,
            ));
        } else if self.help.is_some() {
            self.close_help();
        } else if self.shell.is_some() {
            self.close_shell();
        }
    }

    /// Ctrl-A, resolved by [`crate::keyctx::ctrl_a`].
    pub fn ctrl_a(&mut self) {
        match crate::keyctx::ctrl_a(&self.key_ctx(Instant::now())) {
            crate::keyctx::CtrlA::Input => self.input_select_all(),
            crate::keyctx::CtrlA::BodyMessage => {
                if !self.body_plain.is_empty() {
                    self.body_sel = Some(crate::select::Selection {
                        anchor: 0,
                        cursor: self.body_plain.len(),
                    });
                }
            }
            crate::keyctx::CtrlA::None => {}
        }
    }

    /// Ctrl-Z: bring back the input a Ctrl-C cleared (only into an empty input).
    pub fn input_undo_clear(&mut self) {
        if self.input.is_empty()
            && let Some((text, cur)) = self.cleared_input.take()
        {
            self.input = text;
            self.input_cursor = cur.min(self.input.len());
        }
    }

    pub fn input_up(&mut self) -> bool {
        crate::input::up(&self.input, &mut self.input_cursor)
    }
    pub fn input_down(&mut self) -> bool {
        crate::input::down(&self.input, &mut self.input_cursor)
    }

    /// Turn mouse capture on/off (F6, `/mouse`). The main loop applies it to the terminal.
    pub fn set_mouse_capture(&mut self, on: bool) {
        self.mouse_capture = on;
        self.info(if on {
            "Mouse capture ON: clicks/wheel go to grokbot-tui. F6 or /mouse off to select text."
                .into()
        } else {
            "Mouse capture OFF: drag to select and copy with your terminal (in tmux: copy-mode). \
             Clicking bots and wheel scrolling are off until F6 or /mouse on."
                .into()
        });
    }

    pub fn toggle_mouse_capture(&mut self) {
        self.set_mouse_capture(!self.mouse_capture);
    }

    /// The color variant to draw with: the theme mode resolved against the detected
    /// terminal background (system → detected, else dark).
    pub fn theme_variant(&self) -> crate::theme::Variant {
        self.config.theme_mode.resolve(self.detected_variant)
    }

    /// Switch the appearance (F4, header click, `/theme`) and save it (global pref).
    pub fn set_theme_mode(&mut self, mode: crate::theme::Mode) {
        use crate::theme::{Mode, Variant};
        if let Err(e) = self
            .store
            .set_pref(crate::config::PREF_THEME_MODE, mode.as_str())
        {
            self.error(format!("{e:#}"));
            return;
        }
        self.config.theme_mode = mode;
        let note = match (mode, self.detected_variant) {
            (Mode::System, Some(Variant::Light)) => " (terminal background: light)",
            (Mode::System, Some(Variant::Dark)) => " (terminal background: dark)",
            (Mode::System, None) => " (terminal background not detected: using dark)",
            _ => "",
        };
        self.info(format!(
            "Theme: {}{note}. F4 cycles dark → light → system.",
            mode.as_str()
        ));
    }

    /// dark → light → system → dark (F4).
    pub fn cycle_theme_mode(&mut self) {
        self.set_theme_mode(self.config.theme_mode.next());
    }

    /// `/copy`: put the last bot message (or last message) on the clipboard via OSC 52.
    pub fn copy_last_message(&mut self) -> std::result::Result<(), String> {
        let feed = self.feed();
        let e = feed
            .iter()
            .rev()
            .find(|e| e.kind == EntryKind::Bot)
            .or_else(|| feed.last())
            .ok_or("No message to copy.")?;
        let text = e.text.clone();
        let n = text.chars().count();
        self.pending_clipboard = Some(text);
        self.info(format!(
            "Copied last message ({n} chars) via OSC 52 (needs terminal support; in tmux: set -g set-clipboard on)."
        ));
        Ok(())
    }

    pub fn cancel_input_mode(&mut self) {
        let was = self.mode;
        self.mode = InputMode::Normal;
        self.input.clear();
        self.input_anchor = None;
        self.input_cursor = 0;
        self.input_scroll = 0;
        self.login_url_draft = None;
        match was {
            InputMode::Normal => {}
            _ => self.info("Login cancelled.".into()),
        }
    }

    pub fn submit(&mut self) {
        let line = std::mem::take(&mut self.input);
        self.input_cursor = 0;
        self.input_scroll = 0;
        self.input_anchor = None;
        self.history_idx = None;
        self.history_draft = None;
        self.completion = None;
        let trimmed = line.trim().to_string();
        if !trimmed.is_empty() && self.mode == InputMode::Normal {
            self.remember_prompt(&trimmed);
            self.follow_bottom();
        }
        match self.mode {
            InputMode::GatewayUrl | InputMode::GatewayToken | InputMode::CursorToken
                if trimmed.is_empty() =>
            {
                self.cancel_input_mode();
                return;
            }
            InputMode::GatewayUrl => {
                commands::set_gateway_url(self, &trimmed);
                return;
            }
            InputMode::GatewayToken => {
                self.mode = InputMode::Normal;
                let url = self.login_url_draft.take().unwrap_or_default();
                self.info(format!("Checking gateway {}…", host_of(&url)));
                self.start_login(
                    LoginMethod::Gateway,
                    LoginSource::Interactive,
                    Some(Session::new(url, trimmed)),
                    None,
                );
                return;
            }
            InputMode::CursorToken => {
                self.mode = InputMode::Normal;
                self.info("Exchanging Cursor token (EnsureSandBox)…".into());
                self.start_login(
                    LoginMethod::Cursor,
                    LoginSource::Interactive,
                    None,
                    Some(trimmed),
                );
                return;
            }
            InputMode::Normal => {}
        }
        if trimmed.is_empty() {
            return;
        }
        if let Some(rest) = trimmed.strip_prefix('/') {
            let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            match self.registry.find(name) {
                Some(cmd) => {
                    let run = cmd.run;
                    // Help opens a modal; don't echo it into the chat.
                    if !matches!(
                        cmd.name,
                        "help"
                            | "shell"
                            | "react"
                            | "open"
                            | "view"
                            | "download"
                            | "attach"
                            | "detach"
                    ) && !(cmd.name == "login" && args.trim().is_empty())
                    {
                        self.push(EntryKind::User, trimmed.clone());
                    }
                    if let Err(e) = run(self, args.trim()) {
                        self.error(format!("{e:#}"));
                    }
                }
                None => {
                    let names: Vec<&str> = self
                        .registry
                        .commands
                        .iter()
                        .flat_map(|c| std::iter::once(c.name).chain(c.aliases.iter().copied()))
                        .collect();
                    match crate::help::suggest(name, names) {
                        Some(s) => self.error(format!(
                            "Unknown command /{name}. Did you mean /{s}? (/help {s})"
                        )),
                        None => self.error(format!("Unknown command /{name}. Try /help.")),
                    }
                }
            }
        } else {
            self.send_chat(crate::emoji::expand(&trimmed));
        }
    }

    /// Send a prompt in the background. The local copy is stored as pending until the transcript confirms it.
    pub fn send_chat(&mut self, text: String) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in. Use /login gateway or /login cursor.".into());
            return;
        };
        let Some((bot, session, team)) = self.active_bot_route() else {
            self.error("No active bot. Pick one with /bot or the sidebar.".into());
            return;
        };
        if team && session.is_none() {
            self.error(
                "Cannot send: team bot has no viewer session.                  Re-login with /login cursor or /login desktop so shared agents get a sessionId."
                    .into(),
            );
            return;
        }
        let Some(attachments) = self.take_attachments_for_send(&text) else {
            return;
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        let mine = Message {
            role: Role::User,
            text: text.clone(),
            remote_id: Some(format!("pending:{nonce}")),
            attachments: attachments.clone(),
        };
        if let Err(e) = self.store.append_message(&bot, &mine) {
            self.error(format!("Could not save message: {e:#}"));
            return;
        }
        // Optimistic UX without a second source: the pending row is in SQLite, so
        // rebuilding from it shows the message immediately.
        self.reload_bot_log(&bot);
        let epoch = self.epoch;
        self.spawn(async move {
            api.bind_viewer_session(&bot, session.as_deref());
            let result = api.send_with(&bot, &text, &nonce, &attachments).await;
            Event::Sent {
                epoch,
                bot,
                nonce,
                result,
            }
        });
    }

    pub fn click(&mut self, col: u16, row: u16) {
        let at = ratatui::layout::Position::new(col, row);
        if let Some(h) = self
            .react_hits
            .iter()
            .find(|h| h.y == row && col >= h.x && col < h.x + h.w)
        {
            let k = h.url.clone();
            self.open_react_picker_for(k);
            return;
        }
        if let Some(h) = self
            .chip_hits
            .iter()
            .find(|h| h.y == row && col >= h.x && col < h.x + h.w)
        {
            let act = h.url.clone();
            let n: usize = act[1..].parse().unwrap_or(0);
            if act.starts_with('x') {
                self.detach(n);
            } else if let Some(s) = n.checked_sub(1).and_then(|i| self.attach.staged.get(i)) {
                let p = s.path.display().to_string();
                match &s.state {
                    crate::attach::State::Failed(e) => {
                        let e = format!("{}: {e}", s.name);
                        self.error(e)
                    }
                    _ if s.kind == gw::AttachmentKind::Image => {
                        self.view_local_image(&p);
                    }
                    _ => {
                        let t = format!("{} · {}", s.name, p);
                        self.info(t)
                    }
                }
            }
            return;
        }
        if let Some(h) = self
            .download_hits
            .iter()
            .find(|h| h.y == row && col >= h.x && col < h.x + h.w)
        {
            let url = h.url.clone();
            if let Some(path) = url.strip_prefix(crate::attach::DL_SCHEME) {
                self.open_attachment(path, crate::attach::Then::Download);
            } else if let Some(path) = url.strip_prefix(crate::attach::SCHEME) {
                self.open_attachment(path, crate::attach::Then::Open);
            } else {
                self.open_download(&url);
            }
            return;
        }
        if let Some(h) = self
            .link_hits
            .iter()
            .find(|h| h.y == row && col >= h.x && col < h.x + h.w)
        {
            let url = h.url.clone();
            if h.image && self.view_image(Some(&url)) {
                return;
            }
            match crate::markdown::open_external(&url) {
                Ok(()) => self.info(format!("Opened {url}")),
                Err(e) => self.error(format!("Couldn't open {url}: {e}")),
            }
            return;
        }
        if self.unread_area.contains(at) {
            self.follow_bottom();
            return;
        }
        if self.body_area.contains(at) && self.help.is_none() && self.shell.is_none() {
            if let Some(off) = crate::select::offset_at(
                &self.body_plain,
                self.body_area,
                col,
                row,
                self.body_wrap,
                self.body_row_off,
            ) {
                self.body_sel = Some(crate::select::Selection {
                    anchor: off,
                    cursor: off,
                });
                return;
            }
        } else {
            self.body_sel = None;
        }
        if self.theme_switch_area.contains(at) {
            self.open_theme_picker();
            return;
        }
        if let Some(&(_, mode)) = self.theme_areas.iter().find(|(r, _)| r.contains(at)) {
            self.set_theme_mode(mode);
            return;
        }
        if self.account_area.contains(at) {
            self.open_account_picker();
            return;
        }
        let a = self.sidebar_area;
        if col > a.x
            && col < a.x + a.width.saturating_sub(1)
            && row > a.y
            && row < a.y + a.height.saturating_sub(1)
        {
            let idx = (row - a.y - 1) as usize;
            if idx < self.bots.len() {
                self.select_bot(idx);
            }
        }
    }

    pub fn scroll_up(&mut self, n: u16) {
        if n == 0 {
            return;
        }
        self.follow = false;
        self.scroll = self.scroll.saturating_add(n);
        self.pinned_off = None; // recompute from scroll on next draw
    }
    pub fn scroll_down(&mut self, n: u16) {
        if n == 0 {
            return;
        }
        self.scroll = self.scroll.saturating_sub(n);
        self.pinned_off = None;
        if self.scroll == 0 {
            self.follow_bottom();
        }
    }

    /// Jump to the newest content and resume sticking to the bottom.
    pub fn follow_bottom(&mut self) {
        self.scroll = 0;
        self.follow = true;
        self.pinned_off = None;
        self.unread = 0;
    }

    /// Apply layout totals. Follow (stick to bottom) only when the distance-from-bottom
    /// scroll is exactly 0 — the max absolute offset. Otherwise pin the absolute top
    /// offset so appends/rewraps leave the lines on screen unmoved.
    pub fn apply_body_layout(&mut self, total_rows: u16, inner_h: u16) {
        let max_off = total_rows.saturating_sub(inner_h);
        let prev_max = self.body_rows.saturating_sub(self.body_inner_h);
        // Exact bottom only: no "within a line" tolerance.
        if self.scroll == 0 {
            self.follow = true;
            self.pinned_off = None;
            self.unread = 0;
            self.scroll = 0;
        } else {
            self.follow = false;
            let scroll = match self.pinned_off {
                Some(off) => max_off.saturating_sub(off.min(max_off)),
                None => anchor_scroll(self.scroll, prev_max, max_off, false),
            };
            let off = max_off.saturating_sub(scroll);
            self.pinned_off = Some(off);
            self.scroll = scroll;
        }
        self.body_rows = total_rows;
        self.body_inner_h = inner_h;
    }
}

/// Pure helper: keep the absolute top offset when not following.
pub fn anchor_scroll(scroll: u16, prev_max_off: u16, new_max_off: u16, follow: bool) -> u16 {
    if follow {
        return 0;
    }
    let off = prev_max_off.saturating_sub(scroll.min(prev_max_off));
    new_max_off.saturating_sub(off)
}

fn entry_from_msg(m: &Message) -> Entry {
    Entry {
        kind: if m.role == Role::User {
            EntryKind::User
        } else {
            EntryKind::Bot
        },
        text: m.text.clone(),
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const WAIT: Duration = Duration::from_secs(5);

    pub fn test_app(rt: &tokio::runtime::Runtime) -> (App, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path().join("gb"), None).unwrap();
        let mut app = App::new(store, rt.handle().clone(), true).unwrap();
        // The repo's default.toml, copied into the temp data dir (never the real
        // ~/.grokbot-tui, never a download).
        let dir = app.themes_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(repo_themes().join("default.toml"), dir.join("default.toml")).unwrap();
        app.init_themes();
        // Never touch the real desktop app from tests.
        app.desktop_lister = Arc::new(|| {
            Err(gw::Error::Desktop {
                code: "TEST".into(),
                message: "no lister in tests".into(),
            })
        });
        // Keep toast overlays off so layout/snapshot tests stay stable.
        app.notices_board.settings.dnd = true;
        (app, d)
    }

    /// The repo's shipped themes (themes/default.toml).
    pub fn repo_themes() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("themes")
    }

    pub fn type_line(app: &mut App, s: &str) {
        app.input = s.into();
        app.submit();
    }

    /// Run the event loop until idle (and once more after `extra` for delayed replies).
    pub fn settle(app: &mut App) {
        for _ in 0..5 {
            app.tick();
            app.wait_idle(WAIT);
        }
    }

    pub fn mock_login(app: &mut App, tok: &str) {
        type_line(app, "/login gateway https://box.cursorvm.com");
        type_line(app, tok);
        settle(app);
    }

    fn texts(app: &App) -> Vec<String> {
        app.current_log().iter().map(|e| e.text.clone()).collect()
    }

    #[test]
    fn ui_ticks_do_not_trigger_extra_gateway_polls() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        settle(&mut app);
        assert!(app.active_bot().is_some());
        app.poll.next = Instant::now() + POLL_IDLE;
        let next = app.poll.next;
        for _ in 0..20 {
            app.tick();
            assert!(!app.poll.inflight, "UI tick started a gateway poll early");
        }
        assert_eq!(app.poll.next, next);
        assert!(UI_TICK < POLL_FAST);
    }

    #[test]
    fn anchor_scroll_keeps_absolute_offset_when_content_grows() {
        // Scrolled up 10 from a max of 50 → absolute off = 40.
        // Content grows so max becomes 60 → keep off 40 → scroll = 20.
        assert_eq!(crate::app::anchor_scroll(10, 50, 60, false), 20);
        assert_eq!(crate::app::anchor_scroll(0, 50, 60, true), 0);
        assert_eq!(
            crate::app::anchor_scroll(0, 50, 60, false),
            10,
            "at bottom stays at bottom offset"
        );
    }

    #[test]
    fn scrolled_up_stays_put_when_notices_and_messages_arrive() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        // Give the feed enough height that scrolling up is possible.
        if let Some(bot) = app.active_bot().map(|b| b.id.clone()) {
            for i in 0..40 {
                app.store
                    .append_message(
                        &bot,
                        &crate::api::Message {
                            role: crate::api::Role::Bot,
                            text: format!("line {i} {}", "x".repeat(20)),
                            remote_id: Some(format!("m{i}")),
                            attachments: vec![],
                        },
                    )
                    .unwrap();
            }
            app.reload_bot_log(&bot);
        }
        app.follow_bottom();
        app.apply_body_layout(80, 10); // max_off = 70
        assert!(app.follow);
        app.scroll_up(20);
        assert!(!app.follow);
        assert_eq!(app.scroll, 20);
        app.apply_body_layout(80, 10);
        let pinned = app.pinned_off;
        assert_eq!(pinned, Some(50)); // 70 - 20
        // A notice must not jump to the bottom.
        app.info("toast while reading".into());
        assert!(!app.follow, "notice must not re-enable follow");
        assert_eq!(app.unread, 1);
        app.apply_body_layout(85, 10); // +5 rows
        assert_eq!(
            app.pinned_off,
            Some(50),
            "viewport stays on the same content"
        );
        assert_eq!(app.scroll, 25); // 75 - 50
        // Returning to the bottom clears the badge.
        app.follow_bottom();
        assert!(app.follow);
        assert_eq!(app.unread, 0);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn one_line_above_bottom_does_not_follow_on_new_content() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.follow_bottom();
        app.apply_body_layout(80, 10); // max_off = 70, scroll = 0
        assert!(app.follow);
        // Exactly one wrapped row above the bottom — must NOT auto-follow.
        app.scroll_up(1);
        assert!(!app.follow);
        assert_eq!(app.scroll, 1);
        app.apply_body_layout(80, 10);
        let pinned = app.pinned_off.expect("pinned");
        assert_eq!(pinned, 69); // max_off 70 - 1
        app.info("incoming while almost at bottom".into());
        assert_eq!(app.unread, 1);
        assert!(!app.follow);
        app.apply_body_layout(90, 10); // +10 rows below
        assert_eq!(
            app.pinned_off,
            Some(69),
            "max-1 must keep the same lines on screen"
        );
        assert_eq!(app.scroll, 11); // new max_off 80 - 69
        assert!(!app.follow);
        assert_ne!(app.scroll, 0);
    }

    #[test]
    fn follow_when_at_bottom_tracks_new_content() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.follow_bottom();
        app.apply_body_layout(30, 10);
        assert_eq!(app.scroll, 0);
        app.info("new".into());
        assert_eq!(app.unread, 0, "following: notices do not count as unread");
        app.apply_body_layout(40, 10);
        assert!(app.follow);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn sending_a_message_and_switching_bot_reenable_follow() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.scroll_up(5);
        assert!(!app.follow);
        app.unread = 3;
        app.input = "hello".into();
        app.submit();
        assert!(app.follow);
        assert_eq!(app.unread, 0);
        app.scroll_up(5);
        if !app.bots.is_empty() {
            app.select_bot(0);
            assert!(app.follow);
            assert_eq!(app.scroll, 0);
        }
    }

    #[test]
    fn feed_is_rebuilt_from_db_and_notices_stay_separate() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let bot = app.active_bot().unwrap().id.clone();
        type_line(&mut app, "hi from tui");
        // Appears immediately, and only because it is in SQLite.
        let db: Vec<String> = app
            .store
            .messages(&bot)
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect();
        let feed: Vec<String> = app.feed().iter().map(|e| e.text.clone()).collect();
        assert_eq!(feed, db);
        assert!(feed.contains(&"hi from tui".to_string()));
        // Notices never enter the feed or the DB.
        type_line(&mut app, "/whoami");
        assert!(!app.notices().is_empty());
        assert_eq!(app.feed().len(), db.len());
        assert!(
            app.store
                .messages(&bot)
                .unwrap()
                .iter()
                .all(|m| !m.text.contains("account:"))
        );
        settle(&mut app);
        let db: Vec<String> = app
            .store
            .messages(&bot)
            .unwrap()
            .iter()
            .map(|m| m.text.clone())
            .collect();
        let feed: Vec<String> = app.feed().iter().map(|e| e.text.clone()).collect();
        assert_eq!(feed, db);
    }

    #[test]
    fn help_search_in_modal() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/help keyring");
        let m = app.help.as_ref().unwrap();
        assert!(m.page.id.starts_with("search"));
        assert!(m.page.related.contains(&"login-desktop".to_string()));
        app.help_search_start();
        app.help
            .as_mut()
            .unwrap()
            .query
            .as_mut()
            .unwrap()
            .push_str("polls");
        app.help_search_submit();
        let m = app.help.as_ref().unwrap();
        assert!(m.page.related.contains(&"sync".to_string()));
        let first = m.page.related[0].clone();
        app.help_follow();
        assert_eq!(app.help.as_ref().unwrap().page.id, first);
        app.help_back();
        assert!(app.help.as_ref().unwrap().page.id.starts_with("search"));
    }

    #[test]
    fn saved_login_survives_quit_and_only_logout_removes_it() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mk = || App::new(Store::open(&dir, None).unwrap(), rt.handle().clone(), true).unwrap();
        let mut app = mk();
        mock_login(&mut app, "tok");
        type_line(&mut app, "/quit");
        assert!(app.should_quit);
        drop(app);
        for _ in 0..2 {
            let mut app = mk();
            app.init();
            settle(&mut app);
            assert!(app.user_label.is_some(), "restored without /login");
            drop(app);
        }
        let mut app = mk();
        app.init();
        settle(&mut app);
        type_line(&mut app, "hello");
        settle(&mut app);
        type_line(&mut app, "/memory add k v");
        let bot = app.active_bot().unwrap().id.clone();
        type_line(&mut app, "/logout");
        assert!(app.store.load_login().unwrap().is_none());
        assert!(!app.store.messages(&bot).unwrap().is_empty(), "chats kept");
        assert!(
            !app.store.list_memory(&bot).unwrap().is_empty(),
            "memory kept"
        );
        drop(app);
        let mut app = mk();
        app.init();
        settle(&mut app);
        assert!(app.user_label.is_none());
    }

    #[test]
    fn db_refresh_picks_up_external_writes_without_network() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        settle(&mut app);
        let bot = app.active_bot().unwrap().id.clone();
        app.poll.next = Instant::now() + POLL_IDLE_MAX;
        app.store
            .append_message(
                &bot,
                &Message {
                    role: Role::Bot,
                    text: "written elsewhere".into(),
                    remote_id: Some("x1".into()),
                    attachments: vec![],
                },
            )
            .unwrap();
        app.refresh_feed_from_db();
        assert!(app.feed().iter().any(|e| e.text == "written elsewhere"));
        assert!(!app.poll.inflight, "no gateway call");
    }

    #[test]
    fn http_stored_desktop_401_rereads_app_session_and_keeps_login() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .and(header("authorization", "Bearer old-gw"))
                .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "expired"})))
                .mount(&server)
                .await;
            Mock::given(path("/aiserver.v1.GrokBotService/EnsureSandBox"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "gatewayUrl": uri, "gatewayToken": "new-gw", "anyrunNetworkToken": "n"
                })))
                .mount(&server)
                .await;
            Mock::given(path("/api/listAgents"))
                .and(header("authorization", "Bearer new-gw"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])),
                )
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        app.store
            .save_login(&StoredLogin {
                method: LoginMethod::Desktop,
                session: Session::new(&uri, "old-gw"),
                cursor_token: None,
                desktop_entry: Some("acct2".into()),
            })
            .unwrap();
        let (reader, calls) = fake_reader(|_, entry| {
            Ok((
                gw::desktop::DesktopCredential::CursorToken("cur".into()),
                entry.map(str::to_string),
            ))
        });
        app.desktop_reader = reader;
        app.connect();
        settle(&mut app);
        assert!(app.user_label.is_some(), "restored via desktop re-read");
        assert_eq!(calls.lock().unwrap().len(), 1);
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(l.session.gateway_token, "new-gw");
        assert_eq!(
            l.desktop_entry.as_deref(),
            Some("acct2"),
            "entry choice kept"
        );
    }

    #[test]
    fn bot_and_account_pickers_switch_and_cancel() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        let start = app.active_bot().unwrap().id.clone();
        type_line(&mut app, "/bot");
        let p = app.picker.as_ref().expect("bare /bot opens the picker");
        assert_eq!(
            p.selected().unwrap().value,
            start,
            "starts on the current bot"
        );
        // Esc-equivalent cancel leaves everything as is.
        app.picker_cancel();
        assert_eq!(app.active_bot().unwrap().id, start);
        // Keyboard: move down one and confirm.
        app.open_bot_picker();
        let p = app.picker.as_mut().unwrap();
        let i = p.cursor;
        p.move_by(if i + 1 < p.items.len() { 1 } else { -1 });
        let want = p.selected().unwrap().value.clone();
        app.picker_confirm();
        assert_eq!(app.active_bot().unwrap().id, want);
        assert_ne!(want, start);

        type_line(&mut app, "/account add work");
        type_line(&mut app, "/account use");
        let p = app.picker.as_mut().expect("/account use opens the picker");
        assert!(p.items.iter().any(|i| i.value == "work"));
        let w = p.items.iter().position(|i| i.value == "work").unwrap();
        p.cursor = w;
        app.picker_confirm();
        assert_eq!(app.account().alias, "work");
    }

    #[test]
    fn shell_shows_output_in_modal_not_chat() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let before = app.feed().len();
        let log_before = texts(&app);
        type_line(&mut app, "/shell echo hi");
        let m = app.shell.as_ref().expect("modal opens immediately");
        assert!(m.running() && m.body().contains("running…"));
        settle(&mut app);
        let m = app.shell.as_ref().unwrap();
        assert_eq!(m.body(), "$ echo hi\nhi\n\n[exit 0]");
        assert!(!m.failed());
        assert_eq!(app.feed().len(), before, "not stored as chat");
        assert_eq!(texts(&app), log_before, "nothing echoed to chat/notices");
        app.shell_scroll(5);
        app.shell_scroll(-10);
        assert_eq!(app.shell.as_ref().unwrap().scroll, 0);
        app.close_shell();
        assert!(app.shell.is_none());

        type_line(&mut app, "/shell echo oops >&2; exit 3");
        settle(&mut app);
        let m = app.shell.as_ref().unwrap();
        assert!(m.body().contains("oops\n\n[exit 3]"), "{}", m.body());
        assert!(m.failed());

        type_line(&mut app, "/shell definitely-not-a-command-xyz");
        settle(&mut app);
        assert!(
            app.shell
                .as_ref()
                .unwrap()
                .body()
                .contains("[exit 127 (command not found?)]")
        );

        app.close_shell();
        type_line(&mut app, "/shell");
        assert!(app.shell.is_none());
        assert!(texts(&app).join("\n").contains("Usage: /shell"));
    }

    #[test]
    fn shell_never_calls_the_gateway() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])),
                )
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        let (reader, _calls) = fake_reader(move |_, _| {
            Ok((
                gw::desktop::DesktopCredential::Session(Session::new(uri.clone(), "d1")),
                Some("sandacct".into()),
            ))
        });
        app.desktop_reader = reader;
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        assert!(app.active_bot().is_some());
        // Only count requests made by /shell.
        rt.block_on(server.reset());
        rt.block_on(async {
            Mock::given(path("/api/sendPrompt"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"accepted": true})))
                .expect(0)
                .mount(&server)
                .await;
        });
        type_line(&mut app, "/shell echo hi");
        type_line(&mut app, "/sh echo again");
        settle(&mut app);
        assert!(app.shell.as_ref().unwrap().body().contains("again"));
        let reqs = rt.block_on(server.received_requests()).unwrap();
        assert!(
            !reqs.iter().any(|r| r.url.path().ends_with("/sendPrompt")),
            "no sendPrompt from /shell"
        );
        rt.block_on(server.verify());
    }

    #[test]
    fn shell_output_is_capped() {
        let big = (0..2000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (t, trunc) = cap_output(&big);
        assert!(trunc);
        assert_eq!(t.lines().count(), SHELL_MAX_LINES);
        assert_eq!(cap_output("a\nb"), ("a\nb".into(), false));
    }

    #[test]
    fn mouse_toggle_and_copy() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        assert!(app.mouse_capture);
        app.toggle_mouse_capture();
        assert!(!app.mouse_capture);
        type_line(&mut app, "/mouse on");
        assert!(app.mouse_capture);
        type_line(&mut app, "/mouse off");
        assert!(!app.mouse_capture);
        type_line(&mut app, "/mouse");
        assert!(app.mouse_capture, "bare /mouse toggles");
        type_line(&mut app, "/mouse sideways");
        assert!(texts(&app).join("\n").contains("Usage: /mouse"));

        mock_login(&mut app, "tok");
        type_line(&mut app, "/copy");
        assert!(app.pending_clipboard.is_none());
        assert!(texts(&app).join("\n").contains("No message to copy"));
        let bot = app.active_bot().unwrap().id.clone();
        app.store
            .merge_remote(
                &bot,
                &[
                    crate::api::Message {
                        role: crate::api::Role::Bot,
                        text: "answer one".into(),
                        remote_id: Some("b1".into()),
                        attachments: vec![],
                    },
                    crate::api::Message {
                        role: crate::api::Role::User,
                        text: "q".into(),
                        remote_id: Some("u1".into()),
                        attachments: vec![],
                    },
                ],
                true,
            )
            .unwrap();
        app.refresh_feed_from_db();
        type_line(&mut app, "/copy");
        assert_eq!(app.pending_clipboard.as_deref(), Some("answer one"));
    }

    #[test]
    fn bot_new_creates_selects_and_delete_confirms() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        type_line(&mut app, "/bot new Newbie");
        settle(&mut app);
        let b = app.active_bot().expect("new bot is selected").clone();
        assert_eq!(b.name, "Newbie");
        assert!(app.bots.iter().any(|x| x.id == b.id), "sidebar has it");

        // Create error is shown, nothing added.
        let n = app.bots.len();
        type_line(&mut app, "/bot new reject");
        settle(&mut app);
        assert_eq!(app.bots.len(), n);
        assert!(
            texts(&app)
                .iter()
                .any(|t| t.contains("Could not create bot"))
        );

        // Delete: picker defaults to Cancel; confirming that keeps the bot.
        type_line(&mut app, "/bot delete Newbie");
        let p = app.picker.as_ref().expect("delete asks first");
        assert_eq!(p.selected().unwrap().value, "cancel");
        app.picker_confirm();
        settle(&mut app);
        assert!(app.bots.iter().any(|x| x.id == b.id));
        // Esc also cancels.
        type_line(&mut app, "/bot delete Newbie");
        app.picker_cancel();
        settle(&mut app);
        assert!(app.bots.iter().any(|x| x.id == b.id));

        // Choose Delete.
        type_line(&mut app, "/bot delete");
        let p = app.picker.as_mut().unwrap();
        p.cursor = p
            .items
            .iter()
            .position(|i| i.value.starts_with("delete:"))
            .unwrap();
        app.picker_confirm();
        settle(&mut app);
        assert!(!app.bots.iter().any(|x| x.id == b.id), "deleted bot gone");
        assert!(!app.all_bots.iter().any(|x| x.id == b.id));

        type_line(&mut app, "/bot delete Nope");
        assert!(app.picker.is_none());
    }

    #[test]
    fn help_opens_modal_not_chat_and_navigates() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let before = texts(&app);
        type_line(&mut app, "/help");
        assert_eq!(texts(&app), before, "help must not touch chat history");
        assert_eq!(app.help.as_ref().unwrap().page.id, "overview");
        // follow the 2nd related link, then go back
        app.help_select(1);
        let target = app.help.as_ref().unwrap().page.related[1].clone();
        app.help_follow();
        assert_eq!(app.help.as_ref().unwrap().page.id, target);
        app.help_back();
        assert_eq!(app.help.as_ref().unwrap().page.id, "overview");
        app.help_scroll(-5);
        assert_eq!(app.help.as_ref().unwrap().scroll, 0);
        app.close_help();
        assert!(app.help.is_none());

        type_line(&mut app, "/help login desktop");
        assert_eq!(app.help.as_ref().unwrap().page.id, "login-desktop");
        type_line(&mut app, "/help bot");
        assert_eq!(app.help.as_ref().unwrap().page.id, "cmd-bot");
        assert!(app.help.as_ref().unwrap().page.body.contains("Examples"));
        app.close_help();

        type_line(&mut app, "/login");
        assert_eq!(app.help.as_ref().unwrap().page.id, "login");
        assert_eq!(app.mode, InputMode::Normal);
        app.close_help();

        type_line(&mut app, "/help logn");
        assert!(
            texts(&app)
                .last()
                .unwrap()
                .contains("Did you mean /help login")
        );
        type_line(&mut app, "/hisotry");
        assert!(
            texts(&app)
                .last()
                .unwrap()
                .contains("Did you mean /history")
        );
    }

    #[test]
    fn help_never_shows_token_and_whoami_shows_version() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "sekrit-token-123");
        for name in crate::help::all_names(&app.registry) {
            assert!(app.open_help(&name));
            assert!(
                !app.help
                    .as_ref()
                    .unwrap()
                    .page
                    .body
                    .contains("sekrit-token-123")
            );
        }
        app.close_help();
        type_line(&mut app, "/whoami");
        let out = texts(&app).join("\n");
        assert!(out.contains("client version: "), "{out}");
        assert!(!out.contains("sekrit-token-123"));
    }

    #[test]
    fn team_chomp_send_and_history_via_viewer_session() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        // list_bots bound Chomp's viewer session; select and chat.
        let i = app
            .bots
            .iter()
            .position(|b| b.name == "Chomp")
            .expect("Chomp");
        assert!(app.bots[i].team);
        assert_eq!(app.bots[i].viewer_session_id.as_deref(), Some("vs-chomp"));
        app.select_bot(i);
        type_line(&mut app, "hello chomp");
        settle(&mut app);
        // Without a surfaced send error, the pending/accepted path ran.
        let t = texts(&app);
        assert!(
            t.iter().any(|x| x.contains("hello chomp")),
            "prompt visible: {t:?}"
        );
        assert!(
            !t.iter()
                .any(|x| x.contains("sessionId") || x.contains("Send failed")),
            "should not fail session bind: {t:?}"
        );
        // Simulate lost in-memory session map; bind from Bot must repair on next send.
        if let Some(api) = app.api.clone() {
            api.bind_viewer_session(&app.bots[i].id, None);
        }
        type_line(&mut app, "again");
        settle(&mut app);
        let t = texts(&app);
        assert!(t.iter().any(|x| x.contains("again")), "{t:?}");
        assert!(
            !t.iter()
                .any(|x| x.contains("team agent requires sessionId")),
            "re-bind from Bot.viewer_session_id should repair: {t:?}"
        );
    }

    #[test]
    fn mock_gateway_login_send_poll_logout() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "hello");
        assert_eq!(app.current_log().last().unwrap().kind, EntryKind::Error);

        type_line(&mut app, "/login gateway");
        assert_eq!(app.mode, InputMode::GatewayUrl);
        type_line(&mut app, "https://box.cursorvm.com");
        assert_eq!(app.mode, InputMode::GatewayToken);
        type_line(&mut app, "secret");
        settle(&mut app);
        assert!(
            app.user_label
                .as_deref()
                .unwrap()
                .contains("box.cursorvm.com")
        );
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(
            (l.method, l.session.gateway_token.as_str()),
            (LoginMethod::Gateway, "secret")
        );
        assert_eq!(
            app.bots.len(),
            4,
            "hidden bot filtered; Chomp team bot visible"
        );
        assert_eq!(app.all_bots.len(), 5);

        type_line(&mut app, "/bot coder");
        type_line(&mut app, "hi there");
        settle(&mut app);
        assert!(app.is_typing(), "awaiting reply");
        std::thread::sleep(Duration::from_millis(350));
        app.poll_now();
        settle(&mut app);
        assert!(!app.is_typing());
        let t = texts(&app);
        assert_eq!(
            t.iter().filter(|s| *s == "hi there").count(),
            1,
            "pending message adopted, not duplicated: {t:?}"
        );
        assert!(t.contains(&"echo: hi there".to_string()));
        assert_eq!(app.store.messages("coder").unwrap().len(), 2);

        type_line(&mut app, "/logout");
        assert!(app.user_label.is_none());
        assert!(app.store.load_login().unwrap().is_none());
        assert_eq!(app.store.messages("coder").unwrap().len(), 2, "chats kept");
    }

    #[test]
    fn mock_cursor_login_and_bad_tokens() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/login cursor");
        assert_eq!(app.mode, InputMode::CursorToken);
        type_line(&mut app, "bad");
        settle(&mut app);
        assert!(app.user_label.is_none());
        assert!(app.store.load_login().unwrap().is_none());
        type_line(&mut app, "/login cursor");
        type_line(&mut app, "cur");
        settle(&mut app);
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(l.method, LoginMethod::Cursor);
        assert_eq!(l.cursor_token.as_deref(), Some("cur"));
        type_line(&mut app, "/nope");
        assert!(
            app.current_log()
                .last()
                .unwrap()
                .text
                .contains("Unknown command")
        );
    }

    #[test]
    fn unknown_send_resolved_by_acceptance_status() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        // Never reached the gateway → not-found: placeholder removed, safe to resend.
        type_line(&mut app, "lost");
        settle(&mut app);
        assert!(texts(&app).iter().any(|t| t.contains("Safe to resend")));
        assert!(
            !app.store
                .messages("helper")
                .unwrap()
                .iter()
                .any(|m| m.text == "lost")
        );
        assert!(!texts(&app).iter().any(|t| t == CHECKING_NOTICE));
        // Gateway refused it after all → rejected with its code.
        type_line(&mut app, "refused-later");
        settle(&mut app);
        assert!(
            texts(&app)
                .iter()
                .any(|t| t.contains("Not delivered (rejected by the gateway: mock_refused)"))
        );
        assert!(
            !app.store
                .messages("helper")
                .unwrap()
                .iter()
                .any(|m| m.text == "refused-later")
        );
    }

    #[test]
    fn rejected_and_unknown_delivery() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        type_line(&mut app, "reject");
        settle(&mut app);
        assert!(
            texts(&app)
                .iter()
                .any(|t| t.contains("Not delivered (rejected)"))
        );
        assert!(
            app.store.messages("helper").unwrap().is_empty(),
            "rejected pending removed"
        );
        assert!(!app.is_typing());

        // 503 but the prompt landed: promptAcceptanceStatus says accepted → no warning.
        type_line(&mut app, "unknown");
        settle(&mut app);
        assert!(!texts(&app).iter().any(|t| t.contains("Delivery unknown")));
        assert!(!texts(&app).iter().any(|t| t == CHECKING_NOTICE));
        // The transcript check finds the prompt that did land; no duplicate.
        let msgs = app.store.messages("helper").unwrap();
        assert_eq!(msgs.iter().filter(|m| m.text == "unknown").count(), 1);
        assert!(
            !msgs[0]
                .remote_id
                .as_deref()
                .unwrap()
                .starts_with("pending:")
        );
    }

    #[test]
    fn chat_restored_after_restart_and_prefs_memory() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mk = || App::new(Store::open(&dir, None).unwrap(), rt.handle().clone(), true).unwrap();
        let mut app = mk();
        mock_login(&mut app, "t");
        type_line(&mut app, "/bot scribe");
        type_line(&mut app, "note this");
        settle(&mut app);
        type_line(&mut app, "/memory add topic rust tui");
        type_line(&mut app, "/prefs set ui.sidebar_width 30");
        type_line(&mut app, "/prefs set show_hidden_bots true");
        assert_eq!(app.bots.len(), 5); // includes hidden Secret + team Chomp
        drop(app);

        let mut app = mk(); // fresh mock backend, same DB
        app.init();
        settle(&mut app);
        assert!(app.user_label.is_some(), "stored login reused");
        assert_eq!(app.active_bot().unwrap().id, "scribe");
        assert!(
            texts(&app).contains(&"note this".to_string()),
            "chat restored from DB"
        );
        assert_eq!(app.config.sidebar_width, 30);
        type_line(&mut app, "/memory");
        assert!(
            app.current_log()
                .last()
                .unwrap()
                .text
                .contains("topic: rust tui")
        );
        type_line(&mut app, "/memory clear");
        assert!(app.store.list_memory("scribe").unwrap().is_empty());
        type_line(&mut app, "/prefs bogus");
        assert_eq!(app.current_log().last().unwrap().kind, EntryKind::Error);
    }

    #[test]
    fn accounts_switch_and_isolate_in_app() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        type_line(&mut app, "personal msg");
        settle(&mut app);
        type_line(&mut app, "/memory add who personal");

        type_line(&mut app, "/account add work");
        type_line(&mut app, "/account use work");
        settle(&mut app);
        assert_eq!(app.account().alias, "work");
        assert!(app.user_label.is_none());
        assert!(app.bots.is_empty());
        mock_login(&mut app, "w");
        assert!(!texts(&app).contains(&"personal msg".to_string()));
        type_line(&mut app, "/memory");
        assert!(!app.current_log().last().unwrap().text.contains("personal"));
        type_line(&mut app, "/logout");

        type_line(&mut app, "/account use default");
        settle(&mut app);
        assert!(
            app.user_label.is_some(),
            "default login survives work logout"
        );
        assert!(texts(&app).contains(&"personal msg".to_string()));

        // Removal is confirmed in the pick list; Cancel is the default.
        type_line(&mut app, "/account remove work");
        assert_eq!(
            app.picker.as_ref().unwrap().selected().unwrap().value,
            "cancel"
        );
        app.picker_confirm();
        assert!(app.store.find_account("work").unwrap().is_some());
        type_line(&mut app, "/account remove work");
        app.picker_cancel();
        assert!(app.store.find_account("work").unwrap().is_some());
        assert!(texts(&app).iter().any(|t| t.contains("removal cancelled")));
        type_line(&mut app, "/account remove work");
        app.picker.as_mut().unwrap().move_by(1);
        app.picker_confirm();
        assert!(app.store.find_account("work").unwrap().is_none());
        type_line(&mut app, "/account rename default home");
        assert_eq!(app.account().alias, "home");
    }

    #[test]
    fn stale_results_after_switch_are_ignored() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/account add b");
        type_line(&mut app, "/login gateway https://box.cursorvm.com");
        type_line(&mut app, "t"); // login in flight for 'default'
        type_line(&mut app, "/account use b");
        settle(&mut app);
        assert_eq!(app.account().alias, "b");
        assert!(
            app.user_label.is_none(),
            "default's login must not leak into b"
        );
        assert!(app.store.load_login().unwrap().is_none());
    }

    #[test]
    fn click_account_header_and_bots() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        app.sidebar_area = Rect::new(0, 2, 20, 10);
        app.click(3, 5);
        assert_eq!(app.active, Some(2));
        app.click(3, 2);
        assert_eq!(app.active, Some(2));
        type_line(&mut app, "/account add b");
        app.account_area = Rect::new(0, 1, 24, 1);
        // Clicking the @account switcher in the header opens the account pick list.
        app.click(2, 1);
        let p = app
            .picker
            .as_mut()
            .expect("header click opens the account picker");
        assert_eq!(p.selected().unwrap().value, "default");
        let b = p.items.iter().position(|i| i.value == "b").unwrap();
        p.cursor = b;
        app.picker_confirm();
        assert_eq!(app.account().alias, "b");
    }

    #[test]
    fn disallowed_gateway_url_rejected_before_any_request() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.mock = false;
        type_line(&mut app, "/login gateway https://evil.example");
        assert_eq!(app.mode, InputMode::Normal);
        assert!(
            app.current_log()
                .last()
                .unwrap()
                .text
                .contains("Rejected gateway URL")
        );
        type_line(&mut app, "/login gateway http://127.0.0.1:9");
        assert!(
            app.current_log()
                .last()
                .unwrap()
                .text
                .contains("GROKBOT_ALLOW_LOCAL_GATEWAY")
        );
    }

    #[test]
    fn ambiguous_desktop_errors_point_to_tui_commands() {
        let err = |message: &str| gw::Error::Desktop {
            code: "AMBIGUOUS_ENTRIES".into(),
            message: message.into(),
        };
        assert_eq!(
            desktop_error_text(&err(
                "The desktop app has several saved sessions (personal, work). Specify which one to load by its entry name."
            )),
            "The desktop app has several saved sessions (personal, work). Choose one with /login desktop <name>."
        );
        let accounts = desktop_error_text(&err(
            "The desktop app has several Cursor accounts (1) aaaa1111…, 2) bbbb2222…). Specify which one to use by its number, its full account key, or a unique key prefix of at least 4 characters.",
        ));
        assert!(accounts.starts_with(
            "The desktop app has several Cursor accounts (1) aaaa1111…, 2) bbbb2222…)."
        ));
        assert!(
            accounts.contains("/login desktop <number>")
                && accounts.contains("/account add <alias>")
        );
        assert!(!accounts.contains("Specify which"));
        // Other desktop errors are shown unchanged.
        let other = gw::Error::Desktop {
            code: "NOT_SIGNED_IN".into(),
            message: "Sign in from the desktop app first.".into(),
        };
        assert_eq!(
            desktop_error_text(&other),
            "Sign in from the desktop app first."
        );
    }

    // ---- real GatewayClient against a local mock HTTP server

    fn real_app(rt: &tokio::runtime::Runtime, server: &MockServer) -> (App, tempfile::TempDir) {
        let (mut app, d) = test_app(rt);
        app.mock = false;
        app.opts = ClientOptions {
            policy: Policy::local_testing(),
            backend_base: server.uri(),
            ..Default::default()
        };
        (app, d)
    }

    #[test]
    fn http_gateway_login_send_and_poll() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .and(header("authorization", "Bearer gw"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"agents": [
                    {"id": "a1", "name": "Helper"}, {"id": "h", "name": "Hidden", "isHiddenFromSidebar": true}]})))
                .mount(&server)
                .await;
            Mock::given(path("/api/sendPrompt"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"messageId": "m1"})))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": [
                    {"id": "e1", "kind": "user-prompt", "prompt": "hello"},
                    {"id": "e2", "kind": "send-message", "message": {"type": "text", "content": "hi back"}}]})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        type_line(&mut app, &format!("/login gateway {}", server.uri()));
        type_line(&mut app, "gw");
        settle(&mut app);
        assert_eq!(
            app.bots.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            vec!["a1"]
        );
        assert_eq!(
            app.store.load_login().unwrap().unwrap().session.gateway_url,
            server.uri()
        );
        type_line(&mut app, "hello");
        settle(&mut app);
        app.poll_now();
        settle(&mut app);
        let t = texts(&app);
        assert_eq!(t.iter().filter(|s| *s == "hello").count(), 1, "{t:?}");
        assert!(t.contains(&"hi back".to_string()));
        assert!(!app.is_typing());
    }

    #[test]
    fn http_cursor_login_and_401_reexchange_persists() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/aiserver.v1.GrokBotService/EnsureSandBox"))
                .and(header("authorization", "Bearer cur"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"gatewayUrl": uri, "gatewayToken": "gw1", "anyrunNetworkToken": "n"})))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            Mock::given(path("/aiserver.v1.GrokBotService/EnsureSandBox"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"gatewayUrl": uri, "gatewayToken": "gw2"})))
                .mount(&server)
                .await;
            Mock::given(path("/api/listAgents"))
                .and(header("authorization", "Bearer gw1"))
                .and(header("x-anyrun-network-token", "n"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer gw1"))
                .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "expired"})))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer gw2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": [{"id": "e1", "kind": "send-message", "message": {"content": "welcome"}}]})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        type_line(&mut app, "/login cursor");
        type_line(&mut app, "cur");
        settle(&mut app);
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(l.method, LoginMethod::Cursor);
        // listAgents only matched with the anyrun header from EnsureSandBox, so login proves it was sent.
        assert_eq!(l.cursor_token.as_deref(), Some("cur"));
        // First poll gets 401 -> re-exchange -> retry succeeds; new token persisted.
        settle(&mut app);
        assert!(texts(&app).contains(&"welcome".to_string()));
        assert_eq!(
            app.store
                .load_login()
                .unwrap()
                .unwrap()
                .session
                .gateway_token,
            "gw2"
        );
    }

    type Calls = Arc<std::sync::Mutex<Vec<Option<String>>>>;

    /// A fake desktop reader that records calls (the entry asked for) and returns `f(n)`.
    fn fake_reader(
        f: impl Fn(usize, Option<&str>) -> gw::Result<DesktopRead> + Send + Sync + 'static,
    ) -> (DesktopReader, Calls) {
        let calls: Calls = Default::default();
        let c = calls.clone();
        let r: DesktopReader = Arc::new(move |entry, _policy| {
            let mut v = c.lock().unwrap();
            v.push(entry.map(str::to_string));
            f(v.len(), entry)
        });
        (r, calls)
    }

    #[test]
    fn desktop_several_accounts_opens_picker_and_logs_in_with_choice() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let (reader, calls) = fake_reader(|_, entry| match entry {
            None => Err(gw::Error::Desktop {
                code: "AMBIGUOUS_ENTRIES".into(),
                message: "several".into(),
            }),
            Some(k) => Ok((
                gw::desktop::DesktopCredential::Session(Session::new(
                    "https://box.cursorvm.com",
                    "desk",
                )),
                Some(k.to_string()),
            )),
        });
        app.desktop_reader = reader;
        app.desktop_lister = Arc::new(|| {
            Ok(vec![
                ("1) aaaa1111… (active in app)".into(), "aaaa1111key".into()),
                ("2) bbbb2222…".into(), "bbbb2222key".into()),
            ])
        });
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        let p = app
            .picker
            .as_mut()
            .expect("several desktop accounts → pick list");
        assert_eq!(p.items.len(), 2);
        p.move_by(1);
        app.picker_confirm();
        settle(&mut app);
        assert_eq!(
            calls.lock().unwrap().last().unwrap().as_deref(),
            Some("bbbb2222key")
        );
        assert!(app.user_label.is_some());
        assert_eq!(
            app.store
                .load_login()
                .unwrap()
                .unwrap()
                .desktop_entry
                .as_deref(),
            Some("bbbb2222key")
        );
    }

    #[test]
    fn desktop_login_only_reads_when_asked_and_handles_entries() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let (reader, calls) = fake_reader(|_, entry| {
            match entry {
            None => Err(gw::Error::Desktop {
                code: "AMBIGUOUS_ENTRIES".into(),
                message: "The desktop app has several saved sessions (personal, work). Specify which one to load by its entry name."
                    .into(),
            }),
            Some(name) => Ok((
                gw::desktop::DesktopCredential::Session(Session::new(
                    "https://box.cursorvm.com",
                    "desk",
                )),
                Some(name.to_string()),
            )),
        }
        });
        app.desktop_reader = reader;
        app.connect();
        mock_login(&mut app, "gw");
        assert!(
            calls.lock().unwrap().is_empty(),
            "never read outside /login desktop"
        );
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        assert!(
            texts(&app)
                .iter()
                .any(|t| t.contains("Choose one with /login desktop"))
        );
        assert_eq!(
            app.store.load_login().unwrap().unwrap().method,
            LoginMethod::Gateway
        );
        type_line(&mut app, "/login desktop work");
        settle(&mut app);
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(l.method, LoginMethod::Desktop);
        assert_eq!(l.desktop_entry.as_deref(), Some("work"));
        assert_eq!(l.session.gateway_token, "desk");
        assert_eq!(l.cursor_token, None);
        assert_eq!(*calls.lock().unwrap(), vec![None, Some("work".to_string())]);
    }

    #[test]
    fn desktop_login_errors_are_clear() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let (reader, _calls) = fake_reader(|_, _| {
            Err(gw::Error::Desktop {
                code: "SECRET_TOOL_MISSING".into(),
                message: "secret-tool is not installed".into(),
            })
        });
        app.desktop_reader = reader;
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        assert!(
            texts(&app)
                .iter()
                .any(|t| t.contains("Login failed: secret-tool is not installed"))
        );
        assert!(app.store.load_login().unwrap().is_none());
    }

    #[test]
    fn http_desktop_login_rereads_on_401() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer d1"))
                .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "expired"})))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer d2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": [{"id": "e1", "kind": "send-message", "message": {"content": "welcome"}}]})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        let (reader, calls) = fake_reader(move |n, _| {
            Ok((
                gw::desktop::DesktopCredential::Session(Session::new(
                    uri.clone(),
                    if n == 1 { "d1" } else { "d2" },
                )),
                Some("sandacct".into()),
            ))
        });
        app.desktop_reader = reader;
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        settle(&mut app);
        assert!(texts(&app).contains(&"welcome".to_string()));
        assert_eq!(calls.lock().unwrap().len(), 2, "one automatic re-read");
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(
            (l.method, l.session.gateway_token.as_str()),
            (LoginMethod::Desktop, "d2")
        );
    }

    #[test]
    fn http_desktop_login_rereads_on_417() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer d1"))
                .respond_with(ResponseTemplate::new(417))
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .and(header("authorization", "Bearer d2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": [{"id": "e1", "kind": "send-message", "message": {"content": "welcome"}}]})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        let (reader, calls) = fake_reader(move |n, _| {
            Ok((
                gw::desktop::DesktopCredential::Session(Session::new(
                    uri.clone(),
                    if n == 1 { "d1" } else { "d2" },
                )),
                Some("sandacct".into()),
            ))
        });
        app.desktop_reader = reader;
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        settle(&mut app);
        assert!(texts(&app).contains(&"welcome".to_string()));
        assert_eq!(calls.lock().unwrap().len(), 2, "one automatic re-read");
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(
            (l.method, l.session.gateway_token.as_str()),
            (LoginMethod::Desktop, "d2")
        );
    }

    #[test]
    fn http_desktop_sand_exchanges_cursor_token_like_login_cursor() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        let uri = server.uri();
        rt.block_on(async {
            Mock::given(path("/aiserver.v1.GrokBotService/EnsureSandBox"))
                .and(header("authorization", "Bearer sand-cur"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "gatewayUrl": uri, "gatewayToken": "gw-from-sand", "anyrunNetworkToken": "n"
                })))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path("/api/listAgents"))
                .and(header("authorization", "Bearer gw-from-sand"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])),
                )
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        let (reader, calls) = fake_reader(|_, entry| {
            Ok((
                gw::desktop::DesktopCredential::CursorToken("sand-cur".into()),
                Some(entry.unwrap_or("acctkey").to_string()),
            ))
        });
        app.desktop_reader = reader;
        type_line(&mut app, "/login desktop");
        settle(&mut app);
        let l = app.store.load_login().unwrap().unwrap();
        assert_eq!(l.method, LoginMethod::Desktop);
        assert_eq!(l.session.gateway_token, "gw-from-sand");
        assert_eq!(
            l.cursor_token, None,
            "sand Cursor token is not persisted; re-read on 401"
        );
        assert_eq!(l.desktop_entry.as_deref(), Some("acctkey"));
        assert_eq!(calls.lock().unwrap().len(), 1);
        assert_eq!(app.bots.len(), 1);
    }

    #[test]
    fn http_5xx_send_is_unknown_and_checks_transcript() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let server = rt.block_on(MockServer::start());
        rt.block_on(async {
            Mock::given(path("/api/listAgents"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([{"id": "a1", "name": "A"}])),
                )
                .mount(&server)
                .await;
            Mock::given(path("/api/sendPrompt"))
                .respond_with(
                    ResponseTemplate::new(502).set_body_json(json!({"error": "bad gateway"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"entries": []})))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            Mock::given(path("/api/getAgentTranscriptTail"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"entries": [{"id": "e1", "role": "user", "text": "did it land?"}]}),
                ))
                .mount(&server)
                .await;
        });
        let (mut app, _d) = real_app(&rt, &server);
        type_line(&mut app, &format!("/login gateway {}", server.uri()));
        type_line(&mut app, "gw");
        settle(&mut app);
        type_line(&mut app, "did it land?");
        settle(&mut app);
        assert!(texts(&app).iter().any(|t| t.contains("Delivery unknown")));
        let msgs = app.store.messages("a1").unwrap();
        assert_eq!(msgs.len(), 1, "never resent, no duplicate");
        assert_eq!(
            msgs[0].remote_id.as_deref(),
            Some("e1"),
            "transcript confirmed it landed"
        );
    }
}

#[cfg(test)]
mod perf_probe {
    use super::tests::*;
    use super::*;

    /// Timing, not a pass/fail check: `cargo test --release -- --ignored draw_cost --nocapture`.
    #[test]
    #[ignore = "timing probe"]
    fn react_shows_pills_under_the_newest_bot_message_and_toggles() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![
                Entry {
                    kind: EntryKind::User,
                    text: "hi".into(),
                },
                Entry {
                    kind: EntryKind::Bot,
                    text: "hello there".into(),
                },
            ],
        );
        let draw = |app: &mut App| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
            term.draw(|f| crate::ui::draw(f, app)).unwrap();
            crate::ui::tests::screen_rows(&term).join("\n")
        };
        app.input = "/react :tada:".into();
        app.submit();
        let s = draw(&mut app);
        assert!(s.contains("🎉"), "{s}");
        app.input = "/react 🎉".into();
        app.submit();
        assert!(!draw(&mut app).contains("🎉"));
        app.input = "/react nonsense_name_x".into();
        app.submit();
        assert!(
            app.notices()
                .iter()
                .any(|e| e.text.contains("Unknown emoji"))
        );
    }

    #[test]
    fn open_list_numbers_images_before_links() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![Entry {
                kind: EntryKind::Bot,
                text: "[docs](https://d.dev) ![cat](https://x/cat.png)".into(),
            }],
        );
        app.input = "/open list".into();
        app.submit();
        let t = &app.notices().last().unwrap().text;
        assert!(t.contains(" 1. 🖼 cat  https://x/cat.png"), "{t}");
        assert!(t.contains(" 2. 🔗 docs  https://d.dev"), "{t}");
        app.input = "/open 9".into();
        app.submit();
        assert!(app.notices().last().unwrap().text.contains("Only 2"));
    }

    #[test]
    fn emoji_tab_completion_replaces_the_shortcode() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.input = "nice :thumbsu".into();
        app.input_cursor = app.input.len();
        crate::complete::tab(&mut app, false);
        assert_eq!(app.input, "nice 👍");
    }

    #[test]
    fn draw_cost_with_a_long_chat() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let bot = app.active_bot().unwrap().id.clone();
        let v: Vec<Entry> = (0..400)
            .map(|i| Entry {
                kind: if i % 2 == 0 {
                    EntryKind::User
                } else {
                    EntryKind::Bot
                },
                text: format!(
                    "message {i}: {}\n```rust\nfn x() {{}}\n```\n- item\n- **bold** `code`",
                    "lorem ipsum dolor sit amet ".repeat(12)
                ),
            })
            .collect();
        app.logs.insert(bot, v);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 60)).unwrap();
        let n = 50;
        let t = Instant::now();
        for _ in 0..n {
            app.feed_cache = None;
            term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        }
        let cold = t.elapsed() / n;
        let t = Instant::now();
        for _ in 0..n {
            term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        }
        let warm = t.elapsed() / n;
        let t = Instant::now();
        for _ in 0..n {
            app.tick();
        }
        eprintln!(
            "PROBE 400 messages, 200x60: draw (layout rebuilt) {cold:?}, draw (cached) {warm:?}, tick {:?}",
            t.elapsed() / n
        );
    }
}

#[cfg(test)]
mod prompt_history_tests {
    use super::tests::*;
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(app: &mut App, c: KeyCode) {
        app.history_search_key(KeyEvent::new(c, KeyModifiers::NONE));
    }

    fn mk(rt: &tokio::runtime::Runtime, dir: &std::path::Path) -> App {
        let mut app = App::new(Store::open(dir, None).unwrap(), rt.handle().clone(), true).unwrap();
        app.init();
        app
    }

    #[test]
    fn prompt_history_survives_restart_and_up_recalls_it() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mut app = mk(&rt, &dir);
        mock_login(&mut app, "secret-token-value");
        type_line(&mut app, "first prompt");
        type_line(&mut app, "second prompt");
        type_line(&mut app, "second prompt");
        let bot = app.active_bot().map(|b| b.name.clone());
        assert!(bot.is_some());
        type_line(&mut app, "/quit");
        drop(app);

        // A brand-new App over the same data dir (= quitting and starting again).
        let mut app = mk(&rt, &dir);
        assert_eq!(
            &app.prompt_history[..3],
            ["/quit", "second prompt", "first prompt"],
            "newest first; consecutive duplicate stored once"
        );
        assert!(
            !app.prompt_history
                .iter()
                .any(|p| p.contains("secret-token-value")),
            "the masked token prompt is never remembered"
        );
        assert_eq!(app.prompt_meta[1].bot, bot, "bot remembered per prompt");
        assert!(crate::history::parse_utc(&app.prompt_meta[1].at).is_some());
        app.input.clear();
        app.input_cursor = 0;
        assert!(app.history_up());
        assert_eq!(app.input, "/quit");
        assert!(app.history_up());
        assert_eq!(app.input, "second prompt");
        assert!(app.history_up());
        assert_eq!(app.input, "first prompt");
        assert!(app.history_down());
        assert!(app.history_down());
        assert!(app.history_down());
        assert_eq!(app.input, "", "back to the (empty) draft");

        // Ctrl-R search runs over the persisted history.
        app.open_history_search();
        for c in "first".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.input, "first prompt");
        assert!(app.history_search.is_none());
    }

    #[test]
    fn up_sees_prompts_from_another_instance_on_the_same_db() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mut a = mk(&rt, &dir);
        let mut b = mk(&rt, &dir);
        type_line(&mut b, "/whoami");
        assert!(a.history_up());
        assert_eq!(a.input, "/whoami");
    }

    #[test]
    fn history_command_browses_filters_and_inserts_without_sending() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mut app = mk(&rt, &dir);
        mock_login(&mut app, "tok");
        type_line(&mut app, "deploy the staging box");
        type_line(&mut app, "write release notes");
        drop(app);
        let mut app = mk(&rt, &dir);
        let sent_before = app.current_log().len();
        app.input = "draft".into();
        app.input_cursor = 5;
        // Commands aren't remembered until submitted; run it like Enter would.
        type_line(&mut app, "/history depl");
        let s = app.history_search.as_ref().expect("browser open");
        assert!(s.browse);
        assert_eq!(s.query, "depl");
        assert_eq!(
            app.prompt_history[s.selected().unwrap()],
            "deploy the staging box"
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.input, "deploy the staging box", "inserted, not sent");
        assert!(app.history_search.is_none());
        let log: Vec<_> = app.current_log()[sent_before..]
            .iter()
            .map(|e| (e.kind, e.text.clone()))
            .collect();
        assert!(
            !log.iter()
                .any(|(_, t)| t.contains("deploy the staging box")),
            "nothing sent to the bot: {log:?}"
        );

        // Esc closes and keeps the draft.
        app.input = "keep me".into();
        app.input_cursor = 7;
        app.open_history_browser("");
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Esc);
        assert!(app.history_search.is_none());
        assert_eq!(app.input, "keep me");
    }

    #[test]
    fn history_browser_uses_vim_keys_when_enabled() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let mut app = mk(&rt, &d.path().join("gb"));
        for p in ["one", "two", "three"] {
            type_line(&mut app, p);
        }
        app.config.keys = crate::config::KeysScheme::Vim;
        app.open_history_browser("");
        assert!(!app.history_search.as_ref().unwrap().typing);
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Char('j'));
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.input, "one");
    }

    #[test]
    fn history_clear_asks_first_and_cancel_is_default() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let mut app = mk(&rt, &d.path().join("gb"));
        type_line(&mut app, "remember me");
        type_line(&mut app, "/history clear");
        assert!(app.picker.is_some());
        assert_eq!(
            app.prompt_history[0], "remember me",
            "/history isn't remembered"
        );
        app.picker_confirm(); // Cancel is selected by default
        assert!(app.prompt_history.contains(&"remember me".to_string()));
        type_line(&mut app, "/history clear");
        app.picker.as_mut().unwrap().move_by(1);
        app.picker_confirm();
        assert!(app.prompt_history.is_empty());
        assert!(app.store.prompt_history(10).unwrap().is_empty());
    }

    #[test]
    fn history_tab_completes_clear() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let mut app = mk(&rt, &d.path().join("gb"));
        app.input = "/history cle".into();
        app.input_cursor = app.input.len();
        crate::complete::tab(&mut app, false);
        assert_eq!(app.input, "/history clear");
    }

    #[test]
    fn history_cap_comes_from_prefs() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let mut app = mk(&rt, &d.path().join("gb"));
        assert_eq!(app.config.history_max, crate::config::DEFAULT_HISTORY_MAX);
        type_line(&mut app, "/prefs set history.max_entries 100");
        app.reload_config().unwrap();
        assert_eq!(app.config.history_max, 100);
        for i in 0..150 {
            type_line(&mut app, &format!("/whoami {i}"));
        }
        assert_eq!(app.store.prompt_history(10_000).unwrap().len(), 100);
        assert_eq!(app.prompt_history[0], "/whoami 149");
    }
}
