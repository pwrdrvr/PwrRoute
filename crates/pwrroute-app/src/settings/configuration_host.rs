//! The producer behind Settings › Configuration: which file is in charge
//! as the router reports it (`router::routing_source`), the counts of
//! `rules.json` and of the Finicky config, and the files on disk. The
//! importer is not built, so the pane offers no plan; the source tiles
//! only show the choice, since the router follows the files on disk.

use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use gpui_kit::{App, SharedString};
use pwrroute_engine::{DEFAULT_SHORTENERS, DefaultHost, Engine, Limits, RulesFile, discovery};

use super::model::{Configuration, FinickyConfig, RulesSource, SettingsRequest};
use crate::config_view::display_path;
use crate::router::{self, RoutingSource};
use crate::{settings, settings_store};

/// How long after Reload now the window reads the router's status again:
/// the reload runs on the dispatch thread.
const REREAD_AFTER: Duration = Duration::from_millis(300);

/// What the model is built from: the router's word on the source, and the
/// two files as they stand.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub source: RoutingSource,
    pub rules_path: Option<PathBuf>,
    /// `rules.json` as loaded, or why it did not load; `None` when there is
    /// no file.
    pub rules: Option<Result<RulesFile, String>>,
    pub rules_modified: Option<SystemTime>,
    /// The config the router names, with its counts when it loads.
    pub config: Option<ConfigFacts>,
    pub home: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFacts {
    pub path: PathBuf,
    /// Rewrites and handlers, when the config loaded.
    pub counts: Option<(usize, usize)>,
    /// The shortener domains the config expands, when it loaded: its
    /// `urlShorteners`, or the built-in list when it sets none. Empty when
    /// it turned expansion off.
    pub shorteners: Option<Vec<String>>,
    pub modified: Option<SystemTime>,
}

/// The pane's model as things stand.
pub fn model(cx: &App) -> Configuration {
    let _ = cx;
    build(&facts(), SystemTime::now())
}

fn facts() -> Facts {
    let source = router::routing_source();
    let environment = discovery::Environment::current();
    let rules_path = discovery::rules_path(&environment);
    let rules = rules_path
        .as_deref()
        .filter(|path| path.exists())
        .map(|path| RulesFile::load(path).map_err(|error| format!("{error:#}")));
    let rules_modified = rules_path.as_deref().and_then(modified);
    // Counting the config's rules means evaluating it, as the Rules screen
    // does at open; the same load says which short links it expands.
    let config = source.config.clone().map(|path| {
        let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default()).ok();
        ConfigFacts {
            counts: engine
                .as_ref()
                .and_then(|engine| engine.describe().ok().flatten())
                .map(|description| (description.rewrites.len(), description.handlers.len())),
            shorteners: engine.map(|engine| engine.short_links().domains.clone()),
            modified: modified(&path),
            path,
        }
    });
    Facts {
        source,
        rules_path,
        rules,
        rules_modified,
        config,
        home: std::env::home_dir(),
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// The model from its parts, as of `now`.
pub fn build(facts: &Facts, now: SystemTime) -> Configuration {
    let home = facts.home.as_deref();
    let (visual_summary, rules_status) = match &facts.rules {
        Some(Ok(file)) => (
            format!(
                "rules.json \u{00b7} {} \u{00b7} {}",
                plural(file.rewrites.len(), "rewrite"),
                plural(file.rules.len(), "handler")
            ),
            format!("Loaded \u{00b7} {}", edited(facts.rules_modified, now)),
        ),
        Some(Err(error)) => (
            "rules.json \u{00b7} did not load".to_owned(),
            format!("Not loaded: {error}. The last good snapshot stays in charge."),
        ),
        None => (
            "rules.json \u{00b7} not written yet".to_owned(),
            "No rules.json yet: the Rules view writes it with the first rule.".to_owned(),
        ),
    };
    let finicky = facts.config.as_ref().map(|config| {
        let path = display_path(&config.path, home);
        let summary = match config.counts {
            Some((rewrites, handlers)) => format!(
                "{path} \u{00b7} {} \u{00b7} {}",
                plural(rewrites, "rewrite"),
                plural(handlers, "handler")
            ),
            None => format!("{path} \u{00b7} did not load"),
        };
        let found = match &facts.source.error {
            Some(error) => format!(
                "Found at {path}. The last reload failed: {error}. The previous load stays in \
                 charge."
            ),
            None if facts.source.fallback.is_some() => format!(
                "Found at {path}, but it did not load: every link goes to the fallback browser."
            ),
            None => format!(
                "Found at {path}; runs first and owns the default browser. {}",
                edited(config.modified, now)
            ),
        };
        FinickyConfig {
            path: path.into(),
            summary: summary.into(),
            found: found.into(),
        }
    });
    let active = if facts.source.config.is_some() {
        RulesSource::Finicky
    } else {
        RulesSource::Visual
    };
    Configuration {
        active,
        can_switch: false,
        visual_summary: visual_summary.into(),
        finicky,
        import: None,
        shorteners: shorteners(facts),
        rules_path: facts.rules_path.as_deref().map_or_else(
            || SharedString::from("no data folder"),
            |path| display_path(path, home).into(),
        ),
        rules_status: rules_status.into(),
    }
}

/// The hosts expanded before matching: the loaded config's list, else the
/// engine's built-in one, which is what `rules.json` alone gets.
fn shorteners(facts: &Facts) -> Vec<SharedString> {
    match facts
        .config
        .as_ref()
        .and_then(|config| config.shorteners.as_ref())
    {
        Some(domains) => domains.iter().map(|host| host.clone().into()).collect(),
        None => DEFAULT_SHORTENERS
            .iter()
            .map(|host| (*host).into())
            .collect(),
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// "edited 5 min ago", the Rules banner's wording, from a file's mtime.
fn edited(modified: Option<SystemTime>, now: SystemTime) -> String {
    let Some(age) = modified.and_then(|at| now.duration_since(at).ok()) else {
        return "edited at an unknown time".to_owned();
    };
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "edited just now.".into(),
        1..60 => format!("edited {minutes} min ago."),
        60..1440 => format!("edited {} hr ago.", minutes / 60),
        _ => match minutes / 1440 {
            1 => "edited yesterday.".into(),
            days => format!("edited {days} days ago."),
        },
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    match request {
        SettingsRequest::RevealRules => {
            let environment = discovery::Environment::current();
            match discovery::rules_path(&environment) {
                Some(path) if path.exists() => cx.reveal_path(&path),
                _ => open_data_folder(cx),
            }
        }
        SettingsRequest::OpenDataFolder => open_data_folder(cx),
        SettingsRequest::ReloadRules => {
            if let Some(router) = settings_store::router(cx) {
                let _ = router.refresh();
            }
            refresh(cx);
            // The reload lands on the dispatch thread; read it back once
            // it has.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(REREAD_AFTER).await;
                cx.update(refresh);
            })
            .detach();
        }
        _ => return false,
    }
    true
}

/// Shows the data directory in the file manager, creating it (owner only)
/// when nothing has written there yet.
fn open_data_folder(cx: &App) {
    let Some(dir) = pwrroute_os::data_dir() else {
        eprintln!("pwrroute: settings: no data directory to open");
        return;
    };
    if !dir.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        if let Err(error) = builder.create(&dir) {
            eprintln!("pwrroute: cannot create {}: {error}", dir.display());
            return;
        }
    }
    cx.open_with_system(&dir);
}

/// Reads the model again into the open window, when there is one, with
/// Diagnostics, which reports the same router.
fn refresh(cx: &mut App) {
    if !settings::is_open(cx) {
        return;
    }
    let configuration = model(cx);
    settings::update(cx, move |model| model.configuration = configuration);
    super::system_host::refresh_diagnostics(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> Facts {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        fs::write(&path, text).unwrap();
        Facts {
            rules_path: Some(path.clone()),
            rules: Some(RulesFile::load(&path).map_err(|e| format!("{e:#}"))),
            rules_modified: Some(SystemTime::now() - Duration::from_secs(5 * 60)),
            home: Some(dir.path().to_path_buf()),
            ..Facts::default()
        }
    }

    #[test]
    fn rules_json_alone_is_the_visual_source() {
        let facts = rules(
            r#"{ "defaultBrowser": "Safari",
                 "rewrites": [{ "match": "x.com/*", "url": { "host": "twitter.com" } }],
                 "rules": [{ "match": "a/*", "browser": "Safari" }, { "match": "b/*", "browser": "Safari" }] }"#,
        );
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.active, RulesSource::Visual);
        assert!(!model.can_switch);
        assert_eq!(
            model.visual_summary,
            "rules.json \u{00b7} 1 rewrite \u{00b7} 2 handlers"
        );
        assert_eq!(model.rules_status, "Loaded \u{00b7} edited 5 min ago.");
        assert_eq!(model.rules_path, "~/rules.json");
        assert!(model.finicky.is_none());
        assert!(model.import.is_none());
        assert!(model.shorteners.iter().any(|host| host == "bit.ly"));
    }

    #[test]
    fn a_config_in_charge_is_the_finicky_source() {
        let mut facts = rules(r#"{ "rules": [] }"#);
        let home = facts.home.clone().unwrap();
        let config = home.join(".finicky.ts");
        facts.source = RoutingSource {
            config: Some(config.clone()),
            rules: None,
            fallback: None,
            error: None,
        };
        facts.config = Some(ConfigFacts {
            path: config,
            counts: Some((2, 6)),
            shorteners: None,
            modified: Some(SystemTime::now() - Duration::from_secs(3 * 3600)),
        });
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.active, RulesSource::Finicky);
        let finicky = model.finicky.unwrap();
        assert_eq!(finicky.path, "~/.finicky.ts");
        assert_eq!(
            finicky.summary,
            "~/.finicky.ts \u{00b7} 2 rewrites \u{00b7} 6 handlers"
        );
        assert_eq!(
            finicky.found,
            "Found at ~/.finicky.ts; runs first and owns the default browser. edited 3 hr ago."
        );
        assert_eq!(
            model.visual_summary,
            "rules.json \u{00b7} 0 rewrites \u{00b7} 0 handlers"
        );

        // A reload that failed keeps the last load and says so.
        facts.source.error = Some("SyntaxError: unexpected token".into());
        let model = build(&facts, SystemTime::now());
        assert!(
            model
                .finicky
                .unwrap()
                .found
                .contains("The last reload failed: SyntaxError: unexpected token.")
        );
    }

    #[test]
    fn the_shortener_list_is_the_engines_in_charge() {
        // rules.json alone, or a config that did not load: the built-in
        // list, as the engine applies it.
        let model = build(&Facts::default(), SystemTime::now());
        assert_eq!(model.shorteners.len(), DEFAULT_SHORTENERS.len());
        assert!(model.shorteners.iter().any(|host| host == "bit.ly"));
        let mut facts = rules(r#"{ "rules": [] }"#);
        let config = facts.home.clone().unwrap().join(".finicky.js");
        facts.source = RoutingSource {
            config: Some(config.clone()),
            ..RoutingSource::default()
        };
        facts.config = Some(ConfigFacts {
            path: config,
            counts: None,
            shorteners: None,
            modified: None,
        });
        assert!(
            build(&facts, SystemTime::now())
                .shorteners
                .iter()
                .any(|host| host == "bit.ly")
        );

        // A config's own list replaces it; an empty one is expansion off.
        let config = facts.config.as_mut().unwrap();
        config.shorteners = Some(vec!["go.corp.example".into()]);
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.shorteners, ["go.corp.example"]);
        facts.config.as_mut().unwrap().shorteners = Some(Vec::new());
        assert!(build(&facts, SystemTime::now()).shorteners.is_empty());
    }

    #[test]
    fn a_missing_or_broken_rules_file_says_so() {
        let model = build(&Facts::default(), SystemTime::now());
        assert_eq!(model.visual_summary, "rules.json \u{00b7} not written yet");
        assert_eq!(
            model.rules_status,
            "No rules.json yet: the Rules view writes it with the first rule."
        );
        assert_eq!(model.rules_path, "no data folder");

        let facts = rules("{ not json");
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.visual_summary, "rules.json \u{00b7} did not load");
        assert!(model.rules_status.starts_with("Not loaded: "));
        assert!(
            model
                .rules_status
                .ends_with("The last good snapshot stays in charge.")
        );
    }
}
