//! Detecting whether the terminal has a light or dark background, for the "system"
//! theme mode ([`crate::theme::Mode::System`]).
//!
//! Tried in order, once at startup (before the event loop reads input):
//! 1. an OSC 11 query (`ESC ] 11 ; ? ST`): the terminal answers with its background color;
//! 2. the `COLORFGBG` environment variable (`"15;0"` = light text on black).
//!
//! The query is followed by a DA1 request (`ESC [ c`), which every terminal answers, so a
//! terminal that ignores OSC 11 doesn't make us wait for the timeout.
use crate::theme::Variant;

/// How long to wait for the terminal's answers.
#[cfg(unix)]
const QUERY_TIMEOUT_MS: i32 = 200;

/// Detect the terminal background. Call while the terminal is in raw mode and before
/// anything else reads stdin. `None` when it can't be told.
pub fn detect() -> Option<Variant> {
    query_osc11()
        .and_then(|reply| parse_osc11(&reply))
        .or_else(|| {
            std::env::var("COLORFGBG")
                .ok()
                .and_then(|v| parse_colorfgbg(&v))
        })
}

/// `COLORFGBG` is `"<fg>;<bg>"` (sometimes `"<fg>;default;<bg>"`) with ANSI color numbers.
/// Background 7 (light gray) and 9–15 (bright colors, incl. 15 = white) are light;
/// 0–6 and 8 are dark.
pub fn parse_colorfgbg(v: &str) -> Option<Variant> {
    let bg: u8 = v.rsplit(';').next()?.trim().parse().ok()?;
    Some(match bg {
        7 | 9..=15 => Variant::Light,
        0..=6 | 8 => Variant::Dark,
        _ => return None,
    })
}

/// Parse an OSC 11 reply such as `ESC ] 11 ; rgb:1e1e/1e1e/2e2e ESC \` (or BEL-terminated;
/// 1–4 hex digits per channel) into a variant by its relative luminance.
pub fn parse_osc11(reply: &[u8]) -> Option<Variant> {
    let s = String::from_utf8_lossy(reply);
    let start = s.find("]11;")? + 4;
    let rest = &s[start..];
    let end = rest.find(['\x1b', '\x07']).unwrap_or(rest.len());
    let spec = rest[..end].trim();
    let spec = spec
        .strip_prefix("rgba:")
        .or_else(|| spec.strip_prefix("rgb:"))?;
    let mut ch = spec.split('/').map(|h| {
        let h = h.trim();
        if h.is_empty() || h.len() > 4 {
            return None;
        }
        let v = u32::from_str_radix(h, 16).ok()?;
        let max = (1u32 << (4 * h.len() as u32)) - 1;
        Some(v as f64 / max as f64)
    });
    let (r, g, b) = (ch.next()??, ch.next()??, ch.next()??);
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    Some(if luminance > 0.5 {
        Variant::Light
    } else {
        Variant::Dark
    })
}

/// True once `buf` holds a complete DA1 answer (`ESC [ ? … c`).
#[cfg_attr(not(unix), allow(dead_code))]
fn has_da1(buf: &[u8]) -> bool {
    buf.windows(3)
        .position(|w| w == b"\x1b[?")
        .is_some_and(|i| buf[i + 3..].contains(&b'c'))
}

/// Send OSC 11 + DA1 to the controlling terminal and collect the answers.
#[cfg(unix)]
fn query_osc11() -> Option<Vec<u8>> {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    tty.write_all(b"\x1b]11;?\x1b\\\x1b[c").ok()?;
    tty.flush().ok()?;
    let deadline = Instant::now() + Duration::from_millis(QUERY_TIMEOUT_MS as u64);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    while !has_da1(&buf) && buf.len() < 4096 {
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
    (!buf.is_empty()).then_some(buf)
}

#[cfg(not(unix))]
fn query_osc11() -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_background_decides() {
        assert_eq!(parse_colorfgbg("15;0"), Some(Variant::Dark));
        assert_eq!(parse_colorfgbg("0;15"), Some(Variant::Light));
        assert_eq!(parse_colorfgbg("0;7"), Some(Variant::Light));
        assert_eq!(parse_colorfgbg("7;8"), Some(Variant::Dark));
        assert_eq!(parse_colorfgbg("15;default;0"), Some(Variant::Dark));
        assert_eq!(parse_colorfgbg("default;default"), None);
        assert_eq!(parse_colorfgbg(""), None);
        assert_eq!(parse_colorfgbg("0;200"), None);
    }

    #[test]
    fn osc11_replies_parse_by_luminance() {
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\"),
            Some(Variant::Dark)
        );
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
            Some(Variant::Light)
        );
        // Short channels, rgba, and a DA1 answer after it.
        assert_eq!(
            parse_osc11(b"\x1b]11;rgb:fa/f0/e6\x1b\\\x1b[?62;22c"),
            Some(Variant::Light)
        );
        assert_eq!(
            parse_osc11(b"\x1b]11;rgba:0000/0000/0000/ffff\x1b\\"),
            Some(Variant::Dark)
        );
        // Only DA1 (terminal ignored OSC 11), garbage, or malformed channels.
        assert_eq!(parse_osc11(b"\x1b[?1;2c"), None);
        assert_eq!(parse_osc11(b"nonsense"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:zz/00/00\x07"), None);
        assert_eq!(parse_osc11(b"\x1b]11;rgb:00000/0/0\x07"), None);
    }

    #[test]
    fn da1_ends_the_wait() {
        assert!(!has_da1(b"\x1b]11;rgb:0/0/0\x1b\\"));
        assert!(!has_da1(b"\x1b[?62;22"));
        assert!(has_da1(b"\x1b]11;rgb:0/0/0\x1b\\\x1b[?62;22c"));
    }
}
