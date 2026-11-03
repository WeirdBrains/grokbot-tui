//! Bounds for startup terminal work: the shared query deadline, a helper-thread runner
//! for a call that may never return (the kitty keyboard re-query after a suspend), the
//! late-reply flush, and the no-TTY check. The startup queries themselves go out in one
//! burst from [`crate::probe`].

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
    fn tty_reasons() {
        assert_eq!(no_tty_reason(true, true), None);
        assert_eq!(no_tty_reason(false, true), Some("stdin is not a terminal"));
    }
}
