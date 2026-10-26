//! The producers behind Settings › Diagnostics and About: the version and
//! build compiled in, the OS, the data directory, the router's socket and
//! what it routes with, the MCP door and the diagnostic log. About's
//! buttons open Help's windows. Paths and states only: nothing here can
//! carry a link.

use std::{
    fs,
    path::{Path, PathBuf},
};

use gpui_kit::{App, SharedString};

use super::model::{About, Diagnostics, Listener, LocalAgents, SettingsRequest};
use crate::config_view::display_path;
use crate::router::{self, RoutingSource};
use crate::{settings, settings_store};

/// The license line, the same on every copy.
const LICENSE: &str = "MIT. Includes code from Finicky, copyright John Sterling, under the MIT \
                       license; both notices are in LICENSE.";

/// What Diagnostics is built from, read fresh each time.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub os: String,
    pub data_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// The router's socket, when the OS gives it a directory; Windows uses a
    /// named pipe and has no file.
    pub socket: Option<PathBuf>,
    pub listening: bool,
    pub pid: u32,
    pub source: RoutingSource,
    pub dry_run: bool,
    /// The MCP door as Local agents reports it.
    pub agents: Option<(Listener, SharedString)>,
    /// The diagnostic log, when `diag_log` found a folder for it.
    pub log: Option<PathBuf>,
}

/// The Diagnostics pane as things stand. `agents` is the Local agents
/// model built for the same window.
pub fn diagnostics(cx: &App, agents: &LocalAgents) -> Diagnostics {
    let mut facts = facts(cx);
    facts.agents = Some((agents.listener.clone(), agents.endpoint.clone()));
    build_diagnostics(&facts)
}

/// Everything but the MCP door, read from this process and the OS.
fn facts(cx: &App) -> Facts {
    let dry_run = settings_store::router(cx)
        .and_then(|router| router.status().ok())
        .is_some_and(|status| status.dry_run);
    Facts {
        os: os_description(),
        data_dir: pwrroute_os::data_dir(),
        home: std::env::home_dir(),
        socket: router::socket_path(),
        listening: router::listening(),
        pid: std::process::id(),
        source: router::routing_source(),
        dry_run,
        agents: None,
        log: crate::diag_log::path().map(Path::to_path_buf),
    }
}

/// The model from its parts.
pub fn build_diagnostics(facts: &Facts) -> Diagnostics {
    let home = facts.home.as_deref();
    let shown = |path: &Path| display_path(path, home);
    let router = match (&facts.socket, facts.listening) {
        (Some(socket), true) => {
            format!("Listening on {} \u{00b7} pid {}", shown(socket), facts.pid)
        }
        (None, true) => format!("Listening on a named pipe \u{00b7} pid {}", facts.pid),
        (Some(socket), false) => format!(
            "Not listening on {}: another launch's links cannot reach this process \u{00b7} pid {}",
            shown(socket),
            facts.pid
        ),
        (None, false) => format!(
            "Not listening: no router directory \u{00b7} pid {}",
            facts.pid
        ),
    };
    let agents = match &facts.agents {
        Some((Listener::Listening, endpoint)) => format!("Listening on {endpoint}"),
        Some((Listener::Stopped, endpoint)) => format!("Off \u{00b7} {endpoint}"),
        Some((Listener::Failed(why), endpoint)) => format!("Failed: {why} \u{00b7} {endpoint}"),
        None => "Not installed".to_owned(),
    };
    Diagnostics {
        version: version_line().into(),
        os: facts.os.clone().into(),
        data_dir: facts
            .data_dir
            .as_deref()
            .map_or_else(|| "no data folder: the OS gave none".to_owned(), shown)
            .into(),
        data_dir_exists: facts.data_dir.as_deref().is_some_and(Path::exists),
        router: router.into(),
        router_listening: facts.listening,
        routing: routing(&facts.source, home).into(),
        dry_run: facts.dry_run,
        agents: agents.into(),
        log: facts
            .log
            .as_deref()
            .map_or_else(
                || {
                    "No log file: the OS gave no log folder. Messages go to the terminal that \
                     launched PwrRoute, if any."
                        .to_owned()
                },
                shown,
            )
            .into(),
        log_exists: facts.log.as_deref().is_some_and(Path::exists),
    }
}

/// What routes links, in a sentence.
fn routing(source: &RoutingSource, home: Option<&Path>) -> String {
    let config = source
        .config
        .as_deref()
        .map(|path| display_path(path, home));
    let mut sentence = match (&source.fallback, &config, source.rules.is_some()) {
        (Some(why), _, _) => format!("Every link goes to the fallback browser: {why}."),
        (None, Some(config), true) if source.config_not_loaded => {
            format!("{config} has not loaded, so rules.json routes alone until it does.")
        }
        (None, Some(config), true) => format!("{config} runs first; rules.json follows it."),
        (None, Some(config), false) => format!("{config} is the whole config."),
        (None, None, true) => "rules.json is the whole config.".to_owned(),
        (None, None, false) => "Nothing has loaded yet.".to_owned(),
    };
    match &source.error {
        // The fallback's reason is this error already.
        Some(_) if source.fallback.is_some() => {}
        Some(error) if source.config_not_loaded => {
            sentence.push_str(&format!(" It did not load: {error}."));
        }
        Some(error) => sentence.push_str(&format!(
            " The last reload failed: {error}. The previous load stays in charge."
        )),
        None => {}
    }
    sentence
}

/// "0.3.0 · release · arm64".
fn version_line() -> String {
    format!(
        "{} \u{00b7} {} \u{00b7} {}",
        env!("CARGO_PKG_VERSION"),
        build_kind(),
        std::env::consts::ARCH
    )
}

fn build_kind() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// The OS and its version, from the files that name it; the family's
/// wording. Never spawns a process.
pub fn os_description() -> String {
    let arch = std::env::consts::ARCH;
    let version = if cfg!(target_os = "macos") {
        fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
            .ok()
            .and_then(|xml| plist_string(&xml, "ProductVersion"))
            .map(|version| format!("macOS {version}"))
    } else if cfg!(target_os = "windows") {
        None
    } else {
        fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| os_release_name(&text))
    };
    let name = version.unwrap_or_else(|| match std::env::consts::OS {
        "macos" => "macOS".to_owned(),
        "windows" => "Windows".to_owned(),
        other => other.to_owned(),
    });
    format!("{name} ({arch})")
}

/// The `<string>` after `<key>key</key>` in a plist, without a parser:
/// the system version file is small and regular.
pub fn plist_string(xml: &str, key: &str) -> Option<String> {
    let after_key = &xml[xml.find(&format!("<key>{key}</key>"))?..];
    let start = after_key.find("<string>")? + "<string>".len();
    let end = after_key[start..].find("</string>")? + start;
    Some(after_key[start..end].trim().to_owned())
}

/// `PRETTY_NAME` from `/etc/os-release`, unquoted.
pub fn os_release_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|name| !name.is_empty())
}

// ---- About ----------------------------------------------------------------

/// The About pane for this copy.
pub fn about() -> About {
    About {
        version: env!("CARGO_PKG_VERSION").into(),
        build: format!("{} build for {}", build_kind(), std::env::consts::ARCH).into(),
        license: LICENSE.into(),
        links: vec![
            ("Source".into(), "github.com/pwrdrvr/PwrRoute".into()),
            (
                "Releases".into(),
                "github.com/pwrdrvr/PwrRoute/releases".into(),
            ),
            ("Issues".into(), "github.com/pwrdrvr/PwrRoute/issues".into()),
        ],
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    use crate::doc_window::{self, Doc};
    match request {
        SettingsRequest::OpenChangelog => doc_window::open(Doc::Changelog, cx),
        SettingsRequest::OpenLicense => doc_window::open(Doc::License, cx),
        SettingsRequest::OpenThirdPartyNotices => doc_window::open(Doc::ThirdPartyNotices, cx),
        SettingsRequest::OpenLogs => crate::logs_window::open(cx),
        _ => return false,
    }
    true
}

/// Reads Diagnostics again into the open window: the router's state may
/// have moved. The MCP door is taken from the window's own Local agents
/// pane, which its producer keeps current.
pub fn refresh_diagnostics(cx: &mut App) {
    if !settings::is_open(cx) {
        return;
    }
    let mut facts = facts(cx);
    settings::update(cx, move |model| {
        let agents = &model.local_agents;
        facts.agents = Some((agents.listener.clone(), agents.endpoint.clone()));
        model.diagnostics = build_diagnostics(&facts);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            os: "macOS 15.6 (arm64)".to_owned(),
            data_dir: Some(PathBuf::from(
                "/Users/someone/Library/Application Support/PwrRoute",
            )),
            home: Some(PathBuf::from("/Users/someone")),
            socket: Some(PathBuf::from(
                "/Users/someone/Library/Application Support/PwrRoute/router.sock",
            )),
            listening: true,
            pid: 4242,
            source: RoutingSource {
                config: None,
                rules: Some(PathBuf::from("rules.json")),
                config_not_loaded: false,
                fallback: None,
                error: None,
            },
            dry_run: false,
            agents: Some((
                Listener::Listening,
                "~/Library/Application Support/PwrRoute/mcp.sock".into(),
            )),
            log: Some(PathBuf::from(
                "/Users/someone/Library/Logs/PwrRoute/main.log",
            )),
        }
    }

    /// Nothing a pane shows may be a link.
    fn no_url(text: &str) {
        assert!(!text.contains("://"), "{text}");
    }

    #[test]
    fn diagnostics_show_the_version_the_paths_and_the_router_without_a_url() {
        let model = build_diagnostics(&facts());
        assert!(
            model.version.starts_with(env!("CARGO_PKG_VERSION")),
            "{}",
            model.version
        );
        assert_eq!(model.os, "macOS 15.6 (arm64)");
        assert_eq!(model.data_dir, "~/Library/Application Support/PwrRoute");
        assert_eq!(
            model.router,
            "Listening on ~/Library/Application Support/PwrRoute/router.sock \u{00b7} pid 4242"
        );
        assert!(model.router_listening);
        assert_eq!(model.routing, "rules.json is the whole config.");
        assert_eq!(
            model.agents,
            "Listening on ~/Library/Application Support/PwrRoute/mcp.sock"
        );
        assert!(!model.dry_run);
        assert_eq!(model.log, "~/Library/Logs/PwrRoute/main.log");
        assert!(!model.log_exists);
        for text in [
            &model.version,
            &model.os,
            &model.data_dir,
            &model.router,
            &model.routing,
            &model.agents,
            &model.log,
        ] {
            no_url(text);
        }
        let report = model.report();
        no_url(&report);
        assert!(report.starts_with(&format!("PwrRoute {}\n", model.version)));
        assert!(report.contains("\nRouter: Listening on"));
        assert!(!report.contains("Dry run"));
    }

    #[test]
    fn the_routing_sentence_follows_the_router() {
        let home = Some(Path::new("/Users/someone"));
        let config = Some(PathBuf::from("/Users/someone/.finicky.ts"));
        let source =
            |config: Option<PathBuf>, rules: bool, fallback: Option<&str>, error: Option<&str>| {
                RoutingSource {
                    config,
                    rules: rules.then(|| PathBuf::from("rules.json")),
                    config_not_loaded: false,
                    fallback: fallback.map(str::to_owned),
                    error: error.map(str::to_owned),
                }
            };
        assert_eq!(
            routing(&source(config.clone(), true, None, None), home),
            "~/.finicky.ts runs first; rules.json follows it."
        );
        assert_eq!(
            routing(&source(config.clone(), false, None, None), home),
            "~/.finicky.ts is the whole config."
        );
        assert_eq!(
            routing(
                &source(None, false, Some("no config has loaded"), None),
                home
            ),
            "Every link goes to the fallback browser: no config has loaded."
        );
        assert_eq!(
            routing(&source(config, true, None, Some("SyntaxError")), home),
            "~/.finicky.ts runs first; rules.json follows it. The last reload failed: \
             SyntaxError. The previous load stays in charge."
        );
        assert_eq!(
            routing(&RoutingSource::default(), home),
            "Nothing has loaded yet."
        );

        // A config that never loaded: rules.json routes alone, or, with no
        // rules.json, nothing does.
        let config_never = Some(PathBuf::from("/Users/someone/.finicky.ts"));
        let not_loaded = |rules: bool, fallback: Option<&str>| RoutingSource {
            config_not_loaded: true,
            ..source(config_never.clone(), rules, fallback, Some("SyntaxError"))
        };
        assert_eq!(
            routing(&not_loaded(true, None), home),
            "~/.finicky.ts has not loaded, so rules.json routes alone until it does. It did \
             not load: SyntaxError."
        );
        assert_eq!(
            routing(&not_loaded(false, Some("SyntaxError")), home),
            "Every link goes to the fallback browser: SyntaxError."
        );
    }

    #[test]
    fn a_router_that_is_not_listening_and_a_dry_run_say_so() {
        let mut facts = facts();
        facts.listening = false;
        facts.dry_run = true;
        facts.agents = Some((Listener::Stopped, "~/x/mcp.sock".into()));
        let model = build_diagnostics(&facts);
        assert!(model.router.starts_with("Not listening on ~/Library"));
        assert!(!model.router_listening);
        assert!(model.dry_run);
        assert_eq!(model.agents, "Off \u{00b7} ~/x/mcp.sock");
        assert!(model.report().contains("\nDry run: "));

        facts.socket = None;
        facts.data_dir = None;
        let model = build_diagnostics(&facts);
        assert_eq!(
            model.router,
            "Not listening: no router directory \u{00b7} pid 4242"
        );
        assert_eq!(model.data_dir, "no data folder: the OS gave none");
        assert!(!model.data_dir_exists);
    }

    #[test]
    fn the_os_is_read_from_its_files() {
        let plist = "<?xml version=\"1.0\"?><plist><dict>\n<key>ProductName</key>\n<string>macOS</string>\n<key>ProductVersion</key>\n<string>15.6.1</string>\n</dict></plist>";
        assert_eq!(
            plist_string(plist, "ProductVersion").as_deref(),
            Some("15.6.1")
        );
        assert_eq!(plist_string(plist, "ProductName").as_deref(), Some("macOS"));
        assert_eq!(plist_string(plist, "BuildVersion"), None);
        assert_eq!(
            os_release_name("NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04 LTS\"\n").as_deref(),
            Some("Ubuntu 24.04 LTS")
        );
        assert_eq!(os_release_name("NAME=x\n"), None);
        let described = os_description();
        assert!(
            described.ends_with(&format!("({})", std::env::consts::ARCH)),
            "{described}"
        );
        no_url(&described);
    }

    #[test]
    fn about_shows_the_version_and_the_license_without_a_url() {
        let model = about();
        assert_eq!(model.version, env!("CARGO_PKG_VERSION"));
        assert!(
            model
                .build
                .ends_with(&format!("build for {}", std::env::consts::ARCH))
        );
        assert!(model.license.contains("Finicky"));
        assert_eq!(model.links.len(), 3);
        for (label, address) in &model.links {
            no_url(label);
            no_url(address);
            assert!(address.starts_with("github.com/pwrdrvr/PwrRoute"));
        }
    }
}
