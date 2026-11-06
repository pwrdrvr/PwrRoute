//! Wall-clock release deadlines, with awake time for connectivity settling.

use super::{CHECK_EVERY, LastCheck, Outcome, State};
use std::time::{Duration, Instant, SystemTime};

/// Samples the platform clock separately from the wall-clock deadline.
pub(super) struct AwakeClock {
    started: Instant,
    native_start: Option<Duration>,
}

impl AwakeClock {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            native_start: pwrroute_os::system_awake_time(),
        }
    }

    pub(super) fn elapsed(&self) -> Duration {
        let native = self
            .native_start
            .zip(pwrroute_os::system_awake_time())
            .map(|(start, now)| now.saturating_sub(start));
        elapsed_from(self.started.elapsed(), native)
    }
}

fn elapsed_from(instant: Duration, suspend_excluding: Option<Duration>) -> Duration {
    suspend_excluding.unwrap_or(instant)
}

pub(super) const POLL_EVERY: Duration = Duration::from_secs(10);
const SETTLE_FOR: Duration = Duration::from_secs(30);
const WAKE_GAP: Duration = Duration::from_secs(5);

pub(super) struct Schedule {
    next: SystemTime,
    regular: SystemTime,
    last: Option<LastCheck>,
    retry_delay: Duration,
    connected: Option<bool>,
    recovery: Option<Duration>,
    wall: SystemTime,
    awake: Duration,
}

impl Schedule {
    pub(super) fn new(wall: SystemTime, awake: Duration) -> Self {
        Self {
            next: wall + CHECK_EVERY,
            regular: wall + CHECK_EVERY,
            last: None,
            retry_delay: SETTLE_FOR,
            connected: None,
            recovery: None,
            wall,
            awake,
        }
    }

    /// One check for an overdue deadline, even after many missed periods.
    /// Failed connections retry at 30 s, 1 m, 2 m, ... up to six hours.
    /// An observed reconnection can bring that retry forward, after 30 s
    /// awake with no observed disconnect. HTTP/package errors don't retry.
    pub(super) fn poll(
        &mut self,
        wall: SystemTime,
        awake: Duration,
        state: &State,
        connected: Option<bool>,
    ) -> bool {
        let network_failed = state
            .last
            .as_ref()
            .is_some_and(|last| matches!(last.outcome, Outcome::NetworkFailed(_)));
        if state.last != self.last {
            self.last = state.last.clone();
            self.recovery = None;
            if let Some(last) = &state.last {
                self.regular = last.at + CHECK_EVERY;
                if network_failed {
                    self.next = last.at + self.retry_delay;
                    self.retry_delay = (self.retry_delay * 2).min(CHECK_EVERY);
                } else {
                    self.next = self.regular;
                    self.retry_delay = SETTLE_FOR;
                }
            } else {
                // A selection change clears the previous slot's result.
                self.retry_delay = SETTLE_FOR;
                self.next = wall + CHECK_EVERY;
                self.regular = self.next;
            }
        }
        let elapsed = wall.duration_since(self.wall);
        let woke = elapsed.as_ref().is_ok_and(|elapsed| {
            elapsed.saturating_sub(awake.saturating_sub(self.awake)) > WAKE_GAP
        });
        if elapsed.is_err() {
            // A clock correction must not defer checks by the size of the
            // correction or let the old future timestamp block retries.
            self.next = wall
                + if network_failed {
                    SETTLE_FOR
                } else {
                    CHECK_EVERY
                };
            self.regular = wall + CHECK_EVERY;
        }
        self.wall = wall;
        self.awake = awake;
        if (woke && (wall >= self.next || network_failed))
            || (network_failed && self.connected == Some(false) && connected == Some(true))
        {
            self.recovery = Some(awake + SETTLE_FOR);
        }
        self.connected = connected;
        if connected == Some(false) {
            self.recovery = None;
        }
        if state.checking || state.restarting {
            return false;
        }
        if let Some(ready) = self.recovery {
            if awake < ready {
                return false;
            }
            self.recovery = None;
            if network_failed {
                self.next = wall;
            }
        }
        if wall < self.next {
            return false;
        }
        // Known offline: suppress quick retries, but never suppress the
        // ordinary six-hour request solely on a reachability hint.
        if network_failed && connected == Some(false) && wall < self.regular {
            return false;
        }
        self.next = wall + CHECK_EVERY;
        self.regular = self.next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update_channel::Slot;
    use proptest::prelude::*;

    fn wall(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }
    fn result(at: u64, outcome: Outcome) -> State {
        State {
            last: Some(LastCheck {
                at: wall(at),
                slot: Slot::STABLE_LATEST,
                outcome,
            }),
            ..State::default()
        }
    }
    fn offline(at: u64) -> State {
        result(at, Outcome::NetworkFailed("offline".into()))
    }

    proptest! {
        #[test]
        fn review_windows_clock_excludes_sleep_from_wake_settling(sleep in 21601u64..604800) {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            let state = State::default();
            // Windows' Instant advances during suspend; its unbiased
            // interrupt count advances only for the ten seconds awake.
            let resumed = elapsed_from(Duration::from_secs(sleep + 10), Some(Duration::from_secs(10)));
            prop_assert!(!schedule.poll(wall(sleep + 10), resumed, &state, None));
            let settled = elapsed_from(Duration::from_secs(sleep + 40), Some(Duration::from_secs(40)));
            prop_assert!(schedule.poll(wall(sleep + 40), settled, &state, None));
        }

        #[test]
        fn a_short_windows_sleep_brings_a_backed_off_retry_forward(sleep in 6u64..80) {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            // Three failed attempts: the fourth has a two-minute backoff.
            for (at, delay) in [(0, 30), (31, 60)] {
                let state = offline(at);
                prop_assert!(!schedule.poll(wall(at), Duration::from_secs(at), &state, None));
                prop_assert!(schedule.poll(wall(at + delay), Duration::from_secs(at + delay), &state, None));
            }
            let state = offline(92);
            prop_assert!(!schedule.poll(wall(92), Duration::from_secs(92), &state, None));
            let resumed_wall = 93 + sleep;
            let resumed = elapsed_from(Duration::from_secs(resumed_wall), Some(Duration::from_secs(93)));
            prop_assert!(!schedule.poll(wall(resumed_wall), resumed, &state, None));
            let settled = elapsed_from(Duration::from_secs(resumed_wall + 30), Some(Duration::from_secs(123)));
            // This is still earlier than the ordinary backoff deadline (212).
            prop_assert!(schedule.poll(wall(resumed_wall + 30), settled, &state, None));
        }

        #[test]
        fn sleep_counts_toward_the_release_check_deadline(sleep in 21601u64..604800) {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            let state = State::default();
            prop_assert!(!schedule.poll(wall(sleep), Duration::from_secs(10), &state, Some(true)));
            prop_assert!(schedule.poll(wall(sleep + 30), Duration::from_secs(40), &state, Some(true)));
            prop_assert!(!schedule.poll(wall(sleep + 40), Duration::from_secs(50), &state, Some(true)));
        }

        // Reference model: failure n allows an attempt after
        // min(30 * 2^n, six hours); success restores the ordinary deadline.
        // Generated completion times and failed runs exercise the state
        // machine without sockets, sleeping, or a six-hour test timer.
        #[test]
        fn connection_retries_follow_a_bounded_backoff_model(delays in prop::collection::vec(1u64..300, 1..25)) {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            let mut at = 0;
            for (n, duration) in delays.iter().enumerate() {
                let state = offline(at);
                let delay = (30 * (1u64 << n.min(20))).min(CHECK_EVERY.as_secs());
                prop_assert!(!schedule.poll(wall(at), Duration::from_secs(at), &state, None));
                prop_assert!(!schedule.poll(wall(at + delay - 1), Duration::from_secs(at + delay - 1), &state, None));
                prop_assert!(schedule.poll(wall(at + delay), Duration::from_secs(at + delay), &state, None));
                at += delay + duration;
            }
            let state = result(at, Outcome::UpToDate("1.0.0".into()));
            prop_assert!(!schedule.poll(wall(at), Duration::from_secs(at), &state, None));
            prop_assert!(!schedule.poll(wall(at + CHECK_EVERY.as_secs() - 1), Duration::from_secs(at + CHECK_EVERY.as_secs() - 1), &state, None));
            prop_assert!(schedule.poll(wall(at + CHECK_EVERY.as_secs()), Duration::from_secs(at + CHECK_EVERY.as_secs()), &state, None));
        }

        #[test]
        fn a_reconnection_waits_for_thirty_seconds_without_an_observed_disconnect(down in 31u64..1000, flap in 1u64..30) {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            let state = offline(0);
            prop_assert!(!schedule.poll(wall(0), Duration::ZERO, &state, Some(false)));
            prop_assert!(!schedule.poll(wall(down), Duration::from_secs(down), &state, Some(true)));
            prop_assert!(!schedule.poll(wall(down + flap), Duration::from_secs(down + flap), &state, Some(false)));
            let up = down + flap + 1;
            prop_assert!(!schedule.poll(wall(up), Duration::from_secs(up), &state, Some(true)));
            prop_assert!(!schedule.poll(wall(up + 29), Duration::from_secs(up + 29), &state, Some(true)));
            prop_assert!(schedule.poll(wall(up + 30), Duration::from_secs(up + 30), &state, Some(true)));
        }
    }

    #[test]
    fn windows_wake_waits_thirty_awake_seconds_even_after_six_hours_suspended() {
        let mut schedule = Schedule::new(wall(0), Duration::ZERO);
        let state = State::default();
        let resumed = elapsed_from(Duration::from_secs(21611), Some(Duration::from_secs(10)));
        assert!(!schedule.poll(wall(21611), resumed, &state, None));
        let settled = elapsed_from(Duration::from_secs(21641), Some(Duration::from_secs(40)));
        assert!(schedule.poll(wall(21641), settled, &state, None));
    }

    #[test]
    fn an_overnight_sleep_does_not_require_another_six_hours_awake() {
        let mut schedule = Schedule::new(wall(0), Duration::ZERO);
        let state = result(0, Outcome::UpToDate("0.17.0".into()));
        assert!(!schedule.poll(wall(0), Duration::ZERO, &state, None));
        assert!(!schedule.poll(wall(36000), Duration::from_secs(10), &state, None));
        assert!(schedule.poll(wall(36030), Duration::from_secs(40), &state, None));
    }

    #[test]
    fn non_network_errors_do_not_get_quick_retries_or_reconnection_checks() {
        for outcome in [
            Outcome::Failed("rate limited".into()),
            Outcome::NotInstalled("dev".into()),
        ] {
            let mut schedule = Schedule::new(wall(0), Duration::ZERO);
            let state = result(0, outcome);
            for (at, online) in [(0, false), (10, true), (40, true), (60, true)] {
                assert!(!schedule.poll(wall(at), Duration::from_secs(at), &state, Some(online)));
            }
        }
    }

    #[test]
    fn busy_or_restarting_workers_are_not_poked_and_clock_rollback_is_bounded() {
        let mut schedule = Schedule::new(wall(100000), Duration::ZERO);
        let mut state = offline(100000);
        state.checking = true;
        assert!(!schedule.poll(wall(100030), Duration::from_secs(30), &state, None));
        state.checking = false;
        state.restarting = true;
        assert!(!schedule.poll(wall(100040), Duration::from_secs(40), &state, None));
        state.restarting = false;
        assert!(!schedule.poll(wall(0), Duration::from_secs(50), &state, None));
        assert!(schedule.poll(wall(30), Duration::from_secs(80), &state, None));
    }
}
