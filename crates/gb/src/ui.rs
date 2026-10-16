//! Rendering: top bar, left sidebar, body, bottom input bar.
use crate::app::{App, EntryKind, Focus, InputMode};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

pub fn draw(f: &mut Frame, app: &mut App) {
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .areas(f.area());
    let [side, body] = Layout::horizontal([
        Constraint::Length(app.config.sidebar_width),
        Constraint::Min(10),
    ])
    .areas(mid);

    // Top bar
    let user = match (&app.user_label, app.connecting) {
        (Some(l), _) => l.as_str(),
        (None, true) => "connecting…",
        (None, false) => "not logged in",
    };
    let bot = app.active_bot().map(|b| b.name.as_str()).unwrap_or("none");
    let mut spans = vec![
        Span::styled(
            " grokbot-tui ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  account: {}   login: {user}   bot: {bot}",
            app.account().alias
        )),
    ];
    if app.mock {
        spans.push(Span::styled(
            "   [MOCK]",
            Style::default().fg(Color::Yellow),
        ));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray)),
        top,
    );

    // Sidebar: clickable account header (cycles accounts) above the bot list.
    let [acct, side] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(side);
    app.account_area = acct;
    let n_accounts = app.store.accounts().map(|a| a.len()).unwrap_or(1);
    let hint = if n_accounts > 1 { " ⇄" } else { "" };
    f.render_widget(
        Paragraph::new(format!(" @{}{hint}", app.account().alias)).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        acct,
    );
    app.sidebar_area = side;
    let items: Vec<ListItem> = app
        .bots
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let mut s = Style::default();
            if Some(i) == app.active {
                s = s
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD);
            } else if app.focus == Focus::Sidebar && i == app.sidebar_cursor {
                s = s.add_modifier(Modifier::REVERSED);
            }
            ListItem::new(format!(" {}", b.name)).style(s)
        })
        .collect();
    let side_border = if app.focus == Focus::Sidebar {
        Color::Cyan
    } else {
        Color::Gray
    };
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Bots ")
                .border_style(Style::default().fg(side_border)),
        ),
        side,
    );

    // Body
    let bot_name = app.active_bot().map(|b| b.name.clone());
    // Chat feed (from SQLite), then transient notices in their own section.
    let mut lines: Vec<Line> = app
        .feed()
        .iter()
        .flat_map(|e| entry_lines(e.kind, &e.text, bot_name.as_deref(), true))
        .collect();
    if !app.notices().is_empty() {
        if !lines.is_empty() {
            lines.push(Line::styled(
                "── notices (not saved) ──",
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.extend(
            app.notices()
                .iter()
                .flat_map(|e| entry_lines(e.kind, &e.text, bot_name.as_deref(), false)),
        );
    }
    if lines.is_empty() {
        lines = empty_state(app)
            .into_iter()
            .map(|t| Line::styled(t, Style::default().fg(Color::DarkGray)))
            .collect();
    }
    let inner_w = body.width.saturating_sub(2).max(1) as usize;
    let inner_h = body.height.saturating_sub(2);
    // Exact wrapped height (word wrap can need more rows than width/cols), so
    // "scrolled to bottom" really shows the newest line.
    let total: usize = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(inner_w as u16);
    let max_off = (total as u16).saturating_sub(inner_h);
    let scroll = app.scroll.min(max_off);
    let off = max_off - scroll;
    let title = match &bot_name {
        Some(n) if app.is_typing() => {
            const SPIN: [char; 4] = ['⠋', '⠙', '⠹', '⠸'];
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!(
                " Chat: {n} · {n} is replying {} ",
                SPIN[(ms / 150 % 4) as usize]
            )
        }
        Some(n) => format!(" Chat: {n} "),
        None => " Output ".into(),
    };
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((off, 0))
            .block(Block::default().borders(Borders::ALL).title(title)),
        body,
    );
    app.scroll = scroll;

    // Bottom input bar
    let (shown, title) = match app.mode {
        InputMode::GatewayToken => (
            "*".repeat(app.input.chars().count()),
            " Gateway token (masked, Esc cancels) ",
        ),
        InputMode::CursorToken => (
            "*".repeat(app.input.chars().count()),
            " Cursor access token (masked, Esc cancels) ",
        ),
        InputMode::GatewayUrl => (app.input.clone(), " Gateway URL (Esc cancels) "),
        InputMode::Normal => (
            app.input.clone(),
            " /command or message · F1 or ? help · /help login ",
        ),
        InputMode::ConfirmRemove => (
            app.input.clone(),
            " Type the account alias to confirm removal (Esc cancels) ",
        ),
    };
    let border = if app.focus == Focus::Input {
        Color::Cyan
    } else {
        Color::Gray
    };
    f.render_widget(
        Paragraph::new(format!("> {shown}")).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(border)),
        ),
        bottom,
    );
    if app.help.is_some() {
        draw_help(f, app);
    } else if app.focus == Focus::Input {
        let x = bottom.x + 3 + shown.chars().count() as u16;
        f.set_cursor_position(Position::new(
            x.min(bottom.right().saturating_sub(2)),
            bottom.y + 1,
        ));
    }
}

/// Guidance shown when the body has nothing in it yet.
pub fn empty_state(app: &App) -> Vec<String> {
    let mut v = vec![];
    if app.user_label.is_none() && !app.connecting {
        v.push("Welcome to grokbot-tui — you're not logged in.".into());
        v.push(String::new());
        v.push("  1. Sign in to the Grok Bot desktop app on this computer.".into());
        v.push("  2. Type /login desktop and press Enter (recommended).".into());
        v.push(String::new());
        v.push("Other ways: /help login · All keys: F1 · Everything: /help".into());
    } else if app.connecting {
        v.push("Connecting…".into());
    } else if app.bots.is_empty() {
        v.push("No bots yet. Create one in the Grok Bot app, then /bots.".into());
        v.push("See /help sync.".into());
    } else if app.active.is_none() {
        v.push("Pick a bot on the left (click, or Tab then ↑/↓ + Enter).".into());
    } else {
        v.push("No messages yet. Type a message and press Enter.".into());
        v.push("/history reloads the transcript · /help delivery".into());
    }
    v
}

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = (area.width * pct_w / 100).max(20.min(area.width));
    let h = (area.height * pct_h / 100).max(8.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// The help modal: bordered popup, distinct colours, related links at the bottom.
fn draw_help(f: &mut Frame, app: &mut App) {
    let Some(m) = &mut app.help else { return };
    let area = centered(f.area(), 80, 85);
    let [text_area, links_area] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);
    let inner_w = text_area.width.saturating_sub(2).max(1) as usize;
    let total: usize = Paragraph::new(m.page.body.as_str())
        .wrap(Wrap { trim: false })
        .line_count(inner_w as u16);
    let max = (total as u16).saturating_sub(text_area.height.saturating_sub(2));
    m.scroll = m.scroll.min(max);
    let style = Style::default().bg(Color::Black).fg(Color::White);
    f.render_widget(Clear, area);
    let back = if m.back.is_empty() {
        ""
    } else {
        " · Backspace back"
    };
    f.render_widget(
        Paragraph::new(m.page.body.as_str())
            .wrap(Wrap { trim: false })
            .scroll((m.scroll, 0))
            .style(style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(Span::styled(
                        format!(" Help: {} ", m.page.title),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(format!(" ↑↓ scroll · Esc close{back} ")),
            ),
        text_area,
    );
    let mut spans = vec![Span::raw("Related: ")];
    for (i, id) in m.page.related.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        let st = if i == m.link {
            Style::default().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED)
        };
        spans.push(Span::styled(crate::help::link_label(id), st));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans))
            .wrap(Wrap { trim: true })
            .style(style)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(match &m.query {
                        Some(q) => format!(" Search: {q}▏ (Enter run · Esc cancel) "),
                        None => " Tab/←→ select · Enter open · / search ".into(),
                    }),
            ),
        links_area,
    );
}

/// Lines for one entry. In the chat feed (`feed`), the user's own messages are
/// right-aligned like a typical chat UI; everything else stays left-aligned.
fn entry_lines<'a>(kind: EntryKind, text: &'a str, bot: Option<&str>, feed: bool) -> Vec<Line<'a>> {
    if feed && kind == EntryKind::User {
        let style = Style::default().fg(Color::Green);
        let mut out = vec![
            Line::from(Span::styled("you", style.add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Right),
        ];
        out.extend(
            text.lines()
                .map(|l| Line::from(Span::styled(l, style)).alignment(Alignment::Right)),
        );
        return out;
    }
    let (prefix, style) = match kind {
        EntryKind::System => ("".to_string(), Style::default().fg(Color::Gray)),
        EntryKind::Error => ("! ".to_string(), Style::default().fg(Color::Red)),
        EntryKind::User => ("you: ".to_string(), Style::default().fg(Color::Green)),
        EntryKind::Bot => (
            format!("{}: ", bot.unwrap_or("bot")),
            Style::default().fg(Color::Cyan),
        ),
    };
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            let p = if i == 0 {
                prefix.clone()
            } else {
                " ".repeat(prefix.len())
            };
            Line::from(vec![
                Span::styled(p, style.add_modifier(Modifier::BOLD)),
                Span::styled(l, style),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn renders_bars() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("grokbot-tui"), "{text}");
        assert!(text.contains("account: default"));
        assert!(text.contains("login: gateway · box.cursorvm.com"), "{text}");
        assert!(text.contains("@default"));
        assert!(text.contains("bot: Helper"));
        assert!(text.contains("Coder"));
        assert_eq!(app.sidebar_area.width, 24);
    }

    #[test]
    fn help_modal_and_empty_state_render() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        assert!(
            empty_state(&app)
                .iter()
                .any(|l| l.contains("/login desktop"))
        );
        app.open_help("login");
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("Help: Logging in"));
        assert!(screen.contains("Related:"));
        assert!(screen.contains("/help login desktop"));
    }

    #[test]
    fn newest_message_visible_at_bottom_with_word_wrap() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let bot = app.active_bot().unwrap().id.clone();
        // Many long messages of short words: word wrap needs more rows than width/cols.
        for i in 0..30 {
            let text = format!("m{i} ") + &"abcdefg hij klmnop qrs tuvw xyz ".repeat(12);
            app.store
                .append_message(
                    &bot,
                    &crate::api::Message {
                        role: crate::api::Role::Bot,
                        text,
                        remote_id: Some(format!("r{i}")),
                    },
                )
                .unwrap();
        }
        app.store
            .append_message(
                &bot,
                &crate::api::Message {
                    role: crate::api::Role::Bot,
                    text: "NEWEST-LINE".into(),
                    remote_id: Some("rz".into()),
                },
            )
            .unwrap();
        app.refresh_feed_from_db();
        app.notices.clear();
        let mut term = Terminal::new(TestBackend::new(90, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            screen.contains("NEWEST-LINE"),
            "newest message must be on screen"
        );
        // The user's own message: pending row in SQLite, shown at once at the bottom,
        // right-aligned.
        crate::app::tests::type_line(&mut app, "MY-OWN-MSG");
        app.notices.clear();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let row = (0..30u16)
            .find(|&y| {
                (0..90u16)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .contains("MY-OWN-MSG")
            })
            .expect("sent message must be on screen");
        let line: String = (0..90u16).map(|x| buf[(x, row)].symbol()).collect();
        let end = line.find("MY-OWN-MSG").unwrap() + "MY-OWN-MSG".len();
        assert!(
            line[end..].trim_end_matches(['│', ' ']).is_empty(),
            "right-aligned: {line:?}"
        );
        assert!(
            line.find("MY-OWN-MSG").unwrap() > 50,
            "right side: {line:?}"
        );
    }
}
