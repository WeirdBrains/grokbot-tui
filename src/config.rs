//! Config and credential storage in `~/.grokbot-tui/`.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TOKEN_ENV: &str = "GROKBOT_TOKEN";

/// API endpoint paths. **PLACEHOLDERS** — there is no confirmed Grok Bot API
/// spec yet. Replace these once the real API is documented.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// PLACEHOLDER base URL.
    pub api_base_url: String,
    pub paths: ApiPaths,
    pub default_bot: Option<String>,
    pub request_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_base_url: "https://api.grokbot.example".into(),
            paths: ApiPaths::default(),
            default_bot: None,
            request_timeout_secs: 30,
        }
    }
}

/// Paths for everything stored on disk.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn default_location() -> Result<Self> {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(Self {
            dir: home.join(".grokbot-tui"),
        })
    }

    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join("config.toml")
    }

    pub fn credentials_path(&self) -> PathBuf {
        self.dir.join("credentials")
    }

    fn ensure_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        set_mode(&self.dir, 0o700)
    }

    /// Load config, writing a default file on first run.
    pub fn load_config(&self) -> Result<Config> {
        let p = self.config_path();
        if !p.exists() {
            self.ensure_dir()?;
            let cfg = Config::default();
            fs::write(&p, toml::to_string_pretty(&cfg)?)?;
            return Ok(cfg);
        }
        let s = fs::read_to_string(&p)?;
        toml::from_str(&s).with_context(|| format!("parsing {}", p.display()))
    }

    /// Token resolution: `GROKBOT_TOKEN` env var wins, then the credentials file.
    pub fn load_token(&self) -> Option<String> {
        if let Ok(t) = std::env::var(TOKEN_ENV)
            && !t.trim().is_empty()
        {
            return Some(t.trim().to_string());
        }
        self.load_stored_token()
    }

    pub fn load_stored_token(&self) -> Option<String> {
        let t = fs::read_to_string(self.credentials_path()).ok()?;
        let t = t.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    pub fn save_token(&self, token: &str) -> Result<()> {
        self.ensure_dir()?;
        let p = self.credentials_path();
        write_private(&p, token.trim())
    }

    /// Returns true if a stored credential was deleted.
    pub fn delete_token(&self) -> Result<bool> {
        let p = self.credentials_path();
        if p.exists() {
            fs::remove_file(p)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(unix)]
fn write_private(p: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(p)?;
    f.write_all(contents.as_bytes())?;
    set_mode(p, 0o600)
}

#[cfg(not(unix))]
fn write_private(p: &Path, contents: &str) -> Result<()> {
    Ok(fs::write(p, contents)?)
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_written_and_roundtrips() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(d.path().join("gb"));
        let c = s.load_config().unwrap();
        assert_eq!(c, Config::default());
        assert!(s.config_path().exists());
        assert_eq!(s.load_config().unwrap(), c);
    }

    #[test]
    fn token_save_load_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::at(d.path().join("gb"));
        assert!(s.load_stored_token().is_none());
        s.save_token("  abc123\n").unwrap();
        assert_eq!(s.load_stored_token().as_deref(), Some("abc123"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(s.credentials_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o777, 0o600);
        }
        assert!(s.delete_token().unwrap());
        assert!(!s.delete_token().unwrap());
        assert!(s.load_stored_token().is_none());
    }
}
