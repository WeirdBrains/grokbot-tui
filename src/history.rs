//! Prompt history: Up/Down in the input and the Ctrl-R fuzzy search overlay.

use crate::fuzzy;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// How many prompts to keep per account.
pub const MAX: usize = 500;

/// Ctrl-R fuzzy history search.
#[derive(Debug, Clone)]
pub struct Search {
    /// Query typed into the search field.
    pub query: String,
    /// Indices into the app's history (newest-first) of the current matches.
    pub matches: Vec<(usize, fuzzy::Match)>,
    pub cursor: usize,
    /// The input to restore on Esc.
    pub draft: String,
    pub draft_cursor: usize,
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
            draft,
            draft_cursor,
            list_area: Rect::default(),
            list_rows: vec![],
        };
        s.refilter(history);
        s
    }

    pub fn refilter(&mut self, history: &[String]) {
        self.matches = fuzzy::rank(&self.query, history);
        if self.cursor >= self.matches.len() {
            self.cursor = self.matches.len().saturating_sub(1);
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.matches.get(self.cursor).map(|(i, _)| *i)
    }

    pub fn move_by(&mut self, d: i32) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as i32;
        self.cursor = (self.cursor as i32 + d).rem_euclid(n) as usize;
    }
}

/// A key while the search overlay is open. `Take(text)` replaces the input; `Cancel`
/// restores the draft; `None` stays open.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Take(String),
    Cancel,
}

pub fn key(s: &mut Search, history: &[String], k: KeyEvent) -> Outcome {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc | KeyCode::Char('g') if ctrl || k.code == KeyCode::Esc => Outcome::Cancel,
        KeyCode::Char('g') if ctrl => Outcome::Cancel,
        KeyCode::Enter => s
            .selected()
            .map(|i| Outcome::Take(history[i].clone()))
            .unwrap_or(Outcome::None),
        KeyCode::Up | KeyCode::Char('r') if matches!(k.code, KeyCode::Up) || ctrl => {
            s.move_by(-1);
            Outcome::None
        }
        KeyCode::Char('r') if ctrl => {
            s.move_by(-1);
            Outcome::None
        }
        KeyCode::Down | KeyCode::Char('s') if matches!(k.code, KeyCode::Down) || ctrl => {
            s.move_by(1);
            Outcome::None
        }
        KeyCode::Char('s') if ctrl => {
            s.move_by(1);
            Outcome::None
        }
        KeyCode::Backspace => {
            s.query.pop();
            s.refilter(history);
            Outcome::None
        }
        KeyCode::Char(c) if !ctrl => {
            s.query.push(c);
            s.refilter(history);
            Outcome::None
        }
        _ => Outcome::None,
    }
}

pub fn draw(f: &mut Frame, s: &mut Search, history: &[String], area: Rect) {
    use crate::theme;
    let r = centered(area, 70, 50);
    f.render_widget(Clear, r);
    let block =
        theme::block(&theme::help_frame()).title(" History · Ctrl-R · Enter uses · Esc keeps ");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let [q, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(inner);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            theme::key_hint().span("› "),
            theme::modal_text().span(format!("{}▏", s.query)),
        ])),
        q,
    );
    let h = list.height as usize;
    let start = s.cursor.saturating_sub(h.saturating_sub(1) / 2);
    let shown: Vec<(usize, &fuzzy::Match)> = s
        .matches
        .iter()
        .enumerate()
        .skip(start)
        .take(h)
        .filter_map(|(row, (i, m))| history.get(*i).map(|_| (row, m)))
        .collect();
    // Rebuild with texts
    let mut lines = vec![];
    s.list_rows.clear();
    for (row, _) in s.matches.iter().enumerate().skip(start).take(h) {
        let (i, m) = &s.matches[row];
        let text = &history[*i];
        let sel = row == s.cursor;
        let st = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let mut spans = vec![];
        for (ci, ch) in text.chars().enumerate() {
            if m.positions.contains(&ci) {
                spans.push(theme::title().span(ch.to_string()));
            } else {
                spans.push(st.span(ch.to_string()));
            }
        }
        if spans.is_empty() {
            spans.push(st.span(" "));
        }
        lines.push(Line::from(spans));
        s.list_rows.push(*i);
    }
    s.list_area = list;
    f.render_widget(Paragraph::new(lines), list);
    let _ = shown;
}

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = (area.width as u32 * pct_w as u32 / 100) as u16;
    let h = (area.height as u32 * pct_h as u32 / 100) as u16;
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w.max(20),
        height: h.max(6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            key(
                &mut s,
                &h,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Outcome::Take("git push origin main".into())
        );
        s.cursor = 1;
        assert_eq!(
            key(
                &mut s,
                &h,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            ),
            Outcome::Take("git push --force".into())
        );
        assert_eq!(
            key(&mut s, &h, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Outcome::Cancel
        );
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
}
