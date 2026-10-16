//! SQLite storage at `~/.gb/context.db`.
//!
//! Data is segregated per **account**: credentials, bots, messages, memory and
//! account prefs all carry `account_id` (FK → `accounts`, `ON DELETE CASCADE`).
//! A [`Store`] is always bound to one active account and every scoped query
//! filters by it; only `global_prefs` is shared.
use crate::api::{Bot, Message, Role, Session};
use crate::config::{Config, DEFAULT_ACCOUNT, default_prefs, is_global};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const V1: &str = "CREATE TABLE credentials (
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
    );";

/// v1 -> v2: multi-account. Existing data moves into account 1 ('default').
const V2: &str = "CREATE TABLE accounts (
        id INTEGER PRIMARY KEY,
        alias TEXT NOT NULL UNIQUE COLLATE NOCASE,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    INSERT INTO accounts(id, alias) VALUES (1, 'default');

    CREATE TABLE credentials_v2 (
        account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        token TEXT NOT NULL,
        user_id TEXT,
        user_name TEXT,
        api_base TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    INSERT INTO credentials_v2 SELECT 1, token, user_id, user_name, api_base, created_at, updated_at FROM credentials;
    DROP TABLE credentials;
    ALTER TABLE credentials_v2 RENAME TO credentials;

    CREATE TABLE bots (
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        bot_id TEXT NOT NULL,
        name TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        PRIMARY KEY (account_id, bot_id)
    );
    INSERT INTO bots(account_id, bot_id, name)
        SELECT DISTINCT 1, bot_id, bot_id FROM (SELECT bot_id FROM messages UNION SELECT bot_id FROM memory);

    CREATE TABLE messages_v2 (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        account_id INTEGER NOT NULL,
        bot_id TEXT NOT NULL,
        role TEXT NOT NULL CHECK (role IN ('user','bot')),
        text TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        FOREIGN KEY (account_id, bot_id) REFERENCES bots(account_id, bot_id) ON DELETE CASCADE
    );
    INSERT INTO messages_v2(id, account_id, bot_id, role, text, created_at)
        SELECT id, 1, bot_id, role, text, created_at FROM messages;
    DROP INDEX messages_bot;
    DROP TABLE messages;
    ALTER TABLE messages_v2 RENAME TO messages;
    CREATE INDEX messages_account_bot ON messages(account_id, bot_id, id);

    CREATE TABLE memory_v2 (
        account_id INTEGER NOT NULL,
        bot_id TEXT NOT NULL,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (datetime('now')),
        PRIMARY KEY (account_id, bot_id, key),
        FOREIGN KEY (account_id, bot_id) REFERENCES bots(account_id, bot_id) ON DELETE CASCADE
    );
    INSERT INTO memory_v2 SELECT 1, bot_id, key, value, updated_at FROM memory;
    DROP TABLE memory;
    ALTER TABLE memory_v2 RENAME TO memory;

    CREATE TABLE global_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    INSERT INTO global_prefs SELECT key, value FROM prefs WHERE key IN ('ui.sidebar_width', 'legacy_imported');
    INSERT INTO global_prefs VALUES ('active_account', 'default');

    CREATE TABLE account_prefs (
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        key TEXT NOT NULL,
        value TEXT NOT NULL,
        PRIMARY KEY (account_id, key)
    );
    INSERT INTO account_prefs SELECT 1, key, value FROM prefs
        WHERE key NOT IN ('ui.sidebar_width', 'legacy_imported', 'active_account');
    DROP TABLE prefs;";

/// v2 -> v3: gateway credentials (the v2 placeholder-API token is dropped, it never
/// worked), transcript ids on messages, bot flags, and removal of placeholder API prefs.
const V3: &str = "DROP TABLE credentials;
    CREATE TABLE credentials (
        account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        method TEXT NOT NULL CHECK (method IN ('gateway', 'cursor')),
        gateway_url TEXT NOT NULL,
        gateway_token TEXT NOT NULL,
        gateway_headers TEXT NOT NULL DEFAULT '{}',
        cursor_token TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    ALTER TABLE messages ADD COLUMN remote_id TEXT;
    CREATE UNIQUE INDEX messages_remote ON messages(account_id, bot_id, remote_id) WHERE remote_id IS NOT NULL;
    ALTER TABLE bots ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE bots ADD COLUMN is_group INTEGER NOT NULL DEFAULT 0;
    DELETE FROM account_prefs WHERE key IN ('api_base_url', 'request_timeout_secs') OR key LIKE 'paths.%';
    DELETE FROM global_prefs WHERE key = 'legacy_imported';";

/// Ordered migrations; index + 1 == resulting `PRAGMA user_version`.
const MIGRATIONS: &[&str] = &[V1, V2, V3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    /// Gateway URL + gateway token pasted by the user.
    Gateway,
    /// Cursor access token exchanged via EnsureSandBox (re-exchanged on 401).
    Cursor,
}

/// A stored login for one account.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLogin {
    pub method: LoginMethod,
    pub session: Session,
    pub cursor_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub id: i64,
    pub alias: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Account,
}

pub struct Store {
    pub dir: PathBuf,
    conn: Connection,
    account: Account,
    /// Account `GB_GROKBOT_TOKEN` applies to (the one selected at startup).
    env_account: Option<i64>,
}

impl Store {
    pub fn default_dir() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .context("cannot determine home directory")?
            .join(".gb"))
    }

    /// Open `<dir>/context.db`, migrate, and bind to `account` (alias), else the
    /// saved active account, else `default`. A named account must exist.
    pub fn open(dir: impl Into<PathBuf>, account: Option<&str>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        set_mode(&dir, 0o700)?;
        let path = dir.join("context.db");
        let conn =
            Connection::open(&path).with_context(|| format!("opening {}", path.display()))?;
        set_mode(&path, 0o600)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&conn)?;

        let saved: Option<String> = conn
            .query_row(
                "SELECT value FROM global_prefs WHERE key = 'active_account'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let mut s = Self {
            dir,
            conn,
            account: Account {
                id: 0,
                alias: String::new(),
            },
            env_account: None,
        };
        let acct = match account {
            Some(a) => s
                .find_account(a)?
                .with_context(|| format!("no account named '{a}' (see /account)"))?,
            None => match saved
                .as_deref()
                .map(|a| s.find_account(a))
                .transpose()?
                .flatten()
            {
                Some(a) => a,
                None => match s.find_account(DEFAULT_ACCOUNT)? {
                    Some(a) => a,
                    None => s.add_account(DEFAULT_ACCOUNT)?,
                },
            },
        };
        s.env_account = Some(acct.id);
        s.bind(acct)?;
        for (k, v) in default_prefs().into_iter().filter(|(k, _)| is_global(k)) {
            s.conn.execute(
                "INSERT OR IGNORE INTO global_prefs(key, value) VALUES (?1, ?2)",
                params![k, v],
            )?;
        }
        Ok(s)
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join("context.db")
    }

    #[cfg(test)]
    pub fn schema_version(&self) -> Result<usize> {
        user_version(&self.conn)
    }

    // ---- accounts

    pub fn account(&self) -> &Account {
        &self.account
    }

    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut st = self
            .conn
            .prepare("SELECT id, alias FROM accounts ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok(Account {
                id: r.get(0)?,
                alias: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn find_account(&self, alias: &str) -> Result<Option<Account>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, alias FROM accounts WHERE alias = ?1",
                [alias.trim()],
                |r| {
                    Ok(Account {
                        id: r.get(0)?,
                        alias: r.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Create an account with default prefs.
    pub fn add_account(&self, alias: &str) -> Result<Account> {
        let alias = validate_alias(alias)?;
        if self.find_account(alias)?.is_some() {
            bail!("account '{alias}' already exists");
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT INTO accounts(alias) VALUES (?1)", [alias])?;
        let id = tx.last_insert_rowid();
        for (k, v) in default_prefs().into_iter().filter(|(k, _)| !is_global(k)) {
            tx.execute(
                "INSERT INTO account_prefs(account_id, key, value) VALUES (?1, ?2, ?3)",
                params![id, k, v],
            )?;
        }
        tx.commit()?;
        Ok(Account {
            id,
            alias: alias.to_string(),
        })
    }

    /// Switch the store to another account and remember it as active.
    pub fn use_account(&mut self, alias: &str) -> Result<&Account> {
        let a = self
            .find_account(alias)?
            .with_context(|| format!("no account named '{alias}'"))?;
        self.bind(a)?;
        Ok(&self.account)
    }

    fn bind(&mut self, a: Account) -> Result<()> {
        // Fill in any account prefs added since the account was created.
        for (k, v) in default_prefs().into_iter().filter(|(k, _)| !is_global(k)) {
            self.conn.execute(
                "INSERT OR IGNORE INTO account_prefs(account_id, key, value) VALUES (?1, ?2, ?3)",
                params![a.id, k, v],
            )?;
        }
        self.conn.execute(
            "INSERT INTO global_prefs(key, value) VALUES ('active_account', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&a.alias],
        )?;
        self.account = a;
        Ok(())
    }

    pub fn rename_account(&mut self, old: &str, new: &str) -> Result<()> {
        let new = validate_alias(new)?;
        let a = self
            .find_account(old)?
            .with_context(|| format!("no account named '{old}'"))?;
        if let Some(b) = self.find_account(new)?
            && b.id != a.id
        {
            bail!("account '{new}' already exists");
        }
        self.conn.execute(
            "UPDATE accounts SET alias = ?2 WHERE id = ?1",
            params![a.id, new],
        )?;
        if a.id == self.account.id {
            self.bind(Account {
                id: a.id,
                alias: new.to_string(),
            })?;
        }
        Ok(())
    }

    /// Delete an account and (via cascade) all of its data. The active account cannot be removed.
    pub fn remove_account(&self, alias: &str) -> Result<()> {
        let a = self
            .find_account(alias)?
            .with_context(|| format!("no account named '{alias}'"))?;
        if a.id == self.account.id {
            bail!("cannot remove the active account; switch with /account use first");
        }
        self.conn
            .execute("DELETE FROM accounts WHERE id = ?1", [a.id])?;
        Ok(())
    }

    // ---- preferences (global keys -> global_prefs, everything else -> account_prefs)

    pub fn get_pref(&self, key: &str) -> Result<Option<String>> {
        let v = if is_global(key) {
            self.conn.query_row(
                "SELECT value FROM global_prefs WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
        } else {
            self.conn.query_row(
                "SELECT value FROM account_prefs WHERE account_id = ?1 AND key = ?2",
                params![self.account.id, key],
                |r| r.get(0),
            )
        };
        Ok(v.optional()?)
    }

    pub fn set_pref(&self, key: &str, value: &str) -> Result<()> {
        if key == "active_account" {
            bail!("use /account use <alias> to switch accounts");
        }
        if is_global(key) {
            self.conn.execute(
                "INSERT INTO global_prefs(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO account_prefs(account_id, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, key) DO UPDATE SET value = excluded.value",
                params![self.account.id, key, value],
            )?;
        }
        Ok(())
    }

    /// Global prefs plus the active account's prefs, with their scope.
    pub fn list_prefs_scoped(&self) -> Result<Vec<(Scope, String, String)>> {
        let mut out = vec![];
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM global_prefs ORDER BY key")?;
        for r in st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (k, v) = r?;
            out.push((Scope::Global, k, v));
        }
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM account_prefs WHERE account_id = ?1 ORDER BY key")?;
        for r in st.query_map([self.account.id], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (k, v) = r?;
            out.push((Scope::Account, k, v));
        }
        Ok(out)
    }

    pub fn list_prefs(&self) -> Result<BTreeMap<String, String>> {
        Ok(self
            .list_prefs_scoped()?
            .into_iter()
            .map(|(_, k, v)| (k, v))
            .collect())
    }

    pub fn load_config(&self) -> Result<Config> {
        Ok(Config::from_prefs(&self.list_prefs()?))
    }

    // ---- credentials (active account)

    /// True when the active account is the one selected at startup (where `GB_GROKBOT_TOKEN` applies).
    pub fn is_startup_account(&self) -> bool {
        self.env_account == Some(self.account.id)
    }

    pub fn load_login(&self) -> Result<Option<StoredLogin>> {
        let row = self
            .conn
            .query_row(
                "SELECT method, gateway_url, gateway_token, gateway_headers, cursor_token FROM credentials WHERE account_id = ?1",
                [self.account.id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(m, url, tok, hdrs, cur)| StoredLogin {
            method: if m == "cursor" {
                LoginMethod::Cursor
            } else {
                LoginMethod::Gateway
            },
            session: Session {
                gateway_url: url,
                gateway_token: tok,
                headers: serde_json::from_str(&hdrs).unwrap_or_default(),
            },
            cursor_token: cur,
        }))
    }

    pub fn save_login(&self, l: &StoredLogin) -> Result<()> {
        let method = match l.method {
            LoginMethod::Gateway => "gateway",
            LoginMethod::Cursor => "cursor",
        };
        self.conn.execute(
            "INSERT INTO credentials(account_id, method, gateway_url, gateway_token, gateway_headers, cursor_token)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id) DO UPDATE SET method = excluded.method, gateway_url = excluded.gateway_url,
               gateway_token = excluded.gateway_token, gateway_headers = excluded.gateway_headers,
               cursor_token = excluded.cursor_token, updated_at = datetime('now')",
            params![
                self.account.id,
                method,
                l.session.gateway_url,
                l.session.gateway_token,
                serde_json::to_string(&l.session.headers)?,
                l.cursor_token
            ],
        )?;
        Ok(())
    }

    /// Returns true if a stored credential was deleted.
    pub fn delete_credentials(&self) -> Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM credentials WHERE account_id = ?1",
            [self.account.id],
        )? > 0)
    }

    // ---- bots (active account)

    /// Upsert the bot list for the active account (removed bots are kept so history survives).
    pub fn save_bots(&self, bots: &[Bot]) -> Result<()> {
        for b in bots {
            self.conn.execute(
                "INSERT INTO bots(account_id, bot_id, name, description, hidden, is_group) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, bot_id) DO UPDATE SET name = excluded.name, description = excluded.description,
                   hidden = excluded.hidden, is_group = excluded.is_group",
                params![self.account.id, b.id, b.name, b.description, b.hidden, b.is_group],
            )?;
        }
        Ok(())
    }

    pub fn bots(&self) -> Result<Vec<Bot>> {
        let mut st = self.conn.prepare(
            "SELECT bot_id, name, description, hidden, is_group FROM bots WHERE account_id = ?1 ORDER BY rowid",
        )?;
        let rows = st.query_map([self.account.id], |r| {
            Ok(Bot {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                hidden: r.get(3)?,
                is_group: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn ensure_bot(&self, bot_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO bots(account_id, bot_id, name) VALUES (?1, ?2, ?2)",
            params![self.account.id, bot_id],
        )?;
        Ok(())
    }

    // ---- messages (active account)

    pub fn append_message(&self, bot_id: &str, m: &Message) -> Result<()> {
        self.ensure_bot(bot_id)?;
        let role = if m.role == Role::User { "user" } else { "bot" };
        self.conn.execute(
            "INSERT INTO messages(account_id, bot_id, role, text, remote_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![self.account.id, bot_id, role, m.text, m.remote_id],
        )?;
        Ok(())
    }

    pub fn messages(&self, bot_id: &str) -> Result<Vec<Message>> {
        let mut st = self.conn.prepare(
            "SELECT role, text, remote_id FROM messages WHERE account_id = ?1 AND bot_id = ?2 ORDER BY id",
        )?;
        let rows = st.query_map(params![self.account.id, bot_id], |r| {
            let role: String = r.get(0)?;
            Ok(Message {
                role: if role == "user" {
                    Role::User
                } else {
                    Role::Bot
                },
                text: r.get(1)?,
                remote_id: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Merge transcript entries into the stored chat; returns the newly added messages.
    ///
    /// Entries with an id are de-duplicated by id. A user entry matching a locally
    /// pending send (`remote_id` = `pending:<nonce>`) adopts the server id instead of
    /// duplicating. Entries without an id are added only if no identical message exists.
    pub fn merge_remote(&self, bot_id: &str, msgs: &[Message]) -> Result<Vec<Message>> {
        let id = self.account.id;
        let mut added = vec![];
        for m in msgs {
            let role = if m.role == Role::User { "user" } else { "bot" };
            if let Some(rid) = &m.remote_id {
                let exists: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND remote_id = ?3)",
                    params![id, bot_id, rid],
                    |r| r.get(0),
                )?;
                if exists {
                    continue;
                }
                if m.role == Role::User {
                    let n = self.conn.execute(
                        "UPDATE messages SET remote_id = ?4 WHERE id = (
                           SELECT id FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND text = ?3
                             AND remote_id LIKE 'pending:%' ORDER BY id LIMIT 1)",
                        params![id, bot_id, m.text, rid],
                    )?;
                    if n > 0 {
                        continue;
                    }
                }
            } else {
                let exists: bool = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND role = ?3 AND text = ?4)",
                    params![id, bot_id, role, m.text],
                    |r| r.get(0),
                )?;
                if exists {
                    continue;
                }
            }
            self.append_message(bot_id, m)?;
            added.push(m.clone());
        }
        Ok(added)
    }

    /// Delete a locally pending message (e.g. its send was rejected).
    pub fn delete_pending(&self, bot_id: &str, nonce: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND remote_id = ?3",
            params![self.account.id, bot_id, format!("pending:{nonce}")],
        )?;
        Ok(())
    }

    pub fn clear_messages(&self, bot_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND bot_id = ?2",
            params![self.account.id, bot_id],
        )?;
        Ok(())
    }

    // ---- memory notes (active account)

    pub fn set_memory(&self, bot_id: &str, key: &str, value: &str) -> Result<()> {
        self.ensure_bot(bot_id)?;
        self.conn.execute(
            "INSERT INTO memory(account_id, bot_id, key, value) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, bot_id, key) DO UPDATE SET value = excluded.value, updated_at = datetime('now')",
            params![self.account.id, bot_id, key, value],
        )?;
        Ok(())
    }

    pub fn list_memory(&self, bot_id: &str) -> Result<Vec<(String, String)>> {
        let mut st = self.conn.prepare(
            "SELECT key, value FROM memory WHERE account_id = ?1 AND bot_id = ?2 ORDER BY key",
        )?;
        let rows = st.query_map(params![self.account.id, bot_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn clear_memory(&self, bot_id: &str, key: Option<&str>) -> Result<usize> {
        let id = self.account.id;
        Ok(match key {
            Some(k) => self.conn.execute(
                "DELETE FROM memory WHERE account_id = ?1 AND bot_id = ?2 AND key = ?3",
                params![id, bot_id, k],
            )?,
            None => self.conn.execute(
                "DELETE FROM memory WHERE account_id = ?1 AND bot_id = ?2",
                params![id, bot_id],
            )?,
        })
    }
}

fn user_version(conn: &Connection) -> Result<usize> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))? as usize)
}

fn migrate(conn: &Connection) -> Result<()> {
    let v = user_version(conn)?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(v) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration to v{}", i + 1))?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn validate_alias(a: &str) -> Result<&str> {
    let a = a.trim();
    if a.is_empty()
        || a.len() > 32
        || !a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        bail!("alias must be 1-32 chars of letters, digits, '-', '_' or '.'");
    }
    Ok(a)
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

    fn msg(role: Role, t: &str) -> Message {
        Message {
            role,
            text: t.into(),
            remote_id: None,
        }
    }

    fn rmsg(role: Role, t: &str, id: &str) -> Message {
        Message {
            role,
            text: t.into(),
            remote_id: Some(id.into()),
        }
    }

    fn login(tok: &str) -> StoredLogin {
        StoredLogin {
            method: LoginMethod::Gateway,
            session: Session::new("https://x.cursorvm.com", tok),
            cursor_token: None,
        }
    }

    fn store() -> (Store, tempfile::TempDir) {
        let d = tempfile::tempdir().unwrap();
        (Store::open(d.path().join("gb"), None).unwrap(), d)
    }

    #[test]
    fn migrates_and_permissions() {
        let (s, d) = store();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
        assert_eq!(s.account().alias, DEFAULT_ACCOUNT);
        drop(s);
        let s = Store::open(d.path().join("gb"), None).unwrap();
        assert_eq!(s.accounts().unwrap().len(), 1);
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

    fn v1_db(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let c = Connection::open(dir.join("context.db")).unwrap();
        c.execute_batch(V1).unwrap();
        c.execute_batch(
            "PRAGMA user_version = 1;
             INSERT INTO credentials(id, token, user_id, user_name, api_base) VALUES (1, 'tok', 'u1', 'me', 'http://old');
             INSERT INTO messages(bot_id, role, text) VALUES ('helper', 'user', 'hi'), ('helper', 'bot', 'yo');
             INSERT INTO memory(bot_id, key, value) VALUES ('coder', 'lang', 'rust');
             INSERT INTO prefs VALUES ('api_base_url', 'http://old'), ('paths.bots', '/b'), ('active_bot', 'helper'),
               ('ui.sidebar_width', '30'), ('legacy_imported', '1');",
        )
        .unwrap();
    }

    #[test]
    fn migration_from_v1_to_v3() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        v1_db(&dir);
        let s = Store::open(&dir, None).unwrap();
        assert_eq!(s.schema_version().unwrap(), 3);
        assert_eq!(s.account().alias, "default");
        assert!(
            s.load_login().unwrap().is_none(),
            "placeholder token dropped"
        );
        assert_eq!(
            s.messages("helper").unwrap(),
            vec![msg(Role::User, "hi"), msg(Role::Bot, "yo")]
        );
        assert_eq!(
            s.list_memory("coder").unwrap(),
            vec![("lang".into(), "rust".into())]
        );
        let c = s.load_config().unwrap();
        assert_eq!(c.active_bot.as_deref(), Some("helper"));
        assert_eq!(c.sidebar_width, 30);
        let keys: Vec<String> = s
            .list_prefs_scoped()
            .unwrap()
            .into_iter()
            .map(|(_, k, _)| k)
            .collect();
        assert!(
            !keys
                .iter()
                .any(|k| k == "api_base_url" || k.starts_with("paths.") || k == "legacy_imported"),
            "{keys:?}"
        );
        assert!(keys.iter().any(|k| k == "show_hidden_bots"));
    }

    #[test]
    fn migration_from_v2_keeps_accounts_and_chats() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        v1_db(&dir);
        {
            let c = Connection::open(dir.join("context.db")).unwrap();
            c.execute_batch(V2).unwrap();
            c.execute_batch(
                "PRAGMA user_version = 2;
                 INSERT INTO accounts(id, alias) VALUES (2, 'work');
                 INSERT INTO bots(account_id, bot_id, name) VALUES (2, 'b', 'B');
                 INSERT INTO messages(account_id, bot_id, role, text) VALUES (2, 'b', 'user', 'work msg');
                 INSERT INTO account_prefs VALUES (2, 'paths.send', '/s');",
            )
            .unwrap();
        }
        let mut s = Store::open(&dir, Some("work")).unwrap();
        assert_eq!(s.schema_version().unwrap(), 3);
        assert_eq!(s.messages("b").unwrap(), vec![msg(Role::User, "work msg")]);
        assert!(s.get_pref("paths.send").unwrap().is_none());
        s.use_account("default").unwrap();
        assert_eq!(s.messages("helper").unwrap().len(), 2);
    }

    #[test]
    fn accounts_are_strictly_isolated() {
        let (mut s, _d) = store();
        s.add_account("work").unwrap();
        s.save_login(&login("tokA")).unwrap();
        s.append_message("helper", &msg(Role::User, "secret A"))
            .unwrap();
        s.set_memory("helper", "k", "memA").unwrap();
        s.set_pref("default_bot", "helper").unwrap();
        s.set_pref("ui.sidebar_width", "40").unwrap(); // global

        s.use_account("work").unwrap();
        assert!(s.load_login().unwrap().is_none());
        assert!(s.messages("helper").unwrap().is_empty());
        assert!(s.list_memory("helper").unwrap().is_empty());
        assert!(s.bots().unwrap().is_empty());
        assert_eq!(s.load_config().unwrap().default_bot, None);
        assert_eq!(s.load_config().unwrap().sidebar_width, 40);
        s.save_login(&login("tokB")).unwrap();
        s.append_message("helper", &msg(Role::User, "secret B"))
            .unwrap();
        s.set_memory("helper", "k", "memB").unwrap();
        assert!(s.delete_credentials().unwrap());

        s.use_account("default").unwrap();
        assert_eq!(
            s.load_login().unwrap().unwrap().session.gateway_token,
            "tokA"
        );
        assert_eq!(
            s.messages("helper").unwrap(),
            vec![msg(Role::User, "secret A")]
        );
        assert_eq!(
            s.list_memory("helper").unwrap(),
            vec![("k".into(), "memA".into())]
        );
        assert_eq!(
            s.load_config().unwrap().default_bot.as_deref(),
            Some("helper")
        );

        s.remove_account("work").unwrap();
        let n: i64 = s
            .conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
        let n: i64 = s
            .conn
            .query_row("SELECT count(*) FROM account_prefs WHERE account_id NOT IN (SELECT id FROM accounts)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn account_management_rules() {
        let (mut s, d) = store();
        assert!(s.add_account("bad alias!").is_err());
        s.add_account("work").unwrap();
        assert!(s.add_account("WORK").is_err());
        assert!(s.remove_account("default").is_err());
        s.rename_account("default", "personal").unwrap();
        assert_eq!(s.account().alias, "personal");
        assert!(s.rename_account("work", "personal").is_err());
        s.use_account("work").unwrap();
        assert!(!s.is_startup_account());
        drop(s);
        let s = Store::open(d.path().join("gb"), None).unwrap();
        assert_eq!(s.account().alias, "work");
        assert!(s.is_startup_account());
        assert_eq!(
            Store::open(d.path().join("gb"), Some("personal"))
                .unwrap()
                .account()
                .alias,
            "personal"
        );
        assert!(Store::open(d.path().join("gb"), Some("nope")).is_err());
    }

    #[test]
    fn login_crud_roundtrips_headers_and_method() {
        let (s, _d) = store();
        assert!(s.load_login().unwrap().is_none());
        let l = StoredLogin {
            method: LoginMethod::Cursor,
            session: Session::new("https://x.cursorvm.com", "gw")
                .with_headers([("x-anyrun-network-token".to_string(), "n".to_string())].into()),
            cursor_token: Some("cur".into()),
        };
        s.save_login(&l).unwrap();
        assert_eq!(s.load_login().unwrap().unwrap(), l);
        s.save_login(&login("t2")).unwrap();
        assert_eq!(
            s.load_login().unwrap().unwrap().method,
            LoginMethod::Gateway
        );
        assert!(s.delete_credentials().unwrap());
        assert!(!s.delete_credentials().unwrap());
    }

    #[test]
    fn merge_remote_dedupes_and_adopts_pending() {
        let (s, _d) = store();
        s.append_message(
            "a",
            &Message {
                role: Role::User,
                text: "hi".into(),
                remote_id: Some("pending:n1".into()),
            },
        )
        .unwrap();
        let added = s
            .merge_remote(
                "a",
                &[
                    rmsg(Role::User, "hi", "e1"),
                    rmsg(Role::Bot, "hello", "e2"),
                    msg(Role::Bot, "noid"),
                ],
            )
            .unwrap();
        assert_eq!(
            added,
            vec![rmsg(Role::Bot, "hello", "e2"), msg(Role::Bot, "noid")]
        );
        let again = s
            .merge_remote(
                "a",
                &[
                    rmsg(Role::User, "hi", "e1"),
                    rmsg(Role::Bot, "hello", "e2"),
                    msg(Role::Bot, "noid"),
                ],
            )
            .unwrap();
        assert!(again.is_empty());
        let all = s.messages("a").unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].remote_id.as_deref(), Some("e1"));
        s.append_message(
            "a",
            &Message {
                role: Role::User,
                text: "x".into(),
                remote_id: Some("pending:n2".into()),
            },
        )
        .unwrap();
        s.delete_pending("a", "n2").unwrap();
        assert_eq!(s.messages("a").unwrap().len(), 3);
    }

    #[test]
    fn messages_memory_bots_persist() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = Store::open(d.path(), None).unwrap();
            s.save_bots(&[Bot {
                id: "a".into(),
                name: "A".into(),
                description: String::new(),
                hidden: true,
                is_group: false,
            }])
            .unwrap();
            s.append_message("a", &msg(Role::User, "hi")).unwrap();
            s.set_memory("a", "tone", "brief").unwrap();
            s.set_memory("a", "tone", "terse").unwrap();
        }
        let s = Store::open(d.path(), None).unwrap();
        assert!(s.bots().unwrap()[0].hidden);
        assert_eq!(s.messages("a").unwrap().len(), 1);
        assert_eq!(
            s.list_memory("a").unwrap(),
            vec![("tone".into(), "terse".into())]
        );
        assert_eq!(s.clear_memory("a", None).unwrap(), 1);
        s.clear_messages("a").unwrap();
        assert!(s.messages("a").unwrap().is_empty());
        assert!(s.set_pref("active_account", "x").is_err());
    }
}
