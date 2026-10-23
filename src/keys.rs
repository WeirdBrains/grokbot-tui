//! Key decoding helpers: newlines across terminals (incl. macOS Terminal.app).
//!
//! Terminal.app sends plain CR for Shift-/Ctrl-Enter and has no kitty keyboard
//! protocol. Alt-Enter arrives as ESC then CR when "Use Option as Meta key" is on.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// How long to wait after Esc for a following Enter (Alt-Enter as ESC+CR).
pub const ESC_META_WINDOW: Duration = Duration::from_millis(50);

/// Pending Esc that may combine with the next key into Alt+Enter.
#[derive(Debug, Default)]
pub struct EscMeta {
    pub at: Option<Instant>,
}

impl EscMeta {
    /// Feed a key. Returns the key to handle (possibly rewritten as Alt+Enter),
    /// and whether a lone Esc should fire (timeout with nothing after).
    pub fn feed(&mut self, k: KeyEvent, now: Instant) -> Feed {
        if k.code == KeyCode::Esc
            && k.modifiers.is_empty()
            && !matches!(k.kind, ratatui::crossterm::event::KeyEventKind::Release)
        {
            self.at = Some(now);
            return Feed::Hold;
        }
        if let Some(t0) = self.at.take() {
            if now.duration_since(t0) <= ESC_META_WINDOW
                && k.code == KeyCode::Enter
                && !k.modifiers.contains(KeyModifiers::CONTROL)
            {
                // ESC+CR → Alt+Enter (newline).
                return Feed::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
            }
            // Esc was something else: emit Esc first, then this key.
            return Feed::EscThen(k);
        }
        Feed::Key(k)
    }

    /// If a held Esc has aged out, emit it.
    pub fn poll_timeout(&mut self, now: Instant) -> Option<KeyEvent> {
        let t0 = self.at?;
        if now.duration_since(t0) > ESC_META_WINDOW {
            self.at = None;
            Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Feed {
    Hold,
    Key(KeyEvent),
    EscThen(KeyEvent),
}

/// Whether this key should insert a newline (not submit).
///
/// Includes Shift/Alt/Ctrl+Enter when the terminal reports modifiers, and the
/// remappable action id `input.newline` bindings resolve here for tests via
/// [`is_newline_chord`].
pub fn is_newline_key(k: &KeyEvent) -> bool {
    match k.code {
        KeyCode::Enter => k
            .modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL),
        KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => true,
        // Ctrl-Enter sometimes arrives as Char('\n') with CONTROL on some stacks.
        KeyCode::Char('\n') => true,
        _ => false,
    }
}

/// Encode a key the way [`crate::keymap::encode_key`] does, for newline remaps.
#[cfg(test)]
pub fn is_newline_chord(encoded: &str) -> bool {
    matches!(
        encoded,
        "shift-enter" | "alt-enter" | "ctrl-enter" | "ctrl-j" | "enter" // only when mapped to input.newline in insert mode
    ) || encoded == "input.newline"
}

/// If `input` ends with a single trailing backslash (not escaped), strip it and
/// treat Enter as a newline. Returns the new input when that applies.
pub fn trailing_backslash_newline(input: &str) -> Option<String> {
    if input.ends_with("\\\\") {
        return None;
    }
    if let Some(rest) = input.strip_suffix('\\') {
        return Some(format!("{rest}\n"));
    }
    None
}

/// True when we should show the one-time Apple Terminal newline hint.
pub fn apple_terminal_needs_hint(enhancement_ok: bool, term_program: Option<&str>) -> bool {
    !enhancement_ok && term_program == Some("Apple_Terminal")
}

pub const APPLE_TERMINAL_HINT: &str = "Terminal.app: Shift/Ctrl-Enter send the prompt. Use Ctrl-J, type \\ then Enter, \
     or Option-Enter with “Use Option as Meta key”. See /help keys.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_then_enter_becomes_alt_enter() {
        let mut m = EscMeta::default();
        let t0 = Instant::now();
        assert_eq!(
            m.feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), t0),
            Feed::Hold
        );
        match m.feed(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            t0 + Duration::from_millis(10),
        ) {
            Feed::Key(k) => {
                assert_eq!(k.code, KeyCode::Enter);
                assert!(k.modifiers.contains(KeyModifiers::ALT));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn esc_alone_times_out() {
        let mut m = EscMeta::default();
        let t0 = Instant::now();
        m.feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), t0);
        let k = m
            .poll_timeout(t0 + ESC_META_WINDOW + Duration::from_millis(1))
            .unwrap();
        assert_eq!(k.code, KeyCode::Esc);
    }

    #[test]
    fn newline_keys() {
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn newline_chord_names() {
        assert!(is_newline_chord("ctrl-j"));
        assert!(is_newline_chord("alt-enter"));
        assert!(is_newline_chord("shift-enter"));
        assert!(is_newline_chord("ctrl-enter"));
        assert!(is_newline_chord("input.newline"));
        assert!(is_newline_chord("enter")); // allowed when mapped to input.newline
        assert!(!is_newline_chord("ctrl-m"));
    }

    #[test]
    fn trailing_backslash() {
        assert_eq!(
            trailing_backslash_newline("hello\\"),
            Some("hello\n".into())
        );
        assert_eq!(trailing_backslash_newline("hello\\\\"), None);
        assert_eq!(trailing_backslash_newline("hello"), None);
    }

    #[test]
    fn apple_hint_only_without_enhancement() {
        assert!(apple_terminal_needs_hint(false, Some("Apple_Terminal")));
        assert!(!apple_terminal_needs_hint(true, Some("Apple_Terminal")));
        assert!(!apple_terminal_needs_hint(false, Some("iTerm.app")));
    }
}
