//! The resident router inside the app: the command line, one router per
//! user, and the hand-off between the OS, the IPC socket and GPUI.
//!
//! [`start`] runs before GPUI. A second launch forwards its links (or asks
//! for the window) and exits without drawing anything; the first one starts
//! the router and keeps it in [`Resident`] for as long as the app lives.

use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use pwrroute_engine::{DefaultHost, Engine, Limits};
use pwrroute_router::{
    Browser, DispatchRecord, DryRunLauncher, Launcher, Link, Options, Resolver, ResolverFactory,
    Router, RouterHandle, Source,
    instance::{self, Claim, Endpoint, Server},
    ipc::Request,
};

/// How long a second launch waits for the running router to answer.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(3);

/// A dispatch slower than this is reported on stderr, as the Go router did.
const SLOW_DISPATCH_US: u64 = 500_000;

/// What the command line asked for.
#[derive(Debug, Default)]
pub struct Args {
    /// Links to route: a Windows ProgID or Linux `.desktop` launch, or
    /// `pwrroute URL…` from a shell.
    pub links: Vec<String>,
    /// Resolve and record, but open nothing.
    pub dry_run: bool,
    /// A Finicky config to load instead of the default locations.
    pub config: Option<PathBuf>,
}

impl Args {
    pub fn from_env() -> Self {
        Self::parse(env::args_os().collect())
    }

    fn parse(raw: Vec<OsString>) -> Self {
        let mut args = Self {
            links: pwrroute_os::links_from_args(raw.iter().cloned()),
            ..Self::default()
        };
        let mut rest = raw.iter().skip(1).filter_map(|a| a.to_str());
        while let Some(arg) = rest.next() {
            match arg {
                "--dry-run" => args.dry_run = true,
                "--config" => args.config = rest.next().map(PathBuf::from),
                _ => {
                    if let Some(path) = arg.strip_prefix("--config=") {
                        args.config = Some(PathBuf::from(path));
                    }
                }
            }
        }
        args
    }
}

/// How [`start`] went.
pub enum Start {
    /// Another process routes the links; this one is done.
    Exit,
    /// This process is the router.
    Resident(Resident),
}

/// The router and its IPC server, alive until the app quits.
pub struct Resident {
    router: Router,
    _server: Option<Server>,
    /// A second launch asked for the window.
    pub activate: async_channel::Receiver<()>,
    /// The command line carried links, so this launch is for routing, not
    /// for the window.
    pub launched_with_links: bool,
}

impl Resident {
    pub fn handle(&self) -> RouterHandle {
        self.router.handle()
    }
}

/// Becomes the router, or hands the links to the one already running.
pub fn start(args: &Args) -> Start {
    let links: Vec<Link> = args
        .links
        .iter()
        .map(|url| Link::new(url.as_str(), Source::CommandLine))
        .collect();

    let primary = match pwrroute_os::router_dir().map(|dir| instance::claim(&Endpoint::new(dir))) {
        Some(Ok(Claim::Primary(primary))) => Some(primary),
        Some(Ok(Claim::Running(client))) => {
            let request = if links.is_empty() {
                Request::Activate
            } else {
                Request::Open {
                    links: links.clone(),
                }
            };
            match client.send(&request, FORWARD_TIMEOUT) {
                Ok(response) if response.ok => return Start::Exit,
                Ok(response) => eprintln!(
                    "pwrroute: the running router refused the request: {}",
                    response.error.unwrap_or_default()
                ),
                Err(e) => eprintln!("pwrroute: the running router did not answer: {e}"),
            }
            // Never lose a link to a stuck router: route it here, then go.
            if !links.is_empty() {
                route_here_and_exit(args, links);
            }
            return Start::Exit;
        }
        Some(Err(e)) => {
            eprintln!("pwrroute: cannot take the router lock ({e}); running without it");
            None
        }
        None => {
            eprintln!("pwrroute: no data directory; running without the router lock");
            None
        }
    };

    let router = spawn_router(args);
    let handle = router.handle();
    let (activate_tx, activate) = async_channel::unbounded();
    let server = primary.and_then(|primary| {
        let handle = handle.clone();
        primary
            .serve(move |request| {
                handle.answer(request, || {
                    let _ = activate_tx.try_send(());
                })
            })
            .map_err(|e| eprintln!("pwrroute: cannot listen for other launches: {e}"))
            .ok()
    });
    let launched_with_links = !links.is_empty();
    for link in links {
        let _ = handle.dispatch(link);
    }
    Start::Resident(Resident {
        router,
        _server: server,
        activate,
        launched_with_links,
    })
}

/// Links the OS handed over while the app runs: GPUI's open-URLs callback.
pub fn dispatch_os_links(handle: &RouterHandle, urls: Vec<String>) {
    // Read while AppKit is still handling the Apple Event that carried them.
    let opener = pwrroute_os::current_opener();
    let source = if cfg!(target_os = "macos") {
        Source::AppleEvent
    } else {
        Source::CommandLine
    };
    for url in urls {
        let _ = handle.dispatch(Link::new(url, source).with_opener(opener.clone()));
    }
}

/// Routes the links in this process without the lock or a window, waits for
/// each to finish, and returns.
fn route_here_and_exit(args: &Args, links: Vec<Link>) {
    let router = spawn_router(args);
    let handle = router.handle();
    for link in links {
        let _ = handle.dispatch_and_wait(link);
    }
}

fn spawn_router(args: &Args) -> Router {
    let factory = args
        .config
        .clone()
        .or_else(find_finicky_config)
        .map(|path| -> ResolverFactory {
            Box::new(move || {
                let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default())?;
                Ok(Box::new(engine) as Box<dyn Resolver>)
            })
        });
    let launcher: Box<dyn Launcher> = if args.dry_run {
        Box::new(DryRunLauncher::new())
    } else {
        Box::new(UnwiredLauncher)
    };
    let report_all = env::var_os("PWRROUTE_DIAGNOSTICS").is_some_and(|v| v == "1");
    let options = Options {
        on_dispatch: Some(Arc::new(move |record| report_timing(record, report_all))),
        ..Options::default()
    };
    Router::spawn(options, factory, launcher).expect("start the dispatch thread")
}

/// Stands in until the browser launcher crate lands: every dispatch is
/// resolved and recorded, and fails at the launch step with this message.
struct UnwiredLauncher;

impl Launcher for UnwiredLauncher {
    fn launch(&mut self, _browser: &Browser) -> anyhow::Result<()> {
        anyhow::bail!("this build of PwrRoute cannot open browsers yet")
    }
}

/// Slow dispatches, or all with `PWRROUTE_DIAGNOSTICS=1`, as one line of
/// URL-free JSON on stderr.
fn report_timing(record: &DispatchRecord, report_all: bool) {
    if (report_all || record.timings.total_us >= SLOW_DISPATCH_US)
        && let Ok(json) = serde_json::to_string(&record.timing())
    {
        eprintln!("pwrroute dispatch {json}");
    }
}

/// The first Finicky config in the places Finicky looks.
fn find_finicky_config() -> Option<PathBuf> {
    let home = env::home_dir()?;
    finicky_config_candidates(&home)
        .into_iter()
        .find(|p| p.is_file())
}

fn finicky_config_candidates(home: &Path) -> Vec<PathBuf> {
    [
        ".finicky.js",
        ".finicky.ts",
        ".config/finicky.js",
        ".config/finicky.ts",
        ".config/finicky/finicky.js",
        ".config/finicky/finicky.ts",
    ]
    .iter()
    .map(|p| home.join(p))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn the_command_line_carries_links_and_flags() {
        let args = parse(&[
            "pwrroute",
            "--dry-run",
            "--config",
            "/tmp/finicky.ts",
            "https://example.com/a",
        ]);
        assert!(args.dry_run);
        assert_eq!(args.config.as_deref(), Some(Path::new("/tmp/finicky.ts")));
        assert_eq!(args.links, ["https://example.com/a"]);

        let args = parse(&["pwrroute", "--config=/c.js"]);
        assert_eq!(args.config.as_deref(), Some(Path::new("/c.js")));
        assert!(args.links.is_empty() && !args.dry_run);
    }

    #[test]
    fn finicky_configs_are_looked_for_where_finicky_looks() {
        let candidates = finicky_config_candidates(Path::new("/home/me"));
        assert_eq!(candidates[0], Path::new("/home/me/.finicky.js"));
        assert_eq!(candidates.len(), 6);
    }
}
