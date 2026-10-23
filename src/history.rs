//! Prompt history: Up/Down in the input, the Ctrl-R fuzzy search and the `/history`
//! browser. Both overlays are the same [`Search`]; history lives in `context.db`
//! (`prompt_history`, per account, newest `history.max_entries` kept) so it survives
//! restarts.

use crate::fuzzy;
use crate::storage::PromptEntry;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// Wheel step in the overlay.
pub const WHEEL_STEP: i32 = 3;

/// The history overlay (Ctrl-R search or `/history` browser).
#[derive(Debug, Clone)]
pub struct Search {
    /// Query typed into the filter field.
    pub query: String,
    /// Indices into the app's history (newest-first) of the current matches.
    pub matches: Vec<(usize, fuzzy::Match)>,
    pub cursor: usize,
    /// First visible match row (kept so the wheel scrolls the list, not just the cursor).
    pub top: usize,
    /// The input to restore on Esc.
    pub draft: String,
    pub draft_cursor: usize,
    /// Opened by `/history` (browser title) rather than Ctrl-R.
    pub browse: bool,
    /// `ui.keys = vim`: j/k/g/G/Ctrl-d/Ctrl-u move in the list; `/` or `i` edits the filter.
    pub vim: bool,
    /// Keys go to the filter field (always true without vim keys).
    pub typing: bool,
    /// Clickable rows, set by the renderer.
    pub list_area: Rect,
    pub list_rows: Vec<usize>,
}

impl Search {
    pub fn new(history: &[String], draft: String, draft_cursor: usize) -> Self {
        let mut s = Self {
            query: String::new(),
            matches: vec![],
            cursor: 0,
            top: 0,
            draft,
            draft_cursor,
            browse: false,
            vim: false,
            typing: true,
            list_area: Rect::default(),
            list_rows: vec![],
        };
        s.refilter(history);
        s
    }

    /// The `/history [query]` browser. With vim keys it starts in list mode.
    pub fn browser(
        history: &[String],
        query: &str,
        vim: bool,
        draft: String,
        draft_cursor: usize,
    ) -> Self {
        let mut s = Self::new(&[], draft, draft_cursor);
        s.query = query.to_string();
        s.browse = true;
        s.vim = vim;
        s.typing = !vim;
        s.refilter(history);
        s
    }

    pub fn refilter(&mut self, history: &[String]) {
        self.matches = fuzzy::rank(&self.query, history);
        if self.cursor >= self.matches.len() {
            self.cursor = self.matches.len().saturating_sub(1);
        }
        self.top = self.top.min(self.cursor);
    }

    pub fn selected(&self) -> Option<usize> {
        self.matches.get(self.cursor).map(|(i, _)| *i)
    }

    /// Move the cursor, wrapping at the ends (Ctrl-R / Ctrl-S cycle).
    pub fn move_by(&mut self, d: i32) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as i32;
        self.cursor = (self.cursor as i32 + d).rem_euclid(n) as usize;
    }

    /// Move the cursor, stopping at the ends (paging, wheel).
    pub fn move_clamped(&mut self, d: i32) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() as i64 - 1;
        self.cursor = (self.cursor as i64 + d as i64).clamp(0, last) as usize;
    }

    /// Mouse wheel: scroll the list by `d` rows and keep the cursor visible.
    pub fn wheel(&mut self, d: i32) {
        if self.matches.is_empty() {
            return;
        }
        let h = (self.list_area.height as usize).max(1);
        let max_top = self.matches.len().saturating_sub(h);
        self.top = (self.top as i64 + d as i64).clamp(0, max_top as i64) as usize;
        if self.cursor < self.top {
            self.cursor = self.top;
        } else if self.cursor >= self.top + h {
            self.cursor = self.top + h - 1;
        }
    }

    fn page(&self) -> i32 {
        (self.list_area.height as i32).max(2) - 1
    }

    /// Left click at a screen cell: select the row; clicking the selected row takes it.
    pub fn click(&mut self, history: &[String], col: u16, row: u16) -> Outcome {
        let a = self.list_area;
        if col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return Outcome::None;
        }
        let r = self.top + (row - a.y) as usize;
        if r >= self.matches.len() {
            return Outcome::None;
        }
        if r == self.cursor {
            return Outcome::Take(history[self.matches[r].0].clone());
        }
        self.cursor = r;
        Outcome::None
    }
}

/// A key while the overlay is open. `Take(text)` puts it in the input (never sends);
/// `Cancel` restores the draft; `None` stays open.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Take(String),
    Cancel,
}

pub fn key(s: &mut Search, history: &[String], k: KeyEvent) -> Outcome {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let take = |s: &Search| {
        s.selected()
            .map(|i| Outcome::Take(history[i].clone()))
            .unwrap_or(Outcome::None)
    };
    // Keys that work the same in every mode.
    match k.code {
        KeyCode::Char('g' | 'c') if ctrl => return Outcome::Cancel,
        KeyCode::Enter => return take(s),
        KeyCode::Up => {
            s.move_clamped(-1);
            return Outcome::None;
        }
        KeyCode::Down => {
            s.move_clamped(1);
            return Outcome::None;
        }
        KeyCode::Char('r' | 'p') if ctrl => {
            s.move_by(-1);
            return Outcome::None;
        }
        KeyCode::Char('s' | 'n') if ctrl => {
            s.move_by(1);
            return Outcome::None;
        }
        KeyCode::PageUp => {
            s.move_clamped(-s.page());
            return Outcome::None;
        }
        KeyCode::PageDown => {
            s.move_clamped(s.page());
            return Outcome::None;
        }
        KeyCode::Home if !s.typing => {
            s.cursor = 0;
            return Outcome::None;
        }
        KeyCode::End if !s.typing => {
            s.cursor = s.matches.len().saturating_sub(1);
            return Outcome::None;
        }
        _ => {}
    }
    if s.vim && !s.typing {
        // Vim list mode.
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Cancel,
            KeyCode::Char('j') => {
                s.move_clamped(1);
                Outcome::None
            }
            KeyCode::Char('k') => {
                s.move_clamped(-1);
                Outcome::None
            }
            KeyCode::Char('d') if ctrl => {
                s.move_clamped(s.page() / 2 + 1);
                Outcome::None
            }
            KeyCode::Char('u') if ctrl => {
                s.move_clamped(-(s.page() / 2 + 1));
                Outcome::None
            }
            KeyCode::Char('g') => {
                s.cursor = 0;
                Outcome::None
            }
            KeyCode::Char('G') => {
                s.cursor = s.matches.len().saturating_sub(1);
                Outcome::None
            }
            KeyCode::Char('/' | 'i' | 'a') => {
                s.typing = true;
                Outcome::None
            }
            _ => Outcome::None,
        }
    } else {
        match k.code {
            // Vim: Esc leaves the filter for the list; default keys: Esc closes.
            KeyCode::Esc if s.vim => {
                s.typing = false;
                Outcome::None
            }
            KeyCode::Esc => Outcome::Cancel,
            KeyCode::Backspace => {
                s.query.pop();
                s.refilter(history);
                Outcome::None
            }
            KeyCode::Char('u') if ctrl => {
                s.query.clear();
                s.refilter(history);
                Outcome::None
            }
            KeyCode::Char(c) if !ctrl => {
                s.query.push(c);
                s.cursor = 0;
                s.refilter(history);
                Outcome::None
            }
            _ => Outcome::None,
        }
    }
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse SQLite's UTC `YYYY-MM-DD HH:MM:SS` into Unix seconds.
pub fn parse_utc(s: &str) -> Option<i64> {
    let (date, time) = s.trim().split_once([' ', 'T'])?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.trim_end_matches('Z').split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let se: i64 = t
        .next()
        .and_then(|x| x.split('.').next())
        .and_then(|x| x.parse().ok())
        .unwrap_or(0);
    Some(days_from_civil(y, mo, da) * 86_400 + h * 3600 + mi * 60 + se)
}

/// Short age of a UTC timestamp relative to `now` (Unix seconds): `now`, `5m`, `3h`,
/// `2d`, else the date. Zone-free, so it reads right in any local time zone.
pub fn age(at: &str, now: i64) -> String {
    let Some(t) = parse_utc(at) else {
        return String::new();
    };
    let d = (now - t).max(0);
    match d {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", d / 60),
        3_600..86_400 => format!("{}h", d / 3_600),
        86_400..2_592_000 => format!("{}d", d / 86_400),
        _ => at.get(..10).unwrap_or(at).to_string(),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn draw(f: &mut Frame, s: &mut Search, history: &[String], meta: &[PromptEntry], area: Rect) {
    use crate::theme;
    let r = centered(area, 80, 60);
    f.render_widget(Clear, r);
    let title = match (s.browse, s.vim, s.typing) {
        (_, true, false) => format!(
            " Prompt history ({}) · j/k move · / filter · Enter puts in input · q closes ",
            s.matches.len()
        ),
        (_, true, true) => format!(
            " Prompt history ({}) · type to filter · Esc to list · Enter puts in input ",
            s.matches.len()
        ),
        (true, false, _) => format!(
            " Prompt history ({}) · type to filter · ↑/↓ · Enter puts in input · Esc ",
            s.matches.len()
        ),
        (false, false, _) => " History · Ctrl-R · Enter uses · Esc keeps ".to_string(),
    };
    let block = theme::block(&theme::help_frame()).title(title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let [q, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(inner);
    let caret = if s.typing { "▏" } else { "" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            theme::key_hint().span("› "),
            theme::modal_text().span(format!("{}{caret}", s.query)),
        ])),
        q,
    );
    let h = (list.height as usize).max(1);
    // Keep the cursor visible, preserving the wheel's scroll position otherwise.
    if s.cursor < s.top {
        s.top = s.cursor;
    } else if s.cursor >= s.top + h {
        s.top = s.cursor + 1 - h;
    }
    s.top = s.top.min(s.matches.len().saturating_sub(h));
    let now = now_unix();
    let mut lines = vec![];
    s.list_rows.clear();
    if s.matches.is_empty() {
        lines.push(Line::from(theme::muted().span(if history.is_empty() {
            "No prompts yet. Everything you send is remembered here."
        } else {
            "No matches."
        })));
    }
    for row in s.top..(s.top + h).min(s.matches.len()) {
        let (i, m) = &s.matches[row];
        let text = &history[*i];
        let sel = row == s.cursor;
        let st = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let mut spans: Vec<Span> = vec![];
        if let Some(e) = meta.get(*i) {
            let bot = e.bot.as_deref().unwrap_or("-");
            let bot: String = bot.chars().take(14).collect();
            let label = format!("{:>10}  {:<14}  ", age(&e.at, now), bot);
            spans.push(if sel {
                st.span(label)
            } else {
                theme::muted().span(label)
            });
        }
        let mut more = false;
        for (ci, ch) in text.chars().enumerate() {
            if ch == '\n' {
                more = true;
                break;
            }
            if m.positions.contains(&ci) {
                spans.push(theme::title().span(ch.to_string()));
            } else {
                spans.push(st.span(ch.to_string()));
            }
        }
        if more {
            spans.push(theme::muted().span(" ⏎…"));
        }
        if spans.is_empty() {
            spans.push(st.span(" "));
        }
        lines.push(Line::from(spans));
        s.list_rows.push(*i);
    }
    s.list_area = list;
    f.render_widget(Paragraph::new(lines), list);
}

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = (area.width as u32 * pct_w as u32 / 100) as u16;
    let h = (area.height as u32 * pct_h as u32 / 100) as u16;
    let w = w.max(20).min(area.width);
    let h = h.max(6).min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// Strip secrets before a submitted line is remembered: `/login …` keeps only the
/// command and method (a pasted token on the same line is never stored).
pub fn redact(line: &str) -> String {
    let t = line.trim();
    let mut words = t.split_whitespace();
    match words.next() {
        Some(cmd) if cmd.eq_ignore_ascii_case("/login") => match words.next() {
            Some(method) => format!("{cmd} {method}"),
            None => cmd.to_string(),
        },
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn search_filters_and_enter_takes_the_match() {
        let h = vec![
            "git push origin main".into(),
            "cargo test".into(),
            "git push --force".into(),
        ];
        let mut s = Search::new(&h, "draft".into(), 0);
        assert_eq!(s.matches.len(), 3, "empty query shows all, recent first");
        s.query = "push".into();
        s.refilter(&h);
        assert_eq!(
            s.matches.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            key(&mut s, &h, k(KeyCode::Enter)),
            Outcome::Take("git push origin main".into())
        );
        s.cursor = 1;
        assert_eq!(
            key(&mut s, &h, k(KeyCode::Enter)),
            Outcome::Take("git push --force".into())
        );
        assert_eq!(key(&mut s, &h, k(KeyCode::Esc)), Outcome::Cancel);
    }

    #[test]
    fn ctrl_r_and_ctrl_s_move_through_results() {
        let h: Vec<String> = (0..5).map(|i| format!("cmd {i}")).collect();
        let mut s = Search::new(&h, String::new(), 0);
        key(
            &mut s,
            &h,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        );
        assert_eq!(s.cursor, 4, "wraps up");
        key(
            &mut s,
            &h,
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        );
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn browser_vim_keys_move_and_slash_filters() {
        let h: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let mut s = Search::browser(&h, "", true, String::new(), 0);
        assert!(!s.typing, "vim browser starts in list mode");
        key(&mut s, &h, k(KeyCode::Char('j')));
        key(&mut s, &h, k(KeyCode::Char('j')));
        assert_eq!(s.cursor, 2);
        key(&mut s, &h, k(KeyCode::Char('k')));
        assert_eq!(s.cursor, 1);
        key(&mut s, &h, k(KeyCode::Char('G')));
        assert_eq!(s.cursor, 2);
        key(&mut s, &h, k(KeyCode::Char('g')));
        assert_eq!(s.cursor, 0);
        assert!(
            s.query.is_empty(),
            "j/k never type into the filter in list mode"
        );
        key(&mut s, &h, k(KeyCode::Char('/')));
        assert!(s.typing);
        for c in "gam".chars() {
            key(&mut s, &h, k(KeyCode::Char(c)));
        }
        assert_eq!(s.query, "gam");
        assert_eq!(s.selected(), Some(2));
        assert_eq!(key(&mut s, &h, k(KeyCode::Esc)), Outcome::None);
        assert!(!s.typing, "Esc leaves the filter first");
        assert_eq!(key(&mut s, &h, k(KeyCode::Char('q'))), Outcome::Cancel);
    }

    #[test]
    fn browser_default_keys_type_j_and_k_into_the_filter() {
        let h: Vec<String> = vec!["jq .".into(), "kubectl get pods".into()];
        let mut s = Search::browser(&h, "", false, String::new(), 0);
        assert!(s.typing);
        key(&mut s, &h, k(KeyCode::Char('k')));
        assert_eq!(s.query, "k");
        assert_eq!(s.selected(), Some(1));
        assert_eq!(
            key(&mut s, &h, k(KeyCode::Enter)),
            Outcome::Take("kubectl get pods".into())
        );
    }

    #[test]
    fn wheel_scrolls_and_click_selects_then_takes() {
        let h: Vec<String> = (0..50).map(|i| format!("p{i}")).collect();
        let mut s = Search::browser(&h, "", false, String::new(), 0);
        s.list_area = Rect::new(0, 5, 40, 10);
        s.wheel(WHEEL_STEP);
        assert_eq!(s.top, 3);
        assert_eq!(s.cursor, 3, "cursor kept on screen");
        s.wheel(-100);
        assert_eq!(s.top, 0);
        s.wheel(1000);
        assert_eq!(s.top, 40, "stops at the last page");
        assert_eq!(s.click(&h, 2, 6), Outcome::None);
        assert_eq!(s.cursor, 41);
        assert_eq!(s.click(&h, 2, 6), Outcome::Take("p41".into()));
        assert_eq!(s.click(&h, 2, 30), Outcome::None, "outside the list");
    }

    #[test]
    fn ages_are_short_and_zone_free() {
        let t = parse_utc("2026-10-23 15:00:00").unwrap();
        assert_eq!(parse_utc("1970-01-01 00:00:00"), Some(0));
        assert_eq!(age("2026-10-23 15:00:00", t + 30), "now");
        assert_eq!(age("2026-10-23 15:00:00", t + 300), "5m");
        assert_eq!(age("2026-10-23 15:00:00", t + 3 * 3600), "3h");
        assert_eq!(age("2026-10-23 15:00:00", t + 2 * 86_400), "2d");
        assert_eq!(age("2026-10-23 15:00:00", t + 90 * 86_400), "2026-10-23");
        assert_eq!(age("garbage", t), "");
    }

    #[test]
    fn login_lines_never_keep_tokens() {
        assert_eq!(
            redact("/login gateway https://x.cursorvm.com SECRET"),
            "/login gateway"
        );
        assert_eq!(redact("/login cursor SECRET"), "/login cursor");
        assert_eq!(redact("  hello world  "), "hello world");
    }
}
