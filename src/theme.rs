//! The look of grokbot-tui. Every color comes from a theme file on disk
//! (`<data dir>/themes/<name>.toml`, parsed by [`crate::themes`]); nothing here is a usable
//! theme by itself.
//!
//! This module defines the theme *schema*: the named palette colors ([`COLOR_SLOTS`]), the
//! styled parts of the UI ([`STYLE_SLOTS`]), the in-memory [`Theme`], and the style
//! functions the drawing code calls (`theme::brand()`, `theme::panel(true)`, …). Layout
//! (border shape, padding) stays here in code, so a theme can't break the layout.
//!
//! Each style part also has a minimal *starter rule* ([`starter_style`]): which palette role
//! it uses (`entry_user` → `@ok`) or, for highlights, plain reverse video. A theme file that
//! leaves a part out gets its starter rule (resolved with that file's colors), `/theme init`
//! writes the starter rules with every color `reset`, and the same thing with all colors
//! `reset` is the unstyled fallback used when no theme file can be loaded.
//!
//! The active theme and variant are thread-local and set once per frame ([`set_theme`],
//! [`set_variant`]), so each test thread has its own.
use crate::app::EntryKind;
use ratatui::style::Color;
use ratatui::widgets::Block;
use ratatui_lipgloss::{Border, Style};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

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

// ---------------------------------------------------------------- schema: colors

/// The palette: named colors a theme defines once per variant and styles refer to as
/// `@name`. Order matters (it's the order in files and in the editor).
pub const COLOR_SLOTS: [&str; 14] = [
    "text",
    "background",
    "accent",
    "subtle",
    "muted",
    "header",
    "warn",
    "danger",
    "ok",
    "on_accent",
    "modal_text",
    "modal_background",
    "logo_dark",
    "logo_light",
];

/// One-line descriptions of [`COLOR_SLOTS`] (same order), shown in the editor.
pub const COLOR_HELP: [&str; 14] = [
    "Default text",
    "Screen background",
    "Brand, focus, selections, bot messages",
    "Unfocused borders, labels, notices",
    "De-emphasised text and key hints",
    "Header background",
    "Modal frames, account switcher, warnings",
    "Errors, failed shell commands",
    "Your messages, successful shell commands",
    "Text on accent / warning backgrounds",
    "Modal text",
    "Modal background",
    "Logo: dark parts",
    "Logo: light parts",
];

pub const N_COLORS: usize = COLOR_SLOTS.len();

/// Indexes into [`COLOR_SLOTS`].
pub mod color {
    pub const TEXT: usize = 0;
    pub const BACKGROUND: usize = 1;
    pub const ACCENT: usize = 2;
    pub const SUBTLE: usize = 3;
    pub const MUTED: usize = 4;
    pub const HEADER: usize = 5;
    pub const WARN: usize = 6;
    pub const DANGER: usize = 7;
    pub const OK: usize = 8;
    pub const ON_ACCENT: usize = 9;
    pub const MODAL_TEXT: usize = 10;
    pub const MODAL_BACKGROUND: usize = 11;
    pub const LOGO_DARK: usize = 12;
    pub const LOGO_LIGHT: usize = 13;
}

/// The index of a palette color by name.
pub fn color_slot(name: &str) -> Option<usize> {
    COLOR_SLOTS.iter().position(|c| *c == name)
}

// ---------------------------------------------------------------- schema: styles

/// A styled part of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotInfo {
    /// Key in the theme file's `[<variant>.styles]` table.
    pub key: &'static str,
    /// Group in the editor.
    pub area: &'static str,
    /// Drawn as a box: the `border` color applies.
    pub framed: bool,
    pub help: &'static str,
}

macro_rules! style_slots {
    ($( $id:ident = $key:literal, $area:literal, $framed:literal, $help:literal; )*) => {
        /// Indexes into [`STYLE_SLOTS`].
        pub mod slot {
            style_slots!(@consts 0usize; $($id)*);
        }
        /// Every styled part, in file / editor order.
        pub const STYLE_SLOTS: &[SlotInfo] = &[
            $( SlotInfo { key: $key, area: $area, framed: $framed, help: $help } ),*
        ];
    };
    (@consts $n:expr; $first:ident $($rest:ident)*) => {
        pub const $first: usize = $n;
        style_slots!(@consts $n + 1usize; $($rest)*);
    };
    (@consts $n:expr;) => {};
}

style_slots! {
    BASE = "base", "Screen and header", false, "Whole screen: default text and background";
    BRAND = "brand", "Screen and header", false, "The grokbot-tui name tag (compact header)";
    TOP_BAR = "top_bar", "Screen and header", false, "Header text and background";
    TITLE = "title", "Screen and header", false, "The big Grok Bot title";
    HEADER_LABEL = "header_label", "Screen and header", false, "Header labels (account, theme, …)";
    ACCOUNT_SWITCH = "account_switch", "Screen and header", false, "The @account ▾ switcher";
    THEME_SWITCH = "theme_switch", "Screen and header", false, "The ‹theme› ▾ switcher";
    MODE_ACTIVE = "mode_active", "Screen and header", false, "The selected dark/light/system choice";
    MODE_INACTIVE = "mode_inactive", "Screen and header", false, "The other dark/light/system choices";
    MODE_NORMAL = "mode_normal", "Screen and header", false, "Vim NORMAL mode indicator";
    MODE_INSERT = "mode_insert", "Screen and header", false, "Vim INSERT mode indicator";
    MODE_COMMAND = "mode_command", "Screen and header", false, "Vim COMMAND (:) mode indicator";
    MODE_VISUAL = "mode_visual", "Screen and header", false, "Vim VISUAL mode indicator";
    KEY_HINT = "key_hint", "Screen and header", false, "Key hints (F3, F4) and notes";
    MOUSE_ON = "mouse_on", "Screen and header", false, "'mouse: on'";
    MOUSE_OFF = "mouse_off", "Screen and header", false, "'mouse: OFF' (stands out)";
    MOCK_TAG = "mock_tag", "Screen and header", false, "[MOCK] tag";
    SELECTED = "selected", "Sidebar and lists", false, "Active bot / picked list row";
    CURSOR_ROW = "cursor_row", "Sidebar and lists", false, "Sidebar cursor while the sidebar has focus";
    SELECTION = "selection", "Chat", false, "In-app selected text in the chat body";
    UNREAD_BADGE = "unread_badge", "Chat", false, "'↓ N new' while scrolled up";
    REACTION_PILL = "reaction_pill", "Chat", false, "Reaction pills under a message ( 👍 )";
    MD_HEADING = "md_heading", "Chat", false, "Markdown headings";
    MD_CODE = "md_code", "Chat", false, "Markdown `inline code`";
    MD_CODE_BLOCK = "md_code_block", "Chat", false, "Markdown fenced code blocks";
    MD_QUOTE = "md_quote", "Chat", false, "Markdown > blockquotes (bar and text)";
    MD_LINK = "md_link", "Chat", false, "Markdown link labels";
    MD_LIST_MARKER = "md_list_marker", "Chat", false, "Markdown list bullets, numbers and task boxes";
    MD_RULE = "md_rule", "Chat", false, "Markdown --- rules and link URLs";
    MD_TABLE_BORDER = "md_table_border", "Chat", false, "Markdown table borders";
    MENU_BAR = "menu_bar", "Screen and header", false, "Menu bar labels";
    MENU_ACTIVE = "menu_active", "Screen and header", false, "Open / focused menu label";
    TOOLTIP = "tooltip", "Modals", true, "Hover tooltip frame and text";
    TOAST = "toast", "Modals", true, "Notification toast (info/message)";
    TOAST_WARN = "toast_warn", "Modals", true, "Warning toast";
    TOAST_ERROR = "toast_error", "Modals", true, "Error toast";
    PANEL_FOCUSED = "panel_focused", "Sidebar and lists", true, "Focused box (sidebar, input)";
    PANEL_UNFOCUSED = "panel_unfocused", "Sidebar and lists", true, "Unfocused box";
    CHAT_PANEL = "chat_panel", "Chat", true, "The chat box";
    MUTED = "muted", "Chat", false, "De-emphasised text (empty state, notices rule)";
    ENTRY_SYSTEM = "entry_system", "Chat", false, "System notices";
    ENTRY_ERROR = "entry_error", "Chat", false, "Errors";
    ENTRY_USER = "entry_user", "Chat", false, "Your messages";
    ENTRY_BOT = "entry_bot", "Chat", false, "Bot messages";
    LABEL_SYSTEM = "entry_label_system", "Chat", false, "Prefix of system notices";
    LABEL_ERROR = "entry_label_error", "Chat", false, "Prefix of errors ('! ')";
    LABEL_USER = "entry_label_user", "Chat", false, "Prefix of your messages ('you: ')";
    LABEL_BOT = "entry_label_bot", "Chat", false, "Prefix of bot messages ('Helper: ')";
    MODAL_TEXT = "modal_text", "Modals", false, "Text inside help, pickers, shell, editor";
    HELP_FRAME = "help_frame", "Modals", true, "Help / picker / editor frame";
    HELP_TITLE = "help_title", "Modals", false, "Help / picker / editor title";
    SHELL_FRAME_RUNNING = "shell_frame_running", "Modals", true, "/shell frame while running";
    SHELL_FRAME_FAILED = "shell_frame_failed", "Modals", true, "/shell frame after a failure";
    SHELL_FRAME_OK = "shell_frame_ok", "Modals", true, "/shell frame after success";
    SHELL_TITLE_RUNNING = "shell_title_running", "Modals", false, "/shell title while running";
    SHELL_TITLE_FAILED = "shell_title_failed", "Modals", false, "/shell title after a failure";
    SHELL_TITLE_OK = "shell_title_ok", "Modals", false, "/shell title after success";
    LINK_SELECTED = "link_selected", "Modals", false, "Selected help link";
    LINK_UNSELECTED = "link_unselected", "Modals", false, "Other help links";
}

pub const N_STYLES: usize = STYLE_SLOTS.len();

/// The index of a style part by file key.
pub fn style_slot(key: &str) -> Option<usize> {
    STYLE_SLOTS.iter().position(|s| s.key == key)
}

/// A color in a style: a literal, or a palette color by name (`@accent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRef {
    Lit(Color),
    Slot(usize),
}

/// How one part is drawn. Attributes are only ever *added*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleSpec {
    pub fg: Option<ColorRef>,
    pub bg: Option<ColorRef>,
    /// Border color (framed parts only).
    pub border: Option<ColorRef>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    pub dim: bool,
}

/// The attribute names, in file / editor order.
pub const ATTRS: [&str; 5] = ["bold", "italic", "underline", "reverse", "dim"];

impl StyleSpec {
    pub fn attr(&self, i: usize) -> bool {
        [
            self.bold,
            self.italic,
            self.underline,
            self.reverse,
            self.dim,
        ][i]
    }

    pub fn set_attr(&mut self, i: usize, on: bool) {
        match i {
            0 => self.bold = on,
            1 => self.italic = on,
            2 => self.underline = on,
            3 => self.reverse = on,
            _ => self.dim = on,
        }
    }
}

/// A style with every palette reference looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Resolved {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub border: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
    pub dim: bool,
}

/// The minimal rule for a part when a theme doesn't style it: the palette role it uses,
/// or reverse video for highlights (so they stay visible with every color `reset`).
/// This is not a theme: with all colors `reset` it draws plain terminal colors.
pub fn starter_style(slot_index: usize) -> StyleSpec {
    use color as c;
    use slot as s;
    let r = ColorRef::Slot;
    let fg = |x| StyleSpec {
        fg: Some(r(x)),
        ..StyleSpec::default()
    };
    let border = |x| StyleSpec {
        border: Some(r(x)),
        ..StyleSpec::default()
    };
    let bold = |mut st: StyleSpec| {
        st.bold = true;
        st
    };
    let reverse = StyleSpec {
        reverse: true,
        ..StyleSpec::default()
    };
    match slot_index {
        s::BASE => StyleSpec {
            bg: Some(r(c::BACKGROUND)),
            ..fg(c::TEXT)
        },
        s::TOP_BAR => StyleSpec {
            bg: Some(r(c::HEADER)),
            ..fg(c::TEXT)
        },
        s::BRAND
        | s::MODE_ACTIVE
        | s::MODE_NORMAL
        | s::MODE_INSERT
        | s::SELECTED
        | s::SELECTION => bold(reverse),
        s::MODE_COMMAND | s::MODE_VISUAL => bold(StyleSpec {
            bg: Some(r(c::WARN)),
            fg: Some(r(c::ON_ACCENT)),
            ..StyleSpec::default()
        }),
        s::CURSOR_ROW | s::MOUSE_OFF | s::LINK_SELECTED => reverse,
        s::UNREAD_BADGE => bold(StyleSpec {
            bg: Some(r(c::WARN)),
            fg: Some(r(c::ON_ACCENT)),
            ..StyleSpec::default()
        }),
        s::REACTION_PILL => StyleSpec {
            bg: Some(r(c::HEADER)),
            fg: Some(r(c::TEXT)),
            ..StyleSpec::default()
        },
        s::MD_HEADING => bold(fg(c::ACCENT)),
        s::MD_CODE => fg(c::WARN),
        s::MD_CODE_BLOCK => StyleSpec {
            bg: Some(r(c::HEADER)),
            ..fg(c::TEXT)
        },
        s::MD_QUOTE => StyleSpec {
            italic: true,
            ..fg(c::SUBTLE)
        },
        s::MD_LINK => StyleSpec {
            underline: true,
            ..fg(c::ACCENT)
        },
        s::MD_LIST_MARKER => fg(c::ACCENT),
        s::MD_RULE => fg(c::MUTED),
        s::MD_TABLE_BORDER => fg(c::MUTED),
        s::MENU_BAR => fg(c::TEXT),
        s::MENU_ACTIVE => bold(StyleSpec {
            bg: Some(r(c::ACCENT)),
            fg: Some(r(c::ON_ACCENT)),
            ..StyleSpec::default()
        }),
        s::TOOLTIP => StyleSpec {
            fg: Some(r(c::MODAL_TEXT)),
            bg: Some(r(c::MODAL_BACKGROUND)),
            border: Some(r(c::ACCENT)),
            ..StyleSpec::default()
        },
        s::TOAST => StyleSpec {
            fg: Some(r(c::MODAL_TEXT)),
            bg: Some(r(c::MODAL_BACKGROUND)),
            border: Some(r(c::ACCENT)),
            ..StyleSpec::default()
        },
        s::TOAST_WARN => StyleSpec {
            fg: Some(r(c::ON_ACCENT)),
            bg: Some(r(c::WARN)),
            border: Some(r(c::WARN)),
            ..StyleSpec::default()
        },
        s::TOAST_ERROR => StyleSpec {
            fg: Some(r(c::ON_ACCENT)),
            bg: Some(r(c::DANGER)),
            border: Some(r(c::DANGER)),
            ..StyleSpec::default()
        },
        // Not bold: some terminals draw bold block glyphs from another font.
        s::TITLE => fg(c::ACCENT),
        s::HEADER_LABEL | s::MODE_INACTIVE | s::MOUSE_ON | s::ENTRY_SYSTEM => fg(c::SUBTLE),
        s::ACCOUNT_SWITCH | s::THEME_SWITCH => StyleSpec {
            bold: true,
            underline: true,
            ..StyleSpec::default()
        },
        s::KEY_HINT | s::MUTED => fg(c::MUTED),
        s::MOCK_TAG => fg(c::WARN),
        s::PANEL_FOCUSED => border(c::ACCENT),
        s::PANEL_UNFOCUSED => border(c::SUBTLE),
        s::ENTRY_ERROR => fg(c::DANGER),
        s::ENTRY_USER => fg(c::OK),
        s::ENTRY_BOT => fg(c::ACCENT),
        s::LABEL_SYSTEM => bold(fg(c::SUBTLE)),
        s::LABEL_ERROR => bold(fg(c::DANGER)),
        s::LABEL_USER => bold(fg(c::OK)),
        s::LABEL_BOT => bold(fg(c::ACCENT)),
        s::MODAL_TEXT => StyleSpec {
            bg: Some(r(c::MODAL_BACKGROUND)),
            ..fg(c::MODAL_TEXT)
        },
        s::HELP_FRAME | s::SHELL_FRAME_RUNNING => border(c::WARN),
        s::SHELL_FRAME_FAILED => border(c::DANGER),
        s::SHELL_FRAME_OK => border(c::OK),
        s::HELP_TITLE | s::SHELL_TITLE_RUNNING => bold(fg(c::WARN)),
        s::SHELL_TITLE_FAILED => bold(fg(c::DANGER)),
        s::SHELL_TITLE_OK => bold(fg(c::OK)),
        s::LINK_UNSELECTED => StyleSpec {
            underline: true,
            ..fg(c::ACCENT)
        },
        // CHAT_PANEL and anything new: plain.
        _ => StyleSpec::default(),
    }
}

// ---------------------------------------------------------------- themes

/// One variant (dark or light) of a theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantSpec {
    pub colors: [Color; N_COLORS],
    /// One per [`STYLE_SLOTS`] entry.
    pub styles: Vec<StyleSpec>,
}

impl VariantSpec {
    /// Starter rules with every color `reset` (terminal default).
    pub fn starter() -> Self {
        Self {
            colors: [Color::Reset; N_COLORS],
            styles: (0..N_STYLES).map(starter_style).collect(),
        }
    }

    pub fn color(&self, r: ColorRef) -> Color {
        match r {
            ColorRef::Lit(c) => c,
            ColorRef::Slot(i) => self.colors.get(i).copied().unwrap_or(Color::Reset),
        }
    }

    pub fn resolve(&self, slot_index: usize) -> Resolved {
        let s = self.styles.get(slot_index).copied().unwrap_or_default();
        Resolved {
            fg: s.fg.map(|c| self.color(c)),
            bg: s.bg.map(|c| self.color(c)),
            border: s.border.map(|c| self.color(c)),
            bold: s.bold,
            italic: s.italic,
            underline: s.underline,
            reverse: s.reverse,
            dim: s.dim,
        }
    }

    /// The Lip Gloss style for a part: the theme's colors and attributes plus the layout
    /// (border shape, padding) from code.
    pub fn style(&self, slot_index: usize) -> Style {
        let r = self.resolve(slot_index);
        let mut st = Style::new();
        if let Some(c) = r.fg {
            st = st.foreground(c);
        }
        if let Some(c) = r.bg {
            st = st.background(c);
        }
        if r.bold {
            st = st.bold(true);
        }
        if r.italic {
            st = st.italic(true);
        }
        if r.underline {
            st = st.underline(true);
        }
        if r.reverse {
            st = st.reverse(true);
        }
        if r.dim {
            st = st.faint(true);
        }
        if STYLE_SLOTS.get(slot_index).is_some_and(|s| s.framed) {
            st = st.border(Border::normal());
            if let Some(c) = r.border {
                st = st.border_foreground(c);
            }
        }
        if slot_index == slot::CHAT_PANEL {
            st = st.padding((1, 2, 0, 2));
        }
        st
    }
}

/// A theme: a dark and a light variant, loaded from `<data dir>/themes/<id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// File name without `.toml`.
    pub id: String,
    pub name: String,
    pub description: String,
    pub dark: VariantSpec,
    pub light: VariantSpec,
}

impl Theme {
    /// The starter rules with every color `reset`: what `/theme init` writes, and the
    /// unstyled fallback when no theme file can be loaded.
    pub fn unstyled(id: &str) -> Self {
        Self {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            dark: VariantSpec::starter(),
            light: VariantSpec::starter(),
        }
    }

    pub fn variant(&self, v: Variant) -> &VariantSpec {
        match v {
            Variant::Dark => &self.dark,
            Variant::Light => &self.light,
        }
    }

    pub fn variant_mut(&mut self, v: Variant) -> &mut VariantSpec {
        match v {
            Variant::Dark => &mut self.dark,
            Variant::Light => &mut self.light,
        }
    }
}

thread_local! {
    static ACTIVE: RefCell<Arc<Theme>> = RefCell::new(Arc::new(Theme::unstyled("unstyled")));
}

/// Draw with `t` on this thread from now on (called once per frame).
pub fn set_theme(t: Arc<Theme>) {
    ACTIVE.with(|a| *a.borrow_mut() = t);
}

/// The theme styles are currently built from.
pub fn active() -> Arc<Theme> {
    ACTIVE.with(|a| a.borrow().clone())
}

fn st(slot_index: usize) -> Style {
    ACTIVE.with(|a| a.borrow().variant(variant()).style(slot_index))
}

fn color_of(i: usize) -> Color {
    ACTIVE.with(|a| a.borrow().variant(variant()).colors[i])
}

// ---------------------------------------------------------------- style functions

/// The whole screen: default text color on the variant's background.
pub fn base() -> Style {
    st(slot::BASE)
}

/// The "grokbot-tui" tag in the compact header.
pub fn brand() -> Style {
    st(slot::BRAND)
}

/// The header's background and default text.
pub fn top_bar() -> Style {
    st(slot::TOP_BAR)
}

/// The big "Grok Bot" title.
pub fn title() -> Style {
    st(slot::TITLE)
}

/// Labels in the header ("account", "theme", "login", "bot").
pub fn header_label() -> Style {
    st(slot::HEADER_LABEL)
}

/// The account switcher (`@alias ▾`): stands out and looks clickable.
pub fn account_switch() -> Style {
    st(slot::ACCOUNT_SWITCH)
}

/// The theme switcher (`‹name› ▾`).
pub fn theme_switch() -> Style {
    st(slot::THEME_SWITCH)
}

/// A dark / light / system choice in the header; `active` is the current mode.
pub fn mode_choice(active: bool) -> Style {
    st(if active {
        slot::MODE_ACTIVE
    } else {
        slot::MODE_INACTIVE
    })
}

/// Key hints ("F3", "F4") and other secondary header text.
pub fn key_hint() -> Style {
    st(slot::KEY_HINT)
}

/// "mouse: on (F6)".
pub fn mouse_on() -> Style {
    st(slot::MOUSE_ON)
}

/// "mouse: OFF …": stands out, since clicks and the wheel don't work in this state.
pub fn mouse_off() -> Style {
    st(slot::MOUSE_OFF)
}

/// "[MOCK]".
pub fn mock_tag() -> Style {
    st(slot::MOCK_TAG)
}

/// The logo's dark and light pixel colors.
pub fn logo_colors() -> (Color, Color) {
    (color_of(color::LOGO_DARK), color_of(color::LOGO_LIGHT))
}

/// The active bot in the sidebar and the selected row of a pick list.
pub fn vim_mode_style(mode: crate::keymap::Mode) -> Style {
    use crate::keymap::Mode;
    st(match mode {
        Mode::Normal => slot::MODE_NORMAL,
        Mode::Insert => slot::MODE_INSERT,
        Mode::Command => slot::MODE_COMMAND,
        Mode::Visual => slot::MODE_VISUAL,
    })
}

pub fn menu_bar() -> Style {
    st(slot::MENU_BAR)
}
pub fn menu_active() -> Style {
    st(slot::MENU_ACTIVE)
}
pub fn tooltip() -> Style {
    st(slot::TOOLTIP)
}
pub fn toast(kind: crate::notify::NoticeKind) -> Style {
    use crate::notify::NoticeKind;
    st(match kind {
        NoticeKind::Error => slot::TOAST_ERROR,
        NoticeKind::Warning => slot::TOAST_WARN,
        NoticeKind::Info | NoticeKind::Message => slot::TOAST,
    })
}
pub fn unread_badge() -> Style {
    st(slot::UNREAD_BADGE)
}
pub fn md_heading() -> Style {
    st(slot::MD_HEADING)
}
pub fn md_code() -> Style {
    st(slot::MD_CODE)
}
pub fn md_code_block() -> Style {
    st(slot::MD_CODE_BLOCK)
}
pub fn md_quote() -> Style {
    st(slot::MD_QUOTE)
}
pub fn md_link() -> Style {
    st(slot::MD_LINK)
}
pub fn md_list_marker() -> Style {
    st(slot::MD_LIST_MARKER)
}
pub fn md_rule() -> Style {
    st(slot::MD_RULE)
}
pub fn md_table_border() -> Style {
    st(slot::MD_TABLE_BORDER)
}
pub fn reaction_pill() -> Style {
    st(slot::REACTION_PILL)
}

pub fn selected() -> Style {
    st(slot::SELECTED)
}

/// The sidebar cursor while the sidebar has focus.
pub fn cursor_row() -> Style {
    st(slot::CURSOR_ROW)
}

/// A bordered panel (sidebar, input box), accented when focused.
pub fn panel(focused: bool) -> Style {
    st(if focused {
        slot::PANEL_FOCUSED
    } else {
        slot::PANEL_UNFOCUSED
    })
}

/// The chat body: bordered, with left/right padding 2 and top padding 1.
pub fn chat_panel() -> Style {
    st(slot::CHAT_PANEL)
}

/// De-emphasised text (empty state, the "notices" rule).
pub fn muted() -> Style {
    st(slot::MUTED)
}

/// Message text by kind.
pub fn entry(kind: EntryKind) -> Style {
    st(match kind {
        EntryKind::System => slot::ENTRY_SYSTEM,
        EntryKind::Error => slot::ENTRY_ERROR,
        EntryKind::User => slot::ENTRY_USER,
        EntryKind::Bot => slot::ENTRY_BOT,
    })
}

/// A message's "you: " / "Helper: " prefix.
pub fn entry_label(kind: EntryKind) -> Style {
    st(match kind {
        EntryKind::System => slot::LABEL_SYSTEM,
        EntryKind::Error => slot::LABEL_ERROR,
        EntryKind::User => slot::LABEL_USER,
        EntryKind::Bot => slot::LABEL_BOT,
    })
}

/// Text and background inside the modals (help, pick lists, `/shell`, theme editor).
pub fn modal_text() -> Style {
    st(slot::MODAL_TEXT)
}

/// The help, picker and theme editor frame.
pub fn help_frame() -> Style {
    st(slot::HELP_FRAME)
}

/// The help, picker and theme editor title.
pub fn help_title() -> Style {
    st(slot::HELP_TITLE)
}

/// The `/shell` modal's frame: running, failed or ok.
pub fn shell_frame(running: bool, failed: bool) -> Style {
    st(if running {
        slot::SHELL_FRAME_RUNNING
    } else if failed {
        slot::SHELL_FRAME_FAILED
    } else {
        slot::SHELL_FRAME_OK
    })
}

/// The `/shell` modal's title: running, failed or ok.
pub fn shell_title(running: bool, failed: bool) -> Style {
    st(if running {
        slot::SHELL_TITLE_RUNNING
    } else if failed {
        slot::SHELL_TITLE_FAILED
    } else {
        slot::SHELL_TITLE_OK
    })
}

/// A "Related:" link in the help modal.
pub fn link(selected: bool) -> Style {
    st(if selected {
        slot::LINK_SELECTED
    } else {
        slot::LINK_UNSELECTED
    })
}

/// A ratatui [`Block`] for a boxed style (border, border color and padding).
pub fn block(style: &Style) -> Block<'static> {
    style.to_block()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style as RStyle};

    /// The repo's shipped default theme.
    pub fn repo_default() -> Theme {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/default.toml"),
        )
        .unwrap();
        let (t, w) = crate::themes::parse("default", &text).unwrap();
        assert!(w.is_empty(), "default.toml has warnings: {w:?}");
        t
    }

    /// 1.4.0's hard-coded palettes and style recipes, kept only here as the reference the
    /// shipped default.toml must reproduce.
    fn golden(v: Variant, key: &str) -> Resolved {
        use Color::*;
        // text, background, accent, subtle, muted, header, warn, danger, ok, on_accent,
        // modal_text, modal_background
        let p = match v {
            Variant::Dark => [
                Indexed(252),
                Indexed(234),
                Cyan,
                Gray,
                DarkGray,
                Indexed(237),
                Yellow,
                Red,
                Green,
                Black,
                White,
                Black,
            ],
            Variant::Light => [
                Indexed(235),
                Indexed(255),
                Indexed(25),
                Indexed(242),
                Indexed(245),
                Indexed(253),
                Indexed(130),
                Indexed(160),
                Indexed(28),
                White,
                Indexed(235),
                Indexed(254),
            ],
        };
        let [
            text,
            background,
            accent,
            subtle,
            muted,
            header,
            warn,
            danger,
            ok,
            on_accent,
            mt,
            mb,
        ] = p;
        let r = Resolved::default();
        let fg = |c| Resolved { fg: Some(c), ..r };
        let fgbg = |f, b| Resolved {
            fg: Some(f),
            bg: Some(b),
            ..r
        };
        let bold = |mut x: Resolved| {
            x.bold = true;
            x
        };
        let border = |c| Resolved {
            border: Some(c),
            ..r
        };
        match key {
            "base" => fgbg(text, background),
            "brand" | "mode_active" | "selected" | "selection" => bold(fgbg(on_accent, accent)),
            "top_bar" => fgbg(text, header),
            "title" => fg(accent),
            "header_label" | "mode_inactive" | "mouse_on" | "entry_system" => fg(subtle),
            "account_switch" => Resolved {
                underline: true,
                ..bold(fg(warn))
            },
            "key_hint" | "muted" => fg(muted),
            "mouse_off" => fgbg(on_accent, warn),
            "mock_tag" => fg(warn),
            "cursor_row" => Resolved { reverse: true, ..r },
            "panel_focused" => border(accent),
            "panel_unfocused" => border(subtle),
            "chat_panel" => r,
            "entry_error" => fg(danger),
            "entry_user" => fg(ok),
            "entry_bot" => fg(accent),
            "entry_label_system" => bold(fg(subtle)),
            "entry_label_error" => bold(fg(danger)),
            "entry_label_user" => bold(fg(ok)),
            "entry_label_bot" => bold(fg(accent)),
            "modal_text" => fgbg(mt, mb),
            "help_frame" | "shell_frame_running" => border(warn),
            "shell_frame_failed" => border(danger),
            "shell_frame_ok" => border(ok),
            "help_title" | "shell_title_running" => bold(fg(warn)),
            "shell_title_failed" => bold(fg(danger)),
            "shell_title_ok" => bold(fg(ok)),
            "link_selected" => fgbg(on_accent, accent),
            "link_unselected" => Resolved {
                underline: true,
                ..fg(accent)
            },
            other => panic!("no 1.4.0 reference for {other}"),
        }
    }

    #[test]
    fn shipped_default_theme_reproduces_the_1_4_look_exactly() {
        let t = repo_default();
        for v in [Variant::Dark, Variant::Light] {
            let spec = t.variant(v);
            for (i, s) in STYLE_SLOTS.iter().enumerate() {
                if s.key == "theme_switch"
                    || s.key == "unread_badge"
                    || s.key == "reaction_pill"
                    || s.key.starts_with("md_")
                    || s.key == "menu_bar"
                    || s.key == "menu_active"
                    || s.key == "tooltip"
                    || s.key.starts_with("toast")
                    || s.key.starts_with("mode_n")
                    || s.key.starts_with("mode_i")
                    || s.key.starts_with("mode_c")
                    || s.key.starts_with("mode_v")
                {
                    continue; // theme_switch 1.5; vim mode_* / unread_badge 1.9
                }
                assert_eq!(spec.resolve(i), golden(v, s.key), "{v:?} {}", s.key);
            }
            // 1.3.2 logo colors, the same in both variants.
            assert_eq!(spec.colors[color::LOGO_DARK], Color::Indexed(232));
            assert_eq!(spec.colors[color::LOGO_LIGHT], Color::Indexed(255));
        }
        // And through the public style functions.
        set_theme(Arc::new(t));
        set_variant(Variant::Dark);
        let s: RStyle = (&brand()).into();
        assert_eq!((s.fg, s.bg), (Some(Color::Black), Some(Color::Cyan)));
        assert!(s.add_modifier.contains(Modifier::BOLD));
        assert!(!RStyle::from(&title()).add_modifier.contains(Modifier::BOLD));
        assert_eq!(logo_colors(), (Color::Indexed(232), Color::Indexed(255)));
        assert_eq!(RStyle::from(&entry(EntryKind::Error)).fg, Some(Color::Red));
        set_theme(Arc::new(Theme::unstyled("x")));
    }

    #[test]
    fn slot_constants_match_their_keys() {
        assert_eq!(STYLE_SLOTS[slot::BASE].key, "base");
        assert_eq!(STYLE_SLOTS[slot::THEME_SWITCH].key, "theme_switch");
        assert_eq!(STYLE_SLOTS[slot::LINK_UNSELECTED].key, "link_unselected");
        assert_eq!(STYLE_SLOTS[slot::SELECTION].key, "selection");
        assert_eq!(slot::LINK_UNSELECTED, N_STYLES - 1);
        assert_eq!(COLOR_SLOTS[color::LOGO_LIGHT], "logo_light");
        let mut keys: Vec<&str> = STYLE_SLOTS.iter().map(|s| s.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), N_STYLES, "style keys are unique");
    }

    #[test]
    fn unstyled_fallback_uses_terminal_colors_and_keeps_highlights_visible() {
        let t = Theme::unstyled("none");
        assert!(t.dark.colors.iter().all(|c| *c == Color::Reset));
        set_theme(Arc::new(t));
        set_variant(Variant::Dark);
        let b: RStyle = (&base()).into();
        assert_eq!((b.fg, b.bg), (Some(Color::Reset), Some(Color::Reset)));
        // Selections use reverse video, so they show without any colors.
        assert!(
            RStyle::from(&selected())
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            RStyle::from(&mode_choice(true))
                .add_modifier
                .contains(Modifier::REVERSED)
        );
        assert_eq!(logo_colors(), (Color::Reset, Color::Reset));
    }

    #[test]
    fn active_theme_and_variant_drive_the_style_functions() {
        set_theme(Arc::new(repo_default()));
        set_variant(Variant::Dark);
        let dark: RStyle = (&top_bar()).into();
        let dark_sel: RStyle = (&selected()).into();
        set_variant(Variant::Light);
        assert_eq!(variant(), Variant::Light);
        let light: RStyle = (&top_bar()).into();
        assert_ne!(dark.bg, light.bg);
        assert_ne!(dark_sel, RStyle::from(&selected()));
        set_variant(Variant::Dark);
        assert_eq!(RStyle::from(&top_bar()), dark);
        let mut t = repo_default();
        t.dark.colors[color::HEADER] = Color::Indexed(17);
        set_theme(Arc::new(t));
        assert_eq!(RStyle::from(&top_bar()).bg, Some(Color::Indexed(17)));
        set_theme(Arc::new(Theme::unstyled("x")));
    }

    #[test]
    fn boxed_styles_build_blocks_with_borders_and_padding() {
        use ratatui::layout::Rect;
        set_theme(Arc::new(repo_default()));
        let inner = block(&chat_panel()).inner(Rect::new(0, 0, 40, 10));
        assert_eq!(inner, Rect::new(3, 2, 34, 7));
        let b = block(&panel(true));
        assert_eq!(b.inner(Rect::new(0, 0, 10, 5)), Rect::new(1, 1, 8, 3));
        // Layout doesn't depend on the theme: same boxes with no theme.
        set_theme(Arc::new(Theme::unstyled("x")));
        assert_eq!(
            block(&chat_panel()).inner(Rect::new(0, 0, 40, 10)),
            Rect::new(3, 2, 34, 7)
        );
    }

    #[test]
    fn mode_resolves_system_to_detected_else_dark() {
        assert_eq!(Mode::default(), Mode::System);
        for d in [None, Some(Variant::Dark), Some(Variant::Light)] {
            assert_eq!(Mode::Dark.resolve(d), Variant::Dark);
            assert_eq!(Mode::Light.resolve(d), Variant::Light);
        }
        assert_eq!(Mode::System.resolve(Some(Variant::Light)), Variant::Light);
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
}
