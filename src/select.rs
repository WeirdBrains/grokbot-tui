//! In-app text selection in the chat body (1.30.0: cell-based, mapped back to the source).
//!
//! A selection is two screen cells of the laid-out feed, encoded as
//! `row * COLS + col` (row = feed row counted from the top of the whole feed, col =
//! column inside the chat pane), so ordering positions is ordering numbers. Copying maps
//! the selected cells of message-text lines back to the message's source text
//! ([`extract`]): bubble padding and alignment, the name header, status/react/card rows,
//! code-block headers and table borders are left out, and soft-wrapped lines join into
//! the source's own lines (no wrap newlines).

use ratatui::layout::{Position, Rect};
use ratatui::text::Line;

/// Url prefix of copy buttons: `gbcopy:msg:<message key>` or `gbcopy:<code id>`.
pub const COPY_SCHEME: &str = "gbcopy:";

/// Columns per row in a position.
pub const COLS: usize = 1 << 16;

/// A selection between two feed cells ([`pos`]); both ends inclusive.
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

    /// Everything.
    pub fn all() -> Self {
        Self {
            anchor: 0,
            cursor: usize::MAX,
        }
    }
}

pub fn pos(row: usize, col: usize) -> usize {
    row.saturating_mul(COLS).saturating_add(col.min(COLS - 1))
}

/// Feed position for a screen cell, clipped to the chat pane `inner` (a drag that leaves
/// the pane selects up to its edge). `row_off`: feed rows scrolled above the pane.
pub fn pos_at(inner: Rect, col: u16, row: u16, row_off: usize) -> Option<usize> {
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let c = col.clamp(inner.x, inner.right() - 1);
    let r = row.clamp(inner.y, inner.bottom() - 1);
    Some(pos(
        row_off + (r - inner.y) as usize,
        (c - inner.x) as usize,
    ))
}

/// True when the cell is inside the pane (a press there starts a selection).
pub fn inside(inner: Rect, col: u16, row: u16) -> bool {
    inner.contains(Position::new(col, row))
}

/// One laid-out feed line for [`extract`].
pub struct MapLine<'a> {
    pub line: &'a Line<'static>,
    /// Wrapped height at the pane width.
    pub rows: usize,
    /// The feed entry whose text this line shows; `None` for chrome (headers, buttons,
    /// status rows, code-block headers, blank separators).
    pub entry: Option<usize>,
}

/// Display column where a line starts inside a pane `width` wide.
fn line_x0(l: &Line, width: usize) -> usize {
    let lw = l.width().min(width);
    match l.alignment {
        Some(ratatui::layout::Alignment::Right) => width - lw,
        Some(ratatui::layout::Alignment::Center) => (width - lw) / 2,
        _ => 0,
    }
}

/// The cells of a line: (feed position, char).
pub fn cells(l: &Line, row0: usize, rows: usize, width: usize) -> Vec<(usize, char)> {
    let width = width.max(1);
    let x0 = if rows > 1 { 0 } else { line_x0(l, width) };
    let mut out = vec![];
    let mut dc = 0usize;
    for sp in &l.spans {
        for ch in sp.content.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            let x = x0 + dc;
            out.push((pos(row0 + x / width, x % width), ch));
            dc += w;
        }
    }
    out
}

/// Greedy alignment of rendered chars to byte offsets in `src`: each non-space rendered
/// char maps to the next equal char within a short window (markdown syntax, list markers
/// and borders are skipped). Returns (byte start, byte end) per char, or None.
fn align(chars: &[char], src: &str) -> Vec<Option<(usize, usize)>> {
    const WINDOW: usize = 48;
    let sc: Vec<(usize, char)> = src.char_indices().collect();
    let mut at = 0usize;
    chars
        .iter()
        .map(|&c| {
            if c.is_whitespace() {
                return None;
            }
            let end = (at + WINDOW).min(sc.len());
            let j = (at..end).find(|&j| sc[j].1 == c)?;
            at = j + 1;
            let (b, ch) = sc[j];
            Some((b, b + ch.len_utf8()))
        })
        .collect()
}

/// The source text behind the selected cells. `sources[e]` is entry `e`'s raw text
/// (markdown source for bot replies). Entries fully covered copy whole; partly covered
/// ones copy the source slice from the first to the last selected char. Entries are
/// separated by a blank line.
pub fn extract(lines: &[MapLine], width: usize, sources: &[&str], sel: &Selection) -> String {
    let (a, b) = sel.range();
    // Per entry: every char's (position, char) in order.
    let mut per: std::collections::BTreeMap<usize, Vec<(usize, char)>> = Default::default();
    let mut row = 0usize;
    for l in lines {
        if let Some(e) = l.entry {
            per.entry(e)
                .or_default()
                .extend(cells(l.line, row, l.rows, width));
            // A line break between rendered lines (aligns as whitespace).
            per.entry(e).or_default().push((usize::MAX, '\n'));
        }
        row += l.rows;
    }
    let mut parts = vec![];
    for (e, cs) in per {
        let Some(src) = sources.get(e) else { continue };
        let chars: Vec<char> = cs.iter().map(|(_, c)| *c).collect();
        let map = align(&chars, src);
        let sel_idx: Vec<usize> = (0..cs.len())
            .filter(|&i| cs[i].0 != usize::MAX && cs[i].0 >= a && cs[i].0 <= b)
            .collect();
        if sel_idx.is_empty() {
            continue;
        }
        let mapped: Vec<usize> = (0..cs.len()).filter(|&i| map[i].is_some()).collect();
        let all_mapped_selected =
            !mapped.is_empty() && mapped.iter().all(|&i| cs[i].0 >= a && cs[i].0 <= b);
        if all_mapped_selected {
            parts.push(src.trim_end_matches('\n').to_string());
            continue;
        }
        let spans: Vec<(usize, usize)> = sel_idx.iter().filter_map(|&i| map[i]).collect();
        match (
            spans.iter().map(|s| s.0).min(),
            spans.iter().map(|s| s.1).max(),
        ) {
            (Some(s), Some(t)) => parts.push(src[s..t].to_string()),
            // Nothing maps (e.g. only borders): the visible chars, trimmed.
            _ => {
                let t: String = sel_idx.iter().map(|&i| cs[i].1).collect();
                let t = t.trim().to_string();
                if !t.is_empty() {
                    parts.push(t);
                }
            }
        }
    }
    parts.join("\n\n")
}

/// Plain text of the feed (every entry's source, blank line between).
pub fn plain_text(feed: &[crate::app::Entry]) -> String {
    feed.iter()
        .map(|e| e.text.trim_end_matches('\n'))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Fenced/indented code blocks of a markdown source, in order: just the code (no fences,
/// original indentation, no wrapping).
pub fn code_blocks(src: &str) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut out = vec![];
    let mut cur: Option<String> = None;
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => cur = Some(String::new()),
            Event::Text(t) if cur.is_some() => cur.as_mut().unwrap().push_str(&t),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = cur.take() {
                    out.push(c.trim_end_matches('\n').to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// "Copied 12 lines" / "Copied 40 chars".
pub fn copied_notice(text: &str) -> String {
    let n = text.lines().count();
    if n > 1 {
        format!("Copied {n} lines.")
    } else {
        format!("Copied {} chars.", text.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Alignment;

    fn l(s: &str) -> Line<'static> {
        Line::from(s.to_string())
    }
    fn r(s: &str) -> Line<'static> {
        Line::from(s.to_string()).alignment(Alignment::Right)
    }

    /// you (right-aligned, wrapped) then the bot (header + wrapped markdown + code).
    fn feed() -> (Vec<Line<'static>>, Vec<Option<usize>>, Vec<&'static str>) {
        let user = "the quick brown fox jumps";
        let bot = "Here is **bold** text that wraps\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        let lines = vec![
            r("you"),                    // 0 header
            r("the quick brown"),        // 1
            r("fox jumps"),              // 2
            r("[⧉] [☺+]"),               // 3 buttons
            l("Helper"),                 // 4 header
            l("Here is bold text"),      // 5
            l("that wraps"),             // 6
            l(""),                       // 7
            l("rust            [⧉]"),    // 8 code header
            l("│fn main() {"),           // 9
            l("│    println!(\"hi\");"), // 10
            l("│}"),                     // 11
        ];
        let entry = vec![
            None,
            Some(0),
            Some(0),
            None,
            None,
            Some(1),
            Some(1),
            Some(1),
            None,
            Some(1),
            Some(1),
            Some(1),
        ];
        (lines, entry, vec![user, bot])
    }

    fn ex(sel: Selection) -> String {
        let (lines, entry, src) = feed();
        let ml: Vec<MapLine> = lines
            .iter()
            .zip(&entry)
            .map(|(line, e)| MapLine {
                line,
                rows: 1,
                entry: *e,
            })
            .collect();
        extract(&ml, 30, &src, &sel)
    }

    #[test]
    fn selection_maps_wrapped_bubbles_to_source_without_chrome() {
        // From the pane's left edge on row 1 to the right edge of row 2 (user bubble is
        // right-aligned at col 15..30): the whole message, one line, no wrap newline.
        assert_eq!(
            ex(Selection {
                anchor: pos(1, 0),
                cursor: pos(2, 29)
            }),
            "the quick brown fox jumps"
        );
        // Part of it across the wrap: "brown fox".
        let x = 30 - 15; // "the quick brown" starts at col 15
        assert_eq!(
            ex(Selection {
                anchor: pos(1, x + 10),
                cursor: pos(2, 30 - 9 + 2)
            }),
            "brown fox"
        );
        // Header and buttons rows alone copy nothing.
        assert_eq!(
            ex(Selection {
                anchor: pos(3, 0),
                cursor: pos(4, 29)
            }),
            ""
        );
        // Across both bubbles: whole user message + partial bot (markdown source kept).
        assert_eq!(
            ex(Selection {
                anchor: pos(0, 0),
                cursor: pos(6, 3)
            }),
            "the quick brown fox jumps\n\nHere is **bold** text that"
        );
    }

    #[test]
    fn selection_inside_code_keeps_indentation_and_skips_borders() {
        assert_eq!(
            ex(Selection {
                anchor: pos(9, 0),
                cursor: pos(11, 29)
            }),
            "fn main() {\n    println!(\"hi\");\n}"
        );
        // Everything: each message's full source.
        let all = ex(Selection::all());
        assert!(all.starts_with("the quick brown fox jumps\n\nHere is **bold**"));
        assert!(all.ends_with("```"));
    }

    #[test]
    fn pos_at_clips_to_the_pane() {
        let inner = Rect::new(10, 5, 20, 4);
        assert_eq!(pos_at(inner, 12, 6, 100), Some(pos(101, 2)));
        assert_eq!(pos_at(inner, 0, 0, 0), Some(pos(0, 0)));
        assert_eq!(pos_at(inner, 99, 99, 0), Some(pos(3, 19)));
        assert!(!inside(inner, 9, 6));
    }

    #[test]
    fn code_blocks_are_extracted_verbatim() {
        let src = "intro\n\n```py\ndef f():\n\treturn  1\n```\n\nmid\n\n~~~\na\n~~~\n";
        assert_eq!(code_blocks(src), vec!["def f():\n\treturn  1", "a"]);
    }

    #[test]
    fn notices_count_lines_or_chars() {
        assert_eq!(copied_notice("a\nb\nc"), "Copied 3 lines.");
        assert_eq!(copied_notice("abc"), "Copied 3 chars.");
    }
}
