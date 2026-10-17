//! Hot reload: rebuild the [`Engine`] when its config, the config's imports
//! or `rules.json` change, and swap it in only once the new one has loaded
//! and validated. A failed reload keeps the engine that was working and
//! reports why in [`Status::error`].
//!
//! Change detection polls exact paths, as the Go router did: the config
//! candidates (or the chosen config), every file the config imported, and
//! `rules.json`. Each is fingerprinted by its metadata and contents, so an
//! atomic save (a new inode), an ordinary write and a delete are all seen.
//! No directory is listed or watched.
//!
//! The [`Reloader`] lives on the engine's thread (an engine is not `Send`).
//! Either call [`Reloader::poll`] on a timer, or run a [`Watcher`] thread that
//! calls back when the files change and then call
//! [`Reloader::reload_if_changed`] on the engine's thread.

use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Result, anyhow};

use crate::{Engine, Host, Limits, RulesFile, Sources, discovery};

/// Largest file whose contents go into a fingerprint; larger ones are
/// compared by metadata alone.
const MAX_HASHED_BYTES: u64 = 1024 * 1024;

/// Which JS config is in charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChoice {
    /// The first of these that exists (see [`discovery::config_candidates`]).
    Discover(Vec<PathBuf>),
    /// This file.
    Path(PathBuf),
    /// No JS config: `rules.json` alone ("Stop using" in the design).
    None,
}

impl ConfigChoice {
    /// Discover the config the way Finicky does, from this process's
    /// environment.
    pub fn discover() -> Self {
        Self::Discover(discovery::config_candidates(
            &discovery::Environment::current(),
        ))
    }
}

/// What is in charge, and what went wrong last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The config the active engine was built from.
    pub config: Option<PathBuf>,
    /// The `rules.json` the active engine read, when one is configured.
    pub rules: Option<PathBuf>,
    /// Why the last load failed. While set, the engine is the last one that
    /// worked (or, if none ever did, a fallback that sends everything to
    /// [`FALLBACK_BROWSER`](crate::FALLBACK_BROWSER)).
    pub error: Option<String>,
    /// Bumped each time a new engine is swapped in.
    pub generation: u64,
}

/// The paths a [`Reloader`] wants watched, and their fingerprint when it last
/// loaded them. Shared with a [`Watcher`] thread, which reads both on every
/// poll.
#[derive(Debug, Clone, Default)]
pub struct WatchSet(Arc<Mutex<Watched>>);

#[derive(Debug, Default)]
struct Watched {
    paths: Vec<PathBuf>,
    loaded: u64,
}

impl WatchSet {
    pub fn paths(&self) -> Vec<PathBuf> {
        self.0.lock().map(|w| w.paths.clone()).unwrap_or_default()
    }

    /// The fingerprint of the files the active engine was built from.
    pub fn loaded(&self) -> u64 {
        self.0.lock().map(|w| w.loaded).unwrap_or_default()
    }

    /// Replaces the paths and records their current fingerprint as loaded.
    fn replace(&self, paths: Vec<PathBuf>) -> u64 {
        let loaded = fingerprint_of(&paths);
        if let Ok(mut guard) = self.0.lock() {
            *guard = Watched { paths, loaded };
        }
        loaded
    }

    /// A hash of every watched path's state: missing, or its metadata and
    /// (up to [`MAX_HASHED_BYTES`]) its contents.
    pub fn fingerprint(&self) -> u64 {
        fingerprint_of(&self.paths())
    }
}

fn fingerprint_of(paths: &[PathBuf]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for path in paths {
        path.hash(&mut hasher);
        fingerprint_file(path, &mut hasher);
    }
    hasher.finish()
}

fn fingerprint_file(path: &Path, hasher: &mut DefaultHasher) {
    match fs::metadata(path) {
        Err(e) => e.kind().hash(hasher),
        Ok(meta) => {
            meta.is_file().hash(hasher);
            meta.len().hash(hasher);
            meta.modified().ok().hash(hasher);
            if meta.is_file() && meta.len() <= MAX_HASHED_BYTES {
                fs::read(path).ok().hash(hasher);
            }
        }
    }
}

/// Owns the active [`Engine`] and replaces it when its files change.
pub struct Reloader {
    choice: ConfigChoice,
    rules_path: Option<PathBuf>,
    host: Rc<dyn Host>,
    limits: Limits,
    engine: Engine,
    status: Status,
    watch: WatchSet,
    seen: u64,
    pending: Option<u64>,
}

impl std::fmt::Debug for Reloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reloader")
            .field("choice", &self.choice)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Reloader {
    /// Loads the chosen config and `rules.json`. When that fails, the
    /// reloader still starts, with a fallback engine and the error in
    /// [`Status::error`], so links keep going somewhere explicit.
    pub fn new(
        choice: ConfigChoice,
        rules_path: Option<PathBuf>,
        host: Rc<dyn Host>,
        limits: Limits,
    ) -> Result<Self> {
        let fallback = Engine::from_sources(&Sources::default(), Rc::clone(&host), limits)?;
        let mut reloader = Self {
            choice,
            rules_path,
            host,
            limits,
            engine: fallback,
            status: Status::default(),
            watch: WatchSet::default(),
            seen: 0,
            pending: None,
        };
        let _ = reloader.reload();
        Ok(reloader)
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn choice(&self) -> &ConfigChoice {
        &self.choice
    }

    /// The paths to watch, for a [`Watcher`]. Updated after every reload.
    pub fn watch_set(&self) -> WatchSet {
        self.watch.clone()
    }

    /// Switches the config in charge and reloads.
    pub fn set_choice(&mut self, choice: ConfigChoice) -> Result<()> {
        self.choice = choice;
        self.reload()
    }

    /// Reloads when the watched files changed and have stayed unchanged for
    /// one call, so an editor's burst of writes reloads once. Returns `None`
    /// when nothing was reloaded. Call it on a timer (the Go host polled every
    /// 500 ms).
    pub fn poll(&mut self) -> Option<Result<()>> {
        let now = self.watch.fingerprint();
        if now == self.seen {
            self.pending = None;
            return None;
        }
        if self.pending != Some(now) {
            self.pending = Some(now);
            return None;
        }
        self.pending = None;
        Some(self.reload())
    }

    /// Reloads when the watched files differ from the last load. For use
    /// after a [`Watcher`] reported a settled change.
    pub fn reload_if_changed(&mut self) -> Option<Result<()>> {
        if self.watch.fingerprint() == self.seen {
            return None;
        }
        Some(self.reload())
    }

    /// Builds a new engine from the files as they are now and swaps it in.
    /// On failure the active engine stays and the error is returned and kept
    /// in [`Status::error`].
    pub fn reload(&mut self) -> Result<()> {
        let result = self.try_load();
        let mut watched = match &self.choice {
            ConfigChoice::Discover(candidates) => candidates.clone(),
            ConfigChoice::Path(path) => vec![path.clone()],
            ConfigChoice::None => Vec::new(),
        };
        let outcome = match result {
            Ok((engine, config)) => {
                self.engine = engine;
                self.status = Status {
                    config,
                    rules: self.rules_path.clone(),
                    error: None,
                    generation: self.status.generation + 1,
                };
                Ok(())
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.status.error = Some(message.clone());
                Err(anyhow!(message))
            }
        };
        for path in self.engine.dependencies() {
            if !watched.contains(&path) {
                watched.push(path);
            }
        }
        if let Some(rules) = &self.rules_path {
            watched.push(rules.clone());
        }
        self.seen = self.watch.replace(watched);
        outcome
    }

    fn try_load(&self) -> Result<(Engine, Option<PathBuf>)> {
        let config = match &self.choice {
            ConfigChoice::Path(path) => Some(path.clone()),
            ConfigChoice::None => None,
            ConfigChoice::Discover(candidates) => match discovery::find_config(candidates) {
                Some(path) => Some(path),
                // A config that was in charge and is gone may be mid-save,
                // or deleted by mistake: keep it, as the Go host did, until
                // the user chooses otherwise.
                None => {
                    if let Some(gone) = &self.status.config {
                        return Err(anyhow!(
                            "{} is gone; keeping the last working config",
                            gone.display()
                        ));
                    }
                    None
                }
            },
        };
        let rules = match &self.rules_path {
            Some(path) => RulesFile::load(path)?,
            None => RulesFile::default(),
        };
        let sources = Sources {
            config: config.clone(),
            rules,
        };
        let engine = Engine::from_sources(&sources, Rc::clone(&self.host), self.limits)?;
        let config = engine.config_path().map(Path::to_path_buf);
        Ok((engine, config))
    }
}

/// A thread that polls a [`WatchSet`] and calls back once the files differ
/// from what the [`Reloader`] last loaded and two polls agree on them (the
/// change has settled). It calls back once per settled state, so the
/// callback can simply wake the engine's thread. Dropping it stops the
/// thread.
pub struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

impl Watcher {
    pub fn spawn(
        watch: WatchSet,
        interval: Duration,
        on_change: impl Fn() + Send + 'static,
    ) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("pwrroute-config-watch".into())
            .spawn(move || {
                let mut previous = None;
                let mut reported = None;
                while !stopped.load(Ordering::Relaxed) {
                    let now = watch.fingerprint();
                    let settled = previous == Some(now);
                    if settled && now != watch.loaded() && reported != Some(now) {
                        reported = Some(now);
                        on_change();
                    }
                    previous = Some(now);
                    thread::park_timeout(interval);
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
