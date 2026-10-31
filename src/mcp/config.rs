//! `[mcp.server]` settings (`<data dir>/mcp.toml`), the HTTP bearer token and the audit log.
//!
//! The token lives in `<data dir>/mcp-http-token` (mode 0600 on Unix). It is only ever
//! printed by `grokbot-tui mcp token`, never logged. The audit log records which tool ran
//! for which bot and whether it worked: never message bodies, card answers or secrets.
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// File name of the MCP settings in the data dir.
pub const CONFIG_FILE: &str = "mcp.toml";
/// File name of the HTTP bearer token in the data dir.
pub const TOKEN_FILE: &str = "mcp-http-token";
/// Default `--http` address.
pub const DEFAULT_HTTP_ADDR: &str = "127.0.0.1:8765";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Bot ids or names MCP clients may use. Empty = all bots.
    pub allowed_bots: Vec<String>,
    /// Refuse `send_message` and `answer_card`.
    pub read_only: bool,
    /// Address for `mcp serve --http` without an address. Loopback only.
    pub http_addr: String,
    /// Write `~/.cache/grokbot-tui/mcp-audit.log`.
    pub audit: bool,
    /// Where `download_attachment` saves files when no `dest_dir` is given
    /// (default: the Downloads folder, else `<data dir>/downloads`).
    pub download_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            allowed_bots: vec![],
            read_only: false,
            http_addr: DEFAULT_HTTP_ADDR.into(),
            audit: true,
            download_dir: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct File {
    mcp: McpTable,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct McpTable {
    server: ServerConfig,
}

impl ServerConfig {
    /// Parse the text of `mcp.toml`. Unknown keys under `[mcp.server]` are an error so a
    /// typo in `allowed_bots` can't silently open every bot.
    pub fn parse(text: &str) -> Result<Self> {
        let f: File = toml::from_str(text).context("mcp.toml")?;
        Ok(f.mcp.server)
    }

    /// `<dir>/mcp.toml`, or the defaults when it doesn't exist.
    pub fn load(dir: &Path) -> Result<Self> {
        let p = dir.join(CONFIG_FILE);
        match std::fs::read_to_string(&p) {
            Ok(t) => Self::parse(&t).with_context(|| p.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| p.display().to_string()),
        }
    }

    /// Whether MCP clients may use this bot (matched by id, or by name ignoring case).
    pub fn allows(&self, id: &str, name: &str) -> bool {
        self.allowed_bots.is_empty()
            || self
                .allowed_bots
                .iter()
                .any(|a| a == id || a.eq_ignore_ascii_case(name))
    }
}

/// Parse an `--http` address and refuse anything but loopback.
pub fn parse_http_addr(s: &str) -> Result<SocketAddr> {
    let s = s.trim();
    let addr: SocketAddr = match s.parse() {
        Ok(a) => a,
        Err(_) => match s.strip_prefix("localhost:") {
            Some(port) => format!("127.0.0.1:{port}").parse()?,
            None => match s.parse::<u16>() {
                Ok(port) => SocketAddr::from(([127, 0, 0, 1], port)),
                Err(_) => bail!("--http {s:?}: expected HOST:PORT, e.g. {DEFAULT_HTTP_ADDR}"),
            },
        },
    };
    if !addr.ip().is_loopback() {
        bail!("--http {addr}: only loopback addresses (127.0.0.1, [::1]) are allowed");
    }
    Ok(addr)
}

/// A new random token: 64 hex chars from two v4 UUIDs (OS randomness).
fn new_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// The bearer token, generated on first use (or when `rotate`). Never log the result.
pub fn token(dir: &Path, rotate: bool) -> Result<String> {
    let p = dir.join(TOKEN_FILE);
    if !rotate
        && let Ok(t) = std::fs::read_to_string(&p)
        && !t.trim().is_empty()
    {
        return Ok(t.trim().to_string());
    }
    let t = new_token();
    std::fs::create_dir_all(dir)?;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&p).with_context(|| p.display().to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(t.as_bytes())?;
    Ok(t)
}

/// Compare two tokens without stopping at the first different byte.
pub fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Append-only audit log. One line per tool call: time, transport, tool, bot, outcome.
pub struct Audit {
    file: Option<Mutex<std::fs::File>>,
    transport: &'static str,
}

impl Audit {
    pub fn default_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("grokbot-tui").join("mcp-audit.log"))
    }
    pub fn open(path: Option<PathBuf>, transport: &'static str) -> Self {
        let file = path.and_then(|p| {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).ok()?;
            }
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .ok()
                .map(Mutex::new)
        });
        Self { file, transport }
    }
    pub fn off() -> Self {
        Self {
            file: None,
            transport: "",
        }
    }
    /// `detail` must never carry message text, answers or secrets (kinds and actions only).
    pub fn record(&self, tool: &str, bot: Option<&str>, detail: &str, outcome: &str) {
        let Some(f) = &self.file else { return };
        let line = audit_line(
            &chrono_now(),
            self.transport,
            tool,
            bot.unwrap_or("-"),
            detail,
            outcome,
        );
        if let Ok(mut f) = f.lock() {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

pub fn audit_line(
    ts: &str,
    transport: &str,
    tool: &str,
    bot: &str,
    detail: &str,
    outcome: &str,
) -> String {
    let clean = |s: &str| s.replace(['\n', '\r', '\t'], " ");
    format!(
        "{ts}\t{transport}\t{}\tbot={}\t{}\t{}\n",
        clean(tool),
        clean(bot),
        if detail.is_empty() {
            "-".into()
        } else {
            clean(detail)
        },
        clean(outcome)
    )
}

/// UTC timestamp (RFC 3339, seconds) without a date crate.
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing_or_empty() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            ServerConfig::load(d.path()).unwrap(),
            ServerConfig::default()
        );
        assert_eq!(ServerConfig::parse("").unwrap(), ServerConfig::default());
        let c = ServerConfig::default();
        assert!(c.allows("any", "Any") && !c.read_only && c.audit);
        assert_eq!(c.http_addr, DEFAULT_HTTP_ADDR);
    }

    #[test]
    fn parses_server_table() {
        let c = ServerConfig::parse(
            "[mcp.server]\nallowed_bots = [\"Grokbot-cli\", \"abc-123\"]\nread_only = true\nhttp_addr = \"127.0.0.1:9000\"\naudit = false\n",
        )
        .unwrap();
        assert!(c.read_only && !c.audit);
        assert_eq!(c.http_addr, "127.0.0.1:9000");
        assert!(c.allows("x", "grokbot-CLI"));
        assert!(c.allows("abc-123", "Other"));
        assert!(!c.allows("zzz", "Other"));
    }

    #[test]
    fn unknown_key_is_an_error() {
        assert!(ServerConfig::parse("[mcp.server]\nallowed_bot = [\"x\"]\n").is_err());
    }

    #[test]
    fn http_addr_is_loopback_only() {
        assert_eq!(
            parse_http_addr("127.0.0.1:8765").unwrap().to_string(),
            "127.0.0.1:8765"
        );
        assert_eq!(parse_http_addr("localhost:99").unwrap().port(), 99);
        assert_eq!(
            parse_http_addr("9001").unwrap().to_string(),
            "127.0.0.1:9001"
        );
        assert!(parse_http_addr("[::1]:1234").is_ok());
        assert!(parse_http_addr("0.0.0.0:8765").is_err());
        assert!(parse_http_addr("192.168.1.2:8765").is_err());
        assert!(parse_http_addr("nope").is_err());
    }

    #[test]
    fn token_is_stable_private_and_rotates() {
        let d = tempfile::tempdir().unwrap();
        let a = token(d.path(), false).unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(token(d.path(), false).unwrap(), a);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(d.path().join(TOKEN_FILE)).unwrap();
            assert_eq!(m.permissions().mode() & 0o777, 0o600);
        }
        let b = token(d.path(), true).unwrap();
        assert_ne!(a, b);
        assert!(token_eq(&b, &token(d.path(), false).unwrap()));
        assert!(!token_eq(&a, &b) && !token_eq("ab", "abc"));
    }

    #[test]
    fn audit_lines_are_one_line() {
        let l = audit_line("T", "stdio", "send_message", "b\n1", "", "ok");
        assert_eq!(l, "T\tstdio\tsend_message\tbot=b 1\t-\tok\n");
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a").join("audit.log");
        let a = Audit::open(Some(p.clone()), "http");
        a.record("list_bots", None, "", "ok");
        let t = std::fs::read_to_string(p).unwrap();
        assert!(t.ends_with("\thttp\tlist_bots\tbot=-\t-\tok\n"), "{t}");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_757), (2026, 10, 31));
    }
}
