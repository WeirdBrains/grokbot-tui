//! Application state and input handling.
use crate::api::{ApiClient, Bot, Message, Role, User};
use crate::commands::{self, Registry};
use crate::config::{Config, Store, TOKEN_ENV};
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
    pub should_quit: bool,
}

impl App {
    pub fn new(
        api: Box<dyn ApiClient>,
        store: Store,
        config: Config,
        rt: tokio::runtime::Handle,
        mock: bool,
    ) -> Self {
        Self {
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
            should_quit: false,
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
        self.api.set_token(None);
        self.user = None;
        self.bots.clear();
        self.active = None;
        self.logs.clear();
        self.scroll = 0;
        match self.store.delete_token() {
            Ok(true) => self.info("Logged out; stored credential deleted.".into()),
            Ok(false) => self.info("Logged out (no stored credential).".into()),
            Err(e) => self.error(format!("Could not delete credential: {e:#}")),
        }
        if std::env::var(TOKEN_ENV).is_ok() {
            self.info(format!(
                "Note: {TOKEN_ENV} is set and will log you back in next start."
            ));
        }
    }

    pub fn refresh_bots(&mut self) {
        match self.block(self.api.list_bots()) {
            Ok(b) => {
                self.bots = b;
                let want = self.config.default_bot.clone();
                let idx = want
                    .and_then(|w| self.find_bot(&w))
                    .or(if self.bots.is_empty() { None } else { Some(0) });
                if let Some(i) = idx {
                    self.select_bot(i);
                }
            }
            Err(e) => self.error(format!("Could not list bots: {e:#}")),
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
        if !self.logs.contains_key(&id) {
            self.logs.insert(id.clone(), vec![]);
            self.load_history(&id);
        }
    }

    pub fn load_history(&mut self, id: &str) {
        match self.block(self.api.history(id)) {
            Ok(h) => {
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
        match self.block(self.api.send(&bot.id, &text)) {
            Ok(m) => {
                let e = entry_from_msg(m);
                self.push(e.kind, e.text);
            }
            Err(e) => self.error(format!("Send failed: {e:#}")),
        }
    }

    /// Mouse click at terminal cell; selects a bot if it hit the sidebar list.
    pub fn click(&mut self, col: u16, row: u16) {
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
    use crate::api::MockClient;

    pub fn test_app(rt: &tokio::runtime::Runtime) -> (App, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::at(d.path().join("gb"));
        let app = App::new(
            Box::new(MockClient::new()),
            store,
            Config::default(),
            rt.handle().clone(),
            true,
        );
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
