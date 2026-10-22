//! Rendering: top bar, left sidebar, body, bottom input bar.
//!
//! All colors, borders and padding come from [`crate::theme`] (ratatui-lipgloss styles);
//! this module only lays things out.
use crate::app::{App, EntryKind, Focus, InputMode};
use crate::config::TitleGlyphs;
use crate::theme::{self, Mode};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph, Wrap};
use tui_big_text::{BigText, PixelSize};
use unicode_width::UnicodeWidthStr;

/// Rows of the tall header (logo, big title, four info rows); the compact header is 1 row.
pub const TALL_HEADER_ROWS: u16 = 4;
/// The tall header needs at least this many columns (logo + title + info) …
pub const TALL_MIN_WIDTH: u16 = 80;
/// … and this many rows, so the chat keeps enough room; otherwise the header is compact.
pub const TALL_MIN_HEIGHT: u16 = 24;
/// With half-height glyphs (8 columns per letter) the big title is drawn only when the header
/// is at least this wide (logo, title, gaps and room for the info rows); narrower tall
/// headers show the logo and info rows only. Quadrant titles (4 columns) always fit.
pub const HALF_TITLE_MIN_WIDTH: u16 = 128;

/// The header's size for a terminal of `area`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderKind {
    /// Logo, big "Grok Bot" title and four info rows.
    Tall,
    /// One row: name tag, account and theme switchers, login, bot, mouse.
    Compact,
}

impl HeaderKind {
    pub fn rows(self) -> u16 {
        match self {
            HeaderKind::Tall => TALL_HEADER_ROWS,
            HeaderKind::Compact => 1,
        }
    }
}

/// Tall when the terminal is at least [`TALL_MIN_WIDTH`] × [`TALL_MIN_HEIGHT`], else compact.
pub fn header_kind(area: Rect) -> HeaderKind {
    if area.width >= TALL_MIN_WIDTH && area.height >= TALL_MIN_HEIGHT {
        HeaderKind::Tall
    } else {
        HeaderKind::Compact
    }
}

/// The header title, drawn large with tui-big-text: quadrant pixels (4 columns × 4 rows per
/// letter) by default, or half-height pixels (8 × 4, only `▀ ▄ █`) where quadrant glyphs come
/// out gappy and misshapen, such as macOS Terminal. See [`crate::config::TitleGlyphs`].
pub const TITLE: &str = "Grok Bot";

/// The Grok Bot logo (the desktop app's icon: a white ball with a slanted stroke on a dark
/// squircle) as 8 × 8 square pixels, drawn with half blocks as 8 columns × 4 rows.
/// `d` = dark, `W` = white, space = transparent. Traced from the app's `resources/icon.png`
/// (Lanczos-resampled to 8 × 8, luminance threshold 150), then cleaned up by hand. Square
/// pixels keep the icon round; the old 12 × 8 version stretched it into a blob.
const LOGO: [&str; 8] = [
    " dddddd ", "dddddddd", "dWWWWWdd", "WWWdWWdd", "WWWWdWWW", "WWWWWdWW", "WWWWWWWW", " WWWWWW ",
];
/// Columns the logo takes.
pub const LOGO_WIDTH: u16 = 8;

/// Draw [`LOGO`] at the top-left of `area` (clipped), leaving transparent pixels alone.
fn draw_logo(buf: &mut Buffer, area: Rect) {
    let (dark, white) = theme::logo_colors();
    let px = |row: usize, col: usize| match LOGO[row].as_bytes().get(col) {
        Some(b'd') => Some(dark),
        Some(b'W') => Some(white),
        _ => None,
    };
    for r in 0..(LOGO.len() / 2).min(area.height as usize) {
        for c in 0..(LOGO_WIDTH as usize).min(area.width as usize) {
            let pos = Position::new(area.x + c as u16, area.y + r as u16);
            let Some(cell) = buf.cell_mut(pos) else {
                continue;
            };
            match (px(2 * r, c), px(2 * r + 1, c)) {
                (Some(top), Some(bottom)) => {
                    cell.set_symbol("▀").set_fg(top).set_bg(bottom);
                }
                (Some(top), None) => {
                    cell.set_symbol("▀").set_fg(top);
                }
                (None, Some(bottom)) => {
                    cell.set_symbol("▄").set_fg(bottom);
                }
                (None, None) => {}
            }
        }
    }
}

/// What a click on a header segment does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Account,
    ThemeSwitch,
    Mode(Mode),
}

/// A piece of header text, optionally clickable.
struct Seg {
    text: String,
    style: ratatui_lipgloss::Style,
    target: Option<Target>,
}

fn seg(text: impl Into<String>, style: ratatui_lipgloss::Style) -> Seg {
    Seg {
        text: text.into(),
        style,
        target: None,
    }
}

fn hit(text: impl Into<String>, style: ratatui_lipgloss::Style, target: Target) -> Seg {
    Seg {
        text: text.into(),
        style,
        target: Some(target),
    }
}

/// Render `segs` on the single row `area`, recording where the clickable ones landed.
fn render_segs(f: &mut Frame, app: &mut App, area: Rect, segs: Vec<Seg>) {
    let mut x = area.x;
    let mut spans = Vec::with_capacity(segs.len());
    for s in segs {
        let w = s.text.width() as u16;
        let visible = area.right().saturating_sub(x).min(w);
        if let Some(t) = s.target
            && visible > 0
        {
            let r = Rect::new(x, area.y, visible, 1);
            match t {
                Target::Account => app.account_area = r,
                Target::ThemeSwitch => app.theme_switch_area = r,
                Target::Mode(m) => app.theme_areas.push((r, m)),
            }
        }
        x = x.saturating_add(w);
        spans.push(s.style.span(s.text));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// "account  @alias ▾  F3" (no label when compact)
fn account_segs(app: &App, compact: bool) -> Vec<Seg> {
    let label = if compact { "" } else { "account " };
    vec![
        seg(label, theme::header_label()),
        hit(
            format!(" @{} ▾ ", app.account().alias),
            theme::account_switch(),
            Target::Account,
        ),
        seg(" F3", theme::key_hint()),
    ]
}

/// "theme  ‹default› ▾  dark  light  system  F4 (→ dark)" (no label or "→" note when compact)
fn theme_segs(app: &App, compact: bool) -> Vec<Seg> {
    let current = app.config.theme_mode;
    let label = if compact { "" } else { "theme " };
    let name = match app.theme_problem {
        Some(p) => format!("{}: {p}", app.config.theme),
        None => app.config.theme.clone(),
    };
    let mut v = vec![
        seg(label, theme::header_label()),
        hit(
            format!(" ‹{name}› ▾ "),
            theme::theme_switch(),
            Target::ThemeSwitch,
        ),
    ];
    for m in Mode::ALL {
        v.push(hit(
            format!(" {} ", m.as_str()),
            theme::mode_choice(m == current),
            Target::Mode(m),
        ));
    }
    v.push(seg(" F4", theme::key_hint()));
    if current == Mode::System && !compact {
        let v_name = match app.theme_variant() {
            theme::Variant::Dark => "dark",
            theme::Variant::Light => "light",
        };
        v.push(seg(format!(" (→ {v_name})"), theme::key_hint()));
    }
    v
}

fn login_and_bot(app: &App) -> (String, String) {
    let user = match (&app.user_label, app.connecting) {
        (Some(l), _) => l.clone(),
        (None, true) => "connecting…".into(),
        (None, false) => "not logged in".into(),
    };
    let bot = app
        .active_bot()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "none".into());
    (user, bot)
}

fn mouse_seg(app: &App) -> Seg {
    if app.mouse_capture {
        seg("mouse: on (F6)", theme::mouse_on())
    } else {
        seg("mouse: OFF — select text freely (F6)", theme::mouse_off())
    }
}

/// The header: tall (logo, big title, four info rows) or one compact row.
fn draw_header(f: &mut Frame, app: &mut App, area: Rect, kind: HeaderKind) {
    app.account_area = Rect::default();
    app.theme_switch_area = Rect::default();
    app.theme_areas.clear();
    f.render_widget(Block::new().style(theme::top_bar()), area);
    let (user, bot) = login_and_bot(app);
    match kind {
        HeaderKind::Compact => {
            // Most useful first, so a narrow terminal clips the least important parts.
            let mut segs = vec![
                seg(" grokbot-tui ", theme::brand()),
                seg(" ", theme::top_bar()),
            ];
            segs.extend(account_segs(app, true));
            segs.push(seg("  ", theme::top_bar()));
            segs.extend(theme_segs(app, true));
            segs.push(seg(
                format!("  login: {user}  bot: {bot}  "),
                theme::top_bar(),
            ));
            segs.push(mouse_seg(app));
            if app.mock {
                segs.push(seg("   [MOCK]", theme::mock_tag()));
            }
            render_segs(f, app, area, segs);
        }
        HeaderKind::Tall => {
            let half = app.config.title_glyphs.resolve() == TitleGlyphs::Half;
            let (glyph_w, pixel_size) = if half {
                (8, PixelSize::HalfHeight)
            } else {
                (4, PixelSize::Quadrant)
            };
            let title_w = if !half || area.width >= HALF_TITLE_MIN_WIDTH {
                TITLE.chars().count() as u16 * glyph_w
            } else {
                0
            };
            let [_, logo, _, title, _, info] = Layout::horizontal([
                Constraint::Length(1),
                Constraint::Length(LOGO_WIDTH),
                Constraint::Length(2),
                Constraint::Length(title_w),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .areas(area);
            draw_logo(f.buffer_mut(), logo);
            f.render_widget(
                BigText::builder()
                    .pixel_size(pixel_size)
                    .style(theme::title())
                    .lines(vec![Line::from(TITLE)])
                    .build(),
                title,
            );
            let rows: [Rect; 4] = Layout::vertical([Constraint::Length(1); 4]).areas(info);
            let segs = account_segs(app, false);
            render_segs(f, app, rows[0], segs);
            let segs = theme_segs(app, false);
            render_segs(f, app, rows[1], segs);
            let segs = vec![
                seg("login ", theme::header_label()),
                seg(user, theme::top_bar()),
                seg("   bot ", theme::header_label()),
                seg(bot, theme::top_bar()),
            ];
            render_segs(f, app, rows[2], segs);
            let mut segs = vec![mouse_seg(app)];
            if app.mock {
                segs.push(seg("   [MOCK]", theme::mock_tag()));
            }
            render_segs(f, app, rows[3], segs);
        }
    }
}

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
    theme::set_theme(app.theme.clone());
    theme::set_variant(app.theme_variant());
    if app.editor.is_some() {
        crate::theme_editor::draw(f, app);
        if app.picker.is_some() {
            draw_picker(f, app);
        }
        return;
    }
    let cursor = draw_main(f, app);
    if app.picker.is_some() {
        draw_picker(f, app);
    } else if let Some(s) = app.history_search.as_mut() {
        crate::history::draw(f, s, &app.prompt_history, f.area());
    } else if app.completion.is_some() {
        crate::complete::draw(f, app);
    } else if app.shell.is_some() {
        draw_shell(f, app);
    } else if app.help.is_some() {
        draw_help(f, app);
    } else if let Some(c) = cursor {
        f.set_cursor_position(c);
    }
}

/// The main screen (header, sidebar, chat, input) with the active theme, without modals.
/// Returns where the input cursor goes. Also drawn off-screen as the theme editor preview.
pub fn draw_main(f: &mut Frame, app: &mut App) -> Option<Position> {
    f.render_widget(Block::new().style(theme::base()), f.area());
    // Input box: char-wrapped to its width, grows to 10 rows, then scrolls.
    let (shown, cur) = input_shown(app);
    let text_w = f.area().width.saturating_sub(4).max(1) as usize; // borders + "> "
    let il = crate::input::layout(&shown, cur, text_w);
    let vis = crate::input::visible_rows(il.rows.len());
    app.input_scroll = crate::input::scroll_for(app.input_scroll, il.cursor.0, il.rows.len(), vis);
    let header = header_kind(f.area());
    let [top, mid, bottom] = Layout::vertical([
        Constraint::Length(header.rows()),
        Constraint::Min(3),
        Constraint::Length(vis as u16 + 2),
    ])
    .areas(f.area());
    let [side, body] = Layout::horizontal([
        Constraint::Length(app.config.sidebar_width),
        Constraint::Min(10),
    ])
    .areas(mid);

    draw_header(f, app, top, header);

    // Sidebar: the bot list (the account switcher is in the header).
    app.sidebar_area = side;
    app.body_area = body;
    app.input_area = bottom;
    let items: Vec<ListItem> = app
        .bots
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let tag = if b.team { " ·team" } else { "" };
            let item = ListItem::new(format!(" {}{tag}", b.name));
            if Some(i) == app.active {
                item.style(theme::selected())
            } else if app.focus == Focus::Sidebar && i == app.sidebar_cursor {
                item.style(theme::cursor_row())
            } else {
                item
            }
        })
        .collect();
    f.render_widget(
        List::new(items)
            .block(theme::block(&theme::panel(app.focus == Focus::Sidebar)).title(" Bots ")),
        side,
    );

    // Body
    let bot_name = app.active_bot().map(|b| b.name.clone());
    // Inner padding between the border and the content (left/right 2, top 1): see the theme.
    let chat_block = theme::block(&theme::chat_panel());
    let inner = chat_block.inner(body);
    let inner_w = inner.width.max(1) as usize;
    let inner_h = inner.height;
    // Message bubbles wrap at most 2/3 of the body width.
    let bubble = bubble_width(inner_w);
    // Chat feed (from SQLite), then transient notices in their own section.
    // A blank line separates consecutive messages. The feed's lines and their wrapped
    // heights are cached and only rebuilt when the feed, width, bot name or entry styles
    // change, so a frame doesn't re-wrap the whole transcript.
    let key = feed_key(app.feed(), inner_w, bot_name.as_deref());
    if app.feed_cache.as_ref().is_none_or(|c| c.key != key) {
        let mut lines: Vec<Line<'static>> = vec![];
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
        let rows = row_counts(&lines, inner_w);
        app.feed_cache = Some(FeedCache { key, lines, rows });
    }
    let cache = app.feed_cache.as_ref().expect("built above");
    let mut extra: Vec<Line<'static>> = vec![];
    if !app.notices().is_empty() {
        if !cache.lines.is_empty() {
            extra.push(Line::default());
            extra.push(theme::muted().line("── notices (not saved) ──"));
        }
        extra.extend(
            app.notices()
                .iter()
                .flat_map(|e| entry_lines(e.kind, &e.text, bot_name.as_deref(), None)),
        );
    }
    if cache.lines.is_empty() && extra.is_empty() {
        extra = empty_state(app)
            .into_iter()
            .map(|t| theme::muted().line(t))
            .collect();
    }
    let extra_rows = row_counts(&extra, inner_w);
    // Exact wrapped height (word wrap can need more rows than width/cols), so
    // "scrolled to bottom" really shows the newest line.
    let total: usize = cache.rows.iter().chain(&extra_rows).sum();
    let max_off = (total.min(u16::MAX as usize) as u16).saturating_sub(inner_h);
    let scroll = app.scroll.min(max_off);
    let off = max_off - scroll;
    // Only the lines that reach into the viewport are rendered.
    let (mut lines, skip) = visible_lines(
        cache.lines.iter().chain(&extra),
        cache.rows.iter().chain(&extra_rows).copied(),
        off as usize,
        inner_h as usize,
    );
    app.body_plain = crate::select::plain_text(app.feed());
    app.body_wrap = bubble;
    app.body_row_off = off as usize;
    if let Some(sel) = &app.body_sel
        && !sel.is_empty()
    {
        // Restyle visible lines while a selection is active (theme `selection` / selected).
        lines = lines
            .into_iter()
            .map(|l| {
                Line::from(
                    l.spans
                        .into_iter()
                        .map(|sp| theme::selected().span(sp.content.to_string()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
    }
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
            .scroll((skip as u16, 0))
            .block(chat_block.title(title)),
        body,
    );
    app.scroll = scroll;

    // Bottom input bar
    let title = match app.mode {
        InputMode::GatewayToken => " Gateway token (masked, Esc cancels) ".to_string(),
        InputMode::CursorToken => " Cursor access token (masked, Esc cancels) ".to_string(),
        InputMode::GatewayUrl => " Gateway URL (Esc cancels) ".to_string(),
        InputMode::Normal if app.keys_are_vim() && app.vim_mode == crate::keymap::Mode::Command => {
            format!(
                " :{}▏ · Enter runs · Esc cancels ",
                app.vim_command.as_deref().unwrap_or("")
            )
        }
        InputMode::Normal if app.keys_are_vim() => {
            format!(
                " {} · Esc NORMAL · i INSERT · : command · /keymap ",
                app.vim_mode.label()
            )
        }
        InputMode::Normal => " message or /command · Shift/Alt-Enter newline · F1 help ".to_string(),
    };
    let scroll = app.input_scroll;
    // The selected range in char indices (masked prompts never select).
    let sel = match app.mode {
        InputMode::Normal => app.input_selection().map(|(a, b)| {
            (
                app.input[..a].chars().count(),
                app.input[..b].chars().count(),
            )
        }),
        _ => None,
    };
    let starts = crate::input::row_starts(&shown, text_w);
    let input_lines: Vec<Line> = il
        .rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(vis)
        .map(|(i, r)| {
            let lead = Span::raw(if i == 0 { "> " } else { "  " });
            match (sel, starts.get(i)) {
                (Some((a, b)), Some(&s0)) => {
                    let chars: Vec<char> = r.chars().collect();
                    let n = chars.len();
                    let (x, y) = (a.saturating_sub(s0).min(n), b.saturating_sub(s0).min(n));
                    let part = |p: &[char]| p.iter().collect::<String>();
                    Line::from(vec![
                        lead,
                        Span::raw(part(&chars[..x])),
                        theme::selected().span(part(&chars[x..y])),
                        Span::raw(part(&chars[y..])),
                    ])
                }
                _ => Line::from(vec![lead, Span::raw(r.clone())]),
            }
        })
        .collect();
    let more = if il.rows.len() > vis {
        format!(" {}/{} ", il.cursor.0 + 1, il.rows.len())
    } else {
        String::new()
    };
    let title_line = if app.keys_are_vim() && matches!(app.mode, InputMode::Normal) {
        Line::from(vec![
            theme::vim_mode_style(app.vim_mode).span(format!(" {} ", app.vim_mode.label())),
            theme::key_hint().span(title.trim_start_matches(&format!(" {} ", app.vim_mode.label())).to_string()),
        ])
    } else {
        Line::from(title)
    };
    f.render_widget(
        Paragraph::new(input_lines).block(
            theme::block(&theme::panel(app.focus == Focus::Input))
                .title(title_line)
                .title_bottom(Line::from(more).right_aligned()),
        ),
        bottom,
    );
    (app.focus == Focus::Input).then(|| {
        let (row, col) = il.cursor;
        let x = bottom.x + 3 + col as u16;
        let y = bottom.y + 1 + (row - scroll) as u16;
        Position::new(x.min(bottom.right().saturating_sub(2)), y)
    })
}

/// The chat feed's laid-out lines and the wrapped height of each, for one key.
pub struct FeedCache {
    key: u64,
    lines: Vec<Line<'static>>,
    rows: Vec<usize>,
}

/// What the feed's layout depends on: the entries, the width, the bot's name and the
/// entry styles of the theme being drawn with.
fn feed_key(feed: &[crate::app::Entry], inner_w: usize, bot: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (inner_w, bot, feed.len()).hash(&mut h);
    for e in feed {
        (e.kind as u8, &e.text).hash(&mut h);
    }
    for k in [
        EntryKind::System,
        EntryKind::Error,
        EntryKind::User,
        EntryKind::Bot,
    ] {
        format!("{:?}{:?}", theme::entry(k), theme::entry_label(k)).hash(&mut h);
    }
    h.finish()
}

/// Wrapped height of each line at `width` (what `Paragraph` with `Wrap` would use).
fn row_counts(lines: &[Line<'static>], width: usize) -> Vec<usize> {
    lines
        .iter()
        .map(|l| {
            Paragraph::new(l.clone())
                .wrap(Wrap { trim: false })
                .line_count(width.max(1) as u16)
        })
        .collect()
}

/// The lines that cover rows `off..off + height`, and how many rows of the first one
/// lie above the viewport.
fn visible_lines<'l>(
    lines: impl Iterator<Item = &'l Line<'static>>,
    rows: impl Iterator<Item = usize>,
    off: usize,
    height: usize,
) -> (Vec<Line<'static>>, usize) {
    let (mut out, mut skip, mut at) = (vec![], 0, 0);
    for (l, r) in lines.zip(rows) {
        if at + r > off && at < off + height {
            if out.is_empty() {
                skip = off.saturating_sub(at);
            }
            out.push(l.clone());
        }
        at += r;
        if at >= off + height {
            break;
        }
    }
    (out, skip)
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

/// The selection modal: same look as the help popup (see `theme::help_frame`).
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
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(format!(" {} ", p.title)))
        .title_bottom(" ↑↓ select · Enter/click pick · Esc cancel ");
    let inner = block.inner(area);
    p.popup_area = area;
    p.list_area = inner;
    p.scroll_into_view(inner.height as usize);
    let base = theme::modal_text();
    let lines: Vec<Line> = p
        .items
        .iter()
        .enumerate()
        .skip(p.offset)
        .take(inner.height as usize)
        .map(|(i, it)| {
            let sel = i == p.cursor;
            let st = if sel { theme::selected() } else { base.clone() };
            let mut spans = vec![st.span(format!("{} {}", if sel { "›" } else { " " }, it.label))];
            if let Some(d) = &it.detail {
                let detail = if sel { st.clone() } else { theme::muted() };
                spans.push(detail.span(format!("  ({d})")));
            }
            Line::from(spans).style(&st)
        })
        .collect();
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).style(&base).block(block), area);
}

/// The `/shell` modal: frame colored by state (running / failed / ok).
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
    let (running, failed) = (m.running(), m.failed());
    let state = if m.running() { " · running…" } else { "" };
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(body.as_str())
            .wrap(Wrap { trim: false })
            .scroll((m.scroll, 0))
            .style(theme::modal_text())
            .block(
                theme::block(&theme::shell_frame(running, failed))
                    .title(
                        theme::shell_title(running, failed).span(format!(" Shell (local){state} ")),
                    )
                    .title_bottom(" ↑↓ PgUp/PgDn wheel scroll · Esc/q close "),
            ),
        area,
    );
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
    let style = theme::modal_text();
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
            .style(&style)
            .block(
                theme::block(&theme::help_frame())
                    .title(theme::help_title().span(format!(" Help: {} ", m.page.title)))
                    .title_bottom(format!(" ↑↓ scroll · Esc close{back} ")),
            ),
        text_area,
    );
    let mut spans = vec![Span::raw("Related: ")];
    for (i, id) in m.page.related.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.push(theme::link(i == m.link).span(crate::help::link_label(id)));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans))
            .wrap(Wrap { trim: true })
            .style(&style)
            .block(theme::block(&theme::help_frame()).title(match &m.query {
                Some(q) => format!(" Search: {q}▏ (Enter run · Esc cancel) "),
                None => " Tab/←→ select · Enter open · / search ".into(),
            })),
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
fn entry_lines(
    kind: EntryKind,
    text: &str,
    bot: Option<&str>,
    bubble: Option<usize>,
) -> Vec<Line<'static>> {
    if let Some(max) = bubble
        && kind == EntryKind::User
    {
        let (label, style) = (theme::entry_label(kind), theme::entry(kind));
        let mut out = vec![Line::from(label.span("you")).alignment(Alignment::Right)];
        out.extend(
            bubble_block(text, max)
                .into_iter()
                .map(|l| Line::from(style.span(l)).alignment(Alignment::Right)),
        );
        return out;
    }
    let prefix = match kind {
        EntryKind::System => String::new(),
        EntryKind::Error => "! ".to_string(),
        EntryKind::User => "you: ".to_string(),
        EntryKind::Bot => format!("{}: ", bot.unwrap_or("bot")),
    };
    let (label, style) = (theme::entry_label(kind), theme::entry(kind));
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
            Line::from(vec![label.span(p), style.span(l)])
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
        // 120x20 is too short for the tall header: one compact row.
        assert_eq!(header_kind(Rect::new(0, 0, 120, 20)), HeaderKind::Compact);
        assert!(text.contains("grokbot-tui"), "{text}");
        assert!(text.contains("login: gateway · box.cursorvm.com"), "{text}");
        assert!(text.contains("@default ▾"), "{text}");
        assert!(text.contains("bot: Helper"), "{text}");
        assert!(text.contains(" dark  light  system  F4"), "{text}");
        assert!(text.contains("Coder"));
        assert_eq!(app.sidebar_area.width, 24);
        // The sidebar no longer has an account header: it starts right under the bar.
        assert_eq!(app.sidebar_area.y, 1);
        assert_eq!(app.account_area.y, 0);
        assert_eq!(app.theme_areas.len(), 3);
    }

    #[test]
    fn visible_lines_cover_exactly_the_viewport() {
        let lines: Vec<Line<'static>> = (0..6).map(|i| Line::raw(format!("l{i}"))).collect();
        let rows = [1, 3, 1, 2, 1, 1];
        let (v, skip) = visible_lines(lines.iter(), rows.iter().copied(), 2, 3);
        // Rows: l0=0, l1=1..4, l2=4, l3=5..7 → rows 2..5 are l1 (from its 2nd row) and l2.
        assert_eq!(v, vec![Line::raw("l1"), Line::raw("l2")]);
        assert_eq!(skip, 1);
        let (v, skip) = visible_lines(lines.iter(), rows.iter().copied(), 0, 100);
        assert_eq!((v.len(), skip), (6, 0));
    }

    #[test]
    fn the_feed_cache_follows_new_entries_and_theme_changes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let bot = app.active_bot().unwrap().id.clone();
        let entry = |t: &str| crate::app::Entry {
            kind: EntryKind::Bot,
            text: t.into(),
        };
        app.logs.insert(
            bot.clone(),
            (0..200).map(|i| entry(&format!("old {i}"))).collect(),
        );
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let screen = |term: &mut Terminal<TestBackend>, app: &mut App| {
            term.draw(|f| draw(f, app)).unwrap();
            term.backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
        };
        let s = screen(&mut term, &mut app);
        assert!(
            s.contains("old 199") && !s.contains("old 100"),
            "bottom of the feed"
        );
        let key = app.feed_cache.as_ref().unwrap().key;
        screen(&mut term, &mut app);
        assert_eq!(
            app.feed_cache.as_ref().unwrap().key,
            key,
            "reused while unchanged"
        );
        app.logs.get_mut(&bot).unwrap().push(entry("brand new"));
        assert!(screen(&mut term, &mut app).contains("brand new"));
        // Scrolled up: older lines, same as an uncached layout would show.
        app.scroll = 10_000;
        let s = screen(&mut term, &mut app);
        assert!(s.contains("old 0") && !s.contains("brand new"), "{s}");
        // Another theme's entry styles rebuild the layout.
        app.scroll = 0;
        let key = app.feed_cache.as_ref().unwrap().key;
        let mut t = app.theme.as_ref().clone();
        t.dark.colors[crate::theme::color::ACCENT] = ratatui::style::Color::Indexed(1);
        t.light.colors[crate::theme::color::ACCENT] = ratatui::style::Color::Indexed(1);
        let s = crate::theme::slot::LABEL_BOT;
        t.dark.styles[s].fg = Some(crate::theme::ColorRef::Lit(ratatui::style::Color::Indexed(
            9,
        )));
        t.light.styles[s].fg = Some(crate::theme::ColorRef::Lit(ratatui::style::Color::Indexed(
            9,
        )));
        app.theme = std::sync::Arc::new(t);
        screen(&mut term, &mut app);
        assert_ne!(app.feed_cache.as_ref().unwrap().key, key);
    }

    #[test]
    fn header_kind_falls_back_to_compact_in_small_terminals() {
        let k = |w, h| header_kind(Rect::new(0, 0, w, h));
        assert_eq!(k(80, 24), HeaderKind::Tall);
        assert_eq!(k(200, 60), HeaderKind::Tall);
        assert_eq!(k(79, 40), HeaderKind::Compact);
        assert_eq!(k(120, 23), HeaderKind::Compact);
        assert_eq!(k(40, 30), HeaderKind::Compact);
        assert_eq!(HeaderKind::Tall.rows(), 4);
        assert_eq!(HeaderKind::Compact.rows(), 1);
    }

    #[test]
    fn quadrant_title_fits_an_80_column_header() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        app.config.title_glyphs = TitleGlyphs::Quadrant;
        let mut term = Terminal::new(TestBackend::new(80, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        let header = r[..4].join("\n");
        let title: String = r[..4]
            .iter()
            .flat_map(|row| row.chars().skip(11).take(32))
            .collect();
        assert!(title.chars().any(|c| ('▖'..='▟').contains(&c)), "{header}");
        assert!(r[0].contains("account  @default ▾  F3"), "{header}");
    }

    #[test]
    fn narrow_tall_header_drops_the_big_title_but_keeps_logo_and_info() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        app.config.title_glyphs = TitleGlyphs::Half;
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        let header = r[..4].join("\n");
        let between: String = r[..4]
            .iter()
            .flat_map(|row| row.chars().skip(9).take(4))
            .collect();
        assert!(between.trim().is_empty(), "{header}");
        assert!(r[0].contains("account  @default ▾  F3"), "{header}");
        assert!(r[3].contains("mouse: on (F6)"), "{header}");
    }

    #[test]
    fn tall_header_has_logo_title_and_switchers() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        app.config.title_glyphs = TitleGlyphs::Half;
        let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let r = screen_rows(&term);
        let header = r[..4].join("\n");
        // Logo: half blocks in columns 1..9 on all four rows.
        for row in &r[..4] {
            let logo: String = row.chars().skip(1).take(8).collect();
            assert!(logo.contains('▀') || logo.contains('▄'), "{header}");
        }
        // Big title: only half-height blocks (no quadrant glyphs) between logo and info.
        let title: String = r[..4]
            .iter()
            .flat_map(|row| row.chars().skip(11).take(64))
            .collect();
        assert!(
            title.chars().any(|c| matches!(c, '█' | '▀' | '▄')),
            "{header}"
        );
        assert!(!title.chars().any(|c| ('▖'..='▟').contains(&c)), "{header}");
        assert!(r[0].contains("account  @default ▾  F3"), "{header}");
        assert!(
            r[1].contains("theme  ‹default› ▾  dark  light  system  F4 (→ dark)"),
            "{header}"
        );
        assert!(
            r[2].contains("login gateway · box.cursorvm.com   bot Helper"),
            "{header}"
        );
        assert!(r[3].contains("mouse: on (F6)"), "{header}");
        // The sidebar starts under the 4-row header and the account switcher is clickable.
        assert_eq!(app.sidebar_area.y, 4);
        assert_eq!(app.account_area.y, 0);
        let modes: Vec<Mode> = app.theme_areas.iter().map(|(_, m)| *m).collect();
        assert_eq!(modes, Mode::ALL);
        assert!(app.theme_areas.iter().all(|(a, _)| a.y == 1));
    }

    #[test]
    fn clicking_header_switches_theme_and_opens_account_picker() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let (light, _) = app.theme_areas[1];
        app.click(light.x + 1, light.y);
        assert_eq!(app.config.theme_mode, Mode::Light);
        assert_eq!(app.theme_variant(), theme::Variant::Light);
        // Saved as a global pref and survives a reload.
        app.reload_config().unwrap();
        assert_eq!(app.config.theme_mode, Mode::Light);
        term.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(theme::variant(), theme::Variant::Light);
        let r = screen_rows(&term);
        assert!(!r[1].contains("(→"), "{r:#?}");
        // The header background follows the variant.
        let bg = term.backend().buffer()[(60, 3)].bg;
        assert_eq!(
            bg,
            ratatui::style::Style::from(theme::top_bar()).bg.unwrap()
        );
        let acct = app.account_area;
        app.click(acct.x + 2, acct.y);
        assert!(
            app.picker.is_some(),
            "account switcher click opens the picker"
        );
        theme::set_variant(theme::Variant::Dark);
    }

    #[test]
    fn compact_header_switchers_are_clickable() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        assert_eq!(app.theme_areas.len(), 3);
        let (dark, m) = app.theme_areas[0];
        assert_eq!(m, Mode::Dark);
        app.click(dark.x, 0);
        assert_eq!(app.config.theme_mode, Mode::Dark);
        theme::set_variant(theme::Variant::Dark);
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
