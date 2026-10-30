//! When a sync pass runs. Pure: it is given the time and the local files'
//! stamps on every tick, so tests drive it with a fake clock.
//!
//! A pass runs on the first tick, every [`Schedule::every`], when woken
//! (the window came to the front, Sync now, a save that came through the
//! app or an agent), after the machine slept (a tick that comes much later
//! than it should), and when a local file changed and has stayed the same
//! for one tick (an editor's burst of writes is one pass).

use crate::folder::Stamp;

/// Why a pass runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Start,
    Interval,
    Woken,
    /// A tick came far later than the last one: the machine slept.
    Resumed,
    LocalChange,
}

#[derive(Debug, Clone)]
pub struct Schedule {
    /// Milliseconds between passes when nothing else asks for one.
    pub every: u64,
    /// Milliseconds between ticks.
    pub tick: u64,
    last_pass: Option<u64>,
    last_tick: Option<u64>,
    woken: bool,
    /// The local stamps the last pass saw, and those of the last tick.
    at_pass: Vec<Option<Stamp>>,
    at_tick: Vec<Option<Stamp>>,
}

/// A tick this many ticks late means the machine slept.
const LATE_TICKS: u64 = 5;

impl Schedule {
    pub fn new(every: u64, tick: u64) -> Self {
        Self {
            every,
            tick,
            last_pass: None,
            last_tick: None,
            woken: false,
            at_pass: Vec::new(),
            at_tick: Vec::new(),
        }
    }

    /// Asks for a pass on the next tick.
    pub fn wake(&mut self) {
        self.woken = true;
    }

    /// Whether a pass runs now, and why. `local` is each synced local
    /// file's stamp (`None` when it is missing), in a fixed order.
    pub fn tick(&mut self, now: u64, local: Vec<Option<Stamp>>) -> Option<Why> {
        let late = self
            .last_tick
            .is_some_and(|t| now.saturating_sub(t) > self.tick * LATE_TICKS);
        let changed = local != self.at_pass && local == self.at_tick;
        let why = if self.last_pass.is_none() {
            Some(Why::Start)
        } else if self.woken {
            Some(Why::Woken)
        } else if late {
            Some(Why::Resumed)
        } else if changed {
            Some(Why::LocalChange)
        } else if self
            .last_pass
            .is_some_and(|t| now.saturating_sub(t) >= self.every)
        {
            Some(Why::Interval)
        } else {
            None
        };
        self.last_tick = Some(now);
        if why.is_some() {
            self.last_pass = Some(now);
            self.woken = false;
            self.at_pass.clone_from(&local);
        }
        self.at_tick = local;
        why
    }

    /// Records the local stamps a pass left behind (after it pulled), so
    /// its own writes are not taken for a local change.
    pub fn passed_with(&mut self, local: Vec<Option<Stamp>>) {
        self.at_pass.clone_from(&local);
        self.at_tick = local;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(len: u64) -> Option<Stamp> {
        Some(Stamp { len, modified: 1 })
    }

    #[test]
    fn runs_at_start_then_on_the_interval() {
        let mut s = Schedule::new(60_000, 2_000);
        assert_eq!(s.tick(0, vec![stamp(1)]), Some(Why::Start));
        let mut now = 0;
        let mut passes = Vec::new();
        while now < 130_000 {
            now += 2_000;
            if let Some(why) = s.tick(now, vec![stamp(1)]) {
                passes.push((now, why));
            }
        }
        assert_eq!(
            passes,
            vec![(60_000, Why::Interval), (120_000, Why::Interval)]
        );
    }

    #[test]
    fn a_local_change_runs_once_it_settles() {
        let mut s = Schedule::new(60_000, 2_000);
        s.tick(0, vec![stamp(1)]);
        assert_eq!(s.tick(2_000, vec![stamp(2)]), None, "still being written");
        assert_eq!(s.tick(4_000, vec![stamp(2)]), Some(Why::LocalChange));
        assert_eq!(s.tick(6_000, vec![stamp(2)]), None, "once");
        // A file that appears counts too.
        let mut s = Schedule::new(60_000, 2_000);
        s.tick(0, vec![None]);
        s.tick(2_000, vec![stamp(5)]);
        assert_eq!(s.tick(4_000, vec![stamp(5)]), Some(Why::LocalChange));
    }

    #[test]
    fn a_pass_that_pulled_is_not_a_local_change() {
        let mut s = Schedule::new(60_000, 2_000);
        s.tick(0, vec![stamp(1)]);
        s.passed_with(vec![stamp(9)]);
        assert_eq!(s.tick(2_000, vec![stamp(9)]), None);
        assert_eq!(s.tick(4_000, vec![stamp(9)]), None);
    }

    #[test]
    fn waking_and_sleeping_run_a_pass() {
        let mut s = Schedule::new(60_000, 2_000);
        s.tick(0, vec![]);
        s.wake();
        assert_eq!(s.tick(2_000, vec![]), Some(Why::Woken));
        assert_eq!(s.tick(4_000, vec![]), None);
        // The lid closed for an hour.
        assert_eq!(s.tick(3_604_000, vec![]), Some(Why::Resumed));
        assert_eq!(s.tick(3_606_000, vec![]), None);
    }
}
