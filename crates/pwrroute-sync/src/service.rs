//! The sync thread: a [`Syncer`] driven by a [`Schedule`] and a [`Clock`].
//!
//! The app owns one [`Service`]. Every call sends a command to the thread
//! and returns at once; the thread ticks every [`Schedule::tick`], runs a
//! pass when the schedule says so, and hands each new [`Status`] to the
//! callback it was given. Bounded, isolated workers own cloud I/O; the
//! coordinator alone applies local changes and owns sync state.
//!
//! Ready targets advance in configured order; busy targets are skipped. Each
//! target keeps item order (config before rules), and every local decision reads
//! the current bytes and base under the shared writer guard. A pull schedules
//! another pass so targets already visited receive the new content too.
//!
//! Four process-wide permits cover running I/O and unconsumed results. Each
//! written root has at most one permit, including after removal or Service
//! restart. A stuck filesystem syscall retains its permit until it returns:
//! deadlines never manufacture replacement capacity. If all four are stuck,
//! new remote work waits while commands and Stop remain responsive.
//!
//! Drop retires requests and joins only the coordinator. Detached workers own
//! remote paths and immutable bytes, never local files, state or app callbacks.
//! Late results are discarded by request identity. Cancellation is checked
//! before each controllable remote write stage; an OS write/rename already
//! entered can still finish after removal/Stop, and may leave a staging file or
//! content without its manifest. A subsequent reader must settle it normally.
//!
//! Local filesystem operations and item checks remain serialized here. A local
//! disk stall, including a local Finicky symlink into a cloud mount, or a
//! blocking status callback is outside remote-target isolation. The synchronous
//! Syncer pass/resolve APIs remain blocking helpers; the app uses Service.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

use crate::{
    folder::Manifest,
    remote::{Cancel, Observation, Write},
    schedule::Schedule,
    syncer::{Choice, Item, Status, Syncer, Target},
};

/// Milliseconds since the epoch, from the system or a test.
pub trait Clock: Send + 'static {
    fn now(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// How often the service looks: a pass every minute, a tick every two
/// seconds (local stamps only, which reads no cloud folder).
pub const EVERY: Duration = Duration::from_secs(60);
pub const TICK: Duration = Duration::from_secs(2);

enum Command {
    Wake,
    Targets(Vec<Target>),
    Items(Vec<Item>),
    MachineName(String),
    Resolve {
        root: PathBuf,
        name: String,
        remote_sha: String,
        choice: Choice,
        done: Box<dyn FnOnce(Result<()>) + Send>,
    },
    Stop,
    Completed(Box<Completion>),
}

pub struct Service {
    tx: Sender<Command>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
    stop: Cancel,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").finish_non_exhaustive()
    }
}

impl Service {
    /// Starts the coordinator. `on_status` runs on it when a pass starts,
    /// after each file completes and after target removal. It must not block.
    pub fn spawn(
        syncer: Syncer,
        clock: impl Clock,
        on_status: impl Fn(&Status) + Send + 'static,
    ) -> Result<Self> {
        Self::spawn_with(syncer, clock, on_status, worker::shared())
    }

    fn spawn_with(
        syncer: Syncer,
        clock: impl Clock,
        on_status: impl Fn(&Status) + Send + 'static,
        admission: Arc<Mutex<worker::Admission>>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        let status = Arc::new(Mutex::new(syncer.status().clone()));
        let shared = Arc::clone(&status);
        let stop = Cancel::default();
        let runner_stop = stop.clone();
        let runner_tx = tx.clone();
        let thread = thread::Builder::new()
            .name("pwrroute-sync".into())
            .spawn(move || {
                let publish = |syncer: &Syncer| {
                    if let Ok(mut s) = shared.lock() {
                        *s = syncer.status().clone();
                    }
                    on_status(syncer.status());
                };
                let mut coordinator = Coordinator::new(syncer, runner_stop, admission, runner_tx);
                loop {
                    let message = rx.recv_timeout(Duration::from_millis(25));
                    if coordinator.stop.check().is_err() {
                        break;
                    }
                    match message {
                        Ok(command) => {
                            if !coordinator.command(command, clock.now(), &publish) {
                                break;
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    // Drain a bounded batch before applying ready observations, in
                    // configured target order. Commands still get serviced during
                    // a burst of worker completions.
                    let mut stopped = false;
                    for _ in 0..63 {
                        let Ok(command) = rx.try_recv() else {
                            break;
                        };
                        if !coordinator.command(command, clock.now(), &publish) {
                            stopped = true;
                            break;
                        }
                    }
                    if stopped {
                        break;
                    }
                    coordinator.tick(clock.now(), &publish);
                    coordinator.drive(clock.now(), &publish);
                }
                coordinator.shutdown();
            })?;
        Ok(Self {
            tx,
            status,
            thread: Some(thread),
            stop,
        })
    }

    /// Runs a pass on the next tick: a save, Sync now, the window coming to
    /// the front.
    pub fn wake(&self) {
        let _ = self.tx.send(Command::Wake);
    }

    pub fn set_targets(&self, targets: Vec<Target>) {
        let _ = self.tx.send(Command::Targets(targets));
    }

    pub fn set_items(&self, items: Vec<Item>) {
        let _ = self.tx.send(Command::Items(items));
    }

    pub fn set_machine_name(&self, name: &str) {
        let _ = self.tx.send(Command::MachineName(name.to_owned()));
    }

    /// Settles a conflict using a fresh remote observation; `done` runs on
    /// the coordinator. A folder already syncing returns a busy error at once.
    pub fn resolve(
        &self,
        root: PathBuf,
        name: String,
        remote_sha: String,
        choice: Choice,
        done: impl FnOnce(Result<()>) + Send + 'static,
    ) {
        let _ = self.tx.send(Command::Resolve {
            root,
            name,
            remote_sha,
            choice,
            done: Box::new(done),
        });
    }

    /// The last status published.
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop.cancel();
        let _ = self.tx.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

mod coordinator;
mod worker;
use coordinator::{Completion, Coordinator, Outcome};

#[cfg(test)]
mod tests;
