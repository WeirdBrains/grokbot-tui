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

/// Preference keys shared by all accounts. Every other key is account-scoped.
pub const GLOBAL_KEYS: &[&str] = &["active_account", "ui.sidebar_width", PREF_THEME_MODE];

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
    }
}
