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

/// Poll interval right after a send / while waiting for a reply.
pub const POLL_FAST: Duration = Duration::from_millis(1500);
/// Ceiling while waiting for a reply.
pub const POLL_AWAIT_MAX: Duration = Duration::from_secs(8);
/// Starting interval when idle.
pub const POLL_IDLE: Duration = Duration::from_secs(3);
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
    /// Type the alias in `App::pending_remove` to confirm deleting that account.
    ConfirmRemove,
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
pub fn apply_client_identity(
    opts: &mut ClientOptions,
    config: &crate::config::Config,
    detect: bool,
) {
    let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
    if let Some(v) = env(CLIENT_VERSION_ENV) {
        opts.client_version = v;
    } else if let Some(v) = config.client_version.clone() {
        opts.client_version = v;
    } else if detect
        && let Some(platform) = gw::desktop::Platform::current()
        && let Some(v) = gw::desktop::DesktopConfig::new(platform).detect_app_version()
    {
        opts.client_version = v;
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
}

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
    /// Body logs keyed by bot id ("" when no bot is active).
    pub logs: HashMap<String, Vec<Entry>>,
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
    /// Set after one automatic desktop re-read on 401 so a bad session can't loop.
    desktop_reread_tried: bool,
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
    pub should_quit: bool,
}

pub fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
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
        apply_client_identity(&mut opts, &config, /*detect=*/ true);
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
            desktop_reread_tried: false,
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
                    Err(e) => match source {
                        LoginSource::Interactive => self.error(format!("Login failed: {e}")),
                        LoginSource::Stored => {
                            self.error(format!("Saved credential failed: {e}. Use /login."))
                        }
                        LoginSource::Env => self.error(format!("{TOKEN_ENV} login failed: {e}")),
                    },
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
                full: _full,
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
                        let added = match self.store.merge_remote(&bot, &msgs) {
                            Ok(a) => a,
                            Err(e) => {
                                self.error(format!("Could not store transcript: {e:#}"));
                                vec![]
                            }
                        };
                        let got_reply = added.iter().any(|m| m.role == Role::Bot);
                        // Rebuild the visible log from SQLite so /clear, first select, and
                        // desktop-app messages all converge on the same merged transcript.
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
                            "Gateway rejected the credential (401). Use /login again.".to_string()
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
                    Ok(_) => {
                        self.push_to(&bot, Entry { kind: EntryKind::Error, text: "No receipt from the gateway: delivery unknown. Checking the transcript; don't resend until it does or doesn't appear.".into() });
                        self.awaiting.insert(bot.clone(), Instant::now());
                        self.schedule_fast(&bot, Duration::ZERO);
                    }
                    Err(e) if e.delivery() == Delivery::Rejected => {
                        let _ = self.store.delete_pending(&bot, &nonce);
                        self.push_to(
                            &bot,
                            Entry {
                                kind: EntryKind::Error,
                                text: format!("Not delivered (rejected): {e}"),
                            },
                        );
                    }
                    Err(e) => {
                        self.push_to(&bot, Entry {
                            kind: EntryKind::Error,
                            text: format!("Delivery unknown ({e}). Checking the transcript; don't resend until it does or doesn't appear."),
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
        self.connecting = false;
        self.all_bots.clear();
        self.bots.clear();
        self.active = None;
        self.sidebar_cursor = 0;
        self.logs.clear();
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

    pub fn next_account(&mut self) {
        let Ok(list) = self.store.accounts() else {
            return;
        };
        let cur = self.account().id;
        let i = list.iter().position(|a| a.id == cur).unwrap_or(0);
        if let Some(next) = list.get((i + 1) % list.len().max(1)).cloned()
            && next.id != cur
            && let Err(e) = self.switch_account(&next.alias)
        {
            self.error(format!("{e:#}"));
        }
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
            self.start_login(
                l.method,
                LoginSource::Stored,
                Some(l.session),
                l.cursor_token,
            );
        } else {
            self.info(format!(
                "Account '{}' is not logged in. Use /login gateway, /login cursor or /login desktop.",
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
        apply_client_identity(&mut self.opts, &self.config, true);
        self.connecting = true;
        let (epoch, mock, opts) = (self.epoch, self.mock, self.opts.clone());
        let reader = self.desktop_reader.clone();
        self.spawn(async move {
            let e2 = entry.clone();
            let policy = opts.policy;
            let read = tokio::task::spawn_blocking(move || reader(e2.as_deref(), &policy))
                .await
                .map_err(|e| format!("desktop session reader failed: {e}"))
                .and_then(|r| r.map_err(|e| e.to_string()));
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
            "Gateway rejected the credential (401); re-reading the desktop app session…".into(),
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
        let mut entries: Vec<Entry> = self
            .store
            .messages(bot_id)
            .unwrap_or_default()
            .iter()
            .map(entry_from_msg)
            .collect();
        if let Some(prev) = self.logs.get(bot_id) {
            let mut trailing = Vec::new();
            for e in prev.iter().rev() {
                if matches!(e.kind, EntryKind::System | EntryKind::Error) {
                    trailing.push(e.clone());
                } else {
                    break;
                }
            }
            trailing.reverse();
            entries.extend(trailing);
        }
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

    pub fn current_log(&self) -> &[Entry] {
        self.logs.get(&self.key()).map(Vec::as_slice).unwrap_or(&[])
    }

    fn push_to(&mut self, key: &str, e: Entry) {
        self.logs.entry(key.to_string()).or_default().push(e);
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

    pub fn clear_current(&mut self) {
        let k = self.key();
        self.logs.insert(k, vec![]);
        self.scroll = 0;
    }

    // ------------------------------------------------------------ input

    pub fn cancel_input_mode(&mut self) {
        let was = self.mode;
        self.mode = InputMode::Normal;
        self.input.clear();
        self.login_url_draft = None;
        match was {
            InputMode::ConfirmRemove => {
                self.pending_remove = None;
                self.info("Account removal cancelled.".into());
            }
            InputMode::Normal => {}
            _ => self.info("Login cancelled.".into()),
        }
    }

    pub fn submit(&mut self) {
        let line = std::mem::take(&mut self.input);
        let trimmed = line.trim().to_string();
        match self.mode {
            InputMode::ConfirmRemove => {
                self.mode = InputMode::Normal;
                commands::finish_remove(self, &trimmed);
                return;
            }
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
                    self.push(EntryKind::User, trimmed.clone());
                    if let Err(e) = run(self, args.trim()) {
                        self.error(format!("{e:#}"));
                    }
                }
                None => self.error(format!("Unknown command /{name}. Try /help.")),
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
        }
        self.push(EntryKind::User, text.clone());
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
            self.next_account();
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
        let app = App::new(store, rt.handle().clone(), true).unwrap();
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

        type_line(&mut app, "unknown");
        settle(&mut app);
        assert!(texts(&app).iter().any(|t| t.contains("Delivery unknown")));
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
        type_line(&mut app, "/clear saved");
        assert!(app.store.messages("scribe").unwrap().is_empty());
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

        type_line(&mut app, "/account remove work");
        assert_eq!(app.mode, InputMode::ConfirmRemove);
        type_line(&mut app, "nope");
        assert!(app.store.find_account("work").unwrap().is_some());
        type_line(&mut app, "/account remove work");
        type_line(&mut app, "work");
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
        app.click(2, 1);
        assert_eq!(app.account().alias, "b");
        app.click(2, 1);
        assert_eq!(app.account().alias, "default");
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
