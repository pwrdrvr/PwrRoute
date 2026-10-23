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
    /// worked. A config that no engine has held yet (it never loaded, or it
    /// appeared broken while `rules.json` was in charge) is left out instead:
    /// the engine is `rules.json` alone, so its rules keep routing until the
    /// config loads, and `config` is `None`. If `rules.json` does not load
    /// either and nothing ever did, the engine is a fallback that sends
    /// everything to [`FALLBACK_BROWSER`](crate::FALLBACK_BROWSER).
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
    /// `stale` means the files changed while they were being loaded, so the
    /// recorded fingerprint is made to differ from theirs and the next check
    /// reloads.
    fn replace(&self, paths: Vec<PathBuf>, stale: bool) -> u64 {
        let mut loaded = fingerprint_of(&paths);
        if stale {
            loaded = loaded.wrapping_add(1);
        }
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
    /// Loads the chosen config and `rules.json`. When the config fails, the
    /// reloader still starts, with `rules.json` alone in charge (or, if that
    /// fails too, a fallback engine) and the error in [`Status::error`], so
    /// links keep going somewhere explicit.
    pub fn new(
        choice: ConfigChoice,
        rules_path: Option<PathBuf>,
        host: Rc<dyn Host>,
        limits: Limits,
    ) -> Result<Self> {
        let Attempt {
            result,
            rules_alone,
            read,
            stale,
        } = attempt(&choice, rules_path.as_deref(), &host, limits, None, &[]);
        let (loaded, result) = match result {
            Ok(engine) => (Some(engine), Ok(())),
            Err(e) => (rules_alone, Err(e)),
        };
        let swapped = loaded.is_some();
        let engine = match loaded {
            Some(engine) => engine,
            // The fallback is built only when it is needed.
            None => Engine::from_sources(&Sources::default(), Rc::clone(&host), limits)?,
        };
        let mut reloader = Self {
            choice,
            rules_path,
            host,
            limits,
            engine,
            status: Status::default(),
            watch: WatchSet::default(),
            seen: 0,
            pending: None,
        };
        let _ = reloader.finish(swapped, result, read, stale);
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
    /// in [`Status::error`], unless no config was in charge: then a config
    /// that fails is left out and `rules.json` alone is swapped in, so an
    /// edit to it still applies while the config is broken.
    pub fn reload(&mut self) -> Result<()> {
        let Attempt {
            result,
            rules_alone,
            read,
            stale,
        } = attempt(
            &self.choice,
            self.rules_path.as_deref(),
            &self.host,
            self.limits,
            self.status.config.as_deref(),
            &self.engine.dependencies(),
        );
        let (loaded, result) = match result {
            Ok(engine) => (Some(engine), Ok(())),
            Err(e) => (rules_alone, Err(e)),
        };
        let swapped = loaded.is_some();
        if let Some(engine) = loaded {
            self.engine = engine;
        }
        self.finish(swapped, result, read, stale)
    }

    /// Records a load attempt: `swapped` says a new engine is in place (the
    /// one asked for, or `rules.json` alone when the config failed), the
    /// status follows it and `result`, and the choice's paths, what the
    /// active engine and the attempt read, and `rules.json` are watched.
    fn finish(
        &mut self,
        swapped: bool,
        result: Result<()>,
        read: Vec<PathBuf>,
        stale: bool,
    ) -> Result<()> {
        if swapped {
            self.status.config = self.engine.config_path().map(Path::to_path_buf);
            self.status.rules = self.rules_path.clone();
            self.status.generation += 1;
        }
        let outcome = match result {
            Ok(()) => {
                self.status.error = None;
                Ok(())
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.status.error = Some(message.clone());
                Err(anyhow!(message))
            }
        };
        let mut watched = match &self.choice {
            ConfigChoice::Discover(candidates) => candidates.clone(),
            ConfigChoice::Path(path) => vec![path.clone()],
            ConfigChoice::None => Vec::new(),
        };
        // A failed attempt's imports are watched too, so fixing a broken
        // helper reloads.
        for path in self.engine.dependencies().into_iter().chain(read) {
            if !watched.contains(&path) {
                watched.push(path);
            }
        }
        if let Some(rules) = &self.rules_path {
            watched.push(rules.clone());
        }
        self.seen = self.watch.replace(watched, stale);
        outcome
    }
}

/// One load attempt.
struct Attempt {
    result: Result<Engine>,
    /// When `result` failed on a config that no engine held (none was in
    /// charge): an engine over `rules.json` alone, to route with until the
    /// config loads. `None` when `rules.json` does not load either.
    rules_alone: Option<Engine>,
    /// The config and the files it imported, read whether or not it loaded.
    read: Vec<PathBuf>,
    /// A file the load read changed while it ran, so the engine may hold an
    /// older version than the files now on disk.
    stale: bool,
}

/// Loads the files as they are now. `in_charge` is the config the active
/// engine was built from; `previously_read` is what it read, used to notice
/// writes that land during this load.
fn attempt(
    choice: &ConfigChoice,
    rules_path: Option<&Path>,
    host: &Rc<dyn Host>,
    limits: Limits,
    in_charge: Option<&Path>,
    previously_read: &[PathBuf],
) -> Attempt {
    let config = match choice {
        ConfigChoice::Path(path) => Some(path.clone()),
        ConfigChoice::None => None,
        ConfigChoice::Discover(candidates) => match discovery::find_config(candidates) {
            Some(path) => Some(path),
            // A config that was in charge and is gone may be mid-save, or
            // deleted by mistake: keep it, as the Go host did, until the user
            // chooses otherwise.
            None => {
                if let Some(gone) = in_charge {
                    return Attempt {
                        result: Err(anyhow!(
                            "{} is gone; keeping the last working config",
                            gone.display()
                        )),
                        rules_alone: None,
                        read: Vec::new(),
                        stale: false,
                    };
                }
                None
            }
        },
    };
    let mut known: Vec<PathBuf> = previously_read.to_vec();
    known.extend(config.iter().cloned());
    known.extend(rules_path.map(Path::to_path_buf));
    let before = fingerprint_of(&known);

    let imports = Rc::default();
    let load = |config: Option<PathBuf>, imports: &Rc<_>| {
        let rules = match rules_path {
            Some(path) => RulesFile::load(path)?,
            None => RulesFile::default(),
        };
        let sources = Sources { config, rules };
        Engine::from_sources_reading(&sources, Rc::clone(host), limits, imports)
    };
    let result = load(config.clone(), &imports);
    // A config no engine holds yet that fails is left out rather than
    // taking rules.json down with it. One that was in charge is not: its
    // last load keeps routing, rules.json with it.
    let rules_alone = (result.is_err() && config.is_some() && in_charge.is_none())
        .then(|| load(None, &Rc::default()).ok())
        .flatten();
    let mut read: Vec<PathBuf> = config.into_iter().collect();
    read.extend(imports.borrow().iter().cloned());
    Attempt {
        result,
        rules_alone,
        read,
        stale: fingerprint_of(&known) != before,
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
