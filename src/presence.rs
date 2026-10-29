//! Presence muting (1.21.0): while grokbot-tui is focused and in use, claim this install's
//! presence (`ReportGrokBotDesktopActivity active: true`) so the server holds pushes to your
//! other devices, the way the Grok Bot desktop app does. Heartbeats at half the lease;
//! released (`active: false`) on blur, idle, logout, account switch and quit. If the
//! process is suspended or the machine sleeps, heartbeats stop and the lease lapses.
use crate::app::{App, Event};
use std::time::{Duration, Instant};

/// Never heartbeat faster than this, whatever lease the server returns.
const MIN_BEAT: Duration = Duration::from_secs(5);
/// Fallback lease when the server returns none.
const DEFAULT_LEASE: Duration = Duration::from_secs(60);
/// Wait after a failed report before trying again.
const RETRY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct Presence {
    /// Terminal focus (FocusGained/FocusLost; assumed focused until told otherwise).
    pub focused: bool,
    pub last_input: Instant,
    /// The server holds our claim until about this instant.
    pub claimed_until: Option<Instant>,
    pub lease: Option<Duration>,
    pub next_beat: Instant,
    pub inflight: bool,
    pub last_error: Option<String>,
}

impl Default for Presence {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            focused: true,
            last_input: now,
            claimed_until: None,
            lease: None,
            next_beat: now,
            inflight: false,
            last_error: None,
        }
    }
}

impl Presence {
    /// Holding a live claim (🔕 in the header).
    pub fn claimed(&self, now: Instant) -> bool {
        self.claimed_until.is_some_and(|t| now < t)
    }
}

/// Should we hold a claim right now?
pub fn wanted(
    enabled: bool,
    connected: bool,
    focused: bool,
    idle: Duration,
    limit: Duration,
) -> bool {
    enabled && connected && focused && idle < limit
}

/// Next heartbeat after a lease: half of it, at least [`MIN_BEAT`].
pub fn beat_after(lease: Duration) -> Duration {
    (lease / 2).max(MIN_BEAT)
}

impl App {
    fn install_id(&mut self) -> Option<String> {
        if let Ok(Some(id)) = self.store.get_pref(crate::config::PREF_INSTALL_ID)
            && !id.trim().is_empty()
        {
            return Some(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.store
            .set_pref(crate::config::PREF_INSTALL_ID, &id)
            .ok()
            .map(|_| id)
    }

    /// Key, paste or click: the user is here.
    pub fn presence_input(&mut self) {
        self.presence.last_input = Instant::now();
    }

    pub fn presence_focus(&mut self, focused: bool) {
        self.presence.focused = focused;
        if focused {
            self.presence.last_input = Instant::now();
        }
    }

    /// Called from `tick`: claim, heartbeat or release as needed.
    pub fn presence_tick(&mut self) {
        let now = Instant::now();
        let Some(api) = self.api.clone() else {
            return;
        };
        if self.presence.inflight {
            return;
        }
        let want = wanted(
            self.config.claim_presence,
            api.can_attach(),
            self.presence.focused,
            now.duration_since(self.presence.last_input),
            Duration::from_secs(self.config.idle_minutes * 60),
        );
        let claimed = self.presence.claimed(now);
        let active = if want && (!claimed || now >= self.presence.next_beat) {
            if self.presence.last_error.is_some() && now < self.presence.next_beat {
                return;
            }
            true
        } else if !want && self.presence.claimed_until.is_some() {
            false
        } else {
            return;
        };
        let Some(id) = self.install_id() else {
            return;
        };
        self.presence.inflight = true;
        let epoch = self.epoch;
        self.spawn(async move {
            let result = api
                .report_activity(&id, active)
                .await
                .map_err(|e| e.to_string());
            Event::Presence {
                epoch,
                active,
                result,
            }
        });
    }

    pub fn on_presence(&mut self, epoch: u64, active: bool, result: Result<Duration, String>) {
        self.presence.inflight = false;
        let now = Instant::now();
        if epoch != self.epoch {
            return;
        }
        match result {
            Ok(lease) if active => {
                let lease = if lease.is_zero() {
                    DEFAULT_LEASE
                } else {
                    lease
                };
                self.presence.lease = Some(lease);
                self.presence.claimed_until = Some(now + lease);
                self.presence.next_beat = now + beat_after(lease);
                self.presence.last_error = None;
            }
            Ok(_) => {
                self.presence.claimed_until = None;
                self.presence.last_error = None;
            }
            Err(e) => {
                if active {
                    self.presence.next_beat = now + RETRY;
                } else {
                    // The lease lapses on its own.
                    self.presence.claimed_until = None;
                }
                self.presence.last_error = Some(e);
            }
        }
    }

    /// Release before the session goes away (logout, account switch). Fire and forget.
    pub fn presence_release(&mut self) {
        if self.presence.claimed_until.take().is_none() {
            return;
        }
        let (Some(api), Some(id)) = (self.api.clone(), self.install_id()) else {
            return;
        };
        self.presence.inflight = false;
        self.rt.spawn(async move {
            let _ = api.report_activity(&id, false).await;
        });
    }

    /// Release on quit, waiting up to 2 s so it reaches the server.
    pub fn presence_release_blocking(&mut self) {
        if self.presence.claimed_until.take().is_none() {
            return;
        }
        let (Some(api), Some(id)) = (self.api.clone(), self.install_id()) else {
            return;
        };
        let _ = self.rt.block_on(async move {
            tokio::time::timeout(Duration::from_secs(2), api.report_activity(&id, false)).await
        });
    }

    /// `/whoami` line.
    pub fn presence_summary(&self) -> String {
        let now = Instant::now();
        if !self.config.claim_presence {
            return "presence claim: off (notifications.claim_presence; /mute)".into();
        }
        match (self.presence.claimed(now), &self.presence.last_error) {
            (true, _) => format!(
                "presence claim: 🔕 holding (lease {}s, {}s left; other devices' pushes held)",
                self.presence.lease.unwrap_or_default().as_secs(),
                self.presence
                    .claimed_until
                    .map(|t| t.saturating_duration_since(now).as_secs())
                    .unwrap_or(0)
            ),
            (false, Some(e)) => format!("presence claim: on, last report failed: {e}"),
            (false, None) if !self.presence.focused => {
                "presence claim: on, released (terminal not focused)".into()
            }
            (false, None) => format!(
                "presence claim: on, released (idle ≥ {} min or not connected)",
                self.config.idle_minutes
            ),
        }
    }

    /// Toggle `notifications.claim_presence` (menu, `/mute`).
    pub fn set_claim_presence(&mut self, on: bool) {
        if let Err(e) = self.store.set_pref(
            crate::config::PREF_CLAIM_PRESENCE,
            if on { "true" } else { "false" },
        ) {
            self.error(format!("{e:#}"));
            return;
        }
        self.config.claim_presence = on;
        self.presence.last_input = Instant::now();
        self.info(if on {
            format!(
                "Presence claim ON: while grokbot-tui is focused and used (idle limit {} min), \
                 the server holds Grok Bot pushes to your other devices. 🔕 shows when it's held.",
                self.config.idle_minutes
            )
        } else {
            "Presence claim OFF: your other devices get pushes as usual.".into()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_needs_pref_connection_focus_and_activity() {
        let m = Duration::from_secs(600);
        assert!(wanted(true, true, true, Duration::ZERO, m));
        assert!(!wanted(false, true, true, Duration::ZERO, m));
        assert!(!wanted(true, false, true, Duration::ZERO, m));
        assert!(!wanted(true, true, false, Duration::ZERO, m));
        assert!(!wanted(true, true, true, m, m));
        assert_eq!(beat_after(Duration::from_secs(60)), Duration::from_secs(30));
        assert_eq!(beat_after(Duration::from_secs(2)), MIN_BEAT);
    }

    #[test]
    fn claims_heartbeats_and_releases_on_blur() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let (mut app, _d) = crate::app::tests::test_app(&rt);
        crate::app::tests::mock_login(&mut app, "tok");
        app.presence_tick();
        assert!(!app.presence.inflight, "off by default");
        app.config.claim_presence = true;
        app.presence_tick();
        assert!(app.presence.inflight);
        crate::app::tests::settle(&mut app);
        assert!(app.presence.claimed(Instant::now()));
        assert_eq!(app.presence.lease, Some(Duration::from_secs(60)));
        let id = app
            .store
            .get_pref(crate::config::PREF_INSTALL_ID)
            .unwrap()
            .unwrap();
        assert_eq!(id.len(), 36);
        // Not yet due: nothing sent.
        app.presence_tick();
        assert!(!app.presence.inflight);
        // Blur releases.
        app.presence_focus(false);
        app.presence_tick();
        assert!(app.presence.inflight);
        crate::app::tests::settle(&mut app);
        assert!(!app.presence.claimed(Instant::now()));
        assert!(app.presence_summary().contains("not focused"));
        // Same install id next time.
        app.presence_focus(true);
        app.presence_tick();
        crate::app::tests::settle(&mut app);
        assert_eq!(
            app.store
                .get_pref(crate::config::PREF_INSTALL_ID)
                .unwrap()
                .unwrap(),
            id
        );
    }
}
