//! TOML keymap loader (vim mode). See `docs/vim-mode-plan.md`.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Actions the keymap may name. Unknown ids are load errors.
pub const ACTIONS: &[&str] = &[
    "app.quit",
    "app.ctrl_c",
    "app.ctrl_c_copy",
    "help.open",
    "mouse.toggle",
    "mode.insert",
    "mode.insert_end",
    "mode.normal",
    "mode.visual",
    "mode.command",
    "move.left",
    "move.right",
    "move.up",
    "move.down",
    "move.top",
    "move.bottom",
    "focus.next",
    "focus.prev",
    "input.history_search",
    "input.undo_clear",
    "input.paste",
    "input.newline",
    "selection.select_all",
    "selection.copy",
    "selection.copy_line",
    "selection.clear",
    "command.run",
    "command.complete",
    "keymap.reload",
    "message.react",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mode {
    Normal,
    Insert,
    Command,
    Visual,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Insert => "insert",
            Self::Command => "command",
            Self::Visual => "visual",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Command => "COMMAND",
            Self::Visual => "VISUAL",
        }
    }

    fn from_table(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Self::Normal),
            "insert" => Some(Self::Insert),
            "command" => Some(Self::Command),
            "visual" => Some(Self::Visual),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    pub global: BTreeMap<String, String>,
    pub modes: BTreeMap<Mode, BTreeMap<String, String>>,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    Io(String),
    Parse(String),
    UnknownAction {
        action: String,
        key: String,
        table: String,
    },
    Conflict {
        key: String,
        table: String,
        a: String,
        b: String,
    },
    UnknownMode(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Parse(e) => write!(f, "{e}"),
            Self::UnknownAction { action, key, table } => {
                write!(f, "unknown action '{action}' for key '{key}' in [{table}]")
            }
            Self::Conflict { key, table, a, b } => {
                write!(f, "key '{key}' in [{table}] maps to both '{a}' and '{b}'")
            }
            Self::UnknownMode(m) => write!(f, "unknown mode table [mode.{m}]"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct File {
    #[serde(default)]
    global: BTreeMap<String, String>,
    #[serde(default)]
    mode: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn user_path(data_dir: &Path) -> PathBuf {
    data_dir.join("keymap.toml")
}

pub fn shipped_default() -> &'static str {
    include_str!("../keymap/default.toml")
}

/// Parse and validate. Does not touch disk.
pub fn parse(text: &str) -> Result<Keymap, LoadError> {
    let file: File = toml::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    let mut km = Keymap::default();
    fill_table("global", &file.global, &mut km.global)?;
    for (name, table) in &file.mode {
        let mode = Mode::from_table(name).ok_or_else(|| LoadError::UnknownMode(name.clone()))?;
        let mut m = BTreeMap::new();
        fill_table(&format!("mode.{name}"), table, &mut m)?;
        km.modes.insert(mode, m);
    }
    Ok(km)
}

fn fill_table(
    table: &str,
    src: &BTreeMap<String, String>,
    dst: &mut BTreeMap<String, String>,
) -> Result<(), LoadError> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (key, action) in src {
        let k = normalize_key(key);
        if !ACTIONS.contains(&action.as_str()) {
            return Err(LoadError::UnknownAction {
                action: action.clone(),
                key: key.clone(),
                table: table.into(),
            });
        }
        if let Some(prev) = seen.get(&k).or_else(|| dst.get(&k)) {
            return Err(LoadError::Conflict {
                key: k,
                table: table.into(),
                a: prev.clone(),
                b: action.clone(),
            });
        }
        seen.insert(k.clone(), action.clone());
        dst.insert(k, action.clone());
    }
    Ok(())
}

/// Normalize key names: lowercase modifiers, keep the final key's case (so `g` ≠ `G`).
pub fn normalize_key(raw: &str) -> String {
    let s = raw.trim().replace(' ', "");
    let s = s
        .replace("Control-", "ctrl-")
        .replace("control-", "ctrl-")
        .replace("Ctl-", "ctrl-")
        .replace("ctl-", "ctrl-")
        .replace("Cmd-", "ctrl-")
        .replace("cmd-", "ctrl-")
        .replace("Shift-", "shift-")
        .replace("Alt-", "alt-")
        .replace("CTRL-", "ctrl-")
        .replace("SHIFT-", "shift-")
        .replace("ALT-", "alt-");
    // Lowercase every segment except the last (the key itself).
    let mut parts: Vec<&str> = s.split('-').collect();
    if parts.is_empty() {
        return s;
    }
    let last = parts.pop().unwrap();
    let mut out: Vec<String> = parts.into_iter().map(|p| p.to_ascii_lowercase()).collect();
    // Named keys (esc, tab, enter, …) stay lowercase; single letters keep case.
    if last.len() > 1 {
        out.push(last.to_ascii_lowercase());
    } else {
        out.push(last.to_string());
    }
    out.join("-")
}

/// Load from `path`, or the shipped default if missing. On error returns the
/// previous map (or shipped) plus the error for a notice.
pub fn load(path: &Path, previous: Option<&Keymap>) -> (Keymap, Option<LoadError>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut km = parse(shipped_default()).expect("shipped keymap parses");
            km.path = Some(path.to_path_buf());
            // Best-effort install of the default for editing.
            let _ = std::fs::write(path, shipped_default());
            return (km, None);
        }
        Err(e) => {
            let fallback = previous.cloned().unwrap_or_else(|| {
                let mut km = parse(shipped_default()).expect("shipped");
                km.path = Some(path.to_path_buf());
                km
            });
            return (
                fallback,
                Some(LoadError::Io(format!("{}: {e}", path.display()))),
            );
        }
    };
    match parse(&text) {
        Ok(mut km) => {
            km.path = Some(path.to_path_buf());
            (km, None)
        }
        Err(e) => {
            let fallback = previous.cloned().unwrap_or_else(|| {
                let mut km = parse(shipped_default()).expect("shipped");
                km.path = Some(path.to_path_buf());
                km
            });
            (fallback, Some(e))
        }
    }
}

impl Keymap {
    pub fn lookup(&self, mode: Mode, key: &str) -> Option<&str> {
        let k = normalize_key(key);
        self.modes
            .get(&mode)
            .and_then(|m| m.get(&k))
            .or_else(|| self.global.get(&k))
            .map(String::as_str)
    }

    /// Human listing for `/keymap`.
    pub fn list_lines(&self) -> Vec<String> {
        let mut lines = vec![];
        if let Some(p) = &self.path {
            lines.push(format!("# {}", p.display()));
        }
        lines.push("[global]".into());
        for (k, a) in &self.global {
            lines.push(format!("  {k} = {a}"));
        }
        for mode in [Mode::Normal, Mode::Insert, Mode::Command, Mode::Visual] {
            if let Some(t) = self.modes.get(&mode) {
                lines.push(format!("[mode.{}]", mode.as_str()));
                for (k, a) in t {
                    lines.push(format!("  {k} = {a}"));
                }
            }
        }
        lines
    }
}

/// Encode a crossterm key into the keymap key string.
pub fn encode_key(
    code: ratatui::crossterm::event::KeyCode,
    mods: ratatui::crossterm::event::KeyModifiers,
) -> Option<String> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let mut parts = vec![];
    if mods.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        // Letters already carry case; for Tab/arrows name the shift.
        if !matches!(code, KeyCode::Char(_)) {
            parts.push("shift");
        }
    }
    if mods.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    let key = match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            parts.push("shift");
            "tab".into()
        }
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        _ => return None,
    };
    parts.push(&key);
    Some(normalize_key(&parts.join("-")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_default_parses_and_has_no_conflicts() {
        let km = parse(shipped_default()).unwrap();
        assert!(km.lookup(Mode::Normal, "i") == Some("mode.insert"));
        assert!(km.lookup(Mode::Insert, "esc") == Some("mode.normal"));
        assert!(km.lookup(Mode::Normal, "ctrl-q") == Some("app.quit"));
    }

    #[test]
    fn unknown_action_and_conflict_are_errors() {
        let e = parse(
            r#"[global]
"ctrl-q" = "nope"
"#,
        )
        .unwrap_err();
        assert!(matches!(e, LoadError::UnknownAction { .. }), "{e}");
        let e = parse(
            r#"[mode.normal]
"j" = "move.down"
"j" = "move.up"
"#,
        );
        // TOML map can't have duplicate keys — serde keeps one. Simulate via fill.
        let mut m = BTreeMap::new();
        fill_table(
            "mode.normal",
            &[("j".into(), "move.down".into())].into(),
            &mut m,
        )
        .unwrap();
        let err = fill_table(
            "mode.normal",
            &[("j".into(), "move.up".into())].into(),
            &mut m,
        )
        .unwrap_err();
        assert!(matches!(err, LoadError::Conflict { .. }), "{err}");
        let _ = e;
    }

    #[test]
    fn bad_file_keeps_previous_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keymap.toml");
        std::fs::write(&path, "not = toml {{").unwrap();
        let prev = parse(shipped_default()).unwrap();
        let (km, err) = load(&path, Some(&prev));
        assert!(err.is_some());
        assert_eq!(km.lookup(Mode::Normal, "i"), Some("mode.insert"));
    }

    #[test]
    fn g_and_shift_g_are_distinct() {
        let km = parse(shipped_default()).unwrap();
        assert_eq!(km.lookup(Mode::Normal, "g"), Some("move.top"));
        assert_eq!(km.lookup(Mode::Normal, "shift-g"), Some("move.bottom"));
    }

    #[test]
    fn encode_key_normalizes() {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};
        assert_eq!(
            encode_key(KeyCode::Char('q'), KeyModifiers::CONTROL).as_deref(),
            Some("ctrl-q")
        );
        assert_eq!(
            encode_key(KeyCode::Esc, KeyModifiers::NONE).as_deref(),
            Some("esc")
        );
        assert_eq!(
            encode_key(KeyCode::BackTab, KeyModifiers::NONE).as_deref(),
            Some("shift-tab")
        );
    }
}
