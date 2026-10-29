//! Typed view over the key-value preferences stored in the database.
use std::collections::BTreeMap;

/// Gateway token override for the account selected at startup.
pub const TOKEN_ENV: &str = "GROKBOT_TUI_TOKEN";
/// Optional gateway URL to pair with [`TOKEN_ENV`].
pub const GATEWAY_URL_ENV: &str = "GROKBOT_TUI_GATEWAY_URL";
/// Optional JSON object of extra gateway headers to pair with [`TOKEN_ENV`].
pub const GATEWAY_HEADERS_ENV: &str = "GROKBOT_TUI_GATEWAY_HEADERS";
/// Override the Cursor backend used for EnsureSandBox (must still be *.cursor.sh / *.cursor.com).
pub const CURSOR_API_ENV: &str = "GROKBOT_TUI_CURSOR_API_BASE_URL";
/// Overrides the cap on a single gateway response body, in bytes.
pub const MAX_RESPONSE_ENV: &str = "GROKBOT_TUI_MAX_RESPONSE_BYTES";
/// Default response cap. Accounts with many bots get `listAgents` replies over the
/// library's 2 MiB default, so the TUI allows much more.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// The response cap from `GROKBOT_TUI_MAX_RESPONSE_BYTES`, else the default.
pub fn max_response_bytes() -> usize {
    parse_max_response_bytes(std::env::var(MAX_RESPONSE_ENV).ok().as_deref())
}

pub(crate) fn parse_max_response_bytes(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
}
/// Override `x-cursor-client-version` for EnsureSandBox (highest priority).
pub const CLIENT_VERSION_ENV: &str = "GROKBOT_TUI_CURSOR_CLIENT_VERSION";
/// Override `x-cursor-client-type` for EnsureSandBox.
pub const CLIENT_TYPE_ENV: &str = "GROKBOT_TUI_CURSOR_CLIENT_TYPE";
/// Override `x-sand-box-namespace` for EnsureSandBox.
pub const SANDBOX_NAMESPACE_ENV: &str = "GROKBOT_TUI_SAND_BOX_NAMESPACE";
pub const DEFAULT_ACCOUNT: &str = "default";

/// Pref key for `x-cursor-client-version` (account-scoped).
pub const PREF_CLIENT_VERSION: &str = "cursor.client_version";
pub const PREF_CLIENT_TYPE: &str = "cursor.client_type";
pub const PREF_SANDBOX_NAMESPACE: &str = "sand.box_namespace";

/// Pref key for the appearance: `dark`, `light` or `system` (global).
pub const PREF_THEME_MODE: &str = "ui.theme_mode";

/// Pref key for the selected theme: a file name in `<data dir>/themes/` without `.toml` (global).
pub const PREF_THEME: &str = "ui.theme";

/// Pref key: may grokbot-tui download the shipped default theme from GitHub when the themes
/// folder is empty (`true`/`false`, global). `GROKBOT_TUI_NO_THEME_DOWNLOAD=1` also turns it off.
pub const PREF_THEME_DOWNLOAD: &str = "ui.theme_download";
/// Capture the mouse for clicks/wheel (`true`) or leave it to the terminal (`false`).
pub const PREF_MOUSE_CAPTURE: &str = "ui.mouse_capture";
/// Keybinding scheme: "default" or "vim".
pub const PREF_KEYS: &str = "ui.keys";

/// How many prompts to keep in the per-account prompt history (global).
pub const PREF_HISTORY_MAX: &str = "history.max_entries";
/// Default for [`PREF_HISTORY_MAX`].
pub const DEFAULT_HISTORY_MAX: usize = 10_000;
/// Accepted range for [`PREF_HISTORY_MAX`]; values outside it fall back to the default.
pub const HISTORY_MAX_RANGE: std::ops::RangeInclusive<usize> = 100..=1_000_000;

/// Download size cap in MB (global, `downloads.max_mb`).
pub const PREF_DOWNLOAD_MAX_MB: &str = "downloads.max_mb";
/// Default for [`PREF_DOWNLOAD_MAX_MB`].
pub const DEFAULT_DOWNLOAD_MAX_MB: u64 = 25;

/// Where links open: `builtin` (the in-app text browser) or `system` (global).
pub const PREF_LINKS_OPEN_WITH: &str = "links.open_with";
/// In an SSH session: `builtin` (always the built-in browser) or `respect_pref` (global).
pub const PREF_LINKS_SSH_BEHAVIOR: &str = "links.ssh_behavior";

/// Claim presence while grokbot-tui is focused and in use, so the server holds pushes to
/// your other devices (account-scoped, default off).
pub const PREF_CLAIM_PRESENCE: &str = "notifications.claim_presence";
/// Minutes without input before the presence claim is released (account-scoped).
pub const PREF_IDLE_MINUTES: &str = "notifications.idle_minutes";
/// Per-account install id sent with presence reports (generated once).
pub const PREF_INSTALL_ID: &str = "notifications.install_id";
pub const DEFAULT_IDLE_MINUTES: u64 = 10;

/// Esc-prefix window for Alt+key, ms (global, 0 disables, max 1000).
pub const PREF_ESC_ALT_MS: &str = "keys.esc_alt_timeout_ms";
/// Header: `full` (tall when there's room) or `compact` (one row) (global).
pub const PREF_HEADER: &str = "ui.header";
/// Sidebar collapsed to a 1-column strip (global).
pub const PREF_SIDEBAR_COLLAPSED: &str = "ui.sidebar_collapsed";
/// Clicking an empty part of a message bubble opens the reaction picker (global).
pub const PREF_CLICK_TO_REACT: &str = "message.click_to_react";

/// The prompt `/status` sends (account-scoped).
pub const PREF_STATUS_PROMPT: &str = "commands.status_prompt";
pub const DEFAULT_STATUS_PROMPT: &str =
    "Where are we now? Give me a brief status of your current tasks.";

/// Draw images inline in chat bubbles (global).
pub const PREF_IMAGES_INLINE: &str = "images.inline";
/// Height cap for inline images, in rows (global, 2..=60).
pub const PREF_IMAGES_INLINE_MAX_ROWS: &str = "images.inline_max_rows";

/// `links.open_with`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenWith {
    #[default]
    Builtin,
    System,
}

impl OpenWith {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "builtin" | "built-in" | "internal" => Some(Self::Builtin),
            "system" | "external" => Some(Self::System),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::System => "system",
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Builtin => Self::System,
            Self::System => Self::Builtin,
        }
    }
}

/// Pref key for the big header title's glyphs: `auto`, `quadrant` or `half` (global).
pub const PREF_TITLE_GLYPHS: &str = "ui.title_glyphs";

/// Preference keys shared by all accounts. Every other key is account-scoped.
pub const GLOBAL_KEYS: &[&str] = &[
    "active_account",
    "ui.sidebar_width",
    PREF_THEME_MODE,
    PREF_TITLE_GLYPHS,
    PREF_THEME,
    PREF_THEME_DOWNLOAD,
    PREF_MOUSE_CAPTURE,
    PREF_KEYS,
    PREF_HISTORY_MAX,
    PREF_DOWNLOAD_MAX_MB,
    PREF_LINKS_OPEN_WITH,
    PREF_LINKS_SSH_BEHAVIOR,
    PREF_ESC_ALT_MS,
    PREF_HEADER,
    PREF_SIDEBAR_COLLAPSED,
    PREF_CLICK_TO_REACT,
    PREF_IMAGES_INLINE,
    PREF_IMAGES_INLINE_MAX_ROWS,
];

/// Which block glyphs draw the big header title (pref `ui.title_glyphs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleGlyphs {
    /// Half blocks in macOS Terminal (its fonts mangle quadrant glyphs), quadrants elsewhere.
    #[default]
    Auto,
    /// Quadrant glyphs (`▖ ▗ ▘ ▝ …`): 4 columns per letter.
    Quadrant,
    /// Half-height blocks (only `▀ ▄ █`): 8 columns per letter, clean in every font.
    Half,
}

impl TitleGlyphs {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Quadrant => "quadrant",
            Self::Half => "half",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "quadrant" | "quad" | "quarter" => Some(Self::Quadrant),
            "half" | "half-height" => Some(Self::Half),
            _ => None,
        }
    }

    /// The concrete style: `Auto` checks `$TERM_PROGRAM`.
    pub fn resolve(self) -> Self {
        self.resolve_for(std::env::var("TERM_PROGRAM").ok().as_deref())
    }

    /// [`Self::resolve`] for a given `$TERM_PROGRAM`. Over ssh it is usually unset, so
    /// `Auto` falls back to quadrants; set the pref to `half` in that case.
    pub fn resolve_for(self, term_program: Option<&str>) -> Self {
        match self {
            Self::Auto if term_program == Some("Apple_Terminal") => Self::Half,
            Self::Auto => Self::Quadrant,
            other => other,
        }
    }
}

pub fn is_global(key: &str) -> bool {
    GLOBAL_KEYS.contains(&key)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeysScheme {
    #[default]
    Default,
    Vim,
}

impl KeysScheme {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "default" => Some(Self::Default),
            "vim" => Some(Self::Vim),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Vim => "vim",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub default_bot: Option<String>,
    /// Last active bot id (restored on start).
    pub active_bot: Option<String>,
    /// Show agents flagged `isHiddenFromSidebar`.
    pub show_hidden_bots: bool,
    pub sidebar_width: u16,
    /// Optional EnsureSandBox `x-cursor-client-version` override (pref).
    pub client_version: Option<String>,
    pub client_type: Option<String>,
    pub sandbox_namespace: Option<String>,
    /// Appearance: dark, light or system (follow the terminal background).
    pub theme_mode: crate::theme::Mode,
    /// Glyphs for the big header title.
    pub title_glyphs: TitleGlyphs,
    /// Selected theme id (`<data dir>/themes/<id>.toml`).
    pub theme: String,
    /// Allow downloading the shipped default theme (see `PREF_THEME_DOWNLOAD`).
    pub theme_download: bool,
    /// Capture the mouse for the TUI (F6 / `/mouse`).
    pub mouse_capture: bool,
    /// Keybinding scheme (`default` or `vim`).
    pub keys: KeysScheme,
    /// Prompt history cap per account (`history.max_entries`).
    pub history_max: usize,
    /// Download size cap in MB (`downloads.max_mb`, 1..=4096).
    pub download_max_mb: u64,
    /// Where links open (`links.open_with`).
    pub open_with: OpenWith,
    /// In SSH, links always use the built-in browser (`links.ssh_behavior = builtin`).
    pub ssh_builtin: bool,
    /// `images.inline`.
    pub images_inline: bool,
    /// `images.inline_max_rows`.
    pub inline_max_rows: u16,
    /// `commands.status_prompt`.
    pub status_prompt: String,
    /// `keys.esc_alt_timeout_ms`.
    pub esc_alt_ms: u64,
    /// `ui.header = compact`.
    pub header_compact: bool,
    /// `ui.sidebar_collapsed`.
    pub sidebar_collapsed: bool,
    /// `message.click_to_react`.
    pub click_to_react: bool,
    /// `notifications.claim_presence`.
    pub claim_presence: bool,
    /// `notifications.idle_minutes` (1..=240).
    pub idle_minutes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_bot: None,
            active_bot: None,
            show_hidden_bots: false,
            sidebar_width: 24,
            client_version: None,
            client_type: None,
            sandbox_namespace: None,
            theme_mode: crate::theme::Mode::System,
            title_glyphs: TitleGlyphs::Auto,
            theme: crate::themes::DEFAULT_ID.to_string(),
            theme_download: true,
            mouse_capture: true,
            keys: KeysScheme::Default,
            history_max: DEFAULT_HISTORY_MAX,
            download_max_mb: DEFAULT_DOWNLOAD_MAX_MB,
            open_with: OpenWith::Builtin,
            ssh_builtin: true,
            claim_presence: false,
            idle_minutes: DEFAULT_IDLE_MINUTES,
            images_inline: true,
            inline_max_rows: 12,
            status_prompt: DEFAULT_STATUS_PROMPT.to_string(),
            esc_alt_ms: 50,
            header_compact: false,
            sidebar_collapsed: false,
            click_to_react: true,
        }
    }
}

/// Known preference keys with their defaults (empty = unset).
pub fn default_prefs() -> Vec<(&'static str, String)> {
    let c = Config::default();
    vec![
        ("default_bot", String::new()),
        ("active_bot", String::new()),
        ("show_hidden_bots", c.show_hidden_bots.to_string()),
        ("ui.sidebar_width", c.sidebar_width.to_string()),
        (PREF_CLIENT_VERSION, String::new()),
        (PREF_CLIENT_TYPE, String::new()),
        (PREF_SANDBOX_NAMESPACE, String::new()),
        (PREF_THEME_MODE, c.theme_mode.as_str().to_string()),
        (PREF_TITLE_GLYPHS, c.title_glyphs.as_str().to_string()),
        (PREF_THEME, c.theme.clone()),
        (PREF_THEME_DOWNLOAD, c.theme_download.to_string()),
        (PREF_MOUSE_CAPTURE, c.mouse_capture.to_string()),
        (PREF_KEYS, c.keys.as_str().to_string()),
        (PREF_HISTORY_MAX, c.history_max.to_string()),
        (PREF_DOWNLOAD_MAX_MB, c.download_max_mb.to_string()),
        (PREF_LINKS_OPEN_WITH, c.open_with.as_str().to_string()),
        (PREF_LINKS_SSH_BEHAVIOR, "builtin".to_string()),
        (PREF_CLAIM_PRESENCE, c.claim_presence.to_string()),
        (PREF_IDLE_MINUTES, c.idle_minutes.to_string()),
        (PREF_IMAGES_INLINE, "on".to_string()),
        (PREF_IMAGES_INLINE_MAX_ROWS, c.inline_max_rows.to_string()),
        (PREF_STATUS_PROMPT, c.status_prompt.clone()),
        (PREF_ESC_ALT_MS, c.esc_alt_ms.to_string()),
        (PREF_HEADER, "full".to_string()),
        (PREF_SIDEBAR_COLLAPSED, c.sidebar_collapsed.to_string()),
        (PREF_CLICK_TO_REACT, c.click_to_react.to_string()),
    ]
}

impl Config {
    pub fn from_prefs(p: &BTreeMap<String, String>) -> Self {
        let d = Config::default();
        let o = |k: &str| p.get(k).filter(|v| !v.is_empty()).cloned();
        Config {
            default_bot: o("default_bot"),
            active_bot: o("active_bot"),
            show_hidden_bots: o("show_hidden_bots")
                .is_some_and(|v| matches!(v.as_str(), "true" | "1" | "yes")),
            sidebar_width: o("ui.sidebar_width")
                .and_then(|v| v.parse().ok())
                .filter(|w| (10..=80).contains(w))
                .unwrap_or(d.sidebar_width),
            client_version: o(PREF_CLIENT_VERSION),
            client_type: o(PREF_CLIENT_TYPE),
            sandbox_namespace: o(PREF_SANDBOX_NAMESPACE),
            theme_mode: o(PREF_THEME_MODE)
                .and_then(|v| crate::theme::Mode::parse(&v))
                .unwrap_or(d.theme_mode),
            title_glyphs: o(PREF_TITLE_GLYPHS)
                .and_then(|v| TitleGlyphs::parse(&v))
                .unwrap_or(d.title_glyphs),
            theme: o(PREF_THEME)
                .map(|v| v.trim().to_string())
                .filter(|v| crate::themes::valid_id(v))
                .unwrap_or(d.theme),
            theme_download: o(PREF_THEME_DOWNLOAD)
                .map(|v| !matches!(v.trim(), "false" | "0" | "no" | "off"))
                .unwrap_or(d.theme_download),
            mouse_capture: o(PREF_MOUSE_CAPTURE)
                .map(|v| !matches!(v.trim(), "false" | "0" | "no" | "off"))
                .unwrap_or(d.mouse_capture),
            keys: o(PREF_KEYS)
                .and_then(|v| KeysScheme::parse(&v))
                .unwrap_or(d.keys),
            history_max: o(PREF_HISTORY_MAX)
                .and_then(|v| v.trim().replace('_', "").parse().ok())
                .filter(|n| HISTORY_MAX_RANGE.contains(n))
                .unwrap_or(d.history_max),
            download_max_mb: o(PREF_DOWNLOAD_MAX_MB)
                .and_then(|v| v.trim().parse().ok())
                .filter(|n| (1..=4096).contains(n))
                .unwrap_or(d.download_max_mb),
            open_with: o(PREF_LINKS_OPEN_WITH)
                .and_then(|v| OpenWith::parse(&v))
                .unwrap_or(d.open_with),
            ssh_builtin: o(PREF_LINKS_SSH_BEHAVIOR)
                .map(|v| !matches!(v.trim(), "respect_pref" | "respect-pref" | "pref"))
                .unwrap_or(d.ssh_builtin),
            claim_presence: o(PREF_CLAIM_PRESENCE)
                .is_some_and(|v| matches!(v.trim(), "true" | "1" | "yes" | "on")),
            idle_minutes: o(PREF_IDLE_MINUTES)
                .and_then(|v| v.trim().parse().ok())
                .filter(|n| (1..=240).contains(n))
                .unwrap_or(d.idle_minutes),
            images_inline: o(PREF_IMAGES_INLINE)
                .map(|v| !matches!(v.trim(), "off" | "false" | "0" | "no"))
                .unwrap_or(d.images_inline),
            inline_max_rows: o(PREF_IMAGES_INLINE_MAX_ROWS)
                .and_then(|v| v.trim().parse().ok())
                .filter(|n| (2..=60).contains(n))
                .unwrap_or(d.inline_max_rows),
            status_prompt: o(PREF_STATUS_PROMPT)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or(d.status_prompt),
            esc_alt_ms: o(PREF_ESC_ALT_MS)
                .and_then(|v| v.trim().parse().ok())
                .filter(|n| *n <= 1000)
                .unwrap_or(d.esc_alt_ms),
            header_compact: o(PREF_HEADER).is_some_and(|v| v.trim() == "compact"),
            sidebar_collapsed: o(PREF_SIDEBAR_COLLAPSED)
                .is_some_and(|v| matches!(v.trim(), "true" | "1" | "yes" | "on")),
            click_to_react: o(PREF_CLICK_TO_REACT)
                .map(|v| !matches!(v.trim(), "false" | "0" | "no" | "off"))
                .unwrap_or(d.click_to_react),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefs_roundtrip_defaults_and_overrides() {
        let mut m: BTreeMap<String, String> = default_prefs()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(Config::from_prefs(&m), Config::default());
        m.insert("ui.sidebar_width".into(), "999".into());
        m.insert("show_hidden_bots".into(), "true".into());
        let c = Config::from_prefs(&m);
        assert_eq!(c.sidebar_width, 24);
        assert!(c.show_hidden_bots);
        assert_eq!(c.theme_mode, crate::theme::Mode::System);
        m.insert(PREF_THEME_MODE.into(), "light".into());
        assert_eq!(Config::from_prefs(&m).theme_mode, crate::theme::Mode::Light);
        assert_eq!(c.open_with, OpenWith::Builtin);
        assert!(c.ssh_builtin);
        assert!(is_global(PREF_LINKS_OPEN_WITH));
        m.insert(PREF_LINKS_OPEN_WITH.into(), "system".into());
        m.insert(PREF_LINKS_SSH_BEHAVIOR.into(), "respect_pref".into());
        assert_eq!(Config::from_prefs(&m).open_with, OpenWith::System);
        assert!(!Config::from_prefs(&m).ssh_builtin);
        assert!(!c.claim_presence);
        assert_eq!(c.idle_minutes, 10);
        assert!(!is_global(PREF_CLAIM_PRESENCE), "per account");
        m.insert(PREF_CLAIM_PRESENCE.into(), "true".into());
        m.insert(PREF_IDLE_MINUTES.into(), "0".into());
        assert!(Config::from_prefs(&m).claim_presence);
        assert_eq!(Config::from_prefs(&m).idle_minutes, 10);
        assert_eq!(c.esc_alt_ms, 50);
        assert!(c.click_to_react && !c.header_compact && !c.sidebar_collapsed);
        m.insert(PREF_ESC_ALT_MS.into(), "5000".into());
        m.insert(PREF_HEADER.into(), "compact".into());
        m.insert(PREF_SIDEBAR_COLLAPSED.into(), "true".into());
        m.insert(PREF_CLICK_TO_REACT.into(), "off".into());
        let x = Config::from_prefs(&m);
        assert_eq!(x.esc_alt_ms, 50);
        assert!(x.header_compact && x.sidebar_collapsed && !x.click_to_react);
        for k in [
            PREF_ESC_ALT_MS,
            PREF_HEADER,
            PREF_SIDEBAR_COLLAPSED,
            PREF_CLICK_TO_REACT,
        ] {
            m.remove(k);
        }
        m.remove(PREF_CLAIM_PRESENCE);
        m.remove(PREF_IDLE_MINUTES);
        m.remove(PREF_LINKS_OPEN_WITH);
        m.remove(PREF_LINKS_SSH_BEHAVIOR);
        m.insert(PREF_THEME_MODE.into(), "purple".into());
        assert_eq!(
            Config::from_prefs(&m).theme_mode,
            crate::theme::Mode::System
        );
        assert!(is_global(PREF_THEME_MODE));
        assert_eq!(c.theme, "default");
        assert!(is_global(PREF_THEME));
        assert!(c.theme_download);
        assert!(is_global(PREF_THEME_DOWNLOAD));
        m.insert(PREF_THEME_DOWNLOAD.into(), "false".into());
        assert!(!Config::from_prefs(&m).theme_download);
        m.insert(PREF_THEME_DOWNLOAD.into(), "true".into());
        assert!(Config::from_prefs(&m).theme_download);
        assert_eq!(c.history_max, DEFAULT_HISTORY_MAX);
        assert!(is_global(PREF_HISTORY_MAX));
        m.insert(PREF_HISTORY_MAX.into(), "50000".into());
        assert_eq!(Config::from_prefs(&m).history_max, 50_000);
        m.insert(PREF_HISTORY_MAX.into(), "5".into());
        assert_eq!(Config::from_prefs(&m).history_max, DEFAULT_HISTORY_MAX);
        m.insert(PREF_THEME.into(), "solar-2".into());
        assert_eq!(Config::from_prefs(&m).theme, "solar-2");
        m.insert(PREF_THEME.into(), "../etc/passwd".into());
        assert_eq!(
            Config::from_prefs(&m).theme,
            "default",
            "invalid names are ignored"
        );
    }
}

#[cfg(test)]
mod title_glyph_tests {
    use super::*;

    #[test]
    fn auto_picks_half_only_in_apple_terminal() {
        let a = TitleGlyphs::Auto;
        assert_eq!(a.resolve_for(Some("Apple_Terminal")), TitleGlyphs::Half);
        assert_eq!(a.resolve_for(Some("iTerm.app")), TitleGlyphs::Quadrant);
        assert_eq!(a.resolve_for(None), TitleGlyphs::Quadrant);
        assert_eq!(TitleGlyphs::Half.resolve_for(None), TitleGlyphs::Half);
        assert_eq!(
            TitleGlyphs::Quadrant.resolve_for(Some("Apple_Terminal")),
            TitleGlyphs::Quadrant
        );
    }

    #[test]
    fn parses_pref_values() {
        for g in [TitleGlyphs::Auto, TitleGlyphs::Quadrant, TitleGlyphs::Half] {
            assert_eq!(TitleGlyphs::parse(g.as_str()), Some(g));
        }
        assert_eq!(TitleGlyphs::parse(" Quarter "), Some(TitleGlyphs::Quadrant));
        assert_eq!(TitleGlyphs::parse("nope"), None);
        let mut m = BTreeMap::new();
        m.insert(PREF_TITLE_GLYPHS.to_string(), "half".to_string());
        assert_eq!(Config::from_prefs(&m).title_glyphs, TitleGlyphs::Half);
        m.insert(PREF_TITLE_GLYPHS.to_string(), "bogus".to_string());
        assert_eq!(Config::from_prefs(&m).title_glyphs, TitleGlyphs::Auto);
        assert!(is_global(PREF_TITLE_GLYPHS));
    }
}

#[cfg(test)]
mod max_response_tests {
    use super::*;

    #[test]
    fn parses_override_and_falls_back() {
        assert_eq!(parse_max_response_bytes(None), DEFAULT_MAX_RESPONSE_BYTES);
        assert_eq!(
            parse_max_response_bytes(Some("")),
            DEFAULT_MAX_RESPONSE_BYTES
        );
        assert_eq!(
            parse_max_response_bytes(Some("abc")),
            DEFAULT_MAX_RESPONSE_BYTES
        );
        assert_eq!(
            parse_max_response_bytes(Some("0")),
            DEFAULT_MAX_RESPONSE_BYTES
        );
        assert_eq!(parse_max_response_bytes(Some(" 1048576 ")), 1_048_576);
        const { assert!(DEFAULT_MAX_RESPONSE_BYTES > grokbot_api::MAX_RESPONSE_BYTES) };
    }
}
