//! The producer behind Settings › Privacy: builds the pane's model from
//! `settings.json`, the running router and the journal's files, and
//! answers its requests. A change is saved first, then handed to the
//! router (`set_retention`, `set_journal`), then read back into the open
//! window, so what the pane shows is what the router does.
//!
//! What touches disk, and the words beside it: the settings file (numbers
//! and a level, never a link); the journal, which the router writes at its
//! level and `ClearJournal` deletes; and Export, which writes the links in
//! memory, each at its level, to a file the reader picked in the OS's own
//! dialog. Nothing else here writes a URL anywhere.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

use gpui_kit::{App, SharedString};
use pwrroute_engine::{Privacy as Level, RulesFile, discovery};
use pwrroute_router::{JournalConfig, JournalMode, Options, Retention, RouterHandle};

use super::model::{JournalFiles, Privacy, SettingsRequest};
use crate::activity;
use crate::config_view::display_path;
use crate::settings_store::{self, SettingsStore};

/// The file name Export suggests.
pub const EXPORT_NAME: &str = "pwrroute-activity.json";

/// The pane's model as things stand: the store, the router and the
/// `rules.json` the router routes by.
pub fn model(cx: &App) -> Privacy {
    let store = settings_store::store(cx).map(|store| store.read(cx).clone());
    let router = settings_store::router(cx);
    let rules = discovery::rules_path(&discovery::Environment::current());
    build(
        store.as_ref(),
        router.as_ref(),
        std::env::home_dir().as_deref(),
        rules.as_deref(),
    )
}

/// The model from its parts. `store` is `None` only before `install` ran
/// (the window harness); then the defaults are described. `rules` is the
/// `rules.json` whose levels the confirm sheet names; `None`, or a file
/// that is not there, names no rule.
pub fn build(
    store: Option<&SettingsStore>,
    router: Option<&RouterHandle>,
    home: Option<&Path>,
    rules: Option<&Path>,
) -> Privacy {
    let default_store = SettingsStore::load(None);
    let store = store.unwrap_or(&default_store);
    let activity = &store.settings().activity;
    let retention = store.retention();
    let journal_dir = store.journal_dir();
    let (description, journal_error) = match router {
        Some(router) => (
            router.retention().description,
            router
                .status()
                .ok()
                .and_then(|status| status.journal_error)
                .map(SharedString::from),
        ),
        None => (
            Options {
                retention,
                journal: store.journal(),
                ..Options::default()
            }
            .retention_description(),
            None,
        ),
    };
    let full_description = Options {
        retention,
        journal: journal_dir.clone().map(|dir| JournalConfig {
            dir,
            mode: JournalMode::Full,
            days: activity.journal_days.max(1),
        }),
        ..Options::default()
    }
    .retention_description();
    let journal_path = journal_dir.as_deref().map_or_else(
        || "no data folder".to_owned(),
        |dir| display_path(&dir.join("activity-YYYY-MM-DD.jsonl"), home),
    );
    Privacy {
        keep_count: retention.count,
        keep_hours: retention.hours,
        journal: activity.journal,
        journal_days: activity.journal_days,
        description: description.into(),
        full_description: full_description.into(),
        journal_path: journal_path.into(),
        journal_files: journal_dir.as_deref().and_then(journal_files),
        journal_error,
        rules_keep_less: rules_keep_less(rules),
        can_save: store.dir().is_some(),
    }
}

/// The journal's files under `dir`: their total size, lines and days, and
/// the oldest day. `None` when there are none.
pub fn journal_files(dir: &Path) -> Option<JournalFiles> {
    let mut files = list_journal_files(dir).ok()?;
    if files.is_empty() {
        return None;
    }
    files.sort();
    let mut bytes = 0u64;
    let mut lines = 0u64;
    for (_, path) in &files {
        if let Ok(meta) = fs::metadata(path) {
            bytes += meta.len();
        }
        lines += count_lines(path).unwrap_or(0);
    }
    let days = files.len();
    Some(JournalFiles {
        summary: format!(
            "{} \u{00b7} {} {} \u{00b7} {days} {}",
            size_label(bytes),
            activity::thousands(lines),
            if lines == 1 { "line" } else { "lines" },
            if days == 1 { "day" } else { "days" }
        )
        .into(),
        since: files[0].0.clone().into(),
    })
}

/// `activity-YYYY-MM-DD.jsonl` files under `dir`, each with its day. An
/// absent directory is an empty journal.
fn list_journal_files(dir: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(day) = name
            .strip_prefix("activity-")
            .and_then(|rest| rest.strip_suffix(".jsonl"))
            .filter(|day| day.len() == 10)
        {
            files.push((day.to_owned(), entry.path()));
        }
    }
    Ok(files)
}

fn count_lines(path: &Path) -> io::Result<u64> {
    use std::io::Read as _;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut lines = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(lines);
        }
        lines += buffer[..read].iter().filter(|b| **b == b'\n').count() as u64;
    }
}

/// "312 B", "84 KB", "1.2 MB".
fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// "Handler 9 keeps host only and handler 10 keeps nothing": the rules
/// in the `rules.json` at `path` that keep less than full, for the confirm
/// sheet.
fn rules_keep_less(path: Option<&Path>) -> Option<SharedString> {
    let file = RulesFile::load(path?).ok()?;
    let keeps = |level: Level| match level {
        Level::Full => None,
        Level::Host => Some("host only"),
        Level::None => Some("nothing"),
    };
    let mut parts: Vec<String> = Vec::new();
    for (index, rewrite) in file.rewrites.iter().enumerate() {
        if let Some(what) = keeps(rewrite.effective_privacy()) {
            parts.push(format!("rewrite {} keeps {what}", index + 1));
        }
    }
    for (index, rule) in file.rules.iter().enumerate() {
        if let Some(what) = keeps(rule.effective_privacy()) {
            parts.push(format!("handler {} keeps {what}", index + 1));
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut sentence = parts.join(" and ");
    if let Some(first) = sentence.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    Some(sentence.into())
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    match request {
        SettingsRequest::SetRetention { count, hours } => {
            let retention = Retention {
                count: *count,
                hours: *hours,
            };
            save(cx, |settings| settings.activity.keep = retention);
            if let Some(router) = settings_store::router(cx) {
                let _ = router.set_retention(retention);
            }
            // Activity and the tray follow the swept buffer.
            if let Some(feed) = activity::feed(cx) {
                feed.update(cx, |feed, cx| {
                    feed.reload();
                    cx.notify();
                });
            }
            refresh(cx);
        }
        SettingsRequest::SetJournal(level) => {
            save(cx, |settings| settings.activity.journal = *level);
            apply_journal(cx);
            refresh(cx);
        }
        SettingsRequest::RevealJournal => {
            if let Some(dir) = journal_dir(cx) {
                reveal(&dir, cx);
            }
        }
        SettingsRequest::ClearJournal => {
            clear_journal(cx);
            refresh(cx);
        }
        SettingsRequest::ClearRecent => {
            clear_recent(cx);
            refresh(cx);
        }
        SettingsRequest::ExportActivity => export(cx),
        _ => return false,
    }
    true
}

fn save(cx: &mut App, edit: impl FnOnce(&mut crate::settings_store::Settings)) {
    let Some(store) = settings_store::store(cx) else {
        eprintln!("pwrroute: settings: no store; the change is not saved");
        return;
    };
    let result = store.update(cx, |store, _| store.update(edit));
    if let Err(error) = result {
        eprintln!("pwrroute: cannot save settings: {error:#}");
    }
}

fn journal_dir(cx: &App) -> Option<PathBuf> {
    settings_store::store(cx).and_then(|store| store.read(cx).journal_dir())
}

/// Hands the router the journal as the settings now name it.
fn apply_journal(cx: &mut App) {
    let config = settings_store::store(cx).and_then(|store| store.read(cx).journal());
    if let Some(router) = settings_store::router(cx) {
        let _ = router.set_journal(config);
    }
}

/// Reads the model again into the open window.
fn refresh(cx: &mut App) {
    let privacy = model(cx);
    super::update(cx, move |model| model.privacy = privacy);
}

/// Shows the journal's directory in Finder or the file manager, creating
/// it (owner only) when the journal is on but has not written yet. Opens
/// no browser: the OS's file manager is asked for a directory.
fn reveal(dir: &Path, cx: &App) {
    if !dir.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        if let Err(error) = builder.create(dir) {
            eprintln!("pwrroute: cannot create {}: {error}", dir.display());
            return;
        }
    }
    // The newest file, so the file manager selects it; the directory when
    // there is none yet.
    let newest = list_journal_files(dir)
        .ok()
        .and_then(|mut files| {
            files.sort();
            files.pop().map(|(_, path)| path)
        })
        .unwrap_or_else(|| dir.to_path_buf());
    cx.reveal_path(&newest);
}

/// Closes the journal, deletes its files, reopens it if it is still on,
/// and forgets the links in memory with them.
fn clear_journal(cx: &mut App) {
    let router = settings_store::router(cx);
    // Closed first: the open day's file would otherwise be written to after
    // it is unlinked (and could not be deleted at all on Windows).
    if let Some(router) = &router {
        let _ = router.set_journal(None);
    }
    if let Some(dir) = journal_dir(cx) {
        delete_journal_files(&dir);
    }
    apply_journal(cx);
    clear_recent(cx);
}

/// Deletes every `activity-*.jsonl` under `dir`, reporting what could not
/// be deleted without naming any content.
pub fn delete_journal_files(dir: &Path) {
    match list_journal_files(dir) {
        Ok(files) => {
            for (_, path) in files {
                if let Err(error) = fs::remove_file(&path)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    eprintln!("pwrroute: cannot delete {}: {error}", path.display());
                }
            }
        }
        Err(error) => eprintln!("pwrroute: cannot read {}: {error}", dir.display()),
    }
}

/// Forgets the links in memory: through the feed when the window has one
/// (Activity and the tray follow it), else straight in the router.
fn clear_recent(cx: &mut App) {
    match activity::feed(cx) {
        Some(feed) => feed.update(cx, |feed, cx| {
            feed.clear();
            cx.notify();
        }),
        None => {
            if let Some(router) = settings_store::router(cx) {
                router.clear_recent();
            }
        }
    }
}

/// Asks for a file in the OS's save dialog, then writes the links in
/// memory to it. The dialog is the explicit step: nothing is written
/// anywhere until it names a path. Activity's Export… comes here too.
pub fn export(cx: &mut App) {
    let Some(router) = settings_store::router(cx) else {
        eprintln!("pwrroute: settings: nothing to export without a router");
        return;
    };
    let directory = std::env::home_dir()
        .map(|home| home.join("Downloads"))
        .filter(|downloads| downloads.is_dir())
        .or_else(std::env::home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    let chosen = cx.prompt_for_new_path(&directory, Some(EXPORT_NAME));
    cx.spawn(async move |_| match chosen.await {
        Ok(Ok(Some(path))) => {
            if let Err(error) = write_export(&router, &path) {
                eprintln!(
                    "pwrroute: cannot export Activity to {}: {error}",
                    path.display()
                );
            }
        }
        Ok(Ok(None)) => {}
        Ok(Err(error)) => eprintln!("pwrroute: the save dialog failed: {error:#}"),
        Err(_) => {}
    })
    .detach();
}

/// The export: when it was written, the router's own sentence about what
/// it holds, and the records as the router keeps them (each already
/// reduced to its rule's level), newest first. Owner-readable only.
pub fn write_export(router: &RouterHandle, path: &Path) -> io::Result<()> {
    let records = router.recent();
    let exported_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let export = serde_json::json!({
        "exportedAt": exported_at,
        "kept": router.retention().description,
        "records": records,
    });
    let mut text = serde_json::to_string_pretty(&export).map_err(io::Error::other)?;
    text.push('\n');
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    use std::io::Write as _;
    options.open(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use pwrroute_router::{DryRunLauncher, Link, Router, Source};

    use super::super::model::JournalLevel;
    use super::*;

    #[test]
    fn the_model_describes_the_defaults_without_a_router() {
        let privacy = build(None, None, None, None);
        assert_eq!(privacy.keep_count, 500);
        assert_eq!(privacy.keep_hours, 24);
        assert_eq!(privacy.journal, JournalLevel::Off);
        assert_eq!(
            privacy.description.as_ref(),
            Options::default().retention_description()
        );
        assert_eq!(privacy.journal_files, None);
        assert!(!privacy.can_save);
        assert_eq!(privacy.journal_path.as_ref(), "no data folder");
        assert_eq!(privacy.rules_keep_less, None);
    }

    #[test]
    fn the_sheet_names_the_rules_that_keep_less() {
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join("rules.json");
        // No file, then a file whose rules all keep full: nothing to name.
        assert_eq!(build(None, None, None, Some(&rules)).rules_keep_less, None);
        fs::write(
            &rules,
            r#"{ "defaultBrowser": "Firefox",
                 "rules": [{ "match": "docs.example.com/*", "browser": "Safari" }] }"#,
        )
        .unwrap();
        assert_eq!(build(None, None, None, Some(&rules)).rules_keep_less, None);
        fs::write(
            &rules,
            r#"{ "defaultBrowser": "Firefox",
                 "rewrites": [{ "match": "*.example.com/*", "privacy": "host" }],
                 "rules": [{ "match": "docs.example.com/*", "browser": "Safari" },
                           { "match": "bank.example.com/*", "browser": "Safari", "private": true },
                           { "match": "secret.example.com/*", "browser": "Safari", "privacy": "none" }] }"#,
        )
        .unwrap();
        assert_eq!(
            build(None, None, None, Some(&rules))
                .rules_keep_less
                .as_deref(),
            Some(
                "Rewrite 1 keeps host only and handler 2 keeps host only and handler 3 keeps nothing"
            )
        );
    }

    #[test]
    fn the_model_reads_the_store_the_router_and_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::load(Some(dir.path().to_path_buf()));
        store
            .update(|settings| {
                settings.activity.keep = Retention {
                    count: 50,
                    hours: 2,
                };
                settings.activity.journal = JournalLevel::Hosts;
            })
            .unwrap();
        let router = Router::spawn(
            Options {
                retention: store.retention(),
                journal: store.journal(),
                ..Options::default()
            },
            None,
            DryRunLauncher::new(),
        )
        .unwrap();
        let handle = router.handle();
        let before = build(Some(&store), Some(&handle), Some(dir.path()), None);
        assert_eq!(before.journal_files, None);
        assert!(before.can_save);
        assert_eq!(
            before.journal_path.as_ref(),
            "~/journal/activity-YYYY-MM-DD.jsonl"
        );
        assert_eq!(before.description.as_ref(), handle.retention().description);
        assert!(before.description.contains("scheme and host only"));
        assert!(before.full_description.contains("with full URLs"));

        handle
            .dispatch_and_wait(Link::new(
                "https://a.example/path?secret=1",
                Source::CommandLine,
            ))
            .unwrap();
        let after = build(Some(&store), Some(&handle), Some(dir.path()), None);
        let files = after.journal_files.expect("one file now");
        assert!(
            files.summary.ends_with(" \u{00b7} 1 line \u{00b7} 1 day"),
            "{}",
            files.summary
        );
        assert_eq!(files.since.len(), "2026-10-20".len());
        assert_eq!(after.journal_error, None);

        // Export holds the records at their level and the router's sentence.
        let out = dir.path().join("export.json");
        write_export(&handle, &out).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["records"].as_array().unwrap().len(), 1);
        assert_eq!(json["kept"], handle.retention().description);
        assert!(text.contains("https://a.example/path?secret=1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&out).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        // Deleting the files leaves nothing to summarize.
        delete_journal_files(&dir.path().join("journal"));
        assert_eq!(journal_files(&dir.path().join("journal")), None);
    }

    #[test]
    fn sizes_read_as_the_design_writes_them() {
        assert_eq!(size_label(312), "312 B");
        assert_eq!(size_label(84 * 1024), "84 KB");
        assert_eq!(size_label(1_258_291), "1.2 MB");
    }
}
