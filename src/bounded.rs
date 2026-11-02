//! Bounded startup terminal queries. Each query (OSC 11, Fuzzball, kitty graphics /
//! XTVERSION / cell size / DSR, kitty keyboard) runs on a helper thread and the caller
//! waits at most a fixed time, so a terminal that never answers (no console attached,
//! ConPTY dropping replies) can't keep the UI from drawing. On timeout the caller uses
//! a safe default (dark/unknown background, no Fuzzball, halfblocks, legacy keys).

use std::time::Duration;

/// Smallest wait for one startup query.
pub const MIN_WAIT_MS: u64 = 1000;
/// Largest wait for one startup query.
pub const MAX_WAIT_MS: u64 = 5000;

/// The per-query bound: `graphics.detect_timeout_ms`, clamped to 1–5 s.
pub fn startup_wait(detect_timeout_ms: u64) -> Duration {
    Duration::from_millis(detect_timeout_ms.clamp(MIN_WAIT_MS, MAX_WAIT_MS))
}

/// Run `f` on a helper thread; `None` if it hasn't returned within `wait`. A query that
/// times out is left running (it may still finish later and is then discarded).
pub fn run<T: Send + 'static>(wait: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("startup-query".into())
        .spawn(move || {
            let _ = tx.send(f());
        });
    if spawned.is_err() {
        return None;
    }
    rx.recv_timeout(wait).ok()
}

/// One shared deadline for all startup queries.
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    deadline: std::time::Instant,
}

impl Budget {
    pub fn new(total: Duration) -> Self {
        Budget {
            deadline: std::time::Instant::now() + total,
        }
    }
    /// Time left (zero once spent).
    pub fn left(&self) -> Duration {
        self.deadline
            .saturating_duration_since(std::time::Instant::now())
    }
    /// Run `f` within what is left of the budget; `None` (without running it) when spent.
    pub fn run<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let left = self.left();
        if left.is_zero() {
            return None;
        }
        run(left, f)
    }
}

/// Send DA1 (`ESC [ c`, which every VT terminal answers) and wait at most `wait` for the
/// answer. `false` means the terminal doesn't answer queries at all, so the others are
/// skipped instead of each waiting for its own timeout. Call in raw mode, before anything
/// else reads input. Non-unix: assumed to answer (the other queries stay bounded).
#[cfg(unix)]
pub fn terminal_answers(wait: Duration) -> bool {
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    let Ok(mut tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return false;
    };
    if tty.write_all(b"\x1b[c").and_then(|_| tty.flush()).is_err() {
        return false;
    }
    let deadline = std::time::Instant::now() + wait;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 256];
    while buf.len() < 4096 {
        if has_da1(&buf) {
            return true;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return false;
        }
        let mut pfd = libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for an open file descriptor we own.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as i32) };
        if ready <= 0 {
            return has_da1(&buf);
        }
        match tty.read(&mut chunk) {
            Ok(0) | Err(_) => return has_da1(&buf),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    has_da1(&buf)
}

#[cfg(not(unix))]
pub fn terminal_answers(_wait: Duration) -> bool {
    true
}

/// A complete DA1 answer (`ESC [ ? … c`) is in `buf`.
pub fn has_da1(buf: &[u8]) -> bool {
    buf.windows(3)
        .position(|w| w == b"\x1b[?")
        .is_some_and(|i| buf[i + 3..].contains(&b'c'))
}

/// Throw away terminal input that arrived but wasn't read (late answers to startup
/// queries), so it can't be taken for key presses.
pub fn discard_pending_input() {
    #[cfg(unix)]
    // SAFETY: tcflush on stdin; harmless if stdin isn't a terminal.
    unsafe {
        libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
    }
}

/// Why the UI can't start: stdin or stdout isn't a terminal.
pub fn no_tty_reason(stdin_tty: bool, stdout_tty: bool) -> Option<&'static str> {
    match (stdin_tty, stdout_tty) {
        (true, true) => None,
        (false, true) => Some("stdin is not a terminal"),
        (true, false) => Some("stdout is not a terminal"),
        (false, false) => Some("stdin and stdout are not terminals"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn answers_in_time_are_kept() {
        assert_eq!(run(Duration::from_secs(2), || 7), Some(7));
    }

    #[test]
    fn a_query_that_never_answers_times_out() {
        let t = Instant::now();
        let got: Option<u8> = run(Duration::from_millis(100), || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        });
        assert_eq!(got, None);
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_panicking_query_falls_back() {
        let got: Option<u8> = run(Duration::from_secs(1), || panic!("no terminal"));
        assert_eq!(got, None);
    }

    #[test]
    fn wait_is_clamped() {
        assert_eq!(startup_wait(0), Duration::from_millis(1000));
        assert_eq!(startup_wait(1500), Duration::from_millis(1500));
        assert_eq!(startup_wait(60_000), Duration::from_millis(5000));
    }

    #[test]
    fn budget_is_shared_and_skips_when_spent() {
        let b = Budget::new(Duration::from_millis(150));
        let t = Instant::now();
        let a: Option<u8> = b.run(|| {
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        });
        let c: Option<u8> = b.run(|| {
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        });
        assert_eq!((a, c), (None, None));
        assert!(
            t.elapsed() < Duration::from_millis(600),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(b.run(|| 1), None);
        assert_eq!(Budget::new(Duration::from_secs(1)).run(|| 1), Some(1));
    }

    #[test]
    fn da1_and_tty_reasons() {
        assert!(has_da1(b"\x1b[?62;22c"));
        assert!(!has_da1(b"\x1b[?62;22"));
        assert_eq!(no_tty_reason(true, true), None);
        assert_eq!(no_tty_reason(false, true), Some("stdin is not a terminal"));
    }
}
