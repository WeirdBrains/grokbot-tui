//! Menu bar: categories that map onto slash commands.
//!
//! Adapted from acp-rs-scraper's `src/tui/widgets/menu.rs` + `MenuCommand` gist
//! (commit a60c82a). Dropdown options are existing registry commands.

use crate::commands::Registry;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// One top-level menu on the bar.
#[derive(Debug, Clone)]
pub struct Menu {
    pub label: &'static str,
    pub hint: char,
    /// Slash command names (without `/`) shown in the dropdown.
    pub items: &'static [&'static str],
}

/// Shipped menus. Items must exist in the command registry.
pub const MENUS: &[Menu] = &[
    Menu {
        label: "Chat",
        hint: 'c',
        items: &[
            "help", "copy", "history", "reload", "mouse", "keymap", "quit",
        ],
    },
    Menu {
        label: "Account",
        hint: 'a',
        items: &["login", "logout", "whoami", "account", "switch"],
    },
    Menu {
        label: "Theme",
        hint: 't',
        items: &["theme"],
    },
    Menu {
        label: "Notify",
        hint: 'n',
        items: &["notify"],
    },
    Menu {
        label: "Help",
        hint: 'h',
        items: &["help"],
    },
];

#[derive(Debug, Clone, Default)]
pub struct MenuBar {
    pub focus: Option<usize>,
    pub open: Option<usize>,
    pub opt: usize,
    pub bar_areas: Vec<Rect>,
    pub opt_areas: Vec<Rect>,
    pub dropdown_area: Rect,
}

impl MenuBar {
    pub fn open_menu(&mut self, i: usize) {
        if i >= MENUS.len() {
            return;
        }
        self.focus = Some(i);
        self.open = Some(i);
        self.opt = 0;
    }

    pub fn close(&mut self) {
        self.open = None;
        self.focus = None;
        self.opt = 0;
    }

    pub fn move_bar(&mut self, d: i32) {
        let n = MENUS.len() as i32;
        let cur = self.focus.unwrap_or(0) as i32;
        let next = (cur + d).rem_euclid(n) as usize;
        self.open_menu(next);
    }

    pub fn move_opt(&mut self, d: i32) {
        let Some(i) = self.open else {
            return;
        };
        let n = MENUS[i].items.len() as i32;
        if n == 0 {
            return;
        }
        self.opt = (self.opt as i32 + d).rem_euclid(n) as usize;
    }

    pub fn selected_command(&self) -> Option<&'static str> {
        let i = self.open?;
        MENUS.get(i)?.items.get(self.opt).copied()
    }

    pub fn hit_bar(&self, col: u16, row: u16) -> Option<usize> {
        let at = ratatui::layout::Position::new(col, row);
        self.bar_areas.iter().position(|r| r.contains(at))
    }

    pub fn hit_opt(&self, col: u16, row: u16) -> Option<usize> {
        let at = ratatui::layout::Position::new(col, row);
        self.opt_areas.iter().position(|r| r.contains(at))
    }
}

pub fn draw(f: &mut Frame, bar: &mut MenuBar, area: Rect) {
    use crate::theme;
    f.render_widget(Clear, area);
    f.render_widget(theme::block(&theme::menu_bar()), area);
    bar.bar_areas.clear();
    let mut x = area.x + 1;
    for (i, m) in MENUS.iter().enumerate() {
        let active = bar.focus == Some(i) || bar.open == Some(i);
        let st = if active {
            theme::menu_active()
        } else {
            theme::menu_bar()
        };
        let mut spans = vec![st.span(" ")];
        for ch in m.label.chars() {
            if ch.to_ascii_lowercase() == m.hint {
                spans.push(st.clone().underline(true).span(ch.to_string()));
            } else {
                spans.push(st.span(ch.to_string()));
            }
        }
        spans.push(st.span(" "));
        let w = m.label.chars().count() as u16 + 2;
        if x + w >= area.right() {
            break;
        }
        let r = Rect {
            x,
            y: area.y,
            width: w,
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(spans)), r);
        bar.bar_areas.push(r);
        x += w + 1;
    }
}

pub fn draw_dropdown(f: &mut Frame, bar: &mut MenuBar, registry: &Registry, screen: Rect) {
    use crate::theme;
    let Some(i) = bar.open else {
        bar.opt_areas.clear();
        bar.dropdown_area = Rect::default();
        return;
    };
    let menu = &MENUS[i];
    let anchor = bar.bar_areas.get(i).copied().unwrap_or(Rect {
        x: screen.x,
        y: screen.y,
        width: 1,
        height: 1,
    });
    let rows: Vec<String> = menu
        .items
        .iter()
        .map(|name| {
            let usage = registry.find(name).map(|c| c.usage).unwrap_or(name);
            format!(" /{name}  — {usage} ")
        })
        .collect();
    let w = rows
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(12)
        .min(screen.width.saturating_sub(2) as usize) as u16
        + 2;
    let h = (rows.len() as u16 + 2).min(screen.height.saturating_sub(2));
    let x = anchor.x.min(screen.x + screen.width.saturating_sub(w));
    let y = (anchor.y + 1).min(screen.y + screen.height.saturating_sub(h));
    let r = Rect {
        x,
        y,
        width: w.max(16),
        height: h.max(3),
    };
    f.render_widget(Clear, r);
    let block = theme::block(&theme::help_frame()).title(format!(" {} ", menu.label));
    let inner = block.inner(r);
    f.render_widget(block, r);
    bar.opt_areas.clear();
    for (k, text) in rows.iter().enumerate() {
        if k as u16 >= inner.height {
            break;
        }
        let rr = Rect {
            x: inner.x,
            y: inner.y + k as u16,
            width: inner.width,
            height: 1,
        };
        let st = if k == bar.opt {
            theme::selected()
        } else {
            theme::modal_text()
        };
        f.render_widget(Paragraph::new(st.span(text.clone())), rr);
        bar.opt_areas.push(rr);
    }
    bar.dropdown_area = r;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_reference_known_commands() {
        let r = crate::commands::registry();
        for m in MENUS {
            assert!(!m.items.is_empty(), "{}", m.label);
            for name in m.items {
                assert!(
                    r.find(name).is_some(),
                    "menu {} → /{name} missing from registry",
                    m.label
                );
            }
        }
    }

    #[test]
    fn keyboard_opens_and_selects() {
        let mut bar = MenuBar::default();
        bar.open_menu(0);
        assert_eq!(bar.open, Some(0));
        bar.move_opt(1);
        assert_eq!(bar.opt, 1);
        assert_eq!(bar.selected_command(), Some("copy"));
        bar.close();
        assert!(bar.open.is_none());
    }
}
