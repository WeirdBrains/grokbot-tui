//! Rendering: top bar, left sidebar, body, bottom input bar.
use crate::app::{App, EntryKind, Focus, InputMode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

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
    let user = app
        .user
        .as_ref()
        .map(|u| u.name.as_str())
        .unwrap_or("not logged in");
    let bot = app.active_bot().map(|b| b.name.as_str()).unwrap_or("none");
    let mut spans = vec![
        Span::styled(
            " gb ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  user: {user}   bot: {bot}")),
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

    // Sidebar
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
    let lines: Vec<Line> = app
        .current_log()
        .iter()
        .flat_map(|e| entry_lines(e.kind, &e.text, bot_name.as_deref()))
        .collect();
    let inner_w = body.width.saturating_sub(2).max(1) as usize;
    let inner_h = body.height.saturating_sub(2);
    let total: usize = lines
        .iter()
        .map(|l| l.width().max(1).div_ceil(inner_w))
        .sum();
    let max_off = (total as u16).saturating_sub(inner_h);
    let scroll = app.scroll.min(max_off);
    let off = max_off - scroll;
    let title = match &bot_name {
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
        InputMode::Token => ("*".repeat(app.input.chars().count()), " Token (masked) "),
        InputMode::Normal => (app.input.clone(), " /command or message "),
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
    if app.focus == Focus::Input {
        let x = bottom.x + 3 + shown.chars().count() as u16;
        f.set_cursor_position(Position::new(
            x.min(bottom.right().saturating_sub(2)),
            bottom.y + 1,
        ));
    }
}

fn entry_lines<'a>(kind: EntryKind, text: &'a str, bot: Option<&str>) -> Vec<Line<'a>> {
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
        app.input = "/login t".into();
        app.submit();
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let text: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains("user: mock-user"));
        assert!(text.contains("bot: Helper"));
        assert!(text.contains("Coder"));
        assert_eq!(app.sidebar_area.width, 24);
    }
}
