//! Prompt attachments: `/attach <path>` stages a file, uploads it right away to the Grok
//! Bot attachment store (grokbot_api), and the next prompt carries the aliases. Chips above
//! the input show each file; ✕ removes it. Attachments on sent prompts are resolved to
//! presigned reads for the image viewer and the [⤓] download modal.
use crate::api::{Attachment, Progress};
use crate::app::{App, Event};
use crate::markdown::human_size;
use crate::theme;
use grokbot_api as gw;
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Most files on one prompt (the desktop app's limit).
pub const MAX: usize = gw::attachments::MAX_ATTACHMENTS_PER_PROMPT;
/// Internal link scheme for an attachment on a sent message: `gbattach:<path>`.
pub const SCHEME: &str = "gbattach:";
/// The [⤓] part of a sent attachment chip: `gbattach-dl:<path>`.
pub const DL_SCHEME: &str = "gbattach-dl:";

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Uploading,
    Done(Attachment),
    Failed(String),
}

pub struct Staged {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub kind: gw::AttachmentKind,
    pub bot: String,
    pub progress: Progress,
    pub state: State,
    handle: Option<tokio::task::AbortHandle>,
}

/// What to do once an attachment's read URL is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// Images open in the viewer, other files in the download modal.
    Open,
    Download,
    /// Only remember the read (inline image previews); never shows errors.
    Inline,
}

/// A resolved presigned read (kept until shortly before it expires).
#[derive(Debug, Clone)]
pub struct Read {
    pub url: String,
    pub content_type: String,
    pub until: Instant,
}

#[derive(Default)]
pub struct Attachments {
    pub staged: Vec<Staged>,
    pub seq: u64,
    /// A prompt waiting for uploads to finish.
    pub queued: Option<String>,
    /// path → resolved read.
    pub reads: HashMap<String, Read>,
}

pub fn icon(kind: gw::AttachmentKind, name: &str) -> &'static str {
    match kind {
        gw::AttachmentKind::Image => "🖼",
        gw::AttachmentKind::Video => "🎞",
        gw::AttachmentKind::Audio => "🎵",
        gw::AttachmentKind::File if name.to_lowercase().ends_with(".pdf") => "📄",
        gw::AttachmentKind::File => "📎",
    }
}

/// A user-facing explanation of an upload error.
pub fn error_text(e: &gw::Error) -> String {
    match e {
        gw::Error::Attachment(gw::UploadFailure::UseBox { reason }) => {
            let mut s = "this bot only takes files through the desktop app".to_string();
            if !reason.is_empty() {
                s.push_str(&format!(" ({reason})"));
            }
            s
        }
        gw::Error::Attachment(gw::UploadFailure::NoCursorSession) => {
            "attachments need a Cursor sign-in: /login desktop or /login cursor".into()
        }
        gw::Error::Attachment(gw::UploadFailure::Refused {
            reason,
            limit_bytes,
        }) => format!("refused: {}", reason.describe(*limit_bytes)),
        other => other.to_string(),
    }
}

/// Turn pasted text into file paths when every non-empty line is an existing file.
pub fn pasted_files(text: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let l = l.trim_matches(|c| c == '\'' || c == '"');
        let l = l.strip_prefix("file://").unwrap_or(l);
        let p = crate::download::expand(l);
        if !p.is_file() {
            return vec![];
        }
        out.push(p);
    }
    out
}

/// Split `/attach` arguments into paths, honouring "quoted paths" and `\ ` escapes.
pub fn split_args(args: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', _) => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            (c, None) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (c, _) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

impl App {
    /// Stage and start uploading one file. Errors are user-facing.
    pub fn stage_file(&mut self, raw: &str) -> Result<(), String> {
        let path = crate::download::expand(raw.strip_prefix("file://").unwrap_or(raw));
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if self.attach.staged.len() >= MAX {
            return Err(format!("at most {MAX} attachments per prompt"));
        }
        if self.attach.staged.iter().any(|s| s.path == path) {
            return Err(format!("{name} is already attached"));
        }
        gw::attachments::validate_attachment(&name, meta.len()).map_err(|e| e.to_string())?;
        let Some(api) = self.api.clone() else {
            return Err("not logged in".into());
        };
        if !api.can_attach() {
            return Err(error_text(&gw::Error::Attachment(
                gw::UploadFailure::NoCursorSession,
            )));
        }
        let Some((bot, session, _)) = self.active_bot_route() else {
            return Err("no active bot: pick one with /bot or the sidebar".into());
        };
        self.attach.seq += 1;
        let id = self.attach.seq;
        let progress: Progress = Arc::new(AtomicU64::new(0));
        let (p2, n2, b2, path2) = (progress.clone(), name.clone(), bot.clone(), path.clone());
        let tx = self.event_sender();
        self.inflight += 1;
        let handle = self.rt.spawn(async move {
            let result = match tokio::fs::read(&path2).await {
                Ok(data) => {
                    api.bind_viewer_session(&b2, session.as_deref());
                    api.upload(&b2, &n2, data, p2).await
                }
                Err(e) => Err(gw::Error::Validation(format!("{}: {e}", path2.display()))),
            };
            let _ = tx.send(Event::Uploaded { id, result });
        });
        self.attach.staged.push(Staged {
            id,
            kind: gw::AttachmentKind::from_name(&name),
            path,
            name,
            size: meta.len(),
            bot,
            progress,
            state: State::Uploading,
            handle: Some(handle.abort_handle()),
        });
        Ok(())
    }

    /// `/attach a b c`: stage each, reporting problems one by one.
    pub fn attach_paths(&mut self, paths: &[String]) {
        for p in paths {
            match self.stage_file(p) {
                Ok(()) => {}
                Err(e) => self.error(format!("Can't attach {p}: {e}")),
            }
        }
    }

    pub fn on_uploaded(&mut self, id: u64, result: gw::Result<gw::UploadedAttachment>) {
        let Some(s) = self.attach.staged.iter_mut().find(|s| s.id == id) else {
            return; // removed while uploading
        };
        s.handle = None;
        match result {
            Ok(u) => {
                s.progress.store(s.size, Ordering::Relaxed);
                s.state = State::Done(Attachment {
                    path: u.alias,
                    name: if u.name.is_empty() {
                        s.name.clone()
                    } else {
                        u.name
                    },
                    size: Some(u.size_bytes),
                    content_type: (!u.content_type.is_empty()).then_some(u.content_type),
                });
            }
            Err(e) => {
                let msg = error_text(&e);
                let name = s.name.clone();
                s.state = State::Failed(msg.clone());
                self.error(format!("Upload of {name} failed: {msg}"));
                if let Some(text) = self.attach.queued.take() {
                    self.input = text;
                    self.input_cursor = self.input.len();
                    self.info("Your prompt is back in the input; remove the failed file with /detach and send again.".into());
                }
            }
        }
        self.try_send_queued();
    }

    fn try_send_queued(&mut self) {
        if self.attach.queued.is_some()
            && !self
                .attach
                .staged
                .iter()
                .any(|s| s.state == State::Uploading)
            && let Some(text) = self.attach.queued.take()
        {
            self.send_chat(text);
        }
    }

    /// Called by `send_chat`: `None` = go ahead with these attachments; `Some(())` = the
    /// send was queued or refused.
    pub fn take_attachments_for_send(&mut self, text: &str) -> Option<Vec<Attachment>> {
        if self.attach.staged.is_empty() {
            return Some(vec![]);
        }
        let here = self.active_bot().map(|b| b.id.clone()).unwrap_or_default();
        if self.attach.staged.iter().any(|s| s.bot != here) {
            self.input = text.to_string();
            self.input_cursor = self.input.len();
            self.error(
                "Your attachments were uploaded for another chat. /detach all and attach them here again."
                    .into(),
            );
            return None;
        }
        if let Some(s) = self
            .attach
            .staged
            .iter()
            .find(|s| matches!(s.state, State::Failed(_)))
        {
            let name = s.name.clone();
            self.input = text.to_string();
            self.input_cursor = self.input.len();
            self.error(format!(
                "{name} failed to upload. Remove it with /detach (or ✕) and send again."
            ));
            return None;
        }
        let waiting = self
            .attach
            .staged
            .iter()
            .filter(|s| s.state == State::Uploading)
            .count();
        if waiting > 0 {
            self.attach.queued = Some(text.to_string());
            self.info(format!(
                "Waiting for {waiting} upload{} to finish; the prompt sends right after.",
                if waiting == 1 { "" } else { "s" }
            ));
            return None;
        }
        Some(
            std::mem::take(&mut self.attach.staged)
                .into_iter()
                .filter_map(|s| match s.state {
                    State::Done(a) => Some(a),
                    _ => None,
                })
                .collect(),
        )
    }

    /// Remove chip `n` (1-based); aborts its upload.
    pub fn detach(&mut self, n: usize) -> bool {
        if n == 0 || n > self.attach.staged.len() {
            return false;
        }
        let s = self.attach.staged.remove(n - 1);
        if let Some(h) = s.handle {
            h.abort();
            self.inflight = self.inflight.saturating_sub(1);
        }
        if self.attach.staged.is_empty() {
            self.attach.queued = None;
        }
        self.try_send_queued();
        true
    }

    pub fn detach_all(&mut self) {
        while self.detach(1) {}
    }

    /// Attachments on the active chat's messages, newest message first.
    pub fn feed_attachments(&self) -> Vec<Attachment> {
        let Some(bot) = self.active_bot().map(|b| b.id.clone()) else {
            return vec![];
        };
        self.msg_attachments
            .get(&bot)
            .map(|m| m.iter().rev().flat_map(|(_, a)| a.clone()).collect())
            .unwrap_or_default()
    }

    /// Attachments of the feed entry with this message key.
    pub fn entry_attachments(&self, key: &str) -> Vec<Attachment> {
        let Some(bot) = self.active_bot().map(|b| b.id.clone()) else {
            return vec![];
        };
        self.msg_attachments
            .get(&bot)
            .and_then(|m| m.iter().find(|(k, _)| k == key).map(|(_, a)| a.clone()))
            .unwrap_or_default()
    }

    pub fn fresh_read(&self, path: &str) -> Option<&Read> {
        self.attach
            .reads
            .get(path)
            .filter(|r| r.until > Instant::now())
    }

    /// Open (viewer/download) a sent attachment, resolving its read URL first if needed.
    pub fn open_attachment(&mut self, path: &str, then: Then) {
        if let Some(r) = self.fresh_read(path).cloned() {
            self.act_on_read(path, &r, then);
            return;
        }
        let (Some(api), Some(bot)) = (self.api.clone(), self.active_bot().map(|b| b.id.clone()))
        else {
            self.error("Not logged in.".into());
            return;
        };
        let paths = vec![path.to_string()];
        self.spawn(async move {
            let result = api.resolve_attachments(&bot, &paths).await;
            Event::AttachResolved {
                paths,
                then,
                result,
            }
        });
    }

    pub fn on_attach_resolved(
        &mut self,
        paths: Vec<String>,
        then: Then,
        result: gw::Result<Vec<gw::AttachmentRead>>,
    ) {
        let reads = match result {
            Ok(r) => r,
            Err(e) => {
                if then == Then::Inline {
                    for p in &paths {
                        self.inline.fail(&format!("{SCHEME}{p}"), error_text(&e));
                    }
                } else {
                    self.error(format!("Couldn't open the attachment: {}", error_text(&e)));
                }
                return;
            }
        };
        for p in &paths {
            let name = self.attachment_name(p);
            match reads.iter().find(|r| &r.path == p) {
                Some(r) if r.outcome == gw::ReadOutcome::Store && !r.url.is_empty() => {
                    let ttl = r
                        .expires_at_ms
                        .and_then(|e| {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()?
                                .as_millis() as u64;
                            e.checked_sub(now + 30_000)
                        })
                        .map(Duration::from_millis)
                        .unwrap_or(Duration::from_secs(240));
                    let read = Read {
                        url: r.url.clone(),
                        content_type: r.content_type.clone(),
                        until: Instant::now() + ttl,
                    };
                    self.attach.reads.insert(p.clone(), read.clone());
                    self.feed_cache = None;
                    self.act_on_read(p, &read, then);
                }
                _ if then == Then::Inline => {
                    self.inline
                        .fail(&format!("{SCHEME}{p}"), "not in storage".into());
                }
                Some(r) if r.outcome == gw::ReadOutcome::NotInStore => self.error(format!(
                    "{name} is stored on the bot's computer; open it from the desktop app."
                )),
                _ => self.error(format!("{name}: the bot has no storage for attachments.")),
            }
        }
    }

    fn attachment_name(&self, path: &str) -> String {
        self.feed_attachments()
            .into_iter()
            .find(|a| a.path == path)
            .map(|a| a.name)
            .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string())
    }

    fn act_on_read(&mut self, path: &str, r: &Read, then: Then) {
        if then == Then::Inline {
            return;
        }
        let name = self.attachment_name(path);
        let image = r.content_type.starts_with("image/")
            || gw::AttachmentKind::from_name(&name) == gw::AttachmentKind::Image;
        if then == Then::Open && image && self.view_image(Some(&r.url)) {
            return;
        }
        self.open_download_named(&r.url, &name);
    }

    /// Resolved attachment images/files in the chat, newest first (for the viewer and /download).
    pub fn attachment_openables(&self) -> Vec<crate::markdown::Openable> {
        self.feed_attachments()
            .into_iter()
            .filter_map(|a| {
                let r = self.fresh_read(&a.path)?;
                Some(crate::markdown::Openable {
                    url: r.url.clone(),
                    image: r.content_type.starts_with("image/") || a.is_image(),
                    label: a.name,
                })
            })
            .collect()
    }

    /// Text on a chip.
    fn chip_label(s: &Staged, n: usize) -> String {
        let status = match &s.state {
            State::Uploading => {
                let sent = s.progress.load(Ordering::Relaxed).min(s.size);
                format!(" {}%", (sent * 100).checked_div(s.size).unwrap_or(0))
            }
            State::Done(_) => " ✓".into(),
            State::Failed(_) => " ✗".into(),
        };
        format!(
            "{n}:{} {} {}{status}",
            icon(s.kind, &s.name),
            s.name,
            human_size(s.size)
        )
    }
}

/// The chip row above the input: line plus (start col, end col, action) hits, where the
/// action is `v<n>` (preview) or `x<n>` (remove).
pub fn chip_line(app: &App) -> (Line<'static>, Vec<(usize, usize, String)>) {
    let mut spans: Vec<Span<'static>> = vec![];
    let mut hits = vec![];
    let mut col = 0usize;
    for (i, s) in app.attach.staged.iter().enumerate() {
        if col > 0 {
            spans.push(Span::raw(" "));
            col += 1;
        }
        let label = format!("[{}", App::chip_label(s, i + 1));
        let w = crate::input::str_width(&label);
        let style = match s.state {
            State::Failed(_) => theme::attach_chip_error(),
            _ => theme::attach_chip(),
        };
        spans.push(style.span(label));
        hits.push((col, col + w, format!("v{}", i + 1)));
        col += w;
        let x = " ✕]".to_string();
        spans.push(theme::attach_chip().span(x));
        hits.push((col, col + 3, format!("x{}", i + 1)));
        col += 3;
    }
    (Line::from(spans), hits)
}

/// Chips for a sent message's attachments: (line, hits with `gbattach:` urls).
pub fn sent_line(
    atts: &[Attachment],
    width: usize,
) -> (Line<'static>, Vec<(usize, usize, String)>) {
    let mut parts: Vec<(String, String)> = vec![];
    for a in atts {
        let kind = gw::AttachmentKind::from_name(&a.name);
        let size = a
            .size
            .map(|n| format!(" {}", human_size(n)))
            .unwrap_or_default();
        parts.push((
            format!("[{} {}{size}", icon(kind, &a.name), a.name),
            a.path.clone(),
        ));
    }
    const DL: &str = " ⤓]";
    let total: usize = parts
        .iter()
        .map(|(l, _)| crate::input::str_width(l) + 3)
        .sum::<usize>()
        + parts.len().saturating_sub(1);
    let mut col = width.saturating_sub(total);
    let mut spans = vec![Span::raw(" ".repeat(col))];
    let mut hits = vec![];
    for (i, (l, u)) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
            col += 1;
        }
        let w = crate::input::str_width(&l);
        spans.push(theme::attach_chip().span(l));
        hits.push((col, col + w, format!("{SCHEME}{u}")));
        col += w;
        spans.push(theme::download_button().span(DL));
        hits.push((col, col + 3, format!("{DL_SCHEME}{u}")));
        col += 3;
    }
    (Line::from(spans), hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_handles_quotes_and_escapes() {
        assert_eq!(
            split_args(r#"a.png "my file.pdf" b\ c.txt 'd e'"#),
            vec!["a.png", "my file.pdf", "b c.txt", "d e"]
        );
    }

    #[test]
    fn pasted_files_requires_every_line_to_be_a_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x.png");
        std::fs::write(&f, b"x").unwrap();
        let s = f.display().to_string();
        assert_eq!(pasted_files(&format!("'{s}'\n")), vec![f.clone()]);
        assert_eq!(pasted_files(&format!("file://{s}")), vec![f]);
        assert!(pasted_files(&format!("{s}\nhello")).is_empty());
        assert!(pasted_files("just text").is_empty());
    }

    #[test]
    fn error_texts() {
        let ub = gw::Error::Attachment(gw::UploadFailure::UseBox {
            reason: String::new(),
        });
        assert_eq!(
            error_text(&ub),
            "this bot only takes files through the desktop app"
        );
        let r = gw::Error::Attachment(gw::UploadFailure::Refused {
            reason: gw::UploadRefusal::TooLarge,
            limit_bytes: Some(25 * 1024 * 1024),
        });
        assert!(error_text(&r).contains("25 MiB"));
        assert!(
            error_text(&gw::Error::Attachment(gw::UploadFailure::NoCursorSession))
                .contains("/login desktop")
        );
    }

    #[test]
    fn sent_line_right_aligns_and_links() {
        let a = Attachment {
            path: "al1".into(),
            name: "t.png".into(),
            size: Some(100),
            content_type: None,
        };
        let (l, hits) = sent_line(&[a], 40);
        assert_eq!(l.width(), 40);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].2, "gbattach:al1");
        assert_eq!(hits[1].2, "gbattach-dl:al1");
        assert_eq!(hits[1].1, 40);
    }

    use crate::app::tests::{mock_login, settle, test_app, type_line};

    fn file(d: &std::path::Path, name: &str, bytes: &[u8]) -> String {
        let p = d.join(name);
        std::fs::write(&p, bytes).unwrap();
        p.display().to_string()
    }

    #[test]
    fn attach_uploads_then_send_carries_aliases() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        let f = file(d.path(), "teal.png", b"\x89PNG");
        type_line(&mut app, &format!("/attach {f}"));
        assert_eq!(app.attach.staged.len(), 1);
        settle(&mut app);
        assert!(matches!(app.attach.staged[0].state, State::Done(_)));
        type_line(&mut app, "what colour?");
        settle(&mut app);
        assert!(app.attach.staged.is_empty());
        let bot = app.active_bot().unwrap().id.clone();
        let msgs = app.store.messages(&bot).unwrap();
        let mine = msgs.iter().find(|m| m.text == "what colour?").unwrap();
        assert_eq!(mine.attachments.len(), 1);
        assert_eq!(mine.attachments[0].name, "teal.png");
        assert!(mine.attachments[0].path.starts_with("mock-att-"));
        assert_eq!(app.feed_attachments().len(), 1);
    }

    #[test]
    fn send_waits_for_uploads_and_failed_uploads_block_send() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        let f = file(d.path(), "a.txt", b"hello");
        type_line(&mut app, &format!("/attach {f}"));
        // Sent before the upload event is applied: queued, then sent.
        type_line(&mut app, "queued prompt");
        settle(&mut app);
        assert!(app.attach.queued.is_none());
        assert!(app.attach.staged.is_empty());
        let r = file(d.path(), "refuse.bin", b"x");
        type_line(&mut app, &format!("/attach {r}"));
        settle(&mut app);
        assert!(
            matches!(app.attach.staged[0].state, State::Failed(ref e) if e.contains("not accepted"))
        );
        type_line(&mut app, "blocked");
        assert_eq!(app.input, "blocked", "prompt kept for retry");
        type_line(&mut app, "/detach");
        assert!(app.attach.staged.is_empty());
        let u = file(d.path(), "usebox.pdf", b"x");
        type_line(&mut app, &format!("/attach {u}"));
        settle(&mut app);
        assert!(
            matches!(app.attach.staged[0].state, State::Failed(ref e) if e.contains("desktop app"))
        );
    }

    #[test]
    fn limits_and_validation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        let empty = file(d.path(), "empty.txt", b"");
        assert!(app.stage_file(&empty).unwrap_err().contains("empty"));
        for i in 0..MAX {
            let f = file(d.path(), &format!("f{i}.txt"), b"x");
            app.stage_file(&f).unwrap();
        }
        let f = file(d.path(), "seven.txt", b"x");
        assert!(app.stage_file(&f).unwrap_err().contains("at most 6"));
        let again = d.path().join("f0.txt").display().to_string();
        app.detach(MAX);
        assert!(app.stage_file(&again).unwrap_err().contains("already"));
        type_line(&mut app, "/detach all");
        assert!(app.attach.staged.is_empty());
    }

    #[test]
    fn tab_cycles_replace_the_path_token() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, d) = test_app(&rt);
        mock_login(&mut app, "t");
        file(d.path(), "alpha.png", b"x");
        file(d.path(), "beta.png", b"x");
        let dir = format!("{}/", d.path().display());
        app.input = format!("/attach {dir}");
        app.input_cursor = app.input.len();
        crate::complete::tab(&mut app, false);
        let first = app.input.clone();
        crate::complete::tab(&mut app, false);
        let second = app.input.clone();
        assert_ne!(first, second);
        assert_eq!(second.matches("/attach ").count(), 1);
        assert!(second.starts_with(&format!("/attach {dir}")));
        assert!(!second[dir.len() + 8..].contains(".png") || second.matches(".png").count() == 1);
    }
}
