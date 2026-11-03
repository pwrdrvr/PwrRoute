use super::worker::{self, Admission, Operation, Permit};
use super::*;
use crate::Health;
use std::{collections::HashMap, time::Instant};

pub(super) enum Outcome {
    Read(Observation),
    Written(Result<Manifest>),
    Failed,
}

pub(super) struct Completion {
    pub target: Target,
    pub cancel: Cancel,
    pub permit: Permit,
    pub outcome: Outcome,
}

struct Step {
    health: Health,
    manifest: Option<Manifest>,
    result: Result<()>,
}

type Done = Box<dyn FnOnce(Result<()>) + Send>;
struct Resolution {
    sha: String,
    choice: Choice,
    done: Done,
}

enum Phase {
    Idle,
    Reading,
    Ready(Observation, Permit),
    Writing(Write),
}

struct Run {
    index: usize,
    phase: Phase,
    cancel: Cancel,
    now: u64,
    repeat: bool,
    resolution: Option<Resolution>,
}

pub(super) struct Coordinator {
    syncer: Syncer,
    pub stop: Cancel,
    admission: Arc<Mutex<Admission>>,
    tx: Sender<Command>,
    runs: HashMap<PathBuf, Run>,
    schedule: Schedule,
    next_tick: Instant,
    requested: bool,
}

impl Coordinator {
    pub fn new(
        syncer: Syncer,
        stop: Cancel,
        admission: Arc<Mutex<Admission>>,
        tx: Sender<Command>,
    ) -> Self {
        Self {
            syncer,
            stop,
            admission,
            tx,
            runs: HashMap::new(),
            schedule: Schedule::new(EVERY.as_millis() as u64, TICK.as_millis() as u64),
            next_tick: Instant::now(),
            requested: true,
        }
    }

    pub fn command(&mut self, command: Command, now: u64, publish: &impl Fn(&Syncer)) -> bool {
        match command {
            Command::Stop => return false,
            Command::Wake => self.wake(),
            Command::Targets(targets) => {
                let removed: Vec<_> = self
                    .runs
                    .keys()
                    .filter(|root| !targets.iter().any(|t| &t.root == *root))
                    .cloned()
                    .collect();
                for root in removed {
                    self.retire(&root);
                }
                if self.syncer.set_targets(targets) {
                    publish(&self.syncer);
                }
                self.wake();
            }
            Command::Items(items) => {
                let roots: Vec<_> = self.runs.keys().cloned().collect();
                for root in roots {
                    self.retire(&root);
                }
                self.syncer.set_items(items);
                self.wake();
            }
            Command::MachineName(name) => {
                self.syncer.set_machine_name(&name);
                self.wake();
            }
            Command::Resolve {
                root,
                name,
                remote_sha,
                choice,
                done,
            } => {
                if self.runs.contains_key(&root) {
                    done(Err(anyhow::anyhow!(
                        "that folder is busy; try again after its current sync"
                    )));
                } else if !self.syncer.targets().iter().any(|t| t.root == root) {
                    done(Err(anyhow::anyhow!("that folder is no longer synced")));
                } else if let Some(index) = self.syncer.items().iter().position(|i| i.name == name)
                {
                    self.runs.insert(
                        root,
                        Run {
                            index,
                            phase: Phase::Idle,
                            cancel: self.stop.child(),
                            now,
                            repeat: false,
                            resolution: Some(Resolution {
                                sha: remote_sha,
                                choice,
                                done,
                            }),
                        },
                    );
                } else {
                    done(Err(anyhow::anyhow!("that file no longer syncs")));
                }
            }
            Command::Completed(completion) => {
                let Completion {
                    target,
                    cancel,
                    permit,
                    outcome,
                } = *completion;
                let Some(mut run) = self.runs.remove(&target.root) else {
                    return true;
                };
                if !run.cancel.same(&cancel) {
                    self.runs.insert(target.root, run);
                    return true;
                }
                match outcome {
                    Outcome::Read(observation) => run.phase = Phase::Ready(observation, permit),
                    Outcome::Written(result) => {
                        let Phase::Writing(write) = &run.phase else {
                            self.runs.insert(target.root, run);
                            return true;
                        };
                        let item = self.syncer.items()[run.index].clone();
                        let manifest = result.as_ref().ok().cloned();
                        let health =
                            self.syncer
                                .finish_push(&item, &target, write, result.map(|_| ()));
                        let outcome = match &health {
                            Health::Error(why) => Err(anyhow::anyhow!("{why}")),
                            _ => Ok(()),
                        };
                        self.finish(
                            &target,
                            &item,
                            &mut run,
                            Step {
                                health,
                                manifest,
                                result: outcome,
                            },
                            publish,
                        );
                        // permit drops only after the actual syscall has returned
                    }
                    Outcome::Failed => {
                        let item = self.syncer.items()[run.index].clone();
                        self.finish(
                            &target,
                            &item,
                            &mut run,
                            Step {
                                health: Health::Error("cloud worker failed".into()),
                                manifest: None,
                                result: Err(anyhow::anyhow!("cloud worker failed")),
                            },
                            publish,
                        );
                    }
                }
                self.retain_run(&target, run, now);
            }
        }
        true
    }

    fn wake(&mut self) {
        self.schedule.wake();
        self.requested = true;
    }

    fn retire(&mut self, root: &PathBuf) {
        if let Some(mut run) = self.runs.remove(root) {
            run.cancel.cancel();
            if let Some(resolution) = run.resolution.take() {
                (resolution.done)(Err(anyhow::anyhow!("sync operation was retired")));
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.stop.cancel();
        let roots: Vec<_> = self.runs.keys().cloned().collect();
        for root in roots {
            self.retire(&root);
        }
    }

    pub fn tick(&mut self, now: u64, publish: &impl Fn(&Syncer)) {
        if !self.requested && Instant::now() < self.next_tick {
            return;
        }
        self.requested = false;
        self.next_tick = Instant::now() + TICK;
        if self.syncer.targets().is_empty() {
            return;
        }
        if self
            .schedule
            .tick(now, self.syncer.local_stamps())
            .is_none()
        {
            return;
        }
        for target in self.syncer.targets().to_vec() {
            if let Some(run) = self.runs.get_mut(&target.root) {
                run.repeat = true;
            } else if !self.syncer.items().is_empty() {
                self.syncer.begin_target(&target);
                self.runs.insert(
                    target.root,
                    Run {
                        index: 0,
                        phase: Phase::Idle,
                        cancel: self.stop.child(),
                        now,
                        repeat: false,
                        resolution: None,
                    },
                );
            }
        }
        publish(&self.syncer);
    }

    pub fn drive(&mut self, now: u64, publish: &impl Fn(&Syncer)) {
        // Never wait for an earlier target. Among ready observations, configured
        // order is the tie-breaker. All local decisions/commits stay on this owner
        // and re-read the current local bytes and base inside prepare().
        for target in self.syncer.targets().to_vec() {
            if self.stop.check().is_err() {
                break;
            }
            let Some(mut run) = self.runs.remove(&target.root) else {
                continue;
            };
            if run.index >= self.syncer.items().len() {
                self.retain_run(&target, run, now);
                continue;
            }
            let item = self.syncer.items()[run.index].clone();
            match std::mem::replace(&mut run.phase, Phase::Reading) {
                Phase::Idle => {
                    if let Some(permit) = Permit::acquire(&self.admission, &target.root) {
                        let cached = if run.resolution.is_some() {
                            None
                        } else {
                            self.syncer.cached(&target, &item)
                        };
                        if let Err(e) = worker::spawn(
                            self.syncer.io.clone(),
                            target.clone(),
                            run.cancel.clone(),
                            permit,
                            Operation::Read {
                                name: item.name.clone(),
                                cached,
                            },
                            self.tx.clone(),
                        ) {
                            self.finish(
                                &target,
                                &item,
                                &mut run,
                                Step {
                                    health: Health::Error(e.to_string()),
                                    manifest: None,
                                    result: Err(e.into()),
                                },
                                publish,
                            );
                        }
                    } else {
                        run.phase = Phase::Idle;
                    }
                }
                Phase::Ready(observation, permit) => {
                    let manifest = observation.manifest.clone();
                    let (health, write, result) = if let Some(resolution) = &run.resolution {
                        match self.syncer.prepare_resolution(
                            &target,
                            &item,
                            &resolution.sha,
                            resolution.choice,
                            observation,
                            now,
                        ) {
                            Ok(write) => (
                                if write.is_some() {
                                    Health::Waiting(crate::Wait::Settling)
                                } else {
                                    Health::Pulled
                                },
                                write,
                                Ok(()),
                            ),
                            Err(e) => (Health::Error(e.to_string()), None, Err(e)),
                        }
                    } else {
                        let (health, write) = self.syncer.prepare(&item, &target, observation, now);
                        (health, write, Ok(()))
                    };
                    if let Some(write) = write {
                        // The permit follows read -> write -> completion. A busy
                        // target never gets a second worker, even across retirement.
                        let operation = Operation::Write(write.clone());
                        run.phase = Phase::Writing(write);
                        if let Err(e) = worker::spawn(
                            self.syncer.io.clone(),
                            target.clone(),
                            run.cancel.clone(),
                            permit,
                            operation,
                            self.tx.clone(),
                        ) {
                            self.finish(
                                &target,
                                &item,
                                &mut run,
                                Step {
                                    health: Health::Error(e.to_string()),
                                    manifest,
                                    result: Err(e.into()),
                                },
                                publish,
                            );
                        }
                    } else {
                        self.finish(
                            &target,
                            &item,
                            &mut run,
                            Step {
                                health,
                                manifest,
                                result,
                            },
                            publish,
                        );
                    }
                }
                phase => run.phase = phase,
            }
            self.retain_run(&target, run, now);
        }
    }

    fn finish(
        &mut self,
        target: &Target,
        item: &Item,
        run: &mut Run,
        step: Step,
        publish: &impl Fn(&Syncer),
    ) {
        let Step {
            health,
            manifest,
            result,
        } = step;
        let pulled = health == Health::Pulled;
        let resolving = run.resolution.is_some();
        let finished = resolving || run.index + 1 == self.syncer.items().len();
        self.syncer
            .record(target, &item.name, health, manifest, run.now, finished);
        run.index = if resolving {
            self.syncer.items().len()
        } else {
            run.index + 1
        };
        run.phase = Phase::Idle;
        if let Some(resolution) = run.resolution.take() {
            (resolution.done)(result);
            self.wake();
        }
        // A pull is a new local edit for targets already visited, including ones
        // whose reads/writes are still in flight. Coalesce one subsequent pass.
        if pulled {
            self.wake();
        }
        if finished {
            self.schedule.passed_with(self.syncer.local_stamps());
        }
        publish(&self.syncer);
    }

    fn retain_run(&mut self, target: &Target, mut run: Run, now: u64) {
        if run.index < self.syncer.items().len() {
            self.runs.insert(target.root.clone(), run);
        } else if run.repeat {
            run.index = 0;
            run.phase = Phase::Idle;
            run.repeat = false;
            run.now = now;
            self.syncer.begin_target(target);
            self.runs.insert(target.root.clone(), run);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Content, Kind, Machine, hash,
        remote::{Disk, Remote},
    };
    use std::fs;

    fn target(root: &std::path::Path) -> Target {
        Target {
            root: root.into(),
            kind: Kind::Folder,
            label: root.display().to_string(),
        }
    }
    fn item(local: PathBuf, name: &str) -> Item {
        Item {
            local,
            name: name.into(),
            check: Arc::new(|_| Content::Valid),
            runs_after: None,
        }
    }
    fn coordinator(syncer: Syncer) -> (Coordinator, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::channel();
        (
            Coordinator::new(syncer, Cancel::default(), Admission::new(worker::LIMIT), tx),
            rx,
        )
    }

    #[test]
    fn divergent_ready_snapshots_redecide_from_current_local_and_keep_a_conflict() {
        let data = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let targets = [target(first.path()), target(second.path())];
        let local = data.path().join("rules.json");
        fs::write(&local, b"base").unwrap();
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![item(local.clone(), "rules.json")]);
        syncer.set_targets(targets.to_vec());
        syncer.pass(1);
        for (target, bytes) in targets
            .iter()
            .zip([b"first".as_slice(), b"second".as_slice()])
        {
            target
                .folder()
                .write("rules.json", bytes, &Machine::default(), 2, "test", None)
                .unwrap();
        }
        let (mut c, _rx) = coordinator(syncer);
        c.tick(3, &|_| {});
        // Deliver in reverse arrival order. Both observations were made when
        // local was still base; configured order determines the first apply.
        for target in targets.iter().rev() {
            let observation = Disk.read(target, "rules.json", None, &Cancel::default());
            let permit = Permit::acquire(&c.admission, &target.root).unwrap();
            c.runs.get_mut(&target.root).unwrap().phase = Phase::Ready(observation, permit);
        }
        c.drive(3, &|_| {});
        assert_eq!(fs::read(&local).unwrap(), b"first");
        let status = c.syncer.status();
        assert_eq!(status.targets[0].files[0].health, Health::Pulled);
        let Health::Conflict(conflict) = &status.targets[1].files[0].health else {
            panic!("{status:?}");
        };
        assert_eq!(conflict.local.as_deref(), Some(b"first".as_slice()));
        assert_eq!(conflict.remote, b"second");
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(data.path().join("sync/state.json")).unwrap())
                .unwrap();
        assert_eq!(
            state["bases"][targets[0].id()]["rules.json"],
            hash(b"first")
        );
        assert_eq!(state["bases"][targets[1].id()]["rules.json"], hash(b"base"));
        for bytes in [b"base".as_slice(), b"second".as_slice()] {
            assert!(
                status
                    .backups
                    .iter()
                    .any(|b| fs::read(&b.path).unwrap() == bytes)
            );
        }
    }

    #[test]
    fn an_item_generation_change_discards_a_late_successful_read() {
        let data = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let target = target(cloud.path());
        target
            .folder()
            .write(
                "rules.json",
                b"remote",
                &Machine::default(),
                1,
                "test",
                None,
            )
            .unwrap();
        let old = data.path().join("old.json");
        let new = data.path().join("new.json");
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![item(old.clone(), "rules.json")]);
        syncer.set_targets(vec![target.clone()]);
        let (mut c, _rx) = coordinator(syncer);
        c.tick(1, &|_| {});
        let cancel = c.runs[&target.root].cancel.clone();
        let permit = Permit::acquire(&c.admission, &target.root).unwrap();
        let outcome = Outcome::Read(Disk.read(&target, "rules.json", None, &Cancel::default()));
        c.command(
            Command::Items(vec![item(new.clone(), "rules.json")]),
            2,
            &|_| {},
        );
        c.tick(2, &|_| {});
        c.command(
            Command::Completed(Box::new(Completion {
                target: target.clone(),
                cancel,
                permit,
                outcome,
            })),
            2,
            &|_| {},
        );
        assert!(matches!(c.runs[&target.root].phase, Phase::Idle));
        assert!(!old.exists() && !new.exists());
        assert_eq!(Permit::active(&c.admission), 0);
    }

    #[test]
    fn item_order_is_preserved_while_other_targets_can_advance() {
        let data = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let target = target(cloud.path());
        let config = data.path().join("finicky.ts");
        let rules = data.path().join("rules.json");
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![
            item(config.clone(), "finicky.ts"),
            item(rules.clone(), "rules.json"),
        ]);
        syncer.set_targets(vec![target.clone()]);
        target
            .folder()
            .write(
                "finicky.ts",
                b"config",
                &Machine::default(),
                1,
                "test",
                None,
            )
            .unwrap();
        target
            .folder()
            .write(
                "rules.json",
                b"rules",
                &Machine::default(),
                1,
                "test",
                Some("~/.finicky.ts"),
            )
            .unwrap();
        let before = target.folder().manifest().unwrap();
        let (mut c, _rx) = coordinator(syncer);
        c.tick(2, &|_| {});
        for name in ["finicky.ts", "rules.json"] {
            assert_eq!(c.syncer.items()[c.runs[&target.root].index].name, name);
            let observation = Disk.read(&target, name, None, &Cancel::default());
            let permit = Permit::acquire(&c.admission, &target.root).unwrap();
            c.runs.get_mut(&target.root).unwrap().phase = Phase::Ready(observation, permit);
            c.drive(2, &|_| {});
            assert_eq!(fs::read(&config).unwrap(), b"config");
            if name == "finicky.ts" {
                assert!(!rules.exists());
            }
        }
        assert_eq!(fs::read(rules).unwrap(), b"rules");
        assert_eq!(
            target.folder().manifest().unwrap(),
            before,
            "pulls leave runsAfter and manifest alone"
        );
    }
    #[test]
    fn writer_guard_contention_does_not_wait_or_apply_on_the_coordinator() {
        let data = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let target = target(cloud.path());
        let local = data.path().join("rules.json");
        let held = crate::LocalFile::lock(&local).unwrap();
        target
            .folder()
            .write(
                "rules.json",
                b"remote",
                &Machine::default(),
                1,
                "test",
                None,
            )
            .unwrap();
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![item(local.clone(), "rules.json")]);
        syncer.set_targets(vec![target.clone()]);
        let (mut c, _rx) = coordinator(syncer);
        c.tick(2, &|_| {});
        let observed = Disk.read(&target, "rules.json", None, &Cancel::default());
        let permit = Permit::acquire(&c.admission, &target.root).unwrap();
        c.runs.get_mut(&target.root).unwrap().phase = Phase::Ready(observed, permit);
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            c.drive(2, &|_| {});
            let _ = done_tx.send(c.syncer.status().clone());
        });
        let status = done_rx.recv_timeout(Duration::from_secs(2));
        drop(held);
        worker.join().unwrap();
        let status = status.expect("coordinator must not wait on a cooperating writer");
        assert_eq!(
            status.targets[0].files[0].health,
            Health::Waiting(crate::Wait::Settling)
        );
        assert!(!local.exists());
    }
    fn status_fixture() -> (tempfile::TempDir, tempfile::TempDir, Target, Coordinator) {
        let data = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let target = target(cloud.path());
        let local = data.path().join("rules.json");
        fs::write(&local, b"rules").unwrap();
        target.folder().write("rules.json", b"rules", &Machine { id: "writer".into(), name: "Writer".into() }, 1, "test", None).unwrap();
        target.folder().write("finicky.ts", b"config", &Machine::default(), 2, "test", None).unwrap();
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![item(local, "rules.json")]);
        syncer.set_targets(vec![target.clone()]);
        syncer.pass(3);
        assert_eq!(syncer.status().targets[0].folder_files.len(), 2);
        let (coordinator, _rx) = coordinator(syncer);
        (data, cloud, target, coordinator)
    }

    fn ready(c: &mut Coordinator, target: &Target) {
        let observation = Disk.read(target, "rules.json", None, &Cancel::default());
        let permit = Permit::acquire(&c.admission, &target.root).unwrap();
        c.runs.get_mut(&target.root).unwrap().phase = Phase::Ready(observation, permit);
    }

    #[test]
    fn review_missing_or_invalid_manifest_clears_previous_status_metadata() {
        for replacement in [None, Some(b"{ broken".as_slice()), Some(br#"{"format":999,"files":{}}"#.as_slice())] {
            let (_data, _cloud, target, mut c) = status_fixture();
            let path = target.folder().file(crate::MANIFEST);
            match replacement { Some(bytes) => fs::write(&path, bytes).unwrap(), None => fs::remove_file(&path).unwrap() }
            c.tick(1_000, &|_| {});
            assert_eq!(c.syncer.status().targets[0].folder_files.len(), 2, "starting a read retains the last known manifest");
            ready(&mut c, &target);
            c.drive(1_100, &|_| {});
            let status = &c.syncer.status().targets[0];
            assert_eq!(status.files[0].health, Health::InSync);
            assert!(status.saved_by.is_none(), "completed absence clears authorship");
            assert!(status.saved_at.is_none(), "completed absence clears saved time");
            assert!(status.folder_files.is_empty(), "completed absence clears import candidates");
        }
    }

    #[test]
    fn review_failure_before_observing_manifest_retains_previous_metadata() {
        let (_data, _cloud, target, mut c) = status_fixture();
        let before = c.syncer.status().targets[0].clone();
        c.tick(1_000, &|_| {});
        let cancel = c.runs[&target.root].cancel.clone();
        let permit = Permit::acquire(&c.admission, &target.root).unwrap();
        c.command(Command::Completed(Box::new(Completion { target, cancel, permit, outcome: Outcome::Failed })), 1_100, &|_| {});
        let after = &c.syncer.status().targets[0];
        assert_eq!(after.saved_by, before.saved_by);
        assert_eq!(after.saved_at, before.saved_at);
        assert_eq!(after.folder_files, before.folder_files);
    }

    #[test]
    fn review_old_run_finishing_after_a_newer_target_uses_completion_time() {
        let (data, _cloud, slow, mut c) = status_fixture();
        c.tick(1_000, &|_| {});
        // Own the old target's permit without starting a real worker.
        c.runs.get_mut(&slow.root).unwrap().phase = Phase::Reading;
        let slow_permit = Permit::acquire(&c.admission, &slow.root).unwrap();
        let slow_cancel = c.runs[&slow.root].cancel.clone();
        let slow_observation = Disk.read(&slow, "rules.json", None, &Cancel::default());
        let fast_cloud = tempfile::tempdir().unwrap();
        let fast = target(fast_cloud.path());
        fast.folder().write("rules.json", b"rules", &Machine::default(), 2, "test", None).unwrap();
        c.command(Command::Targets(vec![slow.clone(), fast.clone()]), 2_000, &|_| {});
        c.tick(2_000, &|_| {});
        ready(&mut c, &fast);
        c.drive(2_100, &|_| {});
        let fast_completed = c.syncer.status().last_pass;
        c.command(Command::Completed(Box::new(Completion { target: slow.clone(), cancel: slow_cancel, permit: slow_permit, outcome: Outcome::Read(slow_observation) })), 3_000, &|_| {});
        c.drive(3_000, &|_| {});
        assert_eq!(fast_completed, Some(2_100));
        assert_eq!(c.syncer.status().last_pass, Some(3_000), "an older run cannot move status back to its start time");
        assert_eq!(c.syncer.status().targets[0].last_synced, Some(3_000));
        assert_eq!(c.syncer.status().targets[1].last_synced, Some(2_100));
        let state: serde_json::Value = serde_json::from_slice(&fs::read(data.path().join("sync/state.json")).unwrap()).unwrap();
        assert_eq!(state["lastSynced"][slow.id()], 3_000);
        assert_eq!(state["lastSynced"][fast.id()], 2_100);
    }

    #[test]
    fn review_pending_write_records_status_at_completion_without_rewriting_manifest_time() {
        let data = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let target = target(cloud.path());
        let local = data.path().join("rules.json");
        fs::write(&local, b"rules").unwrap();
        let item = item(local, "rules.json");
        let mut syncer = Syncer::new(data.path(), "test", "test");
        syncer.set_items(vec![item.clone()]);
        syncer.set_targets(vec![target.clone()]);
        let (mut c, _rx) = coordinator(syncer);
        c.tick(1_000, &|_| {});
        let observation = Disk.read(&target, &item.name, None, &Cancel::default());
        let (_, write) = c.syncer.prepare(&item, &target, observation, 1_000);
        let write = write.unwrap();
        let manifest = Disk.write(&target, &write, &Cancel::default()).unwrap();
        let cancel = c.runs[&target.root].cancel.clone();
        c.runs.get_mut(&target.root).unwrap().phase = Phase::Writing(write);
        let permit = Permit::acquire(&c.admission, &target.root).unwrap();
        c.command(Command::Completed(Box::new(Completion { target: target.clone(), cancel, permit, outcome: Outcome::Written(Ok(manifest)) })), 5_000, &|_| {});
        assert_eq!(c.syncer.status().last_pass, Some(5_000));
        assert_eq!(c.syncer.status().targets[0].last_synced, Some(5_000));
        assert_eq!(c.syncer.status().targets[0].saved_at, Some(1_000), "manifest attribution stays with the written content");
        let state: serde_json::Value = serde_json::from_slice(&fs::read(data.path().join("sync/state.json")).unwrap()).unwrap();
        assert_eq!(state["lastSynced"][target.id()], 5_000);
    }

}
