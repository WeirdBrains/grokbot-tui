//! Every environment variable grokbot-tui reads (one list for the man page, the docs and
//! a test that keeps them, the in-app `/help env` and the source in step).

/// (name, description).
pub const ENV_VARS: &[(&str, &str)] = &[
    (
        "GROKBOT_TUI_TOKEN",
        "Gateway token override for the startup account (never saved).",
    ),
    (
        "GROKBOT_TUI_GATEWAY_URL",
        "Gateway URL for that token (else the stored URL).",
    ),
    (
        "GROKBOT_TUI_GATEWAY_HEADERS",
        "JSON object of extra gateway headers.",
    ),
    (
        "GROKBOT_TUI_CURSOR_API_BASE_URL",
        "Cursor backend for the EnsureSandBox exchange (must be *.cursor.sh / *.cursor.com).",
    ),
    (
        "GROKBOT_TUI_CURSOR_CLIENT_VERSION",
        "x-cursor-client-version (env > pref cursor.client_version > detected app > 0.70.2).",
    ),
    (
        "GROKBOT_TUI_CURSOR_CLIENT_TYPE",
        "x-cursor-client-type (default sand).",
    ),
    (
        "GROKBOT_TUI_SAND_BOX_NAMESPACE",
        "x-sand-box-namespace (default prod).",
    ),
    (
        "GROKBOT_TUI_ACCOUNT",
        "Starting account alias (same as --account).",
    ),
    (
        "GROKBOT_TUI_ALLOW_LOCAL_GATEWAY",
        "Set to 1 to allow http(s) loopback gateways (development).",
    ),
    (
        "GROKBOT_TUI_APP_DIR",
        "Grok Bot desktop install dir, for version detection.",
    ),
    (
        "GROKBOT_TUI_MAX_RESPONSE_BYTES",
        "Cap on one gateway response body in bytes (default 64 MiB).",
    ),
    (
        "GROKBOT_TUI_NO_THEME_DOWNLOAD",
        "Set to 1 to never download the default theme.",
    ),
    (
        "GROKBOT_TUI_THEME_URL",
        "Download themes from here instead of the release tag (<url><name>.toml).",
    ),
    (
        "GROKBOT_TUI_NO_HYPERLINKS",
        "Set (to anything) to turn off OSC 8 hyperlinks.",
    ),
    (
        "GROKBOT_TUI_FORCE_SSH",
        "0 or 1: override SSH detection (links use the built-in browser over SSH).",
    ),
    (
        "GROKBOT_TUI_FUZZBALL",
        "off: skip the Fuzzball capability query at startup.",
    ),
    (
        "GROKBOT_TUI_IMAGES",
        "halfblocks: skip the terminal graphics query and draw images with half blocks.",
    ),
];

/// Variables only the opt-in live tests read (not user-facing).
#[cfg(test)]
pub const TEST_ONLY_PREFIX: &str = "GROKBOT_TUI_LIVE_";

#[cfg(test)]
mod tests {
    use super::*;

    /// Every GROKBOT_TUI_* name in the source is in ENV_VARS, and each is documented in
    /// `/help env`, the man page and docs/config.md.
    #[test]
    fn every_env_var_is_listed_and_documented() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|x| x.to_str()) != Some("rs") || p.ends_with("envvars.rs")
                {
                    continue;
                }
                let s = std::fs::read_to_string(&p).unwrap();
                for (i, _) in s.match_indices("GROKBOT_TUI_") {
                    let name: String = s[i..]
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                        .collect();
                    if name.len() > "GROKBOT_TUI_".len()
                        && !name.ends_with('_')
                        && !name.starts_with(TEST_ONLY_PREFIX)
                        && name != "GROKBOT_TUI_X"
                    {
                        seen.insert(name);
                    }
                }
            }
        }
        for n in &seen {
            assert!(
                ENV_VARS.iter().any(|(v, _)| v == n),
                "{n} is read but not in ENV_VARS"
            );
        }
        let help = crate::help::topic("env").unwrap().body;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let man = std::fs::read_to_string(root.join("man/grokbot-tui.1")).unwrap();
        let cfg = std::fs::read_to_string(root.join("docs/config.md")).unwrap();
        for (v, _) in ENV_VARS {
            assert!(help.contains(v), "/help env misses {v}");
            assert!(
                man.contains(&v.replace('_', "\\_")) || man.contains(v),
                "man page misses {v}"
            );
            assert!(cfg.contains(v), "docs/config.md misses {v}");
        }
    }
}
