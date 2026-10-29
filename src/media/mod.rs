//! Inline video, animated GIFs and music (1.24.0).
//!
//! - Videos and GIFs play inside the chat bubble (scaled like images, row-capped) with a
//!   one-row bar under them: `[▶]` toggles playback, the bar seeks, time (GIFs: frame
//!   position). Clicking the picture opens the full-size player modal.
//! - Music links/attachments get the same bar without a picture.
//! - One inline video or track plays at a time; it pauses when scrolled out of view.
//!   GIFs loop on their own (pref `media.gif_autoplay`).
//! - Decoding: GIFs with the image crate, H.264 MP4 with the bundled OpenH264, audio with
//!   rodio/symphonia (MP3, AAC/M4A, FLAC, Vorbis, WAV). Other codecs use the linked FFmpeg
//!   in `-ffmpeg` builds, else a system `ffmpeg` on PATH if there is one.
pub mod modal;
pub mod player;
pub mod source;

use crate::app::{App, Event};
use player::{Cmd, Out};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Largest media file loaded for playback.
pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
/// Key prefix for media inline states (kept apart from still images).
pub const PREFIX: &str = "media:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Gif,
    Audio,
}

/// Kind by file extension (query/fragment ignored).
pub fn kind_of(name_or_url: &str) -> Option<Kind> {
    let p = name_or_url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let ext = p.rsplit('.').next().unwrap_or("");
    match ext {
        "gif" => Some(Kind::Gif),
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "ogv" | "avi" => Some(Kind::Video),
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wav" | "weba" => {
            Some(Kind::Audio)
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decoder {
    /// Built-in decoders first, then linked FFmpeg, then system ffmpeg.
    #[default]
    Auto,
    Builtin,
    Ffmpeg,
    System,
}

impl Decoder {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "auto" => Self::Auto,
            "builtin" | "pure" => Self::Builtin,
            "ffmpeg" | "linked" => Self::Ffmpeg,
            "system" => Self::System,
            _ => return None,
        })
    }
}

pub struct Item {
    pub kind: Kind,
    pub label: String,
    pub pos: Duration,
    pub dur: Option<Duration>,
    pub frame: Option<(usize, usize)>,
    pub playing: bool,
    pub decoder: &'static str,
    pub audio: bool,
    pub error: Option<String>,
    pub ctl: Option<std::sync::mpsc::Sender<Cmd>>,
    pub loading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Button,
    Bar,
}

#[derive(Default)]
pub struct Media {
    pub items: HashMap<String, Item>,
    /// (rect, key, part) drawn this frame.
    pub hits: Vec<(Rect, String, Part)>,
    /// Picture rects drawn this frame (click → modal).
    pub pics: Vec<(Rect, String)>,
    /// Keys whose inline bar was on screen this frame.
    pub shown: Vec<String>,
    /// Open full-size player.
    pub modal: Option<modal::Modal>,
}

/// Is any media file supported by this build at all (for /status and /license)?
pub fn decoders_summary() -> String {
    let mut v = vec!["GIF", "H.264/MP4 (OpenH264)", "MP3/AAC/FLAC/Vorbis/WAV"];
    if cfg!(feature = "ffmpeg") {
        v.push("FFmpeg (linked)");
    }
    let sys = if source::system_ffmpeg().is_some() {
        ", system ffmpeg found"
    } else {
        ""
    };
    format!("{}{sys}", v.join(", "))
}

pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// The bar text for `width` cells: (button, bar, label). Bar is `━` played, `─` rest.
pub fn bar_parts(it: &Item, width: usize) -> (String, String, String) {
    let button = if it.error.is_some() {
        "[!]".to_string()
    } else if it.loading {
        "[…]".to_string()
    } else if it.playing {
        "[⏸]".to_string()
    } else {
        "[▶]".to_string()
    };
    let label = if let Some(e) = &it.error {
        format!(" {e}")
    } else if let Some((i, n)) = it.frame {
        format!(" {}/{}", i + 1, n)
    } else {
        format!(
            " {}/{}",
            fmt_time(it.pos),
            it.dur.map(fmt_time).unwrap_or_else(|| "?".into())
        )
    };
    let room = width.saturating_sub(button.chars().count() + 1 + label.chars().count());
    if it.error.is_some() {
        return (button, String::new(), label);
    }
    let frac = match (it.frame, it.dur) {
        (Some((i, n)), _) if n > 1 => i as f64 / (n - 1) as f64,
        (_, Some(d)) if !d.is_zero() => (it.pos.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let done = ((room as f64) * frac).round() as usize;
    let bar = format!(" {}{}", "━".repeat(done), "─".repeat(room - done.min(room)));
    (button, bar, label)
}

/// Fraction of the bar at column `col` of a bar starting at `x` and `w` wide.
pub fn frac_at(x: u16, w: u16, col: u16) -> f64 {
    if w <= 1 {
        return 0.0;
    }
    (col.saturating_sub(x) as f64 / (w - 1) as f64).clamp(0.0, 1.0)
}

fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    if let Some(p) = crate::imageview::local_path(url) {
        let m = std::fs::metadata(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if m.len() > MAX_BYTES {
            return Err("larger than 200 MB".into());
        }
        return std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()));
    }
    Err("not a local file".into())
}

async fn fetch(url: String) -> Result<Vec<u8>, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return tokio::task::spawn_blocking(move || fetch_bytes(&url))
            .await
            .map_err(|e| e.to_string())?;
    }
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err("larger than 200 MB".into());
    }
    let b = resp.bytes().await.map_err(|e| e.to_string())?;
    if b.len() as u64 > MAX_BYTES {
        return Err("larger than 200 MB".into());
    }
    Ok(b.to_vec())
}

/// Build the playback spec for `bytes` (blocking: probes and opens decoders).
pub fn open_spec(
    kind: Kind,
    name: &str,
    bytes: Vec<u8>,
    decoder: Decoder,
    fps: u32,
) -> Result<player::Spec, String> {
    let bytes = Arc::new(bytes);
    let mut spec = player::Spec {
        video: None,
        audio: None,
        fps,
        looping: kind == Kind::Gif,
        autoplay: false,
        volume: 1.0,
        temp: None,
    };
    match kind {
        Kind::Audio => {
            spec.audio = Some(bytes);
            return Ok(spec);
        }
        Kind::Gif => {
            spec.video = Some(Box::new(source::Gif::open(&bytes)?));
            return Ok(spec);
        }
        Kind::Video => {}
    }
    let mut errs = vec![];
    if matches!(decoder, Decoder::Auto | Decoder::Builtin) && source::is_mp4(&bytes) {
        match source::H264::open(bytes.clone()) {
            Ok(v) => {
                spec.video = Some(Box::new(v));
                spec.audio = Some(bytes);
                return Ok(spec);
            }
            Err(e) => errs.push(e),
        }
    }
    if decoder == Decoder::Builtin {
        return Err(errs.pop().unwrap_or_else(|| "not an H.264 MP4".into()));
    }
    // FFmpeg (linked or system) reads a temp copy.
    let ext = name
        .split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('.').next())
        .filter(|e| e.len() <= 5)
        .unwrap_or("bin");
    let tmp = std::env::temp_dir().join(format!(
        "grokbot-tui-media-{}-{}.{ext}",
        std::process::id(),
        crate::emoji::message_key(0, name)
    ));
    std::fs::write(&tmp, &*bytes).map_err(|e| format!("temp file: {e}"))?;
    spec.temp = Some(tmp.clone());
    #[cfg(feature = "ffmpeg")]
    if matches!(decoder, Decoder::Auto | Decoder::Ffmpeg) {
        match source::LibFfmpeg::open(&tmp) {
            Ok(v) => {
                spec.video = Some(Box::new(v));
                spec.audio = Some(bytes);
                return Ok(spec);
            }
            Err(e) => errs.push(e),
        }
    }
    if matches!(decoder, Decoder::Auto | Decoder::System)
        && let Some(exe) = source::system_ffmpeg()
    {
        match source::SystemFfmpeg::open(exe, tmp.clone(), fps) {
            Ok(v) => {
                spec.video = Some(Box::new(v));
                spec.audio = Some(bytes);
                return Ok(spec);
            }
            Err(e) => errs.push(e),
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(if cfg!(feature = "ffmpeg") {
        errs.pop().unwrap_or_else(|| "unsupported video".into())
    } else {
        "codec not built in: install ffmpeg, or use the -ffmpeg build".into()
    })
}

pub const LICENSE_TEXT: &str = include_str!("../../LICENSE");
pub const THIRD_PARTY: &str = include_str!("../../THIRD_PARTY.md");

impl App {
    /// `/license`: the licence summary and THIRD_PARTY.md (or the full GPL text).
    pub fn show_license(&mut self, full: bool) {
        let body = if full {
            LICENSE_TEXT.to_string()
        } else {
            format!(
                "grokbot-tui {} is free software under the GNU General Public License,\n\
                 version 3 or (at your option) any later version. It comes with ABSOLUTELY\n\
                 NO WARRANTY. `/license full` shows the licence text.\n\n\
                 This build: {}{}.\n\n{}",
                env!("CARGO_PKG_VERSION"),
                if cfg!(feature = "ffmpeg") {
                    "-ffmpeg variant (FFmpeg linked in), "
                } else {
                    "standard variant, "
                },
                decoders_summary(),
                THIRD_PARTY
            )
        };
        self.help = Some(crate::app::HelpModal {
            page: crate::help::Page {
                id: "license".into(),
                title: if full {
                    "GNU GPL v3".into()
                } else {
                    "License".into()
                },
                body,
                related: vec![],
            },
            scroll: 0,
            link: 0,
            back: vec![],
            query: None,
        });
    }

    /// Start loading media `key` (`media:<url>` or `media:gbattach:<path>`).
    pub fn media_want(&mut self, key: &str, kind: Kind, label: &str) {
        if self.media.items.contains_key(key) {
            return;
        }
        let target = &key[PREFIX.len()..];
        let url = if let Some(path) = target.strip_prefix(crate::attach::SCHEME) {
            match self.fresh_read(path) {
                Some(r) => r.url.clone(),
                None => {
                    let marker = format!("{key}#resolving");
                    if let std::collections::hash_map::Entry::Vacant(v) =
                        self.inline.states.entry(marker)
                    {
                        v.insert(crate::inline::State::Loading);
                        let p = path.to_string();
                        self.open_attachment(&p, crate::attach::Then::Inline);
                    }
                    return;
                }
            }
        } else {
            target.to_string()
        };
        self.media.items.insert(
            key.to_string(),
            Item {
                kind,
                label: label.to_string(),
                pos: Duration::ZERO,
                dur: None,
                frame: None,
                playing: false,
                decoder: "",
                audio: false,
                error: None,
                ctl: None,
                loading: true,
            },
        );
        let (k, name) = (key.to_string(), label.to_string());
        let (dec, fps) = (self.config.media_decoder, self.config.media_fps);
        let autoplay = kind == Kind::Gif && self.config.gif_autoplay;
        let tx = self.event_tx();
        self.spawn(async move {
            let res = match fetch(url).await {
                Ok(bytes) => {
                    tokio::task::spawn_blocking(move || open_spec(kind, &name, bytes, dec, fps))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                }
                Err(e) => Err(e),
            };
            match res {
                Ok(mut spec) => {
                    spec.autoplay = autoplay;
                    let (ctl, rx) = std::sync::mpsc::channel();
                    let kk = k.clone();
                    std::thread::spawn(move || {
                        player::run(spec, rx, |out| {
                            tx.send(Event::Media {
                                key: kk.clone(),
                                out,
                            })
                            .is_ok()
                        })
                    });
                    Event::MediaReady {
                        key: k,
                        ctl: Ok(ctl),
                    }
                }
                Err(e) => Event::MediaReady {
                    key: k,
                    ctl: Err(e),
                },
            }
        });
    }

    pub fn on_media_ready(
        &mut self,
        key: String,
        ctl: Result<std::sync::mpsc::Sender<Cmd>, String>,
    ) {
        let Some(it) = self.media.items.get_mut(&key) else {
            if let Ok(c) = ctl {
                let _ = c.send(Cmd::Stop);
            }
            return;
        };
        it.loading = false;
        match ctl {
            Ok(c) => it.ctl = Some(c),
            Err(e) => it.error = Some(e),
        }
    }

    pub fn on_media(&mut self, key: String, out: Out) {
        let Some(it) = self.media.items.get_mut(&key) else {
            return;
        };
        match out {
            Out::Frame(img) => {
                let first = !matches!(
                    self.inline.states.get(&key),
                    Some(crate::inline::State::Ready(_))
                );
                self.inline
                    .states
                    .insert(key.clone(), crate::inline::State::Ready(img));
                self.inline.protos.retain(|(k, _, _), _| *k != key);
                if let Some(m) = self.media.modal.as_mut()
                    && m.key == key
                {
                    m.proto = None;
                }
                if first {
                    // The slot's size is known now: re-lay out the feed.
                    self.inline.generation += 1;
                    self.feed_cache = None;
                }
            }
            Out::Progress {
                pos,
                dur,
                playing,
                frame,
            } => {
                it.pos = pos;
                it.dur = dur;
                it.playing = playing;
                it.frame = frame;
            }
            Out::Info { decoder, audio } => {
                it.decoder = decoder;
                it.audio = audio;
            }
            Out::Error(e) => {
                it.error = Some(e);
                it.ctl = None;
            }
        }
    }

    pub fn media_cmd(&mut self, key: &str, cmd: Cmd) {
        // One video/track at a time (GIFs are exempt).
        let starting = matches!(cmd, Cmd::Play)
            || (cmd == Cmd::Toggle && self.media.items.get(key).is_some_and(|i| !i.playing));
        if starting
            && self
                .media
                .items
                .get(key)
                .is_some_and(|i| i.kind != Kind::Gif)
        {
            for (k, it) in &self.media.items {
                if k != key
                    && it.playing
                    && it.kind != Kind::Gif
                    && let Some(c) = &it.ctl
                {
                    let _ = c.send(Cmd::Pause);
                }
            }
        }
        if let Some(c) = self.media.items.get(key).and_then(|i| i.ctl.as_ref()) {
            let _ = c.send(cmd);
        }
    }

    /// After drawing: pause what scrolled out of view (unless the modal shows it).
    pub fn media_after_draw(&mut self) {
        let modal = self.media.modal.as_ref().map(|m| m.key.clone());
        let off: Vec<String> = self
            .media
            .items
            .iter()
            .filter(|(k, it)| {
                it.playing && !self.media.shown.contains(k) && modal.as_ref() != Some(*k)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in off {
            self.media_cmd(&k, Cmd::Pause);
        }
    }

    /// A left click on a media bar or picture; true when consumed.
    pub fn media_click(&mut self, col: u16, row: u16) -> bool {
        let at = ratatui::layout::Position::new(col, row);
        if let Some((r, key, part)) = self
            .media
            .hits
            .iter()
            .find(|(r, _, _)| r.contains(at))
            .cloned()
        {
            match part {
                Part::Button => self.media_cmd(&key, Cmd::Toggle),
                Part::Bar => self.media_cmd(&key, Cmd::SeekFrac(frac_at(r.x, r.width, col))),
            }
            return true;
        }
        if let Some((_, key)) = self
            .media
            .pics
            .iter()
            .find(|(r, _)| r.contains(at))
            .cloned()
        {
            self.open_media_modal(&key);
            return true;
        }
        false
    }

    /// Pause every player (suspend).
    pub fn media_cmd_all_pause(&mut self) {
        let keys: Vec<String> = self.media.items.keys().cloned().collect();
        for k in keys {
            self.media_cmd(&k, Cmd::Pause);
        }
    }

    /// Stop every player (logout, chat switch, quit).
    pub fn media_stop_all(&mut self) {
        for it in self.media.items.values() {
            if let Some(c) = &it.ctl {
                let _ = c.send(Cmd::Stop);
            }
        }
        self.media.items.clear();
        self.media.modal = None;
        self.inline.states.retain(|k, _| !k.starts_with(PREFIX));
        self.inline
            .protos
            .retain(|(k, _, _), _| !k.starts_with(PREFIX));
        self.feed_cache = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_of("https://x/a.MP4?sig=1"), Some(Kind::Video));
        assert_eq!(kind_of("cat.gif"), Some(Kind::Gif));
        assert_eq!(kind_of("song.flac"), Some(Kind::Audio));
        assert_eq!(kind_of("doc.pdf"), None);
    }

    fn item() -> Item {
        Item {
            kind: Kind::Video,
            label: "v".into(),
            pos: Duration::from_secs(30),
            dur: Some(Duration::from_secs(60)),
            frame: None,
            playing: false,
            decoder: "",
            audio: false,
            error: None,
            ctl: None,
            loading: false,
        }
    }

    #[test]
    fn bar_shows_time_and_fills_by_position() {
        let (b, bar, l) = bar_parts(&item(), 30);
        assert_eq!(b, "[▶]");
        assert_eq!(l, " 0:30/1:00");
        let n = bar.chars().count();
        assert_eq!(3 + n + l.chars().count(), 30);
        let played = bar.chars().filter(|c| *c == '━').count();
        let rest = bar.chars().filter(|c| *c == '─').count();
        assert!(played.abs_diff(rest) <= 1);
        let mut g = item();
        g.kind = Kind::Gif;
        g.frame = Some((4, 10));
        g.playing = true;
        let (b, _, l) = bar_parts(&g, 30);
        assert_eq!((b.as_str(), l.as_str()), ("[⏸]", " 5/10"));
        assert_eq!(fmt_time(Duration::from_secs(3725)), "1:02:05");
        assert_eq!(frac_at(10, 11, 15), 0.5);
        assert_eq!(frac_at(10, 11, 99), 1.0);
    }

    #[test]
    fn non_mp4_video_without_ffmpeg_says_how_to_play_it() {
        if cfg!(feature = "ffmpeg") || source::system_ffmpeg().is_some() {
            return;
        }
        let e = open_spec(Kind::Video, "a.webm", vec![0; 64], Decoder::Auto, 10).err();
        assert!(e.unwrap().contains("-ffmpeg"));
    }

    #[test]
    fn audio_fixture_decodes() {
        use rodio::Source;
        let b = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tiny.mp3"
        ))
        .unwrap();
        let d = rodio::Decoder::new(std::io::Cursor::new(source::ArcBytes(Arc::new(b)))).unwrap();
        assert!(d.sample_rate().get() > 0);
    }

    #[test]
    fn player_thread_sends_poster_progress_and_plays_to_the_end() {
        let b = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/tiny.mp4"
        ))
        .unwrap();
        let mut spec = open_spec(Kind::Video, "tiny.mp4", b, Decoder::Builtin, 30).unwrap();
        spec.audio = None; // no sound device in CI
        let (ctl, rx) = std::sync::mpsc::channel();
        let (otx, orx) = std::sync::mpsc::channel();
        let h = std::thread::spawn(move || player::run(spec, rx, move |o| otx.send(o).is_ok()));
        let first = orx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            first,
            Out::Info {
                decoder: "openh264",
                audio: false
            }
        ));
        assert!(matches!(
            orx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Out::Frame(_)
        ));
        ctl.send(Cmd::Play).unwrap();
        let mut frames = 0;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut finished = false;
        while std::time::Instant::now() < deadline {
            match orx.recv_timeout(Duration::from_secs(1)) {
                Ok(Out::Frame(_)) => frames += 1,
                Ok(Out::Progress {
                    playing: false,
                    pos,
                    ..
                }) if frames > 0 => {
                    assert!(pos >= Duration::from_millis(900));
                    finished = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(finished && frames >= 5, "frames {frames}");
        ctl.send(Cmd::SeekFrac(0.5)).unwrap();
        loop {
            if let Out::Progress { pos, .. } = orx.recv_timeout(Duration::from_secs(5)).unwrap() {
                assert!(
                    pos >= Duration::from_millis(450) && pos <= Duration::from_millis(600),
                    "{pos:?}"
                );
                break;
            }
        }
        ctl.send(Cmd::Stop).unwrap();
        h.join().unwrap();
    }

    #[test]
    fn inline_video_gets_a_picture_and_a_clickable_bar() {
        use ratatui::{Terminal, backend::TestBackend};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "t");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mp4");
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tiny.mp4"),
            &p,
        )
        .unwrap();
        let url = format!("file://{}", p.display());
        let bot = app.active_bot().unwrap().id.clone();
        app.logs.insert(
            bot,
            vec![crate::app::Entry {
                kind: crate::app::EntryKind::Bot,
                text: format!("watch [clip]({url})"),
            }],
        );
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let key = format!("{PREFIX}{url}");
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
            crate::app::tests::settle(&mut app);
            if !app.media.pics.is_empty() {
                break;
            }
        }
        assert!(app.media.items.get(&key).is_some_and(|i| i.ctl.is_some()));
        assert!(!app.media.pics.is_empty(), "poster drawn");
        let (b, _, _) = app
            .media
            .hits
            .iter()
            .find(|h| h.2 == Part::Button)
            .cloned()
            .unwrap();
        assert!(app.media_click(b.x, b.y));
        // Picture click opens the modal.
        let (r, _) = app.media.pics[0].clone();
        assert!(app.media_click(r.x, r.y));
        assert!(app.media.modal.is_some() && app.overlay_open());
        term.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
        app.media_modal_key(ratatui::crossterm::event::KeyEvent::from(
            ratatui::crossterm::event::KeyCode::Esc,
        ));
        assert!(app.media.modal.is_none());
        app.media_stop_all();
        assert!(app.media.items.is_empty());
    }
}
