//! Detecting whether the terminal has a light or dark background, for the "system"
//! theme mode ([`crate::theme::Mode::System`]).
//!
//! Tried in order, once at startup:
//! 1. the OSC 11 reply (background color) from the startup burst ([`crate::probe`]);
//! 2. the `COLORFGBG` environment variable (`"15;0"` = light text on black).
use crate::theme::Variant;

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
}
