//! The producer behind Settings › Browsers: the app's one scan
//! (`browser_scan`), `rules.json`'s handlers for the "in N rules" column
//! and the apps the rules name that the scan did not find, and
//! `settings.json`'s hidden list for the switches. Rescan reads the
//! machine again; a switch is saved and the main window's pickers follow.

use std::path::Path;

use gpui_kit::{App, SharedString};
use pwrroute_engine::{RulesFile, discovery};

use super::model::{BrowserApp, Browsers, DefaultBrowser, SettingsRequest, Usage};
use crate::browser_scan::{self, DetectedApp};
use crate::config_view::display_path;
use crate::settings_store::{self, BrowserSettings};
use crate::widgets::badge_for;
use crate::{default_browser, settings};

/// The pane's model as things stand.
pub fn model(cx: &mut App) -> Browsers {
    let apps = browser_scan::apps(cx);
    build(&apps, &facts(cx), default_browser::card(cx))
}

/// What the model is built from besides the scan: the rules and the
/// hidden list, read fresh each time.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub rules: RulesFile,
    pub hidden: BrowserSettings,
    /// Shortens locations to `~/…`.
    pub home: Option<std::path::PathBuf>,
}

fn facts(cx: &App) -> Facts {
    let rules = discovery::rules_path(&discovery::Environment::current())
        .and_then(|path| RulesFile::load(&path).ok())
        .unwrap_or_default();
    let hidden = settings_store::store(cx)
        .map(|store| store.read(cx).settings().browsers.clone())
        .unwrap_or_default();
    Facts {
        rules,
        hidden,
        home: std::env::home_dir(),
    }
}

/// The model from its parts.
pub fn build(apps: &[DetectedApp], facts: &Facts, default: DefaultBrowser) -> Browsers {
    let detected = apps.iter().map(|app| detected_row(app, facts)).collect();
    Browsers {
        default,
        detected,
        other_apps: other_apps(apps, facts),
    }
}

fn detected_row(app: &DetectedApp, facts: &Facts) -> BrowserApp {
    let path = display_path(&app.location, facts.home.as_deref());
    let location = match &app.bundle_id {
        Some(id) => format!("{id} \u{00b7} {path}"),
        None => path,
    };
    let profiles: Vec<SharedString> = app.profiles.iter().cloned().map(Into::into).collect();
    BrowserApp {
        id: app.id().into(),
        name: app.name.clone().into(),
        badge: badge_for(&app.name),
        location: location.into(),
        note: profiles.is_empty().then(|| "no profiles".into()),
        profiles,
        usage: usage(&facts.rules, |value| app.is_named(value)),
        visible: !facts.hidden.is_hidden(app.id()),
    }
}

/// How the handlers use an app `named` recognizes.
fn usage(rules: &RulesFile, named: impl Fn(&str) -> bool) -> Usage {
    let naming: Vec<_> = rules
        .rules
        .iter()
        .filter(|rule| named(&rule.browser))
        .collect();
    if naming.is_empty() {
        Usage::Unused
    } else {
        Usage::Rules {
            count: naming.len(),
            all_off: naming.iter().all(|rule| !rule.enabled),
        }
    }
}

/// The apps the handlers name that no detected app answers to, one row
/// per distinct value (ignoring case), in the order the rules name them.
fn other_apps(apps: &[DetectedApp], facts: &Facts) -> Vec<BrowserApp> {
    let mut seen: Vec<String> = Vec::new();
    let mut rows = Vec::new();
    for rule in &facts.rules.rules {
        let value = rule.browser.trim();
        if value.is_empty()
            || apps.iter().any(|app| app.is_named(value))
            || seen.iter().any(|s| s.eq_ignore_ascii_case(value))
        {
            continue;
        }
        seen.push(value.to_owned());
        rows.push(other_row(value, facts));
    }
    rows
}

fn other_row(value: &str, facts: &Facts) -> BrowserApp {
    // A path shows its stem; a bundle id or name is shown as written.
    let is_path = value.contains('/') || value.contains('\\');
    let name = if is_path {
        Path::new(value)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(value)
            .to_owned()
    } else {
        value.to_owned()
    };
    let usage = if is_path && !Path::new(value).exists() {
        Usage::NotInstalled
    } else {
        usage(&facts.rules, |named| {
            named.trim().eq_ignore_ascii_case(value)
        })
    };
    BrowserApp {
        id: value.into(),
        name: name.clone().into(),
        badge: badge_for(&name),
        location: display_path(Path::new(value), facts.home.as_deref()).into(),
        profiles: Vec::new(),
        note: Some("named in a rule; not a detected browser".into()),
        usage,
        visible: !facts.hidden.is_hidden(value),
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    match request {
        SettingsRequest::RescanBrowsers => {
            browser_scan::rescan(cx);
            browser_scan::refresh_windows(cx);
            refresh(cx);
        }
        SettingsRequest::SetAppVisible { app, visible } => {
            let hidden = !visible;
            let Some(store) = settings_store::store(cx) else {
                log::warn!("settings: no store; the browser switch is not saved");
                return true;
            };
            let saved = store.update(cx, |store, _| {
                store.update(|settings| settings.browsers.set_hidden(app, hidden))
            });
            if let Err(error) = saved {
                log::warn!("cannot save settings: {error:#}");
            }
            browser_scan::refresh_windows(cx);
            refresh(cx);
        }
        _ => return false,
    }
    true
}

/// Reads the model again into the open window, when there is one.
fn refresh(cx: &mut App) {
    if !settings::is_open(cx) {
        return;
    }
    let browsers = model(cx);
    settings::update(cx, move |model| model.browsers = browsers);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::settings::model::DefaultState;

    fn app(name: &str, bundle_id: Option<&str>, profiles: &[&str]) -> DetectedApp {
        DetectedApp {
            name: name.to_owned(),
            bundle_id: bundle_id.map(str::to_owned),
            location: PathBuf::from(format!("/Applications/{name}.app")),
            profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
            directories: Vec::new(),
        }
    }

    fn default() -> DefaultBrowser {
        DefaultBrowser {
            state: DefaultState::Other("Safari".into()),
            note: None,
        }
    }

    /// `rules.json` in a scratch directory, read back the way the
    /// producer reads the real one.
    fn rules_in(dir: &Path, text: &str) -> RulesFile {
        let path = dir.join("rules.json");
        std::fs::write(&path, text).unwrap();
        RulesFile::load(&path).unwrap()
    }

    #[test]
    fn a_machine_with_only_safari_lists_only_safari() {
        let apps = [app("Safari", Some("com.apple.Safari"), &[])];
        let model = build(&apps, &Facts::default(), default());
        assert_eq!(model.detected.len(), 1);
        let safari = &model.detected[0];
        assert_eq!(safari.name, "Safari");
        assert_eq!(safari.id, "com.apple.Safari");
        assert_eq!(safari.badge, "Sf");
        assert_eq!(
            safari.location,
            "com.apple.Safari \u{00b7} /Applications/Safari.app"
        );
        assert_eq!(safari.note.as_deref(), Some("no profiles"));
        assert_eq!(safari.usage, Usage::Unused);
        assert!(safari.visible);
        assert!(model.other_apps.is_empty());
        assert_eq!(model.default.state, DefaultState::Other("Safari".into()));
    }

    #[test]
    fn rules_count_by_bundle_id_name_or_path_and_say_when_all_are_off() {
        let dir = tempfile::tempdir().unwrap();
        let rules = rules_in(
            dir.path(),
            r#"{ "defaultBrowser": "com.apple.Safari",
                 "rules": [
                   { "match": "github.com/*", "browser": "com.google.chrome" },
                   { "match": "gitlab.com/*", "browser": "Google Chrome", "profile": "Work" },
                   { "match": "docs.google.com/*", "browser": "/Applications/Google Chrome.app", "enabled": false },
                   { "match": "zoom.us/*", "browser": "us.zoom.xos", "enabled": false },
                   { "match": "figma.com/*", "browser": "" },
                   { "match": "*.pdf", "browser": "/Applications/Preview.app" }
                 ] }"#,
        );
        let apps = [
            app(
                "Google Chrome",
                Some("com.google.Chrome"),
                &["Personal", "Work"],
            ),
            app("Safari", Some("com.apple.Safari"), &[]),
        ];
        let facts = Facts {
            rules,
            hidden: BrowserSettings::default(),
            home: Some(PathBuf::from("/Users/someone")),
        };
        let model = build(&apps, &facts, default());
        let chrome = &model.detected[0];
        assert_eq!(
            chrome.usage,
            Usage::Rules {
                count: 3,
                all_off: false
            }
        );
        assert_eq!(chrome.profiles, ["Personal", "Work"]);
        assert_eq!(chrome.note, None);
        // The default browser is not a rule.
        assert_eq!(model.detected[1].usage, Usage::Unused);

        // Apps the rules name that were not detected, once each, in
        // order; an empty browser is no app at all.
        let others: Vec<_> = model
            .other_apps
            .iter()
            .map(|app| (app.id.to_string(), app.name.to_string(), app.usage.clone()))
            .collect();
        assert_eq!(
            others,
            [
                (
                    "us.zoom.xos".to_owned(),
                    "us.zoom.xos".to_owned(),
                    Usage::Rules {
                        count: 1,
                        all_off: true
                    }
                ),
                (
                    "/Applications/Preview.app".to_owned(),
                    "Preview".to_owned(),
                    Usage::NotInstalled
                ),
            ]
        );
        assert_eq!(model.other_apps[0].badge, "Zm");
        assert!(model.other_apps.iter().all(|app| app.visible));
    }

    #[test]
    fn hidden_apps_draw_switched_off() {
        let apps = [
            app("Safari", Some("com.apple.Safari"), &[]),
            app("Firefox", Some("org.mozilla.firefox"), &["default-release"]),
        ];
        let mut hidden = BrowserSettings::default();
        hidden.set_hidden("ORG.mozilla.firefox", true);
        hidden.set_hidden("us.zoom.xos", true);
        let dir = tempfile::tempdir().unwrap();
        let rules = rules_in(
            dir.path(),
            r#"{ "rules": [{ "match": "zoom.us/*", "browser": "us.zoom.xos" }] }"#,
        );
        let facts = Facts {
            rules,
            hidden,
            home: None,
        };
        let model = build(&apps, &facts, default());
        assert!(model.detected[0].visible);
        assert!(!model.detected[1].visible);
        assert!(!model.other_apps[0].visible);
    }

    #[gpui_kit::test]
    fn a_switch_is_saved_to_settings_json_and_leaves_the_pickers(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use crate::settings_store::{Settings, SettingsStore};
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings_store::install(
                SettingsStore::load(Some(dir.path().to_path_buf())),
                None,
                cx,
            );
            browser_scan::install_fixture(
                vec![
                    app("Safari", Some("com.apple.Safari"), &[]),
                    app("Firefox", Some("org.mozilla.firefox"), &["default-release"]),
                ],
                cx,
            );
        });
        let names = |cx: &mut gpui_kit::TestAppContext| {
            cx.update(|cx| {
                browser_scan::choices(cx)
                    .into_iter()
                    .map(|choice| choice.name)
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(names(cx), ["Safari", "Firefox"]);

        let hide = SettingsRequest::SetAppVisible {
            app: "org.mozilla.firefox".into(),
            visible: false,
        };
        assert!(cx.update(|cx| handle_request(&hide, cx)));
        let text = std::fs::read_to_string(Settings::path(dir.path())).unwrap();
        assert!(text.contains("\"org.mozilla.firefox\""), "{text}");
        assert_eq!(names(cx), ["Safari"]);
        // The pane draws it switched off, from the same store.
        let hidden = cx.update(|cx| {
            settings_store::store(cx)
                .unwrap()
                .read(cx)
                .settings()
                .browsers
                .clone()
        });
        let apps = cx.update(browser_scan::apps);
        let facts = Facts {
            hidden,
            ..Facts::default()
        };
        let model = build(&apps, &facts, default());
        assert!(model.detected[0].visible);
        assert!(!model.detected[1].visible);

        let show = SettingsRequest::SetAppVisible {
            app: "org.mozilla.firefox".into(),
            visible: true,
        };
        assert!(cx.update(|cx| handle_request(&show, cx)));
        assert_eq!(names(cx), ["Safari", "Firefox"]);
        let text = std::fs::read_to_string(Settings::path(dir.path())).unwrap();
        assert!(!text.contains("browsers"), "{text}");

        // A request that is not this pane's is left alone.
        assert!(!cx.update(|cx| handle_request(&SettingsRequest::OpenActivity, cx)));
    }

    #[test]
    fn an_app_without_a_bundle_id_goes_by_its_name() {
        let apps = [app("Nameless", None, &[])];
        let model = build(&apps, &Facts::default(), default());
        assert_eq!(model.detected[0].id, "Nameless");
        assert_eq!(model.detected[0].location, "/Applications/Nameless.app");
    }
}
