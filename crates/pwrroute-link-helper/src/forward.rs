//! What the helper does with a link, apart from AppKit: hand it to the
//! running router, or launch the app with it. Plain Rust over two seams
//! ([`Router`], [`Apps`]), so every decision is tested on every OS.

use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use pwrroute_router::{
    Link, OpenerInfo, Source,
    instance::{Client, Endpoint},
    ipc::Request,
};

/// After the helper launched the app, how long a later link waits for that
/// app's router to listen before the helper launches another copy. A cold
/// start takes well under a second; this bounds one that never comes up.
pub const STARTUP_WAIT: Duration = Duration::from_secs(5);

/// The bundle id the helper opens a link in when even launching PwrRoute
/// fails: the router's own last resort on macOS (`FALLBACK_BROWSER`).
pub const LAST_RESORT: &str = "com.apple.Safari";

/// The links of one Apple Event, from the app that sent it.
pub fn links(urls: Vec<String>, opener: Option<OpenerInfo>) -> Vec<Link> {
    urls.into_iter()
        .filter(|url| !url.trim().is_empty())
        .map(|url| Link::new(url, Source::AppleEvent).with_opener(opener.clone()))
        .collect()
}

/// `PwrRoute.app` for the helper at `helper`
/// (`PwrRoute.app/Contents/Helpers/PwrRoute.app`): `None` when the helper is
/// not nested where `cargo xtask bundle` puts it.
pub fn main_app_for(helper: &Path) -> Option<PathBuf> {
    let helpers = helper.parent()?;
    let contents = helpers.parent()?;
    let app = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|n| n == name);
    let is_app = |path: &Path| path.extension().is_some_and(|e| e == "app");
    (is_app(helper) && named(helpers, "Helpers") && named(contents, "Contents") && is_app(app))
        .then(|| app.to_path_buf())
}

/// The running router, as a second launch of the app reaches it.
pub trait Router {
    /// Sends `request`, retrying the connection for up to `wait`. `Ok` only
    /// when the router accepted it.
    fn send(&self, request: &Request, wait: Duration) -> io::Result<()>;
}

/// The apps the helper may start.
pub trait Apps {
    /// Launches PwrRoute.app with `args`, a new copy even if one is running
    /// (a stuck router must not swallow the arguments), in front only when
    /// `activate`. `Err` when the OS could not launch it.
    fn launch_app(&self, args: &[String], activate: bool) -> Result<(), String>;
    /// Opens `url` in the app with `bundle_id`, named explicitly so it can
    /// never come back to the helper.
    fn open_in(&self, bundle_id: &str, url: &str) -> Result<(), String>;
}

/// The router at `pwrroute_os::router_dir()`, through the client a second
/// launch uses. No directory means no router.
pub struct SocketRouter(Option<Client>);

impl SocketRouter {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self(dir.map(|dir| Client::new(Endpoint::new(dir))))
    }
}

impl Router for SocketRouter {
    fn send(&self, request: &Request, wait: Duration) -> io::Result<()> {
        let Some(client) = &self.0 else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no directory for the router",
            ));
        };
        let response = client.send(request, wait)?;
        if response.ok {
            Ok(())
        } else {
            Err(io::Error::other(
                response
                    .error
                    .unwrap_or_else(|| "the router refused the request".into()),
            ))
        }
    }
}

/// What happened to a request. Never carries a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The running router took it.
    Forwarded,
    /// No router took it, so the app was launched with it.
    Launched,
    /// The app could not be launched either: each link went to
    /// [`LAST_RESORT`].
    LastResort,
    /// Nothing could be done.
    Failed,
}

/// The helper's one piece of state: when it last launched the app, so the
/// links right behind wait for that app instead of launching more.
pub struct Forwarder<R, A> {
    router: R,
    apps: A,
    launched_at: Option<Instant>,
}

impl<R: Router, A: Apps> Forwarder<R, A> {
    pub fn new(router: R, apps: A) -> Self {
        Self {
            router,
            apps,
            launched_at: None,
        }
    }

    /// How long to wait for the router to listen: nothing when it should be
    /// up already (an absent router answers a connect at once), the rest of
    /// [`STARTUP_WAIT`] while an app this helper launched is starting.
    pub fn connect_wait(&self, now: Instant) -> Duration {
        self.launched_at
            .map(|at| STARTUP_WAIT.saturating_sub(now.saturating_duration_since(at)))
            .unwrap_or(Duration::ZERO)
    }

    /// Routes the links of one Apple Event. The running router gets them as
    /// a second launch's would send them, with their source and opener.
    /// When no router answers (PwrRoute is not running, or is stuck), the
    /// app is launched in the background with the links and their opener
    /// on its command line: it routes them without opening its window,
    /// becoming the router or, if a stuck one holds the lock, routing them
    /// itself. A link is lost only if macOS can open neither PwrRoute nor
    /// Safari.
    pub fn open(&mut self, links: Vec<Link>, now: Instant) -> Outcome {
        if links.is_empty() {
            return Outcome::Forwarded;
        }
        let request = Request::Open {
            links: links.clone(),
        };
        if self.router.send(&request, self.connect_wait(now)).is_ok() {
            return Outcome::Forwarded;
        }
        let args = pwrroute_os::apple_event_launch_args(&links);
        if self.apps.launch_app(&args, false).is_ok() {
            self.launched_at = Some(now);
            return Outcome::Launched;
        }
        let mut opened = false;
        for link in &links {
            opened |= self.apps.open_in(LAST_RESORT, &link.url).is_ok();
        }
        if opened {
            Outcome::LastResort
        } else {
            Outcome::Failed
        }
    }

    /// The helper was opened by hand, not for a link: show PwrRoute's
    /// window, as a second launch of the app with no links does.
    pub fn activate(&mut self, now: Instant) -> Outcome {
        if self
            .router
            .send(&Request::Activate, self.connect_wait(now))
            .is_ok()
        {
            return Outcome::Forwarded;
        }
        match self.apps.launch_app(&[], true) {
            Ok(()) => {
                self.launched_at = Some(now);
                Outcome::Launched
            }
            Err(_) => Outcome::Failed,
        }
    }
}

/// Whether a reopen at `reopened` asks for the window. LaunchServices can
/// send one beside a link's Apple Event, before or after it; one this close
/// to a link belongs to it (the app's own `REOPEN_AFTER_LINK`).
pub fn reopen_wants_window(last_link: Option<Instant>, reopened: Instant) -> bool {
    const NEAR_A_LINK: Duration = Duration::from_millis(750);
    last_link.is_none_or(|link| {
        let apart = reopened
            .checked_duration_since(link)
            .or_else(|| link.checked_duration_since(reopened))
            .unwrap_or_default();
        apart >= NEAR_A_LINK
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    /// A router that answers by its `up` flag and records each request
    /// and the wait it was given.
    #[derive(Clone, Default)]
    struct FakeRouter {
        up: Rc<RefCell<bool>>,
        sent: Rc<RefCell<Vec<(Request, Duration)>>>,
    }

    impl Router for FakeRouter {
        fn send(&self, request: &Request, wait: Duration) -> io::Result<()> {
            self.sent.borrow_mut().push((request.clone(), wait));
            if *self.up.borrow() {
                Ok(())
            } else {
                Err(io::ErrorKind::ConnectionRefused.into())
            }
        }
    }

    /// One launch: its arguments, and whether it activated.
    type Launch = (Vec<String>, bool);

    #[derive(Clone, Default)]
    struct FakeApps {
        launch_fails: Rc<RefCell<bool>>,
        launched: Rc<RefCell<Vec<Launch>>>,
        opened: Rc<RefCell<Vec<(String, String)>>>,
    }

    impl Apps for FakeApps {
        fn launch_app(&self, args: &[String], activate: bool) -> Result<(), String> {
            if *self.launch_fails.borrow() {
                return Err("no".into());
            }
            self.launched.borrow_mut().push((args.to_vec(), activate));
            Ok(())
        }

        fn open_in(&self, bundle_id: &str, url: &str) -> Result<(), String> {
            self.opened
                .borrow_mut()
                .push((bundle_id.to_owned(), url.to_owned()));
            Ok(())
        }
    }

    fn mail() -> OpenerInfo {
        OpenerInfo {
            name: "Mail".into(),
            bundle_id: "com.apple.mail".into(),
            path: "/System/Applications/Mail.app".into(),
            window_title: None,
        }
    }

    #[test]
    fn an_events_links_carry_its_source_and_opener() {
        let built = links(
            vec![
                "https://example.com/a".into(),
                " ".into(),
                "https://example.org/b".into(),
            ],
            Some(mail()),
        );
        assert_eq!(
            built,
            [
                Link::new("https://example.com/a", Source::AppleEvent).with_opener(Some(mail())),
                Link::new("https://example.org/b", Source::AppleEvent).with_opener(Some(mail())),
            ]
        );
        assert!(links(Vec::new(), None).is_empty());
    }

    #[test]
    fn a_running_router_gets_the_links_at_once() {
        let router = FakeRouter::default();
        *router.up.borrow_mut() = true;
        let apps = FakeApps::default();
        let mut forwarder = Forwarder::new(router.clone(), apps.clone());
        let links = links(vec!["https://example.com/a".into()], Some(mail()));
        assert_eq!(
            forwarder.open(links.clone(), Instant::now()),
            Outcome::Forwarded
        );
        assert_eq!(
            *router.sent.borrow(),
            [(Request::Open { links }, Duration::ZERO)]
        );
        assert!(apps.launched.borrow().is_empty());
    }

    #[test]
    fn no_router_launches_the_app_in_the_background_with_the_links() {
        let router = FakeRouter::default();
        let apps = FakeApps::default();
        let mut forwarder = Forwarder::new(router.clone(), apps.clone());
        let now = Instant::now();
        let links = links(vec!["https://example.com/a".into()], Some(mail()));
        assert_eq!(forwarder.open(links.clone(), now), Outcome::Launched);
        let launched = apps.launched.borrow();
        let (args, activate) = &launched[0];
        assert!(!activate, "a link never brings the window forward");
        assert_eq!(*args, pwrroute_os::apple_event_launch_args(&links));
        assert_eq!(args[0], "https://example.com/a");
        assert!(args.iter().any(|a| a == "--source=apple-event"));
        assert!(args.iter().any(|a| a.starts_with("--opener=")));
    }

    #[test]
    fn links_behind_a_launch_wait_for_that_app() {
        let router = FakeRouter::default();
        let apps = FakeApps::default();
        let mut forwarder = Forwarder::new(router.clone(), apps.clone());
        let start = Instant::now();
        assert_eq!(forwarder.connect_wait(start), Duration::ZERO);
        let one = || links(vec!["https://example.com/a".into()], None);
        assert_eq!(forwarder.open(one(), start), Outcome::Launched);

        // A second later the app is listening: the link waits for it.
        let later = start + Duration::from_secs(1);
        assert_eq!(
            forwarder.connect_wait(later),
            STARTUP_WAIT - Duration::from_secs(1)
        );
        *router.up.borrow_mut() = true;
        assert_eq!(forwarder.open(one(), later), Outcome::Forwarded);
        assert_eq!(
            router.sent.borrow()[1].1,
            STARTUP_WAIT - Duration::from_secs(1)
        );
        assert_eq!(apps.launched.borrow().len(), 1);

        // Long after, the router is expected up: no wait, and a router gone
        // since means another launch.
        let much_later = start + STARTUP_WAIT * 3;
        assert_eq!(forwarder.connect_wait(much_later), Duration::ZERO);
        *router.up.borrow_mut() = false;
        assert_eq!(forwarder.open(one(), much_later), Outcome::Launched);
        assert_eq!(apps.launched.borrow().len(), 2);
    }

    #[test]
    fn a_link_is_not_lost_when_the_app_cannot_launch() {
        let router = FakeRouter::default();
        let apps = FakeApps::default();
        *apps.launch_fails.borrow_mut() = true;
        let mut forwarder = Forwarder::new(router, apps.clone());
        let links = links(
            vec![
                "https://example.com/a".into(),
                "https://example.org/b".into(),
            ],
            None,
        );
        assert_eq!(forwarder.open(links, Instant::now()), Outcome::LastResort);
        assert_eq!(
            *apps.opened.borrow(),
            [
                (LAST_RESORT.to_owned(), "https://example.com/a".to_owned()),
                (LAST_RESORT.to_owned(), "https://example.org/b".to_owned()),
            ]
        );
        assert_eq!(forwarder.connect_wait(Instant::now()), Duration::ZERO);
    }

    #[test]
    fn opening_the_helper_by_hand_shows_the_window() {
        let router = FakeRouter::default();
        let apps = FakeApps::default();
        let mut forwarder = Forwarder::new(router.clone(), apps.clone());
        assert_eq!(forwarder.activate(Instant::now()), Outcome::Launched);
        assert_eq!(*apps.launched.borrow(), [(Vec::new(), true)]);
        *router.up.borrow_mut() = true;
        assert_eq!(forwarder.activate(Instant::now()), Outcome::Forwarded);
        assert_eq!(router.sent.borrow().last().unwrap().0, Request::Activate);
    }

    #[test]
    fn the_app_is_found_from_the_nested_helper() {
        assert_eq!(
            main_app_for(Path::new(
                "/Applications/PwrRoute.app/Contents/Helpers/PwrRoute.app"
            )),
            Some(PathBuf::from("/Applications/PwrRoute.app"))
        );
        for loose in [
            "/Applications/PwrRoute.app",
            "/tmp/Helpers/PwrRoute.app",
            "/Applications/PwrRoute.app/Contents/MacOS/PwrRoute.app",
            "/Applications/PwrRoute/Contents/Helpers/PwrRoute.app",
            "/Applications/PwrRoute.app/Contents/Helpers/pwrroute-links",
        ] {
            assert_eq!(main_app_for(Path::new(loose)), None, "{loose}");
        }
    }

    #[test]
    fn a_reopen_beside_a_link_is_part_of_it() {
        let now = Instant::now();
        assert!(reopen_wants_window(None, now));
        assert!(!reopen_wants_window(
            Some(now),
            now + Duration::from_millis(100)
        ));
        let link = now + Duration::from_millis(100);
        assert!(!reopen_wants_window(Some(link), now), "a link just after");
        assert!(reopen_wants_window(Some(now), now + Duration::from_secs(2)));
    }

    #[test]
    fn the_socket_router_without_a_directory_is_absent() {
        let err = SocketRouter::new(None)
            .send(&Request::Ping, Duration::ZERO)
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    /// The helper's request through the real socket client, to a real
    /// router endpoint in a temp directory: what a running PwrRoute reads.
    #[test]
    fn the_socket_router_speaks_the_routers_protocol() {
        use pwrroute_router::{
            instance::{Claim, claim},
            ipc::Response,
        };
        use std::sync::{Mutex, mpsc};

        let dir = std::env::temp_dir().join(format!("plh{}", std::process::id()));
        let endpoint = Endpoint::new(dir.join("r"));
        let Ok(Claim::Primary(primary)) = claim(&endpoint) else {
            panic!("the temp endpoint is ours");
        };
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let server = primary
            .serve(move |request| {
                tx.lock().unwrap().send(request).unwrap();
                Response::ok()
            })
            .unwrap();
        let router = SocketRouter::new(Some(endpoint.dir().to_path_buf()));
        let links = links(vec!["https://example.com/a".into()], Some(mail()));
        router
            .send(
                &Request::Open {
                    links: links.clone(),
                },
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(rx.recv().unwrap(), Request::Open { links });
        drop(server);
        // Gone: refused at once, which is what sends the helper to launch.
        assert!(router.send(&Request::Ping, Duration::ZERO).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
