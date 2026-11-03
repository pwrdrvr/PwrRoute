use super::*;
use crate::{
    Content, Health, Kind,
    remote::{Cancel, Disk, Observation, Remote},
    syncer::Seen,
};
use std::fs;

const DEADLINE: Duration = Duration::from_secs(2);

struct Gated {
    root: PathBuf,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl Remote for Gated {
    fn read(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
    ) -> Observation {
        let observation = Disk.read(target, name, cached, cancel);
        if target.root == self.root {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        observation
    }
}

fn target(root: PathBuf) -> Target {
    Target {
        kind: Kind::Folder,
        label: root.display().to_string(),
        root,
    }
}

fn setup() -> (tempfile::TempDir, Syncer) {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("rules.json");
    fs::write(&local, b"local").unwrap();
    let mut syncer = Syncer::new(dir.path(), "test", "test");
    syncer.set_items(vec![Item {
        name: "rules.json".into(),
        local,
        check: Arc::new(|_| Content::Valid),
        runs_after: None,
    }]);
    (dir, syncer)
}

fn receive_until(rx: &mpsc::Receiver<Status>, predicate: impl Fn(&Status) -> bool) -> bool {
    let end = std::time::Instant::now() + DEADLINE;
    while let Ok(status) = rx.recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
    {
        if predicate(&status) {
            return true;
        }
    }
    false
}

#[test]
fn blocked_cloud_read_does_not_stall_healthy_status_or_target_removal() {
    let (_data, mut syncer) = setup();
    let blocked = tempfile::tempdir().unwrap();
    let healthy = tempfile::tempdir().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    syncer.io = Arc::new(Gated {
        root: blocked.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    syncer.set_targets(vec![
        target(blocked.path().into()),
        target(healthy.path().into()),
    ]);
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    let progressed = receive_until(&rx, |s| {
        s.targets.iter().any(|t| {
            t.target.root == healthy.path() && t.files.iter().any(|f| f.health == Health::Pushed)
        })
    });
    service.set_targets(vec![target(healthy.path().into())]);
    let removed = receive_until(&rx, |s| {
        s.targets.len() == 1 && s.targets[0].target.root == healthy.path()
    });
    let _ = release_tx.send(());
    drop(service);
    assert!(
        progressed,
        "healthy target must publish while the first target remains blocked"
    );
    assert!(
        removed,
        "target removal must publish while cloud I/O remains blocked"
    );
}

#[test]
fn dropping_service_does_not_join_an_uninterruptible_cloud_read() {
    let (_data, mut syncer) = setup();
    let blocked = tempfile::tempdir().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    syncer.io = Arc::new(Gated {
        root: blocked.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    syncer.set_targets(vec![target(blocked.path().into())]);
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        |_| {},
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let dropping = thread::spawn(move || {
        drop(service);
        done_tx.send(()).unwrap();
    });
    let stopped = done_rx.recv_timeout(DEADLINE).is_ok();
    let _ = release_tx.send(());
    dropping.join().unwrap();
    assert!(
        stopped,
        "Drop must finish while the read still owns its worker slot"
    );
}

struct AllGated {
    entered: mpsc::Sender<PathBuf>,
    gates: std::collections::HashMap<PathBuf, Mutex<mpsc::Receiver<()>>>,
}
impl Remote for AllGated {
    fn read(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
    ) -> Observation {
        let observed = Disk.read(target, name, cached, cancel);
        let _ = self.entered.send(target.root.clone());
        if let Some(gate) = self.gates.get(&target.root) {
            let _ = gate.lock().unwrap().recv();
        }
        observed
    }
}

#[test]
fn saturation_keeps_control_and_stop_responsive_without_replacement_workers() {
    let (_data, mut syncer) = setup();
    let clouds: Vec<_> = (0..worker::LIMIT)
        .map(|_| tempfile::tempdir().unwrap())
        .collect();
    let (entered_tx, entered_rx) = mpsc::channel();
    let mut gates = std::collections::HashMap::new();
    let mut releases = Vec::new();
    for cloud in &clouds {
        let (tx, rx) = mpsc::channel();
        releases.push(tx);
        gates.insert(cloud.path().to_path_buf(), Mutex::new(rx));
    }
    syncer.io = Arc::new(AllGated {
        entered: entered_tx,
        gates,
    });
    syncer.set_targets(clouds.iter().map(|c| target(c.path().into())).collect());
    let admission = worker::Admission::new(worker::LIMIT);
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        admission.clone(),
    )
    .unwrap();
    for _ in &clouds {
        entered_rx.recv_timeout(DEADLINE).unwrap();
    }
    assert_eq!(worker::Permit::active(&admission), worker::LIMIT);
    for _ in 0..100 {
        service.wake();
    }
    service.set_targets(Vec::new());
    let removed = receive_until(&rx, |s| s.targets.is_empty());
    let (done_tx, done_rx) = mpsc::channel();
    let dropping = thread::spawn(move || {
        drop(service);
        let _ = done_tx.send(());
    });
    let stopped = done_rx.recv_timeout(DEADLINE).is_ok();
    // A replacement Service shares the same admission. Its healthy target may
    // wait for capacity, but removal and Stop must still work.
    let (_other_data, mut replacement) = setup();
    let healthy = tempfile::tempdir().unwrap();
    replacement.set_targets(vec![target(healthy.path().into())]);
    let (tx, rx) = mpsc::channel();
    let other = Service::spawn_with(
        replacement,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        admission.clone(),
    )
    .unwrap();
    other.wake();
    let waiting = receive_until(&rx, |s| !s.targets.is_empty());
    other.set_targets(Vec::new());
    let other_removed = receive_until(&rx, |s| s.targets.is_empty());
    drop(other);
    let still_full = worker::Permit::active(&admission) == worker::LIMIT;
    let no_replacements = entered_rx.try_recv().is_err();
    for release in releases {
        let _ = release.send(());
    }
    dropping.join().unwrap();
    assert!(removed && stopped && waiting && other_removed);
    assert!(
        still_full && no_replacements,
        "retired syscalls retain all capacity across service restart"
    );
    assert!(!healthy.path().join("PwrRoute/rules.json").exists());
}

#[test]
fn a_healthy_conflict_can_be_resolved_while_another_target_is_blocked() {
    let (data, mut syncer) = setup();
    let blocked = tempfile::tempdir().unwrap();
    let healthy = tempfile::tempdir().unwrap();
    target(healthy.path().into())
        .folder()
        .write(
            "rules.json",
            b"remote",
            &crate::Machine::default(),
            1,
            "test",
            None,
        )
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    syncer.io = Arc::new(Gated {
        root: blocked.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    syncer.set_targets(vec![
        target(blocked.path().into()),
        target(healthy.path().into()),
    ]);
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    let conflict = receive_until(&rx, |s| {
        s.targets.iter().any(|t| {
            t.target.root == healthy.path()
                && t.files
                    .iter()
                    .any(|f| matches!(f.health, Health::Conflict(_)))
        })
    });
    let (done_tx, done_rx) = mpsc::channel();
    service.resolve(
        healthy.path().into(),
        "rules.json".into(),
        crate::hash(b"remote"),
        Choice::UseFolder,
        move |r| {
            let _ = done_tx.send(r);
        },
    );
    let resolved = done_rx.recv_timeout(DEADLINE).is_ok_and(|r| r.is_ok());
    let applied = fs::read(data.path().join("rules.json")).unwrap() == b"remote";
    let (busy_tx, busy_rx) = mpsc::channel();
    service.resolve(
        blocked.path().into(),
        "rules.json".into(),
        crate::hash(b"remote"),
        Choice::UseFolder,
        move |r| {
            let _ = busy_tx.send(r);
        },
    );
    let busy = busy_rx.recv_timeout(DEADLINE).is_ok_and(|r| r.is_err());
    let _ = release_tx.send(());
    drop(release_tx);
    drop(service);
    assert!(conflict && resolved && applied && busy);
}

#[test]
fn a_late_snapshot_after_remove_and_readd_cannot_apply_old_bytes() {
    let (data, mut syncer) = setup();
    let local = data.path().join("rules.json");
    fs::remove_file(&local).unwrap();
    let cloud = tempfile::tempdir().unwrap();
    let folder = target(cloud.path().into()).folder();
    folder
        .write(
            "rules.json",
            b"old",
            &crate::Machine::default(),
            1,
            "test",
            None,
        )
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    syncer.io = Arc::new(Gated {
        root: cloud.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    syncer.set_targets(vec![target(cloud.path().into())]);
    let (tx, rx) = mpsc::channel();
    let observed_local = local.clone();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send((s.clone(), fs::read(&observed_local).ok()));
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    service.set_targets(Vec::new());
    let end = std::time::Instant::now() + DEADLINE;
    while !rx
        .recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
        .unwrap()
        .0
        .targets
        .is_empty()
    {}
    folder
        .write(
            "rules.json",
            b"new",
            &crate::Machine::default(),
            2,
            "test",
            None,
        )
        .unwrap();
    service.set_targets(vec![target(cloud.path().into())]);
    let _ = release_tx.send(());
    drop(release_tx);
    let end = std::time::Instant::now() + DEADLINE;
    let mut applied_old = false;
    let mut applied_new = false;
    while let Ok((status, bytes)) =
        rx.recv_timeout(end.saturating_duration_since(std::time::Instant::now()))
    {
        if status
            .targets
            .iter()
            .any(|t| t.files.iter().any(|f| f.health == Health::Pulled))
        {
            applied_old |= bytes.as_deref() == Some(b"old");
            if bytes.as_deref() == Some(b"new") {
                applied_new = true;
                break;
            }
        }
    }
    drop(service);
    assert!(
        applied_new && !applied_old,
        "only the readded target's fresh generation can apply"
    );
    assert_eq!(fs::read(local).unwrap(), b"new");
}

struct WriteGated {
    root: PathBuf,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    returned: mpsc::Sender<()>,
}
impl Remote for WriteGated {
    fn read(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
    ) -> Observation {
        Disk.read(target, name, cached, cancel)
    }
    fn write(
        &self,
        target: &Target,
        write: &crate::remote::Write,
        cancel: &Cancel,
    ) -> Result<Manifest> {
        if target.root == self.root {
            let _ = self.entered.send(());
            let _ = self.release.lock().unwrap().recv();
        }
        let result = Disk.write(target, write, cancel);
        let _ = self.returned.send(());
        result
    }
}

#[test]
fn blocked_remote_write_does_not_stall_other_targets_and_cannot_start_after_stop() {
    let (_data, mut syncer) = setup();
    let blocked = tempfile::tempdir().unwrap();
    let healthy = tempfile::tempdir().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (returned_tx, returned_rx) = mpsc::channel();
    syncer.io = Arc::new(WriteGated {
        root: blocked.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
        returned: returned_tx,
    });
    syncer.set_targets(vec![
        target(blocked.path().into()),
        target(healthy.path().into()),
    ]);
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    let healthy_progress = receive_until(&rx, |s| {
        s.targets.iter().any(|t| {
            t.target.root == healthy.path() && t.files.iter().any(|f| f.health == Health::Pushed)
        })
    });
    service.set_targets(vec![target(healthy.path().into())]);
    let removed = receive_until(&rx, |s| {
        s.targets.len() == 1 && s.targets[0].target.root == healthy.path()
    });
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let dropping = thread::spawn(move || {
        drop(service);
        let _ = stopped_tx.send(());
    });
    let stopped = stopped_rx.recv_timeout(DEADLINE).is_ok();
    // Ignore healthy write's notification; after release the retired writer
    // explicitly acknowledges cancellation before any new live-file mutation.
    while returned_rx.try_recv().is_ok() {}
    let _ = release_tx.send(());
    returned_rx.recv_timeout(DEADLINE).unwrap();
    dropping.join().unwrap();
    assert!(healthy_progress && removed && stopped);
    assert_eq!(
        fs::read(healthy.path().join("PwrRoute/rules.json")).unwrap(),
        b"local"
    );
    assert!(!blocked.path().join("PwrRoute/rules.json").exists());
}

#[test]
fn a_remote_edit_during_a_pending_push_is_preserved_and_becomes_a_conflict() {
    let (data, mut syncer) = setup();
    let cloud = tempfile::tempdir().unwrap();
    let t = target(cloud.path().into());
    syncer.set_targets(vec![t.clone()]);
    syncer.pass(1);
    fs::write(data.path().join("rules.json"), b"local edit").unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (returned_tx, _returned_rx) = mpsc::channel();
    syncer.io = Arc::new(WriteGated {
        root: cloud.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
        returned: returned_tx,
    });
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    t.folder()
        .write(
            "rules.json",
            b"remote edit",
            &crate::Machine::default(),
            2,
            "test",
            None,
        )
        .unwrap();
    let _ = release_tx.send(());
    drop(release_tx);
    assert!(receive_until(&rx, |s| s.targets[0]
        .files
        .iter()
        .any(|f| matches!(f.health, Health::Error(_)))));
    service.wake();
    assert!(receive_until(&rx, |s| s.targets[0]
        .files
        .iter()
        .any(|f| matches!(f.health, Health::Conflict(_)))));
    drop(service);
    assert_eq!(
        fs::read(data.path().join("rules.json")).unwrap(),
        b"local edit"
    );
    assert_eq!(
        fs::read(t.folder().file("rules.json")).unwrap(),
        b"remote edit"
    );
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(data.path().join("sync/state.json")).unwrap()).unwrap();
    assert_eq!(state["bases"][t.id()]["rules.json"], crate::hash(b"local"));
}

#[test]
fn a_pull_propagates_to_targets_already_visited_with_runs_after_only_on_push() {
    let (data, mut syncer) = setup();
    let earlier = tempfile::tempdir().unwrap();
    let later = tempfile::tempdir().unwrap();
    let first = target(earlier.path().into());
    let second = target(later.path().into());
    let mut items = syncer.items().to_vec();
    items[0].runs_after = Some("~/.finicky.ts".into());
    syncer.set_items(items);
    syncer.set_targets(vec![first.clone(), second.clone()]);
    syncer.pass(1);
    second
        .folder()
        .write(
            "rules.json",
            b"from later",
            &crate::Machine::default(),
            2,
            "test",
            Some("~/.other.ts"),
        )
        .unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    syncer.io = Arc::new(Gated {
        root: later.path().into(),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    let (tx, rx) = mpsc::channel();
    let service = Service::spawn_with(
        syncer,
        SystemClock,
        move |s| {
            let _ = tx.send(s.clone());
        },
        worker::Admission::new(worker::LIMIT),
    )
    .unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    assert!(receive_until(&rx, |s| s.targets.iter().any(|t| t
        .target
        .root
        == earlier.path()
        && t.files.iter().any(|f| f.health == Health::InSync))));
    let _ = release_tx.send(());
    drop(release_tx);
    assert!(receive_until(&rx, |s| s.targets.iter().any(|t| t
        .target
        .root
        == earlier.path()
        && t.files.iter().any(|f| f.health == Health::Pushed))));
    drop(service);
    assert_eq!(
        fs::read(data.path().join("rules.json")).unwrap(),
        b"from later"
    );
    for target in [&first, &second] {
        assert_eq!(
            fs::read(target.folder().file("rules.json")).unwrap(),
            b"from later"
        );
    }
    assert_eq!(
        first.folder().manifest().unwrap().files["rules.json"]
            .runs_after
            .as_deref(),
        Some("~/.finicky.ts")
    );
    assert_eq!(
        second.folder().manifest().unwrap().files["rules.json"]
            .runs_after
            .as_deref(),
        Some("~/.other.ts")
    );
}
