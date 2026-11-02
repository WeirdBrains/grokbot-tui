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
}
