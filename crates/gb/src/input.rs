//! Multi-line input editing and layout. The buffer is a `String`; the cursor is a byte
//! offset kept on a char boundary. Layout char-wraps each line to the box width.

/// Most visual rows the input box grows to before it scrolls.
pub const MAX_INPUT_ROWS: usize = 10;

fn clamp(s: &str, cur: usize) -> usize {
    let mut c = cur.min(s.len());
    while !s.is_char_boundary(c) {
        c -= 1;
    }
    c
}

fn line_start(s: &str, cur: usize) -> usize {
    s[..cur].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end(s: &str, cur: usize) -> usize {
    s[cur..].find('\n').map(|i| cur + i).unwrap_or(s.len())
}

/// Byte offset of char column `col` in the line starting at `start` (clamped to line end).
fn at_col(s: &str, start: usize, col: usize) -> usize {
    let end = line_end(s, start);
    s[start..end]
        .char_indices()
        .nth(col)
        .map(|(i, _)| start + i)
        .unwrap_or(end)
}

pub fn insert(s: &mut String, cur: &mut usize, text: &str) {
    *cur = clamp(s, *cur);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    s.insert_str(*cur, &text);
    *cur += text.len();
}

pub fn backspace(s: &mut String, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some((i, _)) = s[..*cur].char_indices().next_back() {
        s.replace_range(i..*cur, "");
        *cur = i;
    }
}

pub fn delete(s: &mut String, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(ch) = s[*cur..].chars().next() {
        s.replace_range(*cur..*cur + ch.len_utf8(), "");
    }
}

pub fn left(s: &str, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some((i, _)) = s[..*cur].char_indices().next_back() {
        *cur = i;
    }
}

pub fn right(s: &str, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(ch) = s[*cur..].chars().next() {
        *cur += ch.len_utf8();
    }
}

pub fn home(s: &str, cur: &mut usize) {
    *cur = line_start(s, clamp(s, *cur));
}

pub fn end(s: &str, cur: &mut usize) {
    *cur = line_end(s, clamp(s, *cur));
}

/// Move to the previous logical line, same column. False if already on the first line.
pub fn up(s: &str, cur: &mut usize) -> bool {
    let c = clamp(s, *cur);
    let start = line_start(s, c);
    if start == 0 {
        return false;
    }
    let col = s[start..c].chars().count();
    *cur = at_col(s, line_start(s, start - 1), col);
    true
}

/// Move to the next logical line, same column. False if already on the last line.
pub fn down(s: &str, cur: &mut usize) -> bool {
    let c = clamp(s, *cur);
    let end = line_end(s, c);
    if end == s.len() {
        return false;
    }
    let col = s[line_start(s, c)..c].chars().count();
    *cur = at_col(s, end + 1, col);
    true
}

/// Visual layout of the buffer char-wrapped at `width` columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Visual rows, top to bottom.
    pub rows: Vec<String>,
    /// Cursor (row, column) in `rows`.
    pub cursor: (usize, usize),
}

/// Char-wrap each logical line to `width`. A line of length L takes `L / width + 1` rows,
/// so the cursor at the end of a full row has a row to sit on.
pub fn layout(s: &str, cur: usize, width: usize) -> Layout {
    let w = width.max(1);
    let cur = clamp(s, cur);
    let mut rows = vec![];
    let mut cursor = (0, 0);
    let mut off = 0;
    for line in s.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        let first = rows.len();
        let n = chars.len() / w + 1;
        for r in 0..n {
            let a = (r * w).min(chars.len());
            let b = ((r + 1) * w).min(chars.len());
            rows.push(chars[a..b].iter().collect());
        }
        if cur >= off && cur <= off + line.len() {
            let col = line[..cur - off].chars().count();
            cursor = (first + col / w, col % w);
        }
        off += line.len() + 1;
    }
    Layout { rows, cursor }
}

/// Rows the box shows: all rows, capped at [`MAX_INPUT_ROWS`].
pub fn visible_rows(total: usize) -> usize {
    total.clamp(1, MAX_INPUT_ROWS)
}

/// First visible row so that `cursor_row` stays on screen, moving as little as possible.
pub fn scroll_for(prev: usize, cursor_row: usize, total: usize, visible: usize) -> usize {
    let max = total.saturating_sub(visible);
    let mut s = prev.min(max);
    if cursor_row < s {
        s = cursor_row;
    } else if cursor_row >= s + visible {
        s = cursor_row + 1 - visible;
    }
    s.min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newline_insert_and_cursor_moves() {
        let (mut s, mut c) = (String::new(), 0);
        insert(&mut s, &mut c, "hello");
        insert(&mut s, &mut c, "\n");
        insert(&mut s, &mut c, "wörld");
        assert_eq!(s, "hello\nwörld");
        assert_eq!(layout(&s, c, 40).cursor, (1, 5));
        assert!(up(&s, &mut c));
        assert_eq!(c, 5);
        assert!(!up(&s, &mut c));
        home(&s, &mut c);
        assert_eq!(c, 0);
        assert!(down(&s, &mut c));
        assert_eq!(layout(&s, c, 40).cursor, (1, 0));
        end(&s, &mut c);
        assert_eq!(c, s.len());
        assert!(!down(&s, &mut c));
        left(&s, &mut c);
        backspace(&mut s, &mut c);
        assert_eq!(s, "hello\nwörd");
        home(&s, &mut c);
        backspace(&mut s, &mut c);
        assert_eq!(s, "hellowörd");
        right(&s, &mut c);
        delete(&mut s, &mut c);
        assert_eq!(s, "hellowrd", "multi-byte char deleted cleanly");
    }

    #[test]
    fn long_line_wraps_and_cursor_follows() {
        let s = "abcdefghij".repeat(3); // 30 chars
        let l = layout(&s, s.len(), 12);
        assert_eq!(l.rows, vec!["abcdefghijab", "cdefghijabcd", "efghij"]);
        assert_eq!(l.cursor, (2, 6));
        assert_eq!(layout(&s, 12, 12).cursor, (1, 0));
        // Exactly full row: cursor gets the next row.
        let l = layout("abcd", 4, 4);
        assert_eq!(l.rows, vec!["abcd", ""]);
        assert_eq!(l.cursor, (1, 0));
        // Wrapped rows count toward height, capped at 10.
        let long = "x".repeat(200);
        assert_eq!(layout(&long, 0, 10).rows.len(), 21);
        assert_eq!(visible_rows(21), MAX_INPUT_ROWS);
    }

    #[test]
    fn expansion_caps_at_ten_and_scrolls_to_cursor() {
        assert_eq!(visible_rows(0), 1);
        assert_eq!(visible_rows(3), 3);
        assert_eq!(visible_rows(10), 10);
        assert_eq!(visible_rows(15), 10);
        let s = (0..15)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let l = layout(&s, s.len(), 20);
        assert_eq!(l.rows.len(), 15);
        // Cursor on the last row: show rows 5..15.
        assert_eq!(scroll_for(0, l.cursor.0, 15, 10), 5);
        // Cursor moves to the top: scroll follows up.
        assert_eq!(scroll_for(5, 2, 15, 10), 2);
        // Cursor already visible: keep the scroll.
        assert_eq!(scroll_for(3, 8, 15, 10), 3);
        // Shrinking the buffer clamps the scroll.
        assert_eq!(scroll_for(5, 1, 4, 4), 0);
    }
}
