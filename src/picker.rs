//! Reusable selection modal: a centered, titled list the user picks from with the
//! keyboard (↑/↓, j/k, Home/End, PgUp/PgDn, Enter, Esc) or the mouse (hover to
//! highlight, click to pick, click outside to cancel).
//!
//! Any flow opens one with [`crate::app::App::open_picker`], passing the items and an
//! `on_pick` callback that receives the chosen [`PickItem`].
use crate::app::App;
use ratatui::layout::{Position, Rect};

/// One selectable row. `value` is what the callback gets back (an id, alias, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickItem {
    pub label: String,
    pub value: String,
    /// Optional dim text after the label (e.g. "current").
    pub detail: Option<String>,
}

impl PickItem {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            detail: None,
        }
    }
    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}

/// Called with the chosen item when the user confirms.
pub type OnPick = fn(&mut App, PickItem);

pub struct Picker {
    pub title: String,
    pub items: Vec<PickItem>,
    pub cursor: usize,
    /// First visible row (scrolls to keep the cursor in view).
    pub offset: usize,
    pub on_pick: OnPick,
    /// Optional: called when the user cancels (Esc / click outside).
    pub on_cancel: Option<fn(&mut App)>,
    /// Optional: called after the highlight moves (keys, wheel, hover).
    pub on_move: Option<fn(&mut App)>,
    /// Where the list rows were last drawn (set by the renderer; used for mouse hits).
    pub list_area: Rect,
    /// The whole popup, for "click outside cancels".
    pub popup_area: Rect,
}

/// What a mouse event on the picker should do.
#[derive(Debug, PartialEq, Eq)]
pub enum MouseOutcome {
    None,
    Confirm,
    Cancel,
}

impl Picker {
    pub fn new(title: impl Into<String>, items: Vec<PickItem>, on_pick: OnPick) -> Self {
        Self {
            title: title.into(),
            items,
            cursor: 0,
            offset: 0,
            on_pick,
            on_cancel: None,
            on_move: None,
            list_area: Rect::default(),
            popup_area: Rect::default(),
        }
    }

    /// Run `f` if the user cancels instead of picking.
    pub fn on_cancel(mut self, f: fn(&mut App)) -> Self {
        self.on_cancel = Some(f);
        self
    }

    /// Run `f` whenever the highlight moves.
    pub fn on_move(mut self, f: fn(&mut App)) -> Self {
        self.on_move = Some(f);
        self
    }

    /// Start with the cursor on the item whose value matches (e.g. the current one).
    pub fn with_selected(mut self, value: &str) -> Self {
        if let Some(i) = self.items.iter().position(|it| it.value == value) {
            self.cursor = i;
        }
        self
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as i32 - 1;
        self.cursor = (self.cursor as i32 + delta).clamp(0, last) as usize;
    }

    pub fn first(&mut self) {
        self.cursor = 0;
    }

    pub fn last(&mut self) {
        self.cursor = self.items.len().saturating_sub(1);
    }

    pub fn selected(&self) -> Option<&PickItem> {
        self.items.get(self.cursor)
    }

    /// Keep the cursor visible in a list `height` rows tall.
    pub fn scroll_into_view(&mut self, height: usize) {
        let h = height.max(1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + h {
            self.offset = self.cursor + 1 - h;
        }
        self.offset = self.offset.min(self.items.len().saturating_sub(h));
    }

    /// Item index under a screen cell, if any.
    pub fn item_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self.list_area.contains(Position::new(col, row)) {
            return None;
        }
        let i = self.offset + (row - self.list_area.y) as usize;
        (i < self.items.len()).then_some(i)
    }

    /// Hover: highlight the item under the pointer.
    pub fn hover(&mut self, col: u16, row: u16) {
        if let Some(i) = self.item_at(col, row) {
            self.cursor = i;
        }
    }

    /// Click: pick the item under the pointer, or cancel when clicking outside.
    pub fn click(&mut self, col: u16, row: u16) -> MouseOutcome {
        if let Some(i) = self.item_at(col, row) {
            self.cursor = i;
            MouseOutcome::Confirm
        } else if !self.popup_area.contains(Position::new(col, row)) {
            MouseOutcome::Cancel
        } else {
            MouseOutcome::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &mut App, _: PickItem) {}

    fn picker(n: usize) -> Picker {
        Picker::new(
            "T",
            (0..n)
                .map(|i| PickItem::new(format!("item {i}"), i.to_string()))
                .collect(),
            noop,
        )
    }

    #[test]
    fn keyboard_navigation_clamps_and_scrolls() {
        let mut p = picker(10);
        p.move_by(-1);
        assert_eq!(p.cursor, 0);
        p.move_by(3);
        assert_eq!(p.selected().unwrap().value, "3");
        p.last();
        assert_eq!(p.cursor, 9);
        p.move_by(5);
        assert_eq!(p.cursor, 9);
        p.scroll_into_view(4);
        assert_eq!(p.offset, 6, "cursor kept in the 4-row window");
        p.first();
        p.scroll_into_view(4);
        assert_eq!(p.offset, 0);
        assert_eq!(picker(5).with_selected("2").cursor, 2);
        assert_eq!(picker(5).with_selected("zz").cursor, 0);
        let mut empty = picker(0);
        empty.move_by(1);
        assert!(empty.selected().is_none());
    }

    #[test]
    fn mouse_hover_and_click() {
        let mut p = picker(10);
        p.popup_area = Rect::new(10, 5, 30, 10);
        p.list_area = Rect::new(11, 6, 28, 4);
        p.offset = 2;
        p.hover(15, 7); // second visible row → item 3
        assert_eq!(p.cursor, 3);
        p.hover(15, 20); // outside: unchanged
        assert_eq!(p.cursor, 3);
        assert_eq!(p.click(15, 9), MouseOutcome::Confirm);
        assert_eq!(p.selected().unwrap().value, "5");
        assert_eq!(
            p.click(12, 13),
            MouseOutcome::None,
            "inside popup, not on a row"
        );
        assert_eq!(p.click(1, 1), MouseOutcome::Cancel);
        // Rows past the end of the list are not items.
        let mut short = picker(2);
        short.popup_area = Rect::new(0, 0, 20, 10);
        short.list_area = Rect::new(1, 1, 18, 6);
        assert_eq!(short.item_at(2, 4), None);
    }
}
