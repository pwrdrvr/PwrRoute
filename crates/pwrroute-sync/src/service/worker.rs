//! Admission survives Service restart. A retired syscall retains its permit;
//! neither a timeout nor dropping a JoinHandle creates capacity.
use super::{Command, Completion, Outcome};
use crate::{
    Target,
    remote::{Cancel, Remote, Write},
    syncer::Seen,
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    thread,
};

pub(super) const LIMIT: usize = 4;

pub(super) struct Admission {
    roots: HashSet<PathBuf>,
    limit: usize,
}

impl Admission {
    pub fn new(limit: usize) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            roots: HashSet::new(),
            limit,
        }))
    }
}

pub(super) fn shared() -> Arc<Mutex<Admission>> {
    static ADMISSION: OnceLock<Arc<Mutex<Admission>>> = OnceLock::new();
    ADMISSION.get_or_init(|| Admission::new(LIMIT)).clone()
}

pub(super) struct Permit {
    admission: Arc<Mutex<Admission>>,
    root: PathBuf,
}

impl Permit {
    #[cfg(test)]
    pub fn active(admission: &Arc<Mutex<Admission>>) -> usize {
        admission.lock().unwrap().roots.len()
    }
    pub fn acquire(admission: &Arc<Mutex<Admission>>, root: &std::path::Path) -> Option<Self> {
        let mut slots = admission.lock().unwrap_or_else(|e| e.into_inner());
        if slots.roots.len() >= slots.limit || !slots.roots.insert(root.to_path_buf()) {
            return None;
        }
        Some(Self {
            admission: admission.clone(),
            root: root.to_path_buf(),
        })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.admission
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .roots
            .remove(&self.root);
    }
}

pub(super) enum Operation {
    Read { name: String, cached: Option<Seen> },
    Write(Write),
}

pub(super) fn spawn(
    io: Arc<dyn Remote>,
    target: Target,
    cancel: Cancel,
    permit: Permit,
    operation: Operation,
    tx: std::sync::mpsc::Sender<Command>,
) -> std::io::Result<()> {
    // The handle is intentionally detached. At most LIMIT workers/ready results
    // exist process-wide, and at most one can access any written root. A worker
    // owns only remote paths, immutable bytes and its result channel. It cannot
    // apply a pull, mutate bases/backups, publish status or retain the app.
    thread::Builder::new()
        .name("pwrroute-sync-io".into())
        .spawn(move || {
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match operation {
                    Operation::Read { name, cached } => {
                        Outcome::Read(io.read(&target, &name, cached, &cancel))
                    }
                    Operation::Write(write) => Outcome::Written(io.write(&target, &write, &cancel)),
                }))
                .unwrap_or(Outcome::Failed);
            let _ = tx.send(Command::Completed(Box::new(Completion {
                target,
                cancel,
                permit,
                outcome,
            })));
        })
        .map(|_| ())
}
