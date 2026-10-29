//! Download modal: save an image or file from a reply to disk (`[⤓]`, `/download`, vim `d`).
//!
//! The modal has a path field (prefilled `~/Downloads/<name>`, Tab completes paths) and a
//! folder browser below it. Existing files and missing folders are confirmed first. The
//! transfer runs in the background, capped at `downloads.max_mb`.

use crate::app::{App, Event};
use crate::markdown::{Openable, file_name, human_size, openables};
use crate::theme;
use ratatui::Frame;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use std::path::{Path, PathBuf};

/// The download button drawn before each downloadable item under a reply.
pub const BUTTON: &str = "[⤓]";

/// Images and links that point at a file (not a web page), in order.
pub fn downloadables(text: &str) -> Vec<Openable> {
    openables(text)
        .into_iter()
        .filter(|o| o.image || looks_like_file(&o.url))
        .collect()
}

/// A URL or path whose last segment has a file extension (and isn't an HTML page).
pub fn looks_like_file(u: &str) -> bool {
    let path = match u.split_once("://") {
        Some((scheme, rest)) => {
            if !matches!(scheme, "http" | "https" | "file") {
                return false;
            }
            if scheme == "file" {
                rest
            } else {
                match rest.split_once('/') {
                    Some((_, p)) => p,
                    None => return false,
                }
            }
        }
        None if u.starts_with('/') || u.starts_with("~/") => u,
        None => return false,
    };
    let name = file_name(path);
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    !stem.is_empty()
        && (1..=5).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && !matches!(
            ext.as_str(),
            "html" | "htm" | "php" | "asp" | "aspx" | "jsp" | "cgi"
        )
}

/// Expand a leading `~` to the home directory.
pub fn expand(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_default();
    }
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(h) = dirs::home_dir()
    {
        return h.join(rest);
    }
    PathBuf::from(p)
}

/// A file name safe to save under: the URL's last segment, or `download`.
pub fn save_name(url: &str) -> String {
    let n: String = file_name(url)
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '\0'))
        .collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() { "download".into() } else { n }
}

/// `~/Downloads/<name>` when that folder exists, else `~/<name>` (shown with `~`).
pub fn default_path(url: &str) -> String {
    let name = save_name(url);
    let dl = dirs::home_dir().map(|h| h.join("Downloads"));
    if dl.is_some_and(|d| d.is_dir()) {
        format!("~/Downloads/{name}")
    } else {
        format!("~/{name}")
    }
}

/// Path completions for `token` (fuzzy on the last segment; folders end in `/`). The
/// returned strings keep the token's own prefix (`~/…`, relative or absolute).
pub fn path_candidates(token: &str) -> Vec<String> {
    let (dir_part, query) = match token.rfind('/') {
        Some(i) => (&token[..=i], &token[i + 1..]),
        None => ("", token),
    };
    let dir = if dir_part.is_empty() {
        PathBuf::from(".")
    } else {
        expand(dir_part)
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') && !query.starts_with('.') {
                return None;
            }
            let is_dir = e.path().is_dir();
            Some(if is_dir { format!("{n}/") } else { n })
        })
        .collect();
    names.sort();
    let ranked: Vec<String> = if query.is_empty() {
        names
    } else {
        crate::fuzzy::rank(query, &names)
            .into_iter()
            .map(|(i, _)| names[i].clone())
            .collect()
    };
    ranked
        .into_iter()
        .take(200)
        .map(|n| format!("{dir_part}{n}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Overwrite,
    MakeDirs,
}

pub struct DownloadModal {
    pub url: String,
    pub path: String,
    /// Tab cycling: (candidates, index, the text before the first Tab).
    pub comp: Option<(Vec<String>, usize, String)>,
    /// Folder browser: entries of the path's folder (name, is_dir) and the cursor when the
    /// browser has focus.
    pub entries: Vec<(String, bool)>,
    pub browse: Option<usize>,
    pub top: usize,
    pub confirm: Option<Confirm>,
    pub area: Rect,
    pub list: Rect,
}

impl DownloadModal {
    pub fn new(url: String) -> Self {
        let path = default_path(&url);
        let mut m = Self {
            url,
            path,
            comp: None,
            entries: vec![],
            browse: None,
            top: 0,
            confirm: None,
            area: Rect::default(),
            list: Rect::default(),
        };
        m.refresh();
        m
    }

    /// The folder the path is in (the path itself when it ends in `/`).
    fn dir_text(&self) -> String {
        match self.path.rfind('/') {
            Some(i) => self.path[..=i].to_string(),
            None => String::new(),
        }
    }

    fn refresh(&mut self) {
        let d = self.dir_text();
        let dir = if d.is_empty() {
            PathBuf::from(".")
        } else {
            expand(&d)
        };
        let mut e: Vec<(String, bool)> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| {
                        (
                            e.file_name().to_string_lossy().to_string(),
                            e.path().is_dir(),
                        )
                    })
                    .filter(|(n, _)| !n.starts_with('.'))
                    .collect()
            })
            .unwrap_or_default();
        e.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.entries = e;
        self.top = 0;
        if let Some(c) = self.browse.as_mut() {
            *c = 0;
        }
    }

    fn file_part(&self) -> String {
        let f = match self.path.rfind('/') {
            Some(i) => &self.path[i + 1..],
            None => self.path.as_str(),
        };
        if f.is_empty() {
            save_name(&self.url)
        } else {
            f.to_string()
        }
    }

    fn set_dir(&mut self, dir: &str) {
        let f = self.file_part();
        self.path = format!("{dir}{f}");
        self.refresh();
    }

    fn parent(&mut self) {
        let d = self.dir_text();
        let trimmed = d.trim_end_matches('/');
        let up = if trimmed.is_empty() {
            "/".to_string()
        } else if trimmed == "~" {
            match dirs::home_dir().and_then(|h| h.parent().map(|p| p.to_path_buf())) {
                Some(p) => format!("{}/", p.display()).replace("//", "/"),
                None => return,
            }
        } else {
            match trimmed.rfind('/') {
                Some(i) => trimmed[..=i].to_string(),
                None => String::new(),
            }
        };
        self.set_dir(&up);
    }

    fn tab(&mut self, back: bool) {
        match self.comp.as_mut() {
            Some((items, i, orig)) => {
                // n items + the original text.
                let n = items.len() + 1;
                *i = if back { (*i + n - 1) % n } else { (*i + 1) % n };
                self.path = items.get(*i).cloned().unwrap_or_else(|| orig.clone());
            }
            None => {
                let items = path_candidates(&self.path);
                if items.is_empty() {
                    return;
                }
                let orig = self.path.clone();
                self.path = items[0].clone();
                self.comp = Some((items, 0, orig));
            }
        }
        self.refresh_keep_comp();
    }

    fn refresh_keep_comp(&mut self) {
        let c = self.comp.take();
        self.refresh();
        self.comp = c;
    }

    /// Resolve the target file: a folder (or `…/`) gets the file name appended.
    pub fn target(&self) -> PathBuf {
        let p = expand(&self.path);
        if self.path.ends_with('/') || p.is_dir() {
            p.join(save_name(&self.url))
        } else {
            p
        }
    }
}

pub enum Outcome {
    None,
    Close,
    /// Start: (target, create missing parent folders).
    Start(PathBuf, bool),
}

fn submit(m: &mut DownloadModal) -> Outcome {
    let t = m.target();
    if m.path.trim().is_empty() {
        return Outcome::None;
    }
    let parent_missing = t
        .parent()
        .is_some_and(|p| !p.as_os_str().is_empty() && !p.is_dir());
    if parent_missing {
        m.confirm = Some(Confirm::MakeDirs);
    } else if t.exists() {
        m.confirm = Some(Confirm::Overwrite);
    } else {
        return Outcome::Start(t, false);
    }
    Outcome::None
}

pub fn key(m: &mut DownloadModal, k: KeyEvent) -> Outcome {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    if let Some(c) = m.confirm {
        return match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                m.confirm = None;
                Outcome::Start(m.target(), c == Confirm::MakeDirs)
            }
            _ => {
                m.confirm = None;
                Outcome::None
            }
        };
    }
    if let Some(cur) = m.browse {
        let n = m.entries.len();
        match k.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::Char('i') | KeyCode::Char('q') => {
                m.browse = None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if cur == 0 {
                    m.browse = None
                } else {
                    m.browse = Some(cur - 1)
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                m.browse = Some((cur + 1).min(n.saturating_sub(1)))
            }
            KeyCode::Char('g') => m.browse = Some(0),
            KeyCode::Char('G') => m.browse = Some(n.saturating_sub(1)),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => m.parent(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some((name, is_dir)) = m.entries.get(cur).cloned() {
                    let d = m.dir_text();
                    if is_dir {
                        m.set_dir(&format!("{d}{name}/"));
                    } else {
                        m.path = format!("{d}{name}");
                        m.browse = None;
                        if k.code == KeyCode::Enter {
                            return submit(m);
                        }
                    }
                }
            }
            _ => {}
        }
        return Outcome::None;
    }
    match k.code {
        KeyCode::Esc => {
            if let Some((_, _, orig)) = m.comp.take() {
                m.path = orig;
                m.refresh();
            } else {
                return Outcome::Close;
            }
        }
        KeyCode::Enter => {
            m.comp = None;
            return submit(m);
        }
        KeyCode::Tab => m.tab(false),
        KeyCode::BackTab => m.tab(true),
        KeyCode::Down => {
            m.comp = None;
            if !m.entries.is_empty() {
                m.browse = Some(0);
            }
        }
        KeyCode::Backspace => {
            m.comp = None;
            m.path.pop();
            m.refresh();
        }
        KeyCode::Char('u') if ctrl => {
            m.comp = None;
            m.path.clear();
            m.refresh();
        }
        KeyCode::Char(c) if !ctrl => {
            m.comp = None;
            m.path.push(c);
            if c == '/' {
                m.refresh();
            }
        }
        _ => {}
    }
    Outcome::None
}

pub fn mouse(m: &mut DownloadModal, ev: MouseEvent) -> Outcome {
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            let at = Position::new(ev.column, ev.row);
            if !m.area.contains(at) {
                return Outcome::Close;
            }
            if m.list.contains(at) {
                let i = (ev.row - m.list.y) as usize + m.top;
                if let Some((name, is_dir)) = m.entries.get(i).cloned() {
                    let d = m.dir_text();
                    if is_dir {
                        m.set_dir(&format!("{d}{name}/"));
                    } else {
                        m.path = format!("{d}{name}");
                    }
                }
            }
        }
        MouseEventKind::ScrollDown => m.top = (m.top + 1).min(m.entries.len().saturating_sub(1)),
        MouseEventKind::ScrollUp => m.top = m.top.saturating_sub(1),
        _ => {}
    }
    Outcome::None
}

pub fn draw(f: &mut Frame, m: &mut DownloadModal) {
    let area = crate::history::centered(f.area(), 70, 60);
    m.area = area;
    f.render_widget(Clear, area);
    let hint = match (m.confirm, m.browse) {
        (Some(_), _) => "y / Enter confirms · any other key goes back",
        (None, Some(_)) => "↑↓/jk move · Enter/l opens · h/← parent · Esc back to the path",
        (None, None) => "Tab completes · ↓ browse folders · Enter saves · Esc cancels",
    };
    let block = theme::block(&theme::help_frame())
        .title(theme::help_title().span(" Download "))
        .title_bottom(format!(" {hint} "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 4 {
        return;
    }
    let row = |y: u16| Rect {
        y: inner.y + y,
        height: 1,
        ..inner
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            theme::key_hint().span("from: "),
            theme::modal_text().span(m.url.clone()),
        ])),
        row(0),
    );
    let caret = if m.browse.is_none() && m.confirm.is_none() {
        "▏"
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            theme::key_hint().span("save to: "),
            theme::download_path().span(format!("{}{caret}", m.path)),
        ])),
        row(1),
    );
    let status = match m.confirm {
        Some(Confirm::Overwrite) => format!("{} exists. Overwrite? (y/n)", m.target().display()),
        Some(Confirm::MakeDirs) => format!(
            "{} doesn't exist. Create it? (y/n)",
            m.target()
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
        None => match &m.comp {
            Some((items, i, _)) => {
                format!("completion {}/{}", (*i + 1).min(items.len()), items.len())
            }
            None => format!("→ {}", m.target().display()),
        },
    };
    let st = if m.confirm.is_some() {
        theme::toast(crate::notify::NoticeKind::Warning)
    } else {
        theme::key_hint()
    };
    f.render_widget(Paragraph::new(Line::from(st.span(status))), row(2));
    let list = Rect {
        x: inner.x,
        y: inner.y + 3,
        width: inner.width,
        height: inner.height - 3,
    };
    m.list = list;
    let rows = list.height as usize;
    if let Some(c) = m.browse {
        if c < m.top {
            m.top = c;
        } else if c >= m.top + rows {
            m.top = c + 1 - rows;
        }
    }
    let lines: Vec<Line> = m
        .entries
        .iter()
        .enumerate()
        .skip(m.top)
        .take(rows)
        .map(|(i, (n, d))| {
            let text = if *d {
                format!(" {n}/")
            } else {
                format!(" {n}")
            };
            let st = if Some(i) == m.browse {
                theme::selected()
            } else if *d {
                theme::download_dir()
            } else {
                theme::modal_text()
            };
            Line::from(st.span(text))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list);
}

/// Copy (local) or stream (http/https) `url` to `dest`, at most `cap` bytes. Writes to
/// `<dest>.part` first and renames, so a failed transfer never leaves a partial file.
pub async fn fetch(url: String, dest: PathBuf, make_dirs: bool, cap: u64) -> Result<u64, String> {
    use tokio::io::AsyncWriteExt;
    if make_dirs && let Some(p) = dest.parent() {
        tokio::fs::create_dir_all(p)
            .await
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let too_big = |n: u64| {
        format!(
            "{} is larger than the {} cap (downloads.max_mb)",
            human_size(n),
            human_size(cap)
        )
    };
    let part = part_path(&dest);
    let local = url.strip_prefix("file://").unwrap_or(&url);
    if !local.contains("://") {
        let src = expand(local);
        let m = tokio::fs::metadata(&src)
            .await
            .map_err(|e| format!("{}: {e}", src.display()))?;
        if m.len() > cap {
            return Err(too_big(m.len()));
        }
        let n = tokio::fs::copy(&src, &part)
            .await
            .map_err(|e| format!("{}: {e}", part.display()))?;
        tokio::fs::rename(&part, &dest)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(n);
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("unsupported location".into());
    }
    let mut resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    if let Some(n) = resp.content_length().filter(|n| *n > cap) {
        return Err(too_big(n));
    }
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| format!("{}: {e}", part.display()))?;
    let mut total = 0u64;
    let result: Result<(), String> = async {
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            total += chunk.len() as u64;
            if total > cap {
                return Err(too_big(total));
            }
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        file.flush().await.map_err(|e| e.to_string())
    }
    .await;
    drop(file);
    if let Err(e) = result {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::rename(&part, &dest)
        .await
        .map_err(|e| e.to_string())?;
    Ok(total)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

impl App {
    /// Downloadable items in the active chat's replies, newest reply first.
    pub fn chat_downloads(&self) -> Vec<Openable> {
        let mut out: Vec<Openable> = vec![];
        for e in self
            .feed()
            .iter()
            .rev()
            .filter(|e| e.kind == crate::app::EntryKind::Bot)
        {
            for o in downloadables(&e.text) {
                if !out.iter().any(|x| x.url == o.url) {
                    out.push(o);
                }
            }
        }
        out
    }

    pub fn open_download(&mut self, url: &str) {
        self.download = Some(DownloadModal::new(url.to_string()));
    }

    fn finish_download(&mut self, o: Outcome) {
        match o {
            Outcome::None => {}
            Outcome::Close => self.download = None,
            Outcome::Start(dest, mk) => {
                let Some(m) = self.download.take() else {
                    return;
                };
                let cap = self.config.download_max_mb * 1024 * 1024;
                let url = m.url;
                self.info(format!(
                    "Downloading {} → {}…",
                    save_name(&url),
                    dest.display()
                ));
                self.spawn(async move {
                    let result = fetch(url.clone(), dest.clone(), mk, cap).await;
                    Event::Downloaded { url, dest, result }
                });
            }
        }
    }

    pub fn on_downloaded(&mut self, url: String, dest: PathBuf, result: Result<u64, String>) {
        match result {
            Ok(n) => self.info(format!(
                "Saved {} ({}) to {}",
                save_name(&url),
                human_size(n),
                dest.display()
            )),
            Err(e) => self.error(format!("Download of {} failed: {e}", save_name(&url))),
        }
    }

    pub fn download_key(&mut self, k: KeyEvent) {
        if let Some(m) = self.download.as_mut() {
            let o = key(m, k);
            self.finish_download(o);
        }
    }

    pub fn download_mouse(&mut self, ev: MouseEvent) {
        if let Some(m) = self.download.as_mut() {
            let o = mouse(m, ev);
            self.finish_download(o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(c: KeyCode) -> KeyEvent {
        KeyEvent::new(c, KeyModifiers::NONE)
    }

    #[test]
    fn file_detection() {
        assert!(looks_like_file("https://x.io/a/report.pdf"));
        assert!(looks_like_file("https://x.io/a.zip?dl=1"));
        assert!(looks_like_file("/tmp/a.csv"));
        assert!(!looks_like_file("https://example.com"));
        assert!(!looks_like_file("https://example.com/"));
        assert!(!looks_like_file("https://x.io/page.html"));
        assert!(!looks_like_file("https://x.io/docs/intro"));
        let d = downloadables(
            "see ![c](https://x.io/c.png), [r](https://x.io/r.pdf) and https://x.io/",
        );
        let urls: Vec<_> = d.iter().map(|o| o.url.as_str()).collect();
        assert_eq!(urls, ["https://x.io/c.png", "https://x.io/r.pdf"]);
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(save_name("https://x.io/a/b.png?x=1"), "b.png");
        assert_eq!(save_name("https://x.io/"), "x.io");
        assert_eq!(save_name("https://x.io/..."), "download");
        let h = dirs::home_dir().unwrap();
        assert_eq!(expand("~/a/b"), h.join("a/b"));
        assert_eq!(expand("/a"), PathBuf::from("/a"));
        assert!(default_path("https://x.io/f.txt").ends_with("/f.txt"));
    }

    #[test]
    fn completion_browser_and_confirm() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().display().to_string();
        std::fs::create_dir(d.path().join("photos")).unwrap();
        std::fs::write(d.path().join("pic.png"), b"x").unwrap();
        let c = path_candidates(&format!("{base}/pho"));
        assert_eq!(c[0], format!("{base}/photos/"));

        let mut m = DownloadModal::new("/nowhere/pic.png".into());
        m.path = format!("{base}/pho");
        m.refresh();
        key(&mut m, k(KeyCode::Tab));
        assert_eq!(m.path, format!("{base}/photos/"));
        // Esc restores the typed text before closing.
        assert!(matches!(key(&mut m, k(KeyCode::Esc)), Outcome::None));
        assert_eq!(m.path, format!("{base}/pho"));

        // Browser: dirs first; Enter on a dir descends and keeps the file name.
        m.path = format!("{base}/pic.png");
        m.refresh();
        assert_eq!(m.entries[0], ("photos".to_string(), true));
        key(&mut m, k(KeyCode::Down));
        assert_eq!(m.browse, Some(0));
        key(&mut m, k(KeyCode::Char('l')));
        assert_eq!(m.path, format!("{base}/photos/pic.png"));
        key(&mut m, k(KeyCode::Char('h')));
        assert_eq!(m.path, format!("{base}/pic.png"));
        key(&mut m, k(KeyCode::Esc));
        assert_eq!(m.browse, None);

        // Existing file → overwrite confirm; n backs out, y starts.
        assert!(matches!(key(&mut m, k(KeyCode::Enter)), Outcome::None));
        assert_eq!(m.confirm, Some(Confirm::Overwrite));
        key(&mut m, k(KeyCode::Char('n')));
        assert_eq!(m.confirm, None);
        key(&mut m, k(KeyCode::Enter));
        match key(&mut m, k(KeyCode::Char('y'))) {
            Outcome::Start(p, mk) => {
                assert_eq!(p, d.path().join("pic.png"));
                assert!(!mk);
            }
            _ => panic!("expected start"),
        }

        // Missing folder → confirm creating it.
        m.path = format!("{base}/new/sub/");
        key(&mut m, k(KeyCode::Enter));
        assert_eq!(m.confirm, Some(Confirm::MakeDirs));
        match key(&mut m, k(KeyCode::Enter)) {
            Outcome::Start(p, mk) => {
                assert_eq!(p, d.path().join("new/sub/pic.png"));
                assert!(mk);
            }
            _ => panic!("expected start"),
        }
    }

    #[test]
    fn fetch_copies_local_with_cap() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("a.bin");
        std::fs::write(&src, vec![7u8; 1000]).unwrap();
        let dest = d.path().join("out/x/a.bin");
        let url = src.display().to_string();
        let n = rt
            .block_on(fetch(url.clone(), dest.clone(), true, 4096))
            .unwrap();
        assert_eq!(n, 1000);
        assert_eq!(std::fs::read(&dest).unwrap().len(), 1000);
        let e = rt
            .block_on(fetch(url, d.path().join("b.bin"), false, 10))
            .unwrap_err();
        assert!(e.contains("cap"), "{e}");
        assert!(!d.path().join("b.bin.part").exists());
    }

    #[test]
    fn button_click_and_command_open_modal_then_save_local_file() {
        use crate::app::{Entry, EntryKind};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "tok");
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("chart.png");
        std::fs::write(&src, b"png-bytes").unwrap();
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![Entry {
                kind: EntryKind::Bot,
                text: format!(
                    "Here: ![c]({}) and [notes](https://x.io/notes.pdf), see https://x.io/",
                    src.display()
                ),
            }],
        );
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let s = crate::ui::tests::screen_rows(&term).join("\n");
        assert!(s.contains("[⤓] chart.png  [⤓] notes.pdf"), "{s}");
        assert_eq!(app.download_hits.len(), 2);
        let h = app.download_hits[0].clone();
        app.click(h.x, h.y);
        let m = app.download.as_mut().expect("modal");
        assert_eq!(m.url, src.display().to_string());
        assert!(m.path.ends_with("/chart.png"));
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        let s = crate::ui::tests::screen_rows(&term).join("\n");
        assert!(s.contains("Download") && s.contains("save to:"), "{s}");
        // Save into a fresh folder: confirm creating it, then the copy lands there.
        let m = app.download.as_mut().unwrap();
        m.path = format!("{}/out/", d.path().display());
        app.download_key(k(KeyCode::Enter));
        assert_eq!(
            app.download.as_ref().unwrap().confirm,
            Some(Confirm::MakeDirs)
        );
        app.download_key(k(KeyCode::Char('y')));
        assert!(app.download.is_none());
        let out = d.path().join("out/chart.png");
        for _ in 0..200 {
            app.pump();
            if out.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(&out).unwrap(), b"png-bytes");
        // /download 2 picks the second item; /download list numbers them.
        app.input = "/download 2".into();
        app.submit();
        assert_eq!(app.download.as_ref().unwrap().url, "https://x.io/notes.pdf");
        app.download_key(k(KeyCode::Esc));
        assert!(app.download.is_none());
    }
}
