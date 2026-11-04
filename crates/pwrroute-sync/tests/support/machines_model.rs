//! A revision DAG is the oracle, not sync's three-way decision table.
//! User edits retire their parent; explicit resolutions add supersession
//! edges. Every other replacement must move forward in this history.
use super::*;
use crate::remote::IoPoint;
use proptest::prelude::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

const NAME: &str = "rules.json";
fn bytes(rev: usize) -> Vec<u8> {
    // Distinct lengths also exercise the remote stamp cache without sleeps.
    format!("revision {rev}\n{}\ncomplete", ".".repeat(rev)).into_bytes()
}
fn revision(b: &[u8]) -> Option<usize> {
    let r = std::str::from_utf8(b)
        .ok()?
        .strip_prefix("revision ")?
        .split('\n')
        .next()?
        .parse()
        .ok()?;
    (bytes(r) == b).then_some(r)
}
type Snapshot = [Option<Vec<u8>>; 3];
#[derive(Default)]
struct History {
    parents: Vec<Vec<usize>>,
    retired: Vec<usize>,
    // An external writer keeps its full revision while its client downloads.
    external: Vec<usize>,
    partial: Option<usize>,
}
impl History {
    fn edit(&mut self, parent: Option<usize>) -> usize {
        let r = self.parents.len();
        self.parents.push(parent.into_iter().collect());
        r
    }
    fn ancestor(&self, x: usize, y: usize) -> bool {
        let mut todo = vec![y];
        let mut visited = vec![false; self.parents.len()];
        while let Some(r) = todo.pop() {
            if r == x {
                return true;
            }
            if !visited[r] {
                visited[r] = true;
                todo.extend(&self.parents[r]);
            }
        }
        false
    }
}
struct World {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    away: PathBuf,
    local: [PathBuf; 2],
    syncers: [Mutex<Syncer>; 2],
    history: Mutex<History>,
    now: AtomicU64,
}
impl World {
    fn new() -> Arc<Self> {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cloud");
        fs::create_dir(&root).unwrap();
        let local = [
            tmp.path().join("a/rules.json"),
            tmp.path().join("b/rules.json"),
        ];
        let syncers = std::array::from_fn(|m| {
            fs::create_dir(local[m].parent().unwrap()).unwrap();
            fs::write(&local[m], bytes(0)).unwrap();
            let mut s = Syncer::new(local[m].parent().unwrap(), ["A", "B"][m], "model");
            s.set_items(vec![Item {
                name: NAME.into(),
                local: local[m].clone(),
                runs_after: None,
                check: Arc::new(|b| {
                    if revision(b).is_some() {
                        Content::Valid
                    } else {
                        Content::Invalid("incomplete revision".into())
                    }
                }),
            }]);
            s.set_targets(vec![Target {
                kind: Kind::Folder,
                label: "Cloud".into(),
                root: root.clone(),
            }]);
            Mutex::new(s)
        });
        let away = tmp.path().join("away");
        let w = Arc::new(Self {
            _tmp: tmp,
            root,
            away,
            local,
            syncers,
            history: Mutex::new(History {
                parents: vec![vec![]],
                ..History::default()
            }),
            now: AtomicU64::new(1),
        });
        w.sync(0, None);
        w.sync(1, None);
        w
    }
    fn tick(&self) -> u64 {
        self.now.fetch_add(1, Ordering::SeqCst)
    }
    fn cloud(&self) -> PathBuf {
        Folder::new(&self.root).file(NAME)
    }
    fn snapshot(&self) -> Snapshot {
        [
            fs::read(&self.local[0]).ok(),
            fs::read(&self.local[1]).ok(),
            fs::read(self.cloud())
                .ok()
                .or_else(|| fs::read(Folder::new(&self.away).file(NAME)).ok()),
        ]
    }
    fn edit(&self, m: usize) {
        let parent = fs::read(&self.local[m]).ok().and_then(|b| revision(&b));
        let mut h = self.history.lock().unwrap();
        if let Some(p) = parent {
            h.retired.push(p);
        }
        let r = h.edit(parent);
        fs::write(&self.local[m], bytes(r)).unwrap();
    }
    fn partial(&self) {
        if !self.root.exists() {
            return;
        }
        let mut h = self.history.lock().unwrap();
        if h.partial.is_some() {
            return;
        }
        // The external writer edits the last published revision. A client
        // deleting its downloaded copy does not erase the writer's ancestry.
        // An unrelated third-machine join would need its own sync/conflict
        // protocol, which this download command does not model.
        let parent = fs::read(self.cloud())
            .ok()
            .and_then(|b| revision(&b))
            .or_else(|| {
                let manifest = Folder::new(&self.root).manifest()?;
                let sha = &manifest.files.get(NAME)?.sha256;
                (0..h.parents.len()).find(|&r| hash(&bytes(r)) == *sha)
            });
        let r = h.edit(parent);
        h.partial = Some(r);
        h.external.push(r);
        fs::create_dir_all(Folder::new(&self.root).dir()).unwrap();
        let full = bytes(r);
        fs::write(self.cloud(), &full[..full.len() / 2]).unwrap();
    }
    fn complete(&self) {
        if !self.root.exists() {
            return;
        }
        if let Some(r) = self.history.lock().unwrap().partial.take() {
            let full = bytes(r);
            if fs::read(self.cloud()).ok().as_deref() == Some(&full[..full.len() / 2]) {
                Folder::new(&self.root)
                    .write(
                        NAME,
                        &full,
                        &Machine {
                            id: "external".into(),
                            name: "External".into(),
                        },
                        self.tick(),
                        "model",
                        None,
                    )
                    .unwrap();
            }
        }
    }
    // P2: only an explicit resolution may introduce a backwards/sideways edge.
    fn check_sync(&self, before: &Snapshot, after: &Snapshot) {
        let h = self.history.lock().unwrap();
        for i in 0..3 {
            if before[i] == after[i] {
                continue;
            }
            assert!(after[i].is_some(), "P2: sync deleted file {i}");
            let y = after[i].as_deref().and_then(revision);
            if i < 2 {
                assert!(y.is_some(), "P2: applied a half-written file");
            }
            if let (Some(x), Some(y)) = (before[i].as_deref().and_then(revision), y) {
                assert!(
                    h.ancestor(x, y),
                    "P2: silently replaced revision {x} with {y} in file {i}"
                );
            }
        }
    }
    fn check_kept(&self) {
        let mut kept: Vec<_> = self
            .snapshot()
            .iter()
            .flatten()
            .filter_map(|b| revision(b))
            .collect();
        for p in &self.local {
            let dir = p.parent().unwrap().join("sync/backups");
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(r) = fs::read(entry.path()).ok().and_then(|b| revision(&b)) {
                        kept.push(r);
                    }
                }
            }
        }
        let h = self.history.lock().unwrap();
        // At most 15 generated commands, each making at most one revision:
        // rotation (20 distinct backups per machine) is outside this model.
        assert!(h.parents.len() < crate::KEEP_BACKUPS);
        for r in 0..h.parents.len() {
            assert!(
                kept.contains(&r) || h.retired.contains(&r) || h.external.contains(&r),
                "P1: lost revision {r}"
            );
        }
    }
    fn sync(self: &Arc<Self>, m: usize, injection: Option<(u8, Interference)>) {
        let segment = Arc::new(Mutex::new(self.snapshot()));
        let mut s = self.syncers[m].lock().unwrap();
        if let Some((point, what)) = injection {
            let w = Arc::clone(self);
            let segment = Arc::clone(&segment);
            let fired = AtomicBool::new(false);
            s.io_hook = Some(Arc::new(move |p, _| {
                let expected = [
                    IoPoint::RemoteRead,
                    IoPoint::RemoteManifest,
                    IoPoint::RemoteWrite,
                    IoPoint::LocalBackup,
                ][point as usize];
                if p == expected && !fired.swap(true, Ordering::SeqCst) {
                    w.check_sync(&segment.lock().unwrap(), &w.snapshot());
                    match what {
                        Interference::EditLocal => w.edit(m),
                        Interference::EditOtherAndSync => {
                            w.edit(1 - m);
                            w.sync(1 - m, None);
                        }
                        Interference::Delete => {
                            let _ = fs::remove_file(w.cloud());
                        }
                        Interference::Partial => w.partial(),
                    }
                    *segment.lock().unwrap() = w.snapshot();
                }
            }));
        }
        s.pass(self.tick());
        s.io_hook = None;
        drop(s);
        self.check_sync(&segment.lock().unwrap(), &self.snapshot());
    }
    fn resolve(&self, m: usize, keep: bool) {
        let mut s = self.syncers[m].lock().unwrap();
        let Health::Conflict(c) = s.status().targets[0].files[0].health.clone() else {
            return;
        };
        let before = self.snapshot();
        if s.resolve(
            &self.root,
            NAME,
            &c.remote_sha,
            if keep {
                Choice::KeepLocal
            } else {
                Choice::UseFolder
            },
            self.tick(),
        )
        .is_ok()
            && let (Some(x), Some(y)) =
                (before[m].as_deref().and_then(revision), revision(&c.remote))
        {
            let mut h = self.history.lock().unwrap();
            if keep {
                h.parents[x].push(y);
            } else {
                h.parents[y].push(x);
            }
        }
        drop(s);
        self.check_sync(&before, &self.snapshot());
    }
    fn run(self: &Arc<Self>, c: Command) {
        match c {
            Command::Edit(m) => self.edit(m),
            Command::Sync(m) => self.sync(m, None),
            Command::Resolve(m, keep) => self.resolve(m, keep),
            Command::Inject(m, point, what) => self.sync(m, Some((point, what))),
            Command::CloudDelete => {
                let _ = fs::remove_file(self.cloud());
            }
            Command::CloudPartialWrite => self.partial(),
            Command::CloudComplete => self.complete(),
            Command::RootAway if self.root.exists() => fs::rename(&self.root, &self.away).unwrap(),
            Command::RootBack if self.away.exists() => fs::rename(&self.away, &self.root).unwrap(),
            _ => {}
        }
        self.check_kept();
    }
    fn converge(self: &Arc<Self>) {
        if self.away.exists() {
            fs::rename(&self.away, &self.root).unwrap();
        }
        self.complete();
        // First let a safe edit based on the last manifest restore the copy.
        self.sync(0, None);
        self.sync(1, None);
        // A deliberately deleted cloud copy requires user recovery. Restore
        // the manifest's revision from a live writer, rather than blessing
        // an arbitrary stale re-seed in the oracle.
        if !self.cloud().exists() {
            let manifest = Folder::new(&self.root).manifest().unwrap();
            let sha = &manifest.files[NAME].sha256;
            let mut recoverable: Vec<_> = self.snapshot().into_iter().flatten().collect();
            for p in &self.local {
                if let Ok(entries) = fs::read_dir(p.parent().unwrap().join("sync/backups")) {
                    recoverable.extend(entries.flatten().filter_map(|e| fs::read(e.path()).ok()));
                }
            }
            recoverable.extend(
                self.history
                    .lock()
                    .unwrap()
                    .external
                    .iter()
                    .copied()
                    .map(bytes),
            );
            let recovered = recoverable
                .into_iter()
                .find(|b| hash(b) == *sha)
                .expect("the last cloud revision must be recoverable for user restoration");
            fs::write(self.cloud(), recovered).unwrap();
        }
        for _ in 0..6 {
            for m in 0..2 {
                self.sync(m, None);
                self.resolve(m, true);
                self.check_kept();
            }
        }
        let snap = self.snapshot();
        assert_eq!(snap[0], snap[1], "P3: machines did not converge");
        assert_eq!(snap[1], snap[2], "P3: cloud did not converge");
        for s in &self.syncers {
            assert!(
                s.lock().unwrap().status().targets[0].files[0]
                    .health
                    .in_step()
            );
        }
    }
}
#[derive(Clone, Copy, Debug)]
enum Interference {
    EditLocal,
    EditOtherAndSync,
    Delete,
    Partial,
}
#[derive(Clone, Copy, Debug)]
enum Command {
    Edit(usize),
    Sync(usize),
    Resolve(usize, bool),
    Inject(usize, u8, Interference),
    CloudDelete,
    CloudPartialWrite,
    CloudComplete,
    RootAway,
    RootBack,
}
fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        4 => (0..2usize).prop_map(Command::Edit),
        6 => (0..2usize).prop_map(Command::Sync),
        2 => (0..2usize, any::<bool>()).prop_map(|(m,k)| Command::Resolve(m,k)),
        3 => (0..2usize, 0..4u8, prop_oneof![Just(Interference::EditLocal),Just(Interference::EditOtherAndSync),Just(Interference::Delete),Just(Interference::Partial)]).prop_map(|(m,p,w)| Command::Inject(m,p,w)),
        2 => Just(Command::CloudDelete),
        1 => Just(Command::CloudPartialWrite), 1 => Just(Command::CloudComplete),
        1 => Just(Command::RootAway), 1 => Just(Command::RootBack),
    ]
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]
    #[test]
    fn sync_respects_revision_history(commands in prop::collection::vec(command(), 1..16)) {
        let w = World::new();
        for c in commands { w.run(c); }
        w.converge();
    }
}
#[test]
fn deleted_cloud_copy_cannot_make_a_stale_machine_revert_a_newer_revision() {
    let w = World::new();
    for c in [
        Command::Edit(0),
        Command::Sync(0),
        Command::CloudDelete,
        Command::Sync(1),
        Command::Sync(0),
    ] {
        w.run(c);
    }
    assert_eq!(revision(&fs::read(&w.local[0]).unwrap()), Some(1));
}

#[test]
fn a_deletion_after_an_interleaved_push_cannot_revert_the_other_machine() {
    let w = World::new();
    for c in [
        Command::Inject(0, 0, Interference::EditOtherAndSync),
        Command::CloudDelete,
        Command::Sync(0),
    ] {
        w.run(c);
    }
    w.converge();
}

#[test]
fn an_external_download_keeps_its_parent_after_the_cloud_copy_disappears() {
    let w = World::new();
    for c in [
        Command::Resolve(0, false),
        Command::Inject(0, 0, Interference::Delete),
        Command::CloudPartialWrite,
    ] {
        w.run(c);
    }
    w.converge();
}
