//! The bounded, in-memory list of recent dispatches behind Activity.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use crate::trace::{DispatchId, DispatchRecord};

/// How long dispatches stay in memory: the newest `count`, each for at most
/// `hours`. Zero `count` keeps nothing; zero `hours` means no age limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Retention {
    pub count: usize,
    pub hours: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            count: 500,
            hours: 24,
        }
    }
}

impl Retention {
    /// The oldest `received_at` still kept at `now`, when age is limited.
    /// A span too long to represent is no limit at all.
    fn cutoff(self, now: SystemTime) -> Option<SystemTime> {
        if self.hours == 0 {
            return None;
        }
        let span = Duration::from_secs(self.hours.checked_mul(60 * 60)?);
        now.checked_sub(span)
    }
}

/// The last `count` dispatches of the last `hours`, newest first. Memory
/// only: it is never written to disk, and it is gone when the router exits.
/// Age is enforced on every push and every read, so nothing older than the
/// retention is ever handed out.
#[derive(Debug)]
pub struct RecentDispatches {
    retention: Retention,
    records: Mutex<VecDeque<DispatchRecord>>,
}

impl RecentDispatches {
    pub fn new(retention: Retention) -> Self {
        Self {
            retention,
            records: Mutex::new(VecDeque::with_capacity(retention.count.min(1024))),
        }
    }

    pub fn retention(&self) -> Retention {
        self.retention
    }

    /// The most dispatches ever kept: `retention.count`.
    pub fn capacity(&self) -> usize {
        self.retention.count
    }

    pub(crate) fn push(&self, record: DispatchRecord) {
        if self.retention.count == 0 {
            return;
        }
        let mut records = self.lock();
        records.push_front(record);
        records.truncate(self.retention.count);
        sweep(&mut records, self.retention, SystemTime::now());
    }

    /// Every record, newest first.
    pub fn snapshot(&self) -> Vec<DispatchRecord> {
        self.lock().iter().cloned().collect()
    }

    pub fn get(&self, id: DispatchId) -> Option<DispatchRecord> {
        self.lock().iter().find(|r| r.id == id).cloned()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Forget every record, for Activity's Clear.
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// The deque with everything past its age dropped.
    fn lock(&self) -> MutexGuard<'_, VecDeque<DispatchRecord>> {
        // A panic mid-push leaves the deque intact; keep serving it.
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        sweep(&mut records, self.retention, SystemTime::now());
        records
    }
}

/// Drops every record received before the retention's cutoff. Records are
/// newest first, so the expired ones are at the back; a record whose clock
/// ran ahead is treated as current.
fn sweep(records: &mut VecDeque<DispatchRecord>, retention: Retention, now: SystemTime) {
    let Some(cutoff) = retention.cutoff(now) else {
        return;
    };
    while records
        .back()
        .is_some_and(|record| record.received_at < cutoff)
    {
        records.pop_back();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        link::{Link, Source},
        trace::{Outcome, Privacy, Timings},
    };

    fn record(id: u64, received_at: SystemTime) -> DispatchRecord {
        DispatchRecord {
            id: DispatchId(id),
            received_at,
            privacy: Privacy::Full,
            link: Link::new("https://example.com/", Source::CommandLine),
            forwarded: false,
            final_url: None,
            browser: None,
            steps: Vec::new(),
            outcome: Outcome::DryRun,
            timings: Timings::default(),
        }
    }

    #[test]
    fn records_older_than_the_retention_are_evicted_on_push_and_on_read() {
        let recent = RecentDispatches::new(Retention {
            count: 10,
            hours: 24,
        });
        let now = SystemTime::now();
        let hour = Duration::from_secs(60 * 60);
        recent.push(record(1, now - 25 * hour));
        recent.push(record(2, now - 23 * hour));
        recent.push(record(3, now));
        // 1 was expired on its own push; the rest are within 24 hours.
        assert_eq!(
            recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert!(recent.get(DispatchId(1)).is_none());
        assert_eq!(recent.len(), 2);

        // A record that expires while it sits in the buffer is dropped on
        // the next read: age one through the deque, behind the sweep.
        recent
            .records
            .lock()
            .unwrap()
            .push_back(record(0, now - 30 * hour));
        assert_eq!(recent.len(), 2);
        assert!(recent.get(DispatchId(0)).is_none());
    }

    #[test]
    fn zero_hours_keeps_by_count_only() {
        let recent = RecentDispatches::new(Retention { count: 2, hours: 0 });
        let old = SystemTime::UNIX_EPOCH;
        recent.push(record(1, old));
        recent.push(record(2, old));
        recent.push(record(3, old));
        assert_eq!(
            recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
            vec![3, 2]
        );
        assert_eq!(recent.capacity(), 2);
    }
}
