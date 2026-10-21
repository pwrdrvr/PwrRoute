//! The audit trail: which agent called which tool, when, for how long,
//! and whether it was allowed. Never the arguments or the result: those
//! carry URLs, and URLs that pass through MCP are not written anywhere.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ok,
    /// The agent's role does not allow the tool, or it is not approved.
    Denied,
    /// The tool ran and failed, or its input was rejected.
    Failed,
}

/// One tool call. There is deliberately no field for arguments or results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub tool: String,
    /// The approved client name.
    pub agent: String,
    /// Which of its connections, counted from 1 since PwrRoute started.
    pub connection: u64,
    #[serde(with = "unix_ms")]
    pub started: SystemTime,
    #[serde(with = "millis")]
    pub duration: Duration,
    pub outcome: Outcome,
    /// A fixed word about the outcome, when there is more to say than the
    /// outcome itself: `pwrroute_rules_apply` refused for failing saved
    /// tests, or wrote past them with `force`. Never agent text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

/// Where audit records go. The app may write them to disk: they hold no
/// URLs.
pub trait AuditSink: Send + Sync + 'static {
    fn record(&self, record: AuditRecord);
}

/// Drops every record.
#[derive(Debug, Default)]
pub struct NoAudit;

impl AuditSink for NoAudit {
    fn record(&self, _record: AuditRecord) {}
}

/// Keeps the last `capacity` records in memory, as PwrSnap keeps its last
/// 500.
#[derive(Debug)]
pub struct MemoryAudit {
    capacity: usize,
    records: Mutex<VecDeque<AuditRecord>>,
}

impl MemoryAudit {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            records: Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Oldest first.
    #[must_use]
    pub fn records(&self) -> Vec<AuditRecord> {
        self.lock().iter().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<AuditRecord>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for MemoryAudit {
    fn default() -> Self {
        Self::new(500)
    }
}

impl AuditSink for MemoryAudit {
    fn record(&self, record: AuditRecord) {
        let mut records = self.lock();
        if records.len() == self.capacity {
            records.pop_front();
        }
        records.push_back(record);
    }
}

mod unix_ms {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde::Serializer;

    pub(super) fn serialize<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
        let ms = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        s.serialize_u128(ms)
    }
}

mod millis {
    use std::time::Duration;

    use serde::Serializer;

    pub(super) fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u128(d.as_millis())
    }
}
