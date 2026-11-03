//! The seam between the router and whatever opens a browser.

use std::sync::{Arc, Mutex, PoisonError};

use pwrroute_engine::Browser;

/// Opens a resolved link in a browser.
///
/// The router calls it on its dispatch thread, one link at a time, with the
/// engine's [`Browser`]: the app (by name, bundle id or path), the URL after
/// every rewrite, the profile, extra arguments, and whether to open in the
/// background. It is never called for `browser: null`.
pub trait Launcher: Send + 'static {
    fn launch(&mut self, browser: &Browser) -> anyhow::Result<()>;

    /// An independent launcher for a bounded I/O lane. None keeps all
    /// launches serial on one I/O thread. Clones must share any live options.
    fn fork(&self) -> Option<Box<dyn Launcher>> {
        None
    }

    /// Equal keys serialize attempts, across profiles too. Called on the
    /// resolver thread: use only in-memory catalog data, never I/O. Forkable
    /// launchers must canonicalize aliases of the same application.
    fn ordering_key(&self, browser: &Browser) -> String {
        browser.name.to_ascii_lowercase()
    }

    /// Reports progress while launching. Messages must contain only fixed
    /// labels, counts and timings: never URLs, arguments, browser/profile
    /// names or error text. The router adds the link id and logs at Info,
    /// only while diagnostic logging and link retention are enabled.
    fn launch_observed(
        &mut self,
        browser: &Browser,
        _observe: &mut dyn FnMut(&str),
    ) -> anyhow::Result<()> {
        self.launch(browser)
    }

    /// True when nothing really opens. Recorded on each dispatch, so Activity
    /// can say so.
    fn is_dry_run(&self) -> bool {
        false
    }
}

/// Opens nothing and remembers what it was asked to open. Clones share one
/// log, so a test keeps a clone and reads what the router launched.
#[derive(Debug, Clone, Default)]
pub struct DryRunLauncher {
    launched: Arc<Mutex<Vec<Browser>>>,
}

impl DryRunLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything launched so far, oldest first.
    pub fn launched(&self) -> Vec<Browser> {
        self.launched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Launcher for DryRunLauncher {
    fn launch(&mut self, browser: &Browser) -> anyhow::Result<()> {
        self.launched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(browser.clone());
        Ok(())
    }

    fn is_dry_run(&self) -> bool {
        true
    }
}

/// So the app can pick a launcher at runtime (dry run or real).
impl Launcher for Box<dyn Launcher> {
    fn fork(&self) -> Option<Box<dyn Launcher>> {
        (**self).fork()
    }

    fn ordering_key(&self, browser: &Browser) -> String {
        (**self).ordering_key(browser)
    }
    fn launch(&mut self, browser: &Browser) -> anyhow::Result<()> {
        (**self).launch(browser)
    }

    fn is_dry_run(&self) -> bool {
        (**self).is_dry_run()
    }

    fn launch_observed(
        &mut self,
        browser: &Browser,
        observe: &mut dyn FnMut(&str),
    ) -> anyhow::Result<()> {
        (**self).launch_observed(browser, observe)
    }
}
