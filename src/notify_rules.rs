//! Notification ruleset: decide which events toast, mute, or pass through.
//!
//! Adapted from acp-rs-scraper's `src/rules` (commit a60c82a) gist: conditions +
//! actions in a user-editable file. File: `~/.grokbot-tui/notify-rules.toml`.

use crate::notify::NoticeKind;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub fn user_path(data_dir: &Path) -> PathBuf {
    data_dir.join("notify-rules.toml")
}

pub fn shipped_default() -> &'static str {
    include_str!("../notify-rules/default.toml")
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Show a toast (and keep the log).
    Toast,
    /// Record in the log only — no toast.
    Mute,
    /// Drop entirely (no log, no toast).
    Drop,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    #[allow(dead_code)] // identity in TOML; matching uses kind/keyword
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Lower runs first.
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub kind: Option<String>,
    /// Case-insensitive substring of the body or title.
    #[serde(default)]
    pub keyword: Option<String>,
    pub action: Action,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RulesFile {
    #[serde(default)]
    pub rule: Vec<Rule>,
}

#[derive(Debug, Clone, Default)]
pub struct Ruleset {
    pub rules: Vec<Rule>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) | Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl Ruleset {
    pub fn parse(text: &str) -> Result<Self, LoadError> {
        let file: RulesFile = toml::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
        let mut rules = file.rule;
        rules.sort_by_key(|r| r.priority);
        Ok(Self { rules, path: None })
    }

    pub fn load(path: &Path, previous: Option<&Ruleset>) -> (Self, Option<LoadError>) {
        match std::fs::read_to_string(path) {
            Ok(t) => match Self::parse(&t) {
                Ok(mut rs) => {
                    rs.path = Some(path.to_path_buf());
                    (rs, None)
                }
                Err(e) => {
                    let mut fb = previous.cloned().unwrap_or_else(|| {
                        Self::parse(shipped_default()).expect("shipped rules")
                    });
                    fb.path = Some(path.to_path_buf());
                    (fb, Some(e))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = std::fs::write(path, shipped_default());
                let mut rs = Self::parse(shipped_default()).expect("shipped");
                rs.path = Some(path.to_path_buf());
                (rs, None)
            }
            Err(e) => {
                let mut fb = previous.cloned().unwrap_or_else(|| {
                    Self::parse(shipped_default()).expect("shipped")
                });
                fb.path = Some(path.to_path_buf());
                (fb, Some(LoadError::Io(format!("{}: {e}", path.display()))))
            }
        }
    }

    /// First matching enabled rule wins. Default action is Toast.
    pub fn decide(&self, kind: NoticeKind, title: &str, body: &str) -> Action {
        let hay = format!("{title}\n{body}").to_ascii_lowercase();
        for r in &self.rules {
            if !r.enabled {
                continue;
            }
            if let Some(k) = &r.kind
                && NoticeKind::parse(k) != Some(kind)
            {
                continue;
            }
            if let Some(kw) = &r.keyword
                && !hay.contains(&kw.to_ascii_lowercase())
            {
                continue;
            }
            return r.action.clone();
        }
        Action::Toast
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_default_parses() {
        let rs = Ruleset::parse(shipped_default()).unwrap();
        assert!(!rs.rules.is_empty());
    }

    #[test]
    fn keyword_mute_beats_default_toast() {
        let rs = Ruleset::parse(
            r#"
[[rule]]
name = "quiet heartbeats"
priority = 10
keyword = "heartbeat"
action = "mute"
"#,
        )
        .unwrap();
        assert_eq!(
            rs.decide(NoticeKind::Info, "Info", "agent heartbeat ok"),
            Action::Mute
        );
        assert_eq!(
            rs.decide(NoticeKind::Error, "Error", "boom"),
            Action::Toast
        );
    }

    #[test]
    fn kind_drop_swallows_messages() {
        let rs = Ruleset::parse(
            r#"
[[rule]]
name = "no message toasts"
kind = "message"
action = "drop"
"#,
        )
        .unwrap();
        assert_eq!(
            rs.decide(NoticeKind::Message, "Bot", "hi"),
            Action::Drop
        );
    }
}
