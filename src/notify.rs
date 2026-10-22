//! Local notifications: toast stack, do-not-disturb, per-kind switches.
//!
//! Adapted from acp-rs-scraper's `src/notify.rs` (commit a60c82a) and
//! PLAN-3.0-CLIENT.md §7.14.13. Deviations: see `docs/notifications-menu-tooltip-plan.md`.
//!
//! Toasts are an overlay. They do **not** change the chat scroll (1.9.1 rule).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a toast stays up.
pub const TOAST_TTL: Duration = Duration::from_secs(6);
/// Rows one toast occupies.
pub const TOAST_ROWS: u16 = 2;

/// A kind of notice. Each one can be muted on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NoticeKind {
    Error = 0,
    Warning = 1,
    Info = 2,
    /// A new chat message while the user may be elsewhere.
    Message = 3,
}

impl NoticeKind {
    pub const ALL: [NoticeKind; 4] = [
        NoticeKind::Error,
        NoticeKind::Warning,
        NoticeKind::Info,
        NoticeKind::Message,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Info => "Info",
            Self::Message => "Message",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Message => "message",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warning" | "warn" => Some(Self::Warning),
            "info" | "system" => Some(Self::Info),
            "message" | "chat" => Some(Self::Message),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(kind: NoticeKind, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            body: body.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeSettings {
    pub dnd: bool,
    pub on: [bool; 4],
}

impl Default for NoticeSettings {
    fn default() -> Self {
        Self {
            dnd: false,
            // Info stays in the chat feed; toast only for error/warning/message by default.
            on: [true, true, false, true],
        }
    }
}

impl NoticeSettings {
    pub fn allows(&self, kind: NoticeKind) -> bool {
        !self.dnd && self.on[kind.index()]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub notice: Notice,
    pub until: Instant,
}

/// Toast stack + settings. The chat feed scroll is never touched here.
pub struct NoticeBoard {
    pub log: VecDeque<Notice>,
    pub toasts: Vec<Toast>,
    pub settings: NoticeSettings,
}

impl Default for NoticeBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl NoticeBoard {
    pub fn new() -> Self {
        Self {
            log: VecDeque::new(),
            toasts: Vec::new(),
            settings: NoticeSettings::default(),
        }
    }

    /// Record the notice. Raise a toast only when settings allow.
    pub fn post(&mut self, notice: Notice, now: Instant) {
        self.log.push_front(notice.clone());
        while self.log.len() > 200 {
            self.log.pop_back();
        }
        if self.settings.allows(notice.kind) {
            self.toasts.push(Toast {
                notice,
                until: now + TOAST_TTL,
            });
        }
    }

    pub fn tick(&mut self, now: Instant) {
        self.toasts.retain(|t| t.until > now);
    }

    /// How many toasts to show (cap by a quarter of the screen height).
    pub fn toast_window(&self, height: u16) -> usize {
        if self.toasts.is_empty() || height == 0 {
            return 0;
        }
        let quarter = (height / 4).max(TOAST_ROWS);
        let max = (quarter / TOAST_ROWS) as usize;
        max.min(self.toasts.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dnd_and_kind_switch_keep_the_log_but_skip_toasts() {
        let mut b = NoticeBoard::new();
        let now = Instant::now();
        b.settings.dnd = true;
        b.post(Notice::new(NoticeKind::Error, "Error", "quiet"), now);
        assert!(b.toasts.is_empty());
        assert_eq!(b.log[0].body, "quiet");
        b.settings.dnd = false;
        b.settings.on[NoticeKind::Info.index()] = false;
        b.post(Notice::new(NoticeKind::Info, "Info", "held"), now);
        assert!(b.toasts.iter().all(|t| t.notice.body != "held"));
        assert_eq!(b.log[0].body, "held");
        b.post(Notice::new(NoticeKind::Error, "Error", "loud"), now);
        assert!(b.toasts.iter().any(|t| t.notice.body == "loud"));
    }

    #[test]
    fn tick_expires_toasts() {
        let mut b = NoticeBoard::new();
        let now = Instant::now();
        b.post(Notice::new(NoticeKind::Warning, "W", "soon"), now);
        assert_eq!(b.toasts.len(), 1);
        b.tick(now + TOAST_TTL + Duration::from_millis(1));
        assert!(b.toasts.is_empty());
        assert_eq!(b.log.len(), 1);
    }
}
