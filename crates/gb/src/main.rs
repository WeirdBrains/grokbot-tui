//! grokbot-tui — terminal UI for Grok Bot.
mod api;
mod app;
mod commands;
mod config;
mod help;
mod storage;
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
    #[arg(long, env = "GB_ACCOUNT", value_name = "ALIAS")]
    account: Option<String>,
    /// Data directory holding context.db (default ~/.gb).
    #[arg(long, value_name = "DIR")]
    data_dir: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(
        match cli.data_dir {
            Some(d) => d,
            None => Store::default_dir()?,
        },
        cli.account.as_deref(),
    )?;
    let rt = tokio::runtime::Runtime::new()?;
    let mut app = App::new(store, rt.handle().clone(), cli.mock)?;
    app.init();

    let mut term = ratatui::init();
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let res = run(&mut term, &mut app);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    res
}

fn run(term: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        app.tick();
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
                    KeyCode::Enter => app.submit(),
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Up => app.scroll_up(1),
                    KeyCode::Down => app.scroll_down(1),
                    KeyCode::Char(c) => app.input.push(c),
                    _ => {}
                }
            }
            Event::Paste(s) => app.input.push_str(s.trim_end_matches(['\r', '\n'])),
            Event::Mouse(m) => match m.kind {
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
