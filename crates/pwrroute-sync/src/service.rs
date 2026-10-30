//! The sync thread: a [`Syncer`] driven by a [`Schedule`] and a [`Clock`].
//!
//! The app owns one [`Service`]. Every call sends a command to the thread
//! and returns at once; the thread ticks every [`Schedule::tick`], runs a
//! pass when the schedule says so, and hands each new [`Status`] to the
//! callback it was given. Reads of cloud folders, which can block while a
//! client downloads a file, happen only on this thread.

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
    schedule::{Schedule, Why},
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
}

pub struct Service {
    tx: Sender<Command>,
    status: Arc<Mutex<Status>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").finish_non_exhaustive()
    }
}

impl Service {
    /// Starts the thread. `on_status` runs on it after every pass and every
    /// resolution.
    pub fn spawn(
        mut syncer: Syncer,
        clock: impl Clock,
        on_status: impl Fn(&Status) + Send + 'static,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        let status = Arc::new(Mutex::new(syncer.status().clone()));
        let shared = Arc::clone(&status);
        let thread = thread::Builder::new()
            .name("pwrroute-sync".into())
            .spawn(move || {
                let mut schedule = Schedule::new(EVERY.as_millis() as u64, TICK.as_millis() as u64);
                let publish = |syncer: &Syncer| {
                    if let Ok(mut s) = shared.lock() {
                        *s = syncer.status().clone();
                    }
                    on_status(syncer.status());
                };
                loop {
                    match rx.recv_timeout(TICK) {
                        Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(Command::Wake) => schedule.wake(),
                        Ok(Command::Targets(targets)) => {
                            syncer.set_targets(targets);
                            schedule.wake();
                        }
                        Ok(Command::Items(items)) => {
                            syncer.set_items(items);
                            schedule.wake();
                        }
                        Ok(Command::MachineName(name)) => syncer.set_machine_name(&name),
                        Ok(Command::Resolve {
                            root,
                            name,
                            remote_sha,
                            choice,
                            done,
                        }) => {
                            let result =
                                syncer.resolve(&root, &name, &remote_sha, choice, clock.now());
                            done(result);
                            schedule.wake();
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    if syncer.targets().is_empty() {
                        continue;
                    }
                    let now = clock.now();
                    if let Some(why) = schedule.tick(now, syncer.local_stamps()) {
                        if why != Why::Interval {
                            log::debug!("sync: pass ({why:?})");
                        }
                        syncer.pass(now);
                        schedule.passed_with(syncer.local_stamps());
                        publish(&syncer);
                    }
                }
            })?;
        Ok(Self {
            tx,
            status,
            thread: Some(thread),
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

    /// Settles a conflict on the sync thread; `done` gets the outcome there.
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
        let _ = self.tx.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
