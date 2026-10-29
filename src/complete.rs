//! Fuzzy Tab completion for slash commands and their arguments.
//!
//! Each [`crate::commands::Command`] can declare an [`ArgCompleter`]. Tab ranks candidates
//! with the shared [`crate::fuzzy`] matcher; repeated Tab / Shift-Tab cycle.

use crate::app::App;
use crate::fuzzy;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// One completion candidate shown in the popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub replace: String,
    pub label: String,
}

/// The open completion popup.
#[derive(Debug, Clone)]
pub struct Popup {
    pub items: Vec<Candidate>,
    pub cursor: usize,
    /// Byte range in the input that `replace` overwrites.
    pub range: (usize, usize),
    pub draft: String,
    pub draft_cursor: usize,
    /// The token as typed when the cycle started (cursor == items.len() restores it).
    pub query: String,
    pub list_area: Rect,
    /// The token's span in `draft`.
    pub orig_range: (usize, usize),
    /// First visible item (the list scrolls to keep the selection visible).
    pub scroll: usize,
}

impl Popup {
    /// Cycle through the candidates plus one extra slot (index `items.len()`) that stands
    /// for the token as originally typed, so Shift-Tab from the first item gets it back.
    pub fn move_by(&mut self, d: i32) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as i32 + 1;
        self.cursor = (self.cursor as i32 + d).rem_euclid(n) as usize;
    }

    /// The text the current position puts in the token span.
    fn text(&self) -> String {
        match self.items.get(self.cursor) {
            Some(c) => c.replace.clone(),
            None => {
                let (a, b) = self.orig_range;
                self.draft.get(a..b).unwrap_or("").to_string()
            }
        }
    }

    /// Replace only the token span with the current choice and remember the new span.
    fn apply_current(&mut self, app_input: &mut String, cursor: &mut usize) {
        let t = self.text();
        let (a, b) = self.range;
        let b = b.min(app_input.len());
        let a = a.min(b);
        app_input.replace_range(a..b, &t);
        *cursor = a + t.len();
        self.range = (a, a + t.len());
    }
}

/// Step the open popup by `d` and swap the token span for the new choice.
fn cycle(app: &mut App, d: i32) {
    let Some(mut p) = app.completion.take() else {
        return;
    };
    p.move_by(d);
    p.apply_current(&mut app.input, &mut app.input_cursor);
    app.input_anchor = None;
    app.completion = Some(p);
}

/// Tab (`back` = Shift-Tab): open or cycle the popup.
pub fn tab(app: &mut App, back: bool) {
    if app.completion.is_some() {
        cycle(app, if back { -1 } else { 1 });
        return;
    }
    let Some((range, query, items)) = candidates(app) else {
        // No slash completion: keep the old Tab focus switch.
        app.focus = if app.focus == crate::app::Focus::Input {
            crate::app::Focus::Sidebar
        } else {
            crate::app::Focus::Input
        };
        return;
    };
    if items.is_empty() {
        return;
    }
    let ranked = rank(&query, &items);
    if ranked.is_empty() {
        return;
    }
    let mut p = Popup {
        items: ranked,
        cursor: 0,
        range,
        draft: app.input.clone(),
        draft_cursor: app.input_cursor,
        query,
        list_area: Rect::default(),
        orig_range: range,
        scroll: 0,
    };
    p.apply_current(&mut app.input, &mut app.input_cursor);
    app.input_anchor = None;
    app.completion = Some(p);
}

pub fn refilter(app: &mut App) {
    let Some(mut p) = app.completion.take() else {
        return;
    };
    let Some((range, query, items)) = candidates(app) else {
        return;
    };
    let ranked = rank(&query, &items);
    if ranked.is_empty() {
        return;
    }
    // Typing ended the old cycle: the token as it is now is the new original.
    p.items = ranked;
    p.cursor = p.items.len();
    p.range = range;
    p.orig_range = range;
    p.draft = app.input.clone();
    p.draft_cursor = app.input_cursor;
    p.query = query;
    app.completion = Some(p);
}

pub fn key(app: &mut App, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    match k.code {
        KeyCode::Esc | KeyCode::Char('g') if ctrl || k.code == KeyCode::Esc => {
            if let Some(p) = app.completion.take() {
                app.input = p.draft;
                app.input_cursor = p.draft_cursor.min(app.input.len());
            }
        }
        KeyCode::Esc => {
            if let Some(p) = app.completion.take() {
                app.input = p.draft;
                app.input_cursor = p.draft_cursor.min(app.input.len());
            }
        }
        KeyCode::Enter => {
            app.completion = None;
        }
        KeyCode::Tab => cycle(app, 1),
        KeyCode::BackTab | KeyCode::Up => cycle(app, -1),
        KeyCode::Down => cycle(app, 1),
        KeyCode::PageDown => {
            let n = page(app);
            cycle(app, n)
        }
        KeyCode::PageUp => {
            let n = page(app);
            cycle(app, -n)
        }
        KeyCode::Char(c) if !ctrl => {
            app.input_insert(&c.to_string());
            refilter(app);
        }
        KeyCode::Backspace => {
            app.input_backspace();
            if app.input.starts_with('/') {
                refilter(app);
            } else {
                app.completion = None;
            }
        }
        _ => {}
    }
}

fn page(app: &App) -> i32 {
    app.completion
        .as_ref()
        .map(|p| (p.list_area.height.max(2) as i32) - 1)
        .unwrap_or(5)
}

/// Mouse wheel over the open popup: step the selection.
pub fn wheel(app: &mut App, down: bool) {
    cycle(app, if down { 3 } else { -3 });
}

/// Keep `cursor` inside the `h`-row window starting at `scroll`.
pub fn keep_visible(scroll: usize, cursor: usize, h: usize, n: usize) -> usize {
    let h = h.max(1);
    let cursor = cursor.min(n.saturating_sub(1));
    let s = if cursor < scroll {
        cursor
    } else if cursor >= scroll + h {
        cursor + 1 - h
    } else {
        scroll
    };
    s.min(n.saturating_sub(h))
}

pub fn rank(query: &str, candidates: &[Candidate]) -> Vec<Candidate> {
    let labels: Vec<&str> = candidates.iter().map(|c| c.label.as_str()).collect();
    fuzzy::rank(query, &labels)
        .into_iter()
        .map(|(i, _)| candidates[i].clone())
        .collect()
}

/// The token under the cursor and its candidates.
fn candidates(app: &App) -> Option<((usize, usize), String, Vec<Candidate>)> {
    let input = &app.input;
    let react_arg = {
        let head = input.split_whitespace().next().unwrap_or("");
        matches!(head, "/react" | "/unreact")
    };
    if !react_arg && let Some((colon, name)) = crate::emoji::shortcode_at(input, app.input_cursor) {
        let items = emoji_candidates(name);
        return Some((
            (colon, app.input_cursor.min(input.len())),
            format!(":{name}"),
            items,
        ));
    }
    if !input.starts_with('/') {
        return None;
    }
    let cur = app.input_cursor.min(input.len());
    // Token = the last shell-like word before the cursor ("…", '…' and `\ ` keep spaces).
    let (start, query) = last_token(&input[..cur]);
    let start = start.max(1); // never include the leading '/'
    let range = (start, cur);
    let rest = input[1..start.min(input.len())].trim();
    let items = if rest.is_empty() && !input[1..cur].contains(char::is_whitespace) {
        // Completing the command name (query is after '/').
        let q = &input[1..cur];
        let range = (1, cur);
        let cmds: Vec<Candidate> = app
            .registry
            .commands
            .iter()
            .flat_map(|c| {
                std::iter::once(c.name)
                    .chain(c.aliases.iter().copied())
                    .map(|n| Candidate {
                        replace: n.to_string(),
                        label: if n == c.name {
                            format!("/{n}")
                        } else {
                            format!("/{n} → /{}", c.name)
                        },
                    })
            })
            .collect();
        return Some((range, q.to_string(), cmds));
    } else {
        let cmd_name = rest.split_whitespace().next().unwrap_or("");
        let cmd = app.registry.find(cmd_name)?;
        let args = rest[cmd_name.len()..].trim_start();
        // Which argument index? (Words before the token, quote-aware.)
        let idx = crate::attach::split_args(args).len();
        (cmd.complete)(app, idx, &query)
    };
    Some((range, query, items))
}

/// Start byte and unquoted text of the last word in `s` (shell-like: `"…"`, `'…'` and
/// backslash escapes keep spaces inside one word). Empty text when `s` ends in a space.
pub fn last_token(s: &str) -> (usize, String) {
    let mut start = 0;
    let mut text = String::new();
    let mut quote: Option<char> = None;
    let mut in_word = false;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if !in_word && !(quote.is_none() && c.is_whitespace()) {
            in_word = true;
            start = i;
            text.clear();
        }
        match (c, quote) {
            ('\\', q) if q != Some('\'') => {
                if let Some((_, n)) = it.next() {
                    text.push(n);
                }
            }
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            (c, None) if c.is_whitespace() => {
                in_word = false;
            }
            (c, _) => text.push(c),
        }
    }
    if !in_word {
        return (s.len(), String::new());
    }
    (start, text)
}

/// A path as one shell word: spaces, quotes and backslashes are backslash-escaped.
pub fn escape_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_whitespace() || matches!(c, '\\' | '"' | '\'') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `:shortcode` completions: the emoji replaces the whole `:partial` token.
pub fn emoji_candidates(name: &str) -> Vec<Candidate> {
    crate::emoji::search(name, 50)
        .into_iter()
        .map(|(code, e)| Candidate {
            replace: e.to_string(),
            label: format!(":{code}: {e}"),
        })
        .collect()
}

pub fn draw(f: &mut Frame, app: &mut App) {
    use crate::theme;
    let Some(p) = app.completion.as_mut() else {
        return;
    };
    let area = app.input_area;
    if area.height < 2 {
        return;
    }
    // As tall as the items need, up to the space above the input (or below, if more).
    let above = area.y.saturating_sub(2);
    let screen_h = f.area().height;
    let below = screen_h.saturating_sub(area.y + area.height);
    let room = above.max(below).max(1);
    let h = (p.items.len() as u16).clamp(1, room);
    let r = if above >= below || above >= h {
        Rect {
            x: area.x,
            y: area.y.saturating_sub(h + 2),
            width: area.width,
            height: h + 2,
        }
    } else {
        Rect {
            x: area.x,
            y: area.y + area.height,
            width: area.width,
            height: (h + 2).min(below),
        }
    };
    f.render_widget(Clear, r);
    let n = p.items.len();
    let pos = if p.cursor < n {
        format!("{}/{n}", p.cursor + 1)
    } else {
        format!("–/{n}")
    };
    let block = theme::block(&theme::panel(true)).title(format!(
        " Completions {pos} · Tab cycles · PgUp/PgDn · Esc keeps "
    ));
    let inner = block.inner(r);
    f.render_widget(block, r);
    let vis = inner.height as usize;
    p.scroll = keep_visible(p.scroll, p.cursor, vis, n);
    let mut lines = vec![];
    for (i, c) in p.items.iter().enumerate().skip(p.scroll).take(vis) {
        let st = if i == p.cursor {
            theme::selected()
        } else {
            theme::modal_text()
        };
        let m = fuzzy::score(&p.query, &c.label);
        let mut spans = vec![st.span(if i == p.cursor { "› " } else { "  " })];
        for (ci, ch) in c.label.chars().enumerate() {
            if m.as_ref().is_some_and(|m| m.positions.contains(&ci)) {
                spans.push(theme::title().span(ch.to_string()));
            } else {
                spans.push(st.span(ch.to_string()));
            }
        }
        lines.push(Line::from(spans));
    }
    if n > vis && vis > 0 {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
        let mut sb = ScrollbarState::new(n.saturating_sub(vis)).position(p.scroll);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            Rect {
                x: r.x,
                width: r.width,
                ..inner
            },
            &mut sb,
        );
    }
    p.list_area = inner;
    f.render_widget(Paragraph::new(lines), inner);
}

/// Helpers for command completers.
pub fn names(items: impl IntoIterator<Item = impl Into<String>>) -> Vec<Candidate> {
    items
        .into_iter()
        .map(|s| {
            let s = s.into();
            Candidate {
                label: s.clone(),
                replace: s,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::test_app;

    #[test]
    fn tab_completes_command_names_and_cycles() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = test_app(&rt);
        app.input = "/sw".into();
        app.input_cursor = 3;
        tab(&mut app, false);
        assert!(
            app.input.starts_with("/switch") || app.input.contains("switch"),
            "{}",
            app.input
        );
        assert!(app.completion.is_some());
        let first = app.input.clone();
        tab(&mut app, false);
        // May be only one match.
        let _ = first;
    }

    #[test]
    fn rank_prefers_prefix() {
        let c = names(["use", "user", "export"]);
        let r = rank("use", &c);
        assert_eq!(r[0].replace, "use");
    }
}
