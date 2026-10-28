//! Markdown for chat messages (CommonMark + GFM tables, strikethrough and task lists via
//! pulldown-cmark), rendered to lines that are already wrapped to the target width, so
//! every line is exactly one screen row and the chat's scroll math stays exact.
//!
//! Styles come from theme parts (`md_*`); bold / italic / strikethrough are modifiers on
//! the message's own style.

use crate::input::{grapheme_width, str_width};
use crate::theme;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;

type Seg = (String, Style);

fn rs(s: ratatui_lipgloss::Style) -> Style {
    Style::from(&s)
}

/// One level of line prefix (a quote bar or a list item's marker / hanging indent).
struct Indent {
    first: Vec<Seg>,
    rest: Vec<Seg>,
    used: bool,
    list_item: bool,
}

struct R {
    width: usize,
    base: Style,
    out: Vec<Line<'static>>,
    inline: Vec<Seg>,
    styles: Vec<Style>,
    indents: Vec<Indent>,
    lists: Vec<Option<u64>>,
    heading: bool,
    code: Option<(String, String)>,
    link: Option<(usize, String)>,
    image: Option<(usize, String)>,
    table: Option<Table>,
    /// Just after a gap or a container start: the next block needs no gap.
    fresh: bool,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<Vec<Seg>>>,
    row: Vec<Vec<Seg>>,
    header_rows: usize,
}

fn segs_width(s: &[Seg]) -> usize {
    s.iter().map(|(t, _)| str_width(t)).sum()
}

fn segs_text(s: &[Seg]) -> String {
    s.iter().map(|(t, _)| t.as_str()).collect()
}

/// Cut styled segments to at most `w` columns, ending in '…' when anything was cut.
fn truncate(segs: &[Seg], w: usize) -> Vec<Seg> {
    if segs_width(segs) <= w {
        return segs.to_vec();
    }
    let mut out = vec![];
    let mut used = 0;
    let limit = w.saturating_sub(1);
    'outer: for (t, st) in segs {
        let mut piece = String::new();
        for g in t.graphemes(true) {
            let gw = grapheme_width(g);
            if used + gw > limit {
                if !piece.is_empty() {
                    out.push((piece, *st));
                }
                break 'outer;
            }
            used += gw;
            piece.push_str(g);
        }
        if !piece.is_empty() {
            out.push((piece, *st));
        }
    }
    if w > 0 {
        let st = segs.last().map(|s| s.1).unwrap_or_default();
        out.push(("…".into(), st));
    }
    out
}

/// Word-wrap styled segments to `w` columns. '\n' inside a segment forces a break; words
/// longer than `w` are split between graphemes.
pub fn wrap_segs(segs: &[Seg], w: usize) -> Vec<Vec<Seg>> {
    let w = w.max(1);
    let mut lines: Vec<Vec<Seg>> = vec![];
    let mut line: Vec<Seg> = vec![];
    let mut lw = 0;
    // Words: runs of styled pieces between spaces / newlines.
    enum Tok {
        Word(Vec<Seg>),
        Space(Style),
        Break,
    }
    let mut toks = vec![];
    let mut word: Vec<Seg> = vec![];
    for (t, st) in segs {
        let mut cur = String::new();
        for c in t.chars() {
            if c == ' ' || c == '\n' {
                if !cur.is_empty() {
                    word.push((std::mem::take(&mut cur), *st));
                }
                if !word.is_empty() {
                    toks.push(Tok::Word(std::mem::take(&mut word)));
                }
                toks.push(if c == ' ' {
                    Tok::Space(*st)
                } else {
                    Tok::Break
                });
            } else {
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            word.push((cur, *st));
        }
    }
    if !word.is_empty() {
        toks.push(Tok::Word(word));
    }
    let mut pending_space: Option<Style> = None;
    for tok in toks {
        match tok {
            Tok::Break => {
                lines.push(std::mem::take(&mut line));
                lw = 0;
                pending_space = None;
            }
            Tok::Space(st) => {
                if lw > 0 {
                    pending_space = Some(st);
                }
            }
            Tok::Word(ws) => {
                let ww = segs_width(&ws);
                let sp = usize::from(pending_space.is_some());
                if lw > 0 && lw + sp + ww > w {
                    lines.push(std::mem::take(&mut line));
                    lw = 0;
                } else if let Some(st) = pending_space {
                    line.push((" ".into(), st));
                    lw += 1;
                }
                pending_space = None;
                if ww <= w - lw {
                    lw += ww;
                    line.extend(ws);
                    continue;
                }
                for (t, st) in ws {
                    for g in t.graphemes(true) {
                        let gw = grapheme_width(g);
                        if lw + gw > w && lw > 0 {
                            lines.push(std::mem::take(&mut line));
                            lw = 0;
                        }
                        line.push((g.to_string(), st));
                        lw += gw;
                    }
                }
            }
        }
    }
    lines.push(line);
    lines
}

impl R {
    fn cur(&self) -> Style {
        self.styles.last().copied().unwrap_or(self.base)
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        let s = f(self.cur());
        self.styles.push(s);
    }

    fn text(&mut self, t: &str) {
        if let Some((_, buf)) = self.code.as_mut() {
            buf.push_str(t);
            return;
        }
        let st = self.cur();
        self.inline.push((t.to_string(), st));
    }

    fn prefix(&mut self) -> Vec<Seg> {
        let mut p = vec![];
        for ind in &mut self.indents {
            p.extend(if ind.used {
                ind.rest.clone()
            } else {
                ind.first.clone()
            });
            ind.used = true;
        }
        p
    }

    fn prefix_width(&self) -> usize {
        self.indents.iter().map(|i| segs_width(&i.rest)).sum()
    }

    fn avail(&self) -> usize {
        self.width.saturating_sub(self.prefix_width()).max(4)
    }

    fn emit(&mut self, body: Vec<Seg>) {
        self.fresh = false;
        let mut segs = self.prefix();
        segs.extend(body);
        self.out.push(Line::from(
            segs.into_iter()
                .map(|(t, s)| Span::styled(t, s))
                .collect::<Vec<_>>(),
        ));
    }

    fn gap(&mut self) {
        let in_item = self.indents.iter().any(|i| i.list_item);
        if self.fresh {
            return;
        }
        self.fresh = true;
        if !self.out.is_empty() && !in_item {
            // A blank (prefix-only) line keeps quote bars continuous.
            let p: Vec<Seg> = self
                .indents
                .iter()
                .flat_map(|i| i.rest.clone())
                .map(|(t, s)| (t.trim_end().to_string(), s))
                .collect();
            self.out.push(Line::from(
                p.into_iter()
                    .map(|(t, s)| Span::styled(t, s))
                    .collect::<Vec<_>>(),
            ));
        }
    }

    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let segs = std::mem::take(&mut self.inline);
        if segs.iter().all(|(t, _)| t.trim().is_empty()) {
            return;
        }
        for l in wrap_segs(&segs, self.avail()) {
            self.emit(l);
        }
    }

    fn code_block(&mut self, lang: String, body: String) {
        self.gap();
        let st = rs(theme::md_code_block());
        let avail = self.avail();
        let inner = avail.saturating_sub(2).max(1);
        let mut rows: Vec<String> = vec![];
        for l in body.trim_end_matches('\n').split('\n') {
            let l = l.replace('\t', "    ");
            let mut cur = String::new();
            let mut cw = 0;
            for g in l.graphemes(true) {
                let gw = grapheme_width(g);
                if cw + gw > inner && cw > 0 {
                    rows.push(std::mem::take(&mut cur));
                    cw = 0;
                }
                cur.push_str(g);
                cw += gw;
            }
            rows.push(cur);
        }
        let bw = rows
            .iter()
            .map(|r| str_width(r))
            .max()
            .unwrap_or(0)
            .max(str_width(&lang))
            .min(inner);
        if !lang.is_empty() {
            let label = truncate(&[(lang, rs(theme::md_rule()))], bw + 2);
            self.emit(label);
        }
        for r in rows {
            let pad = bw.saturating_sub(str_width(&r));
            self.emit(vec![(format!(" {r}{} ", " ".repeat(pad)), st)]);
        }
    }

    fn table(&mut self, t: Table) {
        self.gap();
        let border = rs(theme::md_table_border());
        let n = t.rows.iter().map(Vec::len).max().unwrap_or(0);
        if n == 0 {
            return;
        }
        let mut w: Vec<usize> = (0..n)
            .map(|c| {
                t.rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| segs_width(s))
                    .max()
                    .unwrap_or(0)
                    .max(1)
            })
            .collect();
        let avail = self.avail();
        // Borders: "│ a │ b │" = sum(w) + 3n + 1.
        while w.iter().sum::<usize>() + 3 * n + 1 > avail {
            let (i, &m) = w
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| **v)
                .expect("n > 0");
            if m <= 3 {
                break;
            }
            w[i] -= 1;
        }
        let rule = |l: &str, m: &str, r: &str| -> Vec<Seg> {
            let mut s = l.to_string();
            for (i, cw) in w.iter().enumerate() {
                s.push_str(&"─".repeat(cw + 2));
                s.push_str(if i + 1 == n { r } else { m });
            }
            vec![(s, border)]
        };
        self.emit(rule("┌", "┬", "┐"));
        for (ri, row) in t.rows.iter().enumerate() {
            let mut line: Vec<Seg> = vec![("│".into(), border)];
            for (c, cw) in w.iter().enumerate() {
                let cell = row.get(c).cloned().unwrap_or_default();
                let mut cell = truncate(&cell, *cw);
                if ri < t.header_rows {
                    for s in &mut cell {
                        s.1 = s.1.add_modifier(Modifier::BOLD);
                    }
                }
                let pad = cw.saturating_sub(segs_width(&cell));
                line.push((" ".into(), self.base));
                line.extend(cell);
                line.push((format!("{} ", " ".repeat(pad)), self.base));
                line.push(("│".into(), border));
            }
            self.emit(line);
            if ri + 1 == t.header_rows && t.rows.len() > t.header_rows {
                self.emit(rule("├", "┼", "┤"));
            }
        }
        self.emit(rule("└", "┴", "┘"));
    }

    fn event(&mut self, ev: Event) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                let st = self.cur().patch(rs(theme::md_code()));
                self.inline.push((t.to_string(), st));
            }
            Event::Html(t) | Event::InlineHtml(t) => self.text(&t),
            Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t),
            Event::FootnoteReference(t) => self.text(&format!("[^{t}]")),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush();
                self.gap();
                let w = self.avail();
                self.emit(vec![("─".repeat(w), rs(theme::md_rule()))]);
            }
            Event::TaskListMarker(done) => {
                if let Some(ind) = self.indents.iter_mut().rev().find(|i| i.list_item) {
                    let st = rs(theme::md_list_marker());
                    ind.first = vec![(if done { "☑ " } else { "☐ " }.into(), st)];
                    ind.rest = vec![("  ".into(), self.base)];
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.gap();
            }
            Tag::Heading { .. } => {
                self.flush();
                self.gap();
                self.heading = true;
                let h = rs(theme::md_heading());
                self.push_style(|s| s.patch(h));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.gap();
                let q = rs(theme::md_quote());
                self.indents.push(Indent {
                    first: vec![("│ ".into(), q)],
                    rest: vec![("│ ".into(), q)],
                    used: false,
                    list_item: false,
                });
                self.push_style(|s| s.patch(q));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => {
                        l.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.gap();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1);
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => format!("{} ", ["•", "◦", "▪"][depth % 3]),
                };
                let mw = str_width(&marker);
                self.indents.push(Indent {
                    first: vec![(marker, rs(theme::md_list_marker()))],
                    rest: vec![(" ".repeat(mw), self.base)],
                    used: false,
                    list_item: true,
                });
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.link = Some((self.inline.len(), dest_url.to_string()));
                let l = rs(theme::md_link());
                self.push_style(|s| s.patch(l));
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((self.inline.len(), dest_url.to_string()));
                let st = self.cur();
                self.push_style(|_| st);
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table::default());
            }
            Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = false;
                self.styles.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.indents.pop();
                self.styles.pop();
            }
            TagEnd::CodeBlock => {
                if let Some((lang, body)) = self.code.take() {
                    self.code_block(lang, body);
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.flush();
                self.indents.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some((at, url)) = self.link.take() {
                    let label = segs_text(&self.inline[at.min(self.inline.len())..]);
                    let bare = url.trim_start_matches("mailto:");
                    if !url.is_empty() && label.trim() != bare {
                        self.inline
                            .push((format!(" <{url}>"), rs(theme::md_rule())));
                    }
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                if let Some((at, url)) = self.image.take() {
                    let alt = segs_text(&self.inline[at.min(self.inline.len())..]);
                    self.inline.truncate(at);
                    self.inline
                        .push((placeholder(&alt, &url), rs(theme::image_placeholder())));
                    self.inline
                        .push((format!(" <{url}>"), rs(theme::md_rule())));
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(t) = self.table.as_mut() {
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    if tag == TagEnd::TableHead {
                        t.header_rows = t.rows.len();
                    }
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.table(t);
                }
            }
            _ => {}
        }
    }
}

/// Render `text` as markdown in `base` style, wrapped to `width` columns. Every returned
/// line is at most `width` columns wide.
pub fn render(text: &str, width: usize, base: Style) -> Vec<Line<'static>> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut r = R {
        width: width.max(1),
        base,
        out: vec![],
        inline: vec![],
        styles: vec![],
        indents: vec![],
        lists: vec![],
        heading: false,
        code: None,
        link: None,
        image: None,
        table: None,
        fresh: false,
    };
    for ev in Parser::new_ext(text, opts) {
        r.event(ev);
    }
    r.flush();
    if r.out.is_empty() {
        r.out.push(Line::default());
    }
    // Defensive: never return a line wider than `width` (keeps row counts exact).
    r.out
        .into_iter()
        .map(|l| {
            if l.width() <= width.max(1) {
                return l;
            }
            let segs: Vec<Seg> = l
                .spans
                .iter()
                .map(|s| (s.content.to_string(), s.style))
                .collect();
            Line::from(
                truncate(&segs, width.max(1))
                    .into_iter()
                    .map(|(t, s)| Span::styled(t, s))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// The last path segment of a URL or path, without query or fragment.
pub fn file_name(url: &str) -> &str {
    let u = url.split(['?', '#']).next().unwrap_or(url);
    u.trim_end_matches('/')
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(u)
}

/// Human size: 512 B, 3.4 KB, 1.2 MB.
pub fn human_size(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// The text shown in place of an image: `[🖼 alt · file.png · 12.0 KB]`. The size is only
/// known for local files. Terminal graphics (kitty/sixel/iTerm2) are not drawn yet.
pub fn placeholder(alt: &str, url: &str) -> String {
    let name = file_name(url);
    let mut parts = vec![];
    if !alt.trim().is_empty() {
        parts.push(alt.trim().to_string());
    }
    if !name.is_empty() && name != alt.trim() {
        parts.push(name.to_string());
    }
    let local = url.strip_prefix("file://").unwrap_or(url);
    if !local.contains("://")
        && let Ok(m) = std::fs::metadata(local)
    {
        parts.push(human_size(m.len()));
    }
    if parts.is_empty() {
        parts.push("image".into());
    }
    format!("[🖼 {}]", parts.join(" · "))
}

fn looks_like_image(u: &str) -> bool {
    let name = file_name(u).to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"]
        .iter()
        .any(|e| name.ends_with(e))
}

/// Something in a reply that `/open` can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Openable {
    pub url: String,
    pub label: String,
    pub image: bool,
}

/// Images (`![alt](url)` and bare image URLs) and links in `text`, in order, no duplicates.
pub fn openables(text: &str) -> Vec<Openable> {
    let mut out: Vec<Openable> = vec![];
    let mut cur: Option<(String, String, bool)> = None;
    let push = |out: &mut Vec<Openable>, o: Openable| {
        if !o.url.is_empty() && !out.iter().any(|x| x.url == o.url) {
            out.push(o);
        }
    };
    for ev in Parser::new_ext(text, Options::ENABLE_TABLES) {
        match ev {
            Event::Start(Tag::Image { dest_url, .. }) => {
                cur = Some((dest_url.to_string(), String::new(), true))
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                cur = Some((
                    dest_url.to_string(),
                    String::new(),
                    looks_like_image(&dest_url),
                ))
            }
            Event::Text(t) | Event::Code(t) => match cur.as_mut() {
                Some((_, l, _)) => l.push_str(&t),
                None => {
                    for w in t.split_whitespace() {
                        let w = w.trim_matches(|c: char| {
                            matches!(c, '(' | ')' | ',' | '.' | '"' | '\'')
                        });
                        let web = w.starts_with("http://") || w.starts_with("https://");
                        if (web || w.starts_with('/') || w.starts_with("~/")) && looks_like_image(w)
                        {
                            push(
                                &mut out,
                                Openable {
                                    url: w.into(),
                                    label: file_name(w).into(),
                                    image: true,
                                },
                            );
                        } else if web {
                            push(
                                &mut out,
                                Openable {
                                    url: w.into(),
                                    label: w.into(),
                                    image: false,
                                },
                            );
                        }
                    }
                }
            },
            Event::End(TagEnd::Image) | Event::End(TagEnd::Link) => {
                if let Some((url, label, image)) = cur.take() {
                    push(&mut out, Openable { url, label, image });
                }
            }
            _ => {}
        }
    }
    out
}

/// Open a URL or local file with the system handler, without a shell: `xdg-open` (Linux and
/// BSD), `open` (macOS), `rundll32 url.dll,FileProtocolHandler` (Windows).
pub fn open_external(target: &str) -> std::io::Result<()> {
    let t = if let Some(rest) = target.strip_prefix("~/") {
        dirs::home_dir()
            .map(|h| h.join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| target.to_string())
    } else {
        target.to_string()
    };
    let web = t.starts_with("http://") || t.starts_with("https://");
    let file = t.strip_prefix("file://").unwrap_or(&t);
    if !web && !std::path::Path::new(file).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{file} doesn't exist on this machine"),
        ));
    }
    if cfg!(test) {
        return Ok(()); // never launch a viewer from tests
    }
    let arg = if web { t.as_str() } else { file };
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", arg]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(arg);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(arg);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str, w: usize) -> Vec<String> {
        render(text, w, Style::default())
            .into_iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn styled(text: &str, w: usize, needle: &str) -> Style {
        for l in render(text, w, Style::default()) {
            for s in l.spans {
                if s.content == needle {
                    return s.style;
                }
            }
        }
        panic!("{needle} not found")
    }

    #[test]
    fn blocks_render_and_fit() {
        let md = "# Title\n\nSome **bold**, *it*, ~~gone~~ and `code`.\n\n- a\n  - nested\n- [x] done\n- [ ] todo\n\n1. one\n2. two\n\n> quoted text\n\n```rust\nfn main() {}\n```\n\n---\n\n| a | bb |\n|---|----|\n| 1 | 22 |\n\n[site](https://x.dev) and <https://y.dev>";
        let rows = plain(md, 30);
        let joined = rows.join("\n");
        for want in [
            "Title",
            "• a",
            "  ◦ nested",
            "☑ done",
            "☐ todo",
            "1. one",
            "2. two",
            "│ quoted text",
            "rust",
            " fn main() {} ",
            "──────",
            "┌───┬────┐",
            "│ a │ bb │",
            "├───┼────┤",
            "│ 1 │ 22 │",
            "site <https://x.dev>",
        ] {
            assert!(joined.contains(want), "missing {want:?} in\n{joined}");
        }
        assert!(!joined.contains("**") && !joined.contains("~~") && !joined.contains("```"));
        assert!(!joined.contains("y.dev <"), "autolink shows once: {joined}");
        for r in &rows {
            assert!(str_width(r) <= 30, "{r:?}");
        }
        assert!(styled(md, 30, "bold").add_modifier.contains(Modifier::BOLD));
        assert!(styled(md, 30, "it").add_modifier.contains(Modifier::ITALIC));
        assert!(
            styled(md, 30, "gone")
                .add_modifier
                .contains(Modifier::CROSSED_OUT)
        );
    }

    #[test]
    fn wrapping_is_exact_for_long_text_code_and_tables() {
        let md = format!(
            "{}\n\n```\n{}\n```\n\n| {} | b |\n|--|--|\n| c | d |\n\n- {}",
            "word ".repeat(40),
            "x".repeat(70),
            "wide".repeat(10),
            "long item ".repeat(10)
        );
        for w in [8, 13, 20, 41] {
            for r in plain(&md, w) {
                assert!(str_width(&r) <= w, "w={w} {r:?}");
            }
        }
        // hanging indent under a list marker
        let rows = plain("- aaa bbb ccc ddd", 9);
        assert_eq!(rows, ["• aaa bbb", "  ccc ddd"]);
    }

    #[test]
    fn images_and_links_are_found_and_shown_as_placeholders() {
        let o = openables(
            "see ![a cat](https://x/cat.png?s=1), [docs](https://d.dev) and /tmp/b.jpg or https://e.dev/x.webp",
        );
        let urls: Vec<(&str, bool)> = o.iter().map(|o| (o.url.as_str(), o.image)).collect();
        assert_eq!(
            urls,
            [
                ("https://x/cat.png?s=1", true),
                ("https://d.dev", false),
                ("/tmp/b.jpg", true),
                ("https://e.dev/x.webp", true)
            ]
        );
        assert_eq!(
            placeholder("a cat", "https://x/cat.png?s=1"),
            "[🖼 a cat · cat.png]"
        );
        assert_eq!(placeholder("", "https://x/"), "[🖼 x]");
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("p.png");
        std::fs::write(&f, vec![0u8; 2048]).unwrap();
        assert_eq!(placeholder("", f.to_str().unwrap()), "[🖼 p.png · 2.0 KB]");
        let rows = plain("![a cat](https://x/cat.png)", 60).join("");
        assert_eq!(rows, "[🖼 a cat · cat.png] <https://x/cat.png>");
        assert!(open_external("/definitely/not/here.png").is_err());
    }
}
