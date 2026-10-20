//! The end-to-end test hook: a control socket that reports what the window
//! drew and what the router decided, and feeds the window input.
//!
//! Off unless `PWRROUTE_E2E_SOCKET` is set when the app starts. The harness
//! in `e2e/` sets it on the copy it launches; nothing else should. Input goes
//! in through `Window::dispatch_event` and `dispatch_keystroke`, the path the
//! platform's own events take, so a click here runs the same handlers a
//! mouse does. The protocol is `e2e/CONTRACT.md`.
//!
//! Like the router's IPC, the socket is local and private to the user; it
//! answers with full URLs and never logs them.

use std::{
    collections::BTreeMap,
    io::{self, BufRead as _, BufReader, Write as _},
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use gpui_kit::{
    App, Bounds, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    PlatformInput, Window, WindowId,
};
use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use pwrroute_router::RouterHandle;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::{PwrRoute, Screen, Selection};
use crate::app_menu::Quit;

/// The socket's name: a file path on Unix, a named pipe's name on Windows.
pub const SOCKET_ENV: &str = "PWRROUTE_E2E_SOCKET";

/// Where the router keeps its lock and socket while under test, so a test
/// run never meets the developer's own PwrRoute. Read by `router::start`.
pub const ROUTER_DIR_ENV: &str = "PWRROUTE_E2E_ROUTER_DIR";

/// `1` marks a throwaway guest or runner: only then does the hook register
/// PwrRoute with the OS or ask to make it the default browser.
pub const DISPOSABLE_ENV: &str = "PWRROUTE_E2E_DISPOSABLE";

/// `--dry-run` and `--config` for a copy the OS starts for a link, which gets
/// no arguments of the harness's. Read by `router::Args` while the hook is on.
pub const DRY_RUN_ENV: &str = "PWRROUTE_E2E_DRY_RUN";
pub const CONFIG_ENV: &str = "PWRROUTE_E2E_CONFIG";

/// Bumped on any incompatible change to the protocol in `e2e/CONTRACT.md`.
const PROTOCOL_VERSION: u64 = 1;

/// How long a request may wait for the UI thread.
const UI_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a connection may take to send its request or read the answer;
/// longer than [`UI_TIMEOUT`], which the answer may wait out.
const IO_TIMEOUT: Duration = Duration::from_secs(10);

static ENABLED: AtomicBool = AtomicBool::new(false);

/// What the main window has painted, keyed to the window that painted it, so
/// a closed window's regions never pass for a reopened one's.
static PAINTED: Mutex<Option<Painted>> = Mutex::new(None);

struct Painted {
    window: WindowId,
    /// Frames this window has prepainted.
    frames: u64,
    /// The last painted bounds of each named region, in window coordinates.
    regions: BTreeMap<&'static str, Bounds<Pixels>>,
}

/// Updates the record for `window`, started afresh when another window
/// painted last.
fn with_painted(window: WindowId, f: impl FnOnce(&mut Painted)) {
    let mut painted = PAINTED.lock().unwrap_or_else(PoisonError::into_inner);
    let fresh = || Painted {
        window,
        frames: 0,
        regions: BTreeMap::new(),
    };
    f(match &mut *painted {
        Some(p) if p.window == window => p,
        slot => slot.insert(fresh()),
    });
}

/// What `window` has painted: its frame count and regions.
fn painted_by(window: WindowId) -> (u64, BTreeMap<&'static str, Bounds<Pixels>>) {
    match &*PAINTED.lock().unwrap_or_else(PoisonError::into_inner) {
        Some(p) if p.window == window => (p.frames, p.regions.clone()),
        _ => (0, BTreeMap::new()),
    }
}

/// Records the bounds of a div's children under `names`, in order, while the
/// hook is on. For `Div::on_children_prepainted`; keep `names` in step with
/// the children. Extra children are ignored; a name with no child this frame
/// is dropped.
pub fn regions(
    names: &'static [&'static str],
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
    move |bounds, window, _| {
        if !ENABLED.load(Ordering::Relaxed) {
            return;
        }
        with_painted(window.window_handle().window_id(), |painted| {
            // A name past the last child (the editor rail off the Rules
            // screen, a second chip) is not on screen this frame.
            for (i, name) in names.iter().enumerate() {
                match bounds.get(i) {
                    Some(bounds) => painted.regions.insert(name, *bounds),
                    None => painted.regions.remove(name),
                };
            }
        });
    }
}

/// Counts a frame of the main window. Called from the root's prepaint.
pub fn frame(window: &Window) {
    if ENABLED.load(Ordering::Relaxed) {
        with_painted(window.window_handle().window_id(), |p| p.frames += 1);
    }
}

/// A request, as `e2e/CONTRACT.md` lists them.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
enum Request {
    Ping,
    State,
    /// Keystrokes, space separated, in GPUI's syntax: `down`, `cmd-q`.
    Key {
        keys: String,
    },
    /// A left click at the center of a named region.
    Click {
        region: String,
    },
    /// An app action, as the menu bar would dispatch it.
    Action {
        name: String,
    },
    /// The router's recent dispatches, newest first.
    Dispatches,
    /// Which app handles web links now.
    Os,
}

struct Job {
    request: Request,
    reply: mpsc::Sender<Value>,
    /// Signalled once the answer is written to the connection.
    written: async_channel::Receiver<()>,
}

/// What a request does once its answer is out: quitting, or keystrokes that
/// may quit, must not end the process before the client has its answer.
type Then = Box<dyn FnOnce(&mut App)>;

/// The hook's half that lives on the UI thread; see [`attach`].
pub struct Hook {
    jobs: async_channel::Receiver<Job>,
}

/// Starts the control socket when `PWRROUTE_E2E_SOCKET` is set. Call before
/// GPUI starts, then hand the result to [`attach`] inside `run`.
pub fn start(router: RouterHandle) -> Option<Hook> {
    let name = std::env::var_os(SOCKET_ENV)?;
    let name = name.to_string_lossy().into_owned();
    match listen(&name, router) {
        Ok(hook) => {
            ENABLED.store(true, Ordering::Relaxed);
            eprintln!("pwrroute: e2e control socket on {name}");
            Some(hook)
        }
        Err(e) => {
            eprintln!("pwrroute: cannot open the e2e control socket {name}: {e}");
            None
        }
    }
}

/// Serves requests that need the window on the UI thread.
pub fn attach(hook: Hook, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Ok(job) = hook.jobs.recv().await {
            let (answer, then) = cx.update(|cx| on_ui_thread(job.request, cx));
            let _ = job.reply.send(answer);
            if let Some(then) = then {
                // Before the next job, so requests still apply in order.
                let _ = job.written.recv().await;
                cx.update(then);
            }
        }
    })
    .detach();
}

fn listen(name: &str, router: RouterHandle) -> io::Result<Hook> {
    #[cfg(windows)]
    let name = {
        use interprocess::local_socket::GenericNamespaced;
        name.to_owned().to_ns_name::<GenericNamespaced>()?
    };
    #[cfg(not(windows))]
    let name = {
        use interprocess::local_socket::GenericFilePath;
        name.to_owned().to_fs_name::<GenericFilePath>()?
    };
    let listener = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_sync()?;
    let (jobs_tx, jobs) = async_channel::unbounded();
    thread::Builder::new()
        .name("pwrroute-e2e".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let jobs = jobs_tx.clone();
                let router = router.clone();
                let _ = thread::Builder::new()
                    .name("pwrroute-e2e-conn".into())
                    .spawn(move || serve(conn, &jobs, &router));
            }
        })?;
    Ok(Hook { jobs })
}

/// One request per connection, one line of JSON each way.
fn serve(conn: Stream, jobs: &async_channel::Sender<Job>, router: &RouterHandle) {
    // A client that connects and never writes must not hold this thread for
    // good. Windows named pipes have no timeouts; there it does, as with the
    // router's IPC.
    for set in [
        conn.set_recv_timeout(Some(IO_TIMEOUT)),
        conn.set_send_timeout(Some(IO_TIMEOUT)),
    ] {
        if let Err(e) = set
            && e.kind() != io::ErrorKind::Unsupported
        {
            return;
        }
    }
    let mut conn = BufReader::new(conn);
    let mut line = String::new();
    let (written_tx, written) = async_channel::bounded(1);
    let answer = match conn.read_line(&mut line) {
        Ok(0) | Err(_) => return,
        Ok(_) => match serde_json::from_str::<Request>(&line) {
            Ok(request) => answer(request, jobs, written, router),
            Err(e) => error(format!("bad request: {e}")),
        },
    };
    let mut out = answer.to_string();
    out.push('\n');
    let _ = conn.get_mut().write_all(out.as_bytes());
    let _ = conn.get_mut().flush();
    let _ = written_tx.try_send(());
}

fn answer(
    request: Request,
    jobs: &async_channel::Sender<Job>,
    written: async_channel::Receiver<()>,
    router: &RouterHandle,
) -> Value {
    match request {
        // Off the UI thread: these must answer while it is busy.
        Request::Ping => ok(json!({})),
        Request::Dispatches => ok(json!({ "dispatches": router.recent() })),
        // Asked for on its own: it starts OS tools on Linux (xdg-mime).
        Request::Os => ok(os_state()),
        // Off the UI thread too: they run OS tools (xdg-settings,
        // LaunchServices) that can take a moment.
        Request::Action { name } if registration_action(&name) => registration(&name),
        request => {
            let (reply, rx) = mpsc::channel();
            let job = Job {
                request,
                reply,
                written,
            };
            if jobs.send_blocking(job).is_err() {
                return error("the UI thread has stopped".into());
            }
            let mut answer = rx
                .recv_timeout(UI_TIMEOUT)
                .unwrap_or_else(|_| error("the UI thread did not answer in time".into()));
            if let (Some(answer), Ok(status)) = (answer.as_object_mut(), router.status())
                && answer.get("ok") == Some(&Value::Bool(true))
                && answer.contains_key("windows")
            {
                answer.insert(
                    "router".into(),
                    json!({ "dryRun": status.dry_run, "configError": status.config_error }),
                );
            }
            answer
        }
    }
}

/// Answers `request`, and returns what to do once the answer is written.
fn on_ui_thread(request: Request, cx: &mut App) -> (Value, Option<Then>) {
    match request {
        Request::Key { keys } => {
            let keystrokes = match keys
                .split_whitespace()
                .map(Keystroke::parse)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(keystrokes) => keystrokes,
                Err(e) => return (error(e.to_string()), None),
            };
            let answer = with_main_window(cx, |_, _| Ok(json!({})));
            let then = (answer["ok"] == true).then(|| -> Then {
                Box::new(move |cx| {
                    with_main_window(cx, |window, cx| {
                        for keystroke in keystrokes {
                            window.dispatch_keystroke(keystroke, cx);
                        }
                        Ok(json!({}))
                    });
                })
            });
            (answer, then)
        }
        Request::Action { name } if name == "quit" => (
            ok(json!({})),
            Some(Box::new(|cx| cx.dispatch_action(&Quit))),
        ),
        request => (now(request, cx), None),
    }
}

fn now(request: Request, cx: &mut App) -> Value {
    match request {
        Request::State => ok(state(cx)),
        Request::Click { region } => with_main_window(cx, |window, cx| {
            let (_, regions) = painted_by(window.window_handle().window_id());
            let bounds = regions
                .get(region.as_str())
                .copied()
                .ok_or_else(|| format!("the main window has painted no region {region:?}"))?;
            click(window, bounds, cx);
            Ok(json!({}))
        }),
        Request::Action { name } => match name.as_str() {
            // What the window's close button does. Not `CloseWindow`: that
            // closes the active window, and a launched-under-test app is not
            // frontmost.
            "closeWindow" => with_main_window(cx, |window, _| {
                window.remove_window();
                Ok(json!({}))
            }),
            "showWindow" => {
                crate::show_main_window(cx);
                ok(json!({}))
            }
            other => error(format!("unknown action {other:?}")),
        },
        Request::Ping | Request::Dispatches | Request::Os => {
            unreachable!("answered off the UI thread")
        }
        Request::Key { .. } => unreachable!("answered in on_ui_thread"),
    }
}

fn registration_action(name: &str) -> bool {
    matches!(
        name,
        "registerBrowser" | "setDefaultBrowser" | "unregisterBrowser"
    )
}

/// Registers PwrRoute with the OS, or asks to make it the default, through
/// the app's own `pwrroute_os` calls. Refused unless this process was told
/// it runs on a throwaway machine: these change machine-wide state.
fn registration(name: &str) -> Value {
    if std::env::var_os(DISPOSABLE_ENV).is_none_or(|v| v != "1") {
        return error(format!(
            "{name} changes the OS's browser settings; the app refuses it without {DISPOSABLE_ENV}=1"
        ));
    }
    let result = match name {
        "registerBrowser" => pwrroute_os::register_browser().map(|()| json!({})),
        "unregisterBrowser" => pwrroute_os::unregister_browser().map(|()| json!({})),
        _ => pwrroute_os::set_default_browser()
            .map(|request| json!({ "request": format!("{request:?}") })),
    };
    match result {
        Ok(body) => ok(body),
        Err(e) => error(format!("{name}: {e:#}")),
    }
}

/// Which app handles web links now. Reads only.
fn os_state() -> Value {
    match pwrroute_os::default_browser_status() {
        Ok(status) => json!({
            "defaultBrowser": status.is_default,
            "http": status.http,
            "https": status.https,
        }),
        Err(e) => json!({ "defaultBrowser": false, "error": format!("{e:#}") }),
    }
}

fn click(window: &mut Window, bounds: Bounds<Pixels>, cx: &mut App) {
    let position = bounds.center();
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

/// Runs `f` on the main window, or answers that there is none.
fn with_main_window(
    cx: &mut App,
    f: impl FnOnce(&mut Window, &mut App) -> Result<Value, String>,
) -> Value {
    let Some(window) = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<PwrRoute>().is_some())
    else {
        return error("the main window is not open".into());
    };
    match window.update(cx, |_, window, cx| f(window, cx)) {
        Ok(Ok(value)) => ok(value),
        Ok(Err(e)) => error(e),
        Err(e) => error(e.to_string()),
    }
}

/// What the app is showing: every window, and the main window's regions and
/// view state.
fn state(cx: &mut App) -> Value {
    let mut windows = Vec::new();
    let mut main = Value::Null;
    for handle in cx.windows() {
        let _ = handle.update(cx, |root, window, cx| {
            let view = root.downcast::<PwrRoute>().ok();
            windows.push(json!({
                // GPUI reads the title back from the platform on macOS
                // only; elsewhere the harness asks the OS (`e2e/src/os.rs`).
                "title": cfg!(target_os = "macos").then(|| window.window_title()),
                "active": window.is_window_active(),
                "main": view.is_some(),
                "bounds": rect(window.bounds()),
                "viewport": {
                    "width": f32::from(window.viewport_size().width),
                    "height": f32::from(window.viewport_size().height),
                },
                "scaleFactor": window.scale_factor(),
            }));
            if let Some(view) = view {
                let app = view.read(cx);
                let (frames, regions) = painted_by(window.window_handle().window_id());
                let regions: BTreeMap<_, _> = regions
                    .into_iter()
                    .map(|(name, bounds)| (name, rect(bounds)))
                    .collect();
                main = json!({
                    "frames": frames,
                    "focused": app.focus.is_focused(window),
                    "regions": regions,
                    "section": section(app.screen),
                    "configInCharge": app.config.is_some(),
                    "editing": app.editing(),
                    "rules": {
                        "rewrites": app.rewrites.len(),
                        "handlers": app.handlers.len(),
                        "selection": app.selection.map(selection),
                        "selectionEnabled": selection_enabled(app),
                    },
                });
            }
        });
    }
    json!({ "windows": windows, "main": main })
}

/// The rail section on screen, as `e2e/CONTRACT.md` names it.
fn section(screen: Screen) -> &'static str {
    match screen {
        Screen::Rules => "rules",
        Screen::TestLink => "test",
        Screen::Activity => "activity",
    }
}

fn selection(selection: Selection) -> Value {
    match selection {
        Selection::Rewrite(index) => json!({ "kind": "rewrite", "index": index }),
        Selection::Handler(index) => json!({ "kind": "handler", "index": index }),
    }
}

/// Whether the selected rule is switched on.
fn selection_enabled(app: &PwrRoute) -> Option<bool> {
    match app.selection? {
        Selection::Rewrite(i) => app.rewrites.get(i).map(|r| r.rule.enabled),
        Selection::Handler(i) => app.handlers.get(i).map(|h| h.rule.enabled),
    }
}

fn rect(bounds: Bounds<Pixels>) -> Value {
    json!({
        "x": f32::from(bounds.origin.x),
        "y": f32::from(bounds.origin.y),
        "width": f32::from(bounds.size.width),
        "height": f32::from(bounds.size.height),
    })
}

fn ok(body: Value) -> Value {
    let mut body = body;
    if let Some(object) = body.as_object_mut() {
        object.insert("ok".into(), Value::Bool(true));
        object.insert("v".into(), PROTOCOL_VERSION.into());
        object.insert("pid".into(), std::process::id().into());
    }
    body
}

fn error(message: String) -> Value {
    json!({ "ok": false, "v": PROTOCOL_VERSION, "pid": std::process::id(), "error": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_from_the_contract() {
        let parse = |s: &str| serde_json::from_str::<Request>(s).unwrap();
        assert!(matches!(parse(r#"{"cmd":"state"}"#), Request::State));
        assert!(matches!(
            parse(r#"{"cmd":"key","keys":"down cmd-q"}"#),
            Request::Key { keys } if keys == "down cmd-q"
        ));
        assert!(matches!(
            parse(r#"{"cmd":"click","region":"rail.activity"}"#),
            Request::Click { region } if region == "rail.activity"
        ));
        assert!(matches!(
            parse(r#"{"cmd":"action","name":"quit"}"#),
            Request::Action { name } if name == "quit"
        ));
        assert!(matches!(
            parse(r#"{"cmd":"dispatches"}"#),
            Request::Dispatches
        ));
    }

    #[test]
    fn answers_carry_ok_version_and_pid() {
        let answer = ok(json!({ "x": 1 }));
        assert_eq!(answer["ok"], true);
        assert_eq!(answer["v"], PROTOCOL_VERSION);
        assert_eq!(answer["x"], 1);
        let answer = error("nope".into());
        assert_eq!(answer["ok"], false);
        assert_eq!(answer["error"], "nope");
    }

    #[test]
    fn registration_is_refused_off_a_disposable_machine() {
        // Unit tests never set PWRROUTE_E2E_DISPOSABLE, so this must refuse
        // before it reaches the OS.
        assert!(std::env::var_os(DISPOSABLE_ENV).is_none());
        for name in ["registerBrowser", "setDefaultBrowser", "unregisterBrowser"] {
            assert!(registration_action(name));
            let answer = registration(name);
            assert_eq!(answer["ok"], false);
            assert!(answer["error"].as_str().unwrap().contains(DISPOSABLE_ENV));
        }
        assert!(!registration_action("quit"));
    }

    #[test]
    fn requests_include_the_os_query() {
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"cmd":"os"}"#).unwrap(),
            Request::Os
        ));
    }
}
