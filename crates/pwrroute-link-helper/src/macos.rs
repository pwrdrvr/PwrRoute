//! The helper's AppKit side: an accessory `NSApplication` whose delegate
//! takes `application:openURLs:` (AppKit's own `kAEGetURL` handler calls
//! it), and the worker thread that forwards each event's links in order.
//!
//! The delegate class is the only `unsafe` here: objc2's `define_class!`
//! takes an `unsafe` superclass and method attributes, and its initializer
//! is a `super` message, because the compiler cannot check an Objective-C
//! class's declaration against the runtime. Everything else is objc2's safe
//! bindings.
#![allow(unsafe_code)]

use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationLaunchIsDefaultLaunchKey, NSRunningApplication, NSWorkspace,
    NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{
    NSArray, NSBundle, NSError, NSNotification, NSNumber, NSObject, NSObjectProtocol, NSString,
    NSURL,
};
use pwrroute_router::Link;

use crate::{
    forward::{self, Apps, Forwarder, Outcome, SocketRouter},
    replaced::SelfWatch,
};

/// How long a launch by hand waits for a link before it counts as a request
/// for the window. The app waits as long (`LINK_GRACE`, 250 ms); the helper
/// also has AppKit's "default launch" flag, so this is only a backstop.
const LINK_GRACE: Duration = Duration::from_millis(500);

/// How long the helper waits for macOS to say whether it launched the app.
/// Past it the launch is taken as under way, so a slow one is not doubled.
const LAUNCH_ANSWER: Duration = Duration::from_secs(10);

/// What the worker thread does, in the order the events arrived.
enum Job {
    Open(Vec<Link>),
    Activate,
}

/// When the helper last received a link, shared with the threads that
/// decide whether a launch or a reopen was for the window.
#[derive(Clone, Default)]
struct LastLink(Arc<Mutex<Option<Instant>>>);

impl LastLink {
    fn mark(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    fn get(&self) -> Option<Instant> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct Ivars {
    jobs: Sender<Job>,
    last_link: LastLink,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the delegate
    // runs on the main thread only (MainThreadOnly), as AppKit calls it.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PwrRouteLinkHelperDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: each method's selector and signature are NSApplicationDelegate's
    // (the generated protocol in objc2-app-kit).
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, notification: &NSNotification) {
            // Opened by hand (Finder, Spotlight): show PwrRoute, as the app
            // does when launched with no link. A launch for a link is not a
            // "default launch", and its link arrives right after this.
            // Started by the app to be ready for the first link: not a
            // request for the window either.
            if is_default_launch(notification) && !started_by_app() {
                self.activate_unless_a_link_comes(Instant::now(), LINK_GRACE);
            }
        }

        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            // Read while AppKit still handles the Apple Event that carried
            // them: it names the sender.
            let opener = pwrroute_os::current_opener();
            let urls = urls
                .iter()
                .filter_map(|url| url.absoluteString())
                .map(|url| url.to_string())
                .collect();
            self.ivars().last_link.mark();
            let _ = self
                .ivars()
                .jobs
                .send(Job::Open(forward::links(urls, opener)));
        }

        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _application: &NSApplication, _visible: bool) -> bool {
            self.activate_unless_a_link_comes(Instant::now(), Duration::from_millis(250));
            false
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, ivars: Ivars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: NSObject's designated initializer, sent to the superclass
        // of a freshly allocated instance with its ivars set.
        unsafe { msg_send![super(this), init] }
    }

    /// Asks for the window `after` from now, unless a link arrives close
    /// to `at`: LaunchServices can send a reopen beside a link.
    fn activate_unless_a_link_comes(&self, at: Instant, after: Duration) {
        let jobs = self.ivars().jobs.clone();
        let last_link = self.ivars().last_link.clone();
        let _ = thread::Builder::new()
            .name("pwrroute-links-reopen".into())
            .spawn(move || {
                thread::sleep(after);
                if forward::reopen_wants_window(last_link.get(), at) {
                    let _ = jobs.send(Job::Activate);
                }
            });
    }
}

/// Whether the app started this helper ([`pwrroute_os::start_link_helper`]).
fn started_by_app() -> bool {
    std::env::args().any(|arg| arg == pwrroute_os::LINK_HELPER_STARTED_BY_APP)
}

/// `NSApplicationLaunchIsDefaultLaunchKey` in the launch notification:
/// `false` when the app was launched to open something.
fn is_default_launch(notification: &NSNotification) -> bool {
    // SAFETY: an AppKit constant, initialized before any app code runs.
    let key = unsafe { NSApplicationLaunchIsDefaultLaunchKey };
    notification
        .userInfo()
        .and_then(|info| info.objectForKey(key))
        .and_then(|value| value.downcast::<NSNumber>().ok())
        .is_some_and(|value| value.boolValue())
}

/// Runs the helper until it is killed, the session ends, or its executable
/// is replaced or removed (`work`).
pub fn run() {
    let mtm = MainThreadMarker::new().expect("the helper starts on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // LSUIElement says the same; set here too, so a bare binary run by hand
    // never shows in the Dock either.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let bundle = NSBundle::mainBundle();
    let helper = PathBuf::from(bundle.bundlePath().to_string());
    // Noted before the first request, so an update that lands while the
    // helper runs reads as one.
    let watch = SelfWatch::new(
        bundle
            .executablePath()
            .map(|path| PathBuf::from(path.to_string())),
    );
    let (jobs, queue) = mpsc::channel();
    thread::Builder::new()
        .name("pwrroute-links".into())
        .spawn(move || {
            work(
                &queue,
                Workspace::new(forward::main_app_for(&helper)),
                &watch,
            )
        })
        .expect("start the helper's worker thread");
    let delegate = Delegate::new(
        mtm,
        Ivars {
            jobs,
            last_link: LastLink::default(),
        },
    );
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    app.run();
}

/// How often an idle helper checks whether its executable is still there.
/// A helper whose app was moved to the Trash gets no more links (macOS can
/// no longer find its bundle id), so only this ends it before logout.
const IDLE_CHECK: Duration = Duration::from_secs(30);

/// Forwards each job in turn. Off the main thread, so a slow router or
/// launch never holds up the next Apple Event; one thread, so links keep
/// their order. After each job, and every [`IDLE_CHECK`] while idle, if an
/// update or a move took the helper's executable away, forwards what is
/// already queued and exits: the next link then starts the helper that is
/// installed now.
fn work(queue: &Receiver<Job>, apps: Workspace, watch: &SelfWatch) {
    let mut forwarder = Forwarder::new(SocketRouter::new(pwrroute_os::router_dir()), apps);
    loop {
        match queue.recv_timeout(IDLE_CHECK) {
            Ok(job) => run_job(&mut forwarder, job),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if watch.replaced() {
            while let Ok(job) = queue.try_recv() {
                run_job(&mut forwarder, job);
            }
            eprintln!("pwrroute-links: this helper was replaced or removed; exiting");
            std::process::exit(0);
        }
    }
}

fn run_job(forwarder: &mut Forwarder<SocketRouter, Workspace>, job: Job) {
    let outcome = match job {
        Job::Open(links) => forwarder.open(links, Instant::now()),
        Job::Activate => forwarder.activate(Instant::now()),
    };
    // Fixed words only: never a link.
    match outcome {
        Outcome::Forwarded => {}
        Outcome::Launched => eprintln!("pwrroute-links: no router answered; launched PwrRoute"),
        Outcome::LastResort => {
            eprintln!("pwrroute-links: PwrRoute could not be launched; opened the link in Safari")
        }
        Outcome::Failed => eprintln!("pwrroute-links: could not hand the request on"),
    }
}

/// PwrRoute.app and Safari, opened through NSWorkspace and `open`.
struct Workspace {
    app: Option<PathBuf>,
}

impl Workspace {
    fn new(app: Option<PathBuf>) -> Self {
        Self { app }
    }
}

impl Apps for Workspace {
    fn launch_app(&self, args: &[String], activate: bool) -> Result<(), String> {
        let Some(app) = &self.app else {
            return Err("the helper is not inside PwrRoute.app".into());
        };
        let url =
            NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&app.to_string_lossy()), true);
        let configuration = NSWorkspaceOpenConfiguration::configuration();
        configuration.setActivates(activate);
        configuration.setAddsToRecentItems(false);
        // For links, a new copy even beside a running one: a router that
        // holds the lock but does not answer must not get the arguments
        // dropped on a reopen. The copy forwards to it or routes them
        // itself (`router::start`). For the window, the running one.
        configuration.setCreatesNewApplicationInstance(!args.is_empty());
        let args: Vec<Retained<NSString>> = args.iter().map(|a| NSString::from_str(a)).collect();
        configuration.setArguments(&NSArray::from_retained_slice(&args));

        let (answer, answered) = mpsc::channel();
        let done = RcBlock::new(
            move |_app: *mut NSRunningApplication, error: *mut NSError| {
                let _ = answer.send(error.is_null());
            },
        );
        NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
            &url,
            &configuration,
            Some(&done),
        );
        match answered.recv_timeout(LAUNCH_ANSWER) {
            Ok(true) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
            Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("macOS could not launch PwrRoute".into())
            }
        }
    }

    fn open_in(&self, bundle_id: &str, url: &str) -> Result<(), String> {
        let status = Command::new("/usr/bin/open")
            .args(["-b", bundle_id, url])
            .status()
            .map_err(|_| "could not run open".to_owned())?;
        if status.success() {
            Ok(())
        } else {
            Err("open failed".into())
        }
    }
}
