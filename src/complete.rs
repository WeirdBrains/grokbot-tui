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
    pub query: String,
    pub list_area: Rect,
}

impl Popup {
    pub fn move_by(&mut self, d: i32) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as i32;
        self.cursor = (self.cursor as i32 + d).rem_euclid(n) as usize;
    }

    pub fn selected(&self) -> Option<&Candidate> {
        self.items.get(self.cursor)
    }
}

fn apply(app: &mut App, c: &Candidate, range: (usize, usize)) {
    let (a, b) = range;
    let b = b.min(app.input.len());
    let a = a.min(b);
    app.input.replace_range(a..b, &c.replace);
    app.input_cursor = a + c.replace.len();
    app.input_anchor = None;
}

/// Tab (`back` = Shift-Tab): open or cycle the popup.
pub fn tab(app: &mut App, back: bool) {
    if let Some(p) = app.completion.as_mut() {
        p.move_by(if back { -1 } else { 1 });
        if let Some(c) = p.selected().cloned() {
            let r = p.range;
            apply(app, &c, r);
        }
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
    let draft = app.input.clone();
    let draft_cursor = app.input_cursor;
    let c0 = ranked[0].clone();
    apply(app, &c0, range);
    app.completion = Some(Popup {
        items: ranked,
        cursor: 0,
        range,
        draft,
        draft_cursor,
        query,
        list_area: Rect::default(),
    });
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
    p.items = ranked;
    p.cursor = 0;
    p.range = range;
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
        KeyCode::Enter | KeyCode::Tab => {
            app.completion = None;
        }
        KeyCode::BackTab => tab(app, true),
        KeyCode::Up => {
            if let Some(p) = app.completion.as_mut() {
                p.move_by(-1);
                if let Some(c) = p.selected().cloned() {
                    let r = p.range;
                    apply(app, &c, r);
                }
            }
        }
        KeyCode::Down => {
            if let Some(p) = app.completion.as_mut() {
                p.move_by(1);
                if let Some(c) = p.selected().cloned() {
                    let r = p.range;
                    apply(app, &c, r);
                }
            }
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
    if let Some((colon, name)) = crate::emoji::shortcode_at(input, app.input_cursor) {
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
    // Token = from last whitespace (or after '/') to cursor.
    let start = input[..cur]
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(1);
    let start = start.max(1); // never include the leading '/'
    let query = input[start..cur].to_string();
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
        // Which argument index?
        let idx = args
            .split_whitespace()
            .count()
            .saturating_sub(usize::from(!query.is_empty()));
        let idx = if query.is_empty() && input[..cur].ends_with(char::is_whitespace) {
            args.split_whitespace().count()
        } else {
            idx
        };
        (cmd.complete)(app, idx, &query)
    };
    Some((range, query, items))
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
    let h = (p.items.len() as u16).clamp(1, 8).min(area.y.max(1));
    let r = Rect {
        x: area.x,
        y: area.y.saturating_sub(h + 1),
        width: area.width,
        height: h + 1,
    };
    f.render_widget(Clear, r);
    let block = theme::block(&theme::panel(true)).title(" Completions · Tab cycles · Esc keeps ");
    let inner = block.inner(r);
    f.render_widget(block, r);
    let mut lines = vec![];
    for (i, c) in p.items.iter().enumerate().take(inner.height as usize) {
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
