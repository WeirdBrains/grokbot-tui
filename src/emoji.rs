//! Emoji: `:shortcode:` lookup and completion, and the stable keys local reactions use.
//!
//! The gateway API (grokbot_api 1.1) has no reaction endpoints and transcript entries
//! carry no reaction data, so reactions are stored locally in `context.db` (schema v10)
//! and never sent.

/// Characters allowed in a shortcode name (GitHub/Slack style).
fn is_code_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// The `:partial` token ending at `cur`: (byte start of the ':', the partial name).
/// Needs at least one name char, and the ':' must start the input or follow whitespace
/// (so `http://x` or `12:30` don't trigger).
pub fn shortcode_at(input: &str, cur: usize) -> Option<(usize, &str)> {
    let cur = cur.min(input.len());
    if !input.is_char_boundary(cur) {
        return None;
    }
    let before = &input[..cur];
    let colon = before.rfind(':')?;
    let name = &before[colon + 1..];
    if name.is_empty() || !name.chars().all(is_code_char) {
        return None;
    }
    let ok_start = before[..colon]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    ok_start.then_some((colon, name))
}

/// The emoji for a shortcode name (without colons).
pub fn lookup(name: &str) -> Option<&'static str> {
    emojis::get_by_shortcode(name).map(|e| e.as_str())
}

/// Emoji whose shortcodes match `query` (prefix matches first, then substring), as
/// (shortcode, emoji). At most `limit`.
pub fn search(query: &str, limit: usize) -> Vec<(&'static str, &'static str)> {
    let q = query.to_lowercase();
    let mut prefix = vec![];
    let mut sub = vec![];
    for e in emojis::iter() {
        for code in e.shortcodes() {
            if code.starts_with(&q) {
                prefix.push((code, e.as_str()));
            } else if code.contains(&q) {
                sub.push((code, e.as_str()));
            }
        }
    }
    prefix.sort_by_key(|(c, _)| c.len());
    prefix.extend(sub);
    prefix.truncate(limit);
    prefix
}

/// Replace every complete `:name:` with its emoji (unknown names are left as typed).
pub fn expand(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(':') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let end = after.find(|c: char| !is_code_char(c));
        match end {
            Some(j) if j > 0 && after[j..].starts_with(':') => {
                if let Some(e) = lookup(&after[..j]) {
                    out.push_str(e);
                    rest = &after[j + 1..];
                    continue;
                }
                out.push(':');
                rest = after;
            }
            _ => {
                out.push(':');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Accept an emoji argument: `:thumbsup:`, `thumbsup` or a literal emoji.
pub fn parse_arg(arg: &str) -> Option<String> {
    let a = arg.trim();
    if a.is_empty() {
        return None;
    }
    let name = a.trim_matches(':');
    if let Some(e) = lookup(name) {
        return Some(e.to_string());
    }
    if emojis::get(a).is_some() {
        return Some(a.to_string());
    }
    None
}

/// A stable key for a message (FNV-1a over kind and text) so local reactions survive
/// restarts and transcript reloads.
pub fn message_key(kind: u8, text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in std::iter::once(kind).chain(text.bytes()) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcodes() {
        assert_eq!(shortcode_at("hi :thu", 7), Some((3, "thu")));
        assert_eq!(shortcode_at(":+1", 3), Some((0, "+1")));
        assert_eq!(shortcode_at("12:30", 5), None);
        assert_eq!(shortcode_at("http://x", 8), None);
        assert_eq!(shortcode_at("hi :", 4), None);
        assert_eq!(lookup("thumbsup"), Some("👍"));
        assert_eq!(search("thumbsu", 5)[0], ("thumbsup", "👍"));
        assert_eq!(expand("ok :tada: at 12:30 :nope:"), "ok 🎉 at 12:30 :nope:");
        assert_eq!(parse_arg(":heart:").as_deref(), Some("❤️"));
        assert_eq!(parse_arg("🎉").as_deref(), Some("🎉"));
        assert_eq!(parse_arg("notanemojiname"), None);
        assert_eq!(message_key(3, "a"), message_key(3, "a"));
        assert_ne!(message_key(3, "a"), message_key(2, "a"));
    }
}
