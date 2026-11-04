//! Local message search (1.39.0): `/search <words>` or Ctrl-F. Searches every cached
//! message of the active account (all bots, team bots included) plus attachment names,
//! paths and the cached text of small text files you sent. Never touches the network:
//! it only reads `context.db` (FTS5 `messages_fts` / `attachment_text_fts`).

use crate::storage::SearchHit;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// Most results kept per query.
pub const LIMIT: usize = 200;
/// Largest attachment whose text is cached for search.
pub const MAX_TEXT_BYTES: u64 = 256 * 1024;
/// How long a jumped-to message stays highlighted.
pub const FLASH: std::time::Duration = std::time::Duration::from_millis(1500);

/// Query words: split on whitespace, surrounding quotes and punctuation trimmed,
/// empties dropped. Every word must match (case-insensitive).
pub fn parse_query(q: &str) -> Vec<String> {
    q.split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == ';'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The text of a local file if it is small and looks like text (valid UTF-8, no NUL
/// bytes), for the attachment-content index.
pub fn text_of_file(path: &std::path::Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_TEXT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[derive(Debug, Clone)]
pub struct MsgSearch {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub cursor: usize,
    pub top: usize,
    /// `ui.keys = vim`: list mode (j/k/g/G, / or i edits the query, q closes).
    pub vim: bool,
    pub typing: bool,
    pub error: Option<String>,
    /// Clickable rows (each result is two rows), set by the renderer.
    pub list_area: Rect,
    /// A query is running on the worker thread.
    pub searching: bool,
    /// When the debounced query should run.
    pub due: Option<std::time::Instant>,
    /// Backfill percent while older messages are still being indexed (1.39.1).
    pub indexing: Option<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Cancel,
    /// The query changed: run it again.
    Requery,
    /// Jump to result `n`.
    Jump(usize),
    /// Open the matched attachment of result `n`.
    OpenAttachment(usize),
}

impl MsgSearch {
    pub fn new(query: &str, vim: bool) -> Self {
        Self {
            query: query.to_string(),
            hits: vec![],
            cursor: 0,
            top: 0,
            vim,
            typing: !vim || query.is_empty(),
            error: None,
            list_area: Rect::default(),
            searching: false,
            due: None,
            indexing: None,
        }
    }

    pub fn set_hits(&mut self, hits: Vec<SearchHit>) {
        self.hits = hits;
        self.cursor = self.cursor.min(self.hits.len().saturating_sub(1));
        self.top = self.top.min(self.cursor);
    }

    fn rows(&self) -> usize {
        ((self.list_area.height as usize) / 2).max(1)
    }

    pub fn move_by(&mut self, d: i64) {
        if self.hits.is_empty() {
            return;
        }
        let last = self.hits.len() as i64 - 1;
        self.cursor = (self.cursor as i64 + d).clamp(0, last) as usize;
    }

    /// Result under a screen cell, if any.
    pub fn hit_at(&self, col: u16, row: u16) -> Option<usize> {
        let a = self.list_area;
        if col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return None;
        }
        let i = self.top + ((row - a.y) / 2) as usize;
        (i < self.hits.len()).then_some(i)
    }

    pub fn key(&mut self, k: KeyEvent) -> Outcome {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.rows() as i64;
        match k.code {
            KeyCode::Char('g') if ctrl => return Outcome::Cancel,
            KeyCode::Enter if !self.hits.is_empty() => return Outcome::Jump(self.cursor),
            KeyCode::Enter => return Outcome::None,
            KeyCode::Char('o') if ctrl && !self.hits.is_empty() => {
                return Outcome::OpenAttachment(self.cursor);
            }
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::Char('p') if ctrl => self.move_by(-1),
            KeyCode::Char('n') if ctrl => self.move_by(1),
            KeyCode::PageUp => self.move_by(-page),
            KeyCode::PageDown => self.move_by(page),
            _ if self.vim && !self.typing => {
                return match k.code {
                    KeyCode::Esc | KeyCode::Char('q') => Outcome::Cancel,
                    KeyCode::Char('j') => {
                        self.move_by(1);
                        Outcome::None
                    }
                    KeyCode::Char('k') => {
                        self.move_by(-1);
                        Outcome::None
                    }
                    KeyCode::Char('d') if ctrl => {
                        self.move_by(page / 2 + 1);
                        Outcome::None
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.move_by(-(page / 2 + 1));
                        Outcome::None
                    }
                    KeyCode::Char('g') | KeyCode::Home => {
                        self.cursor = 0;
                        Outcome::None
                    }
                    KeyCode::Char('G') | KeyCode::End => {
                        self.cursor = self.hits.len().saturating_sub(1);
                        Outcome::None
                    }
                    KeyCode::Char('o') if !self.hits.is_empty() => {
                        Outcome::OpenAttachment(self.cursor)
                    }
                    KeyCode::Char('/' | 'i' | 'a') => {
                        self.typing = true;
                        Outcome::None
                    }
                    _ => Outcome::None,
                };
            }
            KeyCode::Esc if self.vim => self.typing = false,
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Backspace => {
                self.query.pop();
                self.cursor = 0;
                return Outcome::Requery;
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.cursor = 0;
                return Outcome::Requery;
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.cursor = 0;
                return Outcome::Requery;
            }
            _ => {}
        }
        Outcome::None
    }
}

/// A one-line snippet of `src` around the first match of any word, at most `width`
/// columns, newlines flattened.
pub fn snippet(src: &str, words: &[String], width: usize) -> String {
    let flat: String = src
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let lower = flat.to_lowercase();
    let at = words
        .iter()
        .filter_map(|w| lower.find(&w.to_lowercase()))
        .min()
        .unwrap_or(0);
    // Map the byte offset in `lower` to a char index (lowercasing can change lengths;
    // count chars instead).
    let ci = lower.get(..at).map(|s| s.chars().count()).unwrap_or(0);
    let start = ci.saturating_sub(width / 4);
    let mut out: String = flat.chars().skip(start).take(width.max(1)).collect();
    if start > 0 {
        out = format!("…{}", out.chars().skip(1).collect::<String>());
    }
    out
}

/// Spans for `text` with every case-insensitive occurrence of `words` highlighted.
pub fn highlight(
    text: &str,
    words: &[String],
    base: &ratatui_lipgloss::Style,
    hl: &ratatui_lipgloss::Style,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let low: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut mark = vec![false; chars.len()];
    for w in words {
        let wl: Vec<char> = w.chars().flat_map(char::to_lowercase).collect();
        if wl.is_empty() || wl.len() > low.len() {
            continue;
        }
        for i in 0..=low.len() - wl.len() {
            if low[i..i + wl.len()] == wl[..] {
                mark[i..i + wl.len()].iter_mut().for_each(|m| *m = true);
            }
        }
    }
    let mut spans = vec![];
    let mut i = 0;
    while i < chars.len() {
        let m = mark[i];
        let j = (i..chars.len())
            .find(|&j| mark[j] != m)
            .unwrap_or(chars.len());
        let s: String = chars[i..j].iter().collect();
        spans.push(if m { hl.span(s) } else { base.span(s) });
        i = j;
    }
    spans
}

pub fn draw(f: &mut Frame, s: &mut MsgSearch, area: Rect) {
    use crate::theme;
    let r = crate::history::centered(area, 80, 70);
    f.render_widget(Clear, r);
    let title = if s.vim && !s.typing {
        format!(
            " Search messages ({}) · j/k move · / edit · Enter jumps · o opens file · q closes ",
            s.hits.len()
        )
    } else {
        format!(
            " Search messages ({}) · ↑/↓ · Enter jumps · Ctrl-O opens file · Esc ",
            s.hits.len()
        )
    };
    let block = theme::block(&theme::help_frame()).title(title);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let [q, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(2)]).areas(inner);
    let caret = if s.typing { "▏" } else { "" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            theme::key_hint().span("search › "),
            theme::modal_text().span(format!("{}{caret}", s.query)),
            theme::muted().span(
                s.indexing
                    .map(|p| format!("   indexing… {p}% (older messages appear when done)"))
                    .unwrap_or_default(),
            ),
        ])),
        q,
    );
    s.list_area = list;
    let words = parse_query(&s.query);
    let h = s.rows();
    if s.cursor < s.top {
        s.top = s.cursor;
    } else if s.cursor >= s.top + h {
        s.top = s.cursor + 1 - h;
    }
    let mut lines: Vec<Line> = vec![];
    if let Some(e) = &s.error {
        lines.push(Line::from(theme::muted().span(e.clone())));
    } else if words.is_empty() {
        lines.push(Line::from(theme::muted().span(
            "Type words to search every cached message, attachment name and sent text file (local only).",
        )));
    } else if s.hits.is_empty() && (s.searching || s.due.is_some()) {
        lines.push(Line::from(theme::muted().span("Searching…")));
    } else if s.hits.is_empty() {
        lines.push(Line::from(theme::muted().span("No matches.")));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let w = list.width as usize;
    for i in s.top..(s.top + h).min(s.hits.len()) {
        let hit = &s.hits[i];
        let sel = i == s.cursor;
        let st = if sel {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let who = if hit.role == crate::api::Role::User {
            "you → "
        } else {
            ""
        };
        let mut head = vec![
            if sel { st.clone() } else { theme::title() }.span(format!("{}{}", who, hit.bot_name)),
            if sel { st.clone() } else { theme::muted() }.span(format!(
                "  {}  {}",
                crate::history::age(&hit.at, now),
                hit.at.get(..16).unwrap_or(&hit.at)
            )),
        ];
        if let Some((name, _)) = &hit.attachment {
            let tag = if hit.in_content { "in" } else { "file" };
            head.push(theme::image_placeholder().span(format!("  [📎 {tag} {name}]")));
        }
        lines.push(Line::from(head));
        let text = snippet(&hit.snippet_src, &words, w.saturating_sub(2));
        let mut body = vec![st.span("  ")];
        body.extend(highlight(&text, &words, &st, &theme::md_link()));
        lines.push(Line::from(body));
    }
    f.render_widget(Paragraph::new(lines), list);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_words_and_snippets() {
        assert_eq!(
            parse_query("  Foo \"bar\" baz, "),
            vec!["Foo", "bar", "baz"]
        );
        assert!(parse_query("   ").is_empty());
        let s = snippet("aaaa\nbbbb needle cccc", &["NEEDLE".into()], 10);
        assert!(s.to_lowercase().contains("needle"), "{s}");
        assert!(!s.contains('\n'));
        let sp = highlight(
            "a Needle b",
            &["needle".into()],
            &crate::theme::modal_text(),
            &crate::theme::md_link(),
        );
        let texts: Vec<_> = sp.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(texts, vec!["a ", "Needle", " b"]);
    }

    #[test]
    fn keys_and_clicks() {
        let mut s = MsgSearch::new("", false);
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert_eq!(s.key(k(KeyCode::Char('x'))), Outcome::Requery);
        assert_eq!(s.query, "x");
        assert_eq!(s.key(k(KeyCode::Enter)), Outcome::None, "no hits");
        assert_eq!(s.key(k(KeyCode::Esc)), Outcome::Cancel);
        let mut v = MsgSearch::new("x", true);
        assert!(!v.typing);
        assert_eq!(v.key(k(KeyCode::Char('q'))), Outcome::Cancel);
        assert_eq!(v.key(k(KeyCode::Char('/'))), Outcome::None);
        assert!(v.typing);
        v.list_area = Rect::new(10, 5, 40, 6);
        assert_eq!(v.hit_at(12, 7), None, "no results yet");
    }
}
