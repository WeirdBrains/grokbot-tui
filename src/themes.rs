//! Theme files: `<data dir>/themes/<id>.toml`.
//!
//! Themes are read only from the user's themes dir; there is no theme inside the binary and
//! no system-wide theme dir. When that dir has no themes, the app downloads the repo's
//! `themes/default.toml` from the git tag matching its version ([`download`]) and writes it
//! there. If the selected theme is missing or broken, the app falls back to plain terminal
//! colors ([`Theme::unstyled`]) and tells the user what to fix.
//!
//! Format (schema 1), see also README "Themes":
//!
//! ```toml
//! schema = 1
//! name = "Default"
//! [dark.colors]
//! text = "252"            # 0-255, "#rrggbb", "#rgb", an ANSI name or "reset"
//! [dark.styles]
//! brand = { fg = "@on_accent", bg = "@accent", bold = true }   # "@color" = palette
//! [light.colors]
//! ...
//! ```
//!
//! Parsing is forgiving per key: a missing color is `reset`, a missing style uses its
//! starter rule, and bad or unknown keys become warnings. Only a file that isn't valid TOML,
//! is too big, has no `[dark]`/`[light]` section or comes from a newer schema is refused.
use crate::theme::{
    ATTRS, COLOR_SLOTS, ColorRef, N_COLORS, STYLE_SLOTS, StyleSpec, Theme, VariantSpec, color_slot,
    starter_style, style_slot,
};
use ratatui::style::Color;
use std::path::{Path, PathBuf};

/// The theme format version this build reads and writes.
pub const SCHEMA: i64 = 1;
/// Theme files larger than this are refused.
pub const MAX_BYTES: usize = 64 * 1024;
/// Theme directory name inside the data dir.
pub const DIR_NAME: &str = "themes";
/// The theme selected when the `ui.theme` pref is unset.
pub const DEFAULT_ID: &str = "default";
/// Env var that turns off downloading the default theme (also pref `ui.theme_download`).
pub const NO_DOWNLOAD_ENV: &str = "GROKBOT_TUI_NO_THEME_DOWNLOAD";
/// Give up on a theme download after this long (startup never waits for it).
pub const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The shipped `default.toml` at the git tag of a release, e.g. `v1.6.0`.
pub fn release_url(version: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v{version}/themes/default.toml"
    )
}

/// The shipped `default.toml` matching this build.
pub fn default_url() -> String {
    release_url(env!("CARGO_PKG_VERSION"))
}

/// Whether an env value turns the download off (`1`, `true`, `yes`, `on`; empty / `0` /
/// `false` / `no` / `off` don't).
pub fn opted_out(env_value: Option<&str>) -> bool {
    env_value.is_some_and(|v| {
        !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        )
    })
}

/// Fetch a theme file over HTTPS (with `DOWNLOAD_TIMEOUT`), refusing anything larger than
/// `MAX_BYTES` or that doesn't parse as a theme. Returns the file's exact text.
pub async fn download(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .connect_timeout(std::time::Duration::from_secs(3))
        .user_agent(concat!("grokbot-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().await.map_err(|e| describe(&e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
        return Err(format!("larger than {} KiB", MAX_BYTES / 1024));
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| describe(&e))? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            return Err(format!("larger than {} KiB", MAX_BYTES / 1024));
        }
    }
    let text = String::from_utf8(body).map_err(|_| "not UTF-8 text".to_string())?;
    parse(DEFAULT_ID, &text).map_err(|e| format!("not a usable theme: {e}"))?;
    Ok(text)
}

fn describe(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        format!("timed out after {}s", DOWNLOAD_TIMEOUT.as_secs())
    } else if e.is_connect() {
        "couldn't connect (offline?)".into()
    } else {
        // Without the URL (it's shown separately) and without the error chain's noise.
        e.to_string().replace(
            &format!(" for url ({})", e.url().map(|u| u.as_str()).unwrap_or("")),
            "",
        )
    }
}

// ---------------------------------------------------------------- colors

const NAMES: [(&str, Color); 17] = [
    ("reset", Color::Reset),
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("gray", Color::Gray),
    ("darkgray", Color::DarkGray),
    ("lightred", Color::LightRed),
    ("lightgreen", Color::LightGreen),
    ("lightyellow", Color::LightYellow),
    ("lightblue", Color::LightBlue),
    ("lightmagenta", Color::LightMagenta),
    ("lightcyan", Color::LightCyan),
    ("white", Color::White),
];

/// Parse a color literal: `0`–`255`, `#rrggbb`, `#rgb`, an ANSI name or `reset`.
pub fn parse_color(s: &str) -> Result<Color, String> {
    let t: String = s
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | ' '))
        .collect();
    let t = t.replace("grey", "gray");
    if let Ok(n) = t.parse::<u32>() {
        return u8::try_from(n)
            .map(Color::Indexed)
            .map_err(|_| format!("'{s}': color index must be 0-255"));
    }
    if let Some(h) = t.strip_prefix('#') {
        let hex = |x: &str| u8::from_str_radix(x, 16).ok();
        let rgb = match h.len() {
            6 => (hex(&h[0..2]), hex(&h[2..4]), hex(&h[4..6])),
            3 => {
                let d = |i: usize| hex(&h[i..i + 1]).map(|v| v * 17);
                (d(0), d(1), d(2))
            }
            _ => (None, None, None),
        };
        return match rgb {
            (Some(r), Some(g), Some(b)) => Ok(Color::Rgb(r, g, b)),
            _ => Err(format!("'{s}': expected #rrggbb or #rgb")),
        };
    }
    NAMES
        .iter()
        .find(|(n, _)| *n == t)
        .map(|(_, c)| *c)
        .ok_or_else(|| format!("'{s}': expected 0-255, #rrggbb, an ANSI color name or reset"))
}

/// The file form of a color (inverse of [`parse_color`]).
pub fn format_color(c: Color) -> String {
    match c {
        Color::Indexed(n) => n.to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        other => NAMES
            .iter()
            .find(|(_, c)| *c == other)
            .map(|(n, _)| n.to_string())
            .unwrap_or_else(|| "reset".into()),
    }
}

/// Parse a style color: a literal or `@palette_name`.
pub fn parse_color_ref(s: &str) -> Result<ColorRef, String> {
    match s.trim().strip_prefix('@') {
        Some(name) => color_slot(name.trim()).map(ColorRef::Slot).ok_or_else(|| {
            format!(
                "'{s}': unknown palette color (one of {})",
                COLOR_SLOTS.join(", ")
            )
        }),
        None => parse_color(s).map(ColorRef::Lit),
    }
}

pub fn format_color_ref(r: ColorRef) -> String {
    match r {
        ColorRef::Lit(c) => format_color(c),
        ColorRef::Slot(i) => format!("@{}", COLOR_SLOTS.get(i).copied().unwrap_or("text")),
    }
}

/// Approximate sRGB of a terminal color (xterm defaults); `None` for `reset`.
pub fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let idx = match c {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some((r, g, b)),
        Color::Indexed(n) => n,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    Some(match idx {
        0..=15 => ANSI[idx as usize],
        16..=231 => {
            const L: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let i = idx - 16;
            (
                L[(i / 36) as usize],
                L[(i / 6 % 6) as usize],
                L[(i % 6) as usize],
            )
        }
        _ => {
            let v = 8 + 10 * (idx - 232);
            (v, v, v)
        }
    })
}

/// WCAG contrast ratio (1–21) between two colors; `None` if either is `reset`.
pub fn contrast(a: Color, b: Color) -> Option<f64> {
    fn lum((r, g, b): (u8, u8, u8)) -> f64 {
        let ch = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    }
    let (la, lb) = (lum(rgb_of(a)?), lum(rgb_of(b)?));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    Some((hi + 0.05) / (lo + 0.05))
}

/// Below this contrast ratio the editor warns.
pub const LOW_CONTRAST: f64 = 3.0;

// ---------------------------------------------------------------- parse / serialize

/// Theme ids (file names): `[a-z0-9][a-z0-9_-]{0,31}`.
pub fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

fn value_str(v: &toml::Value) -> Option<String> {
    match v {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(n) => Some(n.to_string()),
        _ => None,
    }
}

fn parse_style(
    key: &str,
    v: &toml::Value,
    fallback: StyleSpec,
    framed: bool,
    w: &mut Vec<String>,
    at: &str,
) -> StyleSpec {
    let Some(t) = v.as_table() else {
        w.push(format!(
            "{at}.{key}: expected a table like {{ fg = \"@text\" }}; using its starter rule"
        ));
        return fallback;
    };
    let mut s = StyleSpec::default();
    for (k, val) in t {
        match k.as_str() {
            "fg" | "bg" | "border" => {
                let parsed = value_str(val)
                    .ok_or_else(|| "expected a string".to_string())
                    .and_then(|x| parse_color_ref(&x));
                match parsed {
                    Ok(c) => match k.as_str() {
                        "fg" => s.fg = Some(c),
                        "bg" => s.bg = Some(c),
                        _ if framed => s.border = Some(c),
                        _ => w.push(format!(
                            "{at}.{key}.border: only boxes have borders; ignored"
                        )),
                    },
                    Err(e) => w.push(format!("{at}.{key}.{k}: {e}; ignored")),
                }
            }
            a if ATTRS.contains(&a) => match val.as_bool() {
                Some(on) => s.set_attr(ATTRS.iter().position(|x| *x == a).unwrap(), on),
                None => w.push(format!("{at}.{key}.{k}: expected true or false; ignored")),
            },
            other => w.push(format!("{at}.{key}: unknown field '{other}' ignored")),
        }
    }
    s
}

fn parse_variant(t: &toml::Table, at: &str, w: &mut Vec<String>) -> VariantSpec {
    let mut v = VariantSpec {
        colors: [Color::Reset; N_COLORS],
        styles: (0..STYLE_SLOTS.len()).map(starter_style).collect(),
    };
    for (k, _) in t {
        if k != "colors" && k != "styles" {
            w.push(format!("[{at}]: unknown key '{k}' ignored"));
        }
    }
    let colors = t.get("colors").and_then(|c| c.as_table());
    if t.get("colors").is_some() && colors.is_none() {
        w.push(format!("[{at}.colors] must be a table"));
    }
    let empty = toml::Table::new();
    let colors = colors.unwrap_or(&empty);
    let mut missing = vec![];
    for (i, name) in COLOR_SLOTS.iter().enumerate() {
        match colors.get(*name) {
            None => missing.push(*name),
            Some(val) => match value_str(val)
                .ok_or_else(|| "expected a string".to_string())
                .and_then(|x| parse_color(&x))
            {
                Ok(c) => v.colors[i] = c,
                Err(e) => w.push(format!("{at}.colors.{name}: {e}; using reset")),
            },
        }
    }
    if !missing.is_empty() {
        w.push(format!(
            "{at}.colors: missing {} (terminal default used)",
            missing.join(", ")
        ));
    }
    for k in colors.keys() {
        if color_slot(k).is_none() {
            w.push(format!("{at}.colors: unknown color '{k}' ignored"));
        }
    }
    if let Some(styles) = t.get("styles") {
        match styles.as_table() {
            Some(styles) => {
                for (k, val) in styles {
                    match style_slot(k) {
                        Some(i) => {
                            v.styles[i] = parse_style(
                                k,
                                val,
                                starter_style(i),
                                STYLE_SLOTS[i].framed,
                                w,
                                &format!("{at}.styles"),
                            )
                        }
                        None => w.push(format!("{at}.styles: unknown part '{k}' ignored")),
                    }
                }
            }
            None => w.push(format!("[{at}.styles] must be a table")),
        }
    }
    v
}

/// Parse a theme file. `Err` only for files that can't be used at all; per-key problems
/// come back as warnings next to the theme.
pub fn parse(id: &str, text: &str) -> Result<(Theme, Vec<String>), String> {
    if text.len() > MAX_BYTES {
        return Err(format!("file is larger than {} KiB", MAX_BYTES / 1024));
    }
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| {
        e.to_string()
            .lines()
            .next()
            .unwrap_or("invalid TOML")
            .to_string()
            + &e.span()
                .map(|s| format!(" (line {})", text[..s.start].matches('\n').count() + 1))
                .unwrap_or_default()
    })?;
    let mut w = vec![];
    match table.get("schema") {
        None => w.push(format!("no schema; reading it as schema {SCHEMA}")),
        Some(toml::Value::Integer(n)) if *n == SCHEMA => {}
        Some(toml::Value::Integer(n)) if *n > SCHEMA => {
            return Err(format!(
                "schema {n} was made by a newer grokbot-tui (this one reads schema {SCHEMA})"
            ));
        }
        Some(other) => w.push(format!(
            "schema: expected {SCHEMA}, got {other}; reading as {SCHEMA}"
        )),
    }
    for k in table.keys() {
        if !matches!(
            k.as_str(),
            "schema" | "name" | "description" | "dark" | "light"
        ) {
            w.push(format!("unknown key '{k}' ignored"));
        }
    }
    let s = |k: &str| table.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let dark = table.get("dark").and_then(|v| v.as_table());
    let light = table.get("light").and_then(|v| v.as_table());
    let (dark, light) = match (dark, light) {
        (None, None) => return Err("no [dark] or [light] section".into()),
        (Some(d), Some(l)) => (
            parse_variant(d, "dark", &mut w),
            parse_variant(l, "light", &mut w),
        ),
        (Some(d), None) => {
            w.push("no [light] section: light uses the dark colors".into());
            let d = parse_variant(d, "dark", &mut w);
            (d.clone(), d)
        }
        (None, Some(l)) => {
            w.push("no [dark] section: dark uses the light colors".into());
            let l = parse_variant(l, "light", &mut w);
            (l.clone(), l)
        }
    };
    Ok((
        Theme {
            id: id.to_string(),
            name: s("name").unwrap_or_else(|| id.to_string()),
            description: s("description").unwrap_or_default(),
            dark,
            light,
        },
        w,
    ))
}

fn quote(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn style_line(s: &StyleSpec) -> String {
    let mut parts = vec![];
    for (k, c) in [("fg", s.fg), ("bg", s.bg), ("border", s.border)] {
        if let Some(c) = c {
            parts.push(format!("{k} = {}", quote(&format_color_ref(c))));
        }
    }
    for (i, a) in ATTRS.iter().enumerate() {
        if s.attr(i) {
            parts.push(format!("{a} = true"));
        }
    }
    if parts.is_empty() {
        "{}".into()
    } else {
        format!("{{ {} }}", parts.join(", "))
    }
}

/// The file text for a theme: every color and every style written out.
pub fn serialize(t: &Theme) -> String {
    let mut out = String::from(
        "# grokbot-tui theme. Edit it in the app (Shift-F5) or by hand; see `/help theme-files`.\n\
         # Colors: 0-255, \"#rrggbb\", \"#rgb\", an ANSI name (cyan, darkgray, …) or \"reset\"\n\
         # (terminal default). Styles: fg / bg / border (boxes only) as a color or \"@palette\n\
         # color\", plus bold / italic / underline / reverse / dim = true.\n",
    );
    out += &format!("schema = {SCHEMA}\nname = {}\n", quote(&t.name));
    if !t.description.is_empty() {
        out += &format!("description = {}\n", quote(&t.description));
    }
    for (vname, v) in [("dark", &t.dark), ("light", &t.light)] {
        out += &format!("\n[{vname}.colors]\n");
        for (i, name) in COLOR_SLOTS.iter().enumerate() {
            out += &format!("{name} = {}\n", quote(&format_color(v.colors[i])));
        }
        out += &format!("\n[{vname}.styles]\n");
        for (i, slot) in STYLE_SLOTS.iter().enumerate() {
            out += &format!("{} = {}\n", slot.key, style_line(&v.styles[i]));
        }
    }
    out
}

// ---------------------------------------------------------------- store

/// The user's themes dir for a data dir.
pub fn user_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

pub fn path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.toml"))
}

/// Theme ids in a dir, sorted.
pub fn list(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension()? == "toml" && p.is_file())
                .then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .filter(|id| valid_id(id))
        .collect();
    v.sort();
    v
}

/// Create the themes dir (0700 on Unix) if it doesn't exist. An existing dir's mode is
/// left alone.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write `bytes` to a new temp file next to `target` (`.<name>.<pid>.tmp`), created with
/// mode 0600 on Unix (no mode on Windows), synced to disk. The caller renames or links it.
fn write_tmp(target: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let dir = target
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent folder"))?;
    ensure_dir(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let r = (|| {
        let mut f = o.open(&tmp)?;
        #[cfg(unix)]
        {
            // In case the temp file already existed with another mode.
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()
    })();
    match r {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Replace `target` with `bytes` atomically: a 0600 temp file in the same folder, renamed
/// over it, so readers see the old or the new file, never half of one.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = write_tmp(target, bytes)?;
    std::fs::rename(&tmp, target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Like [`atomic_write`], but never replaces an existing file (`AlreadyExists`), even one
/// that appears while writing.
pub fn atomic_write_new(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let exists = || {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} already exists", target.display()),
        )
    };
    if target.exists() {
        return Err(exists());
    }
    let tmp = write_tmp(target, bytes)?;
    // A hard link fails when the target exists. Filesystems without hard links get a
    // rename after a last check.
    let r = match std::fs::hard_link(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(exists()),
        Err(_) if !target.exists() => std::fs::rename(&tmp, target),
        Err(e) => Err(e),
    };
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Why a theme couldn't be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Missing(PathBuf),
    Broken(PathBuf, String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Missing(p) => write!(f, "{} is missing", p.display()),
            LoadError::Broken(p, e) => write!(f, "{} is broken: {e}", p.display()),
        }
    }
}

pub fn load(dir: &Path, id: &str) -> Result<(Theme, Vec<String>), LoadError> {
    let p = path(dir, id);
    if !valid_id(id) {
        return Err(LoadError::Broken(
            p,
            format!("'{id}' is not a valid theme name"),
        ));
    }
    let meta = match std::fs::metadata(&p) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing(p)),
        Err(e) => return Err(LoadError::Broken(p, e.to_string())),
    };
    // Checked before reading, so a huge file is never read into memory.
    if meta.len() > MAX_BYTES as u64 {
        return Err(LoadError::Broken(
            p,
            format!(
                "it is {} bytes; theme files are at most {} KiB",
                meta.len(),
                MAX_BYTES / 1024
            ),
        ));
    }
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) => return Err(LoadError::Broken(p, e.to_string())),
    };
    parse(id, &text).map_err(|e| LoadError::Broken(p, e))
}

/// One theme file, for `/theme list`, `--list-themes` and the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    /// Display name and description from the file (empty when it can't be read).
    pub name: String,
    pub description: String,
    /// How many keys were skipped or defaulted.
    pub warnings: usize,
    /// Why it can't be used, if so.
    pub error: Option<String>,
}

/// Every theme in `dir`, sorted by id, with what's wrong with it.
pub fn entries(dir: &Path) -> Vec<Entry> {
    list(dir)
        .into_iter()
        .map(|id| match load(dir, &id) {
            Ok((t, w)) => Entry {
                id,
                name: t.name,
                description: t.description,
                warnings: w.len(),
                error: None,
            },
            Err(e) => Entry {
                id,
                name: String::new(),
                description: String::new(),
                warnings: 0,
                error: Some(match e {
                    LoadError::Broken(_, why) => why,
                    other => other.to_string(),
                }),
            },
        })
        .collect()
}

/// Copy theme `id` (its exact text, comments included) out of the themes dir.
/// `dest`: a file path, or an existing folder (it gets `<id>.toml`); default
/// `<data dir>/exports/<id>-<UTC stamp>.toml`. Only a usable theme is exported, and an
/// existing file is never overwritten.
pub fn export(
    dir: &Path,
    id: &str,
    dest: Option<&Path>,
    data_dir: &Path,
) -> Result<PathBuf, String> {
    match load(dir, id) {
        Ok(_) => {}
        Err(LoadError::Missing(p)) => {
            return Err(format!("no theme '{id}' ({} doesn't exist)", p.display()));
        }
        Err(LoadError::Broken(p, e)) => return Err(format!("{} can't be used: {e}", p.display())),
    }
    let text = std::fs::read(path(dir, id)).map_err(|e| e.to_string())?;
    let target = match dest {
        Some(d) if d.is_dir() => d.join(format!("{id}.toml")),
        Some(d) => d.to_path_buf(),
        None => data_dir.join("exports").join(format!(
            "{id}-{}.toml",
            utc_stamp(std::time::SystemTime::now())
        )),
    };
    atomic_write_new(&target, &text).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => {
            format!("{} already exists; not overwritten", target.display())
        }
        _ => format!("couldn't write {}: {e}", target.display()),
    })?;
    Ok(target)
}

/// The theme list shared by `/theme list` and `--list-themes`: one line per file
/// (● = `current`), with its name and description, its warning count or why it's broken.
pub fn list_text(dir: &Path, current: &str) -> String {
    let all = entries(dir);
    let mut out = vec![format!("Themes in {}:", dir.display())];
    if all.is_empty() {
        out.push("  (none)".into());
    }
    for e in &all {
        let mark = if e.id == current { "●" } else { " " };
        out.push(match &e.error {
            Some(err) => format!("{mark} {:<16} BROKEN: {err}", e.id),
            None => {
                let mut l = format!("{mark} {:<16} {}", e.id, e.name);
                if !e.description.is_empty() {
                    l += &format!(" — {}", e.description);
                }
                if e.warnings > 0 {
                    l += &format!(
                        " ({} warning{})",
                        e.warnings,
                        if e.warnings == 1 { "" } else { "s" }
                    );
                }
                l
            }
        });
    }
    if !all.iter().any(|e| e.id == current) {
        out.push(format!(
            "(the selected theme '{current}' is missing: plain terminal colors are used)"
        ));
    }
    out.join("\n")
}

/// `YYYYmmdd-HHMMSSZ` in UTC (no date crate needed).
pub fn utc_stamp(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// The file's current text (for "changed on disk" checks).
pub fn read_text(dir: &Path, id: &str) -> Option<String> {
    std::fs::read_to_string(path(dir, id)).ok()
}

/// Write a theme atomically (temp file + rename), private to the user.
pub fn save(dir: &Path, t: &Theme) -> std::io::Result<PathBuf> {
    if !valid_id(&t.id) {
        return Err(std::io::Error::other(format!(
            "'{}' is not a valid theme name",
            t.id
        )));
    }
    let p = path(dir, &t.id);
    atomic_write(&p, serialize(t).as_bytes())?;
    Ok(p)
}

/// Write a theme file's exact text as `<id>.toml`, atomically and 0600, but only if no
/// such file exists (never overwrites).
pub fn write_new(dir: &Path, id: &str, text: &str) -> std::io::Result<PathBuf> {
    if !valid_id(id) {
        return Err(std::io::Error::other(format!(
            "'{id}' is not a valid theme name"
        )));
    }
    let p = path(dir, id);
    atomic_write_new(&p, text.as_bytes())?;
    Ok(p)
}

pub fn delete(dir: &Path, id: &str) -> std::io::Result<()> {
    if !valid_id(id) {
        return Err(std::io::Error::other(format!(
            "'{id}' is not a valid theme name"
        )));
    }
    std::fs::remove_file(path(dir, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{color, slot};

    fn repo_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("themes")
    }

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    fn tmp_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn writes_are_atomic_private_and_leave_no_temp_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("themes");
        let (t, _) = parse(
            "x",
            &std::fs::read_to_string(repo_dir().join("default.toml")).unwrap(),
        )
        .unwrap();
        let mut t = t;
        t.id = "mine".into();
        let p = save(&dir, &t).unwrap();
        assert!(tmp_files(&dir).is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(mode(&p), 0o600);
            assert_eq!(mode(&dir), 0o700, "created private");
            // An existing file with looser permissions is replaced by a private one, and an
            // existing folder keeps the mode the user gave it.
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o750)).unwrap();
            save(&dir, &t).unwrap();
            assert_eq!(mode(&p), 0o600);
            assert_eq!(mode(&dir), 0o750);
        }
        // write_new never replaces.
        let e = write_new(&dir, "mine", "schema = 1").unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(load(&dir, "mine").unwrap().0.id, "mine");
        let q = write_new(&dir, "fresh", "schema = 1\n[dark.colors]\n").unwrap();
        #[cfg(unix)]
        assert_eq!(mode(&q), 0o600);
        let _ = q;
        assert!(tmp_files(&dir).is_empty());
        // A failed write (the target is a folder) cleans up its temp file.
        std::fs::create_dir(dir.join("blocked.toml")).unwrap();
        assert!(atomic_write(&dir.join("blocked.toml"), b"x").is_err());
        assert!(tmp_files(&dir).is_empty());
    }

    #[test]
    fn oversized_files_are_refused_before_reading() {
        let d = tempfile::tempdir().unwrap();
        let mut text = String::from("schema = 1\n[dark.colors]\n");
        text += &"# padding\n".repeat(MAX_BYTES / 10 + 1);
        std::fs::write(path(d.path(), "big"), &text).unwrap();
        match load(d.path(), "big") {
            Err(LoadError::Broken(_, e)) => {
                assert!(
                    e.contains(&format!("{} bytes", text.len())) && e.contains("64 KiB"),
                    "{e}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_schema_line_is_a_warning() {
        let (_, w) = parse("x", "[dark.colors]\ntext = 1\n").unwrap();
        assert!(w.iter().any(|w| w.contains("no schema")), "{w:?}");
        let (_, w) = parse("x", "schema = 1\n[dark.colors]\ntext = 1\n").unwrap();
        assert!(!w.iter().any(|w| w.contains("schema")), "{w:?}");
    }

    #[test]
    fn entries_and_list_text_describe_every_file() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        std::fs::copy(repo_dir().join("default.toml"), path(dir, "default")).unwrap();
        std::fs::write(
            path(dir, "odd"),
            "schema = 1\nname = \"Odd\"\nwat = 1\n[dark.colors]\n",
        )
        .unwrap();
        std::fs::write(path(dir, "bad"), "schema = 99\n").unwrap();
        let e = entries(dir);
        assert_eq!(
            e.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["bad", "default", "odd"]
        );
        assert!(e[0].error.as_ref().unwrap().contains("newer"));
        assert!(e[1].error.is_none() && !e[1].description.is_empty());
        assert!(e[2].warnings >= 1);
        let t = list_text(dir, "odd");
        assert!(
            t.starts_with(&format!("Themes in {}:", dir.display())),
            "{t}"
        );
        assert!(t.contains("● odd"), "{t}");
        assert!(t.contains("bad              BROKEN: "), "{t}");
        assert!(t.contains(" — "), "{t}");
        assert!(t.contains("warning"), "{t}");
        assert!(!t.contains("is missing"), "{t}");
        assert!(list_text(dir, "gone").contains("'gone' is missing"));
        let empty = tempfile::tempdir().unwrap();
        assert!(list_text(empty.path(), "default").contains("(none)"));
    }

    #[test]
    fn export_goes_to_exports_or_the_given_path() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("themes");
        std::fs::create_dir(&dir).unwrap();
        std::fs::copy(repo_dir().join("default.toml"), path(&dir, "default")).unwrap();
        let src = std::fs::read(path(&dir, "default")).unwrap();
        let p = export(&dir, "default", None, d.path()).unwrap();
        assert_eq!(p.parent().unwrap(), d.path().join("exports"));
        assert_eq!(std::fs::read(&p).unwrap(), src);
        let f = d.path().join("copy.toml");
        assert_eq!(export(&dir, "default", Some(&f), d.path()).unwrap(), f);
        assert!(
            export(&dir, "default", Some(&f), d.path())
                .unwrap_err()
                .contains("not overwritten")
        );
        assert!(
            export(&dir, "nope", None, d.path())
                .unwrap_err()
                .contains("no theme")
        );
        assert!(export(&dir, "../x", None, d.path()).is_err());
    }

    #[test]
    fn utc_stamps_are_civil_dates() {
        use std::time::{Duration, UNIX_EPOCH};
        let at = |s| utc_stamp(UNIX_EPOCH + Duration::from_secs(s));
        assert_eq!(at(0), "19700101-000000Z");
        assert_eq!(at(951_782_400), "20000229-000000Z");
        assert_eq!(at(1_700_000_000), "20231114-221320Z");
        assert_eq!(at(4_107_542_399), "21000228-235959Z");
    }

    #[test]
    fn color_literals_parse_and_format_round_trip() {
        assert_eq!(parse_color("252"), Ok(Color::Indexed(252)));
        assert_eq!(parse_color(" Cyan "), Ok(Color::Cyan));
        assert_eq!(parse_color("dark_grey"), Ok(Color::DarkGray));
        assert_eq!(parse_color("#1e90FF"), Ok(Color::Rgb(0x1e, 0x90, 0xff)));
        assert_eq!(parse_color("#fff"), Ok(Color::Rgb(255, 255, 255)));
        assert_eq!(parse_color("reset"), Ok(Color::Reset));
        assert!(parse_color("256").unwrap_err().contains("0-255"));
        assert!(parse_color("#12345").is_err());
        assert!(parse_color("purple").is_err());
        for c in [
            Color::Indexed(7),
            Color::Rgb(1, 2, 3),
            Color::LightMagenta,
            Color::Reset,
            Color::DarkGray,
        ] {
            assert_eq!(parse_color(&format_color(c)), Ok(c));
        }
        assert_eq!(
            parse_color_ref("@accent"),
            Ok(ColorRef::Slot(color::ACCENT))
        );
        assert!(
            parse_color_ref("@nope")
                .unwrap_err()
                .contains("unknown palette color")
        );
        assert_eq!(format_color_ref(ColorRef::Slot(color::OK)), "@ok");
    }

    #[test]
    fn contrast_follows_wcag() {
        let r = contrast(Color::Black, Color::White).unwrap();
        assert!((r - 21.0).abs() < 0.01, "{r}");
        assert!((contrast(Color::Indexed(250), Color::Indexed(250)).unwrap() - 1.0).abs() < 1e-9);
        assert_eq!(contrast(Color::Reset, Color::White), None);
        assert_eq!(rgb_of(Color::Indexed(16)), Some((0, 0, 0)));
        assert_eq!(rgb_of(Color::Indexed(231)), Some((255, 255, 255)));
        assert_eq!(rgb_of(Color::Indexed(232)), Some((8, 8, 8)));
        assert!(contrast(Color::Indexed(245), Color::Indexed(255)).unwrap() < LOW_CONTRAST);
    }

    #[test]
    fn theme_names_are_safe_file_names() {
        assert!(valid_id("default"));
        assert!(valid_id("solar-2_dark"));
        for bad in ["", "Dark", "../x", "a/b", "-x", "a.b", &"x".repeat(33)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn serialize_and_parse_round_trip() {
        let text = std::fs::read_to_string(repo_dir().join("default.toml")).unwrap();
        let (t, w) = parse("default", &text).unwrap();
        assert!(w.is_empty());
        let again = serialize(&t);
        let (t2, w2) = parse("default", &again).unwrap();
        assert!(w2.is_empty(), "{w2:?}");
        assert_eq!(t, t2);
        // The shipped file is exactly what the serializer writes (so saving it is a no-op).
        assert_eq!(again, text);
        let mut odd = Theme::unstyled("odd");
        odd.name = "Quote \" and \\ back".into();
        odd.light.colors[0] = Color::Rgb(1, 2, 3);
        odd.light.styles[slot::TITLE].italic = true;
        odd.light.styles[slot::TITLE].dim = true;
        assert_eq!(parse("odd", &serialize(&odd)).unwrap().0, odd);
    }

    #[test]
    fn partial_files_fall_back_per_key_with_warnings() {
        let text = r##"
            name = "Half"
            [dark.colors]
            text = "#ffffff"
            accent = 999
            bogus = "red"
            [dark.styles]
            brand = { fg = "@accent", bold = "yes", sparkle = true }
            base = "red"
            nope = {}
            help_title = { border = "@warn" }
        "##;
        let (t, w) = parse("half", text).unwrap();
        let joined = w.join("\n");
        assert_eq!(t.name, "Half");
        assert_eq!(t.dark.colors[color::TEXT], Color::Rgb(255, 255, 255));
        assert_eq!(
            t.dark.colors[color::ACCENT],
            Color::Reset,
            "bad value → reset"
        );
        assert_eq!(t.dark.colors[color::OK], Color::Reset, "missing → reset");
        assert!(joined.contains("dark.colors.accent"), "{joined}");
        assert!(joined.contains("missing"), "{joined}");
        assert!(joined.contains("unknown color 'bogus'"), "{joined}");
        assert!(joined.contains("expected true or false"), "{joined}");
        assert!(joined.contains("unknown field 'sparkle'"), "{joined}");
        assert!(joined.contains("unknown part 'nope'"), "{joined}");
        assert!(joined.contains("only boxes have borders"), "{joined}");
        assert!(joined.contains("no [light] section"), "{joined}");
        // brand kept its valid field; base (not a table) and missing parts use starter rules.
        assert_eq!(
            t.dark.styles[slot::BRAND].fg,
            Some(ColorRef::Slot(color::ACCENT))
        );
        assert!(!t.dark.styles[slot::BRAND].bold);
        assert_eq!(t.dark.styles[slot::BASE], starter_style(slot::BASE));
        assert_eq!(t.dark.styles[slot::SELECTED], starter_style(slot::SELECTED));
        assert_eq!(t.light, t.dark, "missing light copies dark");
    }

    #[test]
    fn unusable_files_are_refused_with_a_reason() {
        let e = parse("x", "name = \"x\"\n[dark.colors\ntext = 1").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
        assert!(
            parse("x", "name = \"x\"")
                .unwrap_err()
                .contains("no [dark] or [light]")
        );
        assert!(
            parse("x", "schema = 2\n[dark]")
                .unwrap_err()
                .contains("newer grokbot-tui")
        );
        let big = format!("# {}\n[dark]", "x".repeat(MAX_BYTES));
        assert!(parse("x", &big).unwrap_err().contains("larger than"));
        let (_, w) = parse("x", "schema = \"one\"\n[dark]\nextra = 1\n[light]").unwrap();
        assert!(w.iter().any(|w| w.contains("schema")), "{w:?}");
        assert!(w.iter().any(|w| w.contains("unknown key 'extra'")), "{w:?}");
    }

    #[test]
    fn load_save_delete() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("themes");
        assert!(matches!(load(&dir, "default"), Err(LoadError::Missing(_))));
        assert!(matches!(load(&dir, "../x"), Err(LoadError::Broken(..))));
        let mut t = Theme::unstyled("mine");
        t.dark.colors[color::ACCENT] = Color::Indexed(33);
        let p = save(&dir, &t).unwrap();
        assert_eq!(p, path(&dir, "mine"));
        assert!(!dir.join(".mine.toml.tmp").exists());
        assert_eq!(load(&dir, "mine").unwrap().0, t);
        std::fs::write(path(&dir, "broken"), "[dark").unwrap();
        let e = load(&dir, "broken").unwrap_err();
        assert!(matches!(e, LoadError::Broken(..)));
        assert!(e.to_string().contains("broken.toml is broken"), "{e}");
        assert_eq!(list(&dir), vec!["broken", "mine"]);
        delete(&dir, "mine").unwrap();
        assert_eq!(list(&dir), vec!["broken"]);
        assert!(delete(&dir, "../etc").is_err());
        assert!(save(&dir, &Theme::unstyled("Bad")).is_err());
    }

    #[test]
    fn download_urls_and_opt_out() {
        assert_eq!(
            release_url("1.6.0"),
            "https://raw.githubusercontent.com/WeirdBrains/grokbot-tui/v1.6.0/themes/default.toml"
        );
        assert!(default_url().contains(&format!("/v{}/", env!("CARGO_PKG_VERSION"))));
        for on in ["1", "true", "YES", "on", "x"] {
            assert!(opted_out(Some(on)), "{on}");
        }
        for off in ["", "0", "false", "No", " off "] {
            assert!(!opted_out(Some(off)), "{off}");
        }
        assert!(!opted_out(None));
    }

    #[test]
    fn download_validates_what_it_fetches() {
        use wiremock::matchers::path as at;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let good = std::fs::read_to_string(repo_dir().join("default.toml")).unwrap();
        rt.block_on(async {
            let srv = MockServer::start().await;
            let serve =
                |p: &str, r: ResponseTemplate| Mock::given(at(p.to_string())).respond_with(r);
            serve(
                "/ok",
                ResponseTemplate::new(200).set_body_string(good.clone()),
            )
            .mount(&srv)
            .await;
            serve("/bad", ResponseTemplate::new(200).set_body_string("<html>"))
                .mount(&srv)
                .await;
            serve(
                "/big",
                ResponseTemplate::new(200).set_body_string("#".repeat(MAX_BYTES + 1)),
            )
            .mount(&srv)
            .await;
            serve(
                "/bin",
                ResponseTemplate::new(200).set_body_bytes(vec![0xff, 0xfe, 0x00]),
            )
            .mount(&srv)
            .await;
            let url = |p: &str| format!("{}{p}", srv.uri());
            assert_eq!(download(&url("/ok")).await.unwrap(), good, "exact text");
            assert_eq!(
                download(&url("/missing")).await.unwrap_err(),
                "HTTP 404 Not Found"
            );
            assert!(
                download(&url("/bad"))
                    .await
                    .unwrap_err()
                    .contains("not a usable theme")
            );
            assert!(
                download(&url("/big"))
                    .await
                    .unwrap_err()
                    .contains("larger than 64 KiB")
            );
            assert_eq!(download(&url("/bin")).await.unwrap_err(), "not UTF-8 text");
        });
        // Nothing listening: a connection error, quickly.
        let t = std::time::Instant::now();
        let e = rt
            .block_on(download("http://127.0.0.1:9/default.toml"))
            .unwrap_err();
        assert!(e.contains("connect"), "{e}");
        assert!(t.elapsed() < DOWNLOAD_TIMEOUT + std::time::Duration::from_secs(1));
    }

    /// Real network: `cargo test -- --ignored download_from_github`.
    #[test]
    #[ignore = "needs the network"]
    fn download_from_github_release_tag() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let text = rt.block_on(download(&release_url("1.5.0"))).unwrap();
        let (t, w) = parse(DEFAULT_ID, &text).unwrap();
        assert!(w.is_empty());
        assert_eq!(t.name, "Default");
        let e = rt.block_on(download(&release_url("0.0.0"))).unwrap_err();
        assert!(e.starts_with("HTTP 404"), "{e}");
    }

    #[test]
    fn write_new_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("themes");
        let p = write_new(&dir, "default", "# one\n[dark]").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# one\n[dark]");
        let e = write_new(&dir, "default", "# two\n[dark]").unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "# one\n[dark]");
        assert!(write_new(&dir, "../x", "").is_err());
        assert_eq!(list(&dir), vec!["default"], "no temp files left behind");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
