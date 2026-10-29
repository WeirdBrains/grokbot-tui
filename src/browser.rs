//! Built-in text browser: a large modal that fetches a page (no JavaScript, no cookies)
//! and renders it as styled text with numbered links. Works the same on every platform
//! (nothing shells out).
use crate::app::{App, Event};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::time::Duration;

/// Largest page body read (bytes).
pub const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
/// Per-request timeout.
pub const TIMEOUT: Duration = Duration::from_secs(20);
/// Pages kept in back/forward history.
const HISTORY: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub enum Fetched {
    Text {
        url: String,
        content_type: String,
        body: String,
    },
    Binary {
        url: String,
        content_type: String,
        size: Option<u64>,
    },
}

impl Fetched {
    pub fn url(&self) -> &str {
        match self {
            Fetched::Text { url, .. } | Fetched::Binary { url, .. } => url,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub url: String,
    pub image: bool,
}

/// A rendered page.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub title: String,
    pub lines: Vec<Line<'static>>,
    pub plain: Vec<String>,
    pub links: Vec<Link>,
    /// Per line: (start col, end col, link index) of each numbered link marker/text.
    pub hits: Vec<Vec<(usize, usize, usize)>>,
    /// A non-text resource: offer download / system opener.
    pub binary: bool,
}

/// GET `url` with redirects, a size cap and a timeout. No cookie store, no JavaScript.
pub async fn fetch(url: String) -> Result<Fetched, String> {
    let parsed = url::Url::parse(&url).map_err(|e| format!("bad URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("only http and https URLs can be opened here".into());
    }
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent(concat!(
            "grokbot-tui/",
            env!("CARGO_PKG_VERSION"),
            " (text browser; +https://github.com/WeirdBrains/grokbot-tui)"
        ))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(parsed)
        .header(
            "accept",
            "text/html,text/plain,text/markdown,application/json;q=0.9,*/*;q=0.5",
        )
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = resp.status();
    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let textual = content_type.is_empty()
        || content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("markdown");
    if !textual {
        return Ok(Fetched::Binary {
            url: final_url,
            content_type,
            size: resp.content_length(),
        });
    }
    let mut buf: Vec<u8> = vec![];
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        buf.extend_from_slice(&chunk);
        if buf.len() > MAX_PAGE_BYTES {
            buf.truncate(MAX_PAGE_BYTES);
            break;
        }
    }
    Ok(Fetched::Text {
        url: final_url,
        content_type,
        body: String::from_utf8_lossy(&buf).into_owned(),
    })
}

fn resolve(base: &str, href: &str) -> String {
    url::Url::parse(base)
        .and_then(|b| b.join(href))
        .map(|u| u.to_string())
        .unwrap_or_else(|_| href.to_string())
}

fn html_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(a) = lower.find("<title") else {
        return String::new();
    };
    let Some(gt) = lower[a..].find('>') else {
        return String::new();
    };
    let start = a + gt + 1;
    let end = lower[start..]
        .find("</title")
        .map(|e| start + e)
        .unwrap_or(start);
    html.get(start..end)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Strip `<script>`, `<style>` and `<noscript>` blocks (html2text skips most, this is belt and braces).
fn strip_scripts(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    'outer: while i < html.len() {
        for tag in ["<script", "<style", "<noscript"] {
            if lower[i..].starts_with(tag) {
                let close = format!("</{}", &tag[1..]);
                match lower[i..].find(&close) {
                    Some(e) => {
                        let end = i + e;
                        i = lower[end..]
                            .find('>')
                            .map(|g| end + g + 1)
                            .unwrap_or(html.len());
                    }
                    None => i = html.len(),
                }
                continue 'outer;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Render `f` at `width` columns.
pub fn render(f: &Fetched, width: usize) -> Page {
    let width = width.max(20);
    match f {
        Fetched::Binary {
            url,
            content_type,
            size,
        } => {
            let mut p = Page {
                title: crate::download::save_name(url),
                binary: true,
                ..Default::default()
            };
            let size = size
                .map(crate::markdown::human_size)
                .unwrap_or_else(|| "unknown size".into());
            for l in [
                format!("This is not a web page: {content_type} ({size})."),
                String::new(),
                "d  download it (the [⤓] download modal)".into(),
                "o  open it with the system's default app".into(),
            ] {
                p.plain.push(l.clone());
                p.lines.push(Line::raw(l));
                p.hits.push(vec![]);
            }
            p
        }
        Fetched::Text {
            url,
            content_type,
            body,
        } => {
            if content_type.contains("json") {
                let pretty = serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|v| serde_json::to_string_pretty(&v).ok())
                    .unwrap_or_else(|| body.clone());
                plain_page(url, &pretty, width, false)
            } else if content_type.contains("markdown")
                || (content_type.starts_with("text/plain")
                    && (url.ends_with(".md") || url.ends_with(".markdown")))
            {
                plain_page(url, body, width, true)
            } else if content_type.starts_with("text/plain") {
                plain_page(url, body, width, false)
            } else {
                render_html(body, url, width)
            }
        }
    }
}

fn plain_page(url: &str, text: &str, width: usize, markdown: bool) -> Page {
    let mut p = Page {
        title: crate::download::save_name(url),
        ..Default::default()
    };
    let lines: Vec<Line<'static>> = if markdown {
        crate::markdown::render(text, width, rs(theme::modal_text()))
    } else {
        text.lines()
            .flat_map(|l| {
                let chars: Vec<char> = l.chars().collect();
                if chars.is_empty() {
                    vec![String::new()]
                } else {
                    chars
                        .chunks(width)
                        .map(|c| c.iter().collect::<String>())
                        .collect()
                }
            })
            .map(Line::raw)
            .collect()
    };
    for l in lines {
        p.plain.push(line_text(&l));
        p.lines.push(l);
        p.hits.push(vec![]);
    }
    // Links in the text: a numbered list at the bottom.
    let found = crate::markdown::openables(text);
    if !found.is_empty() {
        p.plain.push(String::new());
        p.lines.push(Line::default());
        p.hits.push(vec![]);
        for o in found {
            let resolved = resolve(url, &o.url);
            let n = p.links.len() + 1;
            let mark = format!("[{n}]");
            let w = mark.chars().count();
            let text = format!("{mark} {resolved}");
            p.plain.push(text.clone());
            p.lines.push(Line::from(vec![
                sp(rs(theme::browser_link_number()), mark),
                Span::raw(" "),
                sp(rs(theme::md_link()), resolved.clone()),
            ]));
            p.hits
                .push(vec![(0, w + 1 + resolved.chars().count(), n - 1)]);
            p.links.push(Link {
                url: resolved,
                image: o.image,
            });
        }
    }
    p
}

fn line_text(l: &Line<'_>) -> String {
    l.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// HTML → styled lines with `[n]` after each link and `[🖼 alt]` for images.
pub fn render_html(html: &str, base: &str, width: usize) -> Page {
    use html2text::render::RichAnnotation as A;
    let mut page = Page {
        title: html_title(html),
        ..Default::default()
    };
    let cleaned = strip_scripts(html);
    let lines = html2text::config::rich()
        .lines_from_read(cleaned.as_bytes(), width.saturating_sub(5).max(10))
        .unwrap_or_default();
    // Same target as the previous segment keeps the same number (a link wrapped over spans).
    let mut last_link: Option<(String, usize)> = None;
    for tl in lines {
        let mut spans: Vec<Span<'static>> = vec![];
        let mut hits = vec![];
        let mut col = 0usize;
        let mut plain = String::new();
        let segs: Vec<_> = tl.tagged_strings().cloned().collect();
        let heading = segs.first().is_some_and(|s| s.s.starts_with('#'));
        for (i, ts) in segs.iter().enumerate() {
            let mut style = if heading {
                rs(theme::browser_heading())
            } else {
                rs(theme::modal_text())
            };
            let mut link: Option<(String, bool)> = None;
            for a in &ts.tag {
                match a {
                    A::Strong => style = style.add_modifier(Modifier::BOLD),
                    A::Emphasis => style = style.add_modifier(Modifier::ITALIC),
                    A::Strikeout => style = style.add_modifier(Modifier::CROSSED_OUT),
                    A::Code | A::Preformat(_) => style = style.patch(rs(theme::md_code())),
                    A::Link(u) => link = Some((resolve(base, u), false)),
                    A::Image(src) => link = Some((resolve(base, src), true)),
                    _ => {}
                }
            }
            let text = ts.s.clone();
            let w = crate::input::str_width(&text);
            match link {
                Some((u, image)) => {
                    let idx = match &last_link {
                        Some((lu, li)) if *lu == u => *li,
                        _ => {
                            page.links.push(Link {
                                url: u.clone(),
                                image,
                            });
                            page.links.len() - 1
                        }
                    };
                    last_link = Some((u.clone(), idx));
                    let shown = if image {
                        let alt = text.trim();
                        format!("[🖼 {}]", if alt.is_empty() { "image" } else { alt })
                    } else {
                        text.clone()
                    };
                    let sw = crate::input::str_width(&shown);
                    let st = if image {
                        rs(theme::image_placeholder())
                    } else {
                        style.patch(rs(theme::md_link()))
                    };
                    spans.push(Span::styled(shown.clone(), st));
                    plain.push_str(&shown);
                    let start = col;
                    col += sw;
                    // Number after the last segment of this link on the line.
                    let next_same = segs.get(i + 1).is_some_and(|n| {
                        n.tag.iter().any(|a| match a {
                            A::Link(x) | A::Image(x) => resolve(base, x) == u,
                            _ => false,
                        })
                    });
                    if !next_same {
                        let mark = format!("[{}]", idx + 1);
                        let mw = mark.chars().count();
                        spans.push(sp(rs(theme::browser_link_number()), mark.clone()));
                        plain.push_str(&mark);
                        col += mw;
                    }
                    hits.push((start, col, idx));
                }
                None => {
                    last_link = None;
                    spans.push(Span::styled(text.clone(), style));
                    plain.push_str(&text);
                    col += w;
                }
            }
        }
        page.lines.push(Line::from(spans));
        page.plain.push(plain);
        page.hits.push(hits);
    }
    if page.title.is_empty() {
        page.title = base.to_string();
    }
    page
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Loading(String),
    Error(String),
}

pub struct Browser {
    pub bar: String,
    pub editing: bool,
    pub current: Option<Fetched>,
    pub page: Page,
    pub rendered_width: usize,
    pub status: Status,
    pub back: Vec<String>,
    pub fwd: Vec<String>,
    pub scroll: usize,
    pub sel: Option<usize>,
    pub num: String,
    pub search: Option<String>,
    pub search_edit: Option<String>,
    pub match_i: usize,
    pub load_id: u64,
    pub area: Rect,
    pub body: Rect,
    /// Recent URLs (newest first) for `/browse` completion.
    pub visited: Vec<String>,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            bar: String::new(),
            editing: true,
            current: None,
            page: Page::default(),
            rendered_width: 0,
            status: Status::Idle,
            back: vec![],
            fwd: vec![],
            scroll: 0,
            sel: None,
            num: String::new(),
            search: None,
            search_edit: None,
            match_i: 0,
            load_id: 0,
            area: Rect::default(),
            body: Rect::default(),
            visited: vec![],
        }
    }

    pub fn url(&self) -> Option<&str> {
        self.current.as_ref().map(|f| f.url())
    }

    fn rerender(&mut self, width: usize) {
        if let Some(f) = &self.current {
            self.page = render(f, width);
            self.rendered_width = width;
        }
    }

    pub fn matches(&self) -> Vec<usize> {
        let Some(q) = self.search.as_deref().filter(|q| !q.is_empty()) else {
            return vec![];
        };
        let q = q.to_lowercase();
        self.page
            .plain
            .iter()
            .enumerate()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect()
    }

    fn view_h(&self) -> usize {
        self.body.height.max(1) as usize
    }

    fn max_scroll(&self) -> usize {
        self.page.lines.len().saturating_sub(self.view_h())
    }

    pub fn scroll_by(&mut self, d: isize) {
        let s = self.scroll as isize + d;
        self.scroll = s.clamp(0, self.max_scroll() as isize) as usize;
    }

    fn show_line(&mut self, line: usize) {
        if line < self.scroll || line >= self.scroll + self.view_h() {
            self.scroll = line
                .saturating_sub(self.view_h() / 3)
                .min(self.max_scroll());
        }
    }

    fn link_line(&self, idx: usize) -> Option<usize> {
        self.page
            .hits
            .iter()
            .position(|h| h.iter().any(|(_, _, i)| *i == idx))
    }

    /// Tab / Shift-Tab: step the selected link.
    pub fn cycle_link(&mut self, d: isize) {
        let n = self.page.links.len();
        if n == 0 {
            return;
        }
        let next = match self.sel {
            None if d > 0 => 0,
            None => n - 1,
            Some(i) => (i as isize + d).rem_euclid(n as isize) as usize,
        };
        self.sel = Some(next);
        if let Some(l) = self.link_line(next) {
            self.show_line(l);
        }
    }

    pub fn step_match(&mut self, d: isize) {
        let m = self.matches();
        if m.is_empty() {
            return;
        }
        self.match_i = (self.match_i as isize + d).rem_euclid(m.len() as isize) as usize;
        self.show_line(m[self.match_i]);
    }

    /// Link index at a screen position.
    pub fn link_at(&self, col: u16, row: u16) -> Option<usize> {
        if !self.body.contains(Position::new(col, row)) {
            return None;
        }
        let line = self.scroll + (row - self.body.y) as usize;
        let c = (col - self.body.x) as usize;
        self.page
            .hits
            .get(line)?
            .iter()
            .find(|(a, b, _)| c >= *a && c < *b)
            .map(|(_, _, i)| *i)
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

/// What a key in the browser asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Close,
    Go(String),
    Follow(usize),
    Back,
    Forward,
    Reload,
    OpenSystem,
    CopyUrl,
    Download,
}

pub fn key(b: &mut Browser, k: KeyEvent) -> Action {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    if let Some(q) = b.search_edit.as_mut() {
        match k.code {
            KeyCode::Esc => b.search_edit = None,
            KeyCode::Enter => {
                b.search = b.search_edit.take();
                b.match_i = 0;
                let m = b.matches();
                if let Some(&l) = m.first() {
                    b.show_line(l);
                }
            }
            KeyCode::Backspace => {
                q.pop();
            }
            KeyCode::Char(c) if !ctrl => q.push(c),
            _ => {}
        }
        return Action::None;
    }
    if b.editing {
        match k.code {
            KeyCode::Esc => {
                if b.current.is_none() {
                    return Action::Close;
                }
                b.editing = false;
                b.bar = bar_url(b.url().unwrap_or_default());
            }
            KeyCode::Enter => {
                let t = b.bar.trim();
                let t = t
                    .strip_prefix(":open ")
                    .or_else(|| t.strip_prefix("open "))
                    .unwrap_or(t)
                    .trim();
                if t.is_empty() {
                    return Action::None;
                }
                let url = if t.contains("://") {
                    t.to_string()
                } else {
                    format!("https://{t}")
                };
                b.editing = false;
                return Action::Go(url);
            }
            KeyCode::Backspace => {
                b.bar.pop();
            }
            KeyCode::Char('u') if ctrl => b.bar.clear(),
            KeyCode::Char(c) if !ctrl => b.bar.push(c),
            _ => {}
        }
        return Action::None;
    }
    let page = b.view_h() as isize;
    match k.code {
        KeyCode::Esc if !b.num.is_empty() || b.sel.is_some() => {
            b.num.clear();
            b.sel = None;
        }
        KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
        KeyCode::Char('l') if ctrl => {
            b.editing = true;
            b.bar = b.url().unwrap_or_default().to_string();
        }
        KeyCode::Char('e') => {
            b.editing = true;
            b.bar = b.url().unwrap_or_default().to_string();
        }
        KeyCode::Char(':') => {
            b.editing = true;
            b.bar.clear();
        }
        KeyCode::Char('d') if ctrl => b.scroll_by(page / 2),
        KeyCode::Char('u') if ctrl => b.scroll_by(-page / 2),
        KeyCode::Char('j') | KeyCode::Down => b.scroll_by(1),
        KeyCode::Char('k') | KeyCode::Up => b.scroll_by(-1),
        KeyCode::PageDown | KeyCode::Char(' ') => b.scroll_by(page - 1),
        KeyCode::PageUp => b.scroll_by(-(page - 1)),
        KeyCode::Char('g') | KeyCode::Home => b.scroll = 0,
        KeyCode::Char('G') | KeyCode::End => b.scroll = b.max_scroll(),
        KeyCode::Tab => b.cycle_link(1),
        KeyCode::BackTab => b.cycle_link(-1),
        KeyCode::Char(c) if c.is_ascii_digit() => b.num.push(c),
        KeyCode::Backspace if !b.num.is_empty() => {
            b.num.pop();
        }
        KeyCode::Enter => {
            if let Ok(n) = b.num.parse::<usize>() {
                b.num.clear();
                if n >= 1 && n <= b.page.links.len() {
                    return Action::Follow(n - 1);
                }
            } else if let Some(i) = b.sel {
                return Action::Follow(i);
            }
        }
        KeyCode::Char('H') => return Action::Back,
        KeyCode::Char('L') => return Action::Forward,
        KeyCode::Left if alt => return Action::Back,
        KeyCode::Right if alt => return Action::Forward,
        KeyCode::Backspace => return Action::Back,
        KeyCode::Char('r') => return Action::Reload,
        KeyCode::Char('/') => b.search_edit = Some(String::new()),
        KeyCode::Char('n') => b.step_match(1),
        KeyCode::Char('N') => b.step_match(-1),
        KeyCode::Char('o') => return Action::OpenSystem,
        KeyCode::Char('y') => return Action::CopyUrl,
        KeyCode::Char('d') => return Action::Download,
        _ => {}
    }
    Action::None
}

pub fn mouse(b: &mut Browser, m: MouseEvent) -> Action {
    match m.kind {
        MouseEventKind::ScrollDown => b.scroll_by(3),
        MouseEventKind::ScrollUp => b.scroll_by(-3),
        MouseEventKind::Down(MouseButton::Left) => {
            if !b.area.contains(Position::new(m.column, m.row)) {
                return Action::Close;
            }
            if m.row == b.area.y + 1 {
                b.editing = true;
                b.bar = b.url().unwrap_or_default().to_string();
                return Action::None;
            }
            if let Some(i) = b.link_at(m.column, m.row) {
                return Action::Follow(i);
            }
        }
        _ => {}
    }
    Action::None
}

pub fn draw(f: &mut Frame, b: &mut Browser, ssh: bool) {
    let screen = f.area();
    let w = screen.width.saturating_sub(4).max(20).min(screen.width);
    let h = screen.height.saturating_sub(2).max(8).min(screen.height);
    let area = Rect {
        x: screen.x + (screen.width - w) / 2,
        y: screen.y + (screen.height - h) / 2,
        width: w,
        height: h,
    };
    b.area = area;
    f.render_widget(Clear, area);
    let title = match &b.status {
        Status::Loading(u) => format!(" Browser · loading {} … ", host(u)),
        Status::Error(_) => " Browser · error ".to_string(),
        Status::Idle if b.page.title.is_empty() => " Browser ".to_string(),
        Status::Idle => format!(
            " {} ",
            b.page
                .title
                .chars()
                .take(w as usize / 2)
                .collect::<String>()
        ),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(rs(theme::browser_frame()))
        .title(Span::styled(title, rs(theme::browser_status())));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    // URL bar.
    let bar_text = if b.editing {
        format!(" {}▏", b.bar)
    } else {
        // Presigned links never show their signature.
        format!(" {}", bar_url(b.url().unwrap_or("")))
    };
    let mut bar = vec![sp(rs(theme::browser_url()), bar_text)];
    if ssh {
        bar.push(sp(rs(theme::ssh_indicator()), " SSH "));
    }
    f.render_widget(Paragraph::new(Line::from(bar)), Rect { height: 1, ..inner });
    let body = Rect {
        y: inner.y + 1,
        height: inner.height - 2,
        ..inner
    };
    b.body = body;
    let width = body.width as usize;
    if b.rendered_width != width {
        b.rerender(width);
    }
    let matches = b.matches();
    let lines: Vec<Line<'static>> = match &b.status {
        Status::Error(e) => vec![Line::from(sp(
            rs(theme::attach_chip_error()),
            format!("Couldn't load: {e}"),
        ))],
        _ if b.current.is_none() => vec![Line::raw("Type a URL and press Enter (Esc closes).")],
        _ => b
            .page
            .lines
            .iter()
            .enumerate()
            .skip(b.scroll)
            .take(body.height as usize)
            .map(|(i, l)| {
                let sel_line = b
                    .sel
                    .is_some_and(|s| b.page.hits[i].iter().any(|(_, _, x)| *x == s));
                if matches.contains(&i) || sel_line {
                    Line::from(
                        l.spans
                            .iter()
                            .map(|s| sp(rs(theme::selected()), s.content.to_string()))
                            .collect::<Vec<_>>(),
                    )
                } else {
                    l.clone()
                }
            })
            .collect(),
    };
    f.render_widget(Paragraph::new(lines), body);
    // Footer: keys, link number being typed, search.
    let foot = if let Some(q) = &b.search_edit {
        format!(" /{q}▏")
    } else if b.page.binary && b.status == Status::Idle {
        " d download · o open with the system app · H back · q close".to_string()
    } else if !b.num.is_empty() {
        format!(" link {}… (Enter follows)", b.num)
    } else {
        let m = if matches.is_empty() {
            String::new()
        } else {
            format!(" · match {}/{}", b.match_i + 1, matches.len())
        };
        format!(
            " {} links{m} · Tab/n Enter follow · H/L back/fwd · r reload · / search · e URL · o system · y copy · q close",
            b.page.links.len()
        )
    };
    f.render_widget(
        Paragraph::new(sp(rs(theme::key_hint()), foot)),
        Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        },
    );
}

fn host(u: &str) -> String {
    url::Url::parse(u)
        .ok()
        .and_then(|x| x.host_str().map(str::to_string))
        .unwrap_or_else(|| u.to_string())
}

impl App {
    /// Open the browser modal (on `url`, or a blank URL bar).
    pub fn open_browser(&mut self, url: Option<&str>) {
        if self.browser.is_none() {
            let mut b = Browser::new();
            b.visited = self.browser_visited.clone();
            self.browser = Some(b);
        }
        match url {
            Some(u) => self.browser_go(u.to_string(), true),
            None => {
                if let Some(b) = self.browser.as_mut() {
                    b.editing = true;
                    b.bar.clear();
                }
            }
        }
    }

    fn browser_go(&mut self, url: String, push: bool) {
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        if push
            && let Some(cur) = b.url().map(str::to_string)
            && cur != url
        {
            b.back.push(cur);
            if b.back.len() > HISTORY {
                b.back.remove(0);
            }
            b.fwd.clear();
        }
        b.load_id += 1;
        let id = b.load_id;
        b.status = Status::Loading(url.clone());
        b.editing = false;
        b.bar = url.clone();
        self.spawn(async move {
            let result = fetch(url.clone()).await;
            Event::BrowserLoaded { id, url, result }
        });
    }

    pub fn on_browser_loaded(&mut self, id: u64, url: String, result: Result<Fetched, String>) {
        let Some(b) = self.browser.as_mut() else {
            return;
        };
        if b.load_id != id {
            return;
        }
        match result {
            Ok(f) => {
                let final_url = f.url().to_string();
                b.current = Some(f);
                b.rendered_width = 0;
                b.scroll = 0;
                b.sel = None;
                b.search = None;
                b.status = Status::Idle;
                b.bar = final_url.clone();
                self.browser_visited.retain(|u| *u != final_url);
                self.browser_visited.insert(0, final_url);
                self.browser_visited.truncate(50);
                b.visited = self.browser_visited.clone();
            }
            Err(e) => {
                b.status = Status::Error(e);
                b.bar = url;
            }
        }
    }

    fn browser_act(&mut self, a: Action) {
        match a {
            Action::None => {}
            Action::Close => self.browser = None,
            Action::Go(u) => self.browser_go(u, true),
            Action::Follow(i) => {
                let Some(link) = self
                    .browser
                    .as_ref()
                    .and_then(|b| b.page.links.get(i).cloned())
                else {
                    return;
                };
                if link.image {
                    self.view_image_items(
                        vec![crate::markdown::Openable {
                            label: crate::download::save_name(&link.url),
                            url: link.url,
                            image: true,
                        }],
                        0,
                    );
                } else if link.url.starts_with("http://") || link.url.starts_with("https://") {
                    self.browser_go(link.url, true);
                } else {
                    self.open_system(&link.url);
                }
            }
            Action::Back => {
                if let Some(b) = self.browser.as_mut()
                    && let Some(u) = b.back.pop()
                {
                    if let Some(cur) = b.url().map(str::to_string) {
                        b.fwd.push(cur);
                    }
                    self.browser_go(u, false);
                }
            }
            Action::Forward => {
                if let Some(b) = self.browser.as_mut()
                    && let Some(u) = b.fwd.pop()
                {
                    if let Some(cur) = b.url().map(str::to_string) {
                        b.back.push(cur);
                    }
                    self.browser_go(u, false);
                }
            }
            Action::Reload => {
                if let Some(u) = self
                    .browser
                    .as_ref()
                    .and_then(|b| b.url().map(str::to_string))
                {
                    self.browser_go(u, false);
                }
            }
            Action::OpenSystem => {
                if let Some(u) = self
                    .browser
                    .as_ref()
                    .and_then(|b| b.url().map(str::to_string))
                {
                    self.open_system(&u);
                }
            }
            Action::CopyUrl => {
                if let Some(u) = self.browser.as_ref().and_then(|b| b.url().map(bar_url)) {
                    self.copy_text(u);
                }
            }
            Action::Download => {
                if let Some(u) = self
                    .browser
                    .as_ref()
                    .and_then(|b| b.url().map(str::to_string))
                {
                    self.open_download(&u);
                }
            }
        }
    }

    pub fn browser_key(&mut self, k: KeyEvent) {
        if let Some(b) = self.browser.as_mut() {
            let a = key(b, k);
            self.browser_act(a);
        }
    }

    pub fn browser_mouse(&mut self, m: MouseEvent) {
        if let Some(b) = self.browser.as_mut() {
            let a = mouse(b, m);
            self.browser_act(a);
        }
    }
}

fn sp(style: Style, text: impl Into<String>) -> Span<'static> {
    Span::styled(text.into(), style)
}

fn rs(s: ratatui_lipgloss::Style) -> Style {
    Style::from(&s)
}

/// The URL bar text: signed (presigned) URLs show the host only.
pub fn bar_url(u: &str) -> String {
    if u.contains("Signature=") || u.contains("X-Amz-") {
        crate::links::shown(u)
    } else {
        u.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<html><head><title>Test &amp; Page</title>
        <script>alert('x')</script><style>body{}</style></head>
        <body><h1>Hello</h1><p>See <a href="/a">first link</a> and <a href="https://x.io/b">second</a>.</p>
        <ul><li>one</li><li><strong>two</strong></li></ul>
        <pre>code here</pre><blockquote>quoted</blockquote>
        <img src="pic.png" alt="cat"><p><a href="/a">first again</a></p></body></html>"#;

    #[test]
    fn renders_html_with_numbered_links_and_no_scripts() {
        let p = render_html(HTML, "https://ex.com/dir/page", 60);
        assert_eq!(p.title, "Test & Page");
        let all = p.plain.join("\n");
        assert!(!all.contains("alert"), "{all}");
        assert!(!all.contains("body{}"));
        assert!(all.contains("Hello"));
        assert!(all.contains("first link[1]"), "{all}");
        assert!(all.contains("second[2]"));
        assert!(all.contains("[🖼 cat]"), "{all}");
        assert!(all.contains("code here"));
        assert!(all.contains("quoted"));
        assert_eq!(p.links[0].url, "https://ex.com/a");
        assert_eq!(p.links[1].url, "https://x.io/b");
        assert!(
            p.links
                .iter()
                .any(|l| l.image && l.url == "https://ex.com/dir/pic.png")
        );
        assert_eq!(p.hits.len(), p.lines.len());
        // Every hit points at an existing link.
        assert!(
            p.hits
                .iter()
                .flatten()
                .all(|(a, b, i)| a < b && *i < p.links.len())
        );
    }

    #[test]
    fn plain_json_and_markdown_pages() {
        let j = render(
            &Fetched::Text {
                url: "https://a/x.json".into(),
                content_type: "application/json".into(),
                body: r#"{"a":1}"#.into(),
            },
            40,
        );
        assert!(j.plain.iter().any(|l| l.contains("\"a\": 1")));
        let t = render(
            &Fetched::Text {
                url: "https://a/n.txt".into(),
                content_type: "text/plain".into(),
                body: "see https://b.io/z".into(),
            },
            40,
        );
        assert_eq!(t.links.len(), 1);
        assert!(t.plain.last().unwrap().starts_with("[1] https://b.io/z"));
        let bin = render(
            &Fetched::Binary {
                url: "https://a/f.zip".into(),
                content_type: "application/zip".into(),
                size: Some(10),
            },
            40,
        );
        assert!(bin.binary);
    }

    fn loaded(url: &str, html: &str) -> Browser {
        let mut b = Browser::new();
        b.current = Some(Fetched::Text {
            url: url.into(),
            content_type: "text/html".into(),
            body: html.into(),
        });
        b.editing = false;
        b.body = Rect::new(0, 0, 60, 5);
        b.rerender(60);
        b
    }

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn keys_number_tab_search_and_history_actions() {
        let mut b = loaded("https://ex.com/", HTML);
        assert_eq!(key(&mut b, k(KeyCode::Char('2'))), Action::None);
        assert_eq!(key(&mut b, k(KeyCode::Enter)), Action::Follow(1));
        assert_eq!(key(&mut b, k(KeyCode::Tab)), Action::None);
        assert_eq!(b.sel, Some(0));
        assert_eq!(key(&mut b, k(KeyCode::Enter)), Action::Follow(0));
        key(&mut b, k(KeyCode::BackTab));
        assert_eq!(b.sel, Some(b.page.links.len() - 1));
        key(&mut b, k(KeyCode::Char('/')));
        for c in "quoted".chars() {
            key(&mut b, k(KeyCode::Char(c)));
        }
        key(&mut b, k(KeyCode::Enter));
        assert_eq!(b.matches().len(), 1);
        assert_eq!(key(&mut b, k(KeyCode::Char('H'))), Action::Back);
        assert_eq!(
            key(&mut b, KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)),
            Action::Forward
        );
        assert_eq!(key(&mut b, k(KeyCode::Char('r'))), Action::Reload);
        assert_eq!(key(&mut b, k(KeyCode::Char('o'))), Action::OpenSystem);
        key(&mut b, k(KeyCode::Char('e')));
        assert!(b.editing);
        b.bar = "example.org".into();
        assert_eq!(
            key(&mut b, k(KeyCode::Enter)),
            Action::Go("https://example.org".into())
        );
        key(&mut b, k(KeyCode::Char(':')));
        b.bar = "open http://a.b/c".into();
        assert_eq!(
            key(&mut b, k(KeyCode::Enter)),
            Action::Go("http://a.b/c".into())
        );
        assert_eq!(key(&mut b, k(KeyCode::Char('q'))), Action::Close);
    }

    #[test]
    fn navigation_history_back_and_forward() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        app.open_browser(None);
        // Simulate loads without the network.
        for u in ["https://a.test/", "https://b.test/", "https://c.test/"] {
            app.browser_go(u.into(), true);
            let id = app.browser.as_ref().unwrap().load_id;
            app.on_browser_loaded(
                id,
                u.into(),
                Ok(Fetched::Text {
                    url: u.into(),
                    content_type: "text/plain".into(),
                    body: "x".into(),
                }),
            );
        }
        let b = app.browser.as_ref().unwrap();
        assert_eq!(b.back, vec!["https://a.test/", "https://b.test/"]);
        app.browser_act(Action::Back);
        let b = app.browser.as_ref().unwrap();
        assert_eq!(b.fwd, vec!["https://c.test/"]);
        assert_eq!(b.status, Status::Loading("https://b.test/".into()));
        assert_eq!(app.browser_visited[0], "https://c.test/");
        app.browser_act(Action::Close);
        assert!(app.browser.is_none());
    }
}
