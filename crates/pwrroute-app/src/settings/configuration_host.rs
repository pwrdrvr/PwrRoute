//! The producer behind Settings › Config Files: which file is in charge
//! as the router reports it (`router::routing_source`), the counts of
//! `rules.json` and of the Finicky config, the importer's summary of that
//! config (discovered even while visual rules are in charge), and the files
//! on disk. Review import… opens the importer (`importer_view`); the source
//! tiles only show the choice, which the importer's Choose source makes.
//!
//! While a config is in charge, Everything else says whose default takes
//! the links nothing claims: `rules.json`'s `precedence`, with the config's
//! default as `describe()` gives it and what PwrRoute's default would
//! change. Its answers are written through the main window's `RulesStore`
//! when one is open (so the Rules screen follows at once), else through a
//! store over the file; never around a store.

use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use gpui_kit::{App, AppContext as _, Entity, Global, SharedString, Window, WindowHandle};
use pwrroute_engine::{
    AppType, BrowserSpec, DEFAULT_SHORTENERS, DefaultHost, Description, Engine, HandlerRule,
    Limits, MatcherSpec, Precedence, Review, RulesFile, Verdict, discovery,
};

use super::model::{
    CONFIG_FILES_SHOWN, ConfigFile, ConfigFileState, Configuration, DefaultChoice, DefaultOwner,
    EverythingElse, FinickyConfig, ImportCandidate, ImportPlan, Piece, RulesSource,
    SettingsRequest, ShortLinks, ShortLinksSource,
};
use crate::app::PwrRoute;
use crate::browser_scan::{self, DetectedApp};
use crate::config_view::{self, display_path};
use crate::router::{self, RoutingSource};
use crate::rules_model::{Destination, Matcher};
use crate::rules_store::{RulesStore, Status};
use crate::widgets::badge_for;
use crate::{default_browser, settings, settings_store};

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
    /// Every Finicky config file on disk, in discovery's order, with when
    /// each was last changed.
    pub config_files: Vec<(PathBuf, Option<SystemTime>)>,
    pub home: Option<PathBuf>,
    /// The browser scan less the apps hidden in Settings: what the
    /// Everything else select offers.
    pub browsers: Vec<DetectedApp>,
}

#[derive(Clone, Debug)]
pub struct ConfigFacts {
    pub path: PathBuf,
    /// Rewrites and handlers, when the config loaded.
    pub counts: Option<(usize, usize)>,
    /// The shortener domains the config expands, when it loaded: its
    /// `urlShorteners`, or the built-in list when it sets none. Empty when
    /// it turned expansion off.
    pub shorteners: Option<Vec<String>>,
    pub modified: Option<SystemTime>,
    /// The importer's verdict on each rule, when it loaded.
    pub import: Option<ImportPlan>,
    /// Its `defaultBrowser`, when it loaded.
    pub default: Option<BrowserSpec>,
    /// The first handler (from 1) that matches every link, when one does:
    /// [`catch_all`].
    pub catch_all: Option<usize>,
}

/// The pane's model as things stand.
pub fn model(cx: &mut App) -> Configuration {
    let facts = facts(cx);
    remember(stamp(&facts), cx);
    build(&facts, SystemTime::now())
}

/// The scan less the apps hidden in Settings › Browsers.
fn browsers(cx: &mut App) -> Vec<DetectedApp> {
    let apps = browser_scan::apps(cx);
    let hidden = settings_store::store(cx)
        .map(|store| store.read(cx).settings().browsers.clone())
        .unwrap_or_default();
    apps.into_iter()
        .filter(|app| !hidden.is_hidden(app.id()))
        .collect()
}

fn facts(cx: &mut App) -> Facts {
    let source = router::routing_source();
    let environment = discovery::Environment::current();
    let rules_path = discovery::rules_path(&environment);
    let rules = rules_path
        .as_deref()
        .filter(|path| path.exists())
        .map(|path| RulesFile::load(path).map_err(|error| format!("{error:#}")));
    let rules_modified = rules_path.as_deref().and_then(modified);
    // Counting the config's rules means evaluating it, as the Rules screen
    // does at open; the same load says which short links it expands and
    // what the importer makes of it. While visual rules are in charge, the
    // config discovery finds is still offered to the importer.
    let in_charge = source.config.clone();
    let found = in_charge
        .clone()
        .or_else(|| discovery::find_config(&discovery::config_candidates(&environment)));
    let config = found.map(|path| {
        let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default()).ok();
        let description = engine
            .as_ref()
            .and_then(|engine| engine.describe().ok().flatten());
        let import = engine
            .as_ref()
            .and_then(|engine| engine.review_import(&[]).ok().flatten())
            .zip(description.as_ref())
            .map(|(review, description)| import_plan(description, review));
        ConfigFacts {
            counts: description
                .as_ref()
                .map(|description| (description.rewrites.len(), description.handlers.len())),
            default: description
                .as_ref()
                .map(|description| description.default_browser.clone()),
            catch_all: description
                .as_ref()
                .and_then(|description| catch_all(&description.handlers)),
            shorteners: engine
                .filter(|_| in_charge.is_some())
                .map(|engine| engine.short_links().domains.clone()),
            modified: modified(&path),
            import,
            path,
        }
    });
    let config_files = discovery::config_candidates(&environment)
        .into_iter()
        .filter(|path| path.is_file())
        .take(CONFIG_FILES_SHOWN)
        .map(|path| {
            let modified = modified(&path);
            (path, modified)
        })
        .collect();
    Facts {
        source,
        rules_path,
        rules,
        rules_modified,
        config,
        config_files,
        home: std::env::home_dir(),
        browsers: browsers(cx),
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
                plural(file.rules.len(), "rule")
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
    let precedence = rules_file(facts).map_or(Precedence::Config, |file| file.precedence);
    let finicky = facts.config.as_ref().map(|config| {
        let path = display_path(&config.path, home);
        let summary = match config.counts {
            Some((_, handlers)) => format!("{path} \u{00b7} {}", plural(handlers, "handler")),
            None => format!("{path} \u{00b7} did not load"),
        };
        let order = match precedence {
            Precedence::Config => "runs first and takes everything else",
            Precedence::PwrRoute => "its rewrites run first, then your rules, then its handlers",
        };
        let found = match &facts.source.error {
            _ if facts.source.fallback.is_some() => format!(
                "Found at {path}, but it did not load: every link goes to the fallback browser."
            ),
            Some(error) if facts.source.config_not_loaded => format!(
                "Found at {path}, but it did not load: {error}. Your rules run without it until \
                 it does."
            ),
            Some(error) => format!(
                "Found at {path}. The last reload failed: {error}. The previous load stays in \
                 charge."
            ),
            None => format!(
                "Found at {path}; {order}. {}",
                sentence(&edited(config.modified, now))
            ),
        };
        FinickyConfig {
            path: path.into(),
            summary: summary.into(),
            found: found.into(),
        }
    });
    let import = facts
        .config
        .as_ref()
        .and_then(|config| config.import.clone());
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
        import,
        short_links: short_links(facts, home),
        config_files: config_files(facts, now),
        rules_path: facts.rules_path.as_deref().map_or_else(
            || SharedString::from("no data folder"),
            |path| display_path(path, home).into(),
        ),
        rules_status: rules_status.into(),
        everything_else: facts
            .source
            .config
            .as_deref()
            .map(|config| everything_else(facts, config)),
    }
}

/// `rules.json` as loaded, when it did.
fn rules_file(facts: &Facts) -> Option<&RulesFile> {
    facts.rules.as_ref().and_then(|rules| rules.as_ref().ok())
}

/// The config's `defaultBrowser`, as Everything else names it.
enum ConfigDefault {
    /// A fixed browser, by the name the reader knows.
    Browser(String),
    /// `null`: links its handlers don't claim are not opened.
    Null,
    /// A function, or a config that did not load: no one browser to name.
    Unnamed,
}

pub const FUNCTION_WARNING: &str = "Its defaultBrowser function will no longer run.";
pub const NULL_WARNING: &str = "Unmatched links will now open instead of being dropped.";

/// The Everything else field while the config at `config` is in charge:
/// `rules.json`'s `precedence` and default against the config's own.
fn everything_else(facts: &Facts, config: &Path) -> EverythingElse {
    let path = display_path(config, facts.home.as_deref());
    let rules = rules_file(facts);
    let spec = facts
        .config
        .as_ref()
        .and_then(|config| config.default.as_ref());
    let config_default = match spec {
        Some(BrowserSpec::Browser(fixed)) if fixed.app_type == AppType::None => ConfigDefault::Null,
        Some(BrowserSpec::Browser(fixed)) => {
            ConfigDefault::Browser(app_name(&fixed.name, &facts.browsers))
        }
        Some(BrowserSpec::Function(_)) | None => ConfigDefault::Unnamed,
    };
    let fallback_line = "The browser below still catches config errors, browsers that can't \
                         open, and every link while paused.";
    let (config_label, config_detail, pwrroute_detail) = match &config_default {
        ConfigDefault::Browser(name) => (
            format!("{path}'s default \u{00b7} {name}"),
            format!(
                "{path}'s defaultBrowser handles links its handlers don't claim. {fallback_line}"
            ),
            format!(
                "Your rules run first, then {path}'s handlers. Links nothing claims go here \
                 instead of {name}."
            ),
        ),
        ConfigDefault::Null => (
            format!("{path}'s default"),
            format!(
                "{path}'s defaultBrowser is null, so links its handlers don't claim are not \
                 opened. {fallback_line}"
            ),
            format!("Your rules run first, then {path}'s handlers. Links nothing claims go here."),
        ),
        ConfigDefault::Unnamed => (
            format!("{path}'s default"),
            format!(
                "{path}'s defaultBrowser handles links its handlers don't claim. {fallback_line}"
            ),
            format!("Your rules run first, then {path}'s handlers. Links nothing claims go here."),
        ),
    };
    let default_browser = rules.map_or("", |file| file.default_browser.trim());
    let mut warnings: Vec<SharedString> = Vec::new();
    // PwrRoute's default chosen with none PwrRoute can open (none, or
    // PwrRoute itself): the engine leaves the config's default in place,
    // as the Rules banner says.
    if rules.is_some_and(|file| file.precedence == Precedence::PwrRoute && !file.owns_default()) {
        warnings.push(crate::config_view::NEEDS_DEFAULT.into());
    }
    match (&config_default, spec) {
        (ConfigDefault::Null, _) => warnings.push(NULL_WARNING.into()),
        (_, Some(BrowserSpec::Function(_))) => warnings.push(FUNCTION_WARNING.into()),
        _ => {}
    }
    if let Some(handler) = facts.config.as_ref().and_then(|config| config.catch_all) {
        warnings.push(catch_all_warning(handler).into());
    }
    let mut browsers: Vec<DefaultChoice> = facts
        .browsers
        .iter()
        .map(|app| DefaultChoice {
            value: app.id().to_owned().into(),
            badge: badge_for(&app.name),
            name: app.name.clone().into(),
            profiles: app.profiles.iter().cloned().map(Into::into).collect(),
        })
        .collect();
    let browser = (!default_browser.is_empty()).then(|| {
        facts
            .browsers
            .iter()
            .position(|app| app.is_named(default_browser))
            .unwrap_or_else(|| {
                // Named in rules.json but not found (or hidden): still the
                // choice, by the name the reader knows.
                let name = app_name(default_browser, &[]);
                browsers.push(DefaultChoice {
                    value: default_browser.to_owned().into(),
                    badge: badge_for(default_browser),
                    name: name.into(),
                    profiles: Vec::new(),
                });
                browsers.len() - 1
            })
    });
    let profile = rules
        .map(|file| file.default_profile.trim())
        .filter(|profile| !profile.is_empty())
        .map(|profile| SharedString::from(profile.to_owned()));
    let problem = matches!(facts.rules, Some(Err(_)))
        .then(|| SharedString::from("rules.json did not load, so nothing here can be saved."));
    EverythingElse {
        owner: match rules.map_or(Precedence::Config, |file| file.precedence) {
            Precedence::Config => DefaultOwner::Config,
            Precedence::PwrRoute => DefaultOwner::PwrRoute,
        },
        config_label: config_label.into(),
        config_detail: config_detail.into(),
        pwrroute_detail: pwrroute_detail.into(),
        warnings,
        browsers,
        browser,
        profile,
        problem,
    }
}

/// `browser` as `rules.json` or a config names it (bundle id, name or
/// path), by the name the reader knows: the scanned app's, else the
/// catalog's or the installed app's, never a bundle id when one is known.
fn app_name(browser: &str, apps: &[DetectedApp]) -> String {
    apps.iter().find(|app| app.is_named(browser)).map_or_else(
        || default_browser::browser_name(browser),
        |app| app.name.clone(),
    )
}

/// The first of `handlers` (counting from 1, as the Rules screen numbers
/// them) that matches every link, so nothing after it ever runs: a `*`
/// pattern, a regex that matches anything, or a function that returns
/// `true` whatever it is given. Read from the description, not by running
/// the config.
pub fn catch_all(handlers: &[HandlerRule]) -> Option<usize> {
    handlers
        .iter()
        .position(|handler| handler.matchers.iter().any(matches_everything))
        .map(|index| index + 1)
}

/// The warning for config handler `number` (1-based) matching every link,
/// on the Rules banner, the Overview's callout and here.
pub fn catch_all_warning(number: usize) -> String {
    format!("Config handler {number} matches every link, so PwrRoute's default never applies.")
}

/// A matcher that takes every link: a pattern of nothing but wildcards and
/// separators (`"*"`, `"*://*/*"`), a regex that matches anything, or a
/// function that only returns `true`. The one test every surface uses.
pub fn matches_everything(matcher: &MatcherSpec) -> bool {
    match matcher {
        MatcherSpec::Wildcard(pattern) => {
            let pattern = pattern.trim();
            pattern.contains('*') && pattern.chars().all(|c| matches!(c, '*' | ':' | '/'))
        }
        MatcherSpec::Regex(regex) => {
            // `/source/flags`: the source between the outer slashes.
            let source = regex
                .strip_prefix('/')
                .and_then(|rest| rest.rsplit_once('/'))
                .map_or("", |(source, _)| source);
            matches!(
                source,
                ".*" | "^.*" | ".*$" | "^.*$" | ".+" | "^.+" | "^.+$" | "." | "^" | "(?:)"
            )
        }
        MatcherSpec::Function(function) => function.source.as_deref().is_some_and(returns_true),
        MatcherSpec::Other(_) => false,
    }
}

/// A function whose body only returns `true`: `() => true`,
/// `url => { return true; }`, `function always() { return true }`.
fn returns_true(source: &str) -> bool {
    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    let body = if let Some((_, body)) = compact.split_once("=>") {
        body
    } else if compact.starts_with("function") {
        compact.split_once(')').map_or("", |(_, body)| body)
    } else {
        return false;
    };
    matches!(
        body.trim_end_matches(';'),
        "true" | "!0" | "{returntrue}" | "{returntrue;}" | "(true)"
    )
}

/// The importer card's rows: each rewrite, handler and the default with its
/// verdict, a suggestion as an aside.
fn import_plan(description: &Description, review: Review) -> ImportPlan {
    let row = |what: &str, rule: &crate::rules_model::Rule, verdict: &Verdict| {
        let mut summary = vec![Piece::Text(format!("{what} \u{00b7}").into())];
        let mut function = None;
        for matcher in &rule.matchers {
            match matcher {
                Matcher::Pattern(p) | Matcher::Regex(p) => summary.push(Piece::Tag(p.clone())),
                Matcher::Function(source) => function = Some(source.clone()),
                _ => {}
            }
        }
        let to = match &rule.destination {
            Destination::Browser { name, profile, .. } => match profile {
                Some(profile) => format!("\u{2192} {name} \u{00b7} {profile}"),
                None => format!("\u{2192} {name}"),
            },
            Destination::Rewrite(changes) => changes
                .first()
                .map_or_else(String::new, |c| format!("\u{2192} {} {}", c.kind, c.value)),
            Destination::Function(_) => "\u{2192} fn".to_owned(),
            Destination::Unset(_) => String::new(),
        };
        if let Some(source) = function {
            summary = vec![Piece::Source(format!("{source} {to}").into())];
        } else {
            summary.push(Piece::Text(to.into()));
        }
        if let Verdict::StaysJs {
            suggestion: Some(suggested),
            ..
        } = verdict
        {
            summary.push(Piece::Aside(
                format!(
                    "could be {}; tested, not proved",
                    suggested.rule.patterns.join(" ")
                )
                .into(),
            ));
        }
        ImportCandidate {
            summary,
            importable: matches!(verdict, Verdict::Imports(_)),
        }
    };
    let mut candidates = Vec::new();
    for (rule, reviewed) in description.rewrites.iter().zip(&review.rewrites) {
        candidates.push(row(
            "rewrite",
            &config_view::rewrite_row(rule),
            &reviewed.verdict,
        ));
    }
    for (rule, reviewed) in description.handlers.iter().zip(&review.handlers) {
        candidates.push(row(
            "handler",
            &config_view::handler_row(rule),
            &reviewed.verdict,
        ));
    }
    let default = crate::rules_model::Rule::plain(
        Vec::new(),
        config_view::destination(&description.default_browser),
    );
    let mut last = row("defaultBrowser", &default, &review.default_browser.verdict);
    last.summary.remove(0);
    last.summary.insert(0, Piece::Text("defaultBrowser".into()));
    candidates.push(last);
    ImportPlan { candidates }
}

/// The hosts expanded before matching and whose list they are: the loaded
/// config's `urlShorteners`, else the engine's built-in one, which is what
/// `rules.json` alone gets.
fn short_links(facts: &Facts, home: Option<&Path>) -> ShortLinks {
    let built_in = || -> Vec<SharedString> {
        DEFAULT_SHORTENERS
            .iter()
            .map(|host| (*host).into())
            .collect()
    };
    let Some((config, domains)) = facts
        .config
        .as_ref()
        .and_then(|config| Some((config, config.shorteners.as_ref()?)))
    else {
        // No config in charge, or one that did not load: the router runs
        // rules.json alone, with the built-in list.
        return ShortLinks {
            hosts: built_in(),
            source: ShortLinksSource::BuiltIn { config: None },
        };
    };
    let shown: SharedString = display_path(&config.path, home).into();
    if domains.is_empty() {
        return ShortLinks {
            hosts: Vec::new(),
            source: ShortLinksSource::Off(shown),
        };
    }
    // The engine reads an absent urlShorteners as the built-in list.
    let same = domains.len() == DEFAULT_SHORTENERS.len()
        && DEFAULT_SHORTENERS
            .iter()
            .all(|host| domains.iter().any(|domain| domain == host));
    if same {
        ShortLinks {
            hosts: built_in(),
            source: ShortLinksSource::BuiltIn {
                config: Some(shown),
            },
        }
    } else {
        ShortLinks {
            hosts: domains.iter().map(|host| host.clone().into()).collect(),
            source: ShortLinksSource::Config(shown),
        }
    }
}

/// The Finicky config files on disk, as Config Files lists them: the one
/// the router runs, and why each other one does not.
fn config_files(facts: &Facts, now: SystemTime) -> Vec<ConfigFile> {
    let home = facts.home.as_deref();
    let in_charge = facts.source.config.as_deref();
    let same = |path: &Path| {
        in_charge.is_some_and(|charge| {
            path == charge || fs::canonicalize(path).is_ok_and(|real| real == charge)
        })
    };
    let first: Option<SharedString> = facts
        .config_files
        .iter()
        .find(|(path, _)| same(path))
        .map(|(path, _)| display_path(path, home).into());
    // A config kept in charge by path, where discovery does not look,
    // leads the list too.
    let kept = in_charge.filter(|_| first.is_none()).map(|path| {
        (
            path.to_path_buf(),
            facts.config.as_ref().and_then(|c| c.modified),
        )
    });
    let first = first.or_else(|| {
        kept.as_ref()
            .map(|(path, _)| display_path(path, home).into())
    });
    kept.iter()
        .chain(facts.config_files.iter())
        .take(CONFIG_FILES_SHOWN)
        .map(|(path, modified)| {
            let shown: SharedString = display_path(path, home).into();
            let (state, line) = if same(path) {
                match &facts.source.error {
                    Some(error)
                        if facts.source.config_not_loaded || facts.source.fallback.is_some() =>
                    {
                        (ConfigFileState::Failed, format!("Did not load: {error}"))
                    }
                    _ => (
                        ConfigFileState::InCharge,
                        format!("Loaded \u{00b7} {}", edited(*modified, now)),
                    ),
                }
            } else {
                let why = match &first {
                    Some(first) => format!("{first} is in charge, so this one never runs."),
                    None => "Not used: rules.json alone is in charge.".to_owned(),
                };
                (ConfigFileState::NotUsed, why)
            };
            ConfigFile {
                shown,
                path: path.clone(),
                state,
                line: line.into(),
            }
        })
        .collect()
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// `words` starting a sentence: "Edited 5 min ago."
fn sentence(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
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
        SettingsRequest::RevealConfig(path) => cx.reveal_path(path),
        SettingsRequest::ReviewImport => {
            let environment = discovery::Environment::current();
            let config = router::routing_source()
                .config
                .or_else(|| discovery::find_config(&discovery::config_candidates(&environment)));
            match config {
                Some(path) => crate::importer_view::open(path, false, cx),
                None => log::warn!("settings: no Finicky config to import"),
            }
        }
        SettingsRequest::SetDefaultOwner(_) | SettingsRequest::SetDefaultBrowser { .. } => {
            set_everything_else(request, live_rules_path(), model, cx);
        }
        SettingsRequest::ReloadRules => {
            if let Some(router) = settings_store::router(cx) {
                let _ = router.refresh();
            }
            // Requests run inside the Settings window's own update.
            cx.defer(refresh);
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

fn live_rules_path() -> Option<PathBuf> {
    discovery::rules_path(&discovery::Environment::current())
}

/// The main window, when one is open.
fn main_window(cx: &App) -> Option<WindowHandle<PwrRoute>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<PwrRoute>())
}

/// Makes `edit` to `rules.json` and saves it now. With the main window
/// open, through its store, so the Rules screen, Test a link and the test
/// runs follow at once and nothing it holds unsaved is lost; otherwise
/// through a store over the file at `path`. A store that would refuse the
/// save (the file did not load, or changed on disk under unsaved edits)
/// is left as it was, and the reason comes back.
pub(crate) fn edit_rules(
    path: Option<PathBuf>,
    cx: &mut App,
    edit: impl FnOnce(&mut RulesStore),
) -> Result<(), String> {
    if let Some(main) = main_window(cx) {
        return main
            .update(cx, |app, _, cx| {
                app.sync_with_disk(cx);
                if let Some(reason) = app.store.save_refused() {
                    return Err(reason);
                }
                edit(&mut app.store);
                app.refresh(cx);
                // Only a save made now can fail this change; an earlier
                // failure is the Rules screen's to show.
                let saving = app.store.needs_save();
                app.save_now(cx);
                match app.store.status() {
                    Status::Failed(reason) if saving => Err(reason.clone()),
                    _ => Ok(()),
                }
            })
            .map_err(|error| format!("{error:#}"))?;
    }
    let mut store = RulesStore::load(path);
    if let Some(reason) = store.save_refused() {
        return Err(reason);
    }
    edit(&mut store);
    if store.needs_save() {
        store.save()?;
    }
    Ok(())
}

/// Writes an Everything else answer to `rules.json` at `path` (through
/// the main window's store when one is open), then reads the pane back
/// with `read`: the live producer is [`model`].
pub(crate) fn set_everything_else(
    request: &SettingsRequest,
    path: Option<PathBuf>,
    read: impl FnOnce(&mut App) -> Configuration + 'static,
    cx: &mut App,
) {
    let saved = match request {
        SettingsRequest::SetDefaultOwner(owner) => {
            let precedence = match owner {
                DefaultOwner::Config => Precedence::Config,
                DefaultOwner::PwrRoute => Precedence::PwrRoute,
            };
            edit_rules(path, cx, |store| store.set_precedence(precedence))
        }
        SettingsRequest::SetDefaultBrowser { browser, profile } => edit_rules(path, cx, |store| {
            store.set_default_browser(browser, profile);
        }),
        _ => return,
    };
    settle(saved, read, cx);
}

/// After an Everything else change: the router catches up now rather than
/// at the next link, and the window reads the file back, with the reason
/// when the change was not saved. The request runs inside the Settings
/// window's own update, where the window cannot be updated, so the read
/// back waits until that is done.
fn settle(
    saved: Result<(), String>,
    read: impl FnOnce(&mut App) -> Configuration + 'static,
    cx: &mut App,
) {
    if let Some(router) = settings_store::router(cx) {
        let _ = router.refresh();
    }
    if let Err(reason) = &saved {
        log::warn!("settings: cannot save rules.json: {reason}");
    }
    cx.defer(move |cx| {
        if !settings::is_open(cx) {
            return;
        }
        let mut configuration = read(cx);
        if let (Err(reason), Some(field)) = (saved, configuration.everything_else.as_mut()) {
            field.problem = Some(not_saved(&reason).into());
        }
        settings::update(cx, move |model| model.configuration = configuration);
        super::system_host::refresh_diagnostics(cx);
    });
}

/// The error line for a change that was not saved: "Not saved: " and the
/// store's reason, as one sentence.
pub fn not_saved(reason: &str) -> String {
    format!("Not saved: {}.", reason.trim().trim_end_matches('.'))
}

/// What the pane was last built from, cheaply: the router's word on the
/// source and the two files' times. Another writer (the Rules banner's
/// switch, an editor, an agent) changes one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    source: RoutingSource,
    rules: Option<SystemTime>,
    config: Option<SystemTime>,
}

fn stamp(facts: &Facts) -> Stamp {
    Stamp {
        source: facts.source.clone(),
        rules: facts.rules_modified,
        config: facts.source.config.as_deref().and_then(modified),
    }
}

/// The stamp now, without evaluating the config.
fn current_stamp() -> Stamp {
    let source = router::routing_source();
    let rules = live_rules_path().as_deref().and_then(modified);
    let config = source.config.as_deref().and_then(modified);
    Stamp {
        source,
        rules,
        config,
    }
}

/// Keeps the pane current: the stamp it was built from, and the entity
/// that observes the window coming to the front.
struct Follower {
    seen: Option<Stamp>,
}

struct FollowerGlobal(Entity<Follower>);

impl Global for FollowerGlobal {}

fn follower(cx: &mut App) -> Entity<Follower> {
    if let Some(global) = cx.try_global::<FollowerGlobal>() {
        return global.0.clone();
    }
    let follower = cx.new(|_| Follower { seen: None });
    cx.set_global(FollowerGlobal(follower.clone()));
    follower
}

fn remember(stamp: Stamp, cx: &mut App) {
    follower(cx).update(cx, |follower, _| follower.seen = Some(stamp));
}

/// Reads the pane again whenever the Settings `window` comes to the front
/// and something it was built from changed since: the Rules banner's
/// switch, an editor or an agent may have written `rules.json`, or the
/// router may have reloaded the config.
pub fn follow(window: &mut Window, cx: &mut App) {
    follower(cx).update(cx, |_, cx| {
        cx.observe_window_activation(window, |follower, window, cx| {
            if !window.is_window_active() {
                return;
            }
            let now = current_stamp();
            if follower.seen.as_ref() != Some(&now) {
                follower.seen = Some(now);
                cx.defer(refresh);
            }
        })
        .detach();
    });
}

/// Shows the data directory in the file manager, creating it (owner only)
/// when nothing has written there yet.
fn open_data_folder(cx: &App) {
    let Some(dir) = pwrroute_os::data_dir() else {
        log::warn!("settings: no data directory to open");
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
            log::warn!("cannot create {}: {error}", dir.display());
            return;
        }
    }
    cx.open_with_system(&dir);
}

/// Reads the model again into the open window, when there is one, with
/// Diagnostics, which reports the same router.
pub(crate) fn refresh(cx: &mut App) {
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
            "rules.json \u{00b7} 1 rewrite \u{00b7} 2 rules"
        );
        assert_eq!(model.rules_status, "Loaded \u{00b7} edited 5 min ago.");
        assert_eq!(model.rules_path, "~/rules.json");
        assert!(model.finicky.is_none());
        assert!(model.import.is_none());
        assert!(model.short_links.hosts.iter().any(|host| host == "bit.ly"));
        assert!(model.config_files.is_empty());
    }

    #[test]
    fn a_config_in_charge_is_the_finicky_source() {
        let mut facts = rules(r#"{ "rules": [] }"#);
        let home = facts.home.clone().unwrap();
        let config = home.join(".finicky.ts");
        facts.source = RoutingSource {
            config: Some(config.clone()),
            rules: None,
            config_not_loaded: false,
            fallback: None,
            error: None,
        };
        facts.config = Some(ConfigFacts {
            path: config,
            counts: Some((2, 6)),
            shorteners: None,
            modified: Some(SystemTime::now() - Duration::from_secs(3 * 3600)),
            import: None,
            default: None,
            catch_all: None,
        });
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.active, RulesSource::Finicky);
        let finicky = model.finicky.unwrap();
        assert_eq!(finicky.path, "~/.finicky.ts");
        assert_eq!(finicky.summary, "~/.finicky.ts \u{00b7} 6 handlers");
        assert_eq!(
            finicky.found,
            "Found at ~/.finicky.ts; runs first and takes everything else. Edited 3 hr ago."
        );
        assert_eq!(
            model.visual_summary,
            "rules.json \u{00b7} 0 rewrites \u{00b7} 0 rules"
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

        // One that never loaded is left out, and the rules run without it.
        facts.source.config_not_loaded = true;
        facts.source.rules = Some(home.join("rules.json"));
        let found = build(&facts, SystemTime::now()).finicky.unwrap().found;
        assert_eq!(
            found,
            "Found at ~/.finicky.ts, but it did not load: SyntaxError: unexpected token. Your \
             rules run without it until it does."
        );
        // With nothing to route with, every link falls back.
        facts.source.rules = None;
        facts.source.fallback = facts.source.error.clone();
        let found = build(&facts, SystemTime::now()).finicky.unwrap().found;
        assert!(
            found.ends_with("every link goes to the fallback browser."),
            "{found}"
        );
    }

    #[test]
    fn the_shortener_list_is_the_engines_in_charge() {
        // rules.json alone, or a config that did not load: the built-in
        // list, as the engine applies it.
        let model = build(&Facts::default(), SystemTime::now());
        assert_eq!(model.short_links.hosts.len(), DEFAULT_SHORTENERS.len());
        assert_eq!(
            model.short_links.source,
            ShortLinksSource::BuiltIn { config: None }
        );
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
            import: None,
            default: None,
            catch_all: None,
        });
        let model = build(&facts, SystemTime::now());
        assert!(model.short_links.hosts.iter().any(|host| host == "bit.ly"));
        assert_eq!(
            model.short_links.source,
            ShortLinksSource::BuiltIn { config: None }
        );

        // A loaded config without urlShorteners gets the built-in list.
        facts.config.as_mut().unwrap().shorteners = Some(
            DEFAULT_SHORTENERS
                .iter()
                .map(|host| (*host).to_owned())
                .collect(),
        );
        assert_eq!(
            build(&facts, SystemTime::now()).short_links.source,
            ShortLinksSource::BuiltIn {
                config: Some("~/.finicky.js".into())
            }
        );

        // A config's own list replaces it; an empty one is expansion off.
        facts.config.as_mut().unwrap().shorteners = Some(vec!["go.example.com".into()]);
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.short_links.hosts, ["go.example.com"]);
        assert_eq!(
            model.short_links.source,
            ShortLinksSource::Config("~/.finicky.js".into())
        );
        facts.config.as_mut().unwrap().shorteners = Some(Vec::new());
        let model = build(&facts, SystemTime::now());
        assert!(model.short_links.hosts.is_empty());
        assert_eq!(
            model.short_links.source,
            ShortLinksSource::Off("~/.finicky.js".into())
        );
    }

    #[test]
    fn config_files_say_which_one_runs() {
        let mut facts = rules(r#"{ "rules": [] }"#);
        let home = facts.home.clone().unwrap();
        let (js, ts) = (
            home.join(".finicky.js"),
            home.join(".config/finicky/finicky.ts"),
        );
        facts.config_files = vec![(js.clone(), None), (ts.clone(), None)];

        // Visual rules only: none of them runs.
        let files = build(&facts, SystemTime::now()).config_files;
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|f| f.state == ConfigFileState::NotUsed));
        assert_eq!(files[0].line, "Not used: rules.json alone is in charge.");

        // The first is in charge; the later one never runs.
        facts.source.config = Some(js.clone());
        let files = build(&facts, SystemTime::now()).config_files;
        assert_eq!(files[0].state, ConfigFileState::InCharge);
        assert_eq!(files[0].shown, "~/.finicky.js");
        assert_eq!(files[1].state, ConfigFileState::NotUsed);
        assert_eq!(
            files[1].line,
            "~/.finicky.js is in charge, so this one never runs."
        );

        // One that never loaded failed.
        facts.source.error = Some("SyntaxError".into());
        facts.source.config_not_loaded = true;
        let files = build(&facts, SystemTime::now()).config_files;
        assert_eq!(files[0].state, ConfigFileState::Failed);
        assert_eq!(files[0].line, "Did not load: SyntaxError");

        // A config kept by path, where discovery does not look, leads.
        facts.source = RoutingSource {
            config: Some(home.join("elsewhere/finicky.js")),
            ..RoutingSource::default()
        };
        let files = build(&facts, SystemTime::now()).config_files;
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].state, ConfigFileState::InCharge);
        assert_eq!(
            files[1].line,
            "~/elsewhere/finicky.js is in charge, so this one never runs."
        );
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

    // ---- Everything else ---------------------------------------------------

    fn fixed(name: &str, app_type: AppType) -> BrowserSpec {
        BrowserSpec::Browser(pwrroute_engine::FixedBrowser {
            name: name.into(),
            app_type,
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
        })
    }

    fn function(source: &str) -> pwrroute_engine::FunctionSpec {
        pwrroute_engine::FunctionSpec {
            name: String::new(),
            source: Some(source.into()),
            hostnames: None,
        }
    }

    fn handler(matchers: Vec<MatcherSpec>) -> HandlerRule {
        HandlerRule {
            matchers,
            browser: fixed("Firefox", AppType::AppName),
        }
    }

    fn app(name: &str, bundle_id: &str, profiles: &[&str]) -> DetectedApp {
        DetectedApp {
            name: name.into(),
            bundle_id: Some(bundle_id.into()),
            location: PathBuf::from(format!("/Applications/{name}.app")),
            profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
            directories: Vec::new(),
            profiles_blocked: false,
        }
    }

    /// `~/.finicky.ts` in charge with `default`, over `rules`.
    fn in_charge(rules_text: &str, default: Option<BrowserSpec>) -> Facts {
        let mut facts = rules(rules_text);
        let config = facts.home.clone().unwrap().join(".finicky.ts");
        facts.source = RoutingSource {
            config: Some(config.clone()),
            ..RoutingSource::default()
        };
        facts.config = Some(ConfigFacts {
            path: config,
            counts: Some((2, 5)),
            shorteners: None,
            modified: Some(SystemTime::now() - Duration::from_secs(4 * 60)),
            import: None,
            default,
            catch_all: None,
        });
        facts.browsers = vec![
            app("Google Chrome", "com.google.Chrome", &["Personal", "Work"]),
            app("Safari", "com.apple.Safari", &[]),
        ];
        facts
    }

    fn field_of(facts: &Facts) -> EverythingElse {
        build(facts, SystemTime::now())
            .everything_else
            .expect("the Everything else field")
    }

    #[test]
    fn everything_else_is_only_there_while_a_config_is_in_charge() {
        let facts = rules(r#"{ "defaultBrowser": "com.apple.Safari", "precedence": "pwrroute" }"#);
        assert!(build(&facts, SystemTime::now()).everything_else.is_none());

        let facts = in_charge(
            r#"{ "defaultBrowser": "com.apple.Safari" }"#,
            Some(fixed("Safari", AppType::AppName)),
        );
        assert!(build(&facts, SystemTime::now()).everything_else.is_some());
    }

    #[test]
    fn the_config_owns_everything_else_until_precedence_says_pwrroute() {
        let facts = in_charge(
            r#"{ "defaultBrowser": "com.google.Chrome", "defaultProfile": "Personal" }"#,
            Some(fixed("com.apple.Safari", AppType::BundleId)),
        );
        let model = build(&facts, SystemTime::now());
        let field = model.everything_else.clone().unwrap();
        assert_eq!(field.owner, DefaultOwner::Config);
        // The config's bundle id, by the name the reader knows.
        assert_eq!(
            field.config_label,
            "~/.finicky.ts's default \u{00b7} Safari"
        );
        assert_eq!(
            field.config_detail,
            "~/.finicky.ts's defaultBrowser handles links its handlers don't claim. The browser \
             below still catches config errors, browsers that can't open, and every link while \
             paused."
        );
        assert_eq!(
            field.pwrroute_detail,
            "Your rules run first, then ~/.finicky.ts's handlers. Links nothing claims go here \
             instead of Safari."
        );
        assert!(field.warnings.is_empty());
        assert_eq!(
            field.chosen().map(|c| c.name.as_ref()),
            Some("Google Chrome")
        );
        assert_eq!(field.profile.as_deref(), Some("Personal"));
        assert_eq!(
            model.finicky.unwrap().found,
            "Found at ~/.finicky.ts; runs first and takes everything else. Edited 4 min ago."
        );

        let facts = in_charge(
            r#"{ "defaultBrowser": "com.google.Chrome", "precedence": "pwrroute" }"#,
            Some(fixed("com.apple.Safari", AppType::BundleId)),
        );
        let model = build(&facts, SystemTime::now());
        assert_eq!(model.everything_else.unwrap().owner, DefaultOwner::PwrRoute);
        assert_eq!(
            model.finicky.unwrap().found,
            "Found at ~/.finicky.ts; its rewrites run first, then your rules, then its \
             handlers. Edited 4 min ago."
        );
    }

    #[test]
    fn what_pwrroutes_default_changes_comes_from_the_description() {
        let rules_json = r#"{ "defaultBrowser": "com.google.Chrome", "precedence": "pwrroute" }"#;

        // A function default: no one browser to name, and it stops running.
        let facts = in_charge(
            rules_json,
            Some(BrowserSpec::Function(function("(url) => \"Safari\""))),
        );
        let field = field_of(&facts);
        assert_eq!(field.config_label, "~/.finicky.ts's default");
        assert_eq!(
            field.pwrroute_detail,
            "Your rules run first, then ~/.finicky.ts's handlers. Links nothing claims go here."
        );
        assert_eq!(field.warnings, [FUNCTION_WARNING]);

        // `null`: unmatched links were dropped and will now open.
        let facts = in_charge(rules_json, Some(fixed("", AppType::None)));
        let field = field_of(&facts);
        assert_eq!(field.config_label, "~/.finicky.ts's default");
        assert!(field.config_detail.starts_with(
            "~/.finicky.ts's defaultBrowser is null, so links its handlers don't claim are not \
             opened."
        ));
        assert_eq!(field.warnings, [NULL_WARNING]);

        // A handler that takes every link.
        let mut facts = in_charge(rules_json, Some(fixed("Safari", AppType::AppName)));
        facts.config.as_mut().unwrap().catch_all = Some(5);
        assert_eq!(
            field_of(&facts).warnings,
            ["Config handler 5 matches every link, so PwrRoute's default never applies."]
        );

        // rules.json names no default PwrRoute can open (none, or PwrRoute
        // itself): the select asks for one, and the engine leaves the
        // config's default in place, so the warning says so, as the Rules
        // banner does.
        for rules_json in [
            r#"{ "precedence": "pwrroute" }"#,
            r#"{ "precedence": "pwrroute", "defaultBrowser": "PwrRoute" }"#,
        ] {
            let facts = in_charge(rules_json, Some(fixed("Safari", AppType::AppName)));
            let field = field_of(&facts);
            assert_eq!(
                field.warnings,
                [crate::config_view::NEEDS_DEFAULT],
                "{rules_json}"
            );
        }
        let facts = in_charge(
            r#"{ "precedence": "pwrroute" }"#,
            Some(fixed("Safari", AppType::AppName)),
        );
        assert_eq!(field_of(&facts).browser, None);
        // Under the config's order nothing is missing.
        let facts = in_charge("{}", Some(fixed("Safari", AppType::AppName)));
        assert!(field_of(&facts).warnings.is_empty());
    }

    #[test]
    fn every_catch_all_shape_is_one_test() {
        let handler = |matcher: MatcherSpec| HandlerRule {
            matchers: vec![matcher],
            browser: BrowserSpec::Function(function("x")),
        };
        for everything in ["*", "*://*", "*://*/*", "**"] {
            assert!(
                matches_everything(&MatcherSpec::Wildcard(everything.into())),
                "{everything}"
            );
        }
        for regex in ["/.*/", "/^.*$/i", "/.*$/"] {
            assert!(
                matches_everything(&MatcherSpec::Regex(regex.into())),
                "{regex}"
            );
        }
        assert!(!matches_everything(&MatcherSpec::Wildcard(
            "*.example.com/*".into()
        )));
        assert_eq!(
            catch_all(&[handler(MatcherSpec::Wildcard("*://*/*".into()))]),
            Some(1)
        );
    }

    #[test]
    fn a_default_the_scan_did_not_find_is_still_the_choice() {
        let facts = in_charge(
            r#"{ "defaultBrowser": "org.mozilla.firefox" }"#,
            Some(fixed("Safari", AppType::AppName)),
        );
        let field = field_of(&facts);
        assert_eq!(field.browsers.len(), 3);
        let chosen = field.chosen().unwrap();
        assert_eq!(chosen.value, "org.mozilla.firefox");
        assert_eq!(chosen.name, "Firefox");
        // The scan's apps keep their order, by bundle id.
        assert_eq!(field.browsers[0].value, "com.google.Chrome");
        assert_eq!(field.browsers[0].profiles, ["Personal", "Work"]);
    }

    #[test]
    fn a_rules_file_that_did_not_load_says_nothing_can_be_saved() {
        let mut facts = in_charge("{}", Some(fixed("Safari", AppType::AppName)));
        facts.rules = Some(Err("expected value at line 1".into()));
        let field = field_of(&facts);
        assert_eq!(field.owner, DefaultOwner::Config);
        assert_eq!(
            field.problem.as_deref(),
            Some("rules.json did not load, so nothing here can be saved.")
        );
    }

    #[test]
    fn a_change_not_saved_is_one_sentence() {
        assert_eq!(
            not_saved("rules.json did not load, so it is not overwritten."),
            "Not saved: rules.json did not load, so it is not overwritten."
        );
        assert_eq!(
            not_saved("No configuration directory: edits last for this session."),
            "Not saved: No configuration directory: edits last for this session."
        );
        assert_eq!(
            not_saved("permission denied (os error 13)"),
            "Not saved: permission denied (os error 13)."
        );
    }

    #[test]
    fn a_catch_all_handler_is_found_by_its_matchers() {
        let wildcard = |pattern: &str| MatcherSpec::Wildcard(pattern.into());
        let regex = |source: &str| MatcherSpec::Regex(source.into());
        let func = |source: &str| MatcherSpec::Function(function(source));
        assert_eq!(catch_all(&[]), None);
        assert_eq!(
            catch_all(&[
                handler(vec![wildcard("*.example.com/*")]),
                handler(vec![regex("/^https:\\/\\/a\\.example\\//")]),
                handler(vec![func("(url) => url.host === \"a.example.com\"")]),
                handler(vec![wildcard("b.example.com/*"), wildcard("*")]),
            ]),
            Some(4)
        );
        for everything in [
            wildcard("*"),
            wildcard(" *://* "),
            regex("/.*/"),
            regex("/^.*$/i"),
            func("() => true"),
            func("url => { return true; }"),
            func("function always(url, options) {\n  return true\n}"),
        ] {
            assert_eq!(
                catch_all(&[handler(vec![everything.clone()])]),
                Some(1),
                "{everything:?}"
            );
        }
        for some in [
            wildcard("*.example.com"),
            regex("/example/"),
            func("() => false"),
            func("(url) => url.host.endsWith(\"example.com\") || true"),
            MatcherSpec::Function(pwrroute_engine::FunctionSpec {
                name: "native".into(),
                source: None,
                hostnames: None,
            }),
        ] {
            assert_eq!(catch_all(&[handler(vec![some.clone()])]), None, "{some:?}");
        }
    }
}
