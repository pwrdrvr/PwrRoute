//! Real file/store commands, checked against byte revisions and stable rule
//! identities. Identity lives in the pattern, which DraftEdit never changes.
use super::*;
use proptest::prelude::*;
use std::{
    cell::{Cell, RefCell},
    fs,
    rc::Rc,
};

thread_local! { static WRITE_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None); }
pub(super) fn before_write() {
    let hook = WRITE_HOOK.with(|h| h.borrow_mut().take());
    if let Some(h) = hook {
        h();
    }
}
fn rule(id: usize) -> engine::Rule {
    engine::Rule {
        patterns: vec![format!("r{id}.example/*")],
        browser: "Safari".into(),
        enabled: true,
        ..engine::Rule::default()
    }
}
fn identities(bytes: &[u8]) -> Vec<String> {
    serde_json::from_slice::<RulesFile>(bytes)
        .unwrap()
        .rules
        .iter()
        .map(|r| r.patterns[0].clone())
        .collect()
}
#[derive(Clone, Copy, Debug)]
enum External {
    Replace(usize),
    Insert(usize),
    Delete(usize),
}
fn external(path: &Path, change: External, id: usize) {
    let mut file = RulesFile::load(path).unwrap();
    let n = file.rules.len();
    match change {
        External::Replace(i) if n > 0 => file.rules[i % n] = rule(id),
        External::Insert(i) => file.rules.insert(i % (n + 1), rule(id)),
        External::Delete(i) if n > 0 => {
            file.rules.remove(i % n);
        }
        _ => {}
    }
    file.save(path).unwrap();
}
#[derive(Clone, Copy, Debug)]
enum Command {
    Hold(usize),
    Toggle(usize),
    DraftEdit,
    External(External),
    CheckDisk,
    Save,
    SaveDraft,
    Discard,
    Overwrite,
    SaveWithSync(External),
}
struct World {
    _tmp: tempfile::TempDir,
    path: PathBuf,
    store: RulesStore,
    known: Vec<u8>,
    draft_identity: Option<String>,
    next: usize,
}
impl World {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rules.json");
        RulesFile {
            rules: (0..4).map(rule).collect(),
            ..RulesFile::default()
        }
        .save(&path)
        .unwrap();
        let store = RulesStore::load(Some(path.clone()));
        let known = fs::read(&path).unwrap();
        Self {
            _tmp: tmp,
            path,
            store,
            known,
            draft_identity: None,
            next: 100,
        }
    }
    fn check(&mut self) {
        if self.store.check_disk() == Disk::Reloaded {
            self.known = fs::read(&self.path).unwrap();
        }
    }
    fn write(&mut self, draft: bool, force: bool, interference: Option<External>) {
        self.check();
        let replaced = Rc::new(RefCell::new(None));
        let deferred = Rc::new(Cell::new(false));
        let r = Rc::clone(&replaced);
        let d = Rc::clone(&deferred);
        let path = self.path.clone();
        self.next += 1;
        let id = self.next;
        WRITE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                if let Some(change) = interference {
                    match pwrroute_sync::LocalFile::try_lock(&path) {
                        Ok(held) => external(held.path(), change, id),
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => d.set(true),
                        Err(e) => panic!("sync lock: {e}"),
                    }
                }
                *r.borrow_mut() = Some(fs::read(&path).unwrap());
            }))
        });
        let result = if draft {
            self.store.save_draft()
        } else if force {
            self.store.overwrite()
        } else {
            self.store.save()
        };
        WRITE_HOOK.with(|hook| hook.borrow_mut().take());
        if let Some(before) = replaced.borrow().as_ref() {
            assert!(result.is_ok(), "write hook ran but save failed: {result:?}");
            let after = fs::read(&self.path).unwrap();
            // SP1: no successful write replaces an unread byte revision.
            if !force {
                assert_eq!(*before, self.known, "SP1: replaced unread bytes");
            }
            // No model command deletes a store rule or edits its identity.
            // Explicit whole-file Overwrite is the user's sole exception.
            if !force {
                let kept = identities(&after);
                for id in identities(before) {
                    assert!(kept.contains(&id), "SP2: Save deleted {id}");
                }
            }
            self.known = after;
        }
        if deferred.get() {
            let held = pwrroute_sync::LocalFile::try_lock(&self.path).unwrap();
            external(held.path(), interference.unwrap(), id);
        }
    }
    fn run(&mut self, c: Command) {
        match c {
            Command::Hold(i) if !self.store.draft_changed() && self.store.handler_count() > 0 => {
                let i = i % self.store.handler_count();
                self.store.hold(Selection::Handler(i));
                self.draft_identity = Some(self.store.edited().rules[i].patterns[0].clone());
            }
            Command::Toggle(i) if self.store.handler_count() > 0 => {
                let s = Selection::Handler(i % self.store.handler_count());
                self.store.set_enabled(s, !self.store.enabled(s).unwrap());
            }
            Command::DraftEdit => {
                if let Some(Selection::Handler(i)) = self.store.draft() {
                    self.next += 1;
                    self.store.set_profile(i, &format!("Draft{}", self.next));
                }
            }
            Command::External(e) => {
                self.next += 1;
                external(&self.path, e, self.next);
            }
            Command::CheckDisk => self.check(),
            Command::Save => self.write(false, false, None),
            Command::SaveDraft => self.write(true, false, None),
            Command::SaveWithSync(e) => self.write(false, false, Some(e)),
            Command::Overwrite => self.write(false, true, None),
            Command::Discard => {
                self.store.discard_draft();
            }
            _ => {}
        }
        // SP3: an unflagged changed draft still has its original identity.
        if self.store.draft_changed()
            && self.store.draft_conflict().is_none()
            && let Some(Selection::Handler(i)) = self.store.draft()
        {
            assert_eq!(
                Some(&self.store.edited().rules[i].patterns[0]),
                self.draft_identity.as_ref(),
                "SP3: draft moved onto another rule"
            );
        }
    }
}
fn change() -> impl Strategy<Value = External> {
    prop_oneof![
        (0..6usize).prop_map(External::Replace),
        (0..6usize).prop_map(External::Insert),
        (0..6usize).prop_map(External::Delete)
    ]
}
fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        3 => (0..6usize).prop_map(Command::Hold), 2 => Just(Command::DraftEdit),
        1 => (0..6usize).prop_map(Command::Toggle), 3 => change().prop_map(Command::External),
        2 => Just(Command::CheckDisk), 2 => Just(Command::SaveDraft), 1 => Just(Command::Save),
        1 => Just(Command::Discard), 1 => Just(Command::Overwrite), 2 => change().prop_map(Command::SaveWithSync),
    ]
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn store_preserves_read_revisions_and_rule_identities(commands in prop::collection::vec(command(), 1..32)) {
        let mut w = World::new();
        for c in commands { w.run(c); }
    }
}
#[test]
fn saving_a_draft_keeps_an_external_in_place_replacement() {
    let mut w = World::new();
    for c in [
        Command::Hold(1),
        Command::DraftEdit,
        Command::External(External::Replace(1)),
        Command::CheckDisk,
        Command::SaveDraft,
    ] {
        w.run(c);
    }
}

#[test]
fn a_held_pending_toggle_cannot_replace_an_external_rule_on_save() {
    let mut w = World::new();
    for c in [
        Command::External(External::Delete(0)),
        Command::Toggle(5),
        Command::SaveWithSync(External::Replace(0)),
        Command::Hold(5),
        Command::Hold(0),
        Command::SaveWithSync(External::Replace(3)),
        Command::SaveDraft,
    ] {
        w.run(c);
    }
}
