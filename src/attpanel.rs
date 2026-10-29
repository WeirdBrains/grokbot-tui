//! Attachments panel (1.23.0): a collapsible bar on the right of the chat listing every
//! attachment in it (sent and received) with [view] and [⤓] buttons. Clicking a row jumps
//! to its message. Collapses to a 1-column strip with the count.
use crate::app::{App, EntryKind};
use crate::theme;
use grokbot_api as gw;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Expanded panel width.
pub const WIDTH: u16 = 34;
/// The panel only shows when the chat body is at least this wide.
pub const MIN_BODY: u16 = 70;

#[derive(Debug, Clone, PartialEq)]
pub enum Src {
    /// A file we sent: its store alias/path.
    Sent(String),
    /// A link or image in a reply.
    Url(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    pub kind: gw::AttachmentKind,
    pub size: Option<u64>,
    pub sender: String,
    pub time: Option<String>,
    /// Feed entry index (for jumping).
    pub entry: usize,
    pub src: Src,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TypeFilter {
    #[default]
    All,
    Images,
    Video,
    Audio,
    Docs,
}

impl TypeFilter {
    pub const ALL: [TypeFilter; 5] = [
        Self::All,
        Self::Images,
        Self::Video,
        Self::Audio,
        Self::Docs,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Images => "Img",
            Self::Video => "Vid",
            Self::Audio => "Aud",
            Self::Docs => "Doc",
        }
    }
    pub fn keeps(self, k: gw::AttachmentKind) -> bool {
        use gw::AttachmentKind as K;
        match self {
            Self::All => true,
            Self::Images => k == K::Image,
            Self::Video => k == K::Video,
            Self::Audio => k == K::Audio,
            Self::Docs => k == K::File,
        }
    }
}

#[derive(Debug, Default)]
pub struct Panel {
    pub focused: bool,
    pub cursor: usize,
    pub scroll: usize,
    pub filter: String,
    pub editing: bool,
    pub types: TypeFilter,
    pub area: Rect,
    /// (rect, item index, action) for clicks: 0 = row, 1 = view, 2 = download.
    pub hits: Vec<(Rect, usize, u8)>,
    pub type_hits: Vec<(Rect, TypeFilter)>,
    pub toggle_area: Rect,
    pub filter_area: Rect,
}

/// Every attachment in `feed`, oldest first.
pub fn items(app: &App) -> Vec<Item> {
    let feed = app.feed();
    let bot_name = app
        .active_bot()
        .map(|b| b.name.clone())
        .unwrap_or_else(|| "bot".into());
    let times = app.msg_times.get(&app.key());
    let mut out = vec![];
    for (i, e) in feed.iter().enumerate() {
        let time = times.and_then(|t| t.get(i)).cloned();
        match e.kind {
            EntryKind::User => {
                let k = crate::emoji::message_key(e.kind as u8, &e.text);
                for a in app.entry_attachments(&k) {
                    out.push(Item {
                        kind: if a.is_image() {
                            gw::AttachmentKind::Image
                        } else {
                            gw::AttachmentKind::from_name(&a.name)
                        },
                        name: a.name.clone(),
                        size: a.size,
                        sender: "You".into(),
                        time: time.clone(),
                        entry: i,
                        src: Src::Sent(a.path.clone()),
                    });
                }
            }
            EntryKind::Bot => {
                for o in crate::download::downloadables(&e.text) {
                    let name = crate::download::save_name(&o.url);
                    let kind = if o.image {
                        gw::AttachmentKind::Image
                    } else {
                        gw::AttachmentKind::from_name(&name)
                    };
                    out.push(Item {
                        name,
                        kind,
                        size: None,
                        sender: bot_name.clone(),
                        time: time.clone(),
                        entry: i,
                        src: Src::Url(o.url),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Items after the type filter and the search box.
pub fn filtered(app: &App) -> Vec<Item> {
    let q = app.attpanel.filter.to_lowercase();
    items(app)
        .into_iter()
        .filter(|it| app.attpanel.types.keeps(it.kind))
        .filter(|it| {
            q.is_empty()
                || it.name.to_lowercase().contains(&q)
                || it.sender.to_lowercase().contains(&q)
        })
        .collect()
}

/// Text-like files the built-in browser can preview.
pub fn textual(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        ".txt",
        ".md",
        ".markdown",
        ".html",
        ".htm",
        ".json",
        ".csv",
        ".log",
        ".xml",
        ".yaml",
        ".yml",
        ".toml",
        ".rs",
        ".py",
        ".js",
        ".ts",
        ".sh",
        ".c",
        ".h",
        ".go",
        ".java",
        ".ini",
    ]
    .iter()
    .any(|e| n.ends_with(e))
}

fn rs(s: ratatui_lipgloss::Style) -> Style {
    s.into()
}

/// Draw the panel (expanded or the 1-column strip) into `area`.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let collapsed = !app.config.attachments_panel;
    let list = filtered(app);
    let total = items(app).len();
    let p = &mut app.attpanel;
    p.area = area;
    p.hits.clear();
    p.type_hits.clear();
    p.filter_area = Rect::default();
    if area.width == 0 || area.height == 0 {
        return;
    }
    if collapsed {
        let mut lines = vec![Line::from(Span::styled("«", rs(theme::collapse_button())))];
        let count = total.to_string();
        lines.push(Line::from(Span::styled(
            "📎",
            rs(theme::attach_panel_count()),
        )));
        for ch in count.chars() {
            lines.push(Line::from(Span::styled(
                ch.to_string(),
                rs(theme::attach_panel_count()),
            )));
        }
        f.render_widget(Block::new().style(rs(theme::sidebar_strip())), area);
        f.render_widget(Paragraph::new(lines), area);
        p.toggle_area = area;
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(rs(theme::attach_panel_frame()))
        .title(Span::styled(
            format!(" Files ({total}) "),
            rs(theme::attach_panel_frame()),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    // » collapse button on the top border.
    let t = Rect::new(area.x + area.width.saturating_sub(4), area.y, 3, 1);
    f.render_widget(
        Paragraph::new(Span::styled(" » ", rs(theme::collapse_button()))),
        t,
    );
    p.toggle_area = t;
    if inner.height < 3 {
        return;
    }
    // Row 1: search box.
    let fl = if p.editing {
        format!("/{}▏", p.filter)
    } else if p.filter.is_empty() {
        "/ filter…".to_string()
    } else {
        format!("/{}", p.filter)
    };
    p.filter_area = Rect { height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Span::styled(fl, rs(theme::attach_panel_filter()))),
        p.filter_area,
    );
    // Row 2: type filters.
    let mut x = inner.x;
    let mut spans = vec![];
    for tf in TypeFilter::ALL {
        let l = format!(" {} ", tf.label());
        let w = l.chars().count() as u16;
        let st = if tf == p.types {
            theme::selected()
        } else {
            theme::key_hint()
        };
        if x + w <= inner.x + inner.width {
            p.type_hits.push((Rect::new(x, inner.y + 1, w, 1), tf));
        }
        spans.push(Span::styled(l, rs(st)));
        x += w;
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            y: inner.y + 1,
            height: 1,
            ..inner
        },
    );
    // Rows: two lines per item.
    let body = Rect {
        y: inner.y + 2,
        height: inner.height - 2,
        ..inner
    };
    let per_page = (body.height / 2).max(1) as usize;
    if list.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                if total == 0 {
                    " No attachments in this chat"
                } else {
                    " No matches"
                },
                rs(theme::muted()),
            )),
            body,
        );
        return;
    }
    p.cursor = p.cursor.min(list.len() - 1);
    if p.cursor < p.scroll {
        p.scroll = p.cursor;
    } else if p.cursor >= p.scroll + per_page {
        p.scroll = p.cursor + 1 - per_page;
    }
    let w = body.width as usize;
    let mut lines: Vec<Line> = vec![];
    for (row, (i, it)) in list
        .iter()
        .enumerate()
        .skip(p.scroll)
        .take(per_page)
        .enumerate()
    {
        let sel = i == p.cursor && p.focused;
        let name_st = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let icon = crate::attach::icon(it.kind, &it.name);
        let mut name: String = format!("{icon} {}", it.name);
        if crate::input::str_width(&name) > w {
            name = name.chars().take(w.saturating_sub(1)).collect::<String>() + "…";
        }
        lines.push(Line::from(Span::styled(name, rs(name_st))));
        let meta = format!(
            "{} · {}{}",
            it.size
                .map(crate::markdown::human_size)
                .unwrap_or_else(|| "—".into()),
            it.sender,
            it.time
                .as_deref()
                .map(|t| format!(" · {t}"))
                .unwrap_or_default()
        );
        let buttons = " [view] [⤓]";
        let bw = crate::input::str_width(buttons);
        let room = w.saturating_sub(bw);
        let meta: String = if crate::input::str_width(&meta) > room {
            meta.chars()
                .take(room.saturating_sub(1))
                .collect::<String>()
                + "…"
        } else {
            format!("{meta:<room$}")
        };
        lines.push(Line::from(vec![
            Span::styled(meta, rs(theme::muted())),
            Span::styled(" [view]", rs(theme::download_button())),
            Span::styled(" [⤓]", rs(theme::download_button())),
        ]));
        let y = body.y + (row as u16) * 2;
        p.hits.push((Rect::new(body.x, y, body.width, 2), i, 0));
        let bx = body.x + room as u16;
        p.hits.push((Rect::new(bx + 1, y + 1, 6, 1), i, 1));
        p.hits.push((Rect::new(bx + 8, y + 1, 3, 1), i, 2));
    }
    f.render_widget(Paragraph::new(lines), body);
}

impl App {
    /// `ui.attachments_panel`: show (expanded) or hide (1-column strip).
    pub fn set_attachments_panel(&mut self, on: bool) {
        if let Err(e) = self.store.set_pref(
            crate::config::PREF_ATTACHMENTS_PANEL,
            if on { "show" } else { "hide" },
        ) {
            self.error(format!("{e:#}"));
            return;
        }
        self.config.attachments_panel = on;
        self.attpanel.focused = on;
        self.needs_clear = true;
    }

    /// View an attachment: images in the viewer, text in the built-in browser, else the
    /// system app (SSH asks first).
    pub fn attachment_view(&mut self, it: &Item) {
        if let Some(kind) = crate::media::kind_of(&it.name) {
            let target = match &it.src {
                Src::Sent(p) => format!("{}{p}", crate::attach::SCHEME),
                Src::Url(u) => u.clone(),
            };
            let key = format!("{}{target}", crate::media::PREFIX);
            self.media_want(&key, kind, &it.name);
            self.open_media_modal(&key);
            return;
        }
        match &it.src {
            Src::Sent(path) => self.open_attachment(path, crate::attach::Then::View),
            Src::Url(url) => {
                if it.kind == gw::AttachmentKind::Image && self.view_image(Some(url)) {
                    return;
                }
                if url.starts_with("http://") || url.starts_with("https://") {
                    if textual(&it.name) || !url.contains('?') {
                        self.open_browser(Some(url));
                    } else {
                        self.open_system(url);
                    }
                } else {
                    self.open_system(url);
                }
            }
        }
    }

    pub fn attachment_download(&mut self, it: &Item) {
        match &it.src {
            Src::Sent(path) => self.open_attachment(path, crate::attach::Then::Download),
            Src::Url(url) => self.open_download(url),
        }
    }

    /// Scroll the chat so entry `i` is at the top (applied at the next draw).
    pub fn jump_to_entry(&mut self, i: usize) {
        self.pending_jump = Some(i);
        self.follow = false;
    }

    fn panel_act(&mut self, idx: usize, action: u8) {
        let list = filtered(self);
        let Some(it) = list.get(idx).cloned() else {
            return;
        };
        self.attpanel.cursor = idx;
        match action {
            1 => self.attachment_view(&it),
            2 => self.attachment_download(&it),
            _ => self.jump_to_entry(it.entry),
        }
    }

    /// A left click; true when it was on the panel.
    pub fn attpanel_click(&mut self, col: u16, row: u16) -> bool {
        let at = Position::new(col, row);
        if !self.attpanel.area.contains(at) {
            if self.attpanel.focused {
                self.attpanel.focused = false;
                self.attpanel.editing = false;
            }
            return false;
        }
        if self.attpanel.toggle_area.contains(at) {
            self.set_attachments_panel(!self.config.attachments_panel);
            return true;
        }
        if !self.config.attachments_panel {
            self.set_attachments_panel(true);
            return true;
        }
        self.attpanel.focused = true;
        if self.attpanel.filter_area.contains(at) {
            self.attpanel.editing = true;
            return true;
        }
        if let Some(&(_, tf)) = self.attpanel.type_hits.iter().find(|(r, _)| r.contains(at)) {
            self.attpanel.types = tf;
            self.attpanel.cursor = 0;
            return true;
        }
        // Buttons first (they sit inside the row rect).
        let hit = self
            .attpanel
            .hits
            .iter()
            .filter(|(r, _, _)| r.contains(at))
            .max_by_key(|(_, _, a)| *a)
            .map(|&(_, i, a)| (i, a));
        if let Some((i, a)) = hit {
            self.panel_act(i, a);
        }
        true
    }

    /// Wheel over the panel; true when consumed.
    pub fn attpanel_wheel(&mut self, col: u16, row: u16, down: bool) -> bool {
        if !self.config.attachments_panel || !self.attpanel.area.contains(Position::new(col, row)) {
            return false;
        }
        let n = filtered(self).len();
        let c = &mut self.attpanel.cursor;
        *c = if down {
            (*c + 1).min(n.saturating_sub(1))
        } else {
            c.saturating_sub(1)
        };
        true
    }

    /// Keys while the panel has focus.
    pub fn attpanel_key(&mut self, k: KeyEvent) {
        let p = &mut self.attpanel;
        if p.editing {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => p.editing = false,
                KeyCode::Backspace => {
                    p.filter.pop();
                }
                KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                    p.filter.push(c);
                    p.cursor = 0;
                }
                _ => {}
            }
            return;
        }
        let n = filtered(self).len();
        let p = &mut self.attpanel;
        match k.code {
            KeyCode::Esc => p.focused = false,
            KeyCode::Char('j') | KeyCode::Down => {
                p.cursor = (p.cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => p.cursor = p.cursor.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => p.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => p.cursor = n.saturating_sub(1),
            KeyCode::Char('/') => p.editing = true,
            KeyCode::Char('t') => {
                let i = TypeFilter::ALL
                    .iter()
                    .position(|t| *t == p.types)
                    .unwrap_or(0);
                p.types = TypeFilter::ALL[(i + 1) % TypeFilter::ALL.len()];
                p.cursor = 0;
            }
            KeyCode::Enter => {
                let c = p.cursor;
                self.panel_act(c, 0);
            }
            KeyCode::Char('v') => {
                let c = p.cursor;
                self.panel_act(c, 1);
            }
            KeyCode::Char('d') => {
                let c = p.cursor;
                self.panel_act(c, 2);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Entry;
    use ratatui::{Terminal, backend::TestBackend};

    fn setup() -> (App, tempfile::TempDir, tokio::runtime::Runtime) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let bot = app.active_bot().unwrap().id.clone();
        let mut log: Vec<Entry> = (0..60)
            .map(|i| Entry {
                kind: EntryKind::Bot,
                text: format!("filler {i}"),
            })
            .collect();
        log.insert(
            3,
            Entry {
                kind: EntryKind::Bot,
                text: "here ![cat](https://x.test/cat.png) and [notes](https://x.test/notes.md)"
                    .into(),
            },
        );
        log.push(Entry {
            kind: EntryKind::Bot,
            text: "song https://x.test/a/tune.mp3".into(),
        });
        app.logs.insert(bot, log);
        (app, d, rt)
    }

    fn screen(term: &mut Terminal<TestBackend>, app: &mut App) -> Vec<String> {
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let b = term.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn lists_received_items_and_filters_by_type_and_text() {
        let (mut app, _d, _rt) = setup();
        let all = items(&app);
        let names: Vec<_> = all.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["cat.png", "notes.md", "tune.mp3"]);
        assert_eq!(all[0].kind, gw::AttachmentKind::Image);
        assert_eq!(all[0].entry, 3);
        app.attpanel.types = TypeFilter::Audio;
        assert_eq!(filtered(&app).len(), 1);
        app.attpanel.types = TypeFilter::All;
        app.attpanel.filter = "NOTE".into();
        assert_eq!(filtered(&app)[0].name, "notes.md");
        assert!(textual("a.MD") && !textual("a.pdf"));
    }

    #[test]
    fn collapsed_strip_expands_on_click_and_rows_jump() {
        let (mut app, _d, _rt) = setup();
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let s = screen(&mut term, &mut app);
        let a = app.attpanel.area;
        assert_eq!(a.width, 1, "collapsed by default");
        assert_eq!(a.x, 119);
        assert!(s.iter().any(|l| l.ends_with('«')));
        assert!(app.attpanel_click(a.x, a.y + 2));
        assert!(app.config.attachments_panel && app.attpanel.focused);
        let s = screen(&mut term, &mut app);
        assert_eq!(app.attpanel.area.width, WIDTH);
        assert!(s.iter().any(|l| l.contains("Files (3)")));
        assert!(s.iter().any(|l| l.contains("cat.png")));
        // Row click → jump to entry 3 (far up the feed).
        let (r, i, _) = app.attpanel.hits[0];
        assert_eq!(i, 0);
        assert!(app.attpanel_click(r.x, r.y));
        let s = screen(&mut term, &mut app);
        assert!(s.iter().any(|l| l.contains("here")), "jumped: {s:#?}");
        assert!(!app.follow);
        // Clicking the chat body takes focus away but is not consumed.
        assert!(!app.attpanel_click(20, 10));
        assert!(!app.attpanel.focused);
        // The pref persists.
        assert_eq!(
            app.store
                .get_pref(crate::config::PREF_ATTACHMENTS_PANEL)
                .unwrap()
                .as_deref(),
            Some("show")
        );
        // » collapses.
        let t = app.attpanel.toggle_area;
        assert!(app.attpanel_click(t.x, t.y));
        assert!(!app.config.attachments_panel);
    }

    #[test]
    fn keyboard_and_buttons() {
        let (mut app, _d, _rt) = setup();
        app.set_attachments_panel(true);
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        screen(&mut term, &mut app);
        let k = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
        app.attpanel_key(k('j'));
        app.attpanel_key(k('j'));
        app.attpanel_key(k('j'));
        assert_eq!(app.attpanel.cursor, 2);
        app.attpanel_key(k('k'));
        app.attpanel_key(k('d'));
        assert!(app.download.is_some(), "⤓ opens the download modal");
        app.download = None;
        app.attpanel_key(k('t'));
        assert_eq!(app.attpanel.types, TypeFilter::Images);
        app.attpanel_key(k('t'));
        app.attpanel_key(k('t'));
        app.attpanel_key(k('t'));
        app.attpanel_key(k('t'));
        app.attpanel_key(k('/'));
        app.attpanel_key(k('t'));
        app.attpanel_key(k('u'));
        app.attpanel_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.attpanel.filter, "tu");
        assert_eq!(filtered(&app).len(), 1);
        // [view] on a text file opens the built-in browser.
        app.attpanel.filter = "notes".into();
        screen(&mut term, &mut app);
        let (r, _, _) = *app.attpanel.hits.iter().find(|h| h.2 == 1).unwrap();
        assert!(app.attpanel_click(r.x, r.y));
        assert!(app.browser.is_some());
    }

    #[test]
    fn narrow_terminals_hide_the_panel_and_sidebar_coexists() {
        let (mut app, _d, _rt) = setup();
        app.set_attachments_panel(true);
        app.set_sidebar_collapsed(true);
        app.set_header_compact(true);
        let mut term = Terminal::new(TestBackend::new(90, 24)).unwrap();
        screen(&mut term, &mut app);
        assert_eq!(app.attpanel.area.x + app.attpanel.area.width, 90);
        let mut term = Terminal::new(TestBackend::new(60, 24)).unwrap();
        screen(&mut term, &mut app);
        assert_eq!(app.attpanel.area.width, 0);
    }
}
