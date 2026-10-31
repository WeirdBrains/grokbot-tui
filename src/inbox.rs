//! Grok Bot notifications (1.34.0): a header bell with an unread count, a notifications
//! page, and top-right toasts.
//!
//! The gateway API has no notifications/inbox method, stream, or "clear" call, so this is
//! built from what it really returns (surveyed live, read-only):
//! - transcript entries with `kind: "notice"` (Grok Bot's own system notices, e.g. "A
//!   background task was stopped…"),
//! - `listAgents` fields `unreadCount`/`hasUnread`, `awaitingUserResponse` and
//!   `lastMessagePreview` for bots you aren't looking at,
//! - pending cards (questions, approvals, secrets, forms, drafts).
//!
//! There is no usage or credits API either, so **low credits / usage** is derived: a notice
//! or message preview whose text mentions running out of usage or credits becomes a
//! "Usage" notification whose context is Settings → Usage & Billing.
//!
//! Read state and Clear all are local (per account, `notifications-<account>.json` next to
//! `context.db`); cleared ids are remembered so they don't come back.
use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default toast lifetime (`notifications.toast_timeout_secs`).
pub const DEFAULT_TOAST_SECS: u64 = 20;
/// Notifications kept on the page.
pub const MAX_ITEMS: usize = 200;
/// Ids remembered after a clear so the same notice doesn't come back.
const MAX_SEEN: usize = 2000;
/// Toast box size.
pub const TOAST_W: u16 = 44;
pub const TOAST_H: u16 = 4;
/// Toasts shown at once.
const MAX_TOASTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A Grok Bot system notice from a transcript.
    Notice,
    /// New messages in a bot you aren't looking at.
    Message,
    /// A bot is waiting for your answer.
    Awaiting,
    /// A card is waiting for you.
    Card,
    /// Usage / credits (derived from notice or message text).
    Usage,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Notice => "Notice",
            Kind::Message => "Message",
            Kind::Awaiting => "Waiting for you",
            Kind::Card => "Card",
            Kind::Usage => "Usage",
        }
    }
}

/// Where clicking a notification goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "to", rename_all = "lowercase")]
pub enum Context {
    /// That bot's chat.
    Bot { bot: String },
    /// A card (opens it in that bot's chat).
    Card { bot: String, key: String },
    /// The Settings page, tab index.
    Settings { tab: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// Stable id (source + entry/message id) so a notice is posted once.
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub context: Option<Context>,
    /// Unix ms.
    pub at_ms: i64,
    #[serde(default)]
    pub read: bool,
}

/// A `kind: notice` transcript entry (from [`crate::api::ApiClient::take_notices`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteNotice {
    pub id: String,
    pub text: String,
    pub at_ms: i64,
}

/// Per-bot unread/attention signals from `listAgents`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BotSignal {
    pub bot_id: String,
    pub unread: u32,
    pub awaiting: bool,
    pub last_message_id: Option<String>,
    pub preview: Option<String>,
}

/// Text that reads like a usage/credits warning (the derived "low credits" source).
pub fn is_usage_text(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    [
        "usage ran out",
        "out of usage",
        "usage limit",
        "usage is running low",
        "low on credits",
        "low credits",
        "out of credits",
        "credits ran out",
        "credit balance",
        "top up",
        "top it off",
        "included usage",
    ]
    .iter()
    .any(|w| l.contains(w))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    items: Vec<Notification>,
    seen: VecDeque<String>,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: String,
    pub until: Instant,
}

/// Page state.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub cursor: usize,
    pub confirm_clear: bool,
    pub area: Rect,
    pub rows: Vec<(Rect, usize)>,
    pub clear_area: Rect,
}

#[derive(Debug, Default)]
pub struct Inbox {
    /// Newest first.
    pub items: Vec<Notification>,
    seen: VecDeque<String>,
    pub toasts: Vec<Toast>,
    pub page: Option<Page>,
    pub path: Option<std::path::PathBuf>,
    /// Toast boxes drawn last frame: (box, close button, id).
    pub toast_areas: Vec<(Rect, Rect, String)>,
    /// Rows the toasts used last frame (local toasts draw below them).
    pub toast_rows: u16,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

impl Inbox {
    /// Load the account's saved notifications (missing/corrupt file = empty).
    pub fn load(path: std::path::PathBuf) -> Self {
        let saved: Saved = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            items: saved.items,
            seen: saved.seen,
            path: Some(path),
            ..Self::default()
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(p) = &self.path else {
            return Ok(());
        };
        let s = Saved {
            items: self.items.clone(),
            seen: self.seen.clone(),
        };
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&s)?)?;
        std::fs::rename(tmp, p)
    }

    pub fn unread(&self) -> usize {
        self.items.iter().filter(|n| !n.read).count()
    }

    pub fn known(&self, id: &str) -> bool {
        self.seen.iter().any(|s| s == id)
    }

    /// Add a notification (once per id) and toast it. True when new.
    pub fn post(&mut self, n: Notification, toast_for: Duration, now: Instant) -> bool {
        if self.known(&n.id) {
            return false;
        }
        self.seen.push_back(n.id.clone());
        while self.seen.len() > MAX_SEEN {
            self.seen.pop_front();
        }
        if !toast_for.is_zero() {
            self.toasts.push(Toast {
                id: n.id.clone(),
                until: now + toast_for,
            });
        }
        self.items.insert(0, n);
        self.items.sort_by_key(|n| std::cmp::Reverse(n.at_ms));
        self.items.truncate(MAX_ITEMS);
        true
    }

    /// Drop expired toasts (they stay unread on the page). True when one went away.
    pub fn tick(&mut self, now: Instant) -> bool {
        let n = self.toasts.len();
        self.toasts.retain(|t| t.until > now);
        n != self.toasts.len()
    }

    /// Close a toast (the [x]); it stays unread on the page.
    pub fn dismiss_toast(&mut self, id: &str) {
        self.toasts.retain(|t| t.id != id);
    }

    pub fn mark_read(&mut self, id: &str, read: bool) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.read = read;
        }
    }

    pub fn mark_all_read(&mut self) {
        for n in &mut self.items {
            n.read = true;
        }
    }

    /// Clear all (local; the API has no clear call). Ids stay remembered.
    pub fn clear_all(&mut self) {
        self.items.clear();
        self.toasts.clear();
    }

    pub fn get(&self, id: &str) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }
}

impl App {
    fn inbox_toast_for(&self) -> Duration {
        Duration::from_secs(self.config.toast_timeout_secs)
    }

    fn inbox_post(&mut self, n: Notification) {
        let dur = if self.notices_board.settings.dnd {
            Duration::ZERO
        } else {
            self.inbox_toast_for()
        };
        if self.inbox.post(n, dur, Instant::now()) {
            self.inbox_save();
        }
    }

    fn inbox_save(&mut self) {
        if let Err(e) = self.inbox.save() {
            self.error(format!("Could not save notifications: {e}"));
        }
    }

    fn bot_name(&self, id: &str) -> String {
        self.all_bots
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| "a bot".into())
    }

    /// Grok Bot notices from a transcript poll of `bot`.
    pub fn inbox_notices(&mut self, bot: &str, notices: Vec<RemoteNotice>) {
        let name = self.bot_name(bot);
        for r in notices {
            let usage = is_usage_text(&r.text);
            self.inbox_post(Notification {
                id: format!("notice:{bot}:{}", r.id),
                kind: if usage { Kind::Usage } else { Kind::Notice },
                title: if usage {
                    format!("Usage · {name}")
                } else {
                    format!("Grok Bot · {name}")
                },
                body: r.text,
                context: Some(if usage {
                    Context::Settings { tab: 3 }
                } else {
                    Context::Bot { bot: bot.into() }
                }),
                at_ms: if r.at_ms > 0 { r.at_ms } else { now_ms() },
                read: false,
            });
        }
    }

    /// Unread / waiting signals from a bot-list refresh. The open chat is skipped.
    pub fn inbox_signals(&mut self, signals: Vec<BotSignal>) {
        let open = self.active_bot().map(|b| b.id.clone());
        for s in signals {
            if open.as_deref() == Some(s.bot_id.as_str()) {
                continue;
            }
            let name = self.bot_name(&s.bot_id);
            let tag = s.last_message_id.clone().unwrap_or_else(|| "?".into());
            let preview = s.preview.clone().unwrap_or_default();
            if is_usage_text(&preview) {
                self.inbox_post(Notification {
                    id: format!("usage:{}:{tag}", s.bot_id),
                    kind: Kind::Usage,
                    title: format!("Usage · {name}"),
                    body: preview.clone(),
                    context: Some(Context::Settings { tab: 3 }),
                    at_ms: now_ms(),
                    read: false,
                });
            }
            if s.awaiting {
                self.inbox_post(Notification {
                    id: format!("await:{}:{tag}", s.bot_id),
                    kind: Kind::Awaiting,
                    title: format!("{name} is waiting for you"),
                    body: preview.clone(),
                    context: Some(Context::Bot {
                        bot: s.bot_id.clone(),
                    }),
                    at_ms: now_ms(),
                    read: false,
                });
            } else if s.unread > 0 && s.last_message_id.is_some() {
                self.inbox_post(Notification {
                    id: format!("unread:{}:{tag}", s.bot_id),
                    kind: Kind::Message,
                    title: format!(
                        "{name} · {} new message{}",
                        s.unread,
                        if s.unread == 1 { "" } else { "s" }
                    ),
                    body: preview,
                    context: Some(Context::Bot {
                        bot: s.bot_id.clone(),
                    }),
                    at_ms: now_ms(),
                    read: false,
                });
            }
        }
    }

    /// Pending cards (after `absorb_cards`).
    pub fn inbox_cards(&mut self) {
        let pending: Vec<(String, String, String)> = self
            .cards
            .iter()
            .filter(|(_, (_, c))| c.state.is_pending())
            .map(|(k, (bot, c))| (k.clone(), bot.clone(), c.summary()))
            .collect();
        for (key, bot, summary) in pending {
            let name = self.bot_name(&bot);
            self.inbox_post(Notification {
                id: format!("card:{key}"),
                kind: Kind::Card,
                title: format!("Card from {name}"),
                body: summary,
                context: Some(Context::Card {
                    bot: bot.clone(),
                    key,
                }),
                at_ms: now_ms(),
                read: false,
            });
        }
    }

    /// Re-open the inbox for the current account (login / account switch).
    pub fn inbox_reload(&mut self) {
        let alias: String = self
            .account()
            .alias
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let path = self
            .store
            .db_path()
            .with_file_name(format!("notifications-{alias}.json"));
        if self.inbox.path.as_ref() != Some(&path) {
            self.inbox = Inbox::load(path);
        }
    }

    /// Go to a notification's context, if it still exists. Marks it read.
    pub fn inbox_open(&mut self, id: &str) {
        let Some(n) = self.inbox.get(id).cloned() else {
            return;
        };
        self.inbox.mark_read(id, true);
        self.inbox.dismiss_toast(id);
        self.inbox_save();
        let Some(ctx) = n.context else {
            return;
        };
        let select = |app: &mut App, bot: &str| -> bool {
            match app.bots.iter().position(|b| b.id == bot) {
                Some(i) => {
                    app.select_bot(i);
                    true
                }
                None => {
                    app.info("That bot isn't in your list any more.".into());
                    false
                }
            }
        };
        self.inbox.page = None;
        self.settings = None;
        match ctx {
            Context::Bot { bot } => {
                select(self, &bot);
            }
            Context::Card { bot, key } => {
                if select(self, &bot) && !self.open_card(Some(&key)) {
                    self.info("That card isn't available any more.".into());
                }
            }
            Context::Settings { tab } => self.open_settings(Some(tab)),
        }
        self.needs_clear = true;
    }

    pub fn open_notifications(&mut self) {
        self.inbox.page = Some(Page::default());
        self.tooltip.dismiss();
    }

    pub fn close_notifications(&mut self) {
        self.inbox.page = None;
        self.needs_clear = true;
    }

    /// Left click on a toast: [x] closes it (stays unread); the box opens its context
    /// and marks it read. True when consumed.
    pub fn inbox_toast_click(&mut self, col: u16, row: u16) -> bool {
        let at = Position::new(col, row);
        let Some((_, close, id)) = self
            .inbox
            .toast_areas
            .iter()
            .find(|(r, _, _)| r.contains(at))
            .cloned()
        else {
            return false;
        };
        if close.contains(at) {
            self.inbox.dismiss_toast(&id);
        } else {
            self.inbox_open(&id);
        }
        true
    }

    pub fn notifications_key(&mut self, k: KeyEvent) {
        let n = self.inbox.items.len();
        let Some(p) = self.inbox.page.as_mut() else {
            return;
        };
        if p.confirm_clear {
            p.confirm_clear = false;
            if matches!(
                k.code,
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
            ) {
                self.inbox.clear_all();
                self.inbox_save();
                self.info("Notifications cleared (on this computer; the API has no clear).".into());
            }
            return;
        }
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_notifications(),
            KeyCode::Down | KeyCode::Char('j') => {
                p.cursor = (p.cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => p.cursor = p.cursor.saturating_sub(1),
            KeyCode::Char('c') | KeyCode::Char('C') if n > 0 => p.confirm_clear = true,
            KeyCode::Char('a') => {
                self.inbox.mark_all_read();
                self.inbox_save();
            }
            KeyCode::Char('m') | KeyCode::Char(' ') => {
                let i = p.cursor;
                if let Some(it) = self.inbox.items.get(i) {
                    let (id, r) = (it.id.clone(), it.read);
                    self.inbox.mark_read(&id, !r);
                    self.inbox_save();
                }
            }
            KeyCode::Enter => {
                if let Some(id) = self.inbox.items.get(p.cursor).map(|n| n.id.clone()) {
                    self.inbox_open(&id);
                }
            }
            _ => {}
        }
    }

    pub fn notifications_click(&mut self, col: u16, row: u16) {
        let at = Position::new(col, row);
        let Some(p) = self.inbox.page.as_mut() else {
            return;
        };
        if !p.area.contains(at) {
            self.close_notifications();
            return;
        }
        if p.clear_area.contains(at) && !self.inbox.items.is_empty() {
            p.confirm_clear = true;
            return;
        }
        if let Some(&(_, i)) = p.rows.iter().find(|(r, _)| r.contains(at)) {
            p.cursor = i;
            if let Some(id) = self.inbox.items.get(i).map(|n| n.id.clone()) {
                self.inbox_open(&id);
            }
        }
    }
}

fn ago(at_ms: i64) -> String {
    let s = ((now_ms() - at_ms) / 1000).max(0);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86399 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// Toasts: small boxes stacked in the top-right corner, each with a close [x].
pub fn draw_toasts(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.inbox.toast_areas.clear();
    app.inbox.toast_rows = 0;
    if area.width < TOAST_W + 2 || area.height < TOAST_H + 4 {
        return;
    }
    let ids: Vec<String> = app
        .inbox
        .toasts
        .iter()
        .rev()
        .take(MAX_TOASTS)
        .map(|t| t.id.clone())
        .collect();
    // Below the header, so the bell and buttons stay clickable.
    let start = (area.y + 1).max(app.sidebar_area.y);
    let mut y = start;
    for id in ids {
        let Some(n) = app.inbox.get(&id).cloned() else {
            continue;
        };
        if y + TOAST_H > area.bottom() {
            break;
        }
        let r = Rect::new(area.right() - TOAST_W - 1, y, TOAST_W, TOAST_H);
        let close = Rect::new(r.right() - 4, r.y, 3, 1);
        f.render_widget(Clear, r);
        let title: String = n.title.chars().take(TOAST_W as usize - 8).collect();
        let block = Block::new()
            .borders(Borders::ALL)
            .title(format!(" 🔔 {title} "))
            .style(theme::notification_toast())
            .border_style(theme::notification_toast());
        let inner = block.inner(r);
        f.render_widget(block, r);
        f.render_widget(
            Paragraph::new(Span::styled("[x]", theme::notification_unread())),
            close,
        );
        let body = if n.body.trim().is_empty() {
            n.kind.label().to_string()
        } else {
            n.body.split_whitespace().collect::<Vec<_>>().join(" ")
        };
        f.render_widget(Paragraph::new(body).wrap(Wrap { trim: true }), inner);
        app.inbox.toast_areas.push((r, close, id));
        y += TOAST_H;
    }
    app.inbox.toast_rows = y.saturating_sub(area.y + 1);
    let _ = start;
}

/// The notifications page.
pub fn draw_page(f: &mut Frame, app: &mut App) {
    let full = f.area();
    let w = full.width.saturating_sub(4).min(100);
    let h = full.height.saturating_sub(2);
    let area = Rect::new(full.x + (full.width - w) / 2, full.y + 1, w, h);
    let unread = app.inbox.unread();
    let total = app.inbox.items.len();
    let Some(p) = app.inbox.page.as_mut() else {
        return;
    };
    p.area = area;
    p.rows.clear();
    f.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::ALL)
        .title(format!(" 🔔 Notifications · {unread} unread / {total} "))
        .title_bottom(
            " ↑/↓ move · Enter open · m read/unread · a all read · c clear all · Esc back ",
        )
        .style(theme::settings_frame())
        .border_style(theme::settings_frame());
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let clear = " [Clear all] ";
    p.clear_area = Rect::new(
        inner.right().saturating_sub(clear.len() as u16),
        inner.y,
        clear.len() as u16,
        1,
    );
    let head = if p.confirm_clear {
        "Clear all notifications on this computer? y/n".to_string()
    } else {
        "From Grok Bot: notices, new messages, waiting bots, cards, usage. Stored locally.".into()
    };
    f.render_widget(
        Paragraph::new(Span::styled(head, theme::key_hint())),
        Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(clear.len() as u16),
            1,
        ),
    );
    f.render_widget(
        Paragraph::new(Span::styled(clear, theme::settings_button())),
        p.clear_area,
    );
    let top = inner.y + 2;
    if total == 0 {
        f.render_widget(
            Paragraph::new("No notifications."),
            Rect::new(inner.x + 1, top, inner.width.saturating_sub(1), 1),
        );
        return;
    }
    let per = 2u16;
    let room = (inner.bottom().saturating_sub(top) / per) as usize;
    let skip = p.cursor.saturating_sub(room.saturating_sub(1));
    for (i, n) in app.inbox.items.iter().enumerate().skip(skip).take(room) {
        let y = top + (i - skip) as u16 * per;
        let sel = i == p.cursor;
        let st = if sel {
            theme::settings_tab()
        } else if n.read {
            theme::notification_read()
        } else {
            theme::notification_unread()
        };
        let mark = if n.read { "  " } else { "● " };
        let l1 = Line::from(vec![
            Span::styled(format!("{mark}{} ", n.title), st),
            Span::styled(
                format!(
                    " {} · {}{}",
                    n.kind.label(),
                    ago(n.at_ms),
                    if n.read { " · read" } else { "" }
                ),
                theme::key_hint(),
            ),
        ]);
        let body: String = n.body.split_whitespace().collect::<Vec<_>>().join(" ");
        let l2 = Line::from(Span::styled(
            format!("  {body}"),
            if n.read {
                theme::notification_read()
            } else {
                theme::settings_frame()
            },
        ));
        let r = Rect::new(inner.x, y, inner.width, per);
        p.rows.push((r, i));
        f.render_widget(Paragraph::new(vec![l1, l2]), r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::{mock_login, settle, test_app, type_line};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyModifiers;

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
        // test_app turns DND on for the local toasts; Grok Bot toasts are tested here.
        app.notices_board.settings.dnd = false;
        app.inbox_reload();
        (rt, app, d)
    }
    fn other_bot(app: &App) -> String {
        let open = app.active_bot().unwrap().id.clone();
        app.bots
            .iter()
            .find(|b| b.id != open && !b.team)
            .unwrap()
            .id
            .clone()
    }
    fn key(app: &mut App, c: KeyCode) {
        app.notifications_key(KeyEvent::new(c, KeyModifiers::NONE));
    }

    #[test]
    fn usage_text_is_detected() {
        assert!(is_usage_text("Your Grok Bot usage ran out."));
        assert!(is_usage_text("You're low on credits"));
        assert!(!is_usage_text(
            "A background task was stopped after 50 minutes"
        ));
    }

    #[test]
    fn notice_from_transcript_toasts_and_counts_unread() {
        let (_rt, mut app, _d) = ready();
        type_line(&mut app, "notice A background task was stopped.");
        settle(&mut app);
        app.poll_now();
        settle(&mut app);
        assert_eq!(app.inbox.unread(), 1);
        assert_eq!(app.inbox.toasts.len(), 1);
        let s = screen(&mut app);
        assert!(s.contains("🔔") && s.contains(" 1]"), "{s}");
        assert!(s.contains("A background task"), "{s}");
        assert!(s.contains("[x]"), "{s}");
        // The same notice on the next poll isn't posted again.
        app.poll_now();
        settle(&mut app);
        assert_eq!(app.inbox.items.len(), 1);
    }

    #[test]
    fn close_and_timeout_keep_it_unread_click_marks_read_and_jumps() {
        let (_rt, mut app, _d) = ready();
        let bot = other_bot(&app);
        app.inbox_signals(vec![BotSignal {
            bot_id: bot.clone(),
            unread: 2,
            awaiting: false,
            last_message_id: Some("m1".into()),
            preview: Some("hello there".into()),
        }]);
        assert_eq!(app.inbox.unread(), 1);
        screen(&mut app);
        // [x] closes the toast only.
        let (_, close, _) = app.inbox.toast_areas[0].clone();
        assert!(app.inbox_toast_click(close.x, close.y));
        assert!(app.inbox.toasts.is_empty());
        assert_eq!(app.inbox.unread(), 1, "closed toast stays unread");
        // A second one times out.
        app.inbox_signals(vec![BotSignal {
            bot_id: bot.clone(),
            unread: 3,
            awaiting: false,
            last_message_id: Some("m2".into()),
            preview: None,
        }]);
        assert_eq!(app.inbox.toasts.len(), 1);
        app.inbox
            .tick(Instant::now() + Duration::from_secs(DEFAULT_TOAST_SECS + 1));
        assert!(app.inbox.toasts.is_empty());
        assert_eq!(app.inbox.unread(), 2, "timed-out toast stays unread");
        // A third: clicking the box marks it read and opens that bot's chat.
        app.inbox_signals(vec![BotSignal {
            bot_id: bot.clone(),
            unread: 4,
            awaiting: true,
            last_message_id: Some("m3".into()),
            preview: Some("Which color?".into()),
        }]);
        screen(&mut app);
        let (r, _, id) = app.inbox.toast_areas[0].clone();
        assert!(app.inbox_toast_click(r.x + 1, r.y + 2));
        assert!(app.inbox.get(&id).unwrap().read);
        assert!(app.inbox.toasts.is_empty());
        assert_eq!(app.active_bot().unwrap().id, bot);
        assert_eq!(app.inbox.unread(), 2, "only unread ones count");
    }

    #[test]
    fn page_lists_newest_first_marks_read_and_clears_all() {
        let (_rt, mut app, _d) = ready();
        let bot = other_bot(&app);
        for (i, m) in ["m1", "m2"].iter().enumerate() {
            app.inbox_signals(vec![BotSignal {
                bot_id: bot.clone(),
                unread: i as u32 + 1,
                awaiting: false,
                last_message_id: Some(m.to_string()),
                preview: Some(format!("preview {m}")),
            }]);
            std::thread::sleep(Duration::from_millis(3));
        }
        let b = {
            screen(&mut app);
            app.bell_area
        };
        app.click(b.x, b.y);
        assert!(app.inbox.page.is_some());
        let s = screen(&mut app);
        let (p2, p1) = (s.find("preview m2").unwrap(), s.find("preview m1").unwrap());
        assert!(p2 < p1, "newest first\n{s}");
        key(&mut app, KeyCode::Char('m'));
        assert!(app.inbox.items[0].read);
        assert!(screen(&mut app).contains("· read"));
        assert_eq!(app.inbox.unread(), 1);
        key(&mut app, KeyCode::Char('c'));
        assert!(screen(&mut app).contains("Clear all notifications on this computer? y/n"));
        key(&mut app, KeyCode::Char('y'));
        assert!(app.inbox.items.is_empty());
        // Cleared ones don't come back.
        app.inbox_signals(vec![BotSignal {
            bot_id: bot.clone(),
            unread: 1,
            awaiting: false,
            last_message_id: Some("m1".into()),
            preview: None,
        }]);
        assert!(app.inbox.items.is_empty());
        key(&mut app, KeyCode::Esc);
        assert!(app.inbox.page.is_none());
    }

    #[test]
    fn read_state_persists_and_usage_goes_to_settings() {
        let (_rt, mut app, _d) = ready();
        app.inbox_notices(
            "x",
            vec![RemoteNotice {
                id: "n1".into(),
                text: "Your usage ran out. Top up to keep going.".into(),
                at_ms: now_ms(),
            }],
        );
        let id = app.inbox.items[0].id.clone();
        assert_eq!(app.inbox.items[0].kind, Kind::Usage);
        app.inbox_open(&id);
        assert_eq!(
            app.settings.as_ref().map(|s| s.tab),
            Some(3),
            "Usage & Billing"
        );
        // Reload from disk: still there, still read.
        let path = app.inbox.path.clone().unwrap();
        let again = Inbox::load(path);
        assert_eq!(again.items.len(), 1);
        assert!(again.items[0].read);
        assert!(again.known(&id));
    }

    #[test]
    fn command_and_alt_b() {
        let (_rt, mut app, _d) = ready();
        type_line(&mut app, "/notifications");
        assert!(app.inbox.page.is_some());
        app.close_notifications();
        type_line(&mut app, "/notifications clear");
        assert!(app.inbox.page.is_none());
        assert_eq!(app.config.toast_timeout_secs, DEFAULT_TOAST_SECS);
    }
}
