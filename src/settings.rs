//! Settings page (1.33.0): every Grok Bot setting, grouped by the app's tabs, for the
//! selected account and bot, plus grokbot-tui's own local prefs.
//!
//! What the gateway API can do decides how a row behaves:
//! - **Bot** rows (`listAgents` record) are read live; name, title, description, avatar
//!   shape/color (`updateAgent`), "notify on updates" (`setAgentNotifyOnUpdates`) and
//!   "hidden from sidebar" (`setAgentHiddenFromSidebar`) are editable here.
//! - **Account/app** rows (theme, timezone, auto-review, computers, usage, plan, updates, …)
//!   have no gateway method, so they show "Open in Grok Bot →" and open the app's
//!   `grokbot://app/v1/settings?id=<row>` deep link through the selected link opener
//!   (`links.open_with`, see `links.rs`). Destructive ones (reset computer, cancel trial)
//!   ask first and still run in the app, never from the TUI.
//! - **Local** rows are grokbot-tui prefs, saved in its own data dir.
use crate::api::{AgentChange, AgentSettings};
use crate::app::{App, Event};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Tabs, in the Grok Bot app's order, then the bot and local tabs.
pub const TABS: &[&str] = &[
    "General",
    "Computer",
    "Vault",
    "Usage & Billing",
    "Updates",
    "Bot",
    "Local (grokbot-tui)",
];

/// The Grok Bot app's settings deep link for row `id` (the app's own settings ids).
pub fn app_link(id: &str) -> String {
    format!("grokbot://app/v1/settings?id={id}")
}

/// grokbot-tui's releases page.
pub const RELEASES_URL: &str = "https://github.com/WeirdBrains/grokbot-tui/releases";

/// What activating a row does.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Flip a boolean.
    Toggle,
    /// Cycle through choices.
    Select(Vec<String>),
    /// Edit text inline.
    Text,
    /// Run something in the TUI (e.g. open a picker).
    Action,
    /// Open an external link through the selected opener.
    Link(String),
    /// Shown only.
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: &'static str,
    pub label: String,
    pub value: String,
    pub kind: Kind,
    /// Ask before doing it.
    pub destructive: bool,
}

fn row(key: &'static str, label: &str, value: impl Into<String>, kind: Kind) -> Row {
    Row {
        key,
        label: label.into(),
        value: value.into(),
        kind,
        destructive: false,
    }
}

fn app_row(key: &'static str, label: &str) -> Row {
    row(key, label, "Open in Grok Bot →", Kind::Link(app_link(key)))
}

fn danger(mut r: Row) -> Row {
    r.destructive = true;
    r
}

fn on_off(b: bool) -> String {
    if b { "on" } else { "off" }.into()
}

/// The bot tab's load state.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum BotState {
    #[default]
    None,
    Loading,
    Ready(AgentSettings),
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub tab: usize,
    pub row: usize,
    pub bot_id: Option<String>,
    pub bot_name: String,
    pub bot: BotState,
    /// Inline text edit: (row key, buffer).
    pub edit: Option<(&'static str, String)>,
    /// A destructive row waiting for y/n.
    pub confirm: Option<&'static str>,
    /// A write in flight (row key).
    pub saving: Option<&'static str>,
    pub tab_areas: Vec<Rect>,
    pub row_areas: Vec<(Rect, usize)>,
    pub area: Rect,
}

/// `/settings` tab names → index (case-insensitive prefix: "usage", "bot", "local", …).
pub fn tab_index(name: &str) -> Option<usize> {
    let n = name.trim().to_ascii_lowercase();
    if n.is_empty() {
        return None;
    }
    let alias = match n.as_str() {
        "agent" => "bot",
        "billing" | "plan" | "credits" => "usage",
        "tui" | "prefs" => "local",
        x => x,
    };
    TABS.iter()
        .position(|t| t.to_ascii_lowercase().starts_with(alias))
}

/// The rows of tab `tab` with current values.
pub fn rows(app: &App, s: &Settings, tab: usize) -> Vec<Row> {
    let c = &app.config;
    match tab {
        0 => {
            let login = app
                .user_label
                .clone()
                .unwrap_or_else(|| "not logged in".into());
            vec![
                row(
                    "account",
                    "Account",
                    format!("@{} · {login}  (switch ▸)", app.account().alias),
                    Kind::Action,
                ),
                danger(row(
                    "sign-out",
                    "Sign out",
                    "Log out of this account",
                    Kind::Action,
                )),
                app_row("theme", "Theme (app)"),
                app_row("accent", "Accent color"),
                app_row("language", "Language"),
                app_row("spell-check", "Spell check"),
                app_row("microphone", "Microphone"),
                app_row("hardware-acceleration", "Hardware acceleration"),
                app_row("network-debugger", "Network debugger"),
                app_row("notification-sound-enabled", "Notification sound"),
                app_row("notification-sound", "Notification sound choice"),
                app_row("timezone", "Time zone"),
                app_row("auto-review", "Auto-review"),
                app_row("auto-review-rules", "Auto-review rules"),
                app_row("security-keys", "Security keys"),
            ]
        }
        1 => vec![
            app_row("local-execution", "Local execution"),
            app_row("computers", "Computers"),
            app_row("update-computer", "Update computer"),
            danger(app_row("reset-computer", "Reset computer")),
        ],
        2 => vec![app_row("vault", "Vault (logins, cards, contacts)")],
        3 => vec![
            app_row("usage", "Usage"),
            app_row("plan", "Plan"),
            danger(app_row("cancel-trial", "Cancel trial")),
            app_row("on-demand", "On-demand usage"),
            app_row("billing", "Billing"),
        ],
        4 => vec![
            app_row("update-status", "Update status (app)"),
            app_row("update-channel", "Update channel"),
            app_row("automatic-updates", "Automatic updates"),
            row(
                "tui-version",
                "grokbot-tui version (local)",
                format!("{} · releases →", env!("CARGO_PKG_VERSION")),
                Kind::Link(RELEASES_URL.into()),
            ),
        ],
        5 => bot_rows(s),
        _ => vec![
            row(
                "links.open_with",
                "Links open with",
                c.open_with.as_str(),
                Kind::Select(vec!["builtin".into(), "system".into()]),
            ),
            row(
                "links.ssh_behavior",
                "Links in SSH",
                if c.ssh_builtin { "builtin" } else { "pref" },
                Kind::ReadOnly,
            ),
            row(
                "ui.theme",
                "Theme (▸ picker)",
                c.theme.clone(),
                Kind::Action,
            ),
            row(
                "ui.theme_mode",
                "Appearance",
                c.theme_mode.as_str(),
                Kind::Select(
                    crate::theme::Mode::ALL
                        .iter()
                        .map(|m| m.as_str().to_string())
                        .collect(),
                ),
            ),
            row(
                "ui.header",
                "Compact header",
                on_off(c.header_compact),
                Kind::Toggle,
            ),
            row(
                "ui.sidebar_collapsed",
                "Collapsed sidebar",
                on_off(c.sidebar_collapsed),
                Kind::Toggle,
            ),
            row(
                "ui.attachments_panel",
                "Files panel",
                on_off(c.attachments_panel),
                Kind::Toggle,
            ),
            row(
                "ui.mouse",
                "Mouse capture",
                on_off(app.mouse_capture),
                Kind::Toggle,
            ),
            row(
                "ui.spinner",
                "Animated spinner",
                on_off(c.spinner),
                Kind::Toggle,
            ),
            row("ui.keys", "Key scheme", c.keys.as_str(), Kind::ReadOnly),
            row(
                "agents.refresh_interval_secs",
                "Bot list refresh (s)",
                c.agents_refresh_secs.to_string(),
                Kind::ReadOnly,
            ),
        ],
    }
}

fn bot_rows(s: &Settings) -> Vec<Row> {
    let a = match &s.bot {
        BotState::None => {
            return vec![row("bot", "Bot", "No bot selected", Kind::ReadOnly)];
        }
        BotState::Loading => {
            return vec![row("bot", &s.bot_name, "Loading…", Kind::ReadOnly)];
        }
        BotState::Failed(e) => {
            return vec![row("bot", &s.bot_name, e.clone(), Kind::ReadOnly)];
        }
        BotState::Ready(a) => a,
    };
    let opt = |b: Option<bool>| b.map(on_off).unwrap_or_else(|| "—".into());
    let list = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    vec![
        row("bot.name", "Name", a.name.clone(), Kind::Text),
        row("bot.title", "Title", a.title.clone(), Kind::Text),
        row(
            "bot.description",
            "Description",
            a.description.clone(),
            Kind::Text,
        ),
        row(
            "bot.avatar_shape",
            "Avatar shape",
            a.avatar_shape.clone(),
            Kind::Select(list(grokbot_api::AVATAR_SHAPES)),
        ),
        row(
            "bot.avatar_color",
            "Avatar color",
            a.avatar_color.clone(),
            Kind::Select(list(grokbot_api::AVATAR_COLORS)),
        ),
        row(
            "bot.notify_on_updates",
            "Notify on updates",
            opt(a.notify_on_updates),
            Kind::Toggle,
        ),
        row(
            "bot.hidden",
            "Hidden from sidebar",
            opt(a.hidden),
            Kind::Toggle,
        ),
        row(
            "bot.notifications",
            "Notifications (read-only)",
            opt(a.notifications),
            Kind::ReadOnly,
        ),
        row(
            "bot.voice",
            "Voice (read-only)",
            a.voice.clone().unwrap_or_else(|| "—".into()),
            Kind::ReadOnly,
        ),
        row(
            "bot.harness",
            "Harness (read-only)",
            a.harness.clone().unwrap_or_else(|| "—".into()),
            Kind::ReadOnly,
        ),
        row(
            "bot.avatar_image",
            "Avatar image",
            "Change it in the Grok Bot app (no API)",
            Kind::ReadOnly,
        ),
    ]
}

/// The next choice after `cur` (wraps; unknown → first).
pub fn next_choice(choices: &[String], cur: &str) -> String {
    let i = choices.iter().position(|c| c == cur).map_or(0, |i| i + 1);
    choices[i % choices.len()].clone()
}

impl App {
    /// Open the Settings page (`/settings [tab]`, header button, Alt-, / vim `settings.open`).
    pub fn open_settings(&mut self, tab: Option<usize>) {
        let mut s = Settings {
            tab: tab.unwrap_or(0).min(TABS.len() - 1),
            ..Settings::default()
        };
        if let Some(b) = self.active_bot() {
            s.bot_id = Some(b.id.clone());
            s.bot_name = b.name.clone();
            s.bot = BotState::Loading;
        }
        self.settings = Some(s);
        self.tooltip.dismiss();
        self.settings_reload();
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
        self.needs_clear = true;
    }

    /// Re-read the bot's settings (`listAgents`, read-only).
    pub fn settings_reload(&mut self) {
        let Some(id) = self.settings.as_ref().and_then(|s| s.bot_id.clone()) else {
            return;
        };
        let Some(api) = self.api.clone() else {
            if let Some(s) = self.settings.as_mut() {
                s.bot = BotState::Failed("Not logged in.".into());
            }
            return;
        };
        if let Some(s) = self.settings.as_mut()
            && !matches!(s.bot, BotState::Ready(_))
        {
            s.bot = BotState::Loading;
        }
        self.spawn(async move {
            let result = api.agent_settings(&id).await;
            Event::SettingsLoaded { bot: id, result }
        });
    }

    /// `Event::SettingsLoaded`.
    pub fn settings_loaded(&mut self, bot: String, result: grokbot_api::Result<AgentSettings>) {
        let Some(s) = self.settings.as_mut() else {
            return;
        };
        if s.bot_id.as_deref() != Some(bot.as_str()) {
            return;
        }
        s.bot = match result {
            Ok(a) => BotState::Ready(a),
            Err(e) => BotState::Failed(format!("Couldn't read settings: {e}")),
        };
    }

    /// `Event::SettingSaved`.
    pub fn setting_saved(&mut self, bot: String, label: String, result: grokbot_api::Result<()>) {
        if let Some(s) = self.settings.as_mut() {
            s.saving = None;
        }
        match result {
            Ok(()) => {
                self.info(format!("Saved: {label}"));
                if self.settings.as_ref().and_then(|s| s.bot_id.as_deref()) == Some(bot.as_str()) {
                    self.settings_reload();
                }
                self.refresh_bots(crate::app::BotsFetch::Manual);
            }
            Err(e) => self.error(format!("Couldn't save {label}: {e}")),
        }
    }

    fn settings_write(&mut self, key: &'static str, label: String, change: AgentChange) {
        let Some(api) = self.api.clone() else {
            self.error("Not logged in.".into());
            return;
        };
        let Some(s) = self.settings.as_mut() else {
            return;
        };
        let Some(id) = s.bot_id.clone() else {
            return;
        };
        s.saving = Some(key);
        // Optimistic: show the new value right away; a reload corrects it.
        if let BotState::Ready(a) = &mut s.bot {
            match &change {
                AgentChange::Profile(p) => {
                    a.name = p.name.clone();
                    a.description = p.description.clone();
                    a.title = p.title.clone().unwrap_or_default();
                    a.avatar_shape = p.avatar_shape.clone().unwrap_or_default();
                    a.avatar_color = p.avatar_color.clone().unwrap_or_default();
                }
                AgentChange::NotifyOnUpdates(on) => a.notify_on_updates = Some(*on),
                AgentChange::Hidden(h) => a.hidden = Some(*h),
            }
        }
        self.spawn(async move {
            let result = api.update_agent_setting(&id, change).await;
            Event::SettingSaved {
                bot: id,
                label,
                result,
            }
        });
    }

    fn settings_profile(&self) -> Option<AgentSettings> {
        match &self.settings.as_ref()?.bot {
            BotState::Ready(a) => Some(a.clone()),
            _ => None,
        }
    }

    /// Activate the selected row (Enter / Space / click).
    pub fn settings_activate(&mut self) {
        let Some(s) = self.settings.as_ref() else {
            return;
        };
        let list = rows(self, s, s.tab);
        let Some(r) = list.get(s.row).cloned() else {
            return;
        };
        if r.destructive && s.confirm != Some(r.key) {
            if let Some(s) = self.settings.as_mut() {
                s.confirm = Some(r.key);
            }
            return;
        }
        if let Some(s) = self.settings.as_mut() {
            s.confirm = None;
        }
        self.settings_run(&r);
    }

    fn settings_run(&mut self, r: &Row) {
        match (&r.kind, r.key) {
            (Kind::Link(url), _) => {
                let url = url.clone();
                self.open_link(&url, false);
            }
            (_, "account") => {
                self.close_settings();
                self.open_account_picker();
            }
            (_, "sign-out") => {
                self.close_settings();
                self.logout();
            }
            (_, "ui.theme") => {
                self.close_settings();
                self.open_theme_picker();
            }
            (Kind::Text, k) => {
                let v = r.value.clone();
                if let Some(s) = self.settings.as_mut() {
                    s.edit = Some((k, v));
                }
            }
            (Kind::Select(ch), k) => {
                let v = next_choice(ch, &r.value);
                self.settings_set(k, &v);
            }
            (Kind::Toggle, k) => {
                let v = on_off(r.value != "on");
                self.settings_set(k, &v);
            }
            _ => {}
        }
    }

    /// Apply a value to a writable row.
    pub fn settings_set(&mut self, key: &'static str, v: &str) {
        let on = v == "on";
        match key {
            "links.open_with" => {
                if let Some(w) = crate::config::OpenWith::parse(v) {
                    self.set_open_with(w);
                }
            }
            "ui.theme_mode" => {
                if let Some(m) = crate::theme::Mode::ALL.iter().find(|m| m.as_str() == v) {
                    self.set_theme_mode(*m);
                }
            }
            "ui.header" => self.set_header_compact(on),
            "ui.sidebar_collapsed" => self.set_sidebar_collapsed(on),
            "ui.attachments_panel" => self.set_attachments_panel(on),
            "ui.mouse" => {
                if self.mouse_capture != on {
                    self.toggle_mouse_capture();
                }
            }
            "ui.spinner" => {
                if let Err(e) = self.store.set_pref(
                    crate::config::PREF_UI_SPINNER,
                    if on { "on" } else { "off" },
                ) {
                    self.error(format!("{e:#}"));
                } else {
                    self.config.spinner = on;
                }
            }
            "bot.notify_on_updates" => self.settings_write(
                key,
                "notify on updates".into(),
                AgentChange::NotifyOnUpdates(on),
            ),
            "bot.hidden" => {
                self.settings_write(key, "hidden from sidebar".into(), AgentChange::Hidden(on))
            }
            "bot.name" | "bot.title" | "bot.description" | "bot.avatar_shape"
            | "bot.avatar_color" => {
                let Some(a) = self.settings_profile() else {
                    return;
                };
                let mut a = a;
                match key {
                    "bot.name" => a.name = v.trim().to_string(),
                    "bot.title" => a.title = v.to_string(),
                    "bot.description" => a.description = v.to_string(),
                    "bot.avatar_shape" => a.avatar_shape = v.to_string(),
                    _ => a.avatar_color = v.to_string(),
                }
                if a.name.is_empty() {
                    self.error("Name cannot be blank.".into());
                    return;
                }
                let label = key.trim_start_matches("bot.").replace('_', " ");
                self.settings_write(key, label, AgentChange::Profile(a.profile()));
            }
            _ => {}
        }
    }

    pub fn settings_key(&mut self, k: KeyEvent) {
        let Some(s) = self.settings.as_mut() else {
            return;
        };
        if let Some((key, buf)) = s.edit.as_mut() {
            match k.code {
                KeyCode::Esc => s.edit = None,
                KeyCode::Enter => {
                    let (key, v) = (*key, buf.clone());
                    s.edit = None;
                    self.settings_set(key, &v);
                }
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => buf.push(c),
                _ => {}
            }
            return;
        }
        if s.confirm.is_some() {
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.settings_activate()
                }
                _ => s.confirm = None,
            }
            return;
        }
        let n = {
            let s = self.settings.as_ref().unwrap();
            rows(self, s, s.tab).len()
        };
        let s = self.settings.as_mut().unwrap();
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_settings(),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                s.tab = (s.tab + 1) % TABS.len();
                s.row = 0;
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                s.tab = (s.tab + TABS.len() - 1) % TABS.len();
                s.row = 0;
            }
            KeyCode::Char(c @ '1'..='7') => {
                s.tab = (c as usize - '1' as usize).min(TABS.len() - 1);
                s.row = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => s.row = (s.row + 1).min(n.saturating_sub(1)),
            KeyCode::Up | KeyCode::Char('k') => s.row = s.row.saturating_sub(1),
            KeyCode::Home | KeyCode::Char('g') => s.row = 0,
            KeyCode::End | KeyCode::Char('G') => s.row = n.saturating_sub(1),
            KeyCode::Char('r') => self.settings_reload(),
            KeyCode::Enter | KeyCode::Char(' ') => self.settings_activate(),
            _ => {}
        }
    }

    /// Left click while Settings is open; outside the page closes it.
    pub fn settings_click(&mut self, col: u16, row: u16) {
        let at = Position::new(col, row);
        let Some(s) = self.settings.as_mut() else {
            return;
        };
        if !s.area.contains(at) {
            self.close_settings();
            return;
        }
        if let Some(i) = s.tab_areas.iter().position(|r| r.contains(at)) {
            s.tab = i;
            s.row = 0;
            s.edit = None;
            s.confirm = None;
            return;
        }
        if let Some(&(_, i)) = s.row_areas.iter().find(|(r, _)| r.contains(at)) {
            let again = s.row == i;
            s.row = i;
            if again || s.confirm.is_none() {
                self.settings_activate();
            }
        }
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let Some(s) = app.settings.as_ref() else {
        return;
    };
    let list = rows(app, s, s.tab);
    let s = app.settings.as_mut().unwrap();
    let full = f.area();
    let w = full.width.saturating_sub(4).min(110);
    let h = full.height.saturating_sub(2);
    let area = Rect::new(full.x + (full.width - w) / 2, full.y + 1, w, h);
    s.area = area;
    s.tab_areas.clear();
    s.row_areas.clear();
    f.render_widget(Clear, area);
    let title = match &s.bot_id {
        Some(_) => format!(" ⚙ Settings · @{} · {} ", app_alias(&app.store), s.bot_name),
        None => format!(" ⚙ Settings · @{} ", app_alias(&app.store)),
    };
    let block = Block::new()
        .borders(Borders::ALL)
        .title(title)
        .title_bottom(" ←/→ tab · ↑/↓ row · Enter change/open · r reload · Esc back ")
        .style(theme::settings_frame())
        .border_style(theme::settings_frame());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    // Tabs row.
    let mut x = inner.x;
    let mut spans = vec![];
    for (i, t) in TABS.iter().enumerate() {
        let text = format!(" {} {t} ", i + 1);
        let tw = text.width() as u16;
        let vis = inner.right().saturating_sub(x).min(tw);
        if vis > 0 {
            s.tab_areas.push(Rect::new(x, inner.y, vis, 1));
        }
        x = x.saturating_add(tw + 1);
        let st = if i == s.tab {
            theme::settings_tab()
        } else {
            theme::settings_frame()
        };
        spans.push(Span::styled(text, st));
        spans.push(Span::raw(" "));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let note = match s.tab {
        5 => "From this bot's listAgents record. Editable rows save through the gateway.",
        6 => "Local: grokbot-tui's own prefs on this computer (not synced to Grok Bot).",
        4 => {
            "App updates are managed by the Grok Bot app; grokbot-tui updates from GitHub releases."
        }
        _ => "The gateway API has no method for these: they open in the Grok Bot app.",
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(note, theme::key_hint()))),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
    let label_w = 28u16.min(inner.width / 2);
    let top = inner.y + 3;
    let room = inner.bottom().saturating_sub(top) as usize;
    let skip = s.row.saturating_sub(room.saturating_sub(1));
    for (i, r) in list.iter().enumerate().skip(skip).take(room) {
        let y = top + (i - skip) as u16;
        let sel = i == s.row;
        let mut value = r.value.clone();
        if let Some((k, buf)) = &s.edit
            && *k == r.key
        {
            value = format!("{buf}▏  (Enter save · Esc cancel)");
        } else if s.confirm == Some(r.key) {
            value = match &r.kind {
                Kind::Link(_) => "Opens in Grok Bot. Continue? y/n".into(),
                _ => "Are you sure? y/n".into(),
            };
        } else if s.saving == Some(r.key) {
            value.push_str("  (saving…)");
        }
        let hint = match &r.kind {
            Kind::Toggle => "  [toggle]",
            Kind::Select(_) => "  [cycle]",
            Kind::Text => "  [edit]",
            Kind::ReadOnly => "  (read-only)",
            _ => "",
        };
        let st = if sel {
            theme::settings_tab()
        } else {
            theme::settings_frame()
        };
        let label = format!(
            "{} {:<w$}",
            if sel { "▸" } else { " " },
            r.label,
            w = label_w.saturating_sub(2) as usize
        );
        let line = Line::from(vec![
            Span::styled(label, st),
            Span::styled(format!(" {value}"), theme::settings_frame()),
            Span::styled(if r.destructive { "  ⚠" } else { hint }, theme::key_hint()),
        ]);
        let ra = Rect::new(inner.x, y, inner.width, 1);
        s.row_areas.push((ra, i));
        f.render_widget(Paragraph::new(line), ra);
    }
}

fn app_alias(store: &crate::storage::Store) -> String {
    store.account().alias.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::tests::{mock_login, settle, test_app};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(app: &mut App, c: KeyCode) {
        app.settings_key(KeyEvent::new(c, KeyModifiers::NONE));
    }
    fn screen(app: &mut App) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 36)).unwrap();
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let b = term.backend().buffer();
        (0..b.area.height)
            .map(|y| {
                (0..b.area.width)
                    .map(|x| b[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn ready() -> (tokio::runtime::Runtime, App, tempfile::TempDir) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        assert!(app.active_bot().is_some(), "a mock bot is selected");
        (rt, app, d)
    }
    fn sel(app: &App) -> Row {
        let s = app.settings.as_ref().unwrap();
        rows(app, s, s.tab)[s.row].clone()
    }
    fn goto(app: &mut App, tab: usize, k: &str) {
        app.settings.as_mut().unwrap().tab = tab;
        let i = rows(app, app.settings.as_ref().unwrap(), tab)
            .iter()
            .position(|r| r.key == k)
            .expect(k);
        app.settings.as_mut().unwrap().row = i;
    }

    #[test]
    fn header_button_opens_and_esc_goes_back() {
        let (_rt, mut app, _d) = ready();
        let s = screen(&mut app);
        assert!(s.contains("[⚙ Settings]") || s.contains("⚙"), "{s}");
        let b = app.settings_button_area;
        app.click(b.x, b.y);
        assert!(app.settings.is_some());
        settle(&mut app);
        let s = screen(&mut app);
        assert!(
            s.contains("General") && s.contains("Usage & Billing"),
            "{s}"
        );
        assert!(s.contains("Open in Grok Bot →"), "{s}");
        key(&mut app, KeyCode::Esc);
        assert!(app.settings.is_none());
    }

    #[test]
    fn command_and_tab_navigation() {
        let (_rt, mut app, _d) = ready();
        crate::app::tests::type_line(&mut app, "/settings usage");
        assert_eq!(app.settings.as_ref().unwrap().tab, 3);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.settings.as_ref().unwrap().tab, 4);
        key(&mut app, KeyCode::Char('1'));
        assert_eq!(app.settings.as_ref().unwrap().tab, 0);
        key(&mut app, KeyCode::Down);
        assert_eq!(sel(&app).key, "sign-out");
        key(&mut app, KeyCode::Left);
        assert_eq!(app.settings.as_ref().unwrap().tab, TABS.len() - 1);
    }

    #[test]
    fn bot_tab_reads_and_writes_through_the_gateway() {
        let (_rt, mut app, _d) = ready();
        app.open_settings(tab_index("bot"));
        settle(&mut app);
        let s = screen(&mut app);
        assert!(s.contains("Mock title"), "{s}");
        goto(&mut app, 5, "bot.notify_on_updates");
        assert_eq!(sel(&app).value, "on");
        key(&mut app, KeyCode::Enter);
        settle(&mut app);
        assert_eq!(sel(&app).value, "off");
        goto(&mut app, 5, "bot.title");
        key(&mut app, KeyCode::Enter);
        for _ in 0.."Mock title".len() {
            key(&mut app, KeyCode::Backspace);
        }
        for c in "Squirrel".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Enter);
        settle(&mut app);
        assert_eq!(sel(&app).value, "Squirrel");
        goto(&mut app, 5, "bot.harness");
        key(&mut app, KeyCode::Enter);
        assert_eq!(sel(&app).value, "mock");
    }

    #[test]
    fn destructive_rows_confirm_and_links_use_the_selected_opener() {
        let (_rt, mut app, _d) = ready();
        // SSH so the system opener only arms (nothing launches in tests).
        app.ssh = true;
        app.open_settings(Some(1));
        goto(&mut app, 1, "reset-computer");
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.settings.as_ref().unwrap().confirm,
            Some("reset-computer")
        );
        assert!(screen(&mut app).contains("Continue? y/n"));
        key(&mut app, KeyCode::Char('n'));
        assert!(app.settings.as_ref().unwrap().confirm.is_none());
        assert!(app.ssh_confirm.is_none(), "cancel opens nothing");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('y'));
        assert_eq!(
            app.ssh_confirm.as_ref().unwrap().0,
            app_link("reset-computer"),
            "grokbot:// links go through open_link to the system opener"
        );
        app.ssh_confirm = None;
        goto(&mut app, 4, "tui-version");
        key(&mut app, KeyCode::Enter);
        assert!(app.browser.is_some(), "https opens in the built-in browser");
    }

    #[test]
    fn local_prefs_save() {
        let (_rt, mut app, _d) = ready();
        app.open_settings(tab_index("local"));
        goto(&mut app, 6, "links.open_with");
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.config.open_with, crate::config::OpenWith::System);
        goto(&mut app, 6, "ui.spinner");
        key(&mut app, KeyCode::Enter);
        assert!(!app.config.spinner);
        assert!(screen(&mut app).contains("Local: grokbot-tui"));
    }

    #[test]
    fn tab_names_resolve() {
        assert_eq!(tab_index("usage"), Some(3));
        assert_eq!(tab_index("credits"), Some(3));
        assert_eq!(tab_index("agent"), Some(5));
        assert_eq!(tab_index("local"), Some(6));
        assert_eq!(tab_index("nope"), None);
    }

    #[test]
    fn app_rows_link_to_app_settings() {
        assert_eq!(app_link("usage"), "grokbot://app/v1/settings?id=usage");
        assert_eq!(next_choice(&["a".into(), "b".into()], "b"), "a");
    }
}
