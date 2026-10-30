//! Emoji picker modal: a searchable grid of emoji that toggles a reaction on one message.

use crate::app::App;
use crate::emoji::{Choice, Origin};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// Columns per grid cell (glyph padded to 2 + gap).
const CELL: u16 = 4;

pub struct EmojiPicker {
    /// The message the reaction goes on ([`crate::emoji::message_key`]) and its bot.
    pub msg_key: String,
    pub bot: String,
    pub all: Vec<Choice>,
    pub shown: Vec<Choice>,
    pub query: String,
    pub cursor: usize,
    /// First visible grid row.
    pub top: usize,
    /// vim keys: start in list mode (hjkl move, / searches).
    pub typing: bool,
    pub area: Rect,
    pub grid: Rect,
    pub cols: usize,
    /// The [⧉ copy] button (copies the message).
    pub copy_area: Rect,
}

impl EmojiPicker {
    pub fn new(bot: String, msg_key: String, on: &[String], recent: &[String], vim: bool) -> Self {
        let all = crate::emoji::choices(on, recent);
        Self {
            msg_key,
            bot,
            shown: all.clone(),
            all,
            query: String::new(),
            cursor: 0,
            top: 0,
            typing: !vim,
            area: Rect::default(),
            grid: Rect::default(),
            cols: 1,
            copy_area: Rect::default(),
        }
    }

    fn refilter(&mut self) {
        self.shown = crate::emoji::filter(&self.all, &self.query);
        self.cursor = 0;
        self.top = 0;
    }

    fn step(&mut self, d: isize) {
        if self.shown.is_empty() {
            return;
        }
        let max = self.shown.len() as isize - 1;
        self.cursor = (self.cursor as isize + d).clamp(0, max) as usize;
    }

    pub fn selected(&self) -> Option<&Choice> {
        self.shown.get(self.cursor)
    }
}

pub enum Outcome {
    None,
    Close,
    Pick(String),
    /// Copy the message (Ctrl-Y, `y` in list mode, or the [⧉ copy] button).
    Copy,
}

/// Keys: arrows always move; in list mode (vim) hjkl move and `/` starts typing; Enter
/// picks; Esc closes (or leaves typing in vim mode); typing filters.
pub fn key(p: &mut EmojiPicker, k: KeyEvent) -> Outcome {
    let cols = p.cols.max(1) as isize;
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc => return Outcome::Close,
        KeyCode::Enter => {
            return p
                .selected()
                .map(|c| Outcome::Pick(c.glyph.clone()))
                .unwrap_or(Outcome::None);
        }
        KeyCode::Left => p.step(-1),
        KeyCode::Right | KeyCode::Tab => p.step(1),
        KeyCode::BackTab => p.step(-1),
        KeyCode::Up => p.step(-cols),
        KeyCode::Down => p.step(cols),
        KeyCode::PageUp => p.step(-cols * 5),
        KeyCode::PageDown => p.step(cols * 5),
        KeyCode::Char('y') if ctrl => return Outcome::Copy,
        KeyCode::Char('n') if ctrl => p.step(cols),
        KeyCode::Char('p') if ctrl => p.step(-cols),
        KeyCode::Char(c) if !p.typing && !ctrl => match c {
            'h' => p.step(-1),
            'l' => p.step(1),
            'k' => p.step(-cols),
            'j' => p.step(cols),
            'g' => p.cursor = 0,
            'G' => p.cursor = p.shown.len().saturating_sub(1),
            '/' | 'i' => p.typing = true,
            'y' => return Outcome::Copy,
            'q' => return Outcome::Close,
            _ => {}
        },
        KeyCode::Char(c) if !ctrl => {
            p.query.push(c);
            p.refilter();
        }
        KeyCode::Backspace => {
            p.query.pop();
            p.refilter();
        }
        _ => {}
    }
    Outcome::None
}

pub fn mouse(p: &mut EmojiPicker, m: MouseEvent) -> Outcome {
    let cols = p.cols.max(1) as isize;
    match m.kind {
        MouseEventKind::ScrollUp => p.step(-cols),
        MouseEventKind::ScrollDown => p.step(cols),
        MouseEventKind::Down(MouseButton::Left) => {
            let at = Position::new(m.column, m.row);
            if !p.area.contains(at) {
                return Outcome::Close;
            }
            if p.copy_area.contains(at) {
                return Outcome::Copy;
            }
            if p.grid.contains(at) {
                let c = ((m.column - p.grid.x) / CELL) as usize;
                let r = (m.row - p.grid.y) as usize + p.top;
                let i = r * p.cols + c;
                if c < p.cols && i < p.shown.len() {
                    return Outcome::Pick(p.shown[i].glyph.clone());
                }
            }
        }
        _ => {}
    }
    Outcome::None
}

pub fn draw(f: &mut Frame, p: &mut EmojiPicker) {
    let area = crate::history::centered(f.area(), 60, 60);
    p.area = area;
    f.render_widget(Clear, area);
    let hint = if p.typing {
        "type to search · arrows move · Enter toggles · Ctrl-Y copies · Esc closes"
    } else {
        "hjkl move · / search · Enter toggles · y copies · q closes"
    };
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(format!(" React ({}) ", p.shown.len())))
        .title_bottom(format!(" {hint} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 || inner.width < CELL {
        return;
    }
    // Row 0: search field; last row: the selection's name; grid between.
    let search = Line::from(vec![
        theme::key_hint().span("search: "),
        theme::modal_text().span(format!("{}{}", p.query, if p.typing { "▏" } else { "" })),
    ]);
    f.render_widget(Paragraph::new(search), Rect { height: 1, ..inner });
    const COPY: &str = "[⧉ copy]";
    let cw = crate::input::str_width(COPY) as u16;
    p.copy_area = Rect::default();
    if inner.width > cw + 12 {
        let r = Rect::new(inner.x + inner.width - cw, inner.y, cw, 1);
        f.render_widget(Paragraph::new(theme::copy_button().span(COPY)), r);
        p.copy_area = r;
    }
    let grid = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height - 2,
    };
    p.grid = grid;
    p.cols = (grid.width / CELL).max(1) as usize;
    let rows = grid.height as usize;
    let cur_row = p.cursor / p.cols;
    if cur_row < p.top {
        p.top = cur_row;
    } else if cur_row >= p.top + rows {
        p.top = cur_row + 1 - rows;
    }
    let mut lines = vec![];
    for r in p.top..(p.top + rows) {
        let mut spans = vec![];
        for c in 0..p.cols {
            let i = r * p.cols + c;
            let Some(ch) = p.shown.get(i) else { break };
            let w = crate::input::str_width(&ch.glyph);
            let cell = format!(" {}{} ", ch.glyph, " ".repeat(2usize.saturating_sub(w)));
            let st = if i == p.cursor {
                theme::selected()
            } else if ch.origin == Origin::OnMessage {
                theme::reaction_pill()
            } else {
                theme::modal_text()
            };
            spans.push(st.span(cell));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), grid);
    let info = match p.selected() {
        Some(c) => {
            let origin = match c.origin {
                Origin::OnMessage => " · on message (Enter removes)",
                Origin::Recent => " · recent",
                Origin::Common => " · common",
                Origin::All => "",
            };
            format!(" {} :{}: {} · {}{origin}", c.glyph, c.code, c.name, c.group)
        }
        None => " no match".into(),
    };
    f.render_widget(
        Paragraph::new(Line::from(theme::key_hint().span(info))),
        Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        },
    );
}

impl App {
    /// Open the picker on the message with `msg_key` (in the active bot's chat).
    pub fn open_react_picker_for(&mut self, msg_key: String) {
        let bot = self.key_of_active();
        if bot.is_empty() {
            return;
        }
        let on: Vec<String> = self
            .store
            .reactions(&bot)
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, _)| *k == msg_key)
            .map(|(_, e)| e)
            .collect();
        let recent = self.store.recent_reactions(12).unwrap_or_default();
        let vim = self.keys_are_vim();
        self.react_picker = Some(EmojiPicker::new(bot, msg_key, &on, &recent, vim));
    }

    /// Open the picker on the nth newest bot message. False when there is none.
    pub fn open_react_picker(&mut self, nth: usize) -> bool {
        match self.react_target(nth) {
            Some(k) => {
                self.open_react_picker_for(k);
                true
            }
            None => false,
        }
    }

    fn finish_picker(&mut self, o: Outcome) {
        match o {
            Outcome::None => {}
            Outcome::Close => self.react_picker = None,
            Outcome::Copy => {
                if let Some(p) = self.react_picker.take() {
                    self.copy_message(&p.msg_key);
                }
            }
            Outcome::Pick(g) => {
                if let Some(p) = self.react_picker.take() {
                    match self.store.toggle_reaction(&p.bot, &p.msg_key, &g) {
                        Ok(_) => self.reaction_cache = None,
                        Err(e) => self.error(format!("Couldn't save the reaction: {e:#}")),
                    }
                }
            }
        }
    }

    pub fn react_picker_key(&mut self, k: KeyEvent) {
        if let Some(p) = self.react_picker.as_mut() {
            let o = key(p, k);
            self.finish_picker(o);
        }
    }

    pub fn react_picker_mouse(&mut self, m: MouseEvent) {
        if let Some(p) = self.react_picker.as_mut() {
            let o = mouse(p, m);
            self.finish_picker(o);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{App, Entry, EntryKind};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn setup(rt: &tokio::runtime::Runtime) -> (App, tempfile::TempDir) {
        let (mut app, d) = crate::app::tests::test_app(rt);
        crate::app::tests::mock_login(&mut app, "tok");
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![
                Entry {
                    kind: EntryKind::Bot,
                    text: "older".into(),
                },
                Entry {
                    kind: EntryKind::User,
                    text: "hi".into(),
                },
                Entry {
                    kind: EntryKind::Bot,
                    text: "newest".into(),
                },
            ],
        );
        (app, d)
    }

    fn tab_labels(app: &mut App, input: &str) -> (String, Vec<String>) {
        app.completion = None;
        app.input = input.into();
        app.input_cursor = input.len();
        crate::complete::tab(app, false);
        let labels = app
            .completion
            .as_ref()
            .map(|p| p.items.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default();
        (app.input.clone(), labels)
    }

    #[test]
    fn react_completion_suggests_on_message_then_recent_then_common() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = setup(&rt);
        app.react("🚀", 2).unwrap(); // recent (on the older message)
        app.react("🔥", 1).unwrap(); // on the newest message
        let (input, l) = tab_labels(&mut app, "/react ");
        assert_eq!(input, "/react 🔥", "glyph inserted");
        assert_eq!(l[0], ":fire: 🔥 · on message");
        assert_eq!(l[1], ":rocket: 🚀 · recent");
        assert!(l[2].contains("👍"), "{l:?}");
        // with or without the colon, fuzzy on the label
        assert_eq!(tab_labels(&mut app, "/react :thumbsu").0, "/react 👍");
        assert_eq!(tab_labels(&mut app, "/react thumbsu").0, "/react 👍");
        // unreact: only what is on the target message
        let (input, l) = tab_labels(&mut app, "/unreact ");
        assert_eq!((input.as_str(), l.len()), ("/unreact 🔥", 1));
        // [n]: the older message has 🚀
        let (_, l) = tab_labels(&mut app, "/unreact x 2");
        assert!(
            l.is_empty()
                || l.iter().all(|s| s.chars().all(|c| c.is_ascii_digit()
                    || c == ' '
                    || c.is_alphabetic()
                    || "()".contains(c)))
        );
        let (input, _) = tab_labels(&mut app, "/react 🔥 ");
        assert!(input.ends_with(" 1"), "index completes: {input}");
    }

    #[test]
    fn react_button_click_opens_picker_search_and_enter_toggles() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = setup(&rt);
        let draw = |app: &mut App| {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 30)).unwrap();
            term.draw(|f| crate::ui::draw(f, app)).unwrap();
            crate::ui::tests::screen_rows(&term).join("\n")
        };
        let s = draw(&mut app);
        assert!(s.contains("[☺+]"), "{s}");
        assert_eq!(app.react_hits.len(), 3, "one button per message");
        let h = app.react_hits.last().cloned().unwrap();
        app.click(h.x + 1, h.y);
        assert!(app.react_picker.is_some());
        let k = |app: &mut App, c| app.react_picker_key(KeyEvent::new(c, KeyModifiers::NONE));
        for c in "tada".chars() {
            k(&mut app, KeyCode::Char(c));
        }
        let s = draw(&mut app);
        assert!(s.contains(":tada:"), "{s}");
        k(&mut app, KeyCode::Enter);
        assert!(app.react_picker.is_none());
        let s = draw(&mut app);
        assert!(s.contains("🎉"), "{s}");
        // Reopen on the same message: 🎉 is listed first and Enter removes it.
        let h = app.react_hits.last().cloned().unwrap();
        app.click(h.x, h.y);
        assert_eq!(app.react_picker.as_ref().unwrap().shown[0].glyph, "🎉");
        k(&mut app, KeyCode::Right);
        k(&mut app, KeyCode::Left);
        k(&mut app, KeyCode::Enter);
        assert!(!draw(&mut app).contains("🎉"));
        // Click outside closes; bare /react opens it on the newest bot message.
        app.input = "/react".into();
        app.submit();
        assert!(app.react_picker.is_some());
        app.react_picker_mouse(ratatui::crossterm::event::MouseEvent {
            kind: ratatui::crossterm::event::MouseEventKind::Down(
                ratatui::crossterm::event::MouseButton::Left,
            ),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.react_picker.is_none());
    }
}
