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
