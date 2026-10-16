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
    spans.push(if app.mouse_capture {
        Span::styled("   mouse: on (F6)", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(
            "   mouse: OFF — select text freely (F6)",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        )
    });
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
    // Inner padding between the border and the content (left/right 2, top 1).
    let chat_block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::new(2, 2, 1, 0));
    let inner = chat_block.inner(body);
    let inner_w = inner.width.max(1) as usize;
    let inner_h = inner.height;
    // Message bubbles wrap at most 2/3 of the body width.
    let bubble = bubble_width(inner_w);
    // Chat feed (from SQLite), then transient notices in their own section.
    // A blank line separates consecutive messages.
    let mut lines: Vec<Line> = vec![];
    for (i, e) in app.feed().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(entry_lines(
            e.kind,
            &e.text,
            bot_name.as_deref(),
            Some(bubble),
        ));
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
                .flat_map(|e| entry_lines(e.kind, &e.text, bot_name.as_deref(), None)),
        );
    }
    if lines.is_empty() {
        lines = empty_state(app)
            .into_iter()
            .map(|t| Line::styled(t, Style::default().fg(Color::DarkGray)))
            .collect();
    }
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

/// Your message as a clean block: wrapped to `max`, every line padded to the longest
/// line's width so the block's left and right edges are straight when right-aligned.
pub fn bubble_block(text: &str, max: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    let lines = wrap_text(text, max);
    let bw = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    lines
        .into_iter()
        .map(|l| {
            let pad = bw - l.width();
            l + &" ".repeat(pad)
        })
        .collect()
}

/// Lines for one entry. In the chat feed (`feed`), the user's own messages are
/// right-aligned like a typical chat UI; everything else stays left-aligned.
/// Widest a chat message's text may be: 2/3 of the chat body's inner width.
pub fn bubble_width(inner_w: usize) -> usize {
    (inner_w * 2 / 3).max(1)
}

/// Word-wrap `text` to `width` display columns (long words are split). Keeps blank lines.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let w = width.max(1);
    let cw = |c: char| c.width().unwrap_or(0);
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        let mut lw = 0;
        for word in para.split(' ') {
            let ww: usize = word.chars().map(cw).sum();
            let sep = usize::from(!line.is_empty() || lw > 0);
            if lw + sep + ww <= w {
                if sep == 1 {
                    line.push(' ');
                    lw += 1;
                }
                line.push_str(word);
                lw += ww;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
                lw = 0;
            }
            for c in word.chars() {
                if lw + cw(c) > w && !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                    lw = 0;
                }
                line.push(c);
                lw += cw(c);
            }
        }
        out.push(line);
    }
    out
}

/// Lines for one entry. Chat messages (`bubble = Some(max)`) are pre-wrapped to `max`
/// columns: yours right-aligned, the bot's left-aligned. Notices (`None`) wrap normally.
fn entry_lines<'a>(
    kind: EntryKind,
    text: &'a str,
    bot: Option<&str>,
    bubble: Option<usize>,
) -> Vec<Line<'a>> {
    if let Some(max) = bubble
        && kind == EntryKind::User
    {
        let style = Style::default().fg(Color::Green);
        let mut out = vec![
            Line::from(Span::styled("you", style.add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Right),
        ];
        out.extend(
            bubble_block(text, max)
                .into_iter()
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
    let pw = unicode_width::UnicodeWidthStr::width(prefix.as_str());
    let body: Vec<String> = match bubble {
        // Prefix + text stay within the bubble; keep at least a third for text.
        Some(max) => wrap_text(text, max.saturating_sub(pw).max(max / 3).max(1)),
        None => text.lines().map(str::to_string).collect(),
    };
    body.into_iter()
        .enumerate()
        .map(|(i, l)| {
            let p = if i == 0 {
                prefix.clone()
            } else {
                " ".repeat(pw)
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
    fn wrap_text_respects_width() {
        assert_eq!(wrap_text("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap_text("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap_text("a\n\nb", 10), vec!["a", "", "b"]);
        for l in wrap_text("日本語のテキスト です", 5) {
            assert!(unicode_width::UnicodeWidthStr::width(l.as_str()) <= 5);
        }
    }

    #[test]
    fn user_bubble_is_a_clean_block() {
        let b = bubble_block("the quick brown fox jumps over the lazy dog again", 16);
        assert!(b.len() > 2);
        let w = b[0].len();
        assert!(w <= 16);
        assert!(
            b.iter().all(|l| l.len() == w),
            "all lines same width: {b:?}"
        );
        assert_eq!(
            b.iter().map(|l| l.trim_end()).collect::<Vec<_>>().join(" "),
            "the quick brown fox jumps over the lazy dog again"
        );
        assert_eq!(bubble_block("hi", 16), vec!["hi"]);
    }

    #[test]
    fn chat_bubbles_wrap_within_two_thirds() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let long = "lorem ipsum dolor sit amet consectetur ".repeat(8);
        let bot = app.active_bot().unwrap().id.clone();
        app.store
            .merge_remote(
                &bot,
                &[
                    crate::api::Message {
                        role: crate::api::Role::User,
                        text: long.clone(),
                        remote_id: Some("u1".into()),
                    },
                    crate::api::Message {
                        role: crate::api::Role::Bot,
                        text: long.clone(),
                        remote_id: Some("b1".into()),
                    },
                ],
                true,
            )
            .unwrap();
        app.refresh_feed_from_db();
        let width = 120u16;
        let mut term = Terminal::new(TestBackend::new(width, 60)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let b = term.backend().buffer().clone();
        // Chat body: right of the sidebar; inner = body minus borders and 2+2 padding.
        let body_x = app.config.sidebar_width;
        let inner_x = body_x + 3;
        let inner_w = (width - body_x - 6) as usize;
        let max = inner_w * 2 / 3;
        assert_eq!(bubble_width(inner_w), max);
        let mut msg_rows = 0;
        for y in 0..b.area.height {
            let row: String = (inner_x..inner_x + inner_w as u16)
                .map(|x| b[(x, y)].symbol())
                .collect();
            if !row.contains("lorem") && !row.contains("amet") && !row.contains("dolor") {
                continue;
            }
            msg_rows += 1;
            let first = row.find(|c: char| c != ' ').unwrap();
            let last = row.trim_end().len();
            assert!(last - first <= max, "row wider than 2/3 ({max}): {row:?}");
            if row.contains(": ") {
                assert!(first < 3, "bot message left-aligned: {row:?}");
            }
        }
        assert!(msg_rows >= 8, "long messages wrapped onto several rows");
        // Your message rows (right-aligned) all start at the same column: a clean block.
        let mut user_starts = vec![];
        for y in 0..b.area.height {
            let row: String = (inner_x..inner_x + inner_w as u16)
                .map(|x| b[(x, y)].symbol())
                .collect();
            if let Some(f) = row.find(|c: char| c != ' ')
                && f > inner_w / 4
                && !row.contains("you")
            {
                user_starts.push(f);
            }
        }
        assert!(user_starts.len() >= 3, "{user_starts:?}");
        assert!(
            user_starts.windows(2).all(|w| w[0] == w[1]),
            "jagged: {user_starts:?}"
        );
        // Some user rows end flush right.
        let right_flush = (0..b.area.height).any(|y| {
            let c = b[(inner_x + inner_w as u16 - 1, y)].symbol();
            c != " " && c != "│"
        });
        assert!(right_flush, "user message right-aligned");
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
