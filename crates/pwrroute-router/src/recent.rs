//! The bounded, in-memory list of recent dispatches behind Activity.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, PoisonError},
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};

use crate::{
    link::Link,
    trace::{DispatchId, DispatchRecord},
};

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
    inner: Mutex<Inner>,
}

/// The retention and the records it bounds, under one lock, so a change
/// to the retention sweeps the records it no longer keeps in the same
/// step.
#[derive(Debug)]
struct Inner {
    retention: Retention,
    records: VecDeque<DispatchRecord>,
    /// The whole link of each record that was not opened and can still be,
    /// newest first: a host-only record keeps its full URL here, in memory,
    /// so Open works. Never more than `records` holds: an entry goes when
    /// its record does, when it is opened, or on Clear.
    waiting: VecDeque<(DispatchId, Link)>,
}

impl Inner {
    /// Drops what the retention no longer keeps, and the links of records
    /// that went.
    fn sweep(&mut self, now: SystemTime) {
        self.records.truncate(self.retention.count);
        sweep(&mut self.records, self.retention, now);
        let records = &self.records;
        self.waiting
            .retain(|(id, _)| records.iter().any(|record| record.id == *id));
    }
}

impl RecentDispatches {
    pub fn new(retention: Retention) -> Self {
        Self {
            inner: Mutex::new(Inner {
                retention,
                records: VecDeque::with_capacity(retention.count.min(1024)),
                waiting: VecDeque::new(),
            }),
        }
    }

    pub fn retention(&self) -> Retention {
        self.lock().retention
    }

    /// The most dispatches ever kept: `retention.count`.
    pub fn capacity(&self) -> usize {
        self.retention().count
    }

    /// Changes the retention and drops at once whatever the new one does
    /// not keep: the oldest beyond the count, and everything past the age.
    pub(crate) fn set_retention(&self, retention: Retention) {
        let mut inner = self.lock();
        inner.retention = retention;
        inner.sweep(SystemTime::now());
    }

    #[cfg(test)]
    pub(crate) fn push(&self, record: DispatchRecord) {
        self.push_waiting(record, None);
    }

    /// Adds a record, with the whole link when it was not opened and can
    /// still be. Nothing is kept, the link included, while the retention
    /// keeps nothing.
    pub(crate) fn push_waiting(&self, record: DispatchRecord, link: Option<Link>) {
        let mut inner = self.lock();
        if inner.retention.count == 0 {
            return;
        }
        if let Some(link) = link {
            inner.waiting.push_front((record.id, link));
        }
        inner.records.push_front(record);
        inner.sweep(SystemTime::now());
    }

    /// The records still waiting to be opened, newest first.
    pub fn pending(&self) -> Vec<DispatchId> {
        self.lock().waiting.iter().map(|(id, _)| *id).collect()
    }

    /// Whether `id` is still waiting to be opened.
    pub fn is_pending(&self, id: DispatchId) -> bool {
        self.lock()
            .waiting
            .iter()
            .any(|(waiting, _)| *waiting == id)
    }

    /// The whole link of a record waiting to be opened; it waits no more.
    pub(crate) fn take_pending(&self, id: DispatchId) -> Option<Link> {
        let mut inner = self.lock();
        let at = inner
            .waiting
            .iter()
            .position(|(waiting, _)| *waiting == id)?;
        inner.waiting.remove(at).map(|(_, link)| link)
    }

    /// Every record, newest first.
    pub fn snapshot(&self) -> Vec<DispatchRecord> {
        self.lock().records.iter().cloned().collect()
    }

    pub fn get(&self, id: DispatchId) -> Option<DispatchRecord> {
        self.lock().records.iter().find(|r| r.id == id).cloned()
    }

    pub fn len(&self) -> usize {
        self.lock().records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().records.is_empty()
    }

    /// Forget every record, and every link waiting to be opened, for
    /// Activity's Clear.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.records.clear();
        inner.waiting.clear();
    }

    /// The state with everything past its age dropped.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic mid-push leaves the deque intact; keep serving it.
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.sweep(SystemTime::now());
        inner
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
            .inner
            .lock()
            .unwrap()
            .records
            .push_back(record(0, now - 30 * hour));
        assert_eq!(recent.len(), 2);
        assert!(recent.get(DispatchId(0)).is_none());
    }

    #[test]
    fn a_new_retention_sweeps_at_once() {
        let recent = RecentDispatches::new(Retention {
            count: 10,
            hours: 0,
        });
        let now = SystemTime::now();
        let hour = Duration::from_secs(60 * 60);
        recent.push(record(1, now - 30 * hour));
        recent.push(record(2, now - 2 * hour));
        recent.push(record(3, now - hour / 2));
        recent.push(record(4, now));
        assert_eq!(recent.len(), 4);

        // Fewer by count: the oldest go.
        recent.set_retention(Retention { count: 3, hours: 0 });
        assert_eq!(
            recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
            vec![4, 3, 2]
        );
        // Shorter by age: the expired go, and later pushes honor it.
        recent.set_retention(Retention { count: 3, hours: 1 });
        assert_eq!(
            recent.snapshot().iter().map(|r| r.id.0).collect::<Vec<_>>(),
            vec![4, 3]
        );
        assert_eq!(recent.retention().hours, 1);
        recent.push(record(5, now));
        assert_eq!(recent.len(), 3);
        // Nothing kept at all.
        recent.set_retention(Retention { count: 0, hours: 0 });
        assert!(recent.is_empty());
        recent.push(record(6, now));
        assert!(recent.is_empty());
    }

    #[test]
    fn a_waiting_link_leaves_with_its_record() {
        let recent = RecentDispatches::new(Retention { count: 2, hours: 1 });
        let now = SystemTime::now();
        let link = |n: u32| Link::new(format!("https://example.com/{n}"), Source::CommandLine);
        recent.push_waiting(record(1, now), Some(link(1)));
        recent.push_waiting(record(2, now), None);
        recent.push_waiting(record(3, now), Some(link(3)));
        // 1 went by count, and its link with it.
        assert_eq!(recent.pending(), vec![DispatchId(3)]);
        assert_eq!(recent.take_pending(DispatchId(1)), None);
        // Taken once: the link is the whole one, then it waits no more.
        assert_eq!(
            recent.take_pending(DispatchId(3)).map(|l| l.url),
            Some("https://example.com/3".to_owned())
        );
        assert!(!recent.is_pending(DispatchId(3)));
        assert!(recent.get(DispatchId(3)).is_some(), "the record stays");

        // By age, and on Clear.
        recent.push_waiting(record(4, now), Some(link(4)));
        recent
            .inner
            .lock()
            .unwrap()
            .records
            .iter_mut()
            .for_each(|r| r.received_at = now - Duration::from_secs(2 * 60 * 60));
        assert!(recent.pending().is_empty());
        recent.push_waiting(record(5, now), Some(link(5)));
        recent.clear();
        assert!(recent.pending().is_empty());

        // Nothing kept means nothing waits either.
        recent.set_retention(Retention { count: 0, hours: 0 });
        recent.push_waiting(record(6, now), Some(link(6)));
        assert!(recent.pending().is_empty());
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
