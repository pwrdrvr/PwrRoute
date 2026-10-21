//! The visual rules on disk: `rules.json`, read on open and written after
//! every edit.
//!
//! The store keeps the engine's [`RulesFile`] as the source of truth, so a
//! file written by the Go host, or by hand, comes back out with the fields
//! the editor does not show (`options`, `defaultProfile`) intact. Each
//! rewrite and handler has an id for the session, and the last version of it
//! that reached the file is kept by that id: an edit with problems (a
//! pattern removed, a browser cleared) is not written, and the file keeps
//! the saved version of that one rule while every other edit still saves,
//! as the design's validation turn says. A rule that was never complete, a
//! draft, is not in the file at all.
//!
//! Writes go through [`RulesFile::save`]: a sibling file renamed into place,
//! so the router's watcher never reads half a file. Nothing here logs or
//! writes anything but the rules.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

use pwrroute_engine::{self as engine, Place, Privacy, Problem, RouteTest, RulesFile};

use crate::app::Selection;
use crate::config_view::display_path;
use crate::rules_model::{self, Handler, Rewrite, RowBadge};

/// How the file stands against what is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The file holds every saveable edit. `at` is when it was last
    /// written, by this session or before it; `None` when there is no file
    /// yet.
    Saved { at: Option<SystemTime> },
    /// An edit is waiting for the debounce.
    Unsaved,
    /// The write is under way.
    Saving,
    /// The last write failed, or the file could not be read.
    Failed(String),
}

/// A rewrite or handler for the session: its position is its place in the
/// file, its id survives moves and renumbering.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry<T> {
    id: u64,
    value: T,
}

#[derive(Debug)]
pub struct RulesStore {
    /// `None` when the OS has no configuration directory: edits then live
    /// for the session only, and the status says so.
    path: Option<PathBuf>,
    /// The file's fields the lists do not hold: default browser and
    /// profile, options, and the saved `tests`. Its `rewrites` and `rules`
    /// stay empty.
    base: RulesFile,
    rewrites: Vec<Entry<engine::Rewrite>>,
    rules: Vec<Entry<engine::Rule>>,
    /// The last version of each entry that reached the file.
    saved_rewrites: HashMap<u64, engine::Rewrite>,
    saved_rules: HashMap<u64, engine::Rule>,
    next_id: u64,
    /// The file was there when the store was opened, or has been written
    /// since.
    exists: bool,
    status: Status,
    /// An edit has not reached the file.
    dirty: bool,
    /// The file could not be read: nothing is written over it until the
    /// window is opened again.
    unreadable: bool,
}

impl RulesStore {
    /// Opens the rules at `path`. A missing file is empty; a file that does
    /// not parse opens empty too, with the error in [`RulesStore::status`],
    /// and is never overwritten.
    pub fn load(path: Option<PathBuf>) -> Self {
        let Some(file_path) = path.as_deref() else {
            return Self::from_file(None, RulesFile::default(), false, None);
        };
        let exists = file_path.is_file();
        match RulesFile::load(file_path) {
            Ok(file) => {
                let modified = std::fs::metadata(file_path)
                    .and_then(|meta| meta.modified())
                    .ok();
                Self::from_file(path, file, exists, modified)
            }
            Err(error) => {
                let mut store = Self::from_file(path, RulesFile::default(), exists, None);
                store.status = Status::Failed(format!("rules.json did not load: {error:#}"));
                store.unreadable = true;
                store
            }
        }
    }

    /// A store over `file`, as if read from `path` (modified at `at`).
    pub fn from_file(
        path: Option<PathBuf>,
        file: RulesFile,
        exists: bool,
        at: Option<SystemTime>,
    ) -> Self {
        let mut store = Self {
            path,
            base: RulesFile {
                rewrites: Vec::new(),
                rules: Vec::new(),
                ..file.clone()
            },
            rewrites: Vec::new(),
            rules: Vec::new(),
            saved_rewrites: HashMap::new(),
            saved_rules: HashMap::new(),
            next_id: 1,
            exists,
            status: Status::Saved { at },
            dirty: false,
            unreadable: false,
        };
        for rewrite in file.rewrites {
            let id = store.next();
            store.saved_rewrites.insert(id, rewrite.clone());
            store.rewrites.push(Entry { id, value: rewrite });
        }
        for rule in file.rules {
            let id = store.next();
            store.saved_rules.insert(id, rule.clone());
            store.rules.push(Entry { id, value: rule });
        }
        store
    }

    fn next(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The file's directory as the footer names it: `~/Library/Application
    /// Support/PwrRoute`.
    pub fn location(&self, home: Option<&Path>) -> String {
        match self.path.as_deref().and_then(Path::parent) {
            Some(dir) => display_path(dir, home),
            None => "no configuration directory".into(),
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The file exists, or has been written this session.
    pub fn exists(&self) -> bool {
        self.exists
    }

    /// Nothing to route by: no rewrites and no handlers.
    pub fn is_empty(&self) -> bool {
        self.rewrites.is_empty() && self.rules.is_empty()
    }

    pub fn rewrite_count(&self) -> usize {
        self.rewrites.len()
    }

    pub fn handler_count(&self) -> usize {
        self.rules.len()
    }

    /// An edit is waiting to be written.
    pub fn needs_save(&self) -> bool {
        self.dirty
    }

    /// Every rule as edited, problems and drafts included: what the editor
    /// shows.
    pub fn edited(&self) -> RulesFile {
        RulesFile {
            rewrites: self.rewrites.iter().map(|e| e.value.clone()).collect(),
            rules: self.rules.iter().map(|e| e.value.clone()).collect(),
            ..self.base.clone()
        }
    }

    /// What the file holds once every saveable edit is written: an entry
    /// with problems stands as its last saved version, and a draft is left
    /// out. This is what the router routes by, so Test a link tests it.
    pub fn written(&self) -> RulesFile {
        let problems = self.problems();
        RulesFile {
            rewrites: self
                .rewrites
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    if problems.has(Place::Rewrite(i)) {
                        self.saved_rewrites.get(&entry.id).cloned()
                    } else {
                        Some(entry.value.clone())
                    }
                })
                .collect(),
            rules: self
                .rules
                .iter()
                .enumerate()
                .filter_map(|(i, entry)| {
                    if problems.has(Place::Rule(i)) {
                        self.saved_rules.get(&entry.id).cloned()
                    } else {
                        Some(entry.value.clone())
                    }
                })
                .collect(),
            ..self.base.clone()
        }
    }

    /// The problems of each rewrite and handler. The default browser's are
    /// left out: the engine falls back when the file names none, and the
    /// editor has no control for it yet.
    fn problems(&self) -> Problems {
        Problems(
            self.edited()
                .problems()
                .into_iter()
                .filter(|problem| problem.place != Place::DefaultBrowser)
                .collect(),
        )
    }

    /// The rows of the rewrites card.
    pub fn rewrites(&self) -> Vec<Rewrite> {
        let problems = self.problems();
        self.rewrites
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let problems = problems.at(Place::Rewrite(i));
                let badge = self.badge(&problems, self.saved_rewrites.contains_key(&entry.id));
                rules_model::rewrite(&entry.value, badge, problems)
            })
            .collect()
    }

    /// The rows of the handlers card.
    pub fn handlers(&self) -> Vec<Handler> {
        let problems = self.problems();
        self.rules
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let problems = problems.at(Place::Rule(i));
                let badge = self.badge(&problems, self.saved_rules.contains_key(&entry.id));
                rules_model::handler(&entry.value, badge, problems)
            })
            .collect()
    }

    fn badge(&self, problems: &[String], saved: bool) -> RowBadge {
        match (problems.is_empty(), saved) {
            (true, _) => RowBadge::None,
            (false, true) => RowBadge::NotSaved,
            (false, false) => RowBadge::Draft,
        }
    }

    /// The catch-all row.
    pub fn everything_else(&self) -> rules_model::Rule {
        rules_model::everything_else(&self.base)
    }

    /// The default browser's badge and name.
    pub fn default_browser(&self) -> (gpui_kit::SharedString, gpui_kit::SharedString) {
        rules_model::default_browser(&self.base)
    }

    // --- edits -----------------------------------------------------------

    fn touch(&mut self) {
        self.dirty = true;
        if !matches!(self.status, Status::Failed(_)) {
            self.status = Status::Unsaved;
        }
    }

    /// Appends an empty rewrite and returns its position.
    pub fn add_rewrite(&mut self) -> usize {
        let id = self.next();
        self.rewrites.push(Entry {
            id,
            value: engine::Rewrite::default(),
        });
        self.touch();
        self.rewrites.len() - 1
    }

    /// Appends an empty handler, just above Everything else, and returns its
    /// position.
    pub fn add_handler(&mut self) -> usize {
        let id = self.next();
        self.rules.push(Entry {
            id,
            value: engine::Rule::default(),
        });
        self.touch();
        self.rules.len() - 1
    }

    /// Removes a rule. A rule that is not there is ignored.
    pub fn delete(&mut self, selection: Selection) {
        match selection {
            Selection::Rewrite(i) if i < self.rewrites.len() => {
                let entry = self.rewrites.remove(i);
                self.saved_rewrites.remove(&entry.id);
            }
            Selection::Handler(i) if i < self.rules.len() => {
                let entry = self.rules.remove(i);
                self.saved_rules.remove(&entry.id);
            }
            _ => return,
        }
        self.touch();
    }

    pub fn enabled(&self, selection: Selection) -> Option<bool> {
        match selection {
            Selection::Rewrite(i) => self.rewrites.get(i).map(|e| e.value.enabled),
            Selection::Handler(i) => self.rules.get(i).map(|e| e.value.enabled),
        }
    }

    pub fn set_enabled(&mut self, selection: Selection, enabled: bool) {
        let slot = match selection {
            Selection::Rewrite(i) => self.rewrites.get_mut(i).map(|e| &mut e.value.enabled),
            Selection::Handler(i) => self.rules.get_mut(i).map(|e| &mut e.value.enabled),
        };
        if let Some(slot) = slot.filter(|slot| **slot != enabled) {
            *slot = enabled;
            self.touch();
        }
    }

    fn patterns_mut(&mut self, selection: Selection) -> Option<&mut Vec<String>> {
        match selection {
            Selection::Rewrite(i) => self.rewrites.get_mut(i).map(|e| &mut e.value.patterns),
            Selection::Handler(i) => self.rules.get_mut(i).map(|e| &mut e.value.patterns),
        }
    }

    /// Adds an empty pattern for the editor to fill in; returns its index.
    pub fn add_pattern(&mut self, selection: Selection) -> Option<usize> {
        let patterns = self.patterns_mut(selection)?;
        patterns.push(String::new());
        let index = patterns.len() - 1;
        self.touch();
        Some(index)
    }

    pub fn set_pattern(&mut self, selection: Selection, index: usize, text: &str) {
        if let Some(slot) = self.patterns_mut(selection).and_then(|p| p.get_mut(index))
            && slot != text
        {
            *slot = text.to_owned();
            self.touch();
        }
    }

    pub fn remove_pattern(&mut self, selection: Selection, index: usize) {
        if let Some(patterns) = self.patterns_mut(selection)
            && index < patterns.len()
        {
            patterns.remove(index);
            self.touch();
        }
    }

    fn rule_mut(&mut self, index: usize) -> Option<&mut engine::Rule> {
        self.rules.get_mut(index).map(|e| &mut e.value)
    }

    pub fn add_opener(&mut self, handler: usize) -> Option<usize> {
        let rule = self.rule_mut(handler)?;
        rule.opener.push(String::new());
        let index = rule.opener.len() - 1;
        self.touch();
        Some(index)
    }

    pub fn set_opener(&mut self, handler: usize, index: usize, text: &str) {
        if let Some(slot) = self.rule_mut(handler).and_then(|r| r.opener.get_mut(index))
            && slot != text
        {
            *slot = text.to_owned();
            self.touch();
        }
    }

    pub fn remove_opener(&mut self, handler: usize, index: usize) {
        if let Some(rule) = self.rule_mut(handler)
            && index < rule.opener.len()
        {
            rule.opener.remove(index);
            self.touch();
        }
    }

    pub fn set_browser(&mut self, handler: usize, text: &str) {
        if let Some(rule) = self.rule_mut(handler)
            && rule.browser != text
        {
            rule.browser = text.to_owned();
            self.touch();
        }
    }

    pub fn set_profile(&mut self, handler: usize, text: &str) {
        if let Some(rule) = self.rule_mut(handler)
            && rule.profile != text
        {
            rule.profile = text.to_owned();
            self.touch();
        }
    }

    fn actions_mut(&mut self, rewrite: usize) -> Option<&mut Vec<engine::RewriteAction>> {
        self.rewrites.get_mut(rewrite).map(|e| &mut e.value.actions)
    }

    /// Adds a change of the editor's first kind with no value; returns its
    /// index.
    pub fn add_action(&mut self, rewrite: usize) -> Option<usize> {
        let actions = self.actions_mut(rewrite)?;
        actions.push(rules_model::action(0, ""));
        let index = actions.len() - 1;
        self.touch();
        Some(index)
    }

    pub fn set_action(&mut self, rewrite: usize, index: usize, action: engine::RewriteAction) {
        if let Some(slot) = self.actions_mut(rewrite).and_then(|a| a.get_mut(index))
            && *slot != action
        {
            *slot = action;
            self.touch();
        }
    }

    pub fn remove_action(&mut self, rewrite: usize, index: usize) {
        if let Some(actions) = self.actions_mut(rewrite)
            && index < actions.len()
        {
            actions.remove(index);
            self.touch();
        }
    }

    // --- privacy, examples and tests ------------------------------------

    /// `rules[].private`: open the link in a private window.
    pub fn set_private(&mut self, handler: usize, private: bool) {
        if let Some(rule) = self.rule_mut(handler)
            && rule.private != private
        {
            rule.private = private;
            self.touch();
        }
    }

    /// `rules[].privacy`: what the router keeps of a link this handler
    /// routes. Written as the level's word; an unknown word the file had
    /// is replaced.
    pub fn set_privacy(&mut self, handler: usize, level: Privacy) {
        let setting = Some(engine::PrivacySetting::from(level));
        if let Some(rule) = self.rule_mut(handler)
            && rule.privacy != setting
        {
            rule.privacy = setting;
            self.touch();
        }
    }

    /// Appends a URL that must resolve to this handler. It is the one way
    /// a link reaches `rules.json` from the Rules screen, and only by the
    /// user's choice. Returns its index; a blank URL, or one already there,
    /// is not added.
    pub fn add_example(&mut self, handler: usize, url: &str) -> Option<usize> {
        let url = url.trim();
        let rule = self.rule_mut(handler)?;
        if url.is_empty() {
            return None;
        }
        if let Some(i) = rule.examples.iter().position(|e| e == url) {
            return Some(i);
        }
        rule.examples.push(url.to_owned());
        let index = rule.examples.len() - 1;
        self.touch();
        Some(index)
    }

    pub fn remove_example(&mut self, handler: usize, index: usize) {
        if let Some(rule) = self.rule_mut(handler)
            && index < rule.examples.len()
        {
            rule.examples.remove(index);
            self.touch();
        }
    }

    /// Appends a URL this rewrite must match, with what it must become.
    pub fn add_rewrite_example(
        &mut self,
        rewrite: usize,
        url: &str,
        becomes: &str,
    ) -> Option<usize> {
        let (url, becomes) = (url.trim(), becomes.trim());
        let entry = self.rewrites.get_mut(rewrite).map(|e| &mut e.value)?;
        if url.is_empty() || becomes.is_empty() {
            return None;
        }
        let example = engine::RewriteExample {
            url: url.to_owned(),
            becomes: becomes.to_owned(),
        };
        if let Some(i) = entry.examples.iter().position(|e| *e == example) {
            return Some(i);
        }
        entry.examples.push(example);
        let index = entry.examples.len() - 1;
        self.touch();
        Some(index)
    }

    pub fn remove_rewrite_example(&mut self, rewrite: usize, index: usize) {
        if let Some(entry) = self.rewrites.get_mut(rewrite).map(|e| &mut e.value)
            && index < entry.examples.len()
        {
            entry.examples.remove(index);
            self.touch();
        }
    }

    /// The top-level `tests`: whole resolutions the file pins.
    pub fn tests(&self) -> &[RouteTest] {
        &self.base.tests
    }

    /// Appends a test and returns its index; a test already there is not
    /// added twice.
    pub fn add_test(&mut self, test: RouteTest) -> usize {
        if let Some(i) = self.base.tests.iter().position(|t| *t == test) {
            return i;
        }
        self.base.tests.push(test);
        self.touch();
        self.base.tests.len() - 1
    }

    pub fn remove_test(&mut self, index: usize) {
        if index < self.base.tests.len() {
            self.base.tests.remove(index);
            self.touch();
        }
    }

    /// Inserts a whole handler at `at` (clamped to the end) and returns its
    /// position. Activity's Add a rule sheet builds the rule and lands it
    /// here, complete, so it is written on the next save.
    pub fn insert_rule(&mut self, at: usize, rule: engine::Rule) -> usize {
        let id = self.next();
        let at = at.min(self.rules.len());
        self.rules.insert(at, Entry { id, value: rule });
        self.touch();
        at
    }

    // --- the file --------------------------------------------------------

    /// Writes every saveable edit to the file, now. The status says how it
    /// went; the error is also returned.
    pub fn save(&mut self) -> Result<(), String> {
        let result = self.write();
        self.status = match &result {
            Ok(()) => Status::Saved {
                at: Some(SystemTime::now()),
            },
            Err(error) => Status::Failed(error.clone()),
        };
        result
    }

    fn write(&mut self) -> Result<(), String> {
        if self.unreadable {
            return Err("rules.json did not load, so it is not overwritten.".into());
        }
        let Some(path) = self.path.clone() else {
            return Err("No configuration directory: edits last for this session.".into());
        };
        self.status = Status::Saving;
        let file = self.written();
        file.save(&path)
            .map_err(|error| format!("Could not save rules.json: {error:#}"))?;
        // What was written is what each entry's saved version now is.
        let problems = self.problems();
        for (i, entry) in self.rewrites.iter().enumerate() {
            if !problems.has(Place::Rewrite(i)) {
                self.saved_rewrites.insert(entry.id, entry.value.clone());
            }
        }
        for (i, entry) in self.rules.iter().enumerate() {
            if !problems.has(Place::Rule(i)) {
                self.saved_rules.insert(entry.id, entry.value.clone());
            }
        }
        self.exists = true;
        self.dirty = false;
        Ok(())
    }

    /// The status for the rail's foot: "saved 09:41", "unsaved", "saving",
    /// or what went wrong.
    pub fn status_label(&self) -> String {
        match &self.status {
            Status::Saved { at: Some(at) } => format!("saved {}", clock(*at)),
            Status::Saved { at: None } if self.path.is_some() => "no rules.json yet".into(),
            Status::Saved { at: None } => "not saved: no configuration directory".into(),
            Status::Unsaved => "unsaved".into(),
            Status::Saving => "saving\u{2026}".into(),
            Status::Failed(error) => format!("not saved \u{00b7} {error}"),
        }
    }

    /// The editor's foot for the selected rule: its own state first, then
    /// the file's.
    pub fn rail_label(&self, selection: Selection) -> String {
        let (problems, saved) = match selection {
            Selection::Rewrite(i) => (
                self.problems().at(Place::Rewrite(i)),
                self.rewrites
                    .get(i)
                    .is_some_and(|e| self.saved_rewrites.contains_key(&e.id)),
            ),
            Selection::Handler(i) => (
                self.problems().at(Place::Rule(i)),
                self.rules
                    .get(i)
                    .is_some_and(|e| self.saved_rules.contains_key(&e.id)),
            ),
        };
        match (problems.len(), saved, selection) {
            (0, _, _) => match &self.status {
                Status::Saved { at: Some(at) } => format!("edited {} \u{00b7} saved", clock(*at)),
                _ => self.status_label(),
            },
            (_, false, Selection::Handler(_)) => "saves once it has a pattern and a browser".into(),
            (_, false, Selection::Rewrite(_)) => "saves once it has a pattern and a change".into(),
            (1, true, _) => "1 problem \u{00b7} not saved".into(),
            (n, true, _) => format!("{n} problems \u{00b7} not saved"),
        }
    }
}

/// The problems of a file, looked up by place.
struct Problems(Vec<Problem>);

impl Problems {
    fn has(&self, place: Place) -> bool {
        self.0.iter().any(|p| p.place == place)
    }

    fn at(&self, place: Place) -> Vec<String> {
        self.0
            .iter()
            .filter(|p| p.place == place)
            .map(|p| p.message.clone())
            .collect()
    }
}

/// A time as the design's labels show it: `09:41`, local.
pub fn clock(at: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(at)
        .format("%H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwrroute_engine::RewriteAction;
    use serde_json::json;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-app-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn fixture() -> RulesFile {
        crate::fixtures::rules_file()
    }

    #[test]
    fn a_rules_json_round_trips_through_the_view() {
        let dir = scratch("round-trip");
        let path = dir.join("rules.json");
        let file = fixture();
        file.save(&path).unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        assert!(store.exists());
        assert!(matches!(store.status(), Status::Saved { at: Some(_) }));
        assert_eq!(store.edited(), file);
        assert_eq!(store.written(), file);

        // The rows say what the file says.
        let handlers = store.handlers();
        assert_eq!(handlers.len(), 7);
        assert_eq!(handlers[2].openers, ["com.tinyspeck.slackmacgap"]);
        assert_eq!(handlers[2].browser, "Firefox");
        assert_eq!(handlers[2].profile, "default-release");
        assert!(!handlers[6].rule.enabled);
        assert!(handlers.iter().all(|h| h.problems.is_empty()));
        let rewrites = store.rewrites();
        assert_eq!(rewrites.len(), 3);
        assert_eq!(
            rewrites[1].actions,
            [RewriteAction::DropQuery(vec![
                "utm_*".into(),
                "fbclid".into()
            ])]
        );

        // Saving without an edit writes the same file: defaultProfile and
        // the single-pattern `match` strings included.
        store.save().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::to_value(&file).unwrap()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn options_the_editor_does_not_show_survive_a_save() {
        let dir = scratch("options");
        let path = dir.join("rules.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"{ "defaultBrowser": "Safari", "defaultProfile": "Home",
                 "options": { "keepRunning": true },
                 "rules": [{ "match": "github.com/*", "browser": "Firefox" }] }"#,
        )
        .unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        store.set_profile(0, "Work");
        store.save().unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({
                "defaultBrowser": "Safari",
                "defaultProfile": "Home",
                "options": { "keepRunning": true },
                "rules": [{ "match": "github.com/*", "browser": "Firefox", "profile": "Work" }]
            })
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn add_edit_delete_and_save_produce_the_expected_json() {
        let dir = scratch("edits");
        let path = dir.join("nested").join("rules.json");
        let mut store = RulesStore::load(Some(path.clone()));
        assert!(!store.exists());
        assert!(store.is_empty());
        assert_eq!(store.status_label(), "no rules.json yet");

        // A draft handler is not in the file until it has a pattern and a
        // browser.
        let h = store.add_handler();
        assert_eq!(h, 0);
        assert!(store.needs_save());
        assert_eq!(store.status(), &Status::Unsaved);
        assert_eq!(store.handlers()[0].rule.badge, RowBadge::Draft);
        assert_eq!(
            store.rail_label(Selection::Handler(0)),
            "saves once it has a pattern and a browser"
        );
        store.save().unwrap();
        assert!(store.exists(), "the directory and file are created");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
                .unwrap(),
            json!({ "defaultBrowser": "", "rules": [] })
        );

        let i = store.add_pattern(Selection::Handler(0)).unwrap();
        store.set_pattern(Selection::Handler(0), i, "notion.so/*");
        store.set_browser(0, "Firefox");
        assert_eq!(store.handlers()[0].rule.badge, RowBadge::None);
        store.save().unwrap();
        let json = |path: &Path| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        assert_eq!(
            json(&path)["rules"],
            json!([{ "match": "notion.so/*", "browser": "Firefox" }])
        );

        // A rewrite with its change.
        let r = store.add_rewrite();
        let p = store.add_pattern(Selection::Rewrite(r)).unwrap();
        store.set_pattern(Selection::Rewrite(r), p, "x.com/*");
        let a = store.add_action(r).unwrap();
        store.set_action(r, a, rules_model::action(0, "xcancel.com"));
        store.set_enabled(Selection::Rewrite(r), false);
        store.save().unwrap();
        assert_eq!(
            json(&path)["rewrites"],
            json!([{ "match": "x.com/*", "actions": [{ "setHost": "xcancel.com" }], "enabled": false }])
        );
        assert!(
            store.status_label().starts_with("saved "),
            "{}",
            store.status_label()
        );
        assert!(
            store
                .rail_label(Selection::Rewrite(0))
                .ends_with("\u{00b7} saved"),
            "{}",
            store.rail_label(Selection::Rewrite(0))
        );

        // Clearing the browser is a problem: the file keeps the saved
        // handler, the row says not saved, and Test a link gets the saved
        // version.
        store.set_browser(0, "");
        assert_eq!(store.handlers()[0].rule.badge, RowBadge::NotSaved);
        assert_eq!(store.handlers()[0].problems, ["Select a browser."]);
        assert_eq!(
            store.rail_label(Selection::Handler(0)),
            "1 problem \u{00b7} not saved"
        );
        assert_eq!(store.written().rules[0].browser, "Firefox");
        store.save().unwrap();
        assert_eq!(json(&path)["rules"][0]["browser"], "Firefox");

        // Delete writes the file without it.
        store.delete(Selection::Handler(0));
        store.delete(Selection::Rewrite(0));
        assert!(store.is_empty());
        store.save().unwrap();
        assert_eq!(json(&path), json!({ "defaultBrowser": "", "rules": [] }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A `rules.json` the Go host wrote, plus every field this editor adds:
    /// each comes back out as it went in, around an unrelated edit, and a
    /// file with none of them gains none.
    #[test]
    fn privacy_examples_and_tests_round_trip_through_the_store() {
        let dir = scratch("privacy-round-trip");
        let path = dir.join("rules.json");
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"{
          "defaultBrowser": "Firefox",
          "defaultProfile": "",
          "options": { "keepRunning": true },
          "rewrites": [
            { "match": "x.com/*", "actions": [{ "setHost": "xcancel.com" }], "privacy": "none",
              "examples": [{ "url": "https://x.com/a", "becomes": "https://xcancel.com/a" }] }
          ],
          "rules": [
            { "match": "github.com/*", "browser": "Arc" },
            { "match": "bank.example/*", "browser": "Safari", "private": true,
              "examples": ["https://bank.example/"] },
            { "match": "a.example/*", "browser": "Safari", "privacy": "host" },
            { "match": "b.example/*", "browser": "Safari", "private": true, "privacy": "full" },
            { "match": "c.example/*", "browser": "Safari", "privacy": "hidden" }
          ],
          "tests": [
            { "url": "https://bank.example/", "expect": { "browser": "Safari" } },
            { "url": "https://x.com/a", "opener": "Slack:com.tinyspeck.slackmacgap",
              "expect": { "browser": "Firefox", "profile": "Work", "url": "https://xcancel.com/a" } }
          ]
        }"#;
        std::fs::write(&path, text).unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        let handlers = store.handlers();
        assert!(handlers[1].private);
        assert_eq!(handlers[1].examples, ["https://bank.example/"]);
        assert_eq!(handlers[1].keep(), Some(Privacy::Host));
        assert_eq!(handlers[2].keep(), Some(Privacy::Host));
        assert_eq!(handlers[3].keep(), Some(Privacy::Full));
        // The unknown word is a problem the editor shows; the file keeps it.
        assert_eq!(handlers[4].keep(), None);
        assert_eq!(
            handlers[4].problems,
            ["Privacy must be full, host or none."]
        );
        assert_eq!(handlers[4].rule.badge, RowBadge::NotSaved);
        assert_eq!(store.rewrites()[0].examples.len(), 1);
        assert_eq!(store.tests().len(), 2);

        store.set_profile(0, "Work");
        store.save().unwrap();
        let expected: serde_json::Value = serde_json::from_str(text).unwrap();
        let mut written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["rules"][0]["profile"], "Work");
        written["rules"][0]
            .as_object_mut()
            .unwrap()
            .remove("profile");
        // `defaultProfile: ""` is the one key the Go host wrote that the
        // engine leaves out when empty.
        let mut expected = expected;
        expected.as_object_mut().unwrap().remove("defaultProfile");
        assert_eq!(written, expected);

        // A Go-host file gains none of the new keys.
        let go = dir.join("go.json");
        std::fs::write(
            &go,
            r#"{ "defaultBrowser": "Safari", "rules": [{ "match": "a/*", "browser": "Firefox" }] }"#,
        )
        .unwrap();
        let mut store = RulesStore::load(Some(go.clone()));
        store.set_profile(0, "Home");
        store.save().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&go).unwrap())
                .unwrap(),
            json!({ "defaultBrowser": "Safari", "rules": [{ "match": "a/*", "browser": "Firefox", "profile": "Home" }] })
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn examples_tests_and_privacy_are_edited_and_written() {
        let dir = scratch("examples");
        let path = dir.join("rules.json");
        let mut store = RulesStore::load(Some(path.clone()));
        let h = store.add_handler();
        let p = store.add_pattern(Selection::Handler(h)).unwrap();
        store.set_pattern(Selection::Handler(h), p, "bank.example/*");
        store.set_browser(h, "Safari");
        assert_eq!(store.add_example(h, " https://bank.example/ "), Some(0));
        assert_eq!(
            store.add_example(h, "https://bank.example/"),
            Some(0),
            "not twice"
        );
        assert_eq!(store.add_example(h, "  "), None);
        assert_eq!(store.add_example(h, "https://bank.example/login"), Some(1));
        store.set_private(h, true);
        store.set_privacy(h, Privacy::None);
        let r = store.add_rewrite();
        let p = store.add_pattern(Selection::Rewrite(r)).unwrap();
        store.set_pattern(Selection::Rewrite(r), p, "x.com/*");
        let a = store.add_action(r).unwrap();
        store.set_action(r, a, rules_model::action(0, "xcancel.com"));
        assert_eq!(
            store.add_rewrite_example(r, "https://x.com/a", "https://xcancel.com/a"),
            Some(0)
        );
        assert_eq!(store.add_rewrite_example(r, "https://x.com/a", ""), None);
        let t = store.add_test(RouteTest {
            url: "https://bank.example/".into(),
            opener: None,
            expect: engine::Expectation {
                browser: "Safari".into(),
                ..engine::Expectation::default()
            },
        });
        assert_eq!(t, 0);
        store.save().unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json["rules"][0],
            json!({ "match": "bank.example/*", "browser": "Safari", "private": true, "privacy": "none",
                    "examples": ["https://bank.example/", "https://bank.example/login"] })
        );
        assert_eq!(
            json["rewrites"][0]["examples"],
            json!([{ "url": "https://x.com/a", "becomes": "https://xcancel.com/a" }])
        );
        assert_eq!(
            json["tests"],
            json!([{ "url": "https://bank.example/", "expect": { "browser": "Safari" } }])
        );

        // A bad example is a problem: the file keeps the saved rule.
        store.add_example(h, "bank.example/x");
        assert_eq!(store.handlers()[0].rule.badge, RowBadge::NotSaved);
        assert_eq!(
            store.handlers()[0].problems,
            ["Examples must be full URLs, such as https://example.com/path."]
        );
        store.remove_example(h, 2);
        assert_eq!(store.handlers()[0].rule.badge, RowBadge::None);
        store.remove_example(h, 0);
        store.remove_rewrite_example(r, 0);
        store.remove_test(0);
        // Inserting a complete rule at the top renumbers the rest.
        let at = store.insert_rule(
            0,
            engine::Rule {
                patterns: vec!["a.example/*".into()],
                browser: "Arc".into(),
                examples: vec!["https://a.example/".into()],
                ..engine::Rule::default()
            },
        );
        assert_eq!(at, 0);
        assert_eq!(store.insert_rule(99, engine::Rule::default()), 2);
        store.save().unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["rules"][0]["match"], "a.example/*");
        assert_eq!(
            json["rules"][1]["examples"],
            json!(["https://bank.example/login"])
        );
        assert_eq!(
            json["rules"].as_array().unwrap().len(),
            2,
            "the draft is not written"
        );
        assert_eq!(json["rewrites"][0].get("examples"), None);
        assert_eq!(json.get("tests"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_file_that_does_not_parse_is_shown_and_never_overwritten() {
        let dir = scratch("broken");
        let path = dir.join("rules.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        assert!(store.exists());
        assert!(
            matches!(store.status(), Status::Failed(_)),
            "{:?}",
            store.status()
        );
        assert!(
            store
                .status_label()
                .starts_with("not saved \u{00b7} rules.json did not load")
        );
        store.add_handler();
        assert!(store.save().is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_a_configuration_directory_edits_last_for_the_session() {
        let mut store = RulesStore::load(None);
        store.add_handler();
        assert!(store.save().is_err());
        assert_eq!(store.handler_count(), 1);
        assert!(store.status_label().starts_with("not saved"));
    }

    #[test]
    fn the_empty_state_model() {
        let store = RulesStore::from_file(None, RulesFile::default(), false, None);
        assert!(store.is_empty() && !store.exists());
        assert_eq!(store.rewrite_count() + store.handler_count(), 0);
        let (badge, name) = store.default_browser();
        assert_eq!((badge.as_ref(), name.as_ref()), ("Sf", "Safari"));
        let named = RulesStore::from_file(
            None,
            RulesFile {
                default_browser: "Google Chrome".into(),
                default_profile: "Personal".into(),
                ..RulesFile::default()
            },
            true,
            None,
        );
        assert!(named.is_empty() && named.exists());
        assert_eq!(named.default_browser().1.as_ref(), "Google Chrome");
    }
}
