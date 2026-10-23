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
/// v3 -> v4: allow login method 'desktop' and remember which desktop entry was chosen.
const V4: &str = "CREATE TABLE credentials_v4 (
        account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
        method TEXT NOT NULL CHECK (method IN ('gateway', 'cursor', 'desktop')),
        gateway_url TEXT NOT NULL,
        gateway_token TEXT NOT NULL,
        gateway_headers TEXT NOT NULL DEFAULT '{}',
        cursor_token TEXT,
        desktop_entry TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    INSERT INTO credentials_v4(account_id, method, gateway_url, gateway_token, gateway_headers, cursor_token, created_at, updated_at)
        SELECT account_id, method, gateway_url, gateway_token, gateway_headers, cursor_token, created_at, updated_at FROM credentials;
    DROP TABLE credentials;
    ALTER TABLE credentials_v4 RENAME TO credentials;";

/// v4 -> v5: explicit display order (`ord`) following the gateway transcript order,
/// instead of insertion order.
const V5: &str = "ALTER TABLE messages ADD COLUMN ord REAL;
    UPDATE messages SET ord = id;
    CREATE INDEX messages_ord ON messages(account_id, bot_id, ord);";
/// v5 -> v6: rows inserted without `ord` (e.g. by an older grokbot-tui still running
/// against this DB) get one automatically, so they sort last instead of first.
const V6: &str = "UPDATE messages SET ord = (SELECT COALESCE(MAX(m.ord), 0) FROM messages m
        WHERE m.account_id = messages.account_id AND m.bot_id = messages.bot_id) + id
      WHERE ord IS NULL;
    CREATE TRIGGER messages_ord_default AFTER INSERT ON messages WHEN NEW.ord IS NULL
    BEGIN
      UPDATE messages SET ord = (SELECT COALESCE(MAX(ord), 0) + 1 FROM messages
        WHERE account_id = NEW.account_id AND bot_id = NEW.bot_id AND id != NEW.id)
      WHERE id = NEW.id;
    END;";
/// v6 -> v7: per-account prompt history for Up/Down and Ctrl-R.
const V7: &str = "CREATE TABLE prompt_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        text TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX prompt_history_account ON prompt_history(account_id, id DESC);";

const V8: &str = "ALTER TABLE bots ADD COLUMN team INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE bots ADD COLUMN viewer_session_id TEXT;";

/// v8 -> v9: remember which bot each prompt went to (shown in `/history`).
const V9: &str = "ALTER TABLE prompt_history ADD COLUMN bot TEXT;";

const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7, V8, V9];

/// One remembered prompt (newest-first lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptEntry {
    pub text: String,
    /// Display name of the bot that was active when it was sent, if any.
    pub bot: Option<String>,
    /// UTC `YYYY-MM-DD HH:MM:SS`.
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMethod {
    /// Gateway URL + gateway token pasted by the user.
    Gateway,
    /// Cursor access token exchanged via EnsureSandBox (re-exchanged on 401).
    Cursor,
    /// Read from the desktop app's saved session (`/login desktop`; re-read on 401).
    Desktop,
}

impl LoginMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            LoginMethod::Gateway => "gateway",
            LoginMethod::Cursor => "cursor",
            LoginMethod::Desktop => "desktop",
        }
    }
}

/// A stored login for one account.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLogin {
    pub method: LoginMethod,
    pub session: Session,
    pub cursor_token: Option<String>,
    /// Desktop descriptor entry chosen with `/login desktop <entry>` (v2 multi-entry files).
    pub desktop_entry: Option<String>,
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
    /// Account `GROKBOT_TUI_TOKEN` applies to (the one selected at startup).
    env_account: Option<i64>,
}

impl Store {
    /// Default data directory (`~/.grokbot-tui`), moving a pre-1.2 `~/.gb` there first.
    /// Returns the directory and an optional notice for the user.
    pub fn default_dir() -> Result<(PathBuf, Option<String>)> {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        let r = crate::datadir::resolve(&home);
        Ok((r.dir, r.notice))
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

    /// True when the active account is the one selected at startup (where `GROKBOT_TUI_TOKEN` applies).
    pub fn is_startup_account(&self) -> bool {
        self.env_account == Some(self.account.id)
    }

    pub fn load_login(&self) -> Result<Option<StoredLogin>> {
        let row = self
            .conn
            .query_row(
                "SELECT method, gateway_url, gateway_token, gateway_headers, cursor_token, desktop_entry FROM credentials WHERE account_id = ?1",
                [self.account.id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(m, url, tok, hdrs, cur, de)| StoredLogin {
            method: match m.as_str() {
                "cursor" => LoginMethod::Cursor,
                "desktop" => LoginMethod::Desktop,
                _ => LoginMethod::Gateway,
            },
            desktop_entry: de,
            session: Session {
                gateway_url: url,
                gateway_token: tok,
                headers: serde_json::from_str(&hdrs).unwrap_or_default(),
            },
            cursor_token: cur,
        }))
    }

    pub fn save_login(&self, l: &StoredLogin) -> Result<()> {
        let method = l.method.as_str();
        self.conn.execute(
            "INSERT INTO credentials(account_id, method, gateway_url, gateway_token, gateway_headers, cursor_token, desktop_entry)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account_id) DO UPDATE SET method = excluded.method, gateway_url = excluded.gateway_url,
               gateway_token = excluded.gateway_token, gateway_headers = excluded.gateway_headers,
               cursor_token = excluded.cursor_token, desktop_entry = excluded.desktop_entry, updated_at = datetime('now')",
            params![
                self.account.id,
                method,
                l.session.gateway_url,
                l.session.gateway_token,
                serde_json::to_string(&l.session.headers)?,
                l.cursor_token,
                l.desktop_entry
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
                "INSERT INTO bots(account_id, bot_id, name, description, hidden, is_group, team, viewer_session_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_id, bot_id) DO UPDATE SET name = excluded.name, description = excluded.description,
                   hidden = excluded.hidden, is_group = excluded.is_group,
                   team = excluded.team, viewer_session_id = excluded.viewer_session_id",
                params![
                    self.account.id,
                    b.id,
                    b.name,
                    b.description,
                    b.hidden,
                    b.is_group,
                    b.team,
                    b.viewer_session_id
                ],
            )?;
        }
        Ok(())
    }

    pub fn bots(&self) -> Result<Vec<Bot>> {
        let mut st = self.conn.prepare(
            "SELECT bot_id, name, description, hidden, is_group, team, viewer_session_id
             FROM bots WHERE account_id = ?1 ORDER BY rowid",
        )?;
        let rows = st.query_map([self.account.id], |r| {
            Ok(Bot {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                hidden: r.get(3)?,
                is_group: r.get(4)?,
                team: r.get(5)?,
                viewer_session_id: r.get(6)?,
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
            "INSERT INTO messages(account_id, bot_id, role, text, remote_id, ord)
             VALUES (?1, ?2, ?3, ?4, ?5,
               (SELECT COALESCE(MAX(ord), 0) + 1 FROM messages WHERE account_id = ?1 AND bot_id = ?2))",
            params![self.account.id, bot_id, role, m.text, m.remote_id],
        )?;
        Ok(())
    }

    pub fn messages(&self, bot_id: &str) -> Result<Vec<Message>> {
        let mut st = self.conn.prepare(
            "SELECT role, text, remote_id FROM messages WHERE account_id = ?1 AND bot_id = ?2 ORDER BY ord IS NULL, ord, id",
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
    ///
    /// `msgs` must be in gateway transcript order. Display order (`ord`) follows that
    /// order: new entries are placed between the neighbours they arrived with. When
    /// `from_start` is true (`msgs` is the whole history), every row's `ord` is
    /// rewritten to match the gateway exactly, which also repairs older misordering.
    pub fn merge_remote(
        &self,
        bot_id: &str,
        msgs: &[Message],
        from_start: bool,
    ) -> Result<Vec<Message>> {
        let id = self.account.id;
        let mut added = vec![];
        // (row id, ord before this merge; None for rows inserted now)
        let mut window: Vec<(i64, Option<f64>)> = vec![];
        let row_of =
            |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<Option<(i64, Option<f64>)>> {
                Ok(self
                    .conn
                    .query_row(sql, p, |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?)
            };
        for m in msgs {
            let role = if m.role == Role::User { "user" } else { "bot" };
            let found = if let Some(rid) = &m.remote_id {
                match row_of(
                    "SELECT id, ord FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND remote_id = ?3",
                    &[&id, &bot_id, rid],
                )? {
                    Some(r) => Some(r),
                    None if m.role == Role::User => {
                        let pending = row_of(
                            "SELECT id, ord FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND text = ?3
                               AND remote_id LIKE 'pending:%' ORDER BY id LIMIT 1",
                            &[&id, &bot_id, &m.text],
                        )?;
                        if let Some((rowid, _)) = pending {
                            self.conn.execute(
                                "UPDATE messages SET remote_id = ?2 WHERE id = ?1",
                                params![rowid, rid],
                            )?;
                            // A placeholder takes its real place in the transcript.
                            Some((rowid, None))
                        } else {
                            None
                        }
                    }
                    None => None,
                }
            } else {
                row_of(
                    "SELECT id, ord FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND role = ?3 AND text = ?4",
                    &[&id, &bot_id, &role, &m.text],
                )?
            };
            let entry = match found {
                Some(r) => r,
                None => {
                    self.append_message(bot_id, m)?;
                    added.push(m.clone());
                    (self.conn.last_insert_rowid(), None)
                }
            };
            if !window.iter().any(|(r, _)| *r == entry.0) {
                window.push(entry);
            }
        }
        if from_start {
            self.reorder_full(bot_id, &window)?;
        } else {
            self.place_window(bot_id, &window)?;
        }
        Ok(added)
    }

    /// Give rows without a known position an `ord` between their window neighbours.
    fn place_window(&self, bot_id: &str, window: &[(i64, Option<f64>)]) -> Result<()> {
        let new_ids: Vec<i64> = window
            .iter()
            .filter(|w| w.1.is_none())
            .map(|w| w.0)
            .collect();
        if new_ids.is_empty() {
            return Ok(());
        }
        let max_known: f64 = self.conn.query_row(
            &format!(
                "SELECT COALESCE(MAX(ord), 0) FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND id NOT IN ({})",
                new_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
            ),
            params![self.account.id, bot_id],
            |r| r.get(0),
        )?;
        let mut i = 0;
        let mut prev: Option<f64> = None;
        while i < window.len() {
            if let Some(o) = window[i].1 {
                prev = Some(prev.map_or(o, |p| p.max(o)));
                i += 1;
                continue;
            }
            let start = i;
            while i < window.len() && window[i].1.is_none() {
                i += 1;
            }
            let next = window.get(i).and_then(|w| w.1);
            let n = (i - start) as f64;
            let mut last = prev.unwrap_or(max_known);
            for (k, (rowid, _)) in window[start..i].iter().enumerate() {
                let k = k as f64 + 1.0;
                let ord = match (prev, next) {
                    (Some(a), Some(b)) if b > a => a + (b - a) * k / (n + 1.0),
                    (Some(a), Some(_)) => a + k * 1e-3,
                    (Some(a), None) => a + k,
                    (None, Some(b)) => b - (n + 1.0 - k),
                    (None, None) => max_known + k,
                };
                self.conn.execute(
                    "UPDATE messages SET ord = ?2 WHERE id = ?1",
                    params![rowid, ord],
                )?;
                last = ord;
            }
            prev = Some(last);
        }
        Ok(())
    }

    /// Rewrite `ord` for the whole bot: rows not in the window (older than it) first,
    /// then the window in gateway order, then still-pending local rows.
    fn reorder_full(&self, bot_id: &str, window: &[(i64, Option<f64>)]) -> Result<()> {
        let mut st = self.conn.prepare(
            "SELECT id, COALESCE(remote_id LIKE 'pending:%', 0) FROM messages
             WHERE account_id = ?1 AND bot_id = ?2 ORDER BY ord IS NULL, ord, id",
        )?;
        let all: Vec<(i64, bool)> = st
            .query_map(params![self.account.id, bot_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let in_win: std::collections::HashSet<i64> = window.iter().map(|w| w.0).collect();
        let order: Vec<i64> = all
            .iter()
            .filter(|(r, p)| !in_win.contains(r) && !p)
            .map(|(r, _)| *r)
            .chain(window.iter().map(|w| w.0))
            .chain(
                all.iter()
                    .filter(|(r, p)| !in_win.contains(r) && *p)
                    .map(|(r, _)| *r),
            )
            .collect();
        let tx = self.conn.unchecked_transaction()?;
        for (i, rowid) in order.iter().enumerate() {
            tx.execute(
                "UPDATE messages SET ord = ?2 WHERE id = ?1",
                params![rowid, (i + 1) as f64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Cheap change stamp for a bot's messages: (row count, max row id).
    /// Includes an order checksum so re-ordering is noticed too.
    pub fn message_stamp(&self, bot_id: &str) -> Result<(i64, i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(id), 0),
                    COALESCE(SUM(CAST(ord * 1000 AS INTEGER) * (id % 9973)), 0) FROM messages WHERE account_id = ?1 AND bot_id = ?2",
            params![self.account.id, bot_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    /// Delete a locally pending message (e.g. its send was rejected).
    pub fn delete_pending(&self, bot_id: &str, nonce: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND bot_id = ?2 AND remote_id = ?3",
            params![self.account.id, bot_id, format!("pending:{nonce}")],
        )?;
        Ok(())
    }

    #[cfg(test)]
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

    /// Newest-first prompt history for the active account, at most `limit` entries.
    pub fn prompt_entries(&self, limit: usize) -> Result<Vec<PromptEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT text, bot, created_at FROM prompt_history WHERE account_id = ?1
             ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![self.account.id, limit as i64], |r| {
            Ok(PromptEntry {
                text: r.get(0)?,
                bot: r.get(1)?,
                at: r.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Newest-first prompt texts for the active account (capped).
    #[cfg(test)]
    pub fn prompt_history(&self, limit: usize) -> Result<Vec<String>> {
        Ok(self
            .prompt_entries(limit)?
            .into_iter()
            .map(|e| e.text)
            .collect())
    }

    /// Append a prompt (sent to `bot`, if any), skipping an exact duplicate of the previous
    /// entry. Trims the account's history to the newest `max` rows. Returns whether a row
    /// was added.
    pub fn push_prompt(&self, text: &str, bot: Option<&str>, max: usize) -> Result<bool> {
        let text = text.trim_end();
        if text.trim().is_empty() {
            return Ok(false);
        }
        let last: Option<String> = self
            .conn
            .query_row(
                "SELECT text FROM prompt_history WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
                params![self.account.id],
                |r| r.get(0),
            )
            .optional()?;
        if last.as_deref() == Some(text) {
            return Ok(false);
        }
        self.conn.execute(
            "INSERT INTO prompt_history(account_id, text, bot) VALUES (?1, ?2, ?3)",
            params![self.account.id, text, bot],
        )?;
        self.conn.execute(
            "DELETE FROM prompt_history WHERE account_id = ?1 AND id <= (
                SELECT id FROM prompt_history WHERE account_id = ?1
                ORDER BY id DESC LIMIT 1 OFFSET ?2
            )",
            params![self.account.id, max.max(1) as i64],
        )?;
        Ok(true)
    }

    /// Forget the active account's prompt history. Returns how many prompts were removed.
    pub fn clear_prompt_history(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM prompt_history WHERE account_id = ?1",
            params![self.account.id],
        )?)
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
            desktop_entry: None,
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
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
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
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
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
            desktop_entry: None,
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
    fn desktop_login_roundtrips_entry() {
        let (s, _d) = store();
        let d = StoredLogin {
            method: LoginMethod::Desktop,
            desktop_entry: Some("work".into()),
            ..login("d")
        };
        s.save_login(&d).unwrap();
        assert_eq!(s.load_login().unwrap().unwrap(), d);
    }

    #[test]
    fn migration_v3_to_v4_keeps_credentials() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("gb");
        v1_db(&dir);
        {
            let c = Connection::open(dir.join("context.db")).unwrap();
            c.execute_batch(V2).unwrap();
            c.execute_batch(V3).unwrap();
            c.execute_batch(
                "PRAGMA user_version = 3;
                 INSERT INTO credentials(account_id, method, gateway_url, gateway_token, cursor_token)
                   VALUES (1, 'cursor', 'https://x.cursorvm.com', 'gw', 'cur');",
            )
            .unwrap();
        }
        let s = Store::open(&dir, None).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
        let l = s.load_login().unwrap().unwrap();
        assert_eq!(l.method, LoginMethod::Cursor);
        assert_eq!(l.cursor_token.as_deref(), Some("cur"));
        assert_eq!(l.desktop_entry, None);
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
                false,
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
                false,
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
                team: false,
                viewer_session_id: None,
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

    fn texts(s: &Store, bot: &str) -> Vec<String> {
        s.messages(bot)
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect()
    }

    #[test]
    fn history_after_tail_is_ordered_by_gateway_not_insertion() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("gb"), None).unwrap();
        // A tail poll stored only the newest entry first…
        s.merge_remote("b", &[rmsg(Role::User, "u3", "t3u")], false)
            .unwrap();
        // …then the full history arrives (previously appended AFTER u3).
        let all = [
            rmsg(Role::User, "u1", "t1u"),
            rmsg(Role::Bot, "b1", "t1s0"),
            rmsg(Role::User, "u2", "t2u"),
            rmsg(Role::Bot, "b2", "t2s0"),
            rmsg(Role::User, "u3", "t3u"),
        ];
        s.merge_remote("b", &all, true).unwrap();
        assert_eq!(texts(&s, "b"), ["u1", "b1", "u2", "b2", "u3"]);
    }

    #[test]
    fn late_entry_in_tail_lands_between_neighbours() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("gb"), None).unwrap();
        s.merge_remote(
            "b",
            &[
                rmsg(Role::User, "q42", "t42u"),
                rmsg(Role::Bot, "a42", "t42s0"),
                rmsg(Role::User, "q43", "t43u"),
            ],
            false,
        )
        .unwrap();
        // The gateway now shows a late t42s1 before t43u.
        s.merge_remote(
            "b",
            &[
                rmsg(Role::Bot, "a42", "t42s0"),
                rmsg(Role::Bot, "a42b", "t42s1"),
                rmsg(Role::User, "q43", "t43u"),
                rmsg(Role::Bot, "a43", "t43s0"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(texts(&s, "b"), ["q42", "a42", "a42b", "q43", "a43"]);
    }

    #[test]
    fn pending_placeholder_moves_to_its_real_place_and_full_repairs_order() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("gb"), None).unwrap();
        s.merge_remote(
            "b",
            &[rmsg(Role::User, "q1", "t1u"), rmsg(Role::Bot, "a1", "t1s0")],
            false,
        )
        .unwrap();
        s.append_message(
            "b",
            &Message {
                role: Role::User,
                text: "mine".into(),
                remote_id: Some("pending:n".into()),
            },
        )
        .unwrap();
        // Desktop message arrived on the gateway before mine; then mine; then reply.
        s.merge_remote(
            "b",
            &[
                rmsg(Role::Bot, "a1", "t1s0"),
                rmsg(Role::User, "desk", "t2u"),
                rmsg(Role::Bot, "a2", "t2s0"),
                rmsg(Role::User, "mine", "t3u"),
                rmsg(Role::Bot, "a3", "t3s0"),
            ],
            false,
        )
        .unwrap();
        assert_eq!(texts(&s, "b"), ["q1", "a1", "desk", "a2", "mine", "a3"]);
        assert_eq!(
            s.messages("b").unwrap().len(),
            6,
            "no duplicate of the pending row"
        );
        // Corrupt the order, then a full fetch repairs it.
        s.conn.execute("UPDATE messages SET ord = -id", []).unwrap();
        let full: Vec<Message> = ["q1", "a1", "desk", "a2", "mine", "a3"]
            .iter()
            .zip(["t1u", "t1s0", "t2u", "t2s0", "t3u", "t3s0"])
            .map(|(t, r)| {
                rmsg(
                    if r.ends_with('u') {
                        Role::User
                    } else {
                        Role::Bot
                    },
                    t,
                    r,
                )
            })
            .collect();
        s.merge_remote("b", &full, true).unwrap();
        assert_eq!(texts(&s, "b"), ["q1", "a1", "desk", "a2", "mine", "a3"]);
    }

    #[test]
    fn rows_inserted_without_ord_sort_last() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(d.path().join("gb"), None).unwrap();
        s.merge_remote(
            "b",
            &[rmsg(Role::User, "q", "t1u"), rmsg(Role::Bot, "a", "t1s0")],
            false,
        )
        .unwrap();
        // Simulate an older build inserting without `ord`.
        s.conn
            .execute(
                "INSERT INTO messages(account_id, bot_id, role, text, remote_id) VALUES (?1, 'b', 'bot', 'newest', 't1s1')",
                params![s.account.id],
            )
            .unwrap();
        assert_eq!(texts(&s, "b"), ["q", "a", "newest"]);
    }
}

#[cfg(test)]
mod prompt_history_store_tests {
    use super::*;

    #[test]
    fn prompts_persist_across_reopen_dedupe_trim_and_stay_per_account() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = Store::open(d.path(), None).unwrap();
            assert!(s.push_prompt("a", Some("Chomp"), 3).unwrap());
            assert!(
                !s.push_prompt("a", Some("Chomp"), 3).unwrap(),
                "consecutive dup"
            );
            assert!(!s.push_prompt("   ", None, 3).unwrap(), "blank");
            for p in ["b", "c", "d"] {
                s.push_prompt(p, None, 3).unwrap();
            }
        }
        let mut s = Store::open(d.path(), None).unwrap();
        assert_eq!(
            s.prompt_history(100).unwrap(),
            ["d", "c", "b"],
            "trimmed to 3"
        );
        s.push_prompt("a", Some("Chomp"), 100).unwrap();
        let e = s.prompt_entries(1).unwrap();
        assert_eq!(e[0].bot.as_deref(), Some("Chomp"));
        s.add_account("work").unwrap();
        s.use_account("work").unwrap();
        assert!(
            s.prompt_history(100).unwrap().is_empty(),
            "accounts stay separate"
        );
        s.push_prompt("w", None, 100).unwrap();
        s.use_account("default").unwrap();
        assert_eq!(s.clear_prompt_history().unwrap(), 4);
        s.use_account("work").unwrap();
        assert_eq!(
            s.prompt_history(100).unwrap(),
            ["w"],
            "clear is per account"
        );
    }

    #[test]
    fn v8_history_rows_survive_the_v9_migration() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = Store::open(d.path(), None).unwrap();
            s.push_prompt("old prompt", None, 100).unwrap();
            // Roll the schema back to v8 (no `bot` column), as an older release left it.
            s.conn
                .execute_batch(
                    "ALTER TABLE prompt_history DROP COLUMN bot; PRAGMA user_version = 8;",
                )
                .unwrap();
        }
        let s = Store::open(d.path(), None).unwrap();
        assert_eq!(s.schema_version().unwrap(), MIGRATIONS.len());
        let e = s.prompt_entries(10).unwrap();
        assert_eq!(e[0].text, "old prompt");
        assert_eq!(e[0].bot, None);
    }
}
