//! The look of grokbot-tui: every color, text attribute, border and padding, defined once
//! with [`ratatui_lipgloss`] styles.
//!
//! Drawing code in `ui.rs` only lays things out. To restyle the app, change this file.
//! Lip Gloss styles convert into ratatui styles (`&Style` → `ratatui::style::Style`),
//! build ratatui blocks with [`Style::to_block`], and make spans with [`Style::span`].
use crate::app::EntryKind;
use ratatui::style::Color;
use ratatui::widgets::Block;
use ratatui_lipgloss::{Border, Style};

// ---------------------------------------------------------------- palette

/// Accent: the brand tag, focused borders, selections and bot messages.
const ACCENT: Color = Color::Cyan;
/// Unfocused borders and system notices.
const SUBTLE: Color = Color::Gray;
/// De-emphasised text and the top bar's background.
const MUTED: Color = Color::DarkGray;
/// Modal frames, the account header and warnings.
const WARN: Color = Color::Yellow;
/// Errors and failed shell commands.
const DANGER: Color = Color::Red;
/// Your own messages and successful shell commands.
const OK: Color = Color::Green;
/// Text on accent / warning backgrounds.
const INK: Color = Color::Black;
/// Modal text.
const PAPER: Color = Color::White;

// ---------------------------------------------------------------- top bar

/// The " grokbot-tui " tag at the left of the top bar.
pub fn brand() -> Style {
    Style::new().foreground(INK).background(ACCENT).bold(true)
}

/// The top bar's background.
pub fn top_bar() -> Style {
    Style::new().background(MUTED)
}

/// "mouse: on" in the top bar.
pub fn mouse_on() -> Style {
    Style::new().foreground(MUTED)
}

/// "mouse: OFF" in the top bar: a warning, since clicks don't work while it's off.
pub fn mouse_off() -> Style {
    Style::new().foreground(INK).background(WARN)
}

/// The "[MOCK]" tag.
pub fn mock_tag() -> Style {
    Style::new().foreground(WARN)
}

// ---------------------------------------------------------------- sidebar

/// The clickable "@account" header above the bot list.
pub fn account_header() -> Style {
    Style::new().foreground(WARN).bold(true)
}

/// A highlighted row: the active bot, the picker's selection, the selected help link.
pub fn selected() -> Style {
    Style::new().foreground(INK).background(ACCENT).bold(true)
}

/// The sidebar keyboard cursor (when it isn't on the active bot).
pub fn cursor_row() -> Style {
    Style::new().reverse(true)
}

/// A bordered panel (sidebar, input bar); the border is accented when focused.
pub fn panel(focused: bool) -> Style {
    Style::new()
        .border(Border::normal())
        .border_foreground(if focused { ACCENT } else { SUBTLE })
}

// ---------------------------------------------------------------- chat body

/// The chat body: plain border, 1 row of padding at the top and 2 columns left and right.
pub fn chat_panel() -> Style {
    Style::new().border(Border::normal()).padding((1, 2, 0, 2))
}

/// De-emphasised text: the empty-state guidance, the notices divider, picker details.
pub fn muted() -> Style {
    Style::new().foreground(MUTED)
}

/// Text of a chat entry or notice.
pub fn entry(kind: EntryKind) -> Style {
    Style::new().foreground(match kind {
        EntryKind::System => SUBTLE,
        EntryKind::Error => DANGER,
        EntryKind::User => OK,
        EntryKind::Bot => ACCENT,
    })
}

/// The label of a chat entry ("you", "Helper: ", "! ").
pub fn entry_label(kind: EntryKind) -> Style {
    entry(kind).bold(true)
}

// ---------------------------------------------------------------- modals

/// Modal text (help, picker, shell): white on black.
pub fn modal_text() -> Style {
    Style::new().foreground(PAPER).background(INK)
}

/// A modal's frame in `color`.
fn modal_frame(color: Color) -> Style {
    Style::new()
        .border(Border::normal())
        .border_foreground(color)
}

/// A modal's title in `color`.
fn modal_title(color: Color) -> Style {
    Style::new().foreground(color).bold(true)
}

/// The help and picker modals' frame.
pub fn help_frame() -> Style {
    modal_frame(WARN)
}

/// The help and picker modals' title.
pub fn help_title() -> Style {
    modal_title(WARN)
}

/// The `/shell` modal's color: yellow while running, red on failure, green on success.
fn shell_color(running: bool, failed: bool) -> Color {
    if running {
        WARN
    } else if failed {
        DANGER
    } else {
        OK
    }
}

/// The `/shell` modal's frame.
pub fn shell_frame(running: bool, failed: bool) -> Style {
    modal_frame(shell_color(running, failed))
}

/// The `/shell` modal's title.
pub fn shell_title(running: bool, failed: bool) -> Style {
    modal_title(shell_color(running, failed))
}

/// A "Related:" link in the help modal.
pub fn link(selected: bool) -> Style {
    if selected {
        Style::new().foreground(INK).background(ACCENT)
    } else {
        Style::new().foreground(ACCENT).underline(true)
    }
}

/// A ratatui [`Block`] for a boxed style (border, border color and padding).
pub fn block(style: &Style) -> Block<'static> {
    style.to_block()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style as RStyle};

    #[test]
    fn styles_convert_to_the_expected_ratatui_styles() {
        let s: RStyle = (&brand()).into();
        assert_eq!((s.fg, s.bg), (Some(Color::Black), Some(Color::Cyan)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        let l: RStyle = (&link(false)).into();
        assert!(l.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(RStyle::from(&entry(EntryKind::Error)).fg, Some(Color::Red));
        assert_eq!(
            RStyle::from(&shell_title(false, false)).fg,
            Some(Color::Green)
        );
    }

    #[test]
    fn boxed_styles_build_blocks_with_borders_and_padding() {
        use ratatui::layout::Rect;
        // Chat body: border (1) + padding top 1, left/right 2.
        let inner = block(&chat_panel()).inner(Rect::new(0, 0, 40, 10));
        assert_eq!(inner, Rect::new(3, 2, 34, 7));
        let b = block(&panel(true));
        assert_eq!(b.inner(Rect::new(0, 0, 10, 5)), Rect::new(1, 1, 8, 3));
    }
}
