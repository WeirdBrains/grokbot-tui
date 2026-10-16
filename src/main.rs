//! gb — terminal UI for Grok Bot.
mod api;
mod app;
mod commands;
mod config;
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
use std::time::Duration;
use storage::Store;

#[derive(Parser)]
#[command(name = "gb", version, about = "Terminal UI for Grok Bot")]
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
    let custom = cli.data_dir.is_some();
    let store = Store::open(
        match cli.data_dir {
            Some(d) => d,
            None => Store::default_dir()?,
        },
        cli.account.as_deref(),
    )?;
    // One-time best-effort import from the pre-SQLite location.
    let imported = if custom {
        None
    } else {
        dirs::home_dir().and_then(|h| store.import_legacy(&h.join(".grokbot-tui")))
    };
    let rt = tokio::runtime::Runtime::new()?;
    let mut app = App::new(store, rt.handle().clone(), cli.mock)?;
    if let Some(msg) = imported {
        app.info(msg);
    }
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
        term.draw(|f| ui::draw(f, app))?;
        if !event::poll(Duration::from_millis(250))? {
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
                        if app.mode == app::InputMode::ConfirmRemove {
                            app.mode = app::InputMode::Normal;
                            app.pending_remove = None;
                            app.input.clear();
                            app.info("Account removal cancelled.".into());
                        } else if app.mode == app::InputMode::Token {
                            app.mode = app::InputMode::Normal;
                            app.input.clear();
                            app.info("Login cancelled.".into());
                        }
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
                MouseEventKind::Down(MouseButton::Left) => app.click(m.column, m.row),
                MouseEventKind::ScrollUp => app.scroll_up(3),
                MouseEventKind::ScrollDown => app.scroll_down(3),
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}
