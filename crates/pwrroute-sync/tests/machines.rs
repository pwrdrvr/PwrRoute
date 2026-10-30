//! Several machines syncing through shared folders, each a temp directory,
//! on a fake clock. A "cloud" here is a plain directory both machines see,
//! which is what a cloud-drive client gives each machine once it has caught
//! up; the client's delays are played by writing files by hand.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use pwrroute_sync::{
    Choice, ConflictKind, Content, Folder, Health, Item, Kind, MANIFEST, Schedule, Service, Status,
    Syncer, Target, Wait, Why, hash,
};
use serde_json::{Value, json};

const RULES: &str = "rules.json";

/// `rules.json`'s check, as the app's: it must parse, and one with no
/// rules and no default is blank.
fn check(bytes: &[u8]) -> Content {
    match serde_json::from_slice::<Value>(bytes) {
        Err(e) => Content::Invalid(e.to_string()),
        Ok(v)
            if v.get("rules")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
                && v.get("defaultBrowser")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty) =>
        {
            Content::Blank
        }
        Ok(_) => Content::Valid,
    }
}

fn rules(patterns: &[&str]) -> Vec<u8> {
    let rules: Vec<Value> = patterns
        .iter()
        .map(|p| json!({ "match": p, "browser": "Google Chrome", "profile": "Work" }))
        .collect();
    let mut text =
        serde_json::to_string_pretty(&json!({ "defaultBrowser": "Safari", "rules": rules }))
            .unwrap();
    text.push('\n');
    text.into_bytes()
}

struct Mac {
    _data: tempfile::TempDir,
    rules: PathBuf,
    syncer: Syncer,
    schedule: Schedule,
}

impl Mac {
    fn new(name: &str, clouds: &[&Path]) -> Self {
        let data = tempfile::tempdir().unwrap();
        let rules = data.path().join(RULES);
        let mut syncer = Syncer::new(data.path(), name, "0.8.0");
        syncer.set_items(vec![Item {
            name: RULES.into(),
            local: rules.clone(),
            check: Arc::new(check),
        }]);
        syncer.set_targets(clouds.iter().map(|c| target(c)).collect());
        Self {
            _data: data,
            rules,
            syncer,
            schedule: Schedule::new(60_000, 2_000),
        }
    }

    fn save(&self, bytes: &[u8]) {
        fs::write(&self.rules, bytes).unwrap();
    }

    fn local(&self) -> Vec<u8> {
        fs::read(&self.rules).unwrap()
    }

    fn pass(&mut self, now: u64) -> Status {
        self.syncer.pass(now).clone()
    }

    /// The health of rules.json in folder `i` after a pass.
    fn health(&mut self, now: u64, i: usize) -> Health {
        let status = self.pass(now);
        status.targets[i].files[0].health.clone()
    }
}

fn target(root: &Path) -> Target {
    Target {
        kind: Kind::Folder,
        label: root.file_name().unwrap().to_string_lossy().into_owned(),
        root: root.to_path_buf(),
    }
}

fn cloud() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn remote(cloud: &Path) -> Vec<u8> {
    fs::read(Folder::new(cloud).file(RULES)).unwrap()
}

#[test]
fn the_first_machine_fills_an_empty_folder_and_the_next_one_joins() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["work.example.com/*"]));
    assert_eq!(studio.health(1_000, 0), Health::Pushed);
    assert_eq!(remote(cloud.path()), studio.local());
    let manifest = Folder::new(cloud.path()).manifest().unwrap();
    assert_eq!(manifest.saved_by.name, "Studio Mac");
    assert_eq!(manifest.files[RULES].sha256, hash(&studio.local()));
    assert_eq!(studio.health(2_000, 0), Health::InSync);

    // A new machine with nothing of its own takes the folder's rules.
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    assert_eq!(air.health(3_000, 0), Health::Pulled);
    assert_eq!(air.local(), studio.local());
    // So does one whose rules are blank, and the blank file is backed up.
    let mut linux = Mac::new("thinkpad", &[cloud.path()]);
    linux.save(b"{}\n");
    assert_eq!(linux.health(3_000, 0), Health::Pulled);
    assert_eq!(linux.syncer.status().backups.len(), 1);
    assert_eq!(
        fs::read(&linux.syncer.status().backups[0].path).unwrap(),
        b"{}\n"
    );
}

#[test]
fn an_edit_travels_and_the_other_side_backs_up_what_it_replaces() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    air.pass(2_000);
    let before = air.local();

    studio.save(&rules(&["a.example/*", "b.example/*"]));
    assert_eq!(studio.health(3_000, 0), Health::Pushed);
    assert_eq!(air.health(4_000, 0), Health::Pulled);
    assert_eq!(air.local(), studio.local());
    let backup = &air.syncer.status().backups[0];
    assert_eq!(fs::read(&backup.path).unwrap(), before);
    assert_eq!(backup.from, "This machine");
    assert!(backup.why.contains("Studio Mac"), "{}", backup.why);
    assert_eq!(studio.health(5_000, 0), Health::InSync);
}

#[test]
fn both_changed_is_a_conflict_that_writes_nothing_until_resolved() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    air.pass(2_000);

    studio.save(&rules(&["a.example/*", "studio.example/*"]));
    air.save(&rules(&["a.example/*", "air.example/*"]));
    studio.pass(3_000);
    let theirs = studio.local();
    let mine = air.local();
    let Health::Conflict(conflict) = air.health(4_000, 0) else {
        panic!("expected a conflict");
    };
    assert_eq!(conflict.kind, ConflictKind::BothChanged);
    assert_eq!(conflict.local.as_deref(), Some(mine.as_slice()));
    assert_eq!(conflict.remote, theirs);
    assert_eq!(
        conflict.remote_by.as_ref().map(|m| m.name.as_str()),
        Some("Studio Mac")
    );
    // Nothing moved, and the folder's version is already kept.
    assert_eq!(air.local(), mine);
    assert_eq!(remote(cloud.path()), theirs);
    assert!(
        air.syncer
            .status()
            .backups
            .iter()
            .any(|b| b.sha256 == hash(&theirs))
    );
    // Still a conflict on the next pass: it never resolves itself.
    assert!(matches!(air.health(5_000, 0), Health::Conflict(_)));

    // The folder changed again since it was shown: refused.
    let shown = conflict.remote_sha.clone();
    studio.save(&rules(&[
        "a.example/*",
        "studio.example/*",
        "more.example/*",
    ]));
    studio.pass(6_000);
    let refused = air
        .syncer
        .resolve(cloud.path(), RULES, &shown, Choice::UseFolder, 7_000);
    assert!(refused.is_err());
    assert_eq!(air.local(), mine);

    let Health::Conflict(conflict) = air.health(8_000, 0) else {
        panic!("expected a conflict");
    };
    air.syncer
        .resolve(
            cloud.path(),
            RULES,
            &conflict.remote_sha,
            Choice::UseFolder,
            9_000,
        )
        .unwrap();
    assert_eq!(air.local(), studio.local());
    let after = air.pass(9_500);
    assert_eq!(after.targets[0].files[0].health, Health::InSync);
    assert!(
        after.backups.iter().any(|b| b.sha256 == hash(&mine)),
        "mine is kept"
    );
}

#[test]
fn keep_local_overwrites_the_folder_and_keeps_its_version() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    air.pass(2_000);
    studio.save(&rules(&["studio.example/*"]));
    studio.pass(3_000);
    air.save(&rules(&["air.example/*"]));
    let Health::Conflict(conflict) = air.health(4_000, 0) else {
        panic!("expected a conflict");
    };
    air.syncer
        .resolve(
            cloud.path(),
            RULES,
            &conflict.remote_sha,
            Choice::KeepLocal,
            5_000,
        )
        .unwrap();
    assert_eq!(remote(cloud.path()), air.local());
    let kept = air.pass(6_000);
    assert!(kept.backups.iter().any(|b| b.sha256 == conflict.remote_sha));
    // The other machine had not changed since, so it takes it.
    assert_eq!(studio.health(7_000, 0), Health::Pulled);
    assert_eq!(studio.local(), air.local());
}

#[test]
fn joining_with_rules_of_your_own_asks_first() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    air.save(&rules(&["mine.example/*"]));
    let Health::Conflict(conflict) = air.health(2_000, 0) else {
        panic!("expected a conflict");
    };
    assert_eq!(conflict.kind, ConflictKind::Join);
    assert_eq!(air.local(), rules(&["mine.example/*"]));
}

#[test]
fn another_machines_write_is_applied_only_once_it_settles() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    let folder = Folder::new(cloud.path());

    // The client delivered a new rules.json before the manifest that lists
    // it: wait, then apply once it has stayed put for a pass.
    let new = rules(&["a.example/*", "b.example/*"]);
    fs::write(folder.file(RULES), &new).unwrap();
    assert_eq!(studio.health(2_000, 0), Health::Waiting(Wait::Settling));
    assert_ne!(studio.local(), new);
    assert_eq!(studio.health(3_000, 0), Health::Pulled);
    assert_eq!(studio.local(), new);

    // Half a file that settles is never applied.
    fs::write(folder.file(RULES), b"{ \"rules\": [ {\"match\"").unwrap();
    studio.pass(4_000);
    let Health::Error(why) = studio.health(5_000, 0) else {
        panic!("expected an error");
    };
    assert!(why.contains("does not load"), "{why}");
    assert_eq!(studio.local(), new);

    // A write the manifest lists applies on the first look.
    let next = rules(&["c.example/*"]);
    folder
        .write(
            RULES,
            &next,
            &pwrroute_sync::Machine {
                id: "x".into(),
                name: "thinkpad".into(),
            },
            6_000,
            "",
        )
        .unwrap();
    assert_eq!(studio.health(7_000, 0), Health::Pulled);
    assert_eq!(studio.local(), next);
}

#[test]
fn an_unavailable_folder_keeps_its_base_and_others_carry_on() {
    let dropbox = cloud();
    let drive_parent = cloud();
    let drive = drive_parent.path().join("My Drive");
    fs::create_dir_all(&drive).unwrap();
    let mut studio = Mac::new("Studio Mac", &[dropbox.path(), &drive]);
    studio.save(&rules(&["a.example/*"]));
    let status = studio.pass(1_000);
    assert!(
        status
            .targets
            .iter()
            .all(|t| t.files[0].health == Health::Pushed)
    );

    // Google Drive quits: its folder is gone.
    let parked = drive_parent.path().join("parked");
    fs::rename(&drive, &parked).unwrap();
    studio.save(&rules(&["a.example/*", "b.example/*"]));
    let status = studio.pass(2_000);
    assert_eq!(status.targets[0].files[0].health, Health::Pushed);
    assert!(matches!(
        status.targets[1].files[0].health,
        Health::Unavailable(_)
    ));
    assert_eq!(status.targets[1].last_synced, Some(1_000));
    assert!(!drive.exists(), "a missing root is never created");

    // It comes back with the old file: a plain push, not a conflict.
    fs::rename(&parked, &drive).unwrap();
    assert_eq!(studio.health(3_000, 1), Health::Pushed);
    assert_eq!(remote(&drive), studio.local());
}

#[test]
fn a_file_icloud_has_not_downloaded_waits() {
    let cloud = cloud();
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    let folder = Folder::new(cloud.path());
    fs::create_dir_all(folder.dir()).unwrap();
    fs::write(folder.file(".rules.json.icloud"), b"bplist").unwrap();
    assert_eq!(air.health(1_000, 0), Health::Waiting(Wait::NotDownloaded));
    assert!(!air.rules.exists(), "nothing written while waiting");
}

#[test]
fn a_pull_from_one_folder_reaches_the_others() {
    // Studio syncs with Dropbox and Drive; the Linux box only with Drive.
    let dropbox = cloud();
    let drive = cloud();
    let mut studio = Mac::new("Studio Mac", &[dropbox.path(), drive.path()]);
    let mut linux = Mac::new("thinkpad", &[drive.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    linux.pass(2_000);

    linux.save(&rules(&["a.example/*", "linux.example/*"]));
    linux.pass(3_000);
    // Dropbox comes first: in step. Drive: pulled. The next pass pushes the
    // pull on to Dropbox.
    let status = studio.pass(4_000);
    assert_eq!(status.targets[0].files[0].health, Health::InSync);
    assert_eq!(status.targets[1].files[0].health, Health::Pulled);
    let status = studio.pass(5_000);
    assert_eq!(status.targets[0].files[0].health, Health::Pushed);
    assert_eq!(remote(dropbox.path()), linux.local());
    assert_eq!(remote(drive.path()), linux.local());
}

#[test]
fn removing_a_folder_forgets_it_so_adding_it_back_asks() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    studio.syncer.set_targets(Vec::new());
    studio.save(&rules(&["b.example/*"]));
    studio.syncer.set_targets(vec![target(cloud.path())]);
    let Health::Conflict(conflict) = studio.health(2_000, 0) else {
        panic!("expected a conflict");
    };
    assert_eq!(conflict.kind, ConflictKind::Join);
}

#[test]
fn the_schedule_drives_passes_on_a_fake_clock() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    let mut log: Vec<(&str, u64, Why)> = Vec::new();
    let mut now = 0;
    let tick = |mac: &mut Mac, name: &'static str, now: u64, log: &mut Vec<_>| {
        if let Some(why) = mac.schedule.tick(now, mac.syncer.local_stamps()) {
            mac.syncer.pass(now);
            mac.schedule.passed_with(mac.syncer.local_stamps());
            log.push((name, now, why));
        }
    };
    while now <= 4_000 {
        tick(&mut studio, "studio", now, &mut log);
        tick(&mut air, "air", now, &mut log);
        now += 2_000;
    }
    assert_eq!(air.local(), studio.local(), "joined at start");
    // An edit on the studio goes out within two ticks, and reaches the Air
    // on its next interval.
    studio.save(&rules(&["a.example/*", "b.example/*"]));
    while now <= 70_000 {
        tick(&mut studio, "studio", now, &mut log);
        tick(&mut air, "air", now, &mut log);
        now += 2_000;
    }
    assert_eq!(air.local(), studio.local());
    let studio_change = log
        .iter()
        .find(|(n, _, why)| *n == "studio" && *why == Why::LocalChange)
        .unwrap();
    assert_eq!(studio_change.1, 8_000, "{log:?}");
    let air_interval = log
        .iter()
        .find(|(n, _, why)| *n == "air" && *why == Why::Interval)
        .unwrap();
    assert_eq!(air_interval.1, 60_000);
    // The Air's pull did not count as its own edit.
    assert!(
        !log.iter()
            .any(|(n, _, why)| *n == "air" && *why == Why::LocalChange),
        "{log:?}"
    );
}

#[test]
fn the_manifest_is_written_after_the_file_and_staging_files_do_not_linger() {
    let cloud = cloud();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);
    let names: Vec<String> = fs::read_dir(Folder::new(cloud.path()).dir())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let mut names = names;
    names.sort();
    assert_eq!(names, vec![RULES.to_owned(), MANIFEST.to_owned()]);
}

/// The clock a test moves by hand.
#[derive(Clone, Default)]
struct FakeClock(Arc<AtomicU64>);

impl pwrroute_sync::Clock for FakeClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
fn the_service_runs_a_pass_on_its_thread_and_publishes() {
    let cloud = cloud();
    let data = tempfile::tempdir().unwrap();
    let rules_path = data.path().join(RULES);
    fs::write(&rules_path, rules(&["a.example/*"])).unwrap();
    let mut syncer = Syncer::new(data.path(), "Studio Mac", "0.8.0");
    syncer.set_items(vec![Item {
        name: RULES.into(),
        local: rules_path.clone(),
        check: Arc::new(check),
    }]);
    let clock = FakeClock::default();
    clock.0.store(1_000, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel::<Status>();
    let service = Service::spawn(syncer, clock.clone(), move |status| {
        let _ = tx.send(status.clone());
    })
    .unwrap();
    service.set_targets(vec![target(cloud.path())]);
    let status = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(status.targets[0].files[0].health, Health::Pushed);
    assert_eq!(service.status(), status);
    assert_eq!(remote(cloud.path()), fs::read(&rules_path).unwrap());

    // Sync now runs another pass, at the clock's time.
    clock.0.store(2_000, Ordering::SeqCst);
    service.wake();
    let status = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(status.last_pass, Some(2_000));
    assert_eq!(status.targets[0].files[0].health, Health::InSync);

    // Removing the last folder publishes a status with no folders, so the
    // window stops showing the old one.
    service.set_targets(Vec::new());
    let status = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(status.targets.is_empty(), "{status:?}");
}

#[cfg(unix)]
#[test]
fn a_pull_writes_through_a_symlinked_local_file() {
    let cloud = cloud();
    let elsewhere = tempfile::tempdir().unwrap();
    let mut studio = Mac::new("Studio Mac", &[cloud.path()]);
    studio.save(&rules(&["a.example/*"]));
    studio.pass(1_000);

    // The Air's rules.json is a link into another folder it syncs by hand.
    let mut air = Mac::new("MacBook Air", &[cloud.path()]);
    let real = elsewhere.path().join("rules.json");
    fs::write(&real, b"{}\n").unwrap();
    std::os::unix::fs::symlink(&real, &air.rules).unwrap();
    assert_eq!(air.health(2_000, 0), Health::Pulled);
    assert!(
        fs::symlink_metadata(&air.rules)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&real).unwrap(), studio.local());
}
