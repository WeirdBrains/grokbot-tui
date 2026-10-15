//! SQLite storage at `~/.gb/context.db`: credentials, bot memory, preferences.
use crate::api::{Message, Role, User};
use crate::config::{Config, TOKEN_ENV, default_prefs};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Ordered migrations; index + 1 == resulting `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[
    // v1
    "CREATE TABLE credentials (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        token TEXT NOT NULL,
        user_id TEXT,
        user_name TEXT,
        api_base TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        bot_id TEXT NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('user','bot')),
        text TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX messages_bot ON messages(bot_id, id);
    CREATE TABLE memory (
        bot_id TEXT NOT NULL,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (datetime('now')),
        PRIMARY KEY (bot_id, key)
    );
    CREATE TABLE prefs (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );",
];

pub struct Store {
    pub dir: PathBuf,
    conn: Connection,
}

impl Store {
    pub fn default_dir() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".gb"))
    }

    /// Open (creating if needed) `<dir>/context.db` and run migrations.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        set_mode(&dir, 0o700)?;
        let path = dir.join("context.db");
        let conn =
            Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        set_mode(&path, 0o600)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let s = Self { dir, conn };
        s.migrate()?;
        s.seed_prefs()?;
        Ok(s)
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join("context.db")
    }

    pub fn schema_version(&self) -> Result<usize> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize)
    }

    fn migrate(&self) -> Result<()> {
        let v = self.schema_version()?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    fn seed_prefs(&self) -> Result<()> {
        for (k, v) in default_prefs() {
            self.conn.execute(
                "INSERT OR IGNORE INTO prefs(key, value) VALUES (?1, ?2)",
                params![k, v],
            )?;
        }
        Ok(())
    }

    // ---- preferences

    pub fn get_pref(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM prefs WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_pref(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO prefs(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn list_prefs(&self) -> Result<BTreeMap<String, String>> {
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM prefs ORDER BY key")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn load_config(&self) -> Result<Config> {
        Ok(Config::from_prefs(&self.list_prefs()?))
    }

    // ---- credentials

    /// `GROKBOT_TOKEN` env var wins, then the stored credential.
    pub fn load_token(&self) -> Option<String> {
        if let Ok(t) = std::env::var(TOKEN_ENV)
            && !t.trim().is_empty()
        {
            return Some(t.trim().to_string());
        }
        self.load_stored_token()
    }

    pub fn load_stored_token(&self) -> Option<String> {
        self.conn
            .query_row("SELECT token FROM credentials WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn save_credentials(&self, token: &str, user: Option<&User>, api_base: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO credentials(id, token, user_id, user_name, api_base) VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET token = excluded.token, user_id = excluded.user_id,
               user_name = excluded.user_name, api_base = excluded.api_base, updated_at = datetime('now')",
            params![token.trim(), user.map(|u| &u.id), user.map(|u| &u.name), api_base],
        )?;
        Ok(())
    }

    /// Returns true if a stored credential was deleted.
    pub fn delete_credentials(&self) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM credentials", [])? > 0)
    }

    // ---- bot memory: chat messages

    pub fn append_message(&self, bot_id: &str, m: &Message) -> Result<()> {
        let role = if m.role == Role::User { "user" } else { "bot" };
        self.conn.execute(
            "INSERT INTO messages(bot_id, role, text) VALUES (?1, ?2, ?3)",
            params![bot_id, role, m.text],
        )?;
        Ok(())
    }

    pub fn messages(&self, bot_id: &str) -> Result<Vec<Message>> {
        let mut st = self
            .conn
            .prepare("SELECT role, text FROM messages WHERE bot_id = ?1 ORDER BY id")?;
        let rows = st.query_map([bot_id], |r| {
            let role: String = r.get(0)?;
            Ok(Message {
                role: if role == "user" {
                    Role::User
                } else {
                    Role::Bot
                },
                text: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replace a bot's stored transcript (e.g. with server history).
    pub fn replace_messages(&self, bot_id: &str, msgs: &[Message]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM messages WHERE bot_id = ?1", [bot_id])?;
        tx.commit()?;
        for m in msgs {
            self.append_message(bot_id, m)?;
        }
        Ok(())
    }

    pub fn clear_messages(&self, bot_id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM messages WHERE bot_id = ?1", [bot_id])?;
        Ok(())
    }

    // ---- bot memory: key-value notes

    pub fn set_memory(&self, bot_id: &str, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO memory(bot_id, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(bot_id, key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            params![bot_id, key, value],
        )?;
        Ok(())
    }

    pub fn list_memory(&self, bot_id: &str) -> Result<Vec<(String, String)>> {
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM memory WHERE bot_id = ?1 ORDER BY key")?;
        let rows = st.query_map([bot_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Clear one key, or all keys for the bot when `key` is None. Returns rows removed.
    pub fn clear_memory(&self, bot_id: &str, key: Option<&str>) -> Result<usize> {
        Ok(match key {
            Some(k) => self.conn.execute(
                "DELETE FROM memory WHERE bot_id = ?1 AND key = ?2",
                params![bot_id, k],
            )?,
            None => self
                .conn
                .execute("DELETE FROM memory WHERE bot_id = ?1", [bot_id])?,
        })
    }

    // ---- legacy import

    /// One-time best-effort import from the old `~/.grokbot-tui` (config.toml + credentials).
    /// Marked done via the `legacy_imported` pref. Returns a summary when something was imported.
    pub fn import_legacy(&self, old: &Path) -> Option<String> {
        if self.get_pref("legacy_imported").ok().flatten().is_some() || !old.is_dir() {
            return None;
        }
        let mut done = vec![];
        if let Ok(s) = fs::read_to_string(old.join("config.toml"))
            && let Ok(v) = s.parse::<toml::Table>()
        {
            let mut n = 0;
            for (k, val) in flatten(&v, "") {
                if self.set_pref(&k, &val).is_ok() {
                    n += 1;
                }
            }
            done.push(format!("{n} preferences"));
        }
        if let Ok(t) = fs::read_to_string(old.join("credentials"))
            && !t.trim().is_empty()
            && self.load_stored_token().is_none()
        {
            let base = self
                .get_pref("api_base_url")
                .ok()
                .flatten()
                .unwrap_or_default();
            if self.save_credentials(t.trim(), None, &base).is_ok() {
                done.push("credential".into());
            }
        }
        let _ = self.set_pref("legacy_imported", "1");
        (!done.is_empty())
            .then(|| format!("Imported {} from {}", done.join(" and "), old.display()))
    }
}

fn flatten(t: &toml::Table, prefix: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    for (k, v) in t {
        let key = format!("{prefix}{k}");
        match v {
            toml::Value::Table(sub) => out.extend(flatten(sub, &format!("{key}."))),
            toml::Value::String(s) => out.push((key, s.clone())),
            other => out.push((key, other.to_string())),
        }
    }
    out
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

    fn store() -> (Store, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Store::open(d.path().join("gb")).unwrap(), d)
    }

    #[test]
    fn migrates_and_permissions() {
        let (s, d) = store();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
        drop(s);
        let s = Store::open(d.path().join("gb")).unwrap(); // re-open is idempotent
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(s.db_path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&s.dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn prefs_and_config() {
        let (s, _d) = store();
        assert_eq!(s.load_config().unwrap(), Config::default());
        s.set_pref("api_base_url", "http://local").unwrap();
        assert_eq!(s.load_config().unwrap().api_base_url, "http://local");
        assert_eq!(s.get_pref("nope").unwrap(), None);
    }

    #[test]
    fn credentials_crud() {
        let (s, _d) = store();
        assert!(s.load_stored_token().is_none());
        let u = User {
            id: "u1".into(),
            name: "me".into(),
        };
        s.save_credentials(" abc ", Some(&u), "http://x").unwrap();
        s.save_credentials("def", Some(&u), "http://x").unwrap();
        assert_eq!(s.load_stored_token().as_deref(), Some("def"));
        assert!(s.delete_credentials().unwrap());
        assert!(!s.delete_credentials().unwrap());
        assert!(s.load_stored_token().is_none());
    }

    #[test]
    fn messages_and_memory_persist_per_bot() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = Store::open(d.path()).unwrap();
            s.append_message(
                "a",
                &Message {
                    role: Role::User,
                    text: "hi".into(),
                },
            )
            .unwrap();
            s.append_message(
                "a",
                &Message {
                    role: Role::Bot,
                    text: "yo".into(),
                },
            )
            .unwrap();
            s.set_memory("a", "tone", "brief").unwrap();
            s.set_memory("a", "tone", "terse").unwrap();
        }
        let s = Store::open(d.path()).unwrap();
        assert_eq!(s.messages("a").unwrap().len(), 2);
        assert!(s.messages("b").unwrap().is_empty());
        assert_eq!(
            s.list_memory("a").unwrap(),
            vec![("tone".into(), "terse".into())]
        );
        assert_eq!(s.clear_memory("a", None).unwrap(), 1);
        s.replace_messages("a", &[]).unwrap();
        assert!(s.messages("a").unwrap().is_empty());
    }

    #[test]
    fn legacy_import_once() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join("old");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("config.toml"),
            "api_base_url = \"http://old\"\n[paths]\nbots = \"/b\"\n",
        )
        .unwrap();
        fs::write(old.join("credentials"), "tok\n").unwrap();
        let s = Store::open(d.path().join("new")).unwrap();
        assert!(s.import_legacy(&old).is_some());
        let c = s.load_config().unwrap();
        assert_eq!(c.api_base_url, "http://old");
        assert_eq!(c.paths.bots, "/b");
        assert_eq!(s.load_stored_token().as_deref(), Some("tok"));
        assert!(s.import_legacy(&old).is_none());
    }
}
