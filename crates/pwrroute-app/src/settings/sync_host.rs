//! Settings › Cloud Sync's producer: the pane from the sync thread's last report
//! (`crate::sync`), the cloud folders found here, `rules.json` and
//! `machine.json`, and the answers to its requests.
//!
//! [`build`] is plain data over [`Facts`], so tests feed it contrived
//! folders and rules; [`model`] gathers the facts from the running app.
//! Cloud folders are looked for when the pane is built (Settings opening,
//! or Sync now), never at launch.

use std::path::{Path, PathBuf};

use gpui_kit::{App, SharedString};
use pwrroute_engine::{BindingSource, MachineFile, RulesFile};
use pwrroute_sync::{ConflictKind, Health, Place, Status, Target, TargetStatus, Wait};

use super::model::{
    DestinationRow, FolderConfig, RuleSetRow, SettingsRequest, Sync, SyncConflict, SyncFolder,
    SyncPlace, SyncTone,
};
use crate::activity_actions::BrowserChoice;
use crate::destinations::{self, Choice};
use crate::overview_model::Installed;

/// What the pane is built from.
pub struct Facts<'a> {
    pub status: &'a Status,
    /// Folders configured, in order: shown even before the first pass.
    pub targets: &'a [Target],
    pub places: &'a [Place],
    pub rules: &'a RulesFile,
    pub machine: &'a MachineFile,
    pub computer_name: &'a str,
    /// The Finicky config in charge, shown.
    pub config: Option<String>,
    pub config_on: bool,
    pub choices: &'a [BrowserChoice],
    pub installed: &'a [Installed],
    pub home: Option<&'a Path>,
    /// "this Mac" or "this computer".
    pub here: &'a str,
    /// Milliseconds since the epoch in words: "09:41", "Oct 27 09:41".
    pub when: &'a dyn Fn(u64) -> String,
}

/// The pane, from the facts.
pub fn build(facts: &Facts<'_>) -> Sync {
    let shown = |root: &Path| {
        crate::config_view::display_path(&root.join(pwrroute_sync::FOLDER), facts.home)
    };
    let mut folder_config = None;
    let folders: Vec<SyncFolder> = facts
        .targets
        .iter()
        .map(|target| {
            let status = facts
                .status
                .targets
                .iter()
                .find(|s| s.target.root == target.root);
            let mut row = folder(target, status, shown(&target.root), facts);
            if let Some(found) = status.and_then(|status| config_gap(status, facts)) {
                // Only an otherwise quiet folder says so: a conflict or a
                // wait needs the reader first.
                if row.tone == SyncTone::Ok {
                    row.chip = "Needs you".into();
                    row.tone = SyncTone::Warn;
                    row.line = match &found {
                        FolderConfig::Offer { name, .. } => {
                            format!("rules.json in sync \u{00b7} {name} not used here")
                        }
                        FolderConfig::Missing { runs_after, .. } => {
                            format!("rules.json in sync \u{00b7} {runs_after} is not here")
                        }
                    }
                    .into();
                }
                folder_config.get_or_insert(found);
            }
            row
        })
        .collect();
    let offered = facts
        .places
        .iter()
        .filter(|place| !facts.targets.iter().any(|t| t.root == place.root))
        .map(|place| SyncPlace {
            label: place.label.clone().into(),
            shown: shown(&place.root).into(),
            root: place.root.clone(),
        })
        .collect();
    let (summary, summary_tone) = summary(&folders, facts);
    let last_synced = facts
        .status
        .targets
        .iter()
        .filter_map(|t| t.last_synced)
        .max()
        .map_or_else(|| "never".to_owned(), |at| (facts.when)(at));
    Sync {
        machine_name: facts.machine.name.clone().into(),
        computer_name: facts.computer_name.to_owned().into(),
        folders,
        offered,
        config: facts.config.clone().map(Into::into),
        config_on: facts.config_on,
        summary,
        summary_tone,
        last_synced: last_synced.into(),
        backups: facts.status.backups.len(),
        destinations: destination_rows(facts),
        sets: set_rows(facts),
        problem: None,
        folder_config,
    }
}

/// What the folder's `rules.json` ran after on the machine that saved it,
/// when this machine cannot run the same (Settings Sections 2a, 2b): the
/// folder has the config and this machine has none of its own (Offer), or
/// the folder lacks it and another machine saved the rules (Missing). A
/// machine with its own config is offered nothing: each keeps its own.
fn config_gap(status: &TargetStatus, facts: &Facts<'_>) -> Option<FolderConfig> {
    let rules = status.folder_files.get("rules.json")?;
    let runs_after = rules.runs_after.clone()?;
    let by = if rules.saved_by.name.is_empty() {
        "another machine".to_owned()
    } else {
        rules.saved_by.name.clone()
    };
    let config = status
        .folder_files
        .iter()
        .find(|(name, _)| name.starts_with("finicky."));
    match config {
        Some((name, entry)) if facts.config.is_none() => Some(FolderConfig::Offer {
            root: status.target.root.clone(),
            name: name.clone().into(),
            folder: status.target.label.clone().into(),
            by: by.into(),
            at: (facts.when)(entry.saved_at).into(),
            runs_after: runs_after.into(),
            here: format!("~/.{name}").into(),
        }),
        None if rules.saved_by.id != facts.status.machine.id => Some(FolderConfig::Missing {
            by: by.into(),
            runs_after: runs_after.into(),
        }),
        _ => None,
    }
}

/// The Sync to card's chip: "Not set up", "1 conflict", "2 folders".
fn summary(folders: &[SyncFolder], _facts: &Facts<'_>) -> (SharedString, SyncTone) {
    if folders.is_empty() {
        return ("Not set up".into(), SyncTone::Neutral);
    }
    let conflicts = folders.iter().filter(|f| f.conflict.is_some()).count();
    if conflicts > 0 {
        let s = if conflicts == 1 { "" } else { "s" };
        return (format!("{conflicts} conflict{s}").into(), SyncTone::Bad);
    }
    let n = folders.len();
    let s = if n == 1 { "" } else { "s" };
    let tone = if folders.iter().all(|f| f.tone == SyncTone::Ok) {
        SyncTone::Ok
    } else {
        SyncTone::Neutral
    };
    (format!("{n} folder{s}").into(), tone)
}

fn folder(
    target: &Target,
    status: Option<&TargetStatus>,
    shown: String,
    facts: &Facts<'_>,
) -> SyncFolder {
    let mut folder = SyncFolder {
        label: target.label.clone().into(),
        shown: shown.into(),
        root: target.root.clone(),
        chip: "Syncing\u{2026}".into(),
        tone: SyncTone::Neutral,
        line: "Checking this folder".into(),
        conflict: None,
    };
    let Some(status) = status else {
        return folder;
    };
    let Some(worst) = status.worst() else {
        return folder;
    };
    let saved_by = status
        .saved_by
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty());
    let time = |at: Option<u64>| at.map(|at| (facts.when)(at));
    let (chip, tone, line): (&str, SyncTone, String) = match &worst.health {
        Health::Absent => ("In sync", SyncTone::Ok, "Nothing to sync yet".into()),
        Health::Pulled => {
            let from = saved_by.clone().unwrap_or_else(|| target.label.clone());
            let mut line = format!("Updated from {from}");
            if let Some(at) = time(status.last_synced) {
                line.push_str(&format!(" \u{00b7} {at}"));
            }
            line.push_str(&format!(
                " \u{00b7} {}\u{2019}s saved to backups",
                facts.here
            ));
            ("In sync", SyncTone::Ok, line)
        }
        Health::InSync | Health::Pushed => {
            let mut parts: Vec<String> = time(status.last_synced).into_iter().collect();
            if let Some(by) = &saved_by {
                parts.push(format!("saved by {by}"));
            }
            ("In sync", SyncTone::Ok, parts.join(" \u{00b7} "))
        }
        Health::Waiting(Wait::NotDownloaded) => (
            "Waiting",
            SyncTone::Neutral,
            format!(
                "Waiting for {} to download {}",
                short_kind(target),
                worst.name
            ),
        ),
        Health::Waiting(Wait::Settling) => (
            "Waiting",
            SyncTone::Neutral,
            format!(
                "Changed on {} \u{00b7} waiting for it to finish syncing",
                saved_by.clone().unwrap_or_else(|| "another machine".into())
            ),
        ),
        Health::Unavailable(why) if why.starts_with(pwrroute_sync::folder::NO_ACCESS) => (
            "No access",
            SyncTone::Warn,
            format!(
                "PwrRoute may not use files in {}. Turn it on in System Settings \u{203a} \
                 Privacy & Security \u{203a} Files & Folders, then Sync now.",
                target.label
            ),
        ),
        Health::Unavailable(_) => (
            "Unavailable",
            SyncTone::Warn,
            match target.kind {
                pwrroute_sync::Kind::Folder => {
                    "Folder not found \u{00b7} tries again every minute".to_owned()
                }
                kind => format!(
                    "Folder not found (is {} running?) \u{00b7} tries again every minute",
                    kind.name()
                ),
            },
        ),
        Health::Error(why) => ("Can\u{2019}t read", SyncTone::Warn, why.clone()),
        Health::Conflict(conflict) => {
            let view = conflict_view(&worst.name, conflict, target, facts);
            let join = view.join;
            folder.conflict = Some(view);
            if join {
                (
                    "Joining",
                    SyncTone::Warn,
                    "Has rules already \u{00b7} choose which to keep".into(),
                )
            } else {
                (
                    "Conflict",
                    SyncTone::Bad,
                    "Both changed since the last sync".into(),
                )
            }
        }
    };
    folder.chip = chip.into();
    folder.tone = tone;
    folder.line = line.into();
    folder
}

/// The client's short name in a sentence: "iCloud", "Google Drive".
fn short_kind(target: &Target) -> &'static str {
    match target.kind {
        pwrroute_sync::Kind::ICloud => "iCloud",
        pwrroute_sync::Kind::Folder => "the sync client",
        kind => kind.name(),
    }
}

fn conflict_view(
    name: &str,
    conflict: &pwrroute_sync::Conflict,
    target: &Target,
    facts: &Facts<'_>,
) -> SyncConflict {
    let theirs_name = conflict
        .remote_by
        .as_ref()
        .map(|m| m.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| target.label.clone());
    let parse = |bytes: &[u8]| serde_json::from_slice::<RulesFile>(bytes).ok();
    let is_rules = name == "rules.json";
    let mine_file = conflict.local.as_deref().and_then(parse);
    let theirs_file = parse(&conflict.remote);
    let count = |file: &Option<RulesFile>| match file {
        Some(file) if is_rules => format!("{} rules", file.rules.len()),
        _ => name.to_owned(),
    };
    let mut theirs = count(&theirs_file);
    if let Some(at) = conflict.remote_at {
        theirs.push_str(&format!(" \u{00b7} saved {}", (facts.when)(at)));
    }
    let differences = match (&mine_file, &theirs_file) {
        (Some(mine), Some(theirs)) if is_rules => differences(mine, theirs),
        _ => vec!["The two versions differ.".into()],
    };
    let headline = match conflict.kind {
        ConflictKind::BothChanged => format!(
            "Links keep routing with {}\u{2019}s rules until you choose. Nothing is written to \
             {} meanwhile.",
            facts.here, target.label
        ),
        ConflictKind::Join => format!(
            "This folder already has rules from {theirs_name} ({}).",
            theirs
        ),
    };
    SyncConflict {
        file: name.to_owned().into(),
        headline: headline.into(),
        mine: format!("{} \u{00b7} {}", count(&mine_file), facts.here).into(),
        theirs: theirs.into(),
        theirs_name: theirs_name.into(),
        differences,
        remote_sha: conflict.remote_sha.clone().into(),
        join: conflict.kind == ConflictKind::Join,
    }
}

/// What the folder's rules have that these do not ("+") and the reverse
/// ("−"), a rule a line, at most [`MAX_DIFFERENCES`].
pub fn differences(mine: &RulesFile, theirs: &RulesFile) -> Vec<SharedString> {
    let describe = |rule: &pwrroute_engine::Rule| {
        let target = rule.destination.clone().unwrap_or_else(|| {
            if rule.profile.is_empty() {
                rule.browser.clone()
            } else {
                format!("{} \u{00b7} {}", rule.browser, rule.profile)
            }
        });
        format!("{} \u{2192} {target}", rule.patterns.join(", "))
    };
    let mut lines: Vec<String> = Vec::new();
    for (i, rule) in theirs.rules.iter().enumerate() {
        if !mine.rules.contains(rule) {
            lines.push(format!("+ rule {} {}", i + 1, describe(rule)));
        }
    }
    for (i, rule) in mine.rules.iter().enumerate() {
        if !theirs.rules.contains(rule) {
            lines.push(format!("\u{2212} rule {} {}", i + 1, describe(rule)));
        }
    }
    if mine.default_browser != theirs.default_browser
        || mine.default_profile != theirs.default_profile
    {
        lines.push(format!(
            "Everything else \u{2192} {}",
            if theirs.default_profile.is_empty() {
                theirs.default_browser.clone()
            } else {
                format!(
                    "{} \u{00b7} {}",
                    theirs.default_browser, theirs.default_profile
                )
            }
        ));
    }
    if lines.is_empty() && mine != theirs {
        lines.push("Rewrites, tests or settings differ.".into());
    }
    let more = lines.len().saturating_sub(MAX_DIFFERENCES);
    lines.truncate(MAX_DIFFERENCES);
    if more > 0 {
        lines.push(format!("and {more} more"));
    }
    lines.into_iter().map(Into::into).collect()
}

pub const MAX_DIFFERENCES: usize = 5;

fn destination_rows(facts: &Facts<'_>) -> Vec<DestinationRow> {
    facts
        .rules
        .destinations
        .iter()
        .map(|destination| {
            let (binding, source) = facts
                .rules
                .binding_for(&destination.name, facts.machine)
                .map_or(
                    (destination.binding.clone(), BindingSource::Synced),
                    |(b, s)| (b.clone(), s),
                );
            let name = crate::default_browser::browser_name(&binding.browser);
            let here = if binding.profile.is_empty() {
                name.clone()
            } else {
                format!("{name} \u{00b7} {}", binding.profile)
            };
            let app = crate::rule_pickers::installed_named(&binding.browser, facts.installed);
            let warning = match app {
                None if !facts.installed.is_empty() => Some(format!(
                    "{name} isn\u{2019}t installed on {}: choose what {} means here.",
                    facts.here, destination.name
                )),
                Some(app)
                    if !binding.profile.is_empty()
                        && !app.profiles.is_empty()
                        && !app.profiles.contains(&binding.profile)
                        && !app.directories.contains(&binding.profile) =>
                {
                    Some(format!(
                        "{name} has no profile named {} on {}.",
                        binding.profile, facts.here
                    ))
                }
                _ => None,
            };
            let current = Choice {
                browser: binding.browser.clone(),
                profile: binding.profile.clone(),
                private: binding.private,
            };
            DestinationRow {
                name: destination.name.clone().into(),
                here: here.into(),
                this_machine: source == BindingSource::Machine,
                uses: facts.rules.destination_uses(&destination.name),
                warning: warning.map(Into::into),
                lanes: destinations::lanes(
                    facts.choices,
                    facts.installed,
                    &current,
                    None,
                    true,
                    false,
                ),
            }
        })
        .collect()
}

fn set_rows(facts: &Facts<'_>) -> Vec<RuleSetRow> {
    facts
        .rules
        .set_names()
        .into_iter()
        .map(|name| {
            let entries = facts
                .rules
                .rules
                .iter()
                .filter(|r| r.set.as_deref() == Some(name.as_str()))
                .count()
                + facts
                    .rules
                    .rewrites
                    .iter()
                    .filter(|r| r.set.as_deref() == Some(name.as_str()))
                    .count();
            RuleSetRow {
                on: facts.rules.set_enabled_on(&name, facts.machine),
                synced_on: facts.rules.set_enabled(&name),
                overridden: facts.machine.sets.contains_key(&name),
                entries,
                name: name.into(),
            }
        })
        .collect()
}

/// "this Mac" on macOS, "this PC" on Windows, "this computer" on Linux.
pub fn here() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else if cfg!(windows) {
        "this PC"
    } else {
        "this computer"
    }
}

fn when(ms: u64) -> String {
    use chrono::{Local, TimeZone as _};
    let Some(at) = Local.timestamp_millis_opt(ms as i64).single() else {
        return String::new();
    };
    if at.date_naive() == Local::now().date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%b %-d %H:%M").to_string()
    }
}

/// The pane as the running app stands, looking for cloud folders again:
/// for the window opening.
pub fn model(cx: &mut App) -> Sync {
    if let Some(feed) = crate::sync::feed(cx) {
        feed.update(cx, |feed, _| feed.look_for_places());
    }
    current(cx)
}

/// The pane with the cloud folders found when it opened: a pass or a
/// change redraws it without listing `~/Library/CloudStorage` again.
fn current(cx: &mut App) -> Sync {
    let Some(feed) = crate::sync::feed(cx) else {
        return Sync::default();
    };
    let (status, places) = {
        let feed = feed.read(cx);
        (feed.status.clone(), feed.places.clone())
    };
    let settings = crate::settings_store::store(cx)
        .map(|store| store.read(cx).settings().sync.clone())
        .unwrap_or_default();
    let rules = crate::sync::rules_path()
        .and_then(|p| RulesFile::load(&p).ok())
        .unwrap_or_default();
    let machine = crate::sync::machine();
    let home = std::env::home_dir();
    let config = crate::router::config_in_charge()
        .map(|p| crate::config_view::display_path(&p, home.as_deref()));
    let choices = crate::browser_scan::choices(cx);
    let installed = crate::browser_scan::installed(cx);
    build(&Facts {
        status: &status,
        targets: &settings.targets,
        places: &places,
        rules: &rules,
        machine: &machine,
        computer_name: &crate::sync::computer_name(),
        config,
        config_on: settings.syncs_config(),
        choices: &choices,
        installed: &installed,
        home: home.as_deref(),
        here: here(),
        when: &when,
    })
}

/// A config bigger than this is not one to copy (sync's own limit).
const MAX_CONFIG_BYTES: usize = 8 * 1024 * 1024;

/// Use it on this Mac (Settings Sections 2a): the folder's settled config
/// written to `~/.{name}`, never over a config already here, then put in
/// charge with Also sync on so it keeps up.
fn use_folder_config(root: &Path, name: &str, cx: &mut App) -> anyhow::Result<()> {
    use anyhow::Context as _;
    use pwrroute_engine::discovery;
    if crate::sync::config_name(Path::new(name)).as_deref() != Some(name) {
        anyhow::bail!("{name} is not a Finicky config");
    }
    let env = discovery::Environment::current();
    if let Some(found) = discovery::config_candidates(&env)
        .into_iter()
        .find(|p| p.is_file())
    {
        let home = std::env::home_dir();
        anyhow::bail!(
            "{} is already here",
            crate::config_view::display_path(&found, home.as_deref())
        );
    }
    let bytes = pwrroute_sync::Folder::new(root).settled(name)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        anyhow::bail!("{name} is too big to be a config");
    }
    if let pwrroute_sync::Content::Invalid(why) = crate::sync::check_config(&bytes) {
        anyhow::bail!("{name}: {why}");
    }
    let home = std::env::home_dir().context("no home folder")?;
    let path = home.join(format!(".{name}"));
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("cannot write ~/.{name}"))?;
        file.write_all(&bytes)
            .with_context(|| format!("cannot write ~/.{name}"))?;
    }
    crate::sync::edit_settings(|sync| sync.config = Some(true), cx)?;
    // Discovery finds it first now; the Rules screen's "Found …" notice
    // would only ask what the reader just answered.
    let mut source = crate::settings_store::store(cx)
        .map(|store| store.read(cx).settings().source.clone())
        .unwrap_or_default();
    source.visual_only = false;
    source.config = None;
    if !source.dismissed.contains(&path) {
        source.dismissed.push(path);
    }
    crate::importer_view::set_source_and_show(source, cx);
    super::configuration_host::refresh(cx);
    Ok(())
}

/// Redraws the pane after a pass or a change, when Settings is open.
pub fn refresh(cx: &mut App) {
    if !super::is_open(cx) {
        return;
    }
    let sync = current(cx);
    super::update(cx, |model| model.sync = sync);
}

fn report(problem: String, cx: &mut App) {
    if !super::is_open(cx) {
        return;
    }
    let mut sync = current(cx);
    sync.problem = Some(problem.into());
    super::update(cx, |model| model.sync = sync);
}

/// Answers Sync's requests; `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    let outcome = match request {
        SettingsRequest::SyncNow => {
            crate::sync::wake(cx);
            Ok(())
        }
        SettingsRequest::AddSyncFolder(root) => add(root.clone(), cx),
        SettingsRequest::ChooseSyncFolder => {
            choose(cx);
            Ok(())
        }
        SettingsRequest::RemoveSyncFolder(root) => {
            crate::sync::edit_settings(|sync| sync.targets.retain(|t| &t.root != root), cx)
        }
        SettingsRequest::RevealSyncFolder(root) => {
            let dir = root.join(pwrroute_sync::FOLDER);
            cx.reveal_path(if dir.exists() { &dir } else { root });
            Ok(())
        }
        SettingsRequest::ResolveSync {
            root,
            file,
            remote_sha,
            keep_local,
        } => {
            resolve(
                root.clone(),
                file.to_string(),
                remote_sha.to_string(),
                *keep_local,
                cx,
            );
            Ok(())
        }
        SettingsRequest::SetMachineName(name) => {
            let name = name.trim().to_owned();
            crate::sync::edit_machine(|m| m.name = name, cx)
        }
        SettingsRequest::SetSyncConfig(on) => {
            let on = *on;
            crate::sync::edit_settings(|sync| sync.config = Some(on), cx)
        }
        SettingsRequest::UseFolderConfig { root, name } => use_folder_config(root, name, cx),
        SettingsRequest::RevealSyncBackups => {
            if let Some(newest) = crate::sync::feed(cx)
                .and_then(|feed| feed.read(cx).status.backups.first().map(|b| b.path.clone()))
            {
                cx.reveal_path(&newest);
            }
            Ok(())
        }
        SettingsRequest::BindDestination {
            name,
            browser,
            profile,
            private,
        } => {
            let binding = pwrroute_engine::Binding {
                browser: browser.to_string(),
                profile: profile.to_string(),
                private: *private,
            };
            let name = name.to_string();
            crate::sync::edit_machine(
                |m| {
                    m.destinations.insert(name, binding);
                },
                cx,
            )
        }
        SettingsRequest::UseSyncedBinding(name) => crate::sync::edit_machine(
            |m| {
                m.destinations.remove(name.as_ref());
            },
            cx,
        ),
        SettingsRequest::SetRuleSetHere { name, on } => {
            let synced = crate::sync::rules_path()
                .and_then(|p| RulesFile::load(&p).ok())
                .is_none_or(|rules| rules.set_enabled(name));
            let (name, on) = (name.to_string(), *on);
            crate::sync::edit_machine(
                |m| {
                    if on == synced {
                        m.sets.remove(&name);
                    } else {
                        m.sets.insert(name, on);
                    }
                },
                cx,
            )
        }
        _ => return false,
    };
    match outcome {
        Ok(()) => refresh(cx),
        Err(e) => report(format!("Not changed: {e:#}."), cx),
    }
    true
}

/// Adds a folder: a found one by its label, else by what holds it.
fn add(root: PathBuf, cx: &mut App) -> anyhow::Result<()> {
    let found = crate::sync::feed(cx).and_then(|feed| {
        feed.read(cx)
            .places
            .iter()
            .find(|p| p.root == root)
            .cloned()
    });
    let target = match found {
        Some(place) => Target {
            kind: place.kind,
            label: place.label,
            root,
        },
        None => target_for(root),
    };
    crate::sync::add_target(target, cx)?;
    crate::sync::wake(cx);
    Ok(())
}

/// A folder the reader picked: picking a `PwrRoute` folder means its
/// parent, so sync never nests `PwrRoute/PwrRoute`.
pub fn target_for(picked: PathBuf) -> Target {
    let root = if picked
        .file_name()
        .is_some_and(|n| n == pwrroute_sync::FOLDER)
    {
        picked.parent().map_or(picked.clone(), Path::to_path_buf)
    } else {
        picked
    };
    let kind = pwrroute_sync::classify(&root, &pwrroute_sync::Env::current());
    let label = match kind {
        pwrroute_sync::Kind::Folder => root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        kind => kind.name().to_owned(),
    };
    Target { kind, label, root }
}

fn choose(cx: &mut App) {
    let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Sync here".into()),
    });
    cx.spawn(async move |cx| {
        let chosen = match paths.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            _ => None,
        };
        if let Some(path) = chosen {
            cx.update(|cx| {
                if let Err(e) = add(path, cx) {
                    report(format!("Not added: {e:#}."), cx);
                } else {
                    refresh(cx);
                }
            });
        }
    })
    .detach();
}

fn resolve(root: PathBuf, file: String, sha: String, keep_local: bool, cx: &mut App) {
    let Some(feed) = crate::sync::feed(cx) else {
        return;
    };
    let (tx, rx) = async_channel::bounded::<Result<(), String>>(1);
    let choice = if keep_local {
        pwrroute_sync::Choice::KeepLocal
    } else {
        pwrroute_sync::Choice::UseFolder
    };
    feed.read(cx)
        .resolve(root, file, sha, choice, move |result| {
            let _ = tx.try_send(result.map_err(|e| format!("{e:#}")));
        });
    cx.spawn(async move |cx| {
        if let Ok(result) = rx.recv().await {
            cx.update(|cx| match result {
                Ok(()) => {
                    crate::app::sync_rules_windows(cx);
                    refresh(cx);
                }
                Err(e) => report(format!("Not resolved: {e}."), cx),
            });
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwrroute_engine::{Binding, Destination, Rule, RuleSet};
    use pwrroute_sync::{FileStatus, Kind, Machine};
    use std::collections::BTreeMap;

    fn rules() -> RulesFile {
        RulesFile {
            default_browser: "Safari".into(),
            destinations: vec![
                Destination {
                    name: "Work 1".into(),
                    binding: Binding {
                        browser: "Google Chrome".into(),
                        profile: "Work".into(),
                        private: false,
                    },
                },
                Destination {
                    name: "PwrDrvr".into(),
                    binding: Binding {
                        browser: "Arc".into(),
                        ..Binding::default()
                    },
                },
            ],
            sets: vec![RuleSet {
                name: "Client".into(),
                enabled: true,
            }],
            rules: vec![
                Rule {
                    patterns: vec!["work.example.com/*".into()],
                    browser: "Google Chrome".into(),
                    profile: "Work".into(),
                    destination: Some("Work 1".into()),
                    set: Some("Client".into()),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["pwrdrvr.example/*".into()],
                    browser: "Arc".into(),
                    destination: Some("PwrDrvr".into()),
                    ..Rule::default()
                },
            ],
            ..RulesFile::default()
        }
    }

    fn installed() -> Vec<Installed> {
        vec![Installed {
            name: "Google Chrome".into(),
            bundle_id: Some("com.google.Chrome".into()),
            profiles: vec!["Work".into(), "Personal".into()],
            directories: vec!["Default".into(), "Profile 1".into()],
            hidden: false,
        }]
    }

    fn target(root: &str, label: &str) -> Target {
        Target {
            kind: Kind::Dropbox,
            label: label.into(),
            root: PathBuf::from(root),
        }
    }

    fn with<R>(
        status: &Status,
        targets: &[Target],
        machine: &MachineFile,
        f: impl FnOnce(Sync) -> R,
    ) -> R {
        with_config(status, targets, machine, Some("~/.finicky.ts"), f)
    }

    /// [`with`], with `config` in charge here (or none).
    fn with_config<R>(
        status: &Status,
        targets: &[Target],
        machine: &MachineFile,
        config: Option<&str>,
        f: impl FnOnce(Sync) -> R,
    ) -> R {
        let rules = rules();
        let installed = installed();
        let choices: Vec<BrowserChoice> = Vec::new();
        let when = |ms: u64| format!("t{ms}");
        f(build(&Facts {
            status,
            targets,
            places: &[Place {
                kind: Kind::ICloud,
                label: "iCloud Drive".into(),
                root: PathBuf::from("/Users/jo/Library/Mobile Documents/com~apple~CloudDocs"),
            }],
            rules: &rules,
            machine,
            computer_name: "Studio Mac",
            config: config.map(str::to_owned),
            config_on: false,
            choices: &choices,
            installed: &installed,
            home: Some(Path::new("/Users/jo")),
            here: "this Mac",
            when: &when,
        }))
    }

    #[test]
    fn nothing_set_up_offers_the_folders_found() {
        with(&Status::default(), &[], &MachineFile::default(), |sync| {
            assert!(sync.folders.is_empty());
            assert_eq!(sync.summary, "Not set up");
            assert_eq!(sync.offered.len(), 1);
            assert_eq!(
                sync.offered[0].shown,
                "~/Library/Mobile Documents/com~apple~CloudDocs/PwrRoute"
            );
        });
    }

    #[test]
    fn each_folder_says_how_it_stands() {
        let dropbox = target("/Users/jo/Library/CloudStorage/Dropbox", "Dropbox");
        let drive = target(
            "/Users/jo/Library/CloudStorage/GoogleDrive-jo@example.com/My Drive",
            "Google Drive",
        );
        let status = Status {
            machine: Machine::default(),
            last_pass: Some(5),
            backups: Vec::new(),
            targets: vec![
                TargetStatus {
                    target: dropbox.clone(),
                    files: vec![FileStatus {
                        name: "rules.json".into(),
                        health: Health::InSync,
                    }],
                    last_synced: Some(5),
                    saved_by: Some(Machine {
                        id: "a".into(),
                        name: "MacBook Air".into(),
                    }),
                    saved_at: Some(4),
                    folder_files: Default::default(),
                },
                TargetStatus {
                    target: drive.clone(),
                    files: vec![FileStatus {
                        name: "rules.json".into(),
                        health: Health::Unavailable("not there".into()),
                    }],
                    last_synced: None,
                    saved_by: None,
                    saved_at: None,
                    folder_files: Default::default(),
                },
            ],
        };
        with(
            &status,
            &[dropbox, drive],
            &MachineFile::default(),
            |sync| {
                assert_eq!(sync.folders[0].line, "t5 \u{00b7} saved by MacBook Air");
                assert_eq!(sync.folders[0].chip, "In sync");
                assert_eq!(sync.folders[0].tone, SyncTone::Ok);
                assert_eq!(sync.folders[1].chip, "Unavailable");
                assert_eq!(
                    sync.folders[1].line,
                    "Folder not found (is Dropbox running?) \u{00b7} tries again every minute"
                );
                assert_eq!(sync.summary, "2 folders");
                assert_eq!(sync.summary_tone, SyncTone::Neutral);
                assert_eq!(sync.offered.len(), 1);
            },
        );
    }

    /// A folder in step whose rules.json `other` saved after `runs_after`,
    /// holding `files` besides it.
    fn folder_with(runs_after: Option<&str>, files: &[&str]) -> (Target, Status) {
        let icloud = target("/i", "iCloud Drive");
        let other = Machine {
            id: "other".into(),
            name: "Studio Mac".into(),
        };
        let entry = |runs_after: Option<&str>| pwrroute_sync::FileEntry {
            sha256: "x".into(),
            bytes: 1,
            saved_by: other.clone(),
            saved_at: 7,
            runs_after: runs_after.map(str::to_owned),
        };
        let mut folder_files = BTreeMap::from([("rules.json".to_owned(), entry(runs_after))]);
        for name in files {
            folder_files.insert((*name).to_owned(), entry(None));
        }
        let status = Status {
            machine: Machine {
                id: "me".into(),
                name: "MacBook Air".into(),
            },
            targets: vec![TargetStatus {
                target: icloud.clone(),
                files: vec![FileStatus {
                    name: "rules.json".into(),
                    health: Health::InSync,
                }],
                last_synced: Some(8),
                saved_by: Some(other.clone()),
                saved_at: Some(7),
                folder_files,
            }],
            ..Status::default()
        };
        (icloud, status)
    }

    #[test]
    fn a_machine_without_a_config_is_offered_the_folders() {
        let (icloud, status) = folder_with(Some("~/.finicky.js"), &["finicky.js"]);
        let machine = MachineFile::default();
        with_config(
            &status,
            std::slice::from_ref(&icloud),
            &machine,
            None,
            |sync| {
                assert_eq!(
                    sync.folder_config,
                    Some(FolderConfig::Offer {
                        root: PathBuf::from("/i"),
                        name: "finicky.js".into(),
                        folder: "iCloud Drive".into(),
                        by: "Studio Mac".into(),
                        at: "t7".into(),
                        runs_after: "~/.finicky.js".into(),
                        here: "~/.finicky.js".into(),
                    })
                );
                assert_eq!(sync.folders[0].chip, "Needs you");
                assert_eq!(sync.folders[0].tone, SyncTone::Warn);
                assert_eq!(
                    sync.folders[0].line,
                    "rules.json in sync \u{00b7} finicky.js not used here"
                );
            },
        );
        // A machine with a config of its own keeps it: nothing is offered.
        with(&status, &[icloud], &machine, |sync| {
            assert_eq!(sync.folder_config, None);
            assert_eq!(sync.folders[0].chip, "In sync");
        });
    }

    #[test]
    fn rules_that_ran_after_a_config_the_folder_lacks_say_so() {
        let (icloud, status) = folder_with(Some("~/.finicky.ts"), &[]);
        let machine = MachineFile::default();
        with_config(
            &status,
            std::slice::from_ref(&icloud),
            &machine,
            None,
            |sync| {
                assert_eq!(
                    sync.folder_config,
                    Some(FolderConfig::Missing {
                        by: "Studio Mac".into(),
                        runs_after: "~/.finicky.ts".into(),
                    })
                );
                assert_eq!(sync.folders[0].tone, SyncTone::Warn);
            },
        );
        // Rules that ran alone, or that this machine saved, say nothing.
        let (icloud, mut status) = folder_with(None, &[]);
        with_config(
            &status,
            std::slice::from_ref(&icloud),
            &machine,
            None,
            |sync| {
                assert_eq!(sync.folder_config, None);
            },
        );
        status.machine.id = "other".into();
        let entry = status.targets[0]
            .folder_files
            .get_mut("rules.json")
            .unwrap();
        entry.runs_after = Some("~/.finicky.ts".into());
        with(&status, &[icloud], &machine, |sync| {
            assert_eq!(sync.folder_config, None);
        });
    }

    #[test]
    fn a_conflict_names_both_sides_and_what_differs() {
        let dropbox = target("/d", "Dropbox");
        let mine = rules();
        let mut theirs = rules();
        theirs.rules.push(Rule {
            patterns: vec!["*.example.org/*".into()],
            browser: "Google Chrome".into(),
            destination: Some("Work 1".into()),
            ..Rule::default()
        });
        let status = Status {
            targets: vec![TargetStatus {
                target: dropbox.clone(),
                files: vec![FileStatus {
                    name: "rules.json".into(),
                    health: Health::Conflict(Box::new(pwrroute_sync::Conflict {
                        kind: ConflictKind::BothChanged,
                        local: Some(serde_json::to_vec(&mine).unwrap()),
                        remote: serde_json::to_vec(&theirs).unwrap(),
                        remote_sha: "abc".into(),
                        remote_by: Some(Machine {
                            id: "a".into(),
                            name: "MacBook Air".into(),
                        }),
                        remote_at: Some(9),
                    })),
                }],
                last_synced: None,
                saved_by: None,
                saved_at: None,
                folder_files: Default::default(),
            }],
            ..Status::default()
        };
        with(&status, &[dropbox], &MachineFile::default(), |sync| {
            let conflict = sync.folders[0].conflict.clone().unwrap();
            assert_eq!(conflict.theirs_name, "MacBook Air");
            assert_eq!(conflict.mine, "2 rules \u{00b7} this Mac");
            assert_eq!(conflict.theirs, "3 rules \u{00b7} saved t9");
            assert_eq!(
                conflict.differences,
                vec![SharedString::from(
                    "+ rule 3 *.example.org/* \u{2192} Work 1"
                )]
            );
            assert_eq!(sync.folders[0].chip, "Conflict");
            assert_eq!(sync.summary, "1 conflict");
            assert_eq!(sync.summary_tone, SyncTone::Bad);
        });
    }

    #[test]
    fn destinations_say_what_they_mean_here_and_warn_when_they_cannot_open() {
        let machine = MachineFile {
            destinations: BTreeMap::from([(
                "Work 1".into(),
                Binding {
                    browser: "Google Chrome".into(),
                    profile: "Personal".into(),
                    private: false,
                },
            )]),
            sets: BTreeMap::from([("Client".into(), false)]),
            ..MachineFile::default()
        };
        with(&Status::default(), &[], &machine, |sync| {
            let work = &sync.destinations[0];
            assert!(work.this_machine);
            assert!(work.here.ends_with("Personal"), "{}", work.here);
            assert!(work.warning.is_none());
            let pwrdrvr = &sync.destinations[1];
            assert!(!pwrdrvr.this_machine);
            assert_eq!(pwrdrvr.uses, 1);
            assert!(
                pwrdrvr
                    .warning
                    .as_ref()
                    .unwrap()
                    .contains("isn\u{2019}t installed")
            );
            let set = &sync.sets[0];
            assert_eq!(
                (set.on, set.synced_on, set.overridden, set.entries),
                (false, true, true, 1)
            );
        });
    }

    #[test]
    fn differences_are_capped() {
        let mine = RulesFile::default();
        let theirs = RulesFile {
            rules: (0..8)
                .map(|i| Rule {
                    patterns: vec![format!("{i}.example/*")],
                    browser: "Safari".into(),
                    ..Rule::default()
                })
                .collect(),
            ..RulesFile::default()
        };
        let lines = differences(&mine, &theirs);
        assert_eq!(lines.len(), MAX_DIFFERENCES + 1);
        assert_eq!(lines.last().unwrap(), "and 3 more");
    }

    #[test]
    fn picking_a_pwrroute_folder_syncs_with_its_parent() {
        let target = target_for(PathBuf::from("/mnt/gdrive/PwrRoute"));
        assert_eq!(target.root, PathBuf::from("/mnt/gdrive"));
        assert_eq!(target.label, "gdrive");
        assert_eq!(
            target_for(PathBuf::from("/mnt/share")).root,
            PathBuf::from("/mnt/share")
        );
    }
}
