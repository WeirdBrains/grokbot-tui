//! Loading overlay: a centered spinner + stage message shown while signing in, loading
//! the bot list and the first chat (startup, `/account use`, `/login …`). Purely UI
//! state; the work itself already runs on background tasks, so the spinner animates
//! on the normal UI tick and input (Ctrl-C, Ctrl-Q) keeps working.

use std::time::Instant;

/// Braille spinner frames.
pub const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Milliseconds per spinner frame.
pub const FRAME_MS: u128 = 100;

/// What the app is waiting for right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Restoring / checking a credential for this account.
    SigningIn(String),
    /// `/login gateway`: validating the gateway URL + token.
    CheckingGateway(String),
    /// `/login cursor` (or a desktop Cursor token): the EnsureSandBox exchange.
    ExchangingCursor,
    /// `/login desktop`: reading the desktop app's saved session.
    ReadingDesktop,
    /// Signed in; fetching the bot list.
    LoadingBots,
    /// Bots loaded; fetching the open chat.
    LoadingChat(String),
}

impl Stage {
    pub fn message(&self) -> String {
        match self {
            Stage::SigningIn(a) => format!("Signing in to {a}…"),
            Stage::CheckingGateway(h) => format!("Checking gateway {h}…"),
            Stage::ExchangingCursor => "Exchanging Cursor token…".into(),
            Stage::ReadingDesktop => "Reading the desktop app session…".into(),
            Stage::LoadingBots => "Loading bots…".into(),
            Stage::LoadingChat(b) => format!("Loading chat with {b}…"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Loading {
    /// Frame title, e.g. "Starting grokbot-tui" or "Switching to work".
    pub title: String,
    pub stage: Stage,
    pub started: Instant,
    /// Set on failure: shown in place of the spinner with a retry hint.
    pub error: Option<String>,
}

impl Loading {
    pub fn new(title: impl Into<String>, stage: Stage) -> Self {
        Self {
            title: title.into(),
            stage,
            started: Instant::now(),
            error: None,
        }
    }

    /// The spinner glyph for `now` (static first frame when animation is off).
    pub fn frame(&self, now: Instant, animate: bool) -> &'static str {
        if !animate {
            return FRAMES[0];
        }
        let n = now.saturating_duration_since(self.started).as_millis() / FRAME_MS;
        FRAMES[(n % FRAMES.len() as u128) as usize]
    }
}

/// Draw the overlay centered in `area`.
pub fn draw(f: &mut ratatui::Frame, l: &Loading, animate: bool, area: ratatui::layout::Rect) {
    use crate::theme;
    use ratatui::layout::Rect;
    use ratatui::text::Line;
    use ratatui::widgets::{Clear, Paragraph, Wrap};
    let (st, lines): (ratatui_lipgloss::Style, Vec<Line>) = match &l.error {
        None => {
            let s = theme::loading();
            let line = Line::from(vec![
                theme::spinner().span(format!("{} ", l.frame(Instant::now(), animate))),
                s.span(l.stage.message()),
            ]);
            (s, vec![line])
        }
        Some(e) => {
            let s = theme::toast(crate::notify::NoticeKind::Error);
            (
                s.clone(),
                vec![
                    Line::from(s.span(format!("✗ {e}"))),
                    Line::from(""),
                    Line::from(s.span(
                        "Esc: dismiss · /login desktop|gateway|cursor to retry · /account use <name>",
                    )),
                ],
            )
        }
    };
    let text_w = lines.iter().map(|l| l.width()).max().unwrap_or(0) as u16;
    let w = (text_w + 4)
        .max(l.title.chars().count() as u16 + 6)
        .clamp(24, area.width.saturating_sub(2).max(24))
        .min(area.width);
    let inner_w = w.saturating_sub(2).max(1);
    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    let h = (p.line_count(inner_w) as u16 + 2).min(area.height);
    let r = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    let block = theme::block(&st).title(format!(" {} ", l.title));
    f.render_widget(p.style(&st).block(block), r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spinner_advances_and_respects_off() {
        let l = Loading::new("t", Stage::LoadingBots);
        let t0 = l.started;
        assert_eq!(l.frame(t0, true), FRAMES[0]);
        assert_eq!(
            l.frame(t0 + Duration::from_millis(FRAME_MS as u64), true),
            FRAMES[1]
        );
        assert_eq!(
            l.frame(t0 + Duration::from_millis(FRAME_MS as u64 * 10), true),
            FRAMES[0]
        );
        assert_eq!(l.frame(t0 + Duration::from_millis(500), false), FRAMES[0]);
    }

    #[test]
    fn stage_messages() {
        assert_eq!(
            Stage::SigningIn("work".into()).message(),
            "Signing in to work…"
        );
        assert_eq!(
            Stage::ExchangingCursor.message(),
            "Exchanging Cursor token…"
        );
        assert_eq!(Stage::LoadingBots.message(), "Loading bots…");
        assert_eq!(
            Stage::LoadingChat("Ada".into()).message(),
            "Loading chat with Ada…"
        );
    }

    fn render(l: &Loading) -> String {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 12)).unwrap();
        term.draw(|f| draw(f, l, false, f.area())).unwrap();
        let buf = term.backend().buffer().clone();
        buf.content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn overlay_renders_spinner_then_error() {
        let mut l = Loading::new("Starting grokbot-tui", Stage::SigningIn("default".into()));
        let out = render(&l);
        assert!(out.contains("Starting grokbot-tui"));
        assert!(out.contains("⠋ Signing in to default…"));
        l.error = Some("Sign-in failed: boom".into());
        let out = render(&l);
        assert!(out.contains("✗ Sign-in failed: boom"));
        assert!(out.contains("Esc: dismiss"));
        assert!(!out.contains("⠋"));
    }
}
