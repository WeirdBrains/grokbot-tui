//! grokbot-tui — terminal UI for Grok Bot.
mod api;
mod app;
mod commands;
mod config;
mod datadir;
mod help;
mod input;
mod picker;
mod storage;
mod theme;
mod ui;

use anyhow::Result;
use app::{App, Focus};
use clap::Parser;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use storage::Store;

#[derive(Parser)]
#[command(name = "grokbot-tui", version, about = "Terminal UI for Grok Bot")]
struct Cli {
    /// Use the offline mock API (fake bots, echo replies; any token except "bad" works).
    #[arg(long)]
    mock: bool,
    /// Account alias to start in (default: last active account).
    #[arg(long, env = "GROKBOT_TUI_ACCOUNT", value_name = "ALIAS")]
    account: Option<String>,
    /// Data directory holding context.db (default ~/.grokbot-tui; a pre-1.2 ~/.gb is moved there).
    #[arg(long, value_name = "DIR")]
    data_dir: Option<std::path::PathBuf>,
}

/// Environment variable renames applied at startup, in order: `(old name, new name)`.
/// First the pre-1.2 `GB_*` names fall back onto their `GROKBOT_TUI_*` replacements, then
/// the two variables grokbot_api reads itself are forwarded to the library's names.
const LEGACY_ENV: [(&str, &str); 12] = [
    ("GB_GROKBOT_TOKEN", "GROKBOT_TUI_TOKEN"),
    ("GB_GROKBOT_GATEWAY_URL", "GROKBOT_TUI_GATEWAY_URL"),
    ("GB_GROKBOT_GATEWAY_HEADERS", "GROKBOT_TUI_GATEWAY_HEADERS"),
    ("GB_CURSOR_API_BASE_URL", "GROKBOT_TUI_CURSOR_API_BASE_URL"),
    (
        "GB_CURSOR_CLIENT_VERSION",
        "GROKBOT_TUI_CURSOR_CLIENT_VERSION",
    ),
    ("GB_CURSOR_CLIENT_TYPE", "GROKBOT_TUI_CURSOR_CLIENT_TYPE"),
    ("GB_SAND_BOX_NAMESPACE", "GROKBOT_TUI_SAND_BOX_NAMESPACE"),
    ("GB_ACCOUNT", "GROKBOT_TUI_ACCOUNT"),
    ("GB_ALLOW_LOCAL_GATEWAY", "GROKBOT_TUI_ALLOW_LOCAL_GATEWAY"),
    ("GB_GROKBOT_APP_DIR", "GROKBOT_TUI_APP_DIR"),
    // Read by grokbot_api 1.1 under its own names.
    (
        "GROKBOT_TUI_ALLOW_LOCAL_GATEWAY",
        "GROKBOT_ALLOW_LOCAL_GATEWAY",
    ),
    ("GROKBOT_TUI_APP_DIR", "GROKBOT_APP_DIR"),
];

/// The `(new name, value)` pairs to set so older names keep working. Each rename copies a
/// non-empty old value only when the new name is not set at all, so a new name always
/// wins; later renames see the values set by earlier ones (`GB_X` → `GROKBOT_TUI_X` → library).
fn legacy_env_updates(get: impl Fn(&str) -> Option<String>) -> Vec<(&'static str, String)> {
    let mut set: Vec<(&'static str, String)> = Vec::new();
    for (old, new) in LEGACY_ENV {
        let lookup = |k: &str| {
            set.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.clone())
                .or_else(|| get(k))
        };
        if lookup(new).is_none()
            && let Some(v) = lookup(old).filter(|v| !v.is_empty())
        {
            set.push((new, v));
        }
    }
    set
}

fn main() -> Result<()> {
    for (name, value) in legacy_env_updates(|k| std::env::var(k).ok()) {
        // SAFETY: runs first in `main`, before the tokio runtime or any other thread exists.
        unsafe { std::env::set_var(name, value) };
    }
    let cli = Cli::parse();
    // --data-dir overrides everything, including the ~/.gb migration.
    let (dir, notice) = match cli.data_dir {
        Some(d) => (d, None),
        None => Store::default_dir()?,
    };
    if let Some(n) = &notice {
        eprintln!("grokbot-tui: {n}");
    }
    let store = Store::open(dir, cli.account.as_deref())?;
    let rt = tokio::runtime::Runtime::new()?;
    let mut app = App::new(store, rt.handle().clone(), cli.mock)?;
    app.init();
    if let Some(n) = notice {
        app.info(n);
    }

    let mut term = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    enable_shift_enter();
    let res = run(&mut term, &mut app);
    disable_shift_enter();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

fn run(term: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut captured = true;
    while !app.should_quit {
        app.tick();
        if app.mouse_capture != captured {
            captured = app.mouse_capture;
            if captured {
                execute!(std::io::stdout(), EnableMouseCapture)?;
            } else {
                execute!(std::io::stdout(), DisableMouseCapture)?;
            }
        }
        if let Some(t) = app.pending_clipboard.take() {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = out.write_all(osc52(&t).as_bytes());
            let _ = out.flush();
        }
        term.draw(|f| ui::draw(f, app))?;
        if !event::poll(app::UI_TICK)? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if k.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('d'))
                {
                    app.should_quit = true;
                    continue;
                }
                if let Some(p) = app.picker.as_mut() {
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('q') => app.picker_cancel(),
                        KeyCode::Enter => app.picker_confirm(),
                        KeyCode::Up | KeyCode::Char('k') => p.move_by(-1),
                        KeyCode::Down | KeyCode::Char('j') => p.move_by(1),
                        KeyCode::PageUp => p.move_by(-10),
                        KeyCode::PageDown => p.move_by(10),
                        KeyCode::Home => p.first(),
                        KeyCode::End => p.last(),
                        _ => {}
                    }
                    continue;
                }
                if k.code == KeyCode::F(6) {
                    app.toggle_mouse_capture();
                    continue;
                }
                if k.code == KeyCode::F(2) {
                    app.open_bot_picker();
                    continue;
                }
                if k.code == KeyCode::F(3) {
                    app.open_account_picker();
                    continue;
                }
                if app.shell.is_some() {
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('q') => app.close_shell(),
                        KeyCode::Up | KeyCode::Char('k') => app.shell_scroll(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.shell_scroll(1),
                        KeyCode::PageUp => app.shell_scroll(-10),
                        KeyCode::PageDown | KeyCode::Char(' ') => app.shell_scroll(10),
                        KeyCode::Home => app.shell_scroll(-100_000),
                        KeyCode::End => app.shell_scroll(100_000),
                        _ => {}
                    }
                    continue;
                }
                if app.help.as_ref().is_some_and(|m| m.query.is_some()) {
                    let q = app.help.as_mut().and_then(|m| m.query.as_mut()).unwrap();
                    match k.code {
                        KeyCode::Esc => app.help.as_mut().unwrap().query = None,
                        KeyCode::Enter => app.help_search_submit(),
                        KeyCode::Backspace => {
                            q.pop();
                        }
                        KeyCode::Char(c) => q.push(c),
                        _ => {}
                    }
                    continue;
                }
                if app.help.is_some() {
                    match k.code {
                        KeyCode::Char('/') => app.help_search_start(),
                        KeyCode::Esc | KeyCode::Char('q') | KeyCode::F(1) => app.close_help(),
                        KeyCode::Up | KeyCode::Char('k') => app.help_scroll(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.help_scroll(1),
                        KeyCode::PageUp => app.help_scroll(-10),
                        KeyCode::PageDown | KeyCode::Char(' ') => app.help_scroll(10),
                        KeyCode::Home => app.help_scroll(-10_000),
                        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => app.help_select(1),
                        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                            app.help_select(-1)
                        }
                        KeyCode::Enter => app.help_follow(),
                        KeyCode::Backspace => app.help_back(),
                        _ => {}
                    }
                    continue;
                }
                if k.code == KeyCode::F(1)
                    || (k.code == KeyCode::Char('?')
                        && app.input.is_empty()
                        && app.mode == app::InputMode::Normal)
                {
                    app.open_help(if k.code == KeyCode::F(1) { "" } else { "keys" });
                    continue;
                }
                match k.code {
                    KeyCode::Tab => {
                        app.focus = if app.focus == Focus::Input {
                            Focus::Sidebar
                        } else {
                            Focus::Input
                        };
                        if let Some(i) = app.active {
                            app.sidebar_cursor = i;
                        }
                    }
                    KeyCode::PageUp => app.scroll_up(10),
                    KeyCode::PageDown => app.scroll_down(10),
                    KeyCode::Esc => {
                        app.focus = Focus::Input;
                        app.cancel_input_mode();
                    }
                    code if app.focus == Focus::Sidebar => match code {
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.sidebar_cursor = app.sidebar_cursor.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.sidebar_cursor + 1 < app.bots.len() {
                                app.sidebar_cursor += 1;
                            }
                        }
                        KeyCode::Enter => {
                            app.select_bot(app.sidebar_cursor);
                            app.focus = Focus::Input;
                        }
                        _ => {}
                    },
                    KeyCode::Enter if is_newline_key(&k) => app.input_newline(),
                    KeyCode::Char('j') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.input_newline()
                    }
                    KeyCode::Enter => app.submit(),
                    KeyCode::Backspace => app.input_backspace(),
                    KeyCode::Delete => app.input_delete(),
                    KeyCode::Left => app.input_left(),
                    KeyCode::Right => app.input_right(),
                    KeyCode::Home => app.input_home(),
                    KeyCode::End => app.input_end(),
                    // Move between input lines; at the top/bottom line, scroll the chat.
                    KeyCode::Up => {
                        if !app.input_up() {
                            app.scroll_up(1)
                        }
                    }
                    KeyCode::Down => {
                        if !app.input_down() {
                            app.scroll_down(1)
                        }
                    }
                    KeyCode::Char(c) => app.input_insert(&c.to_string()),
                    _ => {}
                }
            }
            Event::Paste(s) => app.input_insert(s.trim_end_matches(['\r', '\n'])),
            Event::Mouse(m) if app.picker.is_some() => {
                let p = app.picker.as_mut().unwrap();
                match m.kind {
                    MouseEventKind::Moved => p.hover(m.column, m.row),
                    MouseEventKind::ScrollUp => p.move_by(-1),
                    MouseEventKind::ScrollDown => p.move_by(1),
                    MouseEventKind::Down(MouseButton::Left) => match p.click(m.column, m.row) {
                        picker::MouseOutcome::Confirm => app.picker_confirm(),
                        picker::MouseOutcome::Cancel => app.picker_cancel(),
                        picker::MouseOutcome::None => {}
                    },
                    _ => {}
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::ScrollUp if app.shell.is_some() => app.shell_scroll(-3),
                MouseEventKind::ScrollDown if app.shell.is_some() => app.shell_scroll(3),
                MouseEventKind::Down(MouseButton::Left) if app.shell.is_some() => {}
                MouseEventKind::Down(MouseButton::Left) if app.help.is_none() => {
                    app.click(m.column, m.row)
                }
                MouseEventKind::ScrollUp if app.help.is_some() => app.help_scroll(-3),
                MouseEventKind::ScrollDown if app.help.is_some() => app.help_scroll(3),
                MouseEventKind::ScrollUp => app.scroll_up(3),
                MouseEventKind::ScrollDown => app.scroll_down(3),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}

/// OSC 52 "set clipboard" sequence for `text` (base64). Terminals without support ignore it.
fn osc52(text: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = text.as_bytes();
    let mut s = String::with_capacity(b.len() * 4 / 3 + 12);
    s.push_str("\x1b]52;c;");
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s.push('\x07');
    s
}

/// xterm modifyOtherKeys level 1 on / reset. Level 1 only re-encodes keys with no
/// standard sequence (like Shift-Enter), so Ctrl-C / Ctrl-J are untouched. This lets
/// tmux (`extended-keys on`, `extended-keys-format csi-u`) forward Shift-Enter as
/// `CSI 13;2u`, which crossterm parses. (Level 2 would re-encode Ctrl+letters too.)
const MODIFY_OTHER_KEYS_ON: &str = "\x1b[>4;1m";
const MODIFY_OTHER_KEYS_OFF: &str = "\x1b[>4m";

/// Ask the terminal to report modifiers on Enter so Shift-Enter can be told apart.
/// Both requests are sent unconditionally: the kitty "supports?" query fails behind
/// tmux/screen even when the outer terminal could help, and terminals ignore requests
/// they don't understand. Alt-Enter and Ctrl-J always work as fallbacks.
fn enable_shift_enter() {
    use ratatui::crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    let _ = out.write_all(MODIFY_OTHER_KEYS_ON.as_bytes());
    let _ = out.flush();
}

fn disable_shift_enter() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(MODIFY_OTHER_KEYS_OFF.as_bytes());
    let _ = execute!(out, ratatui::crossterm::event::PopKeyboardEnhancementFlags);
    let _ = out.flush();
}

/// Shift-Enter (needs keyboard enhancement) or Alt-Enter (works everywhere) inserts a newline.
fn is_newline_key(k: &event::KeyEvent) -> bool {
    k.code == KeyCode::Enter
        && k.modifiers
            .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT)
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;
    #[test]
    fn osc52_base64() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
        assert_eq!(osc52("abc"), "\x1b]52;c;YWJj\x07");
        assert_eq!(osc52("a"), "\x1b]52;c;YQ==\x07");
    }

    #[test]
    fn shift_or_alt_enter_is_newline_plain_enter_submits() {
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(!is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
    }
}

#[cfg(test)]
mod legacy_env_tests {
    use super::legacy_env_updates;
    use std::collections::HashMap;

    fn updates(vars: &[(&str, &str)]) -> Vec<(&'static str, String)> {
        let m: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        legacy_env_updates(|k| m.get(k).cloned())
    }

    #[test]
    fn gb_names_fall_back_onto_grokbot_tui_names() {
        assert_eq!(
            updates(&[("GB_GROKBOT_TOKEN", "t"), ("GB_ACCOUNT", "work")]),
            vec![
                ("GROKBOT_TUI_TOKEN", "t".to_string()),
                ("GROKBOT_TUI_ACCOUNT", "work".to_string()),
            ]
        );
        // Every GB_ name has a GROKBOT_TUI_ replacement; a redundant GROKBOT_ is dropped
        // (GB_GROKBOT_TOKEN -> GROKBOT_TUI_TOKEN).
        for (old, new) in super::LEGACY_ENV
            .iter()
            .filter(|(o, _)| o.starts_with("GB_"))
        {
            let rest = old.strip_prefix("GB_").unwrap();
            let rest = rest.strip_prefix("GROKBOT_").unwrap_or(rest);
            assert_eq!(new.strip_prefix("GROKBOT_TUI_"), Some(rest), "{old}");
        }
        assert!(
            !super::LEGACY_ENV
                .iter()
                .any(|(o, n)| o.contains("TUI_GROKBOT") || n.contains("TUI_GROKBOT"))
        );
    }

    #[test]
    fn the_new_name_wins_when_both_are_set() {
        assert!(
            updates(&[
                ("GB_CURSOR_CLIENT_VERSION", "1.0"),
                ("GROKBOT_TUI_CURSOR_CLIENT_VERSION", "2.0")
            ])
            .is_empty()
        );
        // Even an empty new value wins.
        assert!(updates(&[("GB_ACCOUNT", "work"), ("GROKBOT_TUI_ACCOUNT", "")]).is_empty());
    }

    #[test]
    fn library_names_are_fed_from_new_or_old_names() {
        let lib = vec![
            ("GROKBOT_ALLOW_LOCAL_GATEWAY", "1".to_string()),
            ("GROKBOT_APP_DIR", "/opt/app".to_string()),
        ];
        assert_eq!(
            updates(&[
                ("GROKBOT_TUI_ALLOW_LOCAL_GATEWAY", "1"),
                ("GROKBOT_TUI_APP_DIR", "/opt/app")
            ]),
            lib
        );
        // Old GB_ names chain through the GROKBOT_TUI_ names to the library's.
        let mut chained = vec![
            ("GROKBOT_TUI_ALLOW_LOCAL_GATEWAY", "1".to_string()),
            ("GROKBOT_TUI_APP_DIR", "/opt/app".to_string()),
        ];
        chained.extend(lib);
        assert_eq!(
            updates(&[
                ("GB_ALLOW_LOCAL_GATEWAY", "1"),
                ("GB_GROKBOT_APP_DIR", "/opt/app")
            ]),
            chained
        );
        // A library name that is already set wins over both.
        assert_eq!(
            updates(&[
                ("GROKBOT_TUI_ALLOW_LOCAL_GATEWAY", "1"),
                ("GROKBOT_ALLOW_LOCAL_GATEWAY", "")
            ]),
            vec![]
        );
        // The new GROKBOT_TUI_ name beats the old GB_ one on the way to the library.
        assert_eq!(
            updates(&[
                ("GB_GROKBOT_APP_DIR", "/old"),
                ("GROKBOT_TUI_APP_DIR", "/new")
            ]),
            vec![("GROKBOT_APP_DIR", "/new".to_string())]
        );
    }

    #[test]
    fn empty_or_missing_old_values_are_ignored() {
        assert!(updates(&[("GB_GROKBOT_APP_DIR", "")]).is_empty());
        assert!(updates(&[]).is_empty());
    }
}
