//! Link routing end to end: a link reaches the running router the way the OS
//! would hand it over, and the router's dispatch decision is read back
//! through the hook (`dispatches`). Every launch is `--dry-run` on
//! `fixtures/routing.js`, so nothing opens; CONTRACT.md lists what each
//! record must carry.
//!
//! The first group hands links over directly (argv, a second launch, `open
//! -a`) and runs everywhere. The second group goes through the OS's handler
//! for web links (`xdg-open`, the Windows ProgID). Those have the app
//! register itself with the OS and, where it can, make itself the default,
//! so they run only where `PWRROUTE_E2E_DISPOSABLE=1` marks a throwaway
//! runner, and skip elsewhere (CONTRACT.md, "Registration").

use std::{
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use pwrroute_e2e::{App, Harness, Launch, WAIT, harness};
use serde_json::Value;

/// What `fixtures/routing.js` decides for each link.
const DEFAULT: &str = "https://www.example.com/plain";
const WORK: &str = "https://work.example.com/q3?utm_source=mail&id=7";
const WORK_FINAL: &str = "https://work.example.com/q3?id=7";
const SAFARI: &str = "https://docs.example.org/guide";
const BLOCKED: &str = "https://blocked.example.net/x";

fn assert_decision(record: &Value, final_url: &str, browser: &str, profile: &str) {
    assert_eq!(record["outcome"], "dryRun", "{record:#}");
    assert_eq!(record["finalUrl"], final_url, "{record:#}");
    assert_eq!(record["browser"]["name"], browser, "{record:#}");
    assert_eq!(record["browser"]["profile"], profile, "{record:#}");
    let steps = record["steps"].as_array().expect("steps");
    assert!(
        steps
            .iter()
            .any(|s| s["kind"] == "launched" && s["dryRun"] == true),
        "the dry-run launcher was not asked: {record:#}"
    );
}

fn dispatch_of(url: &str) -> impl Fn(&Value) -> bool + '_ {
    move |record| record["link"]["url"] == url
}

#[test]
fn a_link_on_the_command_line_is_routed_without_a_window() {
    let harness = harness!("a_link_on_the_command_line_is_routed_without_a_window");
    let app = harness
        .launch_with(Launch {
            links: vec![DEFAULT.to_owned()],
            ..Launch::routing()
        })
        .unwrap();
    let record = app
        .wait_for_dispatch(DEFAULT, dispatch_of(DEFAULT))
        .unwrap();
    assert_eq!(record["link"]["source"], "commandLine");
    assert_eq!(record["forwarded"], false);
    assert_decision(&record, DEFAULT, "Firefox", "");
    assert!(
        record["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "matched" && s["by"]["kind"] == "default"),
        "{record:#}"
    );

    // A launch for a link is not a launch for the window: past macOS's grace
    // period for a reopen, there is still none.
    thread::sleep(Duration::from_secs(1));
    let state = app.state().unwrap();
    assert_eq!(state["windows"], Value::Array(Vec::new()), "{state:#}");
}

#[test]
fn a_second_launch_forwards_its_links_to_the_router() {
    let harness = harness!("a_second_launch_forwards_its_links_to_the_router");
    let app = harness.launch_with(Launch::routing()).unwrap();
    let state = app.wait_for_main_window().unwrap();
    // A config passed in is in charge: the window shows it, read-only.
    assert_eq!(state["main"]["configInCharge"], true);
    assert_eq!(state["router"]["configError"], Value::Null);
    assert_eq!(state["main"]["editing"], false);

    let status = app.second_launch(&[WORK, SAFARI, BLOCKED]).unwrap();
    assert!(status.success(), "the forwarding launch failed: {status}");

    let work = app.wait_for_dispatch(WORK, dispatch_of(WORK)).unwrap();
    assert_eq!(work["forwarded"], true);
    assert_eq!(work["link"]["source"], "commandLine");
    assert_decision(&work, WORK_FINAL, "Google Chrome", "Work");
    let steps = work["steps"].as_array().unwrap();
    assert!(
        steps
            .iter()
            .any(|s| s["kind"] == "rewrite" && s["rule"] == 0 && s["url"] == WORK_FINAL),
        "rewrite[0] did not run: {work:#}"
    );
    assert!(
        steps.iter().any(|s| s["kind"] == "matched"
            && s["by"]["kind"] == "handler"
            && s["by"]["index"] == 0),
        "handlers[0] did not win: {work:#}"
    );

    let safari = app.wait_for_dispatch(SAFARI, dispatch_of(SAFARI)).unwrap();
    assert_decision(&safari, SAFARI, "Safari", "");

    let blocked = app
        .wait_for_dispatch(BLOCKED, dispatch_of(BLOCKED))
        .unwrap();
    assert_eq!(blocked["outcome"], "notOpened", "{blocked:#}");

    // Forwarding is not a request for the window: the window that was open
    // stays the only one.
    let state = app.state().unwrap();
    assert_eq!(state["windows"].as_array().map(Vec::len), Some(1));
}

/// macOS hands a link to a running app as a `kAEGetURL` Apple Event. `open
/// -a` targets this bundle by path, so it works without PwrRoute being the
/// default browser and changes nothing on the machine.
#[test]
#[cfg(target_os = "macos")]
fn an_apple_event_reaches_the_running_router() {
    let harness = harness!("an_apple_event_reaches_the_running_router");
    let Some(bundle) = harness.bundle().map(ToOwned::to_owned) else {
        eprintln!("skipped: PWRROUTE_E2E_APP is not a .app bundle");
        return;
    };
    let app = harness.launch_with(Launch::routing()).unwrap();
    app.wait_for_main_window().unwrap();

    let router_dir = app.router_dir().unwrap().display().to_string();
    let status = Command::new("open")
        // Should LaunchServices start a second copy rather than deliver the
        // event, that copy forwards to this router instead of becoming one.
        .args(["-g", "-a"])
        .arg(&bundle)
        .args(["--env", &format!("PWRROUTE_E2E_ROUTER_DIR={router_dir}")])
        .arg(WORK)
        .status()
        .unwrap();
    assert!(status.success(), "open -a failed: {status}");

    let record = app.wait_for_dispatch(WORK, dispatch_of(WORK)).unwrap();
    assert_eq!(record["link"]["source"], "appleEvent", "{record:#}");
    assert_eq!(record["forwarded"], false);
    assert_decision(&record, WORK_FINAL, "Google Chrome", "Work");
}

// ---------------------------------------------------------------------------
// Through the OS's handler for web links. Disposable runners only.
// ---------------------------------------------------------------------------

/// Whether a scenario that changes the OS's browser settings may run here.
/// Skips (and says why) on a developer machine, and on macOS until the
/// harness can confirm LaunchServices' consent prompt (CONTRACT.md,
/// "Registration").
fn may_change_os_settings(test: &str) -> bool {
    if !Harness::disposable() {
        eprintln!(
            "skipped {test}: it changes the OS's browser settings, so it runs only with \
             PWRROUTE_E2E_DISPOSABLE=1 on a throwaway runner"
        );
        return false;
    }
    if cfg!(target_os = "macos") {
        eprintln!(
            "skipped {test}: pending on macOS, where LaunchServices asks the user to \
             confirm a new default browser (e2e/CONTRACT.md, Registration)"
        );
        return false;
    }
    true
}

/// Makes PwrRoute the OS's handler for web links, through the app's own
/// registration (never a registry or desktop-file write from here).
fn make_default(app: &App) {
    app.action("registerBrowser").unwrap();
    if cfg!(windows) {
        // Windows will not let an app make itself the default (`UserChoice`
        // is hashed); see `open_through_the_os`.
        return;
    }
    app.action("setDefaultBrowser").unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let os = app.os_state().unwrap();
        if os["defaultBrowser"] == true {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "PwrRoute did not become the default browser: {os}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Opens `url` the way another app would: through the OS's handler for web
/// links, which must be PwrRoute by now. A copy the OS starts for it gets
/// the harness's session environment. Returns the process the OS tool left
/// running: on a cold start that can be the router itself, or `xdg-open`
/// waiting on it.
fn open_through_the_os(harness: &Harness, url: &str) -> Child {
    let env = harness.session_env();
    let mut command = if cfg!(target_os = "macos") {
        let mut open = Command::new("open");
        for (key, value) in &env {
            open.args(["--env", &format!("{key}={value}")]);
        }
        open.arg(url);
        open
    } else if cfg!(windows) {
        progid_command(url)
    } else {
        let mut xdg_open = Command::new("xdg-open");
        // $BROWSER would win over the default xdg-open is meant to find.
        xdg_open.arg(url).env_remove("BROWSER");
        xdg_open
    };
    command
        .envs(env)
        .stdin(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot ask the OS to open {url}: {e}"))
}

/// Waits for `child` to exit, and requires success.
fn wait_success(mut child: Child) {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "the OS refused the link: {status}");
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the OS's link hand-off did not finish within {WAIT:?}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// The command the `PwrRouteURL` ProgID registered, for `url`: what the
/// shell runs for a link once the user has picked PwrRoute.
#[cfg(windows)]
fn progid_command(url: &str) -> Command {
    use std::os::windows::process::CommandExt as _;

    let out = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Classes\PwrRouteURL\shell\open\command",
            "/ve",
        ])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find_map(|line| line.split_once("REG_SZ").map(|(_, v)| v.trim().to_owned()))
        .unwrap_or_else(|| panic!("PwrRouteURL has no open command: {text}"));
    // `"C:\…\pwrroute.exe" "%1"`: the quoted program, then its arguments,
    // passed as written, as CreateProcess would get them.
    let (program, args) = line
        .strip_prefix('"')
        .and_then(|rest| rest.split_once('"'))
        .unwrap_or_else(|| panic!("unexpected ProgID command {line}"));
    let mut command = Command::new(program);
    command.raw_arg(args.trim().replace("%1", url));
    command
}

#[cfg(not(windows))]
fn progid_command(_: &str) -> Command {
    unreachable!("Windows only")
}

/// With PwrRoute the OS's handler for web links, `xdg-open` and the Windows
/// ProgID hand the link to the running dry-run router, by way of a new
/// process that forwards it.
#[test]
fn the_os_handler_for_web_links_hands_links_to_the_router() {
    let harness = harness!("the_os_handler_for_web_links_hands_links_to_the_router");
    if !may_change_os_settings(harness.test()) {
        return;
    }
    // Not isolated: the process the OS starts for the link has none of this
    // process's environment, so it must find the router where it usually is.
    let app = harness
        .launch_with(Launch {
            isolated: false,
            ..Launch::routing()
        })
        .unwrap();
    app.wait_for_main_window().unwrap();
    make_default(&app);

    wait_success(open_through_the_os(&harness, WORK));
    let record = app.wait_for_dispatch(WORK, dispatch_of(WORK)).unwrap();
    let expected_source = if cfg!(target_os = "macos") {
        "appleEvent"
    } else {
        "commandLine"
    };
    assert_eq!(record["link"]["source"], expected_source, "{record:#}");
    // Windows and Linux start a new process for the link, which forwards it.
    assert_eq!(record["forwarded"], !cfg!(target_os = "macos"));
    assert_decision(&record, WORK_FINAL, "Google Chrome", "Work");
}

/// Cold start: no router is running when the OS hands over the link. The OS
/// starts PwrRoute for it; that copy becomes the router, routes the link by
/// the config it was given in the environment (R8), and shows no window.
#[test]
fn a_link_from_the_os_starts_the_router_cold() {
    let harness = harness!("a_link_from_the_os_starts_the_router_cold");
    if !may_change_os_settings(harness.test()) {
        return;
    }
    // Register with a first copy, then quit it so none is running.
    let mut first = harness
        .launch_with(Launch {
            isolated: false,
            ..Launch::routing()
        })
        .unwrap();
    first.wait_for_main_window().unwrap();
    make_default(&first);
    first.action("quit").unwrap();
    assert!(first.wait_exit(WAIT).unwrap().success());

    let mut handoff = open_through_the_os(&harness, DEFAULT);
    let app = harness.attach_to_session_router().unwrap();
    let record = app
        .wait_for_dispatch(DEFAULT, dispatch_of(DEFAULT))
        .unwrap();
    assert_eq!(record["forwarded"], false, "{record:#}");
    assert_eq!(record["link"]["source"], "commandLine", "{record:#}");
    assert_decision(&record, DEFAULT, "Firefox", "");
    thread::sleep(Duration::from_secs(1));
    let state = app.state().unwrap();
    assert_eq!(state["windows"], Value::Array(Vec::new()), "{state:#}");
    assert_eq!(state["router"]["dryRun"], true);

    // Dropping the attached app quits that router; then nothing of the
    // hand-off is left running.
    drop(app);
    let _ = handoff.kill();
    let _ = handoff.wait();
}
