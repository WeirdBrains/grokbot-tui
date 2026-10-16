//! Application state and input handling.
use crate::api::{ApiClient, Bot, HttpClient, Message, MockClient, Role, User};
use crate::commands::{self, Registry};
use crate::config::{Config, TOKEN_ENV};
use crate::storage::Account;
use crate::storage::Store;
use anyhow::Result;
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::future::Future;

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
    /// Masked token entry for `/login`.
    Token,
    /// Type the alias in `App::pending_remove` to confirm deleting that account.
    ConfirmRemove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Sidebar,
}

pub struct App {
    pub api: Box<dyn ApiClient>,
    pub store: Store,
    pub config: Config,
    pub rt: tokio::runtime::Handle,
    pub registry: Registry,
    pub mock: bool,
    pub user: Option<User>,
    pub bots: Vec<Bot>,
    pub active: Option<usize>,
    pub sidebar_cursor: usize,
    /// Body logs keyed by bot id ("" when no bot is active).
    pub logs: HashMap<String, Vec<Entry>>,
    pub input: String,
    pub mode: InputMode,
    pub focus: Focus,
    /// Lines scrolled up from the bottom of the body.
    pub scroll: u16,
    pub sidebar_area: Rect,
    /// Clickable account header in the sidebar (click cycles accounts).
    pub account_area: Rect,
    pub pending_remove: Option<String>,
    pub should_quit: bool,
}

/// Build the API client for an account's config.
pub fn build_client(config: &Config, mock: bool) -> Result<Box<dyn ApiClient>> {
    Ok(if mock {
        Box::new(MockClient::new())
    } else {
        Box::new(HttpClient::new(config.clone())?)
    })
}

impl App {
    pub fn new(store: Store, rt: tokio::runtime::Handle, mock: bool) -> Result<Self> {
        let config = store.load_config()?;
        let api = build_client(&config, mock)?;
        Ok(Self {
            api,
            store,
            config,
            rt,
            registry: commands::registry(),
            mock,
            user: None,
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
            should_quit: false,
        })
    }

    pub fn account(&self) -> &Account {
        self.store.account()
    }

    /// Drop all in-memory session state (user, bots, chats).
    fn reset_session(&mut self) {
        self.api.set_token(None);
        self.user = None;
        self.bots.clear();
        self.active = None;
        self.sidebar_cursor = 0;
        self.logs.clear();
        self.scroll = 0;
    }

    /// Reload config from the DB and rebuild the API client for the active account.
    pub fn reload_account_config(&mut self) -> Result<()> {
        self.config = self.store.load_config()?;
        self.api = build_client(&self.config, self.mock)?;
        Ok(())
    }

    /// Switch to another account: reload config, client, bots and chats.
    pub fn switch_account(&mut self, alias: &str) -> Result<()> {
        self.store.use_account(alias)?;
        self.reset_session();
        self.reload_account_config()?;
        self.info(format!("Switched to account '{}'.", self.account().alias));
        self.connect();
        Ok(())
    }

    /// Cycle to the next account (sidebar header click).
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

    pub fn block<F: Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    /// Startup: try the stored/env token.
    pub fn init(&mut self) {
        self.info(format!(
            "Welcome to gb{}. Type /help for commands.",
            if self.mock { " (mock mode)" } else { "" }
        ));
        self.connect();
    }

    /// Log in with the active account's stored (or env) token, if any.
    pub fn connect(&mut self) {
        if let Some(tok) = self.store.load_token() {
            if let Err(e) = self.authenticate(tok) {
                self.error(format!("Saved credential rejected: {e:#}. Use /login."));
            }
        } else {
            self.info("Not logged in. Use /login to paste an API token.".into());
        }
    }

    /// Validate a token via whoami, then load bots. Does not persist.
    pub fn authenticate(&mut self, token: String) -> Result<()> {
        self.api.set_token(Some(token));
        match self.block(self.api.whoami()) {
            Ok(u) => {
                self.info(format!("Logged in as {}.", u.name));
                self.user = Some(u);
                self.refresh_bots();
                Ok(())
            }
            Err(e) => {
                self.api.set_token(None);
                Err(e)
            }
        }
    }

    pub fn logout(&mut self) {
        if self.user.is_some()
            && let Err(e) = self.block(self.api.revoke())
        {
            self.error(format!("Server-side revoke failed: {e:#}"));
        }
        self.reset_session();
        match self.store.delete_credentials() {
            Ok(true) => self.info("Logged out; stored credential deleted.".into()),
            Ok(false) => self.info("Logged out (no stored credential).".into()),
            Err(e) => self.error(format!("Could not delete credential: {e:#}")),
        }
        if self.store.env_token_applies() {
            self.info(format!(
                "Note: {TOKEN_ENV} is set and will log you back in next start."
            ));
        }
    }

    pub fn refresh_bots(&mut self) {
        match self.block(self.api.list_bots()) {
            Ok(b) => {
                if let Err(e) = self.store.save_bots(&b) {
                    self.error(format!("Could not save bots: {e:#}"));
                }
                self.bots = b;
                let want = self
                    .config
                    .active_bot
                    .clone()
                    .or(self.config.default_bot.clone());
                let idx = want
                    .and_then(|w| self.find_bot(&w))
                    .or(if self.bots.is_empty() { None } else { Some(0) });
                if let Some(i) = idx {
                    self.select_bot(i);
                }
            }
            Err(e) => {
                self.error(format!("Could not list bots: {e:#}"));
                // Fall back to this account's cached bot list so saved chats stay reachable.
                if self.bots.is_empty()
                    && let Ok(cached) = self.store.bots()
                    && !cached.is_empty()
                {
                    self.bots = cached;
                    self.info("Showing cached bots.".into());
                }
            }
        }
    }

    pub fn find_bot(&self, q: &str) -> Option<usize> {
        let q = q.trim().to_lowercase();
        self.bots
            .iter()
            .position(|b| b.id.to_lowercase() == q || b.name.to_lowercase() == q)
    }

    /// Switch the body to bot `i`'s chat, loading history on first visit.
    pub fn select_bot(&mut self, i: usize) {
        if i >= self.bots.len() {
            return;
        }
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
        if !self.logs.contains_key(&id) {
            // Restore the ongoing chat from the local DB; fall back to server history.
            match self.store.messages(&id) {
                Ok(m) if !m.is_empty() => {
                    self.logs
                        .insert(id.clone(), m.into_iter().map(entry_from_msg).collect());
                }
                _ => {
                    self.logs.insert(id.clone(), vec![]);
                    self.load_history(&id);
                }
            }
        }
    }

    /// Fetch server history and replace the locally stored transcript.
    pub fn load_history(&mut self, id: &str) {
        match self.block(self.api.history(id)) {
            Ok(h) => {
                if let Err(e) = self.store.replace_messages(id, &h) {
                    self.error(format!("Could not store history: {e:#}"));
                }
                let entries = h.into_iter().map(entry_from_msg).collect();
                self.logs.insert(id.to_string(), entries);
            }
            Err(e) => self.error(format!("Could not load history: {e:#}")),
        }
    }

    pub fn active_bot(&self) -> Option<&Bot> {
        self.active.and_then(|i| self.bots.get(i))
    }

    fn key(&self) -> String {
        self.active_bot().map(|b| b.id.clone()).unwrap_or_default()
    }

    pub fn current_log(&self) -> &[Entry] {
        self.logs.get(&self.key()).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn push(&mut self, kind: EntryKind, text: String) {
        let k = self.key();
        self.logs.entry(k).or_default().push(Entry { kind, text });
        self.scroll = 0;
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

    /// Handle a submitted input line.
    pub fn submit(&mut self) {
        let line = std::mem::take(&mut self.input);
        if self.mode == InputMode::ConfirmRemove {
            self.mode = InputMode::Normal;
            commands::finish_remove(self, line.trim());
            return;
        }
        if self.mode == InputMode::Token {
            self.mode = InputMode::Normal;
            commands::finish_login(self, line.trim());
            return;
        }
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if let Some(rest) = line.strip_prefix('/') {
            let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            match self.registry.find(name) {
                Some(cmd) => {
                    let run = cmd.run;
                    self.push(EntryKind::User, line.to_string());
                    if let Err(e) = run(self, args.trim()) {
                        self.error(format!("{e:#}"));
                    }
                }
                None => self.error(format!("Unknown command /{name}. Try /help.")),
            }
        } else {
            self.send_chat(line.to_string());
        }
    }

    pub fn send_chat(&mut self, text: String) {
        let Some(bot) = self.active_bot().cloned() else {
            self.error("No active bot. Log in and pick one with /bot or the sidebar.".into());
            return;
        };
        self.push(EntryKind::User, text.clone());
        let mine = Message {
            role: Role::User,
            text: text.clone(),
        };
        match self.block(self.api.send(&bot.id, &text)) {
            Ok(m) => {
                for msg in [&mine, &m] {
                    if let Err(e) = self.store.append_message(&bot.id, msg) {
                        self.error(format!("Could not save message: {e:#}"));
                    }
                }
                let e = entry_from_msg(m);
                self.push(e.kind, e.text);
            }
            Err(e) => self.error(format!("Send failed: {e:#}")),
        }
    }

    /// Mouse click at terminal cell; selects a bot if it hit the sidebar list.
    pub fn click(&mut self, col: u16, row: u16) {
        let h = self.account_area;
        if h.contains(ratatui::layout::Position::new(col, row)) {
            self.next_account();
            return;
        }
        let a = self.sidebar_area;
        // Inside the border.
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

fn entry_from_msg(m: Message) -> Entry {
    Entry {
        kind: if m.role == Role::User {
            EntryKind::User
        } else {
            EntryKind::Bot
        },
        text: m.text,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn test_app(rt: &tokio::runtime::Runtime) -> (App, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path().join("gb"), None).unwrap();
        let app = App::new(store, rt.handle().clone(), true).unwrap();
        (app, d)
    }

    fn type_line(app: &mut App, s: &str) {
        app.input = s.into();
        app.submit();
    }

    #[test]
    fn login_select_chat_logout() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "hello");
        assert_eq!(app.current_log().last().unwrap().kind, EntryKind::Error);

        type_line(&mut app, "/login");
        assert_eq!(app.mode, InputMode::Token);
        type_line(&mut app, "secret");
        assert_eq!(app.user.as_ref().unwrap().name, "mock-user");
        assert_eq!(app.store.load_stored_token().as_deref(), Some("secret"));
        assert_eq!(app.bots.len(), 3);
        assert_eq!(app.active, Some(0));

        type_line(&mut app, "/bot coder");
        assert_eq!(app.active_bot().unwrap().id, "coder");
        type_line(&mut app, "hi there");
        assert_eq!(app.current_log().last().unwrap().text, "echo: hi there");

        app.select_bot(0);
        assert!(app.current_log().iter().all(|e| e.text != "echo: hi there"));

        type_line(&mut app, "/logout");
        assert!(app.user.is_none());
        assert!(app.bots.is_empty());
        assert!(app.store.load_stored_token().is_none());
    }

    #[test]
    fn bad_token_not_saved_and_unknown_command() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/login bad");
        assert!(app.user.is_none());
        assert!(app.store.load_stored_token().is_none());
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
    fn chat_restored_after_restart_and_prefs_memory() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        let mk = || {
            let s = Store::open(&dir, None).unwrap();
            App::new(s, rt.handle().clone(), true).unwrap()
        };
        let mut app = mk();
        type_line(&mut app, "/login t");
        type_line(&mut app, "/bot scribe");
        type_line(&mut app, "note this");
        type_line(&mut app, "/memory add topic rust tui");
        type_line(&mut app, "/prefs set ui.sidebar_width 30");
        drop(app);

        let mut app = mk(); // fresh mock server state, same DB
        app.init();
        assert_eq!(app.user.as_ref().unwrap().name, "mock-user");
        assert_eq!(app.active_bot().unwrap().id, "scribe");
        assert!(
            app.current_log()
                .iter()
                .any(|e| e.text == "echo: note this")
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
        type_line(&mut app, "/login t");
        type_line(&mut app, "personal msg");
        type_line(&mut app, "/memory add who personal");

        type_line(&mut app, "/account add work http://work.example");
        type_line(&mut app, "/account use work");
        assert_eq!(app.account().alias, "work");
        assert!(app.user.is_none(), "work account has no login");
        assert!(app.bots.is_empty());
        assert_eq!(app.config.api_base_url, "http://work.example");
        type_line(&mut app, "/login w");
        assert!(app.current_log().iter().all(|e| e.text != "personal msg"));
        type_line(&mut app, "/memory");
        assert!(!app.current_log().last().unwrap().text.contains("personal"));
        type_line(&mut app, "/logout");

        type_line(&mut app, "/account use default");
        assert_eq!(
            app.user.as_ref().unwrap().name,
            "mock-user",
            "default login survives work logout"
        );
        assert!(app.current_log().iter().any(|e| e.text == "personal msg"));

        // Removal needs confirmation by typing the alias.
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
    fn click_account_header_cycles() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/account add b");
        app.account_area = Rect::new(0, 1, 24, 1);
        app.click(2, 1);
        assert_eq!(app.account().alias, "b");
        app.click(2, 1);
        assert_eq!(app.account().alias, "default");
    }

    #[test]
    fn click_selects_bot() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        type_line(&mut app, "/login t");
        app.sidebar_area = Rect::new(0, 1, 20, 10);
        app.click(3, 4); // row 4 -> index 2
        assert_eq!(app.active, Some(2));
        app.click(3, 1); // border: ignored
        assert_eq!(app.active, Some(2));
    }
}
