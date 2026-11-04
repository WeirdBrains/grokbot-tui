//! All startup terminal queries at once. One burst — OSC 11 background, Fuzzball
//! capability + graphics query, kitty graphics, kitty keyboard (`CSI ? u`), cell size
//! (`CSI 16 t`) and a cursor position report — is followed by DA1 (`CSI c`), which every
//! VT terminal answers and answers last. A single reader collects the replies under one
//! deadline (`graphics.detect_timeout_ms`, clamped to 1–5 s) and routes each by its type.
//! It stops when DA1 arrives (in tmux: when everything is in, or a short grace after DA1),
//! or at the deadline. A terminal that answers nothing costs one deadline, not one per
//! query; a slow one costs its slowest reply, not the sum. Replies that arrive later are
//! flushed by [`crate::bounded::discard_pending_input`].

use crate::fuzzball::{self, Caps, Mux};
use crate::theme::Variant;
use std::time::{Duration, Instant};

/// After DA1 under tmux, how long to wait for passthrough replies that trail it.
const TMUX_GRACE_MS: u64 = 80;
/// Never buffer more than this.
const MAX_BUF: usize = 16 * 1024;

/// Which optional queries go into the burst.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Want {
    pub fuzzball: bool,
    pub images: bool,
}

impl Want {
    pub fn from_env() -> Self {
        Self {
            fuzzball: !std::env::var_os("GROKBOT_TUI_FUZZBALL").is_some_and(|v| v == "off"),
            images: !std::env::var_os("GROKBOT_TUI_IMAGES").is_some_and(|v| v == "halfblocks"),
        }
    }
}

/// What the terminal said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Answers {
    /// DA1 came back: the terminal answers queries.
    pub da1: bool,
    /// DA1 lists 4 (sixel graphics).
    pub sixel: bool,
    pub background: Option<Variant>,
    pub fuzz: Option<Caps>,
    pub kitty_graphics: bool,
    /// Kitty graphics accepts zlib-compressed (`o=z`) transmissions.
    pub kitty_compression: bool,
    /// The compression probe got any answer (OK or an error).
    pub kitty_compression_replied: bool,
    /// `CSI ? flags u`: the kitty keyboard protocol is supported.
    pub kitty_keys: bool,
    /// Cell size in pixels (width, height) from `CSI 6 ; h ; w t`.
    pub cell: Option<(u16, u16)>,
    pub cpr: bool,
}

/// Kitty `o=z` (zlib) transmission probe, as ratatui-image's `kitty_compression` option
/// sends it: the one-pixel query with its payload deflated. `eJxjYGAAAAADAAE=` is
/// base64(zlib([0, 0, 0])). A terminal that can inflate answers `Gi=32;OK`.
pub const KITTY_COMPRESSION_QUERY: &str =
    "\x1b_Gi=32,s=1,v=1,a=q,t=d,f=24,o=z;eJxjYGAAAAADAAE=\x1b\\";

/// The burst, DA1 last.
pub fn query(mux: Mux, want: Want) -> String {
    let mut s = String::from("\x1b]11;?\x1b\\");
    if want.fuzzball {
        // detect_query ends in DA1; drop it, DA1 goes last.
        let fq = fuzzball::detect_query(mux);
        s.push_str(fq.strip_suffix("\x1b[c").unwrap_or(&fq));
    }
    if want.images {
        s.push_str(&fuzzball::wrap(
            "\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\",
            mux,
        ));
        s.push_str(&fuzzball::wrap(KITTY_COMPRESSION_QUERY, mux));
    }
    s.push_str("\x1b[?u\x1b[16t\x1b[6n\x1b[c");
    s
}

/// One escape sequence from the reply stream.
#[derive(Debug, PartialEq)]
enum Seq<'a> {
    /// `ESC [ <private> <params> <final>`
    Csi(Option<u8>, &'a [u8], u8),
    /// `ESC ] … (BEL | ST)`
    Osc(&'a [u8]),
    /// `ESC _ … ST`
    Apc(&'a [u8]),
    /// `ESC P … ST`
    Dcs,
}

/// Split `buf` into complete sequences (stray bytes and an unfinished tail are skipped).
fn sequences(buf: &[u8]) -> Vec<Seq<'_>> {
    let mut out = vec![];
    let mut i = 0;
    let st_end = |from: usize, bel: bool| -> Option<(usize, usize)> {
        let mut j = from;
        while j < buf.len() {
            if bel && buf[j] == 0x07 {
                return Some((j, j + 1));
            }
            if buf[j] == 0x1b && buf.get(j + 1) == Some(&b'\\') {
                return Some((j, j + 2));
            }
            j += 1;
        }
        None
    };
    while i + 1 < buf.len() {
        if buf[i] != 0x1b {
            i += 1;
            continue;
        }
        match buf[i + 1] {
            b'[' => {
                let mut j = i + 2;
                let private = buf.get(j).copied().filter(|c| b"?<=>".contains(c));
                if private.is_some() {
                    j += 1;
                }
                let p0 = j;
                while j < buf.len() && (0x20..0x40).contains(&buf[j]) {
                    j += 1;
                }
                let Some(&f) = buf.get(j) else { break };
                out.push(Seq::Csi(private, &buf[p0..j], f));
                i = j + 1;
            }
            k @ (b']' | b'_' | b'P') => {
                let Some((end, next)) = st_end(i + 2, k == b']') else {
                    break;
                };
                let body = &buf[i + 2..end];
                out.push(match k {
                    b']' => Seq::Osc(body),
                    b'_' => Seq::Apc(body),
                    _ => Seq::Dcs,
                });
                i = next;
            }
            _ => i += 1,
        }
    }
    out
}

fn nums(p: &[u8]) -> Vec<u32> {
    std::str::from_utf8(p)
        .unwrap_or_default()
        .split(';')
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// Route every reply in `buf` by its type.
pub fn parse(buf: &[u8], mux: Mux) -> Answers {
    let mut a = Answers::default();
    for s in sequences(buf) {
        match s {
            Seq::Csi(Some(b'?'), p, b'c') => {
                a.da1 = true;
                a.sixel = nums(p).iter().skip(1).any(|&n| n == 4);
            }
            Seq::Csi(Some(b'?'), _, b'u') => a.kitty_keys = true,
            Seq::Csi(None, p, b't') => {
                if let [6, h, w, ..] = nums(p)[..]
                    && h > 0
                    && w > 0
                {
                    a.cell = Some((w.min(u16::MAX as u32) as u16, h.min(u16::MAX as u32) as u16));
                }
            }
            Seq::Csi(None, p, b'R') if nums(p).len() == 2 => a.cpr = true,
            Seq::Osc(body) if body.starts_with(b"11;") => {
                let mut r = b"\x1b]".to_vec();
                r.extend_from_slice(body);
                r.extend_from_slice(b"\x1b\\");
                a.background = crate::appearance::parse_osc11(&r);
            }
            Seq::Apc(body) if body.starts_with(b"Gi=31") => {
                a.kitty_graphics = body.windows(3).any(|w| w == b";OK");
            }
            Seq::Apc(body) if body.starts_with(b"Gi=32") => {
                a.kitty_compression_replied = true;
                a.kitty_compression = body.windows(3).any(|w| w == b";OK");
            }
            _ => {}
        }
    }
    a.fuzz = fuzzball::parse_caps(buf, mux);
    a
}

/// Every wanted reply is in (only some terminals send them all).
fn all_in(a: &Answers, want: Want) -> bool {
    a.da1
        && a.background.is_some()
        && a.kitty_keys
        && a.cell.is_some()
        && a.cpr
        && (!want.fuzzball || a.fuzz.is_some())
        && (!want.images || (a.kitty_graphics && a.kitty_compression_replied))
}

/// Read replies from `fd` until DA1 (tmux: everything, or a grace after DA1) or `wait`
/// runs out. Returns the bytes read.
#[cfg(unix)]
pub fn read_replies(fd: std::os::fd::RawFd, wait: Duration, mux: Mux, want: Want) -> Vec<u8> {
    let mut deadline = Instant::now() + wait;
    let mut grace = false;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while buf.len() < MAX_BUF {
        let a = parse(&buf, mux);
        if a.da1 {
            if mux != Mux::Tmux || all_in(&a, want) {
                break;
            }
            if !grace {
                grace = true;
                deadline = deadline.min(Instant::now() + Duration::from_millis(TMUX_GRACE_MS));
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as i32) };
        if ready < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if ready <= 0 {
            break;
        }
        // SAFETY: reading into a local buffer of the given length.
        let n = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if n <= 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
    }
    buf
}

/// Send the burst to the controlling terminal and collect the answers within `wait`.
/// Call in raw mode before anything else reads input.
#[cfg(unix)]
pub fn probe(wait: Duration) -> Answers {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    let mux = Mux::from_env();
    let want = Want::from_env();
    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Answers::default();
    };
    if tty
        .write_all(query(mux, want).as_bytes())
        .and_then(|_| tty.flush())
        .is_err()
    {
        return Answers::default();
    }
    let buf = read_replies(tty.as_raw_fd(), wait, mux, want);
    let mut a = parse(&buf, mux);
    if !want.fuzzball {
        a.fuzz = None;
    }
    a
}

/// Non-unix: no burst; the terminal is assumed to answer and nothing is detected.
#[cfg(not(unix))]
pub fn probe(_wait: Duration) -> Answers {
    Answers {
        da1: true,
        ..Answers::default()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const ALL: Want = Want {
        fuzzball: true,
        images: true,
    };

    #[test]
    fn burst_has_every_query_and_ends_with_da1() {
        let q = query(Mux::None, ALL);
        for part in [
            "\x1b]11;?",
            "\x1bP=Fz",
            "\x1b_Gi=31",
            "\x1b_Gi=32,s=1,v=1,a=q,t=d,f=24,o=z;",
            "\x1b[?u",
            "\x1b[16t",
            "\x1b[6n",
        ] {
            assert!(q.contains(part), "{part:?} in {q:?}");
        }
        assert!(q.ends_with("\x1b[c"));
        assert_eq!(q.matches("\x1b[c").count(), 1, "one DA1");
        let q = query(
            Mux::None,
            Want {
                fuzzball: false,
                images: false,
            },
        );
        assert!(!q.contains("=Fz") && !q.contains("_G"));
    }

    #[test]
    fn replies_are_routed_by_type_in_any_order() {
        let buf = b"\x1b[3;7R\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b]11;rgb:ffff/ffff/ffff\x07\x1b[?15u\x1b[?62;4;22c";
        let a = parse(buf, Mux::None);
        assert!(a.da1 && a.sixel && a.kitty_graphics && a.kitty_keys && a.cpr);
        assert_eq!(a.cell, Some((10, 20)));
        assert_eq!(a.background, Some(Variant::Light));
        // A kitty-keys reply followed by text with a 'c' isn't DA1.
        let a = parse(b"\x1b[?1u abc", Mux::None);
        assert!(a.kitty_keys && !a.da1);
        // Only DA1: a plain terminal.
        let a = parse(b"\x1b[?1;2c", Mux::None);
        assert!(a.da1 && !a.sixel && !a.kitty_keys && a.background.is_none());
        // Kitty graphics error.
        assert!(!parse(b"\x1b_Gi=31;ENOTSUPPORTED\x1b\\", Mux::None).kitty_graphics);
    }

    #[test]
    fn kitty_compression_reply_is_parsed() {
        let a = parse(
            b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[?62;22c",
            Mux::None,
        );
        assert!(a.kitty_graphics && a.kitty_compression && a.kitty_compression_replied);
        // Kitty graphics without inflate: the probe is refused, images stay uncompressed.
        let a = parse(
            b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;EINVAL:bad\x1b\\\x1b[?62;22c",
            Mux::None,
        );
        assert!(a.kitty_graphics && !a.kitty_compression && a.kitty_compression_replied);
        // No reply at all: uncompressed.
        let a = parse(b"\x1b[?62;22c", Mux::None);
        assert!(!a.kitty_compression && !a.kitty_compression_replied);
        // An OK for the plain probe isn't one for the compressed probe, and vice versa.
        let a = parse(b"\x1b_Gi=32;OK\x1b\\", Mux::None);
        assert!(a.kitty_compression && !a.kitty_graphics);
    }

    #[test]
    fn kitty_compression_probe_payload_is_zlib_of_one_black_pixel() {
        // zlib: CMF 0x78, FLG 0x9c (header checksum), then deflate of [0, 0, 0] and the
        // Adler-32 of [0, 0, 0] (0x00030001).
        let b64 = KITTY_COMPRESSION_QUERY
            .split_once(';')
            .and_then(|(_, r)| r.strip_suffix("\x1b\\"))
            .unwrap();
        assert_eq!(b64, "eJxjYGAAAAADAAE=");
        let bytes = b64_decode(b64);
        assert_eq!(&bytes[..2], &[0x78, 0x9c]);
        assert_eq!(((bytes[0] as u16) << 8 | bytes[1] as u16) % 31, 0);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 3, 0, 1]);
    }

    fn b64_decode(s: &str) -> Vec<u8> {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = vec![];
        let (mut acc, mut bits) = (0u32, 0);
        for c in s.bytes().filter(|&c| c != b'=') {
            acc = acc << 6 | T.iter().position(|&t| t == c).unwrap() as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }

    fn pipe() -> (std::os::fd::RawFd, std::fs::File) {
        use std::os::fd::FromRawFd;
        let mut fds = [0; 2];
        // SAFETY: valid out-array of two fds.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: we own the write end.
        (fds[0], unsafe { std::fs::File::from_raw_fd(fds[1]) })
    }

    #[test]
    fn a_silent_terminal_costs_one_deadline() {
        let (r, _w) = pipe();
        let t = Instant::now();
        let buf = read_replies(r, Duration::from_millis(300), Mux::None, ALL);
        let took = t.elapsed();
        assert!(buf.is_empty());
        assert!(
            took >= Duration::from_millis(290) && took < Duration::from_millis(1000),
            "{took:?}"
        );
        unsafe { libc::close(r) };
    }

    /// A slow terminal answers each query after its own delay (all in flight at once): the
    /// total is the slowest reply, not the sum.
    #[test]
    fn a_slow_terminal_costs_its_slowest_reply_not_the_sum() {
        use std::io::Write;
        let (r, mut w) = pipe();
        let replies: [(u64, &[u8]); 5] = [
            (150, b"\x1b[5;1R"),
            (250, b"\x1b[?1u"),
            (300, b"\x1b]11;rgb:0000/0000/0000\x1b\\"),
            (350, b"\x1b[6;18;9t"),
            (400, b"\x1b[?62;22c"),
        ];
        let sum: u64 = replies.iter().map(|(d, _)| d).sum();
        let t = Instant::now();
        std::thread::spawn(move || {
            for (d, b) in replies {
                std::thread::sleep(Duration::from_millis(d).saturating_sub(t.elapsed()));
                let _ = w.write_all(b);
            }
        });
        let buf = read_replies(r, Duration::from_secs(3), Mux::None, ALL);
        let took = t.elapsed();
        let a = parse(&buf, Mux::None);
        assert!(a.da1 && a.kitty_keys && a.cpr && a.background == Some(Variant::Dark));
        assert_eq!(a.cell, Some((9, 18)));
        assert!(took >= Duration::from_millis(390), "{took:?}");
        assert!(
            took < Duration::from_millis(1000),
            "{took:?} (sum would be {sum} ms)"
        );
        unsafe { libc::close(r) };
    }
}
