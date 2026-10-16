//! Rendering: top bar, left sidebar, body, bottom input bar.
use crate::app::{App, EntryKind, Focus, InputMode};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};

/// Input text as shown (masked for token prompts) and its cursor byte offset.
fn input_shown(app: &App) -> (String, usize) {
    match app.mode {
        InputMode::GatewayToken | InputMode::CursorToken => {
            let n = app.input.chars().count();
            ("*".repeat(n), n)
        }
        _ => (app.input.clone(), app.input_cursor.min(app.input.len())),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    // Input box: char-wrapped to its width, grows to 10 rows, then scrolls.
    let (shown, cur) = input_shown(app);
    let text_w = f.area().width.saturating_sub(4).max(1) as usize; // borders + "> "
    let il = crate::input::layout(&shown, cur, text_w);
    let vis = crate::input::visible_rows(il.rows.len());
    app.input_scroll = crate::input::scroll_for(app.input_scroll, il.cursor.0, il.rows.len(), vis);
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(vis as u16 + 2),
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
    // A blank line separates consecutive messages.
    let mut lines: Vec<Line> = vec![];
    for (i, e) in app.feed().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(entry_lines(e.kind, &e.text, bot_name.as_deref(), true));
    }
    if !app.notices().is_empty() {
        if !lines.is_empty() {
            lines.push(Line::default());
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
    // Inner padding between the border and the content (left/right 2, top 1).
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 0));
    let inner = chat_block.inner(body);
    let inner_w = inner.width.max(1) as usize;
    let inner_h = inner.height;
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
            .block(chat_block.title(title)),
        body,
    );
    app.scroll = scroll;

    // Bottom input bar
    let title = match app.mode {
        InputMode::GatewayToken => " Gateway token (masked, Esc cancels) ",
        InputMode::CursorToken => " Cursor access token (masked, Esc cancels) ",
        InputMode::GatewayUrl => " Gateway URL (Esc cancels) ",
        InputMode::Normal => " message or /command · Shift/Alt-Enter newline · F1 help ",
    };
    let scroll = app.input_scroll;
    let input_lines: Vec<Line> = il
        .rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(vis)
        .map(|(i, r)| Line::raw(format!("{}{r}", if i == 0 { "> " } else { "  " })))
        .collect();
    let more = if il.rows.len() > vis {
        format!(" {}/{} ", il.cursor.0 + 1, il.rows.len())
    } else {
        String::new()
    };
    let border = if app.focus == Focus::Input {
        Color::Cyan
    } else {
        Color::Gray
    };
    f.render_widget(
        Paragraph::new(input_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .title_bottom(Line::from(more).right_aligned())
                .border_style(Style::default().fg(border)),
        ),
        bottom,
    );
    if app.picker.is_some() {
        draw_picker(f, app);
    } else if app.shell.is_some() {
        draw_shell(f, app);
    } else if app.help.is_some() {
        draw_help(f, app);
    } else if app.focus == Focus::Input {
        let (row, col) = il.cursor;
        let x = bottom.x + 3 + col as u16;
        let y = bottom.y + 1 + (row - scroll) as u16;
        f.set_cursor_position(Position::new(x.min(bottom.right().saturating_sub(2)), y));
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

/// The selection modal: same look as the help popup (black panel, yellow border).
fn draw_picker(f: &mut Frame, app: &mut App) {
    let Some(p) = &mut app.picker else { return };
    let screen = f.area();
    let longest = p
        .items
        .iter()
        .map(|i| i.label.chars().count() + i.detail.as_ref().map_or(0, |d| d.chars().count() + 3))
        .max()
        .unwrap_or(0)
        .max(p.title.chars().count() + 4);
    let w =
        ((longest + 8) as u16).clamp(24.min(screen.width), screen.width.saturating_sub(4).max(1));
    let h = (p.items.len() as u16 + 2).clamp(3, screen.height.saturating_sub(4).max(3));
    let area = Rect::new(
        screen.x + screen.width.saturating_sub(w) / 2,
        screen.y + screen.height.saturating_sub(h) / 2,
        w,
        h,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            format!(" {} ", p.title),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(" ↑↓ select · Enter/click pick · Esc cancel ");
    let inner = block.inner(area);
    p.popup_area = area;
    p.list_area = inner;
    p.scroll_into_view(inner.height as usize);
    let base = Style::default().bg(Color::Black).fg(Color::White);
    let lines: Vec<Line> = p
        .items
        .iter()
        .enumerate()
        .skip(p.offset)
        .take(inner.height as usize)
        .map(|(i, it)| {
            let sel = i == p.cursor;
            let st = if sel {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                base
            };
            let mut spans = vec![Span::styled(
                format!("{} {}", if sel { "›" } else { " " }, it.label),
                st,
            )];
            if let Some(d) = &it.detail {
                spans.push(Span::styled(
                    format!("  ({d})"),
                    if sel {
                        st
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ));
            }
            Line::from(spans).style(st)
        })
        .collect();
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).style(base).block(block), area);
}

/// The help modal: bordered popup, distinct colours, related links at the bottom.
fn draw_shell(f: &mut Frame, app: &mut App) {
    let Some(m) = &mut app.shell else { return };
    let area = centered(f.area(), 85, 85);
    let body = m.body();
    let inner_w = area.width.saturating_sub(2).max(1);
    let total = Paragraph::new(body.as_str())
        .wrap(Wrap { trim: false })
        .line_count(inner_w);
    let max = (total as u16).saturating_sub(area.height.saturating_sub(2));
    m.scroll = m.scroll.min(max);
    let border = if m.running() {
        Color::Yellow
    } else if m.failed() {
        Color::Red
    } else {
        Color::Green
    };
    let state = if m.running() { " · running…" } else { "" };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(body.as_str())
            .wrap(Wrap { trim: false })
            .scroll((m.scroll, 0))
            .style(Style::default().bg(Color::Black).fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(Span::styled(
                        format!(" Shell (local){state} "),
                        Style::default().fg(border).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(" ↑↓ PgUp/PgDn wheel scroll · Esc/q close "),
            ),
        area,
    );
}

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

    fn screen_rows(term: &Terminal<TestBackend>) -> Vec<String> {
        let b = term.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn input_box_grows_wraps_caps_and_scrolls() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        let mut term = Terminal::new(TestBackend::new(40, 30)).unwrap();
        // One line: 3 rows tall (borders + 1).
        app.input_insert("hi");
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        assert!(r[28].contains("> hi"), "{r:#?}");
        // Shift-Enter newline: box grows by one row, chat shrinks.
        app.input_newline();
        app.input_insert("there");
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        assert!(
            r[27].contains("> hi") && r[28].contains("  there"),
            "{r:#?}"
        );
        assert_eq!(
            term.get_cursor_position().unwrap(),
            Position::new(3 + 5, 28)
        );
        // A long line wraps at 36 columns (40 - borders - "> ") instead of running off.
        app.input_insert(&"y".repeat(50));
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        assert!(
            r[27].contains("  there") && r[27].contains(&"y".repeat(31)),
            "{r:#?}"
        );
        assert!(
            r[28].contains(&format!("│  {} ", "y".repeat(19))) && !r[28].contains(&"y".repeat(20)),
            "{r:#?}"
        );
        // cursor after 55 chars on that line: row 2 of it, col 55-36 = 19
        assert_eq!(
            term.get_cursor_position().unwrap(),
            Position::new(3 + 19, 28)
        );
        // 20 lines: capped at 10 visible rows (box height 12), scrolled to the cursor.
        for i in 0..20 {
            app.input_newline();
            app.input_insert(&format!("line{i}"));
        }
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        assert!(r[18].starts_with('┌'), "box is 12 rows: {r:#?}");
        assert!(r[28].contains("line19"));
        assert!(!r.iter().any(|l| l.contains("line9 ")));
        assert_eq!(term.get_cursor_position().unwrap().y, 28);
        // Move up past the top of the view: it scrolls to keep the cursor visible.
        for _ in 0..15 {
            app.input_up();
        }
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        let y = term.get_cursor_position().unwrap().y as usize;
        assert!((19..=28).contains(&y));
        assert!(r[y].contains("line4"), "{r:#?}");
    }

    #[test]
    fn shell_modal_renders() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.shell = Some(crate::app::ShellModal {
            id: 1,
            cmd: "echo hi".into(),
            result: None,
            scroll: 0,
        });
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let screen: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(screen.contains("Shell (local)"));
        assert!(screen.contains("$ echo hi"));
        assert!(screen.contains("running…"));
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

    #[test]
    fn picker_renders_and_mouse_click_picks() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let before = app.active_bot().unwrap().id.clone();
        app.open_bot_picker();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("Switch bot"));
        assert!(screen.contains("(current)"));
        // Click the row of a different bot.
        let p = app.picker.as_ref().unwrap();
        let target = p.items.iter().position(|i| i.value != before).unwrap();
        let (x, y) = (
            p.list_area.x + 2,
            p.list_area.y + (target - p.offset) as u16,
        );
        let want = p.items[target].value.clone();
        let p = app.picker.as_mut().unwrap();
        p.hover(x, y);
        assert_eq!(p.cursor, target, "hover highlights");
        assert_eq!(p.click(x, y), crate::picker::MouseOutcome::Confirm);
        app.picker_confirm();
        assert!(app.picker.is_none());
        assert_eq!(app.active_bot().unwrap().id, want);
    }

    #[test]
    fn chat_has_blank_line_between_messages_and_inner_padding() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let bot = app.active_bot().unwrap().id.clone();
        for (i, t) in ["FIRST-MSG", "SECOND-MSG"].iter().enumerate() {
            app.store
                .append_message(
                    &bot,
                    &crate::api::Message {
                        role: crate::api::Role::Bot,
                        text: (*t).into(),
                        remote_id: Some(format!("p{i}")),
                    },
                )
                .unwrap();
        }
        app.refresh_feed_from_db();
        app.notices.clear();
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let row_of = |needle: &str| {
            (0..30u16)
                .find(|&y| {
                    (0..100u16)
                        .map(|x| buf[(x, y)].symbol())
                        .collect::<String>()
                        .contains(needle)
                })
                .unwrap()
        };
        let (a, b) = (row_of("FIRST-MSG"), row_of("SECOND-MSG"));
        assert_eq!(b, a + 2, "one blank line between messages");
        let blank: String = (0..100u16).map(|x| buf[(x, a + 1)].symbol()).collect();
        assert!(!blank.contains("MSG"));
        // Left padding: two spaces between the chat border and the text.
        let line: String = (0..100u16).map(|x| buf[(x, a)].symbol()).collect();
        let border = line[..line.find("FIRST-MSG").unwrap()].rfind('│').unwrap();
        let after_border = &line[border + '│'.len_utf8()..];
        assert!(after_border.starts_with("  "), "{line:?}");
    }
}
