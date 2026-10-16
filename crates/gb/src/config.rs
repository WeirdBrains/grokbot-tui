//! Typed view over the key-value preferences stored in the database.
use std::collections::BTreeMap;

/// Gateway token override for the account selected at startup.
pub const TOKEN_ENV: &str = "GB_GROKBOT_TOKEN";
/// Optional gateway URL to pair with [`TOKEN_ENV`].
pub const GATEWAY_URL_ENV: &str = "GB_GROKBOT_GATEWAY_URL";
/// Optional JSON object of extra gateway headers to pair with [`TOKEN_ENV`].
pub const GATEWAY_HEADERS_ENV: &str = "GB_GROKBOT_GATEWAY_HEADERS";
/// Override the Cursor backend used for EnsureSandBox (must still be *.cursor.sh / *.cursor.com).
pub const CURSOR_API_ENV: &str = "GB_CURSOR_API_BASE_URL";
pub const DEFAULT_ACCOUNT: &str = "default";

/// Preference keys shared by all accounts. Every other key is account-scoped.
pub const GLOBAL_KEYS: &[&str] = &["active_account", "ui.sidebar_width"];

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_bot: None,
            active_bot: None,
            show_hidden_bots: false,
            sidebar_width: 24,
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
    }
}
