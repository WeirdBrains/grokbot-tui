//! Key decoding helpers: newlines across terminals (incl. macOS Terminal.app).
//!
//! Terminal.app sends plain CR for Shift-/Ctrl-Enter and has no kitty keyboard
//! protocol. Alt-Enter arrives as ESC then CR when "Use Option as Meta key" is on.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// Default for `keys.esc_alt_timeout_ms`: how long after a lone Esc a following key still
/// counts as Alt+key (terminals send Alt+x as ESC x; over SSH the two bytes can arrive in
/// separate reads, so crossterm sees a plain Esc and then x).
pub const ESC_META_WINDOW: Duration = Duration::from_millis(50);

/// Pending Esc that may combine with the next key into Alt+key.
#[derive(Debug)]
pub struct EscMeta {
    pub at: Option<Instant>,
    /// Zero disables folding (Esc is delivered at once).
    pub window: Duration,
}

impl Default for EscMeta {
    fn default() -> Self {
        Self {
            at: None,
            window: ESC_META_WINDOW,
        }
    }
}

/// Keys that an ESC prefix turns into Alt+key.
fn foldable(k: &KeyEvent) -> bool {
    !k.modifiers
        .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        && matches!(
            k.code,
            KeyCode::Char(_)
                | KeyCode::Enter
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Backspace
        )
}

impl EscMeta {
    pub fn with_window(window: Duration) -> Self {
        Self { at: None, window }
    }

    /// Feed a key. A lone Esc is held for `window`; a foldable key inside it becomes
    /// Alt+key, anything else releases the Esc first.
    pub fn feed(&mut self, k: KeyEvent, now: Instant) -> Feed {
        if k.code == KeyCode::Esc
            && k.modifiers.is_empty()
            && !matches!(k.kind, ratatui::crossterm::event::KeyEventKind::Release)
        {
            if self.window.is_zero() {
                return Feed::Key(k);
            }
            if self.at.replace(now).is_some() {
                // Esc Esc: the first is a real Esc.
                return Feed::EscHold;
            }
            return Feed::Hold;
        }
        if let Some(t0) = self.at.take() {
            if now.duration_since(t0) <= self.window && foldable(&k) {
                let mut alt = k;
                alt.modifiers |= KeyModifiers::ALT;
                return Feed::Key(alt);
            }
            // Esc was something else: emit Esc first, then this key.
            return Feed::EscThen(k);
        }
        Feed::Key(k)
    }

    /// If a held Esc has aged out, emit it.
    pub fn poll_timeout(&mut self, now: Instant) -> Option<KeyEvent> {
        let t0 = self.at?;
        if now.duration_since(t0) > self.window {
            self.at = None;
            Some(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        } else {
            None
        }
    }
}

/// Feed a raw byte stream through crossterm-style decoding as the terminal would deliver
/// it in separate reads: `chunks` are (delay since previous ms, bytes). Test helper.
#[cfg(test)]
pub fn simulate(chunks: &[(u64, &[u8])], window: Duration) -> Vec<KeyEvent> {
    let mut m = EscMeta::with_window(window);
    let mut out = vec![];
    let mut now = Instant::now();
    for (delay, bytes) in chunks {
        now += Duration::from_millis(*delay);
        if let Some(k) = m.poll_timeout(now) {
            out.push(k);
        }
        // Per read: ESC followed by a byte in the SAME read is Alt+key (crossterm does
        // that itself); a trailing ESC is delivered as a plain Esc.
        let mut i = 0;
        while i < bytes.len() {
            let k = if bytes[i] == 0x1b && i + 1 < bytes.len() {
                i += 1;
                KeyEvent::new(KeyCode::Char(bytes[i] as char), KeyModifiers::ALT)
            } else if bytes[i] == 0x1b {
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
            } else if bytes[i] == b'\r' {
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            } else {
                KeyEvent::new(KeyCode::Char(bytes[i] as char), KeyModifiers::NONE)
            };
            i += 1;
            match m.feed(k, now) {
                Feed::Hold => {}
                Feed::Key(k) => out.push(k),
                Feed::EscThen(k) => {
                    out.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                    out.push(k);
                }
                Feed::EscHold => out.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            }
        }
    }
    now += window + Duration::from_millis(1);
    if let Some(k) = m.poll_timeout(now) {
        out.push(k);
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum Feed {
    Hold,
    /// Emit a plain Esc and keep holding the new one.
    EscHold,
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

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }
    fn plain(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn split_esc_prefix_over_ssh_becomes_alt() {
        let w = ESC_META_WINDOW;
        // Same read: already Alt-c.
        assert_eq!(simulate(&[(0, b"\x1bc")], w), vec![alt('c')]);
        // ESC and c in separate reads 5 ms apart (SSH): still Alt-c.
        assert_eq!(simulate(&[(0, b"\x1b"), (5, b"c")], w), vec![alt('c')]);
        // A real Esc, then typing later: Esc, c.
        assert_eq!(
            simulate(&[(0, b"\x1b"), (200, b"c")], w),
            vec![plain(KeyCode::Esc), plain(KeyCode::Char('c'))]
        );
        // Lone Esc still arrives.
        assert_eq!(simulate(&[(0, b"\x1b")], w), vec![plain(KeyCode::Esc)]);
        // Esc Esc: one plain Esc, then the second folds with m into Alt-m.
        assert_eq!(
            simulate(&[(0, b"\x1b"), (2, b"\x1b"), (2, b"m")], w),
            vec![plain(KeyCode::Esc), alt('m')]
        );
        // ESC CR → Alt-Enter (newline), as before.
        assert_eq!(
            simulate(&[(0, b"\x1b"), (3, b"\r")], w),
            vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)]
        );
        // Disabled: no folding.
        assert_eq!(
            simulate(&[(0, b"\x1b"), (5, b"c")], Duration::ZERO),
            vec![plain(KeyCode::Esc), plain(KeyCode::Char('c'))]
        );
        // A longer window catches slower links.
        assert_eq!(
            simulate(&[(0, b"\x1b"), (90, b"c")], Duration::from_millis(120)),
            vec![alt('c')]
        );
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
    fn ctrl_enter_variants_are_newlines_plain_enter_sends() {
        use ratatui::crossterm::event::KeyEventKind;
        // Windows console: VK_RETURN + LEFT_CTRL_PRESSED → Enter with CONTROL.
        let win =
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::CONTROL, KeyEventKind::Press);
        assert!(is_newline_key(&win));
        // kitty / Ghostty / xterm modifyOtherKeys: CSI 13;5u → Enter with CONTROL (+ maybe SHIFT).
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )));
        // Raw-mode LF (terminals mapping Ctrl-Enter to ^J) → Ctrl-J.
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Char('\n'),
            KeyModifiers::CONTROL
        )));
        // Plain Enter (and plain 'j') must still submit / type.
        assert!(!is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
        assert!(!is_newline_key(&KeyEvent::new(
            KeyCode::Char('j'),
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
