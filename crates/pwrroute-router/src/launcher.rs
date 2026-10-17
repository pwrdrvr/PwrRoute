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
    fn launch(&mut self, browser: &Browser) -> anyhow::Result<()> {
        (**self).launch(browser)
    }

    fn is_dry_run(&self) -> bool {
        (**self).is_dry_run()
    }
}
