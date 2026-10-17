//! The bounded, in-memory list of recent dispatches behind Activity.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::trace::{DispatchId, DispatchRecord};

/// The last `capacity` dispatches, newest first. Memory only: it is never
/// written to disk, and it is gone when the router exits.
#[derive(Debug)]
pub struct RecentDispatches {
    capacity: usize,
    records: Mutex<VecDeque<DispatchRecord>>,
}

impl RecentDispatches {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: Mutex::new(VecDeque::with_capacity(capacity.min(1024))),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn push(&self, record: DispatchRecord) {
        if self.capacity == 0 {
            return;
        }
        let mut records = self.lock();
        records.push_front(record);
        records.truncate(self.capacity);
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

    fn lock(&self) -> MutexGuard<'_, VecDeque<DispatchRecord>> {
        // A panic mid-push leaves the deque intact; keep serving it.
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
