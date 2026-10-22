//! Hover tooltips with a delay. Motion is coalesced by the event loop; we only
//! dirty the screen when the tip appears, changes, or dismisses.
//!
//! Adapted from acp-rs-scraper's `src/tui/tooltip.rs` (commit a60c82a).

use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::time::{Duration, Instant};

/// How long the pointer must sit still on a control before the tip shows.
pub const HOVER_DELAY: Duration = Duration::from_millis(400);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tip {
    pub text: String,
    pub topic: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hit {
    MenuBar(usize),
    MenuOpt(usize),
    ThemeSwitch,
    AccountSwitch,
    UnreadBadge,
    #[allow(dead_code)] // tip text ready; hit area not tracked yet
    MouseToggle,
}

#[derive(Debug, Default)]
pub struct TooltipState {
    pub tip: Option<Tip>,
    pub tip_at: Rect,
    /// Control under the pointer (for delay).
    pub pending: Option<Hit>,
    pub pending_since: Option<Instant>,
    pub pending_at: Rect,
}

impl TooltipState {
    /// Pointer moved over `hit` (or `None` if nowhere tippable). Returns whether
    /// the visible tip changed (caller should redraw).
    pub fn pointer(&mut self, hit: Option<(Hit, Rect)>, now: Instant) -> bool {
        match hit {
            None => {
                let changed = self.tip.take().is_some();
                self.pending = None;
                self.pending_since = None;
                changed
            }
            Some((h, r)) => {
                if self.pending != Some(h) {
                    self.pending = Some(h);
                    self.pending_since = Some(now);
                    self.pending_at = r;
                    // Keep showing the old tip until the new delay elapses, unless
                    // we left the previous control entirely.
                    if self.tip.is_some() {
                        self.tip = None;
                        return true;
                    }
                    return false;
                }
                self.pending_at = r;
                false
            }
        }
    }

    /// Promote a pending hover into a visible tip after [`HOVER_DELAY`].
    pub fn tick(&mut self, now: Instant, resolve: impl FnOnce(Hit) -> Option<Tip>) -> bool {
        let Some(h) = self.pending else {
            return false;
        };
        let Some(since) = self.pending_since else {
            return false;
        };
        if now.duration_since(since) < HOVER_DELAY {
            return false;
        }
        let Some(tip) = resolve(h) else {
            return false;
        };
        if self.tip.as_ref() == Some(&tip) && self.tip_at == self.pending_at {
            return false;
        }
        self.tip = Some(tip);
        self.tip_at = self.pending_at;
        true
    }

    pub fn dismiss(&mut self) -> bool {
        let changed = self.tip.take().is_some();
        self.pending = None;
        self.pending_since = None;
        changed
    }
}

pub fn tip_for(hit: Hit) -> Option<Tip> {
    Some(match hit {
        Hit::MenuBar(i) => {
            let m = crate::menubar::MENUS.get(i)?;
            Tip {
                text: format!("{} menu (F10, Alt+{})", m.label, m.hint),
                topic: "keys",
            }
        }
        Hit::MenuOpt(_) => Tip {
            text: "Run this command (Enter)".into(),
            topic: "keys",
        },
        Hit::ThemeSwitch => Tip {
            text: "Theme picker (F5)".into(),
            topic: "theme",
        },
        Hit::AccountSwitch => Tip {
            text: "Account picker (F3)".into(),
            topic: "accounts",
        },
        Hit::UnreadBadge => Tip {
            text: "Jump to new messages (End)".into(),
            topic: "keys",
        },
        Hit::MouseToggle => Tip {
            text: "Mouse capture (F6)".into(),
            topic: "keys",
        },
    })
}

pub fn paint(f: &mut Frame, tip: &Tip, anchor: Rect, screen: Rect) {
    use crate::theme;
    let link = format!("? {}", tip.topic);
    let width = tip
        .text
        .chars()
        .count()
        .max(link.chars().count())
        .saturating_add(2);
    let w = (width as u16).clamp(8, screen.width.max(1));
    let h = 4u16.min(screen.height);
    if h < 3 || w < 4 {
        return;
    }
    let x = anchor
        .x
        .min(screen.x.saturating_add(screen.width.saturating_sub(w)));
    let mut y = anchor.y.saturating_add(anchor.height.max(1));
    if y.saturating_add(h) > screen.y.saturating_add(screen.height) {
        y = anchor.y.saturating_sub(h);
    }
    if y < screen.y {
        y = screen.y;
    }
    let r = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let b = theme::block(&theme::tooltip());
    let inner = b.inner(r);
    f.render_widget(b, r);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let cols = inner.width as usize;
    let text: String = tip.text.chars().take(cols).collect();
    f.render_widget(
        Paragraph::new(theme::tooltip().span(text)),
        Rect {
            height: 1,
            ..inner
        },
    );
    if inner.height >= 2 {
        let link: String = link.chars().take(cols).collect();
        f.render_widget(
            Paragraph::new(Span::styled(
                link,
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::UNDERLINED),
            )),
            Rect {
                y: inner.y + 1,
                height: 1,
                ..inner
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tip_waits_for_hover_delay() {
        let mut s = TooltipState::default();
        let t0 = Instant::now();
        let r = Rect::new(0, 0, 5, 1);
        assert!(!s.pointer(Some((Hit::ThemeSwitch, r)), t0));
        assert!(s.tip.is_none());
        assert!(!s.tick(t0 + Duration::from_millis(100), tip_for));
        assert!(s.tick(t0 + HOVER_DELAY + Duration::from_millis(1), tip_for));
        assert!(s.tip.is_some());
        assert!(s.dismiss());
        assert!(s.tip.is_none());
    }

    #[test]
    fn leaving_a_control_clears_the_tip() {
        let mut s = TooltipState::default();
        let t0 = Instant::now();
        let r = Rect::new(0, 0, 5, 1);
        s.pointer(Some((Hit::UnreadBadge, r)), t0);
        s.tick(t0 + HOVER_DELAY, tip_for);
        assert!(s.pointer(None, t0 + HOVER_DELAY));
        assert!(s.tip.is_none());
    }
}
