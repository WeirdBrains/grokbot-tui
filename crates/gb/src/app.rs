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
    /// `GB_GROKBOT_TOKEN` (never saved).
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
        source = "env GB_CURSOR_CLIENT_VERSION";
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
    /// `/shell` finished (output is a transient notice, never chat or gateway).
    Shell {
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
    /// Transient notices (command echoes, info, errors, delivery warnings) keyed by
    /// bot id ("" when no bot is active). Never stored, never mixed into the feed;
    /// rendered in their own section below it.
    pub notices: HashMap<String, Vec<Entry>>,
    pub input: String,
    pub mode: InputMode,
    pub focus: Focus,
    pub scroll: u16,
    pub sidebar_area: Rect,
    pub account_area: Rect,
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
        (Arc::new(GatewayApi(c)), s)
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
            ..Default::default()
        };
        let client_version_source = apply_client_identity(&mut opts, &config, true);
        let (tx, rx) = mpsc::unbounded_channel();
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
            notices: HashMap::new(),
            input: String::new(),
            mode: InputMode::Normal,
            focus: Focus::Input,
            scroll: 0,
            sidebar_area: Rect::default(),
            account_area: Rect::default(),
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
            picker: None,
            client_version_source,
            desktop_reread_tried: false,
            login_retry: None,
            db_refresh: (Instant::now(), None),
            tx,
            rx,
            should_quit: false,
        })
    }

    pub fn account(&self) -> &Account {
        self.store.account()
    }

    // ------------------------------------------------------------ tasks & events

    fn spawn<F: Future<Output = Event> + Send + 'static>(&mut self, fut: F) {
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
        let (Some(api), Some(bot)) = (self.api.clone(), self.active_bot().map(|b| b.id.clone()))
        else {
            return;
        };
        if self.poll.inflight || now < self.poll.next {
            return;
        }
        self.poll.inflight = true;
        let (epoch, limit) = (self.epoch, self.poll.limit);
        self.poll.limit = POLL_LIMIT;
        self.spawn(async move {
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
            let scroll = self.scroll;
            self.reload_bot_log(&bot);
            self.scroll = scroll;
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
            Event::Shell { cmd, result } => match result {
                Ok(o) => {
                    let status = match o.code {
                        Some(0) => "exit 0".to_string(),
                        Some(127) => "exit 127 (command not found?)".to_string(),
                        Some(c) => format!("exit {c}"),
                        None => "killed by signal".to_string(),
                    };
                    let mut t = format!("$ {cmd}");
                    if !o.text.is_empty() {
                        t.push('\n');
                        t.push_str(&o.text);
                    }
                    if o.truncated {
                        t.push_str(&format!(
                            "\n… output truncated (first {SHELL_MAX_LINES} lines / {} KB)",
                            SHELL_MAX_BYTES / 1024
                        ));
                    }
                    t.push_str(&format!("\n[{status}]"));
                    if o.code == Some(0) {
                        self.info(t);
                    } else {
                        self.error(t);
                    }
                }
                Err(e) => self.error(format!("$ {cmd}\n[shell error: {e}]")),
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
                        if announce {
                            let list = self
                                .all_bots
                                .iter()
                                .map(|b| {
                                    let mut flags = vec![];
                                    if b.is_group {
                                        flags.push("group");
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
                    // Network error / 5xx: ask the gateway whether it landed.
                    Err(_) => self.check_acceptance(bot, nonce, 1, Duration::ZERO),
                }
            }
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
        let epoch = self.epoch;
        self.spawn(async move {
            if !after.is_zero() {
                tokio::time::sleep(after).await;
            }
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
        self.scroll = 0;
        self.poll = PollState {
            next: Instant::now(),
            interval: POLL_IDLE,
            inflight: false,
            limit: POLL_LIMIT,
        };
        self.last_poll_error = None;
    }

    pub fn reload_config(&mut self) -> Result<()> {
        self.config = self.store.load_config()?;
        Ok(())
    }

    pub fn switch_account(&mut self, alias: &str) -> Result<()> {
        self.store.use_account(alias)?;
        self.reset_session();
        self.reload_config()?;
        self.info(format!("Switched to account '{}'.", self.account().alias));
        self.connect();
        Ok(())
    }

    pub fn init(&mut self) {
        self.info(format!(
            "Welcome to grokbot-tui{}. Type /help for commands.",
            if self.mock { " (mock mode)" } else { "" }
        ));
        self.connect();
    }

    /// Log in with `GB_GROKBOT_TOKEN` (startup account only) or the stored credential.
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
            // Restore the saved session as-is (keeping the desktop entry choice).
            self.spawn_login(
                l.method,
                LoginSource::Stored,
                Some(l.session),
                l.cursor_token,
                l.desktop_entry,
            );
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
                .and_then(|(r, _)| r.map_err(|e| e.to_string()));
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

    /// `/shell <cmd>`: run locally off the UI thread; output arrives as a notice.
    pub fn run_shell_command(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        self.info(format!("$ {cmd}  (running…)"));
        self.spawn(async move {
            let result = run_shell(&cmd).await;
            Event::Shell { cmd, result }
        });
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

    /// Switch the body to bot `i`'s chat: restore from the DB, then poll its transcript.
    pub fn select_bot(&mut self, i: usize) {
        if i >= self.bots.len() {
            return;
        }
        let changed = self.active != Some(i);
        self.active = Some(i);
        self.sidebar_cursor = i;
        self.scroll = 0;
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
        let entries: Vec<Entry> = self
            .store
            .messages(bot_id)
            .unwrap_or_default()
            .iter()
            .map(entry_from_msg)
            .collect();
        self.logs.insert(bot_id.to_string(), entries);
        if self.active_bot().is_some_and(|b| b.id == bot_id) {
            self.scroll = 0;
        }
    }

    /// Fetch the full (paged) chat history for the active bot now.
    pub fn request_history(&mut self) {
        let (Some(api), Some(bot)) = (self.api.clone(), self.active_bot().map(|b| b.id.clone()))
        else {
            self.poll.limit = HISTORY_LIMIT;
            self.poll_now();
            return;
        };
        if self.poll.inflight {
            // Queue a full fetch as soon as the in-flight poll finishes.
            self.poll.limit = HISTORY_LIMIT;
            return;
        }
        self.poll.inflight = true;
        let epoch = self.epoch;
        self.spawn(async move {
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

    fn key(&self) -> String {
        self.active_bot().map(|b| b.id.clone()).unwrap_or_default()
    }

    /// The active bot's chat feed (rebuilt from SQLite).
    pub fn feed(&self) -> &[Entry] {
        self.logs.get(&self.key()).map(Vec::as_slice).unwrap_or(&[])
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
        if key == self.key() {
            self.scroll = 0;
        }
    }

    pub fn push(&mut self, kind: EntryKind, text: String) {
        let k = self.key();
        self.push_to(&k, Entry { kind, text });
    }
    pub fn info(&mut self, t: String) {
        self.push(EntryKind::System, t);
    }
    pub fn error(&mut self, t: String) {
        self.push(EntryKind::Error, t);
    }

    // ------------------------------------------------------------ input

    pub fn cancel_input_mode(&mut self) {
        let was = self.mode;
        self.mode = InputMode::Normal;
        self.input.clear();
        self.login_url_draft = None;
        match was {
            InputMode::Normal => {}
            _ => self.info("Login cancelled.".into()),
        }
    }

    pub fn submit(&mut self) {
        let line = std::mem::take(&mut self.input);
        let trimmed = line.trim().to_string();
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
                    if cmd.name != "help" && !(cmd.name == "login" && args.trim().is_empty()) {
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
            self.send_chat(trimmed);
        }
    }

    /// Send a prompt in the background. The local copy is stored as pending until the transcript confirms it.
    pub fn send_chat(&mut self, text: String) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in. Use /login gateway or /login cursor.".into());
            return;
        };
        let Some(bot) = self.active_bot().map(|b| b.id.clone()) else {
            self.error("No active bot. Pick one with /bot or the sidebar.".into());
            return;
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        let mine = Message {
            role: Role::User,
            text: text.clone(),
            remote_id: Some(format!("pending:{nonce}")),
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
            let result = api.send(&bot, &text, &nonce).await;
            Event::Sent {
                epoch,
                bot,
                nonce,
                result,
            }
        });
    }

    pub fn click(&mut self, col: u16, row: u16) {
        if self
            .account_area
            .contains(ratatui::layout::Position::new(col, row))
        {
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
        self.scroll = self.scroll.saturating_add(n);
    }
    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
    }
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
        // Never touch the real desktop app from tests.
        app.desktop_lister = Arc::new(|| {
            Err(gw::Error::Desktop {
                code: "TEST".into(),
                message: "no lister in tests".into(),
            })
        });
        (app, d)
    }

    pub fn type_line(app: &mut App, s: &str) {
        app.input = s.into();
        app.submit();
    }

    /// Run the event loop until idle (and once more after `extra` for delayed replies).
    fn settle(app: &mut App) {
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
    fn shell_echo_shows_output_as_notice_not_chat() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "tok");
        let before = app.feed().len();
        type_line(&mut app, "/shell echo hi");
        settle(&mut app);
        let t = texts(&app).join("\n");
        assert!(t.contains("$ echo hi\nhi\n[exit 0]"), "{t}");
        assert_eq!(app.feed().len(), before, "not stored as chat");

        type_line(&mut app, "/shell echo oops >&2; exit 3");
        settle(&mut app);
        let t = texts(&app).join("\n");
        assert!(t.contains("oops\n[exit 3]"), "{t}");

        type_line(&mut app, "/shell definitely-not-a-command-xyz");
        settle(&mut app);
        assert!(
            texts(&app)
                .join("\n")
                .contains("[exit 127 (command not found?)]")
        );

        type_line(&mut app, "/shell");
        assert!(texts(&app).join("\n").contains("Usage: /shell"));
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
        assert_eq!(app.bots.len(), 3, "hidden bot filtered");
        assert_eq!(app.all_bots.len(), 4);

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
        assert_eq!(app.bots.len(), 4);
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
        // Clicking the @account header opens the account pick list.
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
                .contains("GB_ALLOW_LOCAL_GATEWAY")
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
        let (reader, calls) = fake_reader(|_, entry| match entry {
            None => Err(gw::Error::Desktop {
                code: "AMBIGUOUS_ENTRIES".into(),
                message:
                    "several saved sessions: personal, work. Choose one with /login desktop <name>."
                        .into(),
            }),
            Some(name) => Ok((
                gw::desktop::DesktopCredential::Session(Session::new(
                    "https://box.cursorvm.com",
                    "desk",
                )),
                Some(name.to_string()),
            )),
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
