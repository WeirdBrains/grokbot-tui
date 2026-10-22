//! In-app text selection in the chat body.

use ratatui::layout::Rect;

/// A selection in the laid-out chat feed (byte offsets into the joined plain text).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: usize,
    pub cursor: usize,
}

impl Selection {
    pub fn range(&self) -> (usize, usize) {
        (self.anchor.min(self.cursor), self.anchor.max(self.cursor))
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.cursor
    }
}

/// Plain text of the feed as shown (unwrapped logical lines, blank line between entries).
pub fn plain_text(feed: &[crate::app::Entry]) -> String {
    let mut out = String::new();
    for (i, e) in feed.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&e.text);
        if !e.text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Byte offset in [`plain_text`] for a click at (col, row) inside the body, given the
/// visible lines currently drawn and the scroll offset into the plain text's visual rows.
pub fn offset_at(
    plain: &str,
    body: Rect,
    col: u16,
    row: u16,
    wrap: usize,
    row_off: usize,
) -> Option<usize> {
    if !body.contains(ratatui::layout::Position::new(col, row)) {
        return None;
    }
    let wrap = wrap.max(1);
    let visual_row = row_off + (row - body.y) as usize;
    let visual_col = (col - body.x) as usize;
    // Walk plain text, wrapping at `wrap` and on newlines.
    let mut vrow = 0usize;
    let mut byte = 0usize;
    for line in plain.split_inclusive('\n') {
        let content = line.trim_end_matches('\n');
        let chars: Vec<(usize, char)> = content.char_indices().collect();
        let n = chars.len();
        let rows = (n / wrap) + 1;
        for r in 0..rows {
            if vrow == visual_row {
                let start = r * wrap;
                let idx = (start + visual_col).min(n);
                return Some(byte + chars.get(idx).map(|(i, _)| *i).unwrap_or(content.len()));
            }
            vrow += 1;
        }
        byte += line.len();
    }
    Some(plain.len())
}

/// Slice of `plain` for the selection (logical text, no gutter).
pub fn selected_text(plain: &str, sel: &Selection) -> String {
    let (a, b) = sel.range();
    plain.get(a..b.min(plain.len())).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Entry, EntryKind};

    #[test]
    fn plain_text_joins_entries_and_preserves_code() {
        let feed = [
            Entry { kind: EntryKind::User, text: "hi".into() },
            Entry {
                kind: EntryKind::Bot,
                text: "```rust\nfn x() {}\n```".into(),
            },
        ];
        let p = plain_text(&feed);
        assert_eq!(p, "hi\n\n```rust\nfn x() {}\n```\n");
        let mut sel = Selection { anchor: 4, cursor: p.len() - 1 };
        let t = selected_text(&p, &sel);
        assert!(t.starts_with("```rust"));
        assert!(t.contains("fn x() {}"));
        // Swap anchor/cursor.
        sel = Selection { anchor: p.len(), cursor: 0 };
        assert_eq!(selected_text(&p, &sel), p);
    }

    #[test]
    fn offset_at_maps_wrapped_rows() {
        let plain = "abcdefghij\nxyz\n";
        let body = Rect::new(0, 0, 20, 10);
        // wrap=4: "abcd" "efgh" "ij" then "xyz"
        assert_eq!(offset_at(plain, body, 2, 0, 4, 0), Some(2)); // 'c'
        assert_eq!(offset_at(plain, body, 1, 1, 4, 0), Some(5)); // 'f'
        assert_eq!(offset_at(plain, body, 0, 3, 4, 0), Some(11)); // 'x'
    }
}
