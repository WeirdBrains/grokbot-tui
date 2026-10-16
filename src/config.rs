//! Typed view over the key-value preferences stored in the database.
use std::collections::BTreeMap;

pub const TOKEN_ENV: &str = "GB_GROKBOT_TOKEN";
pub const DEFAULT_ACCOUNT: &str = "default";

/// Preference keys shared by all accounts. Every other key is account-scoped.
pub const GLOBAL_KEYS: &[&str] = &["active_account", "ui.sidebar_width", "legacy_imported"];

pub fn is_global(key: &str) -> bool {
    GLOBAL_KEYS.contains(&key)
}

/// API endpoint paths. **PLACEHOLDERS** — no confirmed Grok Bot API spec yet.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiPaths {
    pub whoami: String,
    pub bots: String,
    /// `{bot}` is replaced with the bot id.
    pub history: String,
    /// `{bot}` is replaced with the bot id.
    pub send: String,
    pub revoke: Option<String>,
}

impl Default for ApiPaths {
    fn default() -> Self {
        Self {
            whoami: "/v1/me".into(),
            bots: "/v1/bots".into(),
            history: "/v1/bots/{bot}/messages".into(),
            send: "/v1/bots/{bot}/messages".into(),
            revoke: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// PLACEHOLDER base URL.
    pub api_base_url: String,
    pub paths: ApiPaths,
    pub default_bot: Option<String>,
    /// Last active bot id (restored on start).
    pub active_bot: Option<String>,
    pub request_timeout_secs: u64,
    pub sidebar_width: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.grokbot.example".into(),
            paths: ApiPaths::default(),
            default_bot: None,
            active_bot: None,
            request_timeout_secs: 30,
            sidebar_width: 24,
        }
    }
}

/// Known preference keys with their defaults (empty = unset).
pub fn default_prefs() -> Vec<(&'static str, String)> {
    let c = Config::default();
    vec![
        ("api_base_url", c.api_base_url),
        ("request_timeout_secs", c.request_timeout_secs.to_string()),
        ("paths.whoami", c.paths.whoami),
        ("paths.bots", c.paths.bots),
        ("paths.history", c.paths.history),
        ("paths.send", c.paths.send),
        ("paths.revoke", String::new()),
        ("default_bot", String::new()),
        ("active_bot", String::new()),
        ("ui.sidebar_width", c.sidebar_width.to_string()),
    ]
}

impl Config {
    pub fn from_prefs(p: &BTreeMap<String, String>) -> Self {
        let d = Config::default();
        let s = |k: &str, def: String| p.get(k).filter(|v| !v.is_empty()).cloned().unwrap_or(def);
        let o = |k: &str| p.get(k).filter(|v| !v.is_empty()).cloned();
        Config {
            api_base_url: s("api_base_url", d.api_base_url),
            paths: ApiPaths {
                whoami: s("paths.whoami", d.paths.whoami),
                bots: s("paths.bots", d.paths.bots),
                history: s("paths.history", d.paths.history),
                send: s("paths.send", d.paths.send),
                revoke: o("paths.revoke"),
            },
            default_bot: o("default_bot"),
            active_bot: o("active_bot"),
            request_timeout_secs: o("request_timeout_secs")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.request_timeout_secs),
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
        m.insert("api_base_url".into(), "http://x".into());
        m.insert("ui.sidebar_width".into(), "999".into());
        m.insert("paths.revoke".into(), "/r".into());
        let c = Config::from_prefs(&m);
        assert_eq!(c.api_base_url, "http://x");
        assert_eq!(c.sidebar_width, 24);
        assert_eq!(c.paths.revoke.as_deref(), Some("/r"));
    }
}
