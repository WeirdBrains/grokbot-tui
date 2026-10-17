//! The look of grokbot-tui: every color, text attribute, border and padding, defined once
//! with [`ratatui_lipgloss`] styles, in a dark and a light variant.
//!
//! Drawing code in `ui.rs` only lays things out. To restyle the app, change this file.
//! Lip Gloss styles convert into ratatui styles (`&Style` → `ratatui::style::Style`),
//! build ratatui blocks with [`Style::to_block`], and make spans with [`Style::span`].
//!
//! The variant in use is set once per frame with [`set_variant`] (see [`Mode`] for how the
//! user's dark / light / system choice resolves to a variant). It is thread-local, so each
//! test thread has its own.
use crate::app::EntryKind;
use ratatui::style::Color;
use ratatui::widgets::Block;
use ratatui_lipgloss::{Border, Style};
use std::cell::Cell;

// ---------------------------------------------------------------- variants and modes

/// A concrete color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

/// The user's appearance choice (pref `ui.theme_mode`, F4, `/theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    Dark,
    Light,
    /// Follow the terminal's background (detected at startup), else dark.
    #[default]
    System,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Dark, Mode::Light, Mode::System];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Dark => "dark",
            Mode::Light => "light",
            Mode::System => "system",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(Mode::Dark),
            "light" => Some(Mode::Light),
            "system" | "auto" => Some(Mode::System),
            _ => None,
        }
    }

    /// dark → light → system → dark (F4).
    pub fn next(self) -> Mode {
        match self {
            Mode::Dark => Mode::Light,
            Mode::Light => Mode::System,
            Mode::System => Mode::Dark,
        }
    }

    /// The variant to draw with. `System` uses the detected terminal background and falls
    /// back to dark when it couldn't be detected.
    pub fn resolve(self, detected: Option<Variant>) -> Variant {
        match self {
            Mode::Dark => Variant::Dark,
            Mode::Light => Variant::Light,
            Mode::System => detected.unwrap_or(Variant::Dark),
        }
    }
}

thread_local! {
    static VARIANT: Cell<Variant> = const { Cell::new(Variant::Dark) };
}

/// Use `v` for every style built on this thread from now on (called once per frame).
pub fn set_variant(v: Variant) {
    VARIANT.with(|c| c.set(v));
}

/// The variant styles are currently built with.
pub fn variant() -> Variant {
    VARIANT.with(Cell::get)
}

// ---------------------------------------------------------------- palette

/// The named colors every style is built from.
struct Palette {
    /// Default text and background of the whole screen.
    text: Color,
    background: Color,
    /// Accent: the brand tag, focused borders, selections and bot messages.
    accent: Color,
    /// Unfocused borders and system notices.
    subtle: Color,
    /// De-emphasised text.
    muted: Color,
    /// The header's background.
    header: Color,
    /// Modal frames, the account switcher and warnings.
    warn: Color,
    /// Errors and failed shell commands.
    danger: Color,
    /// Your own messages and successful shell commands.
    ok: Color,
    /// Text on accent / warning backgrounds.
    on_accent: Color,
    /// Modal text and background.
    modal_text: Color,
    modal_background: Color,
}

/// The original grokbot-tui colors.
const DARK: Palette = Palette {
    text: Color::Indexed(252),
    background: Color::Indexed(234),
    accent: Color::Cyan,
    subtle: Color::Gray,
    muted: Color::DarkGray,
    header: Color::Indexed(237),
    warn: Color::Yellow,
    danger: Color::Red,
    ok: Color::Green,
    on_accent: Color::Black,
    modal_text: Color::White,
    modal_background: Color::Black,
};

/// Dark text on a near-white background, with deeper accents that stay readable on it.
const LIGHT: Palette = Palette {
    text: Color::Indexed(235),
    background: Color::Indexed(255),
    accent: Color::Indexed(25),
    subtle: Color::Indexed(242),
    muted: Color::Indexed(245),
    header: Color::Indexed(253),
    warn: Color::Indexed(130),
    danger: Color::Indexed(160),
    ok: Color::Indexed(28),
    on_accent: Color::White,
    modal_text: Color::Indexed(235),
    modal_background: Color::Indexed(254),
};

fn pal() -> &'static Palette {
    match variant() {
        Variant::Dark => &DARK,
        Variant::Light => &LIGHT,
    }
}

// ---------------------------------------------------------------- screen and header

/// The whole screen: default text color on the variant's background.
pub fn base() -> Style {
    Style::new()
        .foreground(pal().text)
        .background(pal().background)
}

/// The " grokbot-tui " tag in the compact header.
pub fn brand() -> Style {
    Style::new()
        .foreground(pal().on_accent)
        .background(pal().accent)
        .bold(true)
}

/// The header's background (tall and compact).
pub fn top_bar() -> Style {
    Style::new().foreground(pal().text).background(pal().header)
}

/// The big "Grok Bot" title in the tall header.
pub fn title() -> Style {
    // Not bold: some terminals draw bold block glyphs from a different font, which breaks
    // the pixel art.
    Style::new().foreground(pal().accent)
}

/// Labels in the header ("account", "theme", "login:").
pub fn header_label() -> Style {
    Style::new().foreground(pal().subtle)
}

/// The clickable account switcher ("@alias ▾") in the header.
pub fn account_switch() -> Style {
    Style::new()
        .foreground(pal().warn)
        .bold(true)
        .underline(true)
}

/// A dark / light / system choice in the header: the active one is highlighted.
pub fn mode_choice(active: bool) -> Style {
    if active {
        Style::new()
            .foreground(pal().on_accent)
            .background(pal().accent)
            .bold(true)
    } else {
        Style::new().foreground(pal().subtle)
    }
}

/// Key hints in the header ("F3", "F4").
pub fn key_hint() -> Style {
    Style::new().foreground(pal().muted)
}

/// "mouse: on" in the header.
pub fn mouse_on() -> Style {
    Style::new().foreground(pal().subtle)
}

/// "mouse: OFF" in the header: a warning, since clicks don't work while it's off.
pub fn mouse_off() -> Style {
    Style::new()
        .foreground(pal().on_accent)
        .background(pal().warn)
}

/// The "[MOCK]" tag.
pub fn mock_tag() -> Style {
    Style::new().foreground(pal().warn)
}

/// The two colors of the Grok Bot logo (its dark squircle and white ball). They are the
/// logo's own colors, the same in both variants.
pub fn logo_colors() -> (Color, Color) {
    (Color::Indexed(232), Color::Indexed(255))
}

// ---------------------------------------------------------------- sidebar

/// A highlighted row: the active bot, the picker's selection, the selected help link.
pub fn selected() -> Style {
    Style::new()
        .foreground(pal().on_accent)
        .background(pal().accent)
        .bold(true)
}

/// The sidebar keyboard cursor (when it isn't on the active bot).
pub fn cursor_row() -> Style {
    Style::new().reverse(true)
}

/// A bordered panel (sidebar, input bar); the border is accented when focused.
pub fn panel(focused: bool) -> Style {
    Style::new()
        .border(Border::normal())
        .border_foreground(if focused { pal().accent } else { pal().subtle })
}

// ---------------------------------------------------------------- chat body

/// The chat body: plain border, 1 row of padding at the top and 2 columns left and right.
pub fn chat_panel() -> Style {
    Style::new().border(Border::normal()).padding((1, 2, 0, 2))
}

/// De-emphasised text: the empty-state guidance, the notices divider, picker details.
pub fn muted() -> Style {
    Style::new().foreground(pal().muted)
}

/// Text of a chat entry or notice.
pub fn entry(kind: EntryKind) -> Style {
    Style::new().foreground(match kind {
        EntryKind::System => pal().subtle,
        EntryKind::Error => pal().danger,
        EntryKind::User => pal().ok,
        EntryKind::Bot => pal().accent,
    })
}

/// The label of a chat entry ("you", "Helper: ", "! ").
pub fn entry_label(kind: EntryKind) -> Style {
    entry(kind).bold(true)
}

// ---------------------------------------------------------------- modals

/// Modal text (help, picker, shell).
pub fn modal_text() -> Style {
    Style::new()
        .foreground(pal().modal_text)
        .background(pal().modal_background)
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
    modal_frame(pal().warn)
}

/// The help and picker modals' title.
pub fn help_title() -> Style {
    modal_title(pal().warn)
}

/// The `/shell` modal's color: yellow while running, red on failure, green on success.
fn shell_color(running: bool, failed: bool) -> Color {
    if running {
        pal().warn
    } else if failed {
        pal().danger
    } else {
        pal().ok
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
        Style::new()
            .foreground(pal().on_accent)
            .background(pal().accent)
    } else {
        Style::new().foreground(pal().accent).underline(true)
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
    fn mode_resolves_system_to_detected_else_dark() {
        assert_eq!(Mode::default(), Mode::System);
        for d in [None, Some(Variant::Dark), Some(Variant::Light)] {
            assert_eq!(Mode::Dark.resolve(d), Variant::Dark);
            assert_eq!(Mode::Light.resolve(d), Variant::Light);
        }
        assert_eq!(Mode::System.resolve(Some(Variant::Light)), Variant::Light);
        assert_eq!(Mode::System.resolve(Some(Variant::Dark)), Variant::Dark);
        assert_eq!(Mode::System.resolve(None), Variant::Dark);
    }

    #[test]
    fn mode_parses_and_cycles() {
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
        assert_eq!(Mode::parse(" Light "), Some(Mode::Light));
        assert_eq!(Mode::parse("auto"), Some(Mode::System));
        assert_eq!(Mode::parse("blue"), None);
        assert_eq!(Mode::Dark.next(), Mode::Light);
        assert_eq!(Mode::Light.next(), Mode::System);
        assert_eq!(Mode::System.next(), Mode::Dark);
    }

    #[test]
    fn light_variant_changes_the_palette() {
        let dark: RStyle = (&top_bar()).into();
        let dark_sel: RStyle = (&selected()).into();
        set_variant(Variant::Light);
        assert_eq!(variant(), Variant::Light);
        let light: RStyle = (&top_bar()).into();
        let light_sel: RStyle = (&selected()).into();
        set_variant(Variant::Dark);
        assert_ne!(dark.bg, light.bg);
        assert_ne!(dark.fg, light.fg);
        assert_ne!(dark_sel, light_sel);
        let back: RStyle = (&top_bar()).into();
        assert_eq!(back, dark);
    }

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
