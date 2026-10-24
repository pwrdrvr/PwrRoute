//! What the rule editor's pickers offer (design: Rule Editor Pickers 3a to
//! 4c): the browsers the scan found with their profiles, the apps the
//! rules name that the scan did not find, and the apps rules require links
//! to come from. Plain data over the scan (`PwrRoute::browsers` and
//! `PwrRoute::installed`) and the edited handlers; `editor.rs` draws it.
//! A value the scan cannot match is never rewritten here: the editor shows
//! it as written, with a warning, until the person picks something else.

use crate::activity_actions::BrowserChoice;
use crate::overview_model::Installed;
use crate::rules_model::Handler;

/// One row of the Open in list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserEntry {
    /// What a click writes to the rule's `browser`.
    pub name: String,
    /// "Personal · Work", "4 profiles", a bundle id, "not installed".
    pub hint: Option<String>,
    /// Named by a rule, and the scan found nothing it names: dimmed.
    pub missing: bool,
}

/// The Open in list: what is on this machine, then what the rules name
/// that the scan did not find. "Other app…" always follows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserMenu {
    pub detected: Vec<BrowserEntry>,
    pub named: Vec<BrowserEntry>,
}

impl BrowserMenu {
    /// `browsers` is the scan less the hidden apps, `installed` the whole
    /// scan, `scanned` whether it ran (before it does, nothing is drawn as
    /// missing).
    pub fn build(
        browsers: &[BrowserChoice],
        installed: &[Installed],
        handlers: &[Handler],
        scanned: bool,
    ) -> Self {
        let detected = browsers
            .iter()
            .map(|choice| BrowserEntry {
                name: choice.name.clone(),
                hint: crate::widgets::profiles_hint(&choice.profiles),
                missing: false,
            })
            .collect::<Vec<_>>();
        let mut named: Vec<BrowserEntry> = Vec::new();
        for handler in handlers {
            let value = handler.browser.trim();
            if value.is_empty()
                || detected.iter().any(|e| e.name.eq_ignore_ascii_case(value))
                || named.iter().any(|e| e.name.eq_ignore_ascii_case(value))
            {
                continue;
            }
            let found = installed_named(value, installed);
            let (hint, missing) = match found {
                // A rule names an app the scan found by its bundle id, or
                // a hidden one: it is here, by another name.
                Some(app) if !app.name.eq_ignore_ascii_case(value) => {
                    (Some(app.name.clone()), false)
                }
                Some(_) => (None, false),
                None if scanned => (Some("not installed".to_owned()), true),
                None => (None, false),
            };
            named.push(BrowserEntry {
                name: value.to_owned(),
                hint,
                missing,
            });
        }
        Self { detected, named }
    }
}

/// The app the scan found that `value` names: its name or bundle id,
/// ignoring case, as the launcher matches them.
pub fn installed_named<'a>(value: &str, installed: &'a [Installed]) -> Option<&'a Installed> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    installed.iter().find(|app| {
        app.name.eq_ignore_ascii_case(value)
            || app
                .bundle_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(value))
    })
}

/// Whether the scan has nothing for `value` once it ran: the rule names an
/// app that is not on this machine.
pub fn is_missing(
    value: &str,
    browsers: &[BrowserChoice],
    installed: &[Installed],
    scanned: bool,
) -> bool {
    let value = value.trim();
    scanned
        && !value.is_empty()
        && !browsers.iter().any(|b| b.name.eq_ignore_ascii_case(value))
        && installed_named(value, installed).is_none()
}

/// One profile of the chosen browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileEntry {
    /// The name the browser shows, which a click writes.
    pub name: String,
    /// Its folder ("Profile 1") when that differs from the name: a rule
    /// may name a profile by it.
    pub directory: Option<String>,
}

impl ProfileEntry {
    /// Whether a rule's `profile` value is this profile, by name or folder.
    pub fn is(&self, value: &str) -> bool {
        let value = value.trim();
        self.name.eq_ignore_ascii_case(value)
            || self
                .directory
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(value))
    }
}

/// What the Profile picker can offer for a browser value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Profiles {
    /// No browser chosen yet.
    NoBrowser,
    /// An app the scan does not know: its profiles are typed.
    Unknown,
    /// The scan's browser, with the profiles it found (maybe none).
    Known(Vec<ProfileEntry>),
}

/// The profiles of the browser `value` names, from the scan.
pub fn profiles_of(value: &str, browsers: &[BrowserChoice], installed: &[Installed]) -> Profiles {
    let value = value.trim();
    if value.is_empty() {
        return Profiles::NoBrowser;
    }
    let app = installed_named(value, installed);
    let names = browsers
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(value))
        .map(|b| b.profiles.clone())
        .or_else(|| app.map(|app| app.profiles.clone()));
    let Some(names) = names else {
        return Profiles::Unknown;
    };
    let directories = app.map(|app| app.directories.as_slice()).unwrap_or(&[]);
    Profiles::Known(
        names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let directory = directories
                    .get(i)
                    .filter(|d| !d.is_empty() && !d.eq_ignore_ascii_case(&name))
                    .cloned();
                ProfileEntry { name, directory }
            })
            .collect(),
    )
}

/// The apps the rules require links to come from, each once, less the
/// ones handler `except` already names: what "+ app" offers.
pub fn named_openers(handlers: &[Handler], except: usize) -> Vec<String> {
    let own = handlers
        .get(except)
        .map(|h| h.openers.as_slice())
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for opener in handlers.iter().flat_map(|h| h.openers.iter()) {
        let opener = opener.trim();
        if opener.is_empty()
            || own.iter().any(|o| o.trim().eq_ignore_ascii_case(opener))
            || names.iter().any(|n| n.eq_ignore_ascii_case(opener))
        {
            continue;
        }
        names.push(opener.to_owned());
    }
    names
}

/// How the private-window switch reads for a browser value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivateMode {
    /// "an Incognito window", "an InPrivate window", "a private window".
    Window(String),
    /// The scan's browser can't open one from a link (Safari, Arc).
    Unsupported,
    /// No browser yet, or one the catalog does not know: the plain words.
    Unknown,
}

/// What the switch says for `value`: the browser's own word when the
/// catalog has one; "can't" only for a browser the scan found.
pub fn private_mode(value: &str, detected: bool) -> PrivateMode {
    match crate::overview_model::private_term(value) {
        Some(term) => PrivateMode::Window(crate::overview_model::window_phrase(&term)),
        None if detected => PrivateMode::Unsupported,
        None => PrivateMode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(name: &str, profiles: &[&str]) -> BrowserChoice {
        BrowserChoice {
            name: name.into(),
            profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    fn installed(name: &str, bundle: &str, profiles: &[(&str, &str)]) -> Installed {
        Installed {
            name: name.into(),
            bundle_id: Some(bundle.into()),
            profiles: profiles.iter().map(|(n, _)| (*n).to_owned()).collect(),
            directories: profiles.iter().map(|(_, d)| (*d).to_owned()).collect(),
            hidden: false,
        }
    }

    fn handler(browser: &str, openers: &[&str]) -> Handler {
        let mut rule = crate::fixtures::rules_file().rules.remove(0);
        rule.browser = browser.into();
        rule.opener = openers.iter().map(|o| (*o).to_owned()).collect();
        crate::rules_model::handler(&rule, crate::rules_model::RowBadge::None, Vec::new())
    }

    #[test]
    fn the_list_puts_this_machine_first_then_what_the_rules_name() {
        let browsers = [
            choice("Google Chrome", &["Personal", "Work"]),
            choice("Safari", &[]),
        ];
        let scan = [
            installed("Google Chrome", "com.google.Chrome", &[]),
            installed("Safari", "com.apple.Safari", &[]),
            installed("Example Hidden", "com.example.hidden", &[]),
        ];
        let handlers = [
            handler("Google Chrome", &[]),
            handler("google chrome", &[]),
            handler("Example Notes", &[]),
            handler("com.example.hidden", &[]),
            handler("", &[]),
        ];
        let menu = BrowserMenu::build(&browsers, &scan, &handlers, true);
        assert_eq!(
            menu.detected
                .iter()
                .map(|e| (e.name.as_str(), e.hint.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("Google Chrome", Some("Personal \u{00b7} Work")),
                ("Safari", None)
            ]
        );
        assert_eq!(
            menu.named,
            [
                BrowserEntry {
                    name: "Example Notes".into(),
                    hint: Some("not installed".into()),
                    missing: true,
                },
                BrowserEntry {
                    name: "com.example.hidden".into(),
                    hint: Some("Example Hidden".into()),
                    missing: false,
                },
            ]
        );
        // Before the scan runs, nothing is drawn as missing.
        let early = BrowserMenu::build(&[], &[], &handlers, false);
        assert!(early.named.iter().all(|e| !e.missing && e.hint.is_none()));
        assert!(is_missing("Example Notes", &browsers, &scan, true));
        assert!(!is_missing("Example Notes", &browsers, &scan, false));
        assert!(!is_missing("com.apple.safari", &browsers, &scan, true));
    }

    #[test]
    fn profiles_come_from_the_scan_with_their_folders() {
        let browsers = [choice("Microsoft Edge", &["Personal", "Work", "Travel"])];
        let scan = [installed(
            "Microsoft Edge",
            "com.microsoft.edgemac",
            &[
                ("Personal", "Default"),
                ("Work", "Profile 1"),
                ("Travel", "Travel"),
            ],
        )];
        let Profiles::Known(entries) = profiles_of("microsoft edge", &browsers, &scan) else {
            panic!("Edge is known");
        };
        assert_eq!(entries[0].directory.as_deref(), Some("Default"));
        assert_eq!(entries[1].directory.as_deref(), Some("Profile 1"));
        assert_eq!(entries[2].directory, None, "the same as its name");
        assert!(entries[1].is("profile 1") && entries[1].is("Work"));
        assert_eq!(profiles_of("", &browsers, &scan), Profiles::NoBrowser);
        assert_eq!(
            profiles_of("/Applications/Example Viewer.app", &browsers, &scan),
            Profiles::Unknown
        );
        // By bundle id, the scan's app.
        assert!(matches!(
            profiles_of("com.microsoft.edgemac", &browsers, &scan),
            Profiles::Known(ref e) if e.len() == 3
        ));
    }

    #[test]
    fn plus_app_offers_the_apps_other_rules_name() {
        let handlers = [
            handler("Firefox", &["Example Chat"]),
            handler("Firefox", &["example chat", "com.example.mail"]),
            handler("Firefox", &["com.example.mail"]),
        ];
        assert_eq!(named_openers(&handlers, 2), ["Example Chat"]);
        assert_eq!(
            named_openers(&handlers, 9),
            ["Example Chat", "com.example.mail"]
        );
    }

    #[test]
    fn the_private_switch_uses_the_browsers_word() {
        assert_eq!(
            private_mode("Microsoft Edge", true),
            PrivateMode::Window("an InPrivate window".into())
        );
        assert_eq!(
            private_mode("Google Chrome", true),
            PrivateMode::Window("an Incognito window".into())
        );
        assert_eq!(private_mode("Safari", true), PrivateMode::Unsupported);
        assert_eq!(private_mode("Example Viewer", false), PrivateMode::Unknown);
    }
}
