//! Fuzzball terminal protocol (Mouser), v0 wire format: capability detection and
//! graphics. Spec: github.com/WeirdBrains/mouser `docs/protocol/` (README, graphics,
//! keyboard). Messages are DCS strings `ESC P = F z <sub> <verb> ; <params> [: payload] ESC \`.
//!
//! Graphics: images are uploaded once per id (base64, chunked to `max_chunk`) and placed
//! at a cell with `cols`/`rows`, `move=0`. Placements are cell-anchored and removed by
//! erases, so after a full clear everything is placed again.
//!
//! Keyboard: the capability reply's `k=1` is detected and shown, but structured key
//! events are DCS strings on stdin, which the input reader (crossterm) cannot decode, so
//! the mode is never switched on; the kitty keyboard protocol is used instead.

use ratatui::layout::Rect;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Capability-query reply id.
const CAPS_ID: u32 = 1;
/// Graphics-query reply id (for the cell size).
const GQ_ID: u32 = 2;
/// Bound on the detection wait (DA1 normally ends it much sooner).
#[cfg(unix)]
const QUERY_TIMEOUT_MS: u64 = 250;
/// Extra wait after DA1 inside tmux (tmux answers DA1 itself, then forwards passthrough).
#[cfg(unix)]
const TMUX_EXTRA_MS: u64 = 80;
/// GNU screen passthrough pieces (spec §3.2: at most 768 bytes).
const SCREEN_PIECE: usize = 768;
/// Largest pixel edge we upload (the terminal scales to the cells).
const MAX_UPLOAD_EDGE: u32 = 2048;

/// What the terminal advertised in `ESC P = F z c r ; … ESC \`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Caps {
    pub v: u32,
    /// Subsystem letters (`g` graphics, `k` keyboard, `p` plugins, `s` shell).
    pub sub: String,
    pub raw: bool,
    pub max_chunk: usize,
    pub g_max_dim: u32,
    pub g_fmt: Vec<String>,
    pub keyboard: bool,
    pub k_release: bool,
    /// Cell size in pixels, from the graphics query (`cell_w`, `cell_h`).
    pub cell: Option<(u16, u16)>,
    /// The reply came through tmux or screen (base64 only, wrapped output).
    pub mux: Mux,
}

impl Caps {
    pub fn graphics(&self) -> bool {
        self.sub.contains('g')
    }
    pub fn keyboard(&self) -> bool {
        self.sub.contains('k') && self.keyboard
    }
    pub fn summary(&self) -> String {
        format!(
            "v={} sub={} max_chunk={} fmt={} keyboard={}{}",
            self.v,
            self.sub,
            self.max_chunk,
            self.g_fmt.join(":"),
            if self.keyboard() { "yes" } else { "no" },
            match self.mux {
                Mux::None => "",
                Mux::Tmux => " (through tmux)",
                Mux::Screen => " (through screen)",
            }
        )
    }
}

/// Multiplexer between us and the terminal (spec §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mux {
    #[default]
    None,
    Tmux,
    Screen,
}

impl Mux {
    pub fn from_env() -> Self {
        Self::from_vars(
            std::env::var_os("TMUX").is_some(),
            std::env::var_os("STY").is_some(),
        )
    }
    pub fn from_vars(tmux: bool, sty: bool) -> Self {
        if tmux {
            Self::Tmux
        } else if sty {
            Self::Screen
        } else {
            Self::None
        }
    }
}

/// Wrap one message for the multiplexer (tmux doubles every ESC; screen pieces ≤ 768 B).
pub fn wrap(msg: &str, mux: Mux) -> String {
    match mux {
        Mux::None => msg.to_string(),
        Mux::Tmux => format!("\x1bPtmux;{}\x1b\\", msg.replace('\x1b', "\x1b\x1b")),
        Mux::Screen => {
            let mut out = String::new();
            let b = msg.as_bytes();
            for piece in b.chunks(SCREEN_PIECE) {
                out.push_str("\x1bP");
                out.push_str(std::str::from_utf8(piece).unwrap_or_default());
                out.push_str("\x1b\\");
            }
            out
        }
    }
}

fn dcs(sub: char, verb: char, params: &str) -> String {
    format!("\x1bP=Fz{sub}{verb};{params}\x1b\\")
}

/// Capability query + graphics query (cell size), then DA1 so any terminal answers.
pub fn detect_query(mux: Mux) -> String {
    let mut s = wrap(&dcs('c', 'q', &format!("id={CAPS_ID}")), mux);
    s.push_str(&wrap(&dcs('g', 'q', &format!("id={GQ_ID}")), mux));
    s.push_str("\x1b[c");
    s
}

/// Params of every `ESC P = F z <sub> r ; … ESC \` reply in `buf`, by subsystem.
fn replies(buf: &[u8]) -> Vec<(char, HashMap<String, String>)> {
    let s = String::from_utf8_lossy(buf);
    let mut out = vec![];
    let mut rest: &str = &s;
    while let Some(i) = rest.find("\x1bP=Fz") {
        let r = &rest[i + 5..];
        let Some(end) = r.find("\x1b\\") else { break };
        let body = &r[..end];
        let mut ch = body.chars();
        if let (Some(sub), Some('r')) = (ch.next(), ch.next()) {
            let params = ch.as_str().strip_prefix(';').unwrap_or(ch.as_str());
            let map = params
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
                .collect();
            out.push((sub, map));
        }
        rest = &r[end + 2..];
    }
    out
}

/// Parse the detection answers. `None` when no Fuzzball capability reply is present
/// (DA1 alone means Fuzzball is unavailable).
pub fn parse_caps(buf: &[u8], mux: Mux) -> Option<Caps> {
    let all = replies(buf);
    let (_, c) = all
        .iter()
        .find(|(sub, m)| *sub == 'c' && m.get("id").is_some_and(|i| i == "1"))?;
    let num = |k: &str| c.get(k).and_then(|v| v.parse::<u64>().ok());
    let mut caps = Caps {
        v: num("v").unwrap_or(0) as u32,
        sub: c.get("sub").cloned().unwrap_or_default(),
        raw: num("raw") == Some(1),
        max_chunk: num("max_chunk").unwrap_or(4096).clamp(256, 1 << 20) as usize,
        g_max_dim: num("g_max_dim").unwrap_or(10_000) as u32,
        g_fmt: c
            .get("g_fmt")
            .map(|v| v.split(':').map(str::to_string).collect())
            .unwrap_or_default(),
        keyboard: num("k") == Some(1),
        k_release: num("k_release") == Some(1),
        cell: None,
        mux,
    };
    if let Some((_, g)) = all
        .iter()
        .find(|(sub, m)| *sub == 'g' && m.get("id").is_some_and(|i| i == "2"))
    {
        let w = g.get("cell_w").and_then(|v| v.parse::<u16>().ok());
        let h = g.get("cell_h").and_then(|v| v.parse::<u16>().ok());
        if let (Some(w), Some(h)) = (w, h)
            && w > 0
            && h > 0
        {
            caps.cell = Some((w, h));
        }
    }
    Some(caps)
}

/// True once `buf` holds a complete DA1 answer (`ESC [ ? … c`).
#[cfg(unix)]
fn has_da1(buf: &[u8]) -> bool {
    buf.windows(3)
        .position(|w| w == b"\x1b[?")
        .is_some_and(|i| buf[i + 3..].contains(&b'c'))
}

/// Ask the controlling terminal. Call in raw mode before anything else reads stdin.
/// `GROKBOT_TUI_FUZZBALL=off` skips the query.
#[cfg(unix)]
pub fn detect() -> Option<Caps> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};
    if std::env::var_os("GROKBOT_TUI_FUZZBALL").is_some_and(|v| v == "off") {
        return None;
    }
    let mux = Mux::from_env();
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    tty.write_all(detect_query(mux).as_bytes()).ok()?;
    tty.flush().ok()?;
    let mut deadline = Instant::now() + Duration::from_millis(QUERY_TIMEOUT_MS);
    let mut extended = false;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    while buf.len() < 8192 {
        if has_da1(&buf) {
            // Direct: the Fuzzball reply comes before DA1. tmux: wait a little more.
            if mux != Mux::Tmux || parse_caps(&buf, mux).is_some() {
                break;
            }
            if !extended {
                extended = true;
                deadline = deadline.min(Instant::now() + Duration::from_millis(TMUX_EXTRA_MS));
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for an open file descriptor we own.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as i32) };
        if ready <= 0 {
            break;
        }
        match tty.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    parse_caps(&buf, mux)
}

#[cfg(not(unix))]
pub fn detect() -> Option<Caps> {
    None
}

pub fn b64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            s.push(if i <= c.len() {
                T[(n >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    s
}

/// Upload messages (graphics §2.1) for RGBA pixels, base64, chunked so each payload is
/// at most `max_chunk` wire bytes. `quiet=2`: no replies (they would reach our stdin).
pub fn upload(id: u32, w: u32, h: u32, rgba: &[u8], max_chunk: usize) -> Vec<String> {
    let data = b64(rgba);
    let step = (max_chunk / 4).max(1) * 4;
    let pieces: Vec<&str> = if data.is_empty() {
        vec![""]
    } else {
        data.as_bytes()
            .chunks(step)
            .map(|c| std::str::from_utf8(c).unwrap_or_default())
            .collect()
    };
    let n = pieces.len();
    pieces
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let more = u8::from(i + 1 < n);
            let params = if i == 0 {
                format!(
                    "id={id},fmt=rgba,w={w},h={h},size={},enc=b64,len={},more={more},quiet=2",
                    rgba.len(),
                    p.len()
                )
            } else {
                format!("id={id},enc=b64,len={},more={more},quiet=2", p.len())
            };
            format!("\x1bP=Fzgu;{params}:{p}\x1b\\")
        })
        .collect()
}

/// Place image `id` (placement `pid`) at the cursor, `cols × rows` cells, cursor kept.
pub fn place(id: u32, pid: u32, cols: u16, rows: u16) -> String {
    dcs(
        'g',
        'p',
        &format!("id={id},pid={pid},cols={cols},rows={rows},move=0,quiet=2"),
    )
}

/// Remove one placement (image data stays for re-placing).
pub fn delete_placement(id: u32, pid: u32) -> String {
    dcs('g', 'd', &format!("what=image,id={id},pid={pid},quiet=2"))
}

/// Remove every placement on the current screen and free unplaced data.
pub fn delete_all() -> String {
    dcs('g', 'd', "what=all,free=1,quiet=2")
}

/// Keyboard mode switch (keyboard §1). Encoded for completeness and tests; see the module
/// note for why grokbot-tui doesn't turn it on.
#[allow(dead_code)]
pub fn keyboard_mode(on: bool) -> String {
    dcs('k', 's', if on { "on=1,v=0" } else { "on=0" })
}

/// Reserve `r` in the buffer for a placement: blank and marked, so overlays drawn later
/// (which reset or overwrite the cells) are noticed by [`reserved`].
pub fn reserve(buf: &mut ratatui::buffer::Buffer, r: Rect) {
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ");
                // Still written (spaces under the picture), and an overlay's reset clears it.
                c.set_diff_option(ratatui::buffer::CellDiffOption::AlwaysUpdate);
            }
        }
    }
}

/// Whether every cell of `r` is still reserved after the frame was drawn.
pub fn reserved(buf: &ratatui::buffer::Buffer, r: Rect) -> bool {
    (r.y..r.bottom()).all(|y| {
        (r.x..r.right()).all(|x| {
            buf.cell((x, y)).is_some_and(|c| {
                c.diff_option == ratatui::buffer::CellDiffOption::AlwaysUpdate && c.symbol() == " "
            })
        })
    })
}

/// Per-run graphics state: uploaded images and what is placed on screen.
#[derive(Debug, Default)]
pub struct Gfx {
    pub caps: Caps,
    /// (source key, cols, rows) -> (image id, identity of the pixels uploaded).
    images: HashMap<(String, u16, u16), (u32, usize)>,
    next_id: u32,
    /// Bytes to write after the frame (uploads).
    pending: String,
    /// Placements requested by this frame: (rect, image id).
    pub frame: Vec<(Rect, u32)>,
    /// Placements on screen now.
    shown: Vec<(Rect, u32)>,
}

impl Gfx {
    pub fn new(caps: Caps) -> Self {
        Self {
            caps,
            next_id: 1,
            ..Self::default()
        }
    }

    /// The cell size in pixels (from the graphics query, else `fallback`).
    pub fn cell(&self, fallback: (u16, u16)) -> (u16, u16) {
        self.caps.cell.unwrap_or(fallback)
    }

    /// Ask to show `img` (identity `ident`, e.g. its Arc pointer) in `r` this frame.
    /// Uploads it (scaled to the cells' pixel size) when new or changed.
    pub fn show(
        &mut self,
        key: &str,
        ident: usize,
        img: &image::DynamicImage,
        r: Rect,
        cell: (u16, u16),
    ) {
        if r.width == 0 || r.height == 0 {
            return;
        }
        let k = (key.to_string(), r.width, r.height);
        let id = match self.images.get(&k) {
            Some(&(id, who)) if who == ident => id,
            other => {
                let id = other.map(|&(id, _)| id).unwrap_or_else(|| {
                    let id = self.next_id;
                    self.next_id = self.next_id.wrapping_add(1).max(1);
                    id
                });
                let max = self.caps.g_max_dim.clamp(1, MAX_UPLOAD_EDGE);
                let tw = (r.width as u32 * cell.0.max(1) as u32).clamp(1, max);
                let th = (r.height as u32 * cell.1.max(1) as u32).clamp(1, max);
                let scaled = if img.width() > tw || img.height() > th {
                    img.resize(tw, th, image::imageops::FilterType::Triangle)
                } else {
                    img.clone()
                };
                let rgba = scaled.to_rgba8();
                for m in upload(id, rgba.width(), rgba.height(), rgba.as_raw(), self.chunk()) {
                    self.pending.push_str(&wrap(&m, self.caps.mux));
                }
                self.images.insert(k, (id, ident));
                // Re-uploading replaces the image and drops its placements.
                self.shown.retain(|(_, i)| *i != id);
                id
            }
        };
        self.frame.push((r, id));
    }

    fn chunk(&self) -> usize {
        match self.caps.mux {
            // Keep each wrapped message inside one 768-byte screen piece is impractical;
            // small chunks keep pieces few.
            Mux::Screen => 512,
            _ => self.caps.max_chunk,
        }
    }

    /// Drop this frame's placements that something drew over (an overlay resets the
    /// reserved cells). `visible(rect)` says whether the rect is still ours.
    pub fn retain_visible(&mut self, visible: impl Fn(Rect) -> bool) {
        self.frame.retain(|(r, _)| visible(*r));
    }

    /// The terminal screen was cleared: nothing is placed any more.
    pub fn cleared(&mut self) {
        self.shown.clear();
    }

    /// Bytes to write after the frame: uploads, removed placements, new placements.
    /// Cursor saved and restored around placing. Resets the frame list.
    pub fn flush(&mut self) -> String {
        let mut out = std::mem::take(&mut self.pending);
        let frame = std::mem::take(&mut self.frame);
        let mux = self.caps.mux;
        let pid = |i: usize| i as u32 + 1;
        // Same image twice in a frame gets pids 1, 2, …
        let with_pid = |list: &[(Rect, u32)]| -> Vec<(Rect, u32, u32)> {
            let mut seen: HashMap<u32, usize> = HashMap::new();
            list.iter()
                .map(|&(r, id)| {
                    let n = seen.entry(id).or_default();
                    *n += 1;
                    (r, id, pid(*n - 1))
                })
                .collect()
        };
        let old = with_pid(&self.shown);
        let new = with_pid(&frame);
        for o in &old {
            if !new.contains(o) && !new.iter().any(|n| n.1 == o.1 && n.2 == o.2) {
                out.push_str(&wrap(&delete_placement(o.1, o.2), mux));
            }
        }
        let todo: Vec<_> = new.iter().filter(|n| !old.contains(n)).collect();
        if !todo.is_empty() {
            out.push_str("\x1b7");
            for &&(r, id, p) in &todo {
                let _ = write!(out, "\x1b[{};{}H", r.y + 1, r.x + 1);
                out.push_str(&wrap(&place(id, p, r.width, r.height), mux));
            }
            out.push_str("\x1b8");
        }
        self.shown = frame;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &[u8] = b"\x1bP=Fzcr;id=1,v=0,sub=gkps,raw=0,max_chunk=65536,g_mem=335544320,g_max_dim=10000,g_fmt=rgba:rgb:png,g_max_place=4096,g_fit=w,g_reflow=1,k=1,k_ime=0,k_release=1\x1b\\\x1bP=Fzgr;id=2,ok=1,mem_used=0,mem_max=335544320,images=0,cell_w=9,cell_h=18\x1b\\\x1b[?62;22c";

    #[test]
    fn caps_reply_parses_and_da1_alone_means_unavailable() {
        let c = parse_caps(REPLY, Mux::None).unwrap();
        assert!(c.graphics() && c.keyboard() && c.k_release && !c.raw);
        assert_eq!(c.max_chunk, 65536);
        assert_eq!(c.g_fmt, vec!["rgba", "rgb", "png"]);
        assert_eq!(c.cell, Some((9, 18)));
        assert_eq!(parse_caps(b"\x1b[?62;22c", Mux::None), None);
        assert_eq!(parse_caps(b"", Mux::None), None);
        // A reply for some other id isn't ours.
        assert_eq!(
            parse_caps(b"\x1bP=Fzcr;id=9,v=0,sub=g\x1b\\", Mux::None),
            None
        );
        // Graphics without keyboard.
        let c = parse_caps(b"\x1bP=Fzcr;id=1,v=0,sub=g\x1b\\\x1b[?1c", Mux::None).unwrap();
        assert!(c.graphics() && !c.keyboard());
    }

    #[test]
    fn query_and_tmux_screen_wrapping() {
        assert_eq!(
            detect_query(Mux::None),
            "\x1bP=Fzcq;id=1\x1b\\\x1bP=Fzgq;id=2\x1b\\\x1b[c"
        );
        assert_eq!(
            wrap("\x1bP=Fzcq;id=1\x1b\\", Mux::Tmux),
            "\x1bPtmux;\x1b\x1bP=Fzcq;id=1\x1b\x1b\\\x1b\\"
        );
        let long = "a".repeat(1000);
        assert_eq!(wrap(&long, Mux::Screen).matches("\x1bP").count(), 2);
        assert_eq!(Mux::from_vars(true, true), Mux::Tmux);
        assert_eq!(Mux::from_vars(false, true), Mux::Screen);
        assert_eq!(Mux::from_vars(false, false), Mux::None);
    }

    #[test]
    fn upload_matches_the_spec_example_and_chunks() {
        // graphics.md §5: 2x1 RGBA red, blue.
        let px = [0xFF, 0, 0, 0xFF, 0, 0, 0xFF, 0xFF];
        assert_eq!(
            upload(1, 2, 1, &px, 65536),
            vec![
                "\x1bP=Fzgu;id=1,fmt=rgba,w=2,h=1,size=8,enc=b64,len=12,more=0,quiet=2:/wAA/wAA//8=\x1b\\"
            ]
        );
        let big = vec![7u8; 3000]; // 4000 b64 chars
        let m = upload(5, 50, 15, &big, 1024);
        assert_eq!(m.len(), 4);
        assert!(m[0].contains("size=3000,enc=b64,len=1024,more=1"));
        assert!(m[1].starts_with("\x1bP=Fzgu;id=5,enc=b64,len=1024,more=1,quiet=2:"));
        assert!(m[3].contains("len=928,more=0"));
        assert_eq!(
            place(1, 1, 4, 2),
            "\x1bP=Fzgp;id=1,pid=1,cols=4,rows=2,move=0,quiet=2\x1b\\"
        );
        assert_eq!(delete_all(), "\x1bP=Fzgd;what=all,free=1,quiet=2\x1b\\");
        assert_eq!(keyboard_mode(true), "\x1bP=Fzks;on=1,v=0\x1b\\");
    }

    #[test]
    fn overlays_unreserve_cells() {
        use ratatui::buffer::Buffer;
        let mut b = Buffer::empty(Rect::new(0, 0, 10, 5));
        let r = Rect::new(1, 1, 3, 2);
        reserve(&mut b, r);
        assert!(reserved(&b, r));
        b[(2, 2)].reset();
        assert!(!reserved(&b, r));
        reserve(&mut b, r);
        b[(1, 1)].set_symbol("x");
        assert!(!reserved(&b, r));
    }

    #[test]
    fn gfx_uploads_once_places_moves_and_deletes() {
        let mut g = Gfx::new(parse_caps(REPLY, Mux::None).unwrap());
        let img = image::DynamicImage::new_rgba8(20, 20);
        let r = Rect::new(3, 4, 2, 1);
        g.show("a", 1, &img, r, (9, 18));
        let out = g.flush();
        assert_eq!(out.matches("Fzgu;").count(), 1);
        assert!(out.contains("\x1b7\x1b[5;4H\x1bP=Fzgp;id=1,pid=1,cols=2,rows=1"));
        // Same frame again: nothing to send.
        g.show("a", 1, &img, r, (9, 18));
        assert_eq!(g.flush(), "");
        // Moved: placed again (same pid moves it), no upload.
        g.show("a", 1, &img, Rect::new(3, 2, 2, 1), (9, 18));
        let out = g.flush();
        assert!(!out.contains("Fzgu;") && out.contains("\x1b[3;4H"));
        assert!(!out.contains("Fzgd;"));
        // New pixels (video frame): re-upload the same id.
        g.show("a", 2, &img, Rect::new(3, 2, 2, 1), (9, 18));
        let out = g.flush();
        assert!(out.contains("Fzgu;id=1,") && out.contains("Fzgp;id=1"));
        // Gone: deleted.
        let out = g.flush();
        assert!(out.contains("Fzgd;what=image,id=1,pid=1"));
        // After a clear everything is placed again.
        g.show("a", 2, &img, Rect::new(3, 2, 2, 1), (9, 18));
        g.flush();
        g.cleared();
        g.show("a", 2, &img, Rect::new(3, 2, 2, 1), (9, 18));
        assert!(g.flush().contains("Fzgp;"));
        // Through tmux everything is wrapped.
        let mut t = Gfx::new(parse_caps(REPLY, Mux::Tmux).unwrap());
        t.show("b", 1, &img, r, (9, 18));
        let out = t.flush();
        assert!(out.contains("\x1bPtmux;\x1b\x1bP=Fzgu;"));
    }
}
