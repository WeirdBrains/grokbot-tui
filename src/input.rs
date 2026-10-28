//! Multi-line input editing and layout. The buffer is a `String`; the cursor is a byte
//! offset kept on a grapheme boundary. Layout wraps each line by display width.

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

/// Display width of one grapheme cluster (emoji ZWJ sequences, flags, VS16 → 2). Never 0
/// so the cursor always advances.
pub fn grapheme_width(g: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    g.width().max(1)
}

/// Display width of a line, grapheme by grapheme.
pub fn str_width(s: &str) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    s.graphemes(true).map(grapheme_width).sum()
}

/// Byte offset at display column `col` in the line starting at `start` (clamped to line
/// end; a wide grapheme straddling `col` snaps to its start).
fn at_col(s: &str, start: usize, col: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let end = line_end(s, start);
    let mut w = 0;
    for (i, g) in s[start..end].grapheme_indices(true) {
        let gw = grapheme_width(g);
        if w + gw > col {
            return start + i;
        }
        w += gw;
    }
    end
}

/// Byte offset of the grapheme boundary before `cur`.
fn prev_boundary(s: &str, cur: usize) -> Option<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    s[..cur].grapheme_indices(true).next_back().map(|(i, _)| i)
}

/// Byte length of the grapheme starting at `cur`.
fn next_len(s: &str, cur: usize) -> Option<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    s[cur..].graphemes(true).next().map(str::len)
}

pub fn insert(s: &mut String, cur: &mut usize, text: &str) {
    *cur = clamp(s, *cur);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    s.insert_str(*cur, &text);
    *cur += text.len();
}

/// Deletes the whole grapheme before the cursor (an emoji ZWJ family is one keypress).
pub fn backspace(s: &mut String, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(i) = prev_boundary(s, *cur) {
        s.replace_range(i..*cur, "");
        *cur = i;
    }
}

pub fn delete(s: &mut String, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(n) = next_len(s, *cur) {
        s.replace_range(*cur..*cur + n, "");
    }
}

pub fn left(s: &str, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(i) = prev_boundary(s, *cur) {
        *cur = i;
    }
}

pub fn right(s: &str, cur: &mut usize) {
    *cur = clamp(s, *cur);
    if let Some(n) = next_len(s, *cur) {
        *cur += n;
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
    let col = str_width(&s[start..c]);
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
    let col = str_width(&s[line_start(s, c)..c]);
    *cur = at_col(s, end + 1, col);
    true
}

/// Visual layout of the buffer wrapped at `width` columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Visual rows, top to bottom.
    pub rows: Vec<String>,
    /// Cursor (row, display column) in `rows`.
    pub cursor: (usize, usize),
}

/// One logical line split into rows of at most `w` display columns, never splitting a
/// grapheme. Returns (byte start, byte end) per row. A row that is exactly full gets an
/// empty row after it so the cursor at its end has somewhere to sit.
fn wrap_line(line: &str, w: usize) -> Vec<(usize, usize)> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = vec![];
    let (mut a, mut cw) = (0, 0);
    for (i, g) in line.grapheme_indices(true) {
        let gw = grapheme_width(g);
        if cw + gw > w && cw > 0 {
            out.push((a, i));
            a = i;
            cw = 0;
        }
        cw += gw;
    }
    out.push((a, line.len()));
    if cw >= w {
        out.push((line.len(), line.len()));
    }
    out
}

/// Wrap each logical line to `width` display columns (grapheme-aware: wide emoji take
/// two cells and are never split).
pub fn layout(s: &str, cur: usize, width: usize) -> Layout {
    let w = width.max(1);
    let cur = clamp(s, cur);
    let mut rows = vec![];
    let mut cursor = (0, 0);
    let mut off = 0;
    for line in s.split('\n') {
        let first = rows.len();
        let parts = wrap_line(line, w);
        let in_line = cur >= off && cur <= off + line.len();
        for (r, &(a, b)) in parts.iter().enumerate() {
            rows.push(line[a..b].to_string());
            let local = cur.wrapping_sub(off);
            // The cursor sits on the row whose range holds it; at a full row's end it
            // moves to the next row.
            if in_line && local >= a && (local < b || (local == b && r + 1 == parts.len())) {
                cursor = (first + r, str_width(&line[a..local]));
            }
        }
        off += line.len() + 1;
    }
    Layout { rows, cursor }
}

/// The char index (in the whole buffer, counting each newline as one char) where each of
/// [`layout`]'s rows starts.
pub fn row_starts(s: &str, width: usize) -> Vec<usize> {
    let w = width.max(1);
    let mut out = vec![];
    let mut off = 0;
    for line in s.split('\n') {
        for (a, _) in wrap_line(line, w) {
            out.push(off + line[..a].chars().count());
        }
        off += line.chars().count() + 1;
    }
    out
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
    #[test]
    fn emoji_graphemes_are_one_step_and_two_cells() {
        use super::*;
        // family (ZWJ), flag, heart with VS16, thumbs up with skin tone.
        let fam = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let s = format!("a{fam}\u{1F1FA}\u{1F1F8}\u{2764}\u{FE0F}\u{1F44D}\u{1F3FD}b");
        let mut c = s.len();
        left(&s, &mut c); // before b
        left(&s, &mut c); // before 👍🏽
        assert_eq!(&s[c..c + 8], "\u{1F44D}\u{1F3FD}");
        let l = layout(&s, s.len(), 80);
        assert_eq!(l.cursor, (0, 1 + 2 + 2 + 2 + 2 + 1));
        // wrapping never splits a grapheme: width 3 → "a👨‍👩‍👧" then "🇺🇸❤️"...
        let l = layout(&s, 0, 3);
        assert_eq!(l.rows[0], format!("a{fam}"));
        assert_eq!(l.rows[1], "\u{1F1FA}\u{1F1F8}");
        let mut t = s.clone();
        let mut c = 1 + fam.len();
        backspace(&mut t, &mut c);
        assert_eq!(c, 1);
        assert!(t.starts_with("a\u{1F1FA}"));
    }

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
