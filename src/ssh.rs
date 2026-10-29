//! SSH session detection. In an SSH session "open in the system browser" would open a
//! browser on the remote machine, so links default to the built-in text browser.

/// Detection from an env lookup (and, on Linux, the parent process chain).
pub fn detect_with(env: &dyn Fn(&str) -> Option<String>, check_parents: bool) -> bool {
    match env("GROKBOT_TUI_FORCE_SSH").as_deref().map(str::trim) {
        Some("1" | "true" | "yes" | "on") => return true,
        Some("0" | "false" | "no" | "off") => return false,
        _ => {}
    }
    if ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|k| env(k).is_some_and(|v| !v.trim().is_empty()))
    {
        return true;
    }
    check_parents && parent_is_sshd()
}

/// Real detection (environment, then parent processes on Linux).
pub fn detect() -> bool {
    detect_with(&|k| std::env::var(k).ok(), true)
}

#[cfg(target_os = "linux")]
fn parent_is_sshd() -> bool {
    let mut pid = std::process::id();
    for _ in 0..32 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        // "pid (comm) state ppid …"; comm may contain spaces/parens, so split at the last ')'.
        let Some((head, rest)) = stat.rsplit_once(')') else {
            return false;
        };
        let comm = head.split_once('(').map(|(_, c)| c).unwrap_or("");
        if comm == "sshd" || comm.starts_with("sshd-") || comm == "sshd-session" {
            return true;
        }
        let ppid: u32 = rest
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        if ppid <= 1 {
            return false;
        }
        pid = ppid;
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn parent_is_sshd() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    #[test]
    fn detects_ssh_vars_and_honours_override() {
        assert!(!detect_with(&env(&[]), false));
        assert!(detect_with(
            &env(&[("SSH_CONNECTION", "1.2.3.4 5 6.7.8.9 22")]),
            false
        ));
        assert!(detect_with(&env(&[("SSH_CLIENT", "1.2.3.4 5 22")]), false));
        assert!(detect_with(&env(&[("SSH_TTY", "/dev/pts/3")]), false));
        assert!(!detect_with(&env(&[("SSH_TTY", "  ")]), false));
        assert!(!detect_with(
            &env(&[("SSH_TTY", "/dev/pts/3"), ("GROKBOT_TUI_FORCE_SSH", "0")]),
            false
        ));
        assert!(detect_with(&env(&[("GROKBOT_TUI_FORCE_SSH", "1")]), false));
        assert!(!detect_with(
            &env(&[("GROKBOT_TUI_FORCE_SSH", "maybe")]),
            false
        ));
    }
}
