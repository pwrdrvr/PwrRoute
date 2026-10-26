//! The default-browser glue: what the OS says about who handles web links,
//! the one explicit request to make it PwrRoute, and the surfaces that show
//! the answer (the strip chip, Settings › Browsers, first run step 3).
//!
//! The OS calls come in as [`Os`], so tests supply fakes and nothing here
//! ever reaches `pwrroute_os` from a test. [`request`] runs only from a
//! click (Settings, first run, the tray), or the explicit `--make-default`
//! startup flag. The OS has the last word (a consent prompt on macOS,
//! Settings › Default apps on Windows), so the answer is read back rather
//! than assumed: on every
//! window activation ([`watch`]), when the tray dropdown opens, for a while
//! after a request on a fast timer, every second while first run's step 3
//! is open and not done ([`follow_while`]), and always on a slow timer
//! ([`RECHECK_EVERY`]), so a router running with no window still notices
//! when another browser takes the default.
//!
//! LaunchServices answers late: right after the reader approves macOS's
//! dialog, and so right when the window comes back to the front, it can
//! still name the old browser for a second or more. An activation therefore
//! does not settle a request on the spot; the poll keeps reading for
//! [`ACTIVATION_GRACE`] first.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Div, Entity, Global, SharedString, Window};
use pwrroute_browsers::Catalog;
use pwrroute_os::{DefaultRequest, DefaultStatus};

use crate::settings::model::{DefaultBrowser, SettingsRequest};
use crate::theme;
use crate::widgets::status_chip;

/// How often the status is re-read after a request, and for how long. The
/// OS answers in its own dialog, which need not hand the window back, so
/// the poll is what notices the answer; once a second keeps first run's
/// step 3 close behind the click, and a minute bounds a refused prompt.
pub const POLL_EVERY: Duration = Duration::from_secs(1);
pub const POLL_FOR: Duration = Duration::from_secs(60);

/// How long after the window comes back to the front an unchanged answer
/// still counts as "not yet" rather than "no". LaunchServices can report
/// the old handler for a moment after the dialog is answered; the lab VM
/// reports the change within 3 s.
pub const ACTIVATION_GRACE: Duration = Duration::from_secs(5);

/// How often the status is re-read for the life of the app, window or no
/// window. Another browser's "make me the default" prompt is answered
/// without PwrRoute's windows ever activating, and the tray chip would
/// otherwise say "default" until one did. A minute is soon enough for a
/// chip and cheap enough for a LaunchServices or registry read.
pub const RECHECK_EVERY: Duration = Duration::from_secs(60);

/// A read of the OS's default browser this slow is logged: it runs on the
/// main thread, which a link activating the window waits on.
const SLOW_READ: Duration = Duration::from_millis(50);

/// Who the OS hands web links to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultState {
    /// PwrRoute handles `http` and `https`.
    PwrRoute,
    /// Another app does, by the name the reader knows ("Google Chrome"),
    /// or by the id the OS gave when the catalog does not know it.
    Other(SharedString),
    /// The OS did not say: no handler is set, or the query failed.
    Unknown,
}

impl DefaultState {
    /// From the OS's answer. `None` is a failed query. `app_name` names an
    /// installed app the catalog does not list (see [`handler_name`]).
    pub fn from_status(status: Option<&DefaultStatus>, app_name: &AppName) -> Self {
        let Some(status) = status else {
            return Self::Unknown;
        };
        if status.is_default {
            return Self::PwrRoute;
        }
        // Windows can have the two schemes on different apps ("1 of 2
        // done"); name the one that is not PwrRoute yet.
        let ours = |id: &String| {
            id.eq_ignore_ascii_case(pwrroute_os::BUNDLE_ID)
                || id.eq_ignore_ascii_case(pwrroute_os::PROG_ID)
                || id.eq_ignore_ascii_case(pwrroute_os::DESKTOP_ID)
        };
        match [&status.http, &status.https]
            .into_iter()
            .flatten()
            .find(|id| !ours(id))
        {
            Some(id) => Self::Other(handler_name(id, app_name).into()),
            None => Self::Unknown,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::PwrRoute
    }

    /// The browser links open in today, for a sentence: "Google Chrome",
    /// or "your current browser" when the OS did not say.
    pub fn current_name(&self) -> String {
        match self {
            Self::Other(name) => name.to_string(),
            Self::PwrRoute | Self::Unknown => "your current browser".to_owned(),
        }
    }
}

/// A browser or app as `rules.json`, a config or the router names it
/// (bundle id, ProgID, desktop file id or name), as the reader knows it:
/// [`handler_name`] with the installed app's name, looked up once per id.
/// The tray, its recents and Activity's Opened in all name apps this way.
pub fn browser_name(id: &str) -> String {
    handler_name(id, &installed_name)
}

/// `pwrroute_os::app_display_name`, remembered per id for the life of the
/// process: the tray asks on every open and Activity on every row. Tests
/// never ask the OS.
fn installed_name(id: &str) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    thread_local! {
        static NAMES: RefCell<std::collections::HashMap<String, Option<String>>> =
            RefCell::new(std::collections::HashMap::new());
    }
    NAMES.with(|names| {
        names
            .borrow_mut()
            .entry(id.to_owned())
            .or_insert_with(|| pwrroute_os::app_display_name(id))
            .clone()
    })
}

/// The name of the installed app with a handler id, when the OS knows one:
/// `pwrroute_os::app_display_name`.
pub type AppName = dyn Fn(&str) -> Option<String>;

/// The default handler as the reader knows it: the catalog's name, else
/// the installed app's own name ("PwrFinicky" for a Finicky fork the
/// catalog does not list), else the id itself when no app has it.
pub fn handler_name(id: &str, app_name: &AppName) -> String {
    match Catalog::builtin().find(id) {
        Some(browser) => browser.name.clone(),
        None => app_name(id).unwrap_or_else(|| id.to_owned()),
    }
}

/// The OS calls this module makes, as functions so tests supply fakes.
#[derive(Clone)]
pub struct Os {
    /// `pwrroute_os::default_browser_status`; `None` when it failed.
    pub status: Rc<dyn Fn() -> Option<DefaultStatus>>,
    /// `pwrroute_os::set_default_browser`. Only [`request`] calls it.
    pub set_default: Rc<dyn Fn() -> anyhow::Result<DefaultRequest>>,
    /// `pwrroute_os::open_default_apps_settings`.
    pub open_settings: Rc<dyn Fn() -> anyhow::Result<()>>,
    /// `pwrroute_os::app_display_name`.
    pub app_name: Rc<AppName>,
}

impl Os {
    /// The real OS. Built in `main` only; never in a test.
    pub fn real() -> Self {
        Self {
            status: Rc::new(|| pwrroute_os::default_browser_status().ok()),
            set_default: Rc::new(pwrroute_os::set_default_browser),
            open_settings: Rc::new(pwrroute_os::open_default_apps_settings),
            app_name: Rc::new(pwrroute_os::app_display_name),
        }
    }

    /// An OS that answers with `status`, records each request in `requests`
    /// and never prompts: for tests and `--dry-run`-style checks.
    #[cfg(test)]
    pub fn fake(
        status: Rc<RefCell<Option<DefaultStatus>>>,
        requests: Rc<std::cell::Cell<usize>>,
    ) -> Self {
        let read = status.clone();
        Self {
            status: Rc::new(move || read.borrow().clone()),
            set_default: Rc::new(move || {
                requests.set(requests.get() + 1);
                Ok(DefaultRequest::Prompted)
            }),
            open_settings: Rc::new(|| Ok(())),
            app_name: Rc::new(|_| None),
        }
    }

    /// The OS's answer, read back and named.
    fn read(&self) -> DefaultState {
        DefaultState::from_status((self.status)().as_ref(), &*self.app_name)
    }
}

/// The glue's state: the last answer, and whether a request is out.
pub struct Registrar {
    os: Os,
    pub state: DefaultState,
    /// A request went out and the OS has not answered since: the window
    /// says it is waiting.
    pub pending: Option<DefaultRequest>,
    /// The last request ended with the same browser as before: the OS said
    /// no, or the reader did. Shown once, until the next change.
    pub kept: bool,
    /// Counts requests. A poll stops when a newer request has started its
    /// own, so "Ask again" never leaves two running.
    generation: u64,
    /// The window came back to the front while a request was out: how many
    /// more poll reads may still find the old answer before it counts as
    /// no ([`ACTIVATION_GRACE`]).
    grace: Option<u64>,
}

impl Registrar {
    fn new(os: Os) -> Self {
        let state = os.read();
        Self {
            os,
            state,
            pending: None,
            kept: false,
            generation: 0,
            grace: None,
        }
    }

    /// A request went out and the OS has not answered since.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// Re-reads the OS. `settled` says the OS had its chance to answer (the
    /// poll ran out, or ran [`ACTIVATION_GRACE`] past the window's return):
    /// an unchanged answer then means the request was refused, and the
    /// window stops waiting.
    fn refresh(&mut self, settled: bool, cx: &mut Context<'_, Self>) {
        let started = std::time::Instant::now();
        let state = self.os.read();
        let took = started.elapsed();
        if took >= SLOW_READ {
            log::warn!(
                "reading the default browser took {}ms on the main thread",
                took.as_millis()
            );
        }
        if state != self.state {
            self.state = state;
            self.pending = None;
            self.grace = None;
            self.kept = false;
            cx.notify();
        } else if settled && self.pending.take().is_some() {
            self.grace = None;
            self.kept = !self.state.is_default();
            cx.notify();
        }
    }

    /// The window came to the front: read the OS now, and if a request is
    /// still out, give LaunchServices [`ACTIVATION_GRACE`] of poll reads to
    /// catch up before an unchanged answer counts as no.
    fn activated(&mut self, cx: &mut Context<'_, Self>) {
        self.refresh(false, cx);
        if self.waiting() && self.grace.is_none() {
            self.grace = Some(reads_in(ACTIVATION_GRACE));
        }
    }

    /// One read of the request's poll. `last`: the poll has run its
    /// [`POLL_FOR`].
    fn poll(&mut self, last: bool, cx: &mut Context<'_, Self>) {
        let grace_over = match &mut self.grace {
            Some(left) => {
                *left = left.saturating_sub(1);
                *left == 0
            }
            None => false,
        };
        self.refresh(last || grace_over, cx);
    }

    /// The Settings › Browsers card for this state.
    pub fn card(&self) -> DefaultBrowser {
        let note = match (&self.pending, self.kept, &self.state) {
            (Some(request), _, _) => Some(waiting_note(*request).into()),
            (None, true, DefaultState::Other(name)) => {
                Some(format!("{name} is still the default.").into())
            }
            _ => None,
        };
        DefaultBrowser {
            state: self.state.clone(),
            note,
        }
    }
}

/// How many [`POLL_EVERY`] reads fit in `span`.
fn reads_in(span: Duration) -> u64 {
    (span.as_millis() / POLL_EVERY.as_millis()).max(1) as u64
}

/// What the card says while the OS has the question.
fn waiting_note(request: DefaultRequest) -> &'static str {
    match request {
        DefaultRequest::Prompted => {
            "Waiting for macOS. Choose Use \u{201c}PwrRoute\u{201d} in its dialog."
        }
        DefaultRequest::OpenedSettings => {
            "Finish in Windows Settings: select Set default on PwrRoute\u{2019}s page, or choose \
             PwrRoute for HTTP and for HTTPS."
        }
        DefaultRequest::Set => "Set with xdg-settings; reading it back.",
    }
}

/// The installed registrar.
struct Installed(Entity<Registrar>);

impl Global for Installed {}

thread_local! {
    /// The last answer, for surfaces drawn without an `App` in hand (the
    /// main window's strip chip). Kept in step by [`refresh`].
    static SNAPSHOT: RefCell<DefaultState> = const { RefCell::new(DefaultState::Unknown) };
}

pub(crate) fn snapshot() -> DefaultState {
    SNAPSHOT.with(|snapshot| snapshot.borrow().clone())
}

/// Reads the OS once and keeps the answer where every surface can see it,
/// re-reading it every [`RECHECK_EVERY`] from then on. Call once, before
/// any window opens.
pub fn install(os: Os, cx: &mut App) -> Entity<Registrar> {
    let registrar = cx.new(|_| Registrar::new(os));
    SNAPSHOT.with(|snapshot| *snapshot.borrow_mut() = registrar.read(cx).state.clone());
    cx.observe(&registrar, |registrar, cx| {
        let state = registrar.read(cx).state.clone();
        SNAPSHOT.with(|snapshot| *snapshot.borrow_mut() = state);
        let card = registrar.read(cx).card();
        crate::settings::update(cx, move |model| model.browsers.default = card);
        // The strip chip reads the snapshot: redraw it.
        cx.refresh_windows();
    })
    .detach();
    cx.set_global(Installed(registrar.clone()));
    // Not settled: a slow tick says nothing about a request still out.
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(RECHECK_EVERY).await;
            cx.update(|cx| refresh(false, cx));
        }
    })
    .detach();
    registrar
}

/// The installed registrar, if [`install`] ran.
pub fn registrar(cx: &App) -> Option<Entity<Registrar>> {
    cx.try_global::<Installed>()
        .map(|installed| installed.0.clone())
}

/// The last answer.
pub fn state(cx: &App) -> DefaultState {
    registrar(cx).map_or(DefaultState::Unknown, |registrar| {
        registrar.read(cx).state.clone()
    })
}

/// The Settings › Browsers card as things stand.
pub fn card(cx: &App) -> DefaultBrowser {
    registrar(cx).map_or_else(
        || DefaultBrowser {
            state: DefaultState::Unknown,
            note: None,
        },
        |registrar| registrar.read(cx).card(),
    )
}

/// Re-reads the OS now. `settled`: see [`Registrar::refresh`].
pub fn refresh(settled: bool, cx: &mut App) {
    if let Some(registrar) = registrar(cx) {
        registrar.update(cx, |registrar, cx| registrar.refresh(settled, cx));
    }
}

/// Re-reads the OS whenever `window` comes to the front: that is when the
/// OS's own dialog or Settings page has been dealt with (see
/// [`Registrar::activated`]).
pub fn watch(window: &mut Window, cx: &mut App) {
    if let Some(registrar) = registrar(cx) {
        registrar.update(cx, |_, cx| {
            cx.observe_window_activation(window, |registrar, window, cx| {
                if window.is_window_active() {
                    registrar.activated(cx);
                }
            })
            .detach();
        });
    }
}

/// The window was activated: [`Registrar::activated`], for tests, which
/// have no window to activate.
#[cfg(test)]
pub fn activated(cx: &mut App) {
    if let Some(registrar) = registrar(cx) {
        registrar.update(cx, |registrar, cx| registrar.activated(cx));
    }
}

/// Re-reads the OS every [`POLL_EVERY`] for as long as `keep` says, while
/// PwrRoute is not the default: first run's step 3, which must flip as soon
/// as the OS reports PwrRoute, however long the reader took over macOS's
/// dialog and however late LaunchServices caught up. The read is two
/// LaunchServices lookups. Stops for good once `keep` says no.
pub fn follow_while(keep: impl Fn(&mut App) -> bool + 'static, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL_EVERY).await;
            let go = cx.update(|cx| {
                if !keep(cx) {
                    return false;
                }
                if !state(cx).is_default() {
                    refresh(false, cx);
                }
                true
            });
            if !go {
                break;
            }
        }
    })
    .detach();
}

/// Asks the OS to make PwrRoute the default browser. The explicit action,
/// and the only caller of `set_default_browser` in the app: wire it to a
/// click or an explicit `--make-default` launch. Then reads the answer back
/// every [`POLL_EVERY`] for [`POLL_FOR`]: until PwrRoute is the default, the
/// answer settles the other way ([`ACTIVATION_GRACE`] after the window came to the front, or a
/// newer request took over), or the time is up, which settles it too.
pub fn request(cx: &mut App) -> anyhow::Result<DefaultRequest> {
    let Some(registrar) = registrar(cx) else {
        anyhow::bail!("default-browser registration is not installed");
    };
    let os = registrar.read(cx).os.clone();
    let request = (os.set_default)()?;
    let generation = registrar.update(cx, |registrar, cx| {
        registrar.pending = Some(request);
        registrar.kept = false;
        registrar.grace = None;
        registrar.generation += 1;
        cx.notify();
        registrar.generation
    });
    cx.spawn(async move |cx| {
        let ticks = POLL_FOR.as_secs() / POLL_EVERY.as_secs();
        for tick in 1..=ticks {
            cx.background_executor().timer(POLL_EVERY).await;
            let done = cx.update(|cx| {
                let current = registrar.read(cx);
                if current.generation != generation || !current.waiting() {
                    return true;
                }
                registrar.update(cx, |registrar, cx| registrar.poll(tick == ticks, cx));
                let registrar = registrar.read(cx);
                registrar.state.is_default() || !registrar.waiting()
            });
            if done {
                break;
            }
        }
    })
    .detach();
    Ok(request)
}

/// Opens the OS's default-apps page.
pub fn open_settings(cx: &mut App) -> anyhow::Result<()> {
    let Some(registrar) = registrar(cx) else {
        anyhow::bail!("default-browser registration is not installed");
    };
    let open = registrar.read(cx).os.open_settings.clone();
    open()
}

/// The strip chip on the Rules screen: whether links reach PwrRoute at all.
pub fn chip() -> Div {
    let (label, dot) = match snapshot() {
        DefaultState::PwrRoute => ("default browser".to_owned(), theme::status_ok()),
        DefaultState::Other(name) => (format!("{name} is default"), theme::status_warning()),
        DefaultState::Unknown => (
            "default browser unknown".to_owned(),
            theme::status_suspended(),
        ),
    };
    status_chip(label, dot)
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_settings_request(asked: &SettingsRequest, cx: &mut App) -> bool {
    match asked {
        SettingsRequest::RegisterDefaultBrowser => {
            match request(cx) {
                Ok(outcome) => {
                    log::info!("asked the OS for the default browser: {outcome:?}");
                }
                Err(error) => {
                    log::warn!("could not ask for the default browser: {error}");
                }
            }
            true
        }
        SettingsRequest::OpenDefaultAppsSettings => {
            if let Err(error) = open_settings(cx) {
                log::warn!("could not open the default-apps settings: {error}");
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui_kit::TestAppContext;

    use super::*;

    fn status(http: Option<&str>, https: Option<&str>) -> DefaultStatus {
        let ours = |id: Option<&str>| id.is_some_and(|id| id == pwrroute_os::BUNDLE_ID);
        DefaultStatus {
            is_default: ours(http) && ours(https),
            http: http.map(str::to_owned),
            https: https.map(str::to_owned),
        }
    }

    #[test]
    fn the_state_names_the_browser_the_os_reports() {
        let none: &AppName = &|_| None;
        assert_eq!(DefaultState::from_status(None, none), DefaultState::Unknown);
        assert_eq!(
            DefaultState::from_status(Some(&status(None, None)), none),
            DefaultState::Unknown
        );
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some("com.google.Chrome"),
                    Some("com.google.Chrome")
                )),
                none
            ),
            DefaultState::Other("Google Chrome".into())
        );
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some(pwrroute_os::BUNDLE_ID),
                    Some(pwrroute_os::BUNDLE_ID)
                )),
                none
            ),
            DefaultState::PwrRoute
        );
        // Half done: the scheme that is not ours yet names the state.
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some(pwrroute_os::BUNDLE_ID),
                    Some("org.mozilla.firefox")
                )),
                none
            ),
            DefaultState::Other("Firefox".into())
        );
        // An app the catalog does not know keeps its id.
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some("com.example.Browser"),
                    Some("com.example.Browser")
                )),
                none
            ),
            DefaultState::Other("com.example.Browser".into())
        );
        // An app the catalog does not know, but the OS does, goes by the
        // name its bundle gives: the Finicky fork, not its bundle id.
        let installed: &AppName =
            &|id| (id == "com.pwrdrvr.pwrfinicky").then(|| "PwrFinicky".to_owned());
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some("com.pwrdrvr.pwrfinicky"),
                    Some("com.pwrdrvr.pwrfinicky")
                )),
                installed
            ),
            DefaultState::Other("PwrFinicky".into())
        );
        // The catalog's name still wins for a browser it lists.
        let wrong: &AppName = &|_| Some("Chrome (renamed)".to_owned());
        assert_eq!(
            DefaultState::from_status(
                Some(&status(
                    Some("com.google.Chrome"),
                    Some("com.google.Chrome")
                )),
                wrong
            ),
            DefaultState::Other("Google Chrome".into())
        );
    }

    #[gpui_kit::test]
    fn a_request_waits_for_the_os_and_reads_the_answer_back(cx: &mut TestAppContext) {
        let os_status = Rc::new(RefCell::new(Some(status(
            Some("com.google.Chrome"),
            Some("com.google.Chrome"),
        ))));
        let requests = Rc::new(Cell::new(0));
        let registrar = cx.update(|cx| install(Os::fake(os_status.clone(), requests.clone()), cx));
        let card = |cx: &mut TestAppContext| cx.update(|cx| super::card(cx));

        assert_eq!(card(cx).state, DefaultState::Other("Google Chrome".into()));
        assert_eq!(card(cx).note, None);

        cx.update(|cx| request(cx).unwrap());
        assert_eq!(requests.get(), 1);
        assert!(
            card(cx)
                .note
                .is_some_and(|note| note.starts_with("Waiting for macOS"))
        );

        // The prompt is still up: a poll keeps waiting.
        cx.update(|cx| refresh(false, cx));
        assert!(
            card(cx)
                .note
                .is_some_and(|note| note.starts_with("Waiting"))
        );

        // The window came back and nothing changed: the OS said no.
        cx.update(|cx| refresh(true, cx));
        assert_eq!(
            card(cx).note.as_deref(),
            Some("Google Chrome is still the default.")
        );
        assert!(registrar.read_with(cx, |registrar, _| registrar.pending.is_none()));

        // Asked again, and this time the OS agreed.
        cx.update(|cx| request(cx).unwrap());
        *os_status.borrow_mut() = Some(status(
            Some(pwrroute_os::BUNDLE_ID),
            Some(pwrroute_os::BUNDLE_ID),
        ));
        cx.update(|cx| refresh(false, cx));
        assert_eq!(card(cx).state, DefaultState::PwrRoute);
        assert_eq!(card(cx).note, None);
        assert!(cx.update(|cx| state(cx).is_default()));
    }

    /// No window, no request: the slow re-read alone notices another
    /// browser taking the default, and everything that follows the
    /// registrar hears of it.
    #[gpui_kit::test]
    fn losing_the_default_is_noticed_without_a_window(cx: &mut TestAppContext) {
        let ours = || status(Some(pwrroute_os::BUNDLE_ID), Some(pwrroute_os::BUNDLE_ID));
        let chrome = || status(Some("com.google.Chrome"), Some("com.google.Chrome"));
        let os_status = Rc::new(RefCell::new(Some(ours())));
        let requests = Rc::new(Cell::new(0));
        let registrar = cx.update(|cx| install(Os::fake(os_status.clone(), requests.clone()), cx));
        let heard = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let heard = heard.clone();
            cx.observe(&registrar, move |registrar, cx| {
                heard.borrow_mut().push(registrar.read(cx).state.clone());
            })
            .detach();
        });
        assert!(cx.update(|cx| state(cx).is_default()));
        assert_eq!(cx.windows().len(), 0);

        // Chrome takes the default between two ticks. Nothing is heard
        // until the next re-read, and then exactly once.
        cx.executor().advance_clock(RECHECK_EVERY);
        cx.run_until_parked();
        assert!(heard.borrow().is_empty());
        *os_status.borrow_mut() = Some(chrome());
        cx.executor().advance_clock(RECHECK_EVERY / 2);
        cx.run_until_parked();
        assert!(heard.borrow().is_empty(), "half a tick is too soon");
        cx.executor().advance_clock(RECHECK_EVERY / 2);
        cx.run_until_parked();
        assert_eq!(
            *heard.borrow(),
            [DefaultState::Other("Google Chrome".into())]
        );
        assert_eq!(
            cx.update(|cx| state(cx)),
            DefaultState::Other("Google Chrome".into())
        );
        assert_eq!(snapshot(), DefaultState::Other("Google Chrome".into()));
        // Nothing asked the OS for anything: PwrRoute never re-registers
        // itself.
        assert_eq!(requests.get(), 0);
        assert!(registrar.read_with(cx, |r, _| !r.waiting() && !r.kept));

        // Taken back by hand elsewhere: heard the same way.
        *os_status.borrow_mut() = Some(ours());
        cx.executor().advance_clock(RECHECK_EVERY);
        cx.run_until_parked();
        assert_eq!(heard.borrow().len(), 2);
        assert!(cx.update(|cx| state(cx).is_default()));
    }

    /// The poll behind a request: once a second, until the OS agrees, or
    /// a minute is up and the answer counts as no. Asking again hands the
    /// poll to the new request.
    #[gpui_kit::test]
    fn the_poll_reads_the_answer_back_once_a_second_for_a_minute(cx: &mut TestAppContext) {
        let chrome = || status(Some("com.google.Chrome"), Some("com.google.Chrome"));
        let ours = || status(Some(pwrroute_os::BUNDLE_ID), Some(pwrroute_os::BUNDLE_ID));
        let os_status = Rc::new(RefCell::new(Some(chrome())));
        let requests = Rc::new(Cell::new(0));
        let registrar = cx.update(|cx| install(Os::fake(os_status.clone(), requests.clone()), cx));
        let waiting = |cx: &mut TestAppContext| registrar.read_with(cx, |r, _| r.waiting());
        let tick = |cx: &mut TestAppContext| {
            cx.executor().advance_clock(POLL_EVERY);
            cx.run_until_parked();
        };
        let ticks = (POLL_FOR.as_secs() / POLL_EVERY.as_secs()) as usize;
        assert_eq!(ticks, 60);

        // The OS agrees on the third second: the poll sees it and stops.
        cx.update(|cx| request(cx).unwrap());
        assert!(waiting(cx));
        tick(cx);
        tick(cx);
        assert!(waiting(cx));
        *os_status.borrow_mut() = Some(ours());
        tick(cx);
        assert!(!waiting(cx));
        assert!(cx.update(|cx| state(cx).is_default()));
        assert!(!registrar.read_with(cx, |r, _| r.kept));

        // The prompt is refused and the window never comes back: after a
        // minute the answer settles as no.
        *os_status.borrow_mut() = Some(chrome());
        cx.update(|cx| refresh(false, cx));
        cx.update(|cx| request(cx).unwrap());
        for _ in 0..ticks - 1 {
            tick(cx);
            assert!(waiting(cx));
        }
        tick(cx);
        assert!(!waiting(cx));
        assert!(registrar.read_with(cx, |r, _| r.kept));
        assert_eq!(
            cx.update(|cx| super::card(cx)).note.as_deref(),
            Some("Google Chrome is still the default.")
        );

        // Ask again a second in: the first poll stops, and only the new one
        // settles, a minute after its own request.
        cx.update(|cx| request(cx).unwrap());
        tick(cx);
        cx.update(|cx| request(cx).unwrap());
        assert_eq!(requests.get(), 4);
        for _ in 0..ticks - 1 {
            tick(cx);
            assert!(waiting(cx), "the first poll must not settle the second");
        }
        tick(cx);
        assert!(!waiting(cx));
    }

    /// An OS that names Chrome for its first `stale` reads and PwrRoute
    /// from then on, counting reads in `reads`: LaunchServices catching up
    /// some time after the reader approved macOS's dialog. `stale` can be
    /// moved while a test runs.
    fn flips_after(stale: Rc<Cell<usize>>, reads: Rc<Cell<usize>>) -> Os {
        let mut os = Os::fake(Rc::new(RefCell::new(None)), Rc::new(Cell::new(0)));
        os.status = Rc::new(move || {
            reads.set(reads.get() + 1);
            Some(if reads.get() <= stale.get() {
                status(Some("com.google.Chrome"), Some("com.google.Chrome"))
            } else {
                status(Some(pwrroute_os::BUNDLE_ID), Some(pwrroute_os::BUNDLE_ID))
            })
        });
        os
    }

    fn tick(cx: &mut TestAppContext) {
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
    }

    /// The field report: the dialog is approved, the window comes back to
    /// the front, and LaunchServices still names the old browser for a
    /// moment. That read is not a no: the poll keeps reading and the state
    /// flips on the first read that reports PwrRoute. A real no still
    /// settles, `ACTIVATION_GRACE` after the window came back.
    #[gpui_kit::test]
    fn a_stale_answer_when_the_window_returns_is_not_a_no(cx: &mut TestAppContext) {
        let reads = Rc::new(Cell::new(0));
        let stale = Rc::new(Cell::new(4));
        let registrar = cx.update(|cx| install(flips_after(stale.clone(), reads.clone()), cx));
        let note = |cx: &mut TestAppContext| cx.update(|cx| super::card(cx)).note;
        let waiting = |cx: &mut TestAppContext| registrar.read_with(cx, |r, _| r.waiting());
        assert_eq!(reads.get(), 1);

        cx.update(|cx| request(cx).unwrap());
        // Approved; the window is active again before LaunchServices knows.
        cx.update(activated);
        assert_eq!(reads.get(), 2);
        assert!(waiting(cx), "a read on activation alone must not settle");
        assert!(note(cx).is_some_and(|note| note.starts_with("Waiting")));
        tick(cx);
        tick(cx);
        assert_eq!(reads.get(), 4);
        assert!(waiting(cx));
        assert!(!registrar.read_with(cx, |r, _| r.kept));
        // Read 5 is the first to report PwrRoute: the state flips on it.
        tick(cx);
        assert_eq!(reads.get(), 5);
        assert!(cx.update(|cx| state(cx).is_default()));
        assert!(!waiting(cx));
        assert_eq!(note(cx), None);
        // The poll stopped with the answer.
        tick(cx);
        assert_eq!(reads.get(), 5);

        // A refusal: the answer never changes. The window comes back, and
        // the request settles as no once the grace has run, not before.
        let reads = Rc::new(Cell::new(0));
        let stale = Rc::new(Cell::new(usize::MAX));
        let registrar = cx.update(|cx| install(flips_after(stale, reads), cx));
        let waiting = |cx: &mut TestAppContext| registrar.read_with(cx, |r, _| r.waiting());
        cx.update(|cx| request(cx).unwrap());
        cx.update(activated);
        for _ in 1..reads_in(ACTIVATION_GRACE) {
            tick(cx);
            assert!(waiting(cx));
        }
        tick(cx);
        assert!(!waiting(cx));
        assert!(registrar.read_with(cx, |r, _| r.kept));
        assert_eq!(
            note(cx).as_deref(),
            Some("Google Chrome is still the default.")
        );
    }

    /// First run's step 3 reads the answer every second while its window
    /// is open, so it flips even after the request's own poll gave up (the
    /// dialog sat unanswered for over a minute), and stops reading once the
    /// window is gone.
    #[gpui_kit::test]
    fn first_run_follows_the_answer_after_the_poll_gave_up(cx: &mut TestAppContext) {
        let reads = Rc::new(Cell::new(0));
        let stale = Rc::new(Cell::new(usize::MAX));
        let registrar = cx.update(|cx| install(flips_after(stale.clone(), reads.clone()), cx));
        let open = Rc::new(Cell::new(true));
        let keep = open.clone();
        cx.update(|cx| follow_while(move |_| keep.get(), cx));

        cx.update(|cx| request(cx).unwrap());
        for _ in 0..POLL_FOR.as_secs() {
            tick(cx);
        }
        assert!(registrar.read_with(cx, |r, _| !r.waiting() && r.kept));
        assert!(!cx.update(|cx| state(cx).is_default()));

        // Approved at last; LaunchServices reports it two reads later.
        stale.set(reads.get() + 2);
        tick(cx);
        assert!(!cx.update(|cx| state(cx).is_default()));
        tick(cx);
        assert!(!cx.update(|cx| state(cx).is_default()));
        tick(cx);
        assert!(cx.update(|cx| state(cx).is_default()));
        assert!(registrar.read_with(cx, |r, _| !r.kept));
        // Done: no more reads while PwrRoute is the default.
        let done = reads.get();
        tick(cx);
        assert_eq!(reads.get(), done);

        // Another browser takes the default back (the slow re-read sees
        // it); the open window reads every second again, and not once it
        // closes.
        stale.set(usize::MAX);
        cx.update(|cx| refresh(false, cx));
        assert!(!cx.update(|cx| state(cx).is_default()));
        let before = reads.get();
        tick(cx);
        assert_eq!(reads.get(), before + 1);
        open.set(false);
        tick(cx);
        tick(cx);
        assert_eq!(reads.get(), before + 1);
    }
}
