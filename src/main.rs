//! grokbot-tui — terminal UI for Grok Bot.
mod api;
mod app;
mod appearance;
mod commands;
mod complete;
mod config;
mod datadir;
mod emoji;
mod fuzzy;
mod help;
mod history;
mod imageview;
mod input;
mod keyctx;
mod keymap;
mod keys;
mod markdown;
mod menubar;
mod notify;
mod notify_rules;
mod picker;
mod select;
mod storage;
mod theme;
mod theme_editor;
mod themes;
mod tooltip;
mod ui;

use anyhow::Result;
use app::{App, Focus};
use clap::Parser;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
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
    /// Use this theme (a file in <data dir>/themes, without .toml) for this run only; the
    /// ui.theme pref, F5 or /theme use set it for good.
    #[arg(long, value_name = "NAME")]
    theme: Option<String>,
    /// List the themes in <data dir>/themes (● marks the one in use; broken files say why)
    /// and exit.
    #[arg(long)]
    list_themes: bool,
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
    let themes_dir = themes::user_dir(&store.dir);
    if cli.list_themes || (cli.theme.is_some() && !cli.mock) {
        // First run (no themes yet): fetch the default before listing or checking, waiting
        // at most CLI_DOWNLOAD_TIMEOUT. Never in mock mode for --theme (the app doesn't
        // download there either).
        let allowed = !themes::opted_out(std::env::var(themes::NO_DOWNLOAD_ENV).ok().as_deref())
            && store.load_config()?.theme_download;
        let base = allowed.then(themes::download_base_now);
        match themes::first_run_download(&themes_dir, base.as_deref(), themes::CLI_DOWNLOAD_TIMEOUT)
        {
            themes::FirstRun::NotNeeded => {}
            themes::FirstRun::Off => eprintln!(
                "grokbot-tui: no themes in {} and theme downloads are off ({}=1 or \
                 ui.theme_download = false); plain terminal colors are used.",
                themes_dir.display(),
                themes::NO_DOWNLOAD_ENV
            ),
            themes::FirstRun::Downloaded(p) => {
                eprintln!(
                    "grokbot-tui: downloaded the default theme into {}",
                    p.display()
                )
            }
            themes::FirstRun::Failed(e) => eprintln!(
                "grokbot-tui: warning: {e}; plain terminal colors are used (/theme init retries)."
            ),
        }
    }
    if cli.list_themes {
        let current = cli.theme.clone().unwrap_or(store.load_config()?.theme);
        println!("{}", themes::list_text(&themes_dir, &current));
        return Ok(());
    }
    if let Some(t) = &cli.theme {
        check_theme_flag(&themes_dir, t)?;
    }
    let rt = tokio::runtime::Runtime::new()?;
    let mut app = App::new(store, rt.handle().clone(), cli.mock)?;
    app.init();
    if let Some(t) = cli.theme {
        app.info(format!(
            "Theme '{t}' for this run (--theme); the saved choice (ui.theme = {}) is unchanged.",
            app.config.theme
        ));
        app.config.theme = t.clone();
        app.theme_override = Some(t);
    }
    // Download the default theme if the user has none, then load the selected theme.
    app.init_themes();
    if let Some(n) = notice {
        app.info(n);
    }

    let mut term = ratatui::init();
    // Raw mode is on and nothing reads input yet: ask the terminal for its background
    // (for the "system" theme mode) before mouse capture and the event loop start.
    app.detected_variant = appearance::detect();
    app.image_picker = imageview::detect_picker();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let enhancement = enable_shift_enter();
    app.keyboard_enhancement = enhancement;
    app.maybe_apple_terminal_hint();
    // Pop enhancement / paste even on panic.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_shift_enter();
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        hook(info);
    }));
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut term, &mut app)));
    disable_shift_enter();
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    match res {
        Ok(r) => r,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// `--theme <name>`: refuse a bad name, or a theme that doesn't exist or can't be used,
/// before the terminal UI starts. `default` may be missing from an empty themes folder:
/// the first-run download fetches it.
fn check_theme_flag(dir: &std::path::Path, name: &str) -> Result<()> {
    if !themes::valid_id(name) {
        anyhow::bail!("--theme {name:?}: theme names are 1-32 of a-z, 0-9, - and _");
    }
    match themes::load(dir, name) {
        Ok(_) => Ok(()),
        Err(themes::LoadError::Missing(_))
            if name == themes::DEFAULT_ID && themes::list(dir).is_empty() =>
        {
            Ok(())
        }
        Err(themes::LoadError::Missing(p)) => {
            let have = themes::list(dir);
            anyhow::bail!(
                "--theme {name}: {} doesn't exist (themes: {})",
                p.display(),
                if have.is_empty() {
                    "none".into()
                } else {
                    have.join(", ")
                }
            )
        }
        Err(themes::LoadError::Broken(p, e)) => {
            anyhow::bail!("--theme {name}: {} can't be used: {e}", p.display())
        }
    }
}

fn run(term: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut captured = true;
    let mut dirty = true;
    let mut last_draw = std::time::Instant::now();
    let mut esc_meta = crate::keys::EscMeta::default();
    while !app.should_quit {
        app.tick();
        let now = std::time::Instant::now();
        if let Some(k) = esc_meta.poll_timeout(now) {
            dirty |= handle_event(app, Event::Key(k));
        }
        if app.tooltip.tick(now, crate::tooltip::tip_for) {
            dirty = true;
        }
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
        // Draw after input changed something, else at the UI tick (spinners, replies
        // arriving from background tasks).
        if app.needs_clear {
            app.needs_clear = false;
            if let Some(seq) = imageview::kitty_delete_all(&app.image_picker) {
                use std::io::Write;
                let _ = std::io::stdout().write_all(seq.as_bytes());
            }
            term.clear()?;
            dirty = true;
        }
        if dirty || last_draw.elapsed() >= app::UI_TICK {
            let frame = term.draw(|f| ui::draw(f, app))?;
            if hyperlinks_on() && !app.link_hits.is_empty() {
                let _ = write_hyperlinks(frame.buffer, &app.link_hits);
            }
            dirty = false;
            last_draw = std::time::Instant::now();
        }
        if !event::poll(app::UI_TICK.saturating_sub(last_draw.elapsed()))? {
            continue;
        }
        // Handle everything already queued before drawing again. Mouse capture reports
        // every pointer move (xterm mode 1003), so a click used to wait behind one full
        // redraw per queued move.
        let first = event::read()?;
        let batch = read_batch(first, || {
            Ok(if event::poll(std::time::Duration::ZERO)? {
                Some(event::read()?)
            } else {
                None
            })
        })?;
        for ev in coalesce(batch) {
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let now = std::time::Instant::now();
                    match esc_meta.feed(k, now) {
                        crate::keys::Feed::Hold => {}
                        crate::keys::Feed::Key(k) => {
                            dirty |= handle_event(app, Event::Key(k));
                        }
                        crate::keys::Feed::EscThen(k) => {
                            dirty |= handle_event(
                                app,
                                Event::Key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                            );
                            dirty |= handle_event(app, Event::Key(k));
                        }
                    }
                }
                Event::Paste(text) => {
                    // Bracketed paste: keep newlines; never submit.
                    if app.focus == Focus::Input && app.mode == app::InputMode::Normal {
                        app.input_insert(&text);
                        dirty = true;
                    }
                }
                ev => {
                    dirty |= handle_event(app, ev);
                }
            }
            if app.should_quit {
                break;
            }
        }
    }
    Ok(())
}

/// What a Ctrl key that could end the program does.
#[derive(Debug, PartialEq, Eq)]
enum QuitKey {
    /// Ctrl-Q: quit right away.
    Quit,
    /// Ctrl-Shift-C (terminals' copy shortcut): copy, never quit.
    Copy,
    CtrlC,
    CtrlD,
}

/// Classify Ctrl-C / Ctrl-Shift-C / Ctrl-D / Ctrl-Q. Shift is checked first, so the copy
/// shortcut can't fall through to Ctrl-C, whichever case the terminal reports the letter in.
fn quit_key(k: &ratatui::crossterm::event::KeyEvent) -> Option<QuitKey> {
    if !k.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    let KeyCode::Char(c) = k.code else {
        return None;
    };
    let shift = k.modifiers.contains(KeyModifiers::SHIFT) || c.is_ascii_uppercase();
    match (c.to_ascii_lowercase(), shift) {
        ('c', true) => Some(QuitKey::Copy),
        ('c', false) => Some(QuitKey::CtrlC),
        ('d', false) => Some(QuitKey::CtrlD),
        ('q', false) => Some(QuitKey::Quit),
        _ => None,
    }
}

/// Most events read in one go before a redraw (so a flood can't starve the screen).
const MAX_BATCH: usize = 4096;

/// `first` plus every event `more` returns without waiting, up to [`MAX_BATCH`].
fn read_batch(first: Event, mut more: impl FnMut() -> Result<Option<Event>>) -> Result<Vec<Event>> {
    let mut v = vec![first];
    while v.len() < MAX_BATCH {
        match more()? {
            Some(e) => v.push(e),
            None => break,
        }
    }
    Ok(v)
}

/// Drop pointer motion nobody needs: of the moves and of the drags only the last one
/// matters (pick-list hover / selection). Everything else keeps its order.
fn coalesce(batch: Vec<Event>) -> Vec<Event> {
    let is_move = |e: &Event| matches!(e, Event::Mouse(m) if m.kind == MouseEventKind::Moved);
    let is_drag =
        |e: &Event| matches!(e, Event::Mouse(m) if matches!(m.kind, MouseEventKind::Drag(_)));
    let last_move = batch.iter().rposition(is_move);
    let last_drag = batch.iter().rposition(is_drag);
    batch
        .into_iter()
        .enumerate()
        .filter(|(i, e)| match e {
            e if is_drag(e) => Some(*i) == last_drag,
            e if is_move(e) => Some(*i) == last_move,
            _ => true,
        })
        .map(|(_, e)| e)
        .collect()
}

/// Apply one terminal event. Returns whether the screen may have changed (so a redraw is
/// due); pointer moves outside a pick list, button releases and key releases don't.
fn tooltip_hit(
    app: &App,
    col: u16,
    row: u16,
) -> Option<(crate::tooltip::Hit, ratatui::layout::Rect)> {
    use crate::tooltip::Hit;
    let at = ratatui::layout::Position::new(col, row);
    if app.unread_area.contains(at) {
        return Some((Hit::UnreadBadge, app.unread_area));
    }
    if let Some(i) = app.menu.hit_bar(col, row) {
        return Some((Hit::MenuBar(i), app.menu.bar_areas[i]));
    }
    if let Some(i) = app.menu.hit_opt(col, row) {
        return Some((Hit::MenuOpt(i), app.menu.opt_areas[i]));
    }
    if app.theme_switch_area.contains(at) {
        return Some((Hit::ThemeSwitch, app.theme_switch_area));
    }
    if app.account_area.contains(at) {
        return Some((Hit::AccountSwitch, app.account_area));
    }
    None
}

fn handle_event(app: &mut App, ev: Event) -> bool {
    if let Event::Mouse(m) = &ev {
        app.mouse_pos = Some((m.column, m.row));
        if matches!(m.kind, MouseEventKind::Down(_)) {
            app.quit_armed = None;
        }
    }
    match ev {
        Event::Mouse(m) if m.kind == MouseEventKind::Moved => {
            app.mouse_pos = Some((m.column, m.row));
            if app.picker.is_some() {
                // picker hover handled below
            } else {
                let hit = tooltip_hit(app, m.column, m.row);
                let now = std::time::Instant::now();
                let changed = app.tooltip.pointer(hit, now);
                // Coalesced moves: only redraw when the tip changes.
                if app.body_sel.is_none() {
                    return changed;
                }
            }
        }
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Drag(_)) && app.body_sel.is_some() => {
            app.body_drag(m.column, m.row);
            return true;
        }
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Drag(_)) => {
            return false;
        }
        Event::Mouse(m) if matches!(m.kind, MouseEventKind::Up(_)) => {
            if app.body_sel.as_ref().is_some_and(|s| !s.is_empty()) {
                app.body_mouse_up();
                return true;
            }
            return false;
        }
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            if let Some(q) = quit_key(&k) {
                let now = std::time::Instant::now();
                match q {
                    QuitKey::Quit => app.should_quit = true,
                    QuitKey::Copy => app.ctrl_c(true, now),
                    QuitKey::CtrlC => app.ctrl_c(false, now),
                    // Ctrl-D: the theme editor's duplicate; delete forward while there's
                    // text; else like Ctrl-C.
                    QuitKey::CtrlD if app.editor.is_some() && app.picker.is_none() => {
                        app.quit_armed = None;
                        app.editor_key(k)
                    }
                    QuitKey::CtrlD if !app.input.is_empty() && !app.overlay_open() => {
                        app.quit_armed = None;
                        app.input_delete()
                    }
                    QuitKey::CtrlD => app.ctrl_c(false, now),
                }
                return true;
            }
            // Anything else done in between disarms "press Ctrl-C again".
            app.quit_armed = None;

            // Vim keymap (opt-in via ui.keys=vim). Overlays keep their own keys.
            if app.keys_are_vim()
                && app.editor.is_none()
                && app.picker.is_none()
                && app.help.is_none()
                && app.shell.is_none()
                && app.history_search.is_none()
            {
                use crate::keymap::{self, Mode};
                if app.vim_mode == Mode::Command {
                    if let Some(action) = keymap::encode_key(k.code, k.modifiers)
                        .and_then(|key| app.keymap.lookup(Mode::Command, &key).map(str::to_string))
                    {
                        app.vim_action(&action);
                        return true;
                    }
                    match k.code {
                        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                            if let Some(cmd) = app.vim_command.as_mut() {
                                cmd.push(c);
                            }
                            return true;
                        }
                        KeyCode::Backspace => {
                            if let Some(cmd) = app.vim_command.as_mut() {
                                cmd.pop();
                            }
                            return true;
                        }
                        _ => {}
                    }
                } else if app.vim_mode != Mode::Insert {
                    if let Some(action) = keymap::encode_key(k.code, k.modifiers)
                        .and_then(|key| app.keymap.lookup(app.vim_mode, &key).map(str::to_string))
                    {
                        app.vim_action(&action);
                        return true;
                    }
                } else if let Some(action) = keymap::encode_key(k.code, k.modifiers)
                    .filter(|key| {
                        key == "esc" || key.starts_with("ctrl-") || key.starts_with("alt-")
                    })
                    .and_then(|key| app.keymap.lookup(Mode::Insert, &key).map(str::to_string))
                {
                    app.vim_action(&action);
                    return true;
                }
            }

            if let Some(p) = app.picker.as_mut() {
                let before = p.cursor;
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
                app.picker_moved(before);
                return true;
            }
            if app.editor.is_some() {
                app.editor_key(k);
                return true;
            }
            if app.history_search.is_some() {
                app.history_search_key(k);
                return true;
            }
            if k.code == KeyCode::F(10)
                || (k.code == KeyCode::Char('m')
                    && k.modifiers.contains(KeyModifiers::ALT)
                    && app.menu.open.is_none())
            {
                app.menu.open_menu(0);
                app.tooltip.dismiss();
                return true;
            }
            if app.menu.open.is_some() {
                match k.code {
                    KeyCode::Esc => app.menu.close(),
                    KeyCode::Left | KeyCode::Char('h') => app.menu.move_bar(-1),
                    KeyCode::Right | KeyCode::Char('l') => app.menu.move_bar(1),
                    KeyCode::Up | KeyCode::Char('k') => app.menu.move_opt(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.menu.move_opt(1),
                    KeyCode::Enter => {
                        if let Some(cmd) = app.menu.selected_command() {
                            app.run_menu_command(cmd);
                        }
                    }
                    KeyCode::Char(c) => {
                        if let Some(i) = crate::menubar::MENUS
                            .iter()
                            .position(|m| m.hint == c.to_ascii_lowercase())
                        {
                            app.menu.open_menu(i);
                        }
                    }
                    _ => {}
                }
                return true;
            }
            if app.completion.is_some() {
                crate::complete::key(app, k);
                return true;
            }
            if k.code == KeyCode::F(6) {
                app.toggle_mouse_capture();
                return true;
            }
            // Shift-F5 (most terminals send F5 + Shift; some send F17): theme editor.
            if (k.code == KeyCode::F(5) && k.modifiers.contains(KeyModifiers::SHIFT))
                || k.code == KeyCode::F(17)
            {
                app.open_theme_editor(None);
                return true;
            }
            if k.code == KeyCode::F(5) {
                app.open_theme_picker();
                return true;
            }
            if k.code == KeyCode::F(2) {
                app.open_bot_picker();
                return true;
            }
            if k.code == KeyCode::F(3) {
                app.open_account_picker();
                return true;
            }
            if k.code == KeyCode::F(4) {
                app.cycle_theme_mode();
                return true;
            }
            if app.image_view.is_some() {
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => app.close_image_view(),
                    KeyCode::Char('o') | KeyCode::Enter => app.image_open_external(),
                    KeyCode::Right
                    | KeyCode::Char('n')
                    | KeyCode::Char('l')
                    | KeyCode::Char(' ') => app.image_step(1),
                    KeyCode::Left
                    | KeyCode::Char('p')
                    | KeyCode::Char('h')
                    | KeyCode::Backspace => app.image_step(-1),
                    _ => {}
                }
                return true;
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
                return true;
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
                return true;
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
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => app.help_select(-1),
                    KeyCode::Enter => app.help_follow(),
                    KeyCode::Backspace => app.help_back(),
                    _ => {}
                }
                return true;
            }
            if k.code == KeyCode::F(1)
                || (k.code == KeyCode::Char('?')
                    && app.input.is_empty()
                    && app.mode == app::InputMode::Normal)
            {
                app.open_help(if k.code == KeyCode::F(1) { "" } else { "keys" });
                return true;
            }
            match k.code {
                KeyCode::Tab
                    if !(app.focus == Focus::Input
                        && (app.input.starts_with('/')
                            || crate::emoji::shortcode_at(&app.input, app.input_cursor)
                                .is_some())) =>
                {
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
                // End → bottom of chat when reading history or the sidebar has focus;
                // otherwise it moves to the end of the input line (arm below).
                KeyCode::End
                    if app.focus == Focus::Sidebar
                        || (!app.follow && app.mode == app::InputMode::Normal) =>
                {
                    app.follow_bottom()
                }
                KeyCode::Home if app.focus == Focus::Sidebar => {
                    app.scroll_up(u16::MAX / 2);
                }
                KeyCode::Esc => {
                    if app.body_sel.take().is_some() {
                        return true;
                    }
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
                KeyCode::Enter => {
                    if app.mode == app::InputMode::Normal
                        && let Some(next) = crate::keys::trailing_backslash_newline(&app.input)
                    {
                        app.input = next;
                        app.input_cursor = app.input.len();
                        app.input_anchor = None;
                    } else {
                        app.submit();
                    }
                }
                KeyCode::Char('a') if k.modifiers.contains(KeyModifiers::CONTROL) => app.ctrl_a(),
                KeyCode::Char('z') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.input_undo_clear()
                }
                KeyCode::Backspace => app.input_backspace(),
                KeyCode::Delete => app.input_delete(),
                // Shift+movement selects; plain movement drops the selection.
                KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                    if k.modifiers.contains(KeyModifiers::SHIFT) {
                        app.input_extend();
                    } else {
                        app.input_anchor = None;
                    }
                    match k.code {
                        KeyCode::Left => app.input_left(),
                        KeyCode::Right => app.input_right(),
                        KeyCode::Home => app.input_home(),
                        _ => app.input_end(),
                    }
                }
                // Lines first; at the first/last line, prompt history. PageUp/Down scroll the chat.
                KeyCode::Up if app.focus == Focus::Input && app.mode == app::InputMode::Normal => {
                    app.input_anchor = None;
                    app.completion = None;
                    let _ = app.history_up();
                }
                KeyCode::Down
                    if app.focus == Focus::Input && app.mode == app::InputMode::Normal =>
                {
                    app.input_anchor = None;
                    app.completion = None;
                    let _ = app.history_down();
                }
                KeyCode::Up => {
                    app.input_anchor = None;
                    if !app.input_up() {
                        app.scroll_up(1)
                    }
                }
                KeyCode::Down => {
                    app.input_anchor = None;
                    if !app.input_down() {
                        app.scroll_down(1)
                    }
                }
                KeyCode::Char('r')
                    if k.modifiers.contains(KeyModifiers::CONTROL)
                        && app.focus == Focus::Input
                        && app.mode == app::InputMode::Normal
                        && app.editor.is_none() =>
                {
                    app.open_history_search();
                }
                KeyCode::Tab if app.focus == Focus::Input && app.input.starts_with('/') => {
                    crate::complete::tab(app, false);
                }
                KeyCode::BackTab if app.focus == Focus::Input && app.completion.is_some() => {
                    crate::complete::tab(app, true);
                }
                KeyCode::Char(c) => {
                    app.input_insert(&c.to_string());
                    if app.completion.is_some() {
                        crate::complete::refilter(app);
                    }
                }
                _ => {}
            }
        }
        Event::Paste(_) if app.editor.is_some() => {}
        Event::Paste(s) => app.input_insert(s.trim_end_matches(['\r', '\n'])),
        Event::Mouse(m) if app.picker.is_some() => {
            let p = app.picker.as_mut().unwrap();
            let before = p.cursor;
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
            app.picker_moved(before);
        }
        Event::Mouse(m) if app.image_view.is_some() => {
            if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                app.image_view_click(m.column, m.row)
            }
        }
        Event::Mouse(m) if app.history_search.is_some() => app.history_search_mouse(m),
        Event::Mouse(m) if app.editor.is_some() => match m.kind {
            MouseEventKind::Down(MouseButton::Left) => app.editor_click(m.column, m.row),
            MouseEventKind::ScrollUp => app.editor_scroll(-1),
            MouseEventKind::ScrollDown => app.editor_scroll(1),
            _ => {}
        },
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
        // Graphics don't reflow: repaint from scratch so no image ghost survives a resize.
        Event::Resize(..) => app.needs_clear = true,
        _ => return false,
    }
    true
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

/// OSC 8 hyperlinks are on unless `GROKBOT_TUI_NO_HYPERLINKS` is set.
fn hyperlinks_on() -> bool {
    std::env::var_os("GROKBOT_TUI_NO_HYPERLINKS").is_none()
}

/// The OSC 8 open sequence for `url` (control characters stripped so a URL can't end the
/// sequence early or inject others).
fn osc8_open(url: &str) -> String {
    let clean: String = url.chars().filter(|c| !c.is_control()).collect();
    format!("\x1b]8;;{clean}\x1b\\")
}
const OSC8_CLOSE: &str = "\x1b]8;;\x1b\\";

/// After ratatui has drawn a frame, re-print each link's cells (same symbols and colors,
/// taken from the frame buffer) wrapped in OSC 8 so terminals that support hyperlinks
/// (Ghostty, kitty, WezTerm, Windows Terminal, foot, recent Alacritty, iTerm2) make them
/// clickable. ratatui's buffer and width math never see the escapes; the cursor is saved
/// and restored around the pass.
fn write_hyperlinks(buf: &ratatui::buffer::Buffer, hits: &[ui::LinkHit]) -> std::io::Result<()> {
    use ratatui::crossterm::{cursor, queue, style};
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    queue!(out, cursor::SavePosition)?;
    for h in hits {
        queue!(out, cursor::MoveTo(h.x, h.y))?;
        out.write_all(osc8_open(&h.url).as_bytes())?;
        let mut x = h.x;
        while x < h.x + h.w {
            let Some(c) = buf.cell((x, h.y)) else { break };
            let st = c.style();
            queue!(
                out,
                style::SetAttribute(style::Attribute::Reset),
                style::SetForegroundColor(ratatui::backend::IntoCrossterm::into_crossterm(
                    st.fg.unwrap_or(ratatui::style::Color::Reset)
                )),
                style::SetBackgroundColor(ratatui::backend::IntoCrossterm::into_crossterm(
                    st.bg.unwrap_or(ratatui::style::Color::Reset)
                )),
            )?;
            for (m, a) in [
                (ratatui::style::Modifier::BOLD, style::Attribute::Bold),
                (ratatui::style::Modifier::ITALIC, style::Attribute::Italic),
                (
                    ratatui::style::Modifier::UNDERLINED,
                    style::Attribute::Underlined,
                ),
                (
                    ratatui::style::Modifier::CROSSED_OUT,
                    style::Attribute::CrossedOut,
                ),
                (ratatui::style::Modifier::DIM, style::Attribute::Dim),
                (
                    ratatui::style::Modifier::REVERSED,
                    style::Attribute::Reverse,
                ),
            ] {
                if st.add_modifier.contains(m) {
                    queue!(out, style::SetAttribute(a))?;
                }
            }
            let sym = c.symbol();
            out.write_all(sym.as_bytes())?;
            x += (crate::input::str_width(sym) as u16).max(1);
        }
        out.write_all(OSC8_CLOSE.as_bytes())?;
    }
    queue!(
        out,
        style::SetAttribute(style::Attribute::Reset),
        style::ResetColor,
        cursor::RestorePosition
    )?;
    out.flush()
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
/// Push keyboard enhancement when the terminal supports it. Always send modifyOtherKeys
/// level 1 (harmless when ignored). Returns whether kitty-style enhancement is active.
fn enable_shift_enter() -> bool {
    use ratatui::crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
    use std::io::Write;
    let supported = ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    let mut out = std::io::stdout();
    if supported {
        let _ = execute!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    let _ = out.write_all(MODIFY_OTHER_KEYS_ON.as_bytes());
    let _ = out.flush();
    supported
}

fn disable_shift_enter() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(MODIFY_OTHER_KEYS_OFF.as_bytes());
    let _ = execute!(out, ratatui::crossterm::event::PopKeyboardEnhancementFlags);
    let _ = out.flush();
}

fn is_newline_key(k: &event::KeyEvent) -> bool {
    crate::keys::is_newline_key(k)
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
    fn shift_alt_or_ctrl_enter_is_newline_plain_enter_submits() {
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(is_newline_key(&KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL
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

#[cfg(test)]
mod theme_flag_tests {
    use super::*;

    #[test]
    fn theme_flags_parse() {
        let c = Cli::try_parse_from(["grokbot-tui", "--theme", "solar"]).unwrap();
        assert_eq!(c.theme.as_deref(), Some("solar"));
        assert!(!c.list_themes);
        let c = Cli::try_parse_from(["grokbot-tui", "--list-themes"]).unwrap();
        assert!(c.list_themes && c.theme.is_none());
    }

    #[test]
    fn theme_flag_is_checked_before_the_ui() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path();
        // Empty folder: only `default` is allowed (the first-run download brings it).
        assert!(check_theme_flag(dir, "default").is_ok());
        assert!(
            check_theme_flag(dir, "solar")
                .unwrap_err()
                .to_string()
                .contains("themes: none")
        );
        assert!(
            check_theme_flag(dir, "Bad Name")
                .unwrap_err()
                .to_string()
                .contains("theme names")
        );
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("themes/default.toml"),
            themes::path(dir, "default"),
        )
        .unwrap();
        std::fs::write(themes::path(dir, "bad"), "schema = 99\n").unwrap();
        assert!(check_theme_flag(dir, "default").is_ok());
        let e = check_theme_flag(dir, "solar").unwrap_err().to_string();
        assert!(e.contains("themes: bad, default"), "{e}");
        assert!(
            check_theme_flag(dir, "bad")
                .unwrap_err()
                .to_string()
                .contains("can't be used")
        );
    }
}

#[cfg(test)]
mod event_batch_tests {
    use super::*;
    use crate::app::tests::{mock_login, test_app};
    use ratatui::crossterm::event::{KeyEvent, MouseEvent};
    use std::collections::VecDeque;

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn coalesce_keeps_only_the_last_move_and_the_last_drag() {
        let batch = vec![
            mouse(MouseEventKind::Moved, 1, 1),
            mouse(MouseEventKind::Drag(MouseButton::Left), 2, 2),
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            mouse(MouseEventKind::Moved, 3, 3),
            mouse(MouseEventKind::Down(MouseButton::Left), 4, 4),
            mouse(MouseEventKind::Drag(MouseButton::Left), 4, 5),
            mouse(MouseEventKind::Moved, 5, 5),
        ];
        let out = coalesce(batch);
        assert_eq!(
            out,
            vec![
                Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
                mouse(MouseEventKind::Down(MouseButton::Left), 4, 4),
                mouse(MouseEventKind::Drag(MouseButton::Left), 4, 5),
                mouse(MouseEventKind::Moved, 5, 5),
            ]
        );
    }

    #[test]
    fn read_batch_takes_everything_queued_up_to_the_cap() {
        let mut q: VecDeque<Event> = (0..10)
            .map(|i| mouse(MouseEventKind::Moved, i, 0))
            .collect();
        let b = read_batch(Event::FocusGained, || Ok(q.pop_front())).unwrap();
        assert_eq!(b.len(), 11);
        let b = read_batch(Event::FocusGained, || Ok(Some(Event::FocusLost))).unwrap();
        assert_eq!(b.len(), MAX_BATCH, "a flood can't starve the screen");
    }

    #[test]
    fn pointer_moves_outside_a_pick_list_need_no_redraw() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        assert!(!handle_event(&mut app, mouse(MouseEventKind::Moved, 3, 3)));
        assert!(!handle_event(
            &mut app,
            mouse(MouseEventKind::Up(MouseButton::Left), 3, 3)
        ));
        assert!(!handle_event(
            &mut app,
            mouse(MouseEventKind::Drag(MouseButton::Left), 3, 3)
        ));
        assert!(!handle_event(
            &mut app,
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            ))
        ));
        assert!(handle_event(&mut app, Event::Resize(80, 24)));
        // In a pick list the hover follows the pointer, so moves do redraw.
        app.open_theme_picker();
        assert!(handle_event(&mut app, mouse(MouseEventKind::Moved, 3, 3)));
    }

    fn press(app: &mut App, code: KeyCode, m: KeyModifiers) {
        handle_event(app, Event::Key(KeyEvent::new(code, m)));
    }

    #[test]
    fn ctrl_shift_c_copies_and_never_quits() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('c'), cs);
            press(&mut app, KeyCode::Char('C'), cs);
            press(&mut app, KeyCode::Char('C'), KeyModifiers::CONTROL);
        }
        assert!(!app.should_quit);
        assert!(app.quit_armed.is_none(), "copy doesn't arm the quit");
        let n: Vec<String> = app.notices().iter().map(|e| e.text.clone()).collect();
        assert!(
            n.iter().any(|t| t.contains("Nothing selected to copy")),
            "{n:?}"
        );
    }

    #[test]
    fn ctrl_c_twice_quits_and_ctrl_q_quits_at_once() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        // Text in the input: the first Ctrl-C clears it.
        app.input = "draft".into();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.input.is_empty() && !app.should_quit);
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.should_quit, "one press only asks");
        let n: Vec<String> = app.notices().iter().map(|e| e.text.clone()).collect();
        assert!(
            n.iter().any(|t| t.contains("Press Ctrl-C again to quit")),
            "{n:?}"
        );
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.should_quit, "the second press quits");
        // Too slow: the second press asks again.
        let (mut app, _d) = test_app(&rt);
        let t0 = std::time::Instant::now();
        app.ctrl_c(false, t0);
        app.ctrl_c(
            false,
            t0 + app::QUIT_CONFIRM + std::time::Duration::from_millis(1),
        );
        assert!(!app.should_quit);
        app.ctrl_c(
            false,
            t0 + app::QUIT_CONFIRM + std::time::Duration::from_millis(500),
        );
        assert!(app.should_quit);
        // Ctrl-D deletes while there's text, else counts like Ctrl-C.
        let (mut app, _d) = test_app(&rt);
        app.input = "ab".into();
        app.input_cursor = 0;
        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(app.input, "b");
        app.input.clear();
        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(app.should_quit);
        // In the theme editor Ctrl-D still duplicates (asks for a name).
        let (mut app, _d) = test_app(&rt);
        app.open_theme_editor(Some("default"));
        press(&mut app, KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert!(!app.should_quit && app.quit_armed.is_none());
        assert!(matches!(
            app.editor.as_ref().unwrap().edit,
            crate::theme_editor::Edit::Name {
                duplicate: true,
                ..
            }
        ));
        // Ctrl-Q quits directly; plain q / Esc never quit.
        let (mut app, _d) = test_app(&rt);
        press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.should_quit);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_stops_waiting_for_a_reply_first() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        let bot = app.active_bot().unwrap().id.clone();
        app.awaiting.insert(bot, std::time::Instant::now());
        assert!(app.is_typing());
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.is_typing() && !app.should_quit && app.quit_armed.is_none());
    }

    fn notices_text(app: &App) -> String {
        app.notices()
            .iter()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_selection_in_the_input_is_copied_not_cleared() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.input_insert("hello world");
        // Shift-Left ×5 selects "world".
        for _ in 0..5 {
            press(&mut app, KeyCode::Left, KeyModifiers::SHIFT);
        }
        assert_eq!(app.input_selection(), Some((6, 11)));
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.pending_clipboard.as_deref(), Some("world"));
        assert_eq!(app.input, "hello world", "copying never clears");
        assert!(notices_text(&app).contains("Copied 5 chars"));
        // Ctrl-A selects everything; Ctrl-Shift-C copies it; typing replaces it.
        press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
        press(
            &mut app,
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.pending_clipboard.as_deref(), Some("hello world"));
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(app.input, "x");
        assert!(app.input_selection().is_none());
        // Plain movement drops a selection.
        app.input_select_all();
        press(&mut app, KeyCode::Left, KeyModifiers::NONE);
        assert!(app.input_selection().is_none());
        // Ctrl-Shift-C without a selection copies the whole input; Ctrl-C clears it and
        // Ctrl-Z brings it back.
        press(
            &mut app,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.pending_clipboard.as_deref(), Some("x"));
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.input.is_empty());
        press(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(app.input, "x");
        assert!(!app.should_quit);
    }

    #[test]
    fn ctrl_c_closes_overlays_and_hints_over_the_chat() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.open_theme_picker();
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.picker.is_none() && !app.should_quit && app.quit_armed.is_none());
        app.open_help("keys");
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.help.is_none());
        // The mouse rests on the chat: a hint, never a quit.
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let b = app.body_area;
        handle_event(&mut app, mouse(MouseEventKind::Moved, b.x + 3, b.y + 3));
        for _ in 0..3 {
            press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        }
        assert!(!app.should_quit);
        assert!(notices_text(&app).contains("Nothing selected"));
        // Over the header: the double press quits, unless something happens in between.
        handle_event(&mut app, mouse(MouseEventKind::Moved, 0, 0));
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        app.input.clear();
        app.input_cursor = 0;
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(
            !app.should_quit,
            "typing in between disarmed the first press"
        );
        press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(app.should_quit);
    }

    #[test]
    fn input_row_starts_match_the_layout() {
        let s = "abcdef\nxy";
        assert_eq!(crate::input::row_starts(s, 4), vec![0, 4, 7]);
        assert_eq!(crate::input::layout(s, 0, 4).rows, vec!["abcd", "ef", "xy"]);
    }

    /// The reported bug: a click queued behind a burst of pointer moves. All of it is
    /// handled in one batch, so the click takes effect before the next frame.
    #[test]
    fn a_click_behind_hundreds_of_moves_lands_in_one_batch() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        mock_login(&mut app, "t");
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        term.draw(|f| ui::draw(f, &mut app)).unwrap();
        let r = app.theme_switch_area;
        let mut q: VecDeque<Event> = (0..500)
            .map(|i| mouse(MouseEventKind::Moved, (i % 120) as u16, (i % 40) as u16))
            .collect();
        q.push_back(mouse(MouseEventKind::Down(MouseButton::Left), r.x + 1, r.y));
        q.push_back(mouse(MouseEventKind::Up(MouseButton::Left), r.x + 1, r.y));
        let first = q.pop_front().unwrap();
        let batch = coalesce(read_batch(first, || Ok(q.pop_front())).unwrap());
        assert!(batch.len() <= 3, "{batch:?}");
        let mut dirty = false;
        for ev in batch {
            dirty |= handle_event(&mut app, ev);
        }
        assert!(dirty);
        assert!(
            app.picker.is_some(),
            "the click opened the theme picker in the same frame"
        );
    }
}
