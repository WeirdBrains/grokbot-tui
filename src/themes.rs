//! Theme files: `<data dir>/themes/<id>.toml`.
//!
//! There is no theme inside the binary. Packages ship `default.toml` in a shared data dir
//! (`/usr/share/grokbot-tui/themes`, Homebrew's `share/grokbot-tui/themes`, next to the
//! Windows exe, …); on startup, when the user's themes dir has no themes, [`seed`] copies
//! the shipped files there. If the selected theme is missing or broken, the app falls back
//! to plain terminal colors ([`Theme::unstyled`]) and tells the user what to fix.
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
/// Theme directory name inside the data dir and inside shared `…/grokbot-tui/` dirs.
pub const DIR_NAME: &str = "themes";
/// The theme selected when the `ui.theme` pref is unset.
pub const DEFAULT_ID: &str = "default";
/// Where users can get the default theme by hand (shown in the "missing" notice).
pub const DEFAULT_URL: &str =
    "https://github.com/WeirdBrains/grokbot-tui/raw/main/themes/default.toml";

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
        None => {}
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

/// Shared dirs packages install themes into, most specific first.
pub fn shared_dirs() -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        // Homebrew (bin/../share), /usr/bin → /usr/share, the Windows MSI (bin\..\share).
        v.push(exe_dir.join("../share/grokbot-tui").join(DIR_NAME));
        // An unpacked release archive, or a portable copy.
        v.push(exe_dir.join(DIR_NAME));
    }
    if cfg!(unix) {
        let xdg = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        for d in xdg.split(':').filter(|d| !d.is_empty()) {
            v.push(Path::new(d).join("grokbot-tui").join(DIR_NAME));
        }
        for d in ["/usr/share", "/usr/local/share", "/opt/homebrew/share"] {
            v.push(Path::new(d).join("grokbot-tui").join(DIR_NAME));
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.canonicalize().unwrap_or_else(|_| p.clone())));
    v
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

#[cfg(unix)]
fn private(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn private(_: &Path) {}

fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// When `dir` has no themes, copy the `*.toml` themes from `sources` into it (the first
/// source with a given name wins). Returns the copied files' sources. Nothing is ever
/// overwritten.
pub fn seed(dir: &Path, sources: &[PathBuf]) -> std::io::Result<Vec<PathBuf>> {
    if !list(dir).is_empty() {
        return Ok(vec![]);
    }
    let mut copied = vec![];
    for src in sources {
        for id in list(src) {
            let from = path(src, &id);
            let to = path(dir, &id);
            if to.exists()
                || std::fs::metadata(&from)
                    .map(|m| m.len() as usize > MAX_BYTES)
                    .unwrap_or(true)
            {
                continue;
            }
            ensure_dir(dir)?;
            std::fs::copy(&from, &to)?;
            private(&to);
            copied.push(from);
        }
    }
    Ok(copied)
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
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing(p)),
        Err(e) => return Err(LoadError::Broken(p, e.to_string())),
    };
    parse(id, &text).map_err(|e| LoadError::Broken(p, e))
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
    ensure_dir(dir)?;
    let p = path(dir, &t.id);
    let tmp = dir.join(format!(".{}.toml.tmp", t.id));
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        private(&tmp);
        f.write_all(serialize(t).as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &p)?;
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
    fn seed_copies_shipped_themes_only_into_an_empty_dir() {
        let d = tempfile::tempdir().unwrap();
        let user = d.path().join("data/themes");
        let other = d.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("default.toml"), "# not this one\n[dark]").unwrap();
        std::fs::write(other.join("extra.toml"), "[dark]").unwrap();
        std::fs::write(other.join("Bad Name.toml"), "[dark]").unwrap();
        let copied = seed(
            &user,
            &[repo_dir(), d.path().join("missing"), other.clone()],
        )
        .unwrap();
        assert_eq!(copied.len(), 2, "{copied:?}");
        assert_eq!(list(&user), vec!["default", "extra"]);
        // First source wins for a name.
        assert_eq!(
            std::fs::read_to_string(path(&user, "default")).unwrap(),
            std::fs::read_to_string(repo_dir().join("default.toml")).unwrap()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path(&user, "default"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Not empty any more: nothing is copied or overwritten.
        std::fs::write(path(&user, "default"), "# mine\n[dark]").unwrap();
        std::fs::remove_file(path(&user, "extra")).unwrap();
        assert!(seed(&user, &[repo_dir(), other]).unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(path(&user, "default")).unwrap(),
            "# mine\n[dark]"
        );
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
}
