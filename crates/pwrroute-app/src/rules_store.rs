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
//!
//! The file has other writers: an agent's `pwrroute_rules_apply`, an editor,
//! a sync from another machine. The store keeps a [`Stamp`] of the bytes it
//! last read or wrote, and [`RulesStore::check_disk`] compares the file to
//! it: a changed file is read again while nothing is unsaved, and is never
//! written over while something is. Then the status says so, and the person
//! chooses [`RulesStore::reload`] or [`RulesStore::overwrite`].

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash as _, Hasher as _},
    path::{Path, PathBuf},
    time::SystemTime,
};

use pwrroute_engine::{self as engine, Place, Precedence, Privacy, Problem, RouteTest, RulesFile};

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
    /// The file changed on disk with nothing unsaved here, and was read
    /// again at `at`.
    Reloaded { at: SystemTime },
    /// The file changed on disk under unsaved edits. Nothing is written
    /// until [`RulesStore::reload`] drops the edits or
    /// [`RulesStore::overwrite`] writes them anyway.
    Changed,
}

/// What [`RulesStore::check_disk`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disk {
    /// The file is as the store last read or wrote it.
    Unchanged,
    /// The file had changed and nothing was unsaved: the store now holds
    /// what it says.
    Reloaded,
    /// The file had changed under unsaved edits: the store is as it was,
    /// and [`Status::Changed`] until reload or overwrite.
    Changed,
}

/// The message a refused write carries, and the status after it.
pub const CHANGED_ON_DISK: &str = "rules.json changed on disk; reload or overwrite";

/// The file as last read or written: its length and a hash of its bytes.
/// A hash rather than a modification time, since some editors and sync
/// tools keep the time. `None` is no file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    hash: u64,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Some(Self {
            len: bytes.len() as u64,
            hash: hasher.finish(),
        })
    }
}

/// A rewrite or handler for the session: its position is its place in the
/// file, its id survives moves and renumbering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<T> {
    id: u64,
    value: T,
}

/// Cloned only to ask what an edit would do before making it (the row
/// menu's and the drag's "N links change").
#[derive(Clone, Debug)]
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
    /// The file could not be read: nothing is written over it until it
    /// changes on disk and reads, or the window is opened again.
    unreadable: bool,
    /// The file as last read or written here; what [`Self::check_disk`]
    /// compares the file to.
    stamp: Option<Stamp>,
}

impl RulesStore {
    /// Opens the rules at `path`. A missing file is empty; a file that does
    /// not parse opens empty too, with the error in [`RulesStore::status`],
    /// and is never overwritten.
    pub fn load(path: Option<PathBuf>) -> Self {
        let Some(file_path) = path.as_deref() else {
            return Self::from_file(None, RulesFile::default(), false, None);
        };
        // Stamped before it is read: a write that lands between the two is
        // then a change the next check sees, never one it misses.
        let stamp = Stamp::of(file_path);
        let exists = file_path.is_file();
        let mut store = match RulesFile::load(file_path) {
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
        };
        store.stamp = stamp;
        store
    }

    /// A store over `file`, as if read from `path` (modified at `at`). The
    /// file at `path`, when there is one, is what [`Self::check_disk`]
    /// will call unchanged.
    pub fn from_file(
        path: Option<PathBuf>,
        file: RulesFile,
        exists: bool,
        at: Option<SystemTime>,
    ) -> Self {
        let stamp = path.as_deref().and_then(Stamp::of);
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
            stamp,
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

    #[cfg(test)]
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

    /// The file changed on disk under unsaved edits: the rail foot offers
    /// Reload and Overwrite.
    pub fn changed_on_disk(&self) -> bool {
        self.status == Status::Changed
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
    pub fn default_browser(&self) -> (crate::widgets::AppBadge, gpui_kit::SharedString) {
        rules_model::default_browser(&self.base)
    }

    /// `rules.json`'s `precedence`: whether these rules and the default
    /// browser go before a JS config's handlers and default.
    /// [`Precedence::Config`] (the file's absent field) is the Go host's
    /// order.
    pub fn precedence(&self) -> Precedence {
        self.base.precedence
    }

    /// Sets `precedence`, as an edit written on the next save like any
    /// other. Setting the value the store already has changes nothing and
    /// leaves the store clean. [`Precedence::Config`] is written by leaving
    /// the field out.
    pub fn set_precedence(&mut self, precedence: Precedence) {
        if self.base.precedence != precedence {
            self.base.precedence = precedence;
            self.touch();
        }
    }

    /// Sets `defaultBrowser` and `defaultProfile` ("" for the browser's
    /// own default), as an edit written on the next save: the Overview's
    /// setup card and Settings' Everything else. The same values change
    /// nothing.
    pub fn set_default_browser(&mut self, browser: &str, profile: &str) {
        if self.base.default_browser != browser || self.base.default_profile != profile {
            browser.clone_into(&mut self.base.default_browser);
            profile.clone_into(&mut self.base.default_profile);
            self.touch();
        }
    }

    // --- edits -----------------------------------------------------------

    fn touch(&mut self) {
        self.dirty = true;
        if !matches!(self.status, Status::Failed(_) | Status::Changed) {
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

    /// Takes a rule out of its list for Undo delete: its id and saved
    /// version go with it, so [`Self::restore`] puts back the same rule,
    /// saved or not as it was. `None` when it is not there.
    pub fn remove(&mut self, selection: Selection) -> Option<Removed> {
        let removed = match selection {
            Selection::Rewrite(i) if i < self.rewrites.len() => {
                let entry = self.rewrites.remove(i);
                let saved = self.saved_rewrites.remove(&entry.id);
                Removed::Rewrite {
                    at: i,
                    entry,
                    saved,
                }
            }
            Selection::Handler(i) if i < self.rules.len() => {
                let entry = self.rules.remove(i);
                let saved = self.saved_rules.remove(&entry.id);
                Removed::Rule {
                    at: i,
                    entry,
                    saved,
                }
            }
            _ => return None,
        };
        self.touch();
        Some(removed)
    }

    /// Undo delete: the rule back where it was (or at the end of its list,
    /// if the list is now shorter). Returns where it landed.
    pub fn restore(&mut self, removed: Removed) -> Selection {
        let restored = match removed {
            Removed::Rewrite { at, entry, saved } => {
                let at = at.min(self.rewrites.len());
                if let Some(saved) = saved {
                    self.saved_rewrites.insert(entry.id, saved);
                }
                self.rewrites.insert(at, entry);
                Selection::Rewrite(at)
            }
            Removed::Rule { at, entry, saved } => {
                let at = at.min(self.rules.len());
                if let Some(saved) = saved {
                    self.saved_rules.insert(entry.id, saved);
                }
                self.rules.insert(at, entry);
                Selection::Handler(at)
            }
        };
        self.touch();
        restored
    }

    /// Moves a rule to position `to` of its own list (clamped to the end),
    /// keeping its id: its saved version moves with it. A rule never moves
    /// between the lists. Returns where it landed, or `None` when it is not
    /// there or is already at `to`.
    pub fn move_to(&mut self, selection: Selection, to: usize) -> Option<Selection> {
        fn shift<T>(list: &mut Vec<T>, from: usize, to: usize) -> Option<usize> {
            if from >= list.len() {
                return None;
            }
            let to = to.min(list.len() - 1);
            if to == from {
                return None;
            }
            let item = list.remove(from);
            list.insert(to, item);
            Some(to)
        }
        let moved = match selection {
            Selection::Rewrite(i) => shift(&mut self.rewrites, i, to).map(Selection::Rewrite),
            Selection::Handler(i) => shift(&mut self.rules, i, to).map(Selection::Handler),
        };
        if moved.is_some() {
            self.touch();
        }
        moved
    }

    /// A copy of a rule just below it, as a new rule. Its examples stay
    /// with the original: the copy runs second, so the same links would
    /// fail as its examples. Returns the copy's position.
    pub fn duplicate(&mut self, selection: Selection) -> Option<Selection> {
        let copy = match selection {
            Selection::Rewrite(i) => {
                let mut value = self.rewrites.get(i)?.value.clone();
                value.examples.clear();
                let id = self.next();
                self.rewrites.insert(i + 1, Entry { id, value });
                Selection::Rewrite(i + 1)
            }
            Selection::Handler(i) => {
                let mut value = self.rules.get(i)?.value.clone();
                value.examples.clear();
                let id = self.next();
                self.rules.insert(i + 1, Entry { id, value });
                Selection::Handler(i + 1)
            }
        };
        self.touch();
        Some(copy)
    }

    /// How many rows `selection`'s list has.
    pub fn len_of(&self, selection: Selection) -> usize {
        match selection {
            Selection::Rewrite(_) => self.rewrites.len(),
            Selection::Handler(_) => self.rules.len(),
        }
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

    /// Replaces every rule and the file's other fields with `file`, as one
    /// edit: the importer's write and Undo import. Written on the next save.
    pub fn replace_all(&mut self, file: RulesFile) {
        self.base = RulesFile {
            rewrites: Vec::new(),
            rules: Vec::new(),
            ..file.clone()
        };
        self.rewrites.clear();
        self.rules.clear();
        for value in file.rewrites {
            let id = self.next();
            self.rewrites.push(Entry { id, value });
        }
        for value in file.rules {
            let id = self.next();
            self.rules.push(Entry { id, value });
        }
        self.touch();
    }

    // --- the file --------------------------------------------------------

    /// Compares the file to what was last read or written here. A changed
    /// file is read again when nothing is unsaved; under unsaved edits the
    /// store keeps them and the status turns [`Status::Changed`], which
    /// also stops the next save. Cheap enough for every refresh: one read
    /// of a small file.
    pub fn check_disk(&mut self) -> Disk {
        let Some(path) = self.path.clone() else {
            return Disk::Unchanged;
        };
        if Stamp::of(&path) == self.stamp {
            // The file is back as it was read: an editor undid its change.
            if self.status == Status::Changed {
                self.status = Status::Unsaved;
            }
            return Disk::Unchanged;
        }
        if self.dirty {
            self.status = Status::Changed;
            return Disk::Changed;
        }
        self.reload();
        Disk::Reloaded
    }

    /// Reads the file again, dropping every unsaved edit. The entries get
    /// new ids; the status says when.
    pub fn reload(&mut self) {
        let path = self.path.take();
        *self = Self::load(path);
        if !self.unreadable {
            self.status = Status::Reloaded {
                at: SystemTime::now(),
            };
        }
    }

    /// Writes every saveable edit to the file, now. The status says how it
    /// went; the error is also returned. A file that changed on disk since
    /// it was read is not written: see [`Self::overwrite`].
    pub fn save(&mut self) -> Result<(), String> {
        self.save_with(false)
    }

    /// Writes every saveable edit over a file that changed on disk: the
    /// person's choice from the rail foot.
    pub fn overwrite(&mut self) -> Result<(), String> {
        self.save_with(true)
    }

    fn save_with(&mut self, force: bool) -> Result<(), String> {
        let result = self.write(force);
        self.status = match &result {
            Ok(()) => Status::Saved {
                at: Some(SystemTime::now()),
            },
            Err(error) if error == CHANGED_ON_DISK => Status::Changed,
            Err(error) => Status::Failed(error.clone()),
        };
        result
    }

    /// Why the next save would be refused before writing anything: the
    /// file did not load, there is no directory for it, or it changed on
    /// disk since it was read. `None` when it would be written.
    pub fn save_refused(&self) -> Option<String> {
        self.refusal(false)
    }

    fn refusal(&self, force: bool) -> Option<String> {
        if self.unreadable {
            return Some("rules.json did not load, so it is not overwritten.".into());
        }
        let Some(path) = &self.path else {
            return Some("No configuration directory: edits last for this session.".into());
        };
        if !force && Stamp::of(path) != self.stamp {
            return Some(CHANGED_ON_DISK.into());
        }
        None
    }

    fn write(&mut self, force: bool) -> Result<(), String> {
        if let Some(reason) = self.refusal(force) {
            return Err(reason);
        }
        let Some(path) = self.path.clone() else {
            return Err("No configuration directory: edits last for this session.".into());
        };
        self.status = Status::Saving;
        let file = self.written();
        file.save(&path)
            .map_err(|error| format!("Could not save rules.json: {error:#}"))?;
        // The file as written is the file as read: the store's own save is
        // not a change on disk.
        self.stamp = Stamp::of(&path);
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
    /// "reloaded 09:41", or what went wrong.
    pub fn status_label(&self) -> String {
        match &self.status {
            Status::Saved { at: Some(at) } => format!("saved {}", clock(*at)),
            Status::Saved { at: None } if self.path.is_some() => "no rules.json yet".into(),
            Status::Saved { at: None } => "not saved: no configuration directory".into(),
            Status::Unsaved => "unsaved".into(),
            Status::Saving => "saving\u{2026}".into(),
            Status::Failed(error) => format!("not saved \u{00b7} {error}"),
            Status::Reloaded { at } => format!("reloaded {}", clock(*at)),
            Status::Changed => format!("not saved \u{00b7} {CHANGED_ON_DISK}"),
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

/// A rule [`RulesStore::remove`] took out, for [`RulesStore::restore`].
#[derive(Clone, Debug)]
pub enum Removed {
    Rewrite {
        at: usize,
        entry: Entry<engine::Rewrite>,
        saved: Option<engine::Rewrite>,
    },
    Rule {
        at: usize,
        entry: Entry<engine::Rule>,
        saved: Option<engine::Rule>,
    },
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
    fn precedence_is_an_edit_that_saves_and_survives_other_edits() {
        let dir = scratch("precedence");
        let path = dir.join("rules.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"{ "defaultBrowser": "Firefox", "defaultProfile": "Work",
                 "rules": [{ "match": "a.example/*", "browser": "Safari" }] }"#,
        )
        .unwrap();
        let read = |path: &Path| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let mut store = RulesStore::load(Some(path.clone()));
        assert_eq!(store.precedence(), Precedence::Config);

        // The value it has already: nothing to save.
        store.set_precedence(Precedence::Config);
        assert!(!store.needs_save());

        store.set_precedence(Precedence::PwrRoute);
        assert!(store.needs_save());
        assert_eq!(store.status(), &Status::Unsaved);
        assert_eq!(store.edited().precedence, Precedence::PwrRoute);
        assert_eq!(store.written().precedence, Precedence::PwrRoute);
        store.save().unwrap();
        assert!(!store.needs_save());
        assert_eq!(read(&path)["precedence"], "pwrroute");

        // A later edit keeps it, and so does reading the file again.
        store.set_browser(0, "Arc");
        store.save().unwrap();
        assert_eq!(
            read(&path),
            json!({
                "defaultBrowser": "Firefox",
                "defaultProfile": "Work",
                "precedence": "pwrroute",
                "rules": [{ "match": "a.example/*", "browser": "Arc" }]
            })
        );
        let mut store = RulesStore::load(Some(path.clone()));
        assert_eq!(store.precedence(), Precedence::PwrRoute);

        // Back to the Go order: the field goes.
        store.set_precedence(Precedence::Config);
        store.save().unwrap();
        assert!(read(&path).get("precedence").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_default_browser_is_an_edit_and_the_same_values_change_nothing() {
        let dir = scratch("default-browser");
        let path = dir.join("rules.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            r#"{ "defaultBrowser": "Firefox", "defaultProfile": "Work" }"#,
        )
        .unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        store.set_default_browser("Firefox", "Work");
        assert!(!store.needs_save());

        store.set_default_browser("Google Chrome", "");
        assert!(store.needs_save());
        store.save().unwrap();
        let read: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            read,
            json!({ "defaultBrowser": "Google Chrome", "rules": [] })
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
        store.remove(Selection::Handler(0));
        store.remove(Selection::Rewrite(0));
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
        assert_eq!((badge.letters().as_ref(), name.as_ref()), ("Sf", "Safari"));
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

    /// Another writer's `rules.json`: `n` complete handlers, saved the way
    /// `pwrroute_rules_apply` and the editor save it.
    fn written_by_someone_else(path: &Path, n: usize) {
        RulesFile {
            default_browser: "Safari".into(),
            rules: (0..n)
                .map(|i| engine::Rule {
                    patterns: vec![format!("h{i}.example/*")],
                    browser: "Firefox".into(),
                    ..engine::Rule::default()
                })
                .collect(),
            ..RulesFile::default()
        }
        .save(path)
        .unwrap();
    }

    #[test]
    fn an_external_write_is_reloaded_while_nothing_is_unsaved() {
        let dir = scratch("external-reload");
        let path = dir.join("rules.json");
        written_by_someone_else(&path, 1);
        let mut store = RulesStore::load(Some(path.clone()));
        assert_eq!(store.handler_count(), 1);
        assert_eq!(store.check_disk(), Disk::Unchanged);

        // An agent writes two handlers: the next check reads them.
        written_by_someone_else(&path, 2);
        assert_eq!(store.check_disk(), Disk::Reloaded);
        assert_eq!(store.handler_count(), 2);
        assert_eq!(store.handlers()[1].patterns, ["h1.example/*"]);
        assert!(!store.needs_save());
        assert!(matches!(store.status(), Status::Reloaded { .. }));
        assert!(
            store.status_label().starts_with("reloaded "),
            "{}",
            store.status_label()
        );
        assert_eq!(store.check_disk(), Disk::Unchanged, "read once");

        // The same bytes written again (an editor's save of an unchanged
        // buffer) are not a change.
        written_by_someone_else(&path, 2);
        assert_eq!(store.check_disk(), Disk::Unchanged);

        // A file taken away is a change too: the store follows it.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(store.check_disk(), Disk::Reloaded);
        assert!(store.is_empty() && !store.exists());

        // A file that did not parse is read again once it is fixed.
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(store.check_disk(), Disk::Reloaded);
        assert!(matches!(store.status(), Status::Failed(_)));
        store.add_handler();
        assert!(store.save().is_err(), "never written over");
        written_by_someone_else(&path, 3);
        assert_eq!(store.check_disk(), Disk::Changed, "an edit is unsaved");
        store.reload();
        assert_eq!(store.handler_count(), 3);
        assert!(matches!(store.status(), Status::Reloaded { .. }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_stores_own_save_is_not_a_change_on_disk() {
        let dir = scratch("own-save");
        let path = dir.join("rules.json");
        written_by_someone_else(&path, 1);
        let mut store = RulesStore::load(Some(path.clone()));
        store.set_profile(0, "Work");
        store.save().unwrap();
        assert_eq!(store.check_disk(), Disk::Unchanged);
        assert!(matches!(store.status(), Status::Saved { at: Some(_) }));
        assert_eq!(store.handlers()[0].profile, "Work");

        // A file created by the first save, too.
        let fresh = dir.join("fresh").join("rules.json");
        let mut store = RulesStore::load(Some(fresh.clone()));
        let h = store.add_handler();
        let p = store.add_pattern(Selection::Handler(h)).unwrap();
        store.set_pattern(Selection::Handler(h), p, "a.example/*");
        store.set_browser(h, "Firefox");
        store.save().unwrap();
        assert_eq!(store.check_disk(), Disk::Unchanged);
        assert_eq!(store.handler_count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsaved_edits_are_not_written_over_a_changed_file() {
        let dir = scratch("changed-under-edits");
        let path = dir.join("rules.json");
        written_by_someone_else(&path, 1);
        let mut store = RulesStore::load(Some(path.clone()));
        store.set_profile(0, "Work");
        assert_eq!(store.status(), &Status::Unsaved);

        // The agent's two handlers land while the profile is unsaved.
        written_by_someone_else(&path, 2);
        assert_eq!(store.check_disk(), Disk::Changed);
        assert_eq!(store.status(), &Status::Changed);
        assert_eq!(store.handler_count(), 1, "the edits stay on screen");
        assert_eq!(store.handlers()[0].profile, "Work");
        assert_eq!(
            store.status_label(),
            format!("not saved \u{00b7} {CHANGED_ON_DISK}")
        );
        assert_eq!(store.save(), Err(CHANGED_ON_DISK.to_owned()));
        assert_eq!(store.status(), &Status::Changed);
        assert!(store.needs_save());
        let on_disk = RulesFile::load(&path).unwrap();
        assert_eq!(on_disk.rules.len(), 2, "the agent's rules are intact");
        // Another edit keeps the status.
        store.set_browser(0, "Arc");
        assert_eq!(store.status(), &Status::Changed);

        // A save that never looked at the disk is refused the same way.
        let mut other = RulesStore::load(Some(path.clone()));
        other.set_profile(1, "Home");
        written_by_someone_else(&path, 3);
        assert_eq!(other.save(), Err(CHANGED_ON_DISK.to_owned()));
        assert_eq!(RulesFile::load(&path).unwrap().rules.len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reload_drops_the_edits_and_overwrite_writes_them() {
        let dir = scratch("reload-overwrite");
        let path = dir.join("rules.json");
        written_by_someone_else(&path, 1);

        // Reload: the file wins.
        let mut store = RulesStore::load(Some(path.clone()));
        store.set_profile(0, "Work");
        written_by_someone_else(&path, 2);
        assert_eq!(store.check_disk(), Disk::Changed);
        store.reload();
        assert_eq!(store.handler_count(), 2);
        assert_eq!(store.handlers()[0].profile, "");
        assert!(!store.needs_save());
        assert!(matches!(store.status(), Status::Reloaded { .. }));
        assert_eq!(store.check_disk(), Disk::Unchanged);
        // Editing after a reload is an ordinary unsaved edit.
        store.set_profile(1, "Home");
        assert_eq!(store.status(), &Status::Unsaved);
        store.save().unwrap();
        assert_eq!(RulesFile::load(&path).unwrap().rules[1].profile, "Home");

        // Overwrite: the edits win.
        store.set_profile(0, "Work");
        written_by_someone_else(&path, 3);
        assert_eq!(store.check_disk(), Disk::Changed);
        assert!(store.save().is_err());
        store.overwrite().unwrap();
        assert!(matches!(store.status(), Status::Saved { at: Some(_) }));
        assert!(!store.needs_save());
        let on_disk = RulesFile::load(&path).unwrap();
        assert_eq!(on_disk.rules.len(), 2);
        assert_eq!(on_disk.rules[0].profile, "Work");
        assert_eq!(store.check_disk(), Disk::Unchanged, "its own write");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moves_keep_each_rules_saved_version_and_undo_restores_it() {
        let dir = scratch("move-undo");
        let path = dir.join("rules.json");
        RulesFile {
            rules: ["a.com/*", "b.com/*", "c.com/*"]
                .iter()
                .map(|p| engine::Rule {
                    patterns: vec![(*p).into()],
                    browser: "Firefox".into(),
                    examples: vec![format!("https://{}", p.trim_end_matches("/*"))],
                    ..engine::Rule::default()
                })
                .collect(),
            ..RulesFile::default()
        }
        .save(&path)
        .unwrap();
        let mut store = RulesStore::load(Some(path.clone()));
        let order = |file: &RulesFile| -> Vec<String> {
            file.rules.iter().map(|r| r.patterns[0].clone()).collect()
        };

        // Rule 3 to the top; a move to where it is does nothing.
        assert_eq!(
            store.move_to(Selection::Handler(2), 0),
            Some(Selection::Handler(0))
        );
        assert_eq!(store.move_to(Selection::Handler(1), 1), None);
        assert_eq!(store.move_to(Selection::Handler(9), 0), None);
        store.save().unwrap();
        assert_eq!(
            order(&RulesFile::load(&path).unwrap()),
            ["c.com/*", "a.com/*", "b.com/*"]
        );

        // A rule with a problem keeps its saved version where it moved.
        store.set_browser(1, "");
        store.move_to(Selection::Handler(1), 2);
        store.save().unwrap();
        let file = RulesFile::load(&path).unwrap();
        assert_eq!(order(&file), ["c.com/*", "b.com/*", "a.com/*"]);
        assert_eq!(file.rules[2].browser, "Firefox");
        store.set_browser(2, "Firefox");

        // A copy goes just below, without the examples.
        assert_eq!(
            store.duplicate(Selection::Handler(0)),
            Some(Selection::Handler(1))
        );
        let edited = store.edited();
        assert_eq!(edited.rules[1].patterns, edited.rules[0].patterns);
        assert!(edited.rules[1].examples.is_empty());
        assert_eq!(edited.rules[0].examples.len(), 1);

        // Remove and restore: the same rule at the same place, saved.
        let removed = store.remove(Selection::Handler(2)).unwrap();
        assert_eq!(store.handler_count(), 3);
        assert_eq!(store.restore(removed), Selection::Handler(2));
        store.save().unwrap();
        assert_eq!(
            order(&RulesFile::load(&path).unwrap()),
            ["c.com/*", "c.com/*", "b.com/*", "a.com/*"]
        );
        assert!(
            store
                .rail_label(Selection::Handler(2))
                .ends_with("\u{00b7} saved")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
