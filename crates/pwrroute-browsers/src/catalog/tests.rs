//! Validates `data/browsers.json`. A failure here names the entry and field.

use std::collections::HashMap;

use serde::Deserialize;

use super::*;
use crate::env::{split_token, token_allowed};

fn catalog() -> &'static Catalog {
    Catalog::builtin()
}

const PLATFORMS: [Platform; 3] = [Platform::MacOs, Platform::Windows, Platform::Linux];

fn templates(b: &BrowserInfo, platform: Platform) -> Vec<&str> {
    let mut t: Vec<&str> = b
        .profile_templates(platform)
        .iter()
        .map(String::as_str)
        .collect();
    if platform == Platform::Windows
        && let Some(w) = &b.windows
    {
        t.extend(w.exe.iter().map(String::as_str));
    }
    t
}

#[test]
fn ids_and_names_are_unique_and_well_formed() {
    let mut seen = HashMap::new();
    for b in catalog().browsers() {
        assert!(
            !b.id.is_empty()
                && b.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{}: id must be kebab-case",
            b.id
        );
        assert!(!b.name.trim().is_empty(), "{}: empty name", b.id);
        for key in [&b.id, &b.name] {
            let key = key.to_ascii_lowercase();
            if let Some(other) = seen.insert(key.clone(), &b.id)
                && *other != b.id
            {
                panic!("{key:?} is used by both {other} and {}", b.id);
            }
        }
    }
}

#[test]
fn every_entry_has_a_platform_and_sources() {
    for b in catalog().browsers() {
        assert!(
            PLATFORMS.iter().any(|p| b.supports(*p)),
            "{}: no platform section",
            b.id
        );
        assert!(!b.sources.is_empty(), "{}: no sources", b.id);
        for s in &b.sources {
            assert!(
                s.starts_with("https://"),
                "{}: source {s:?} is not a URL",
                b.id
            );
        }
    }
}

#[test]
fn singleton_protocols_are_explicit_and_sourced() {
    for b in catalog().browsers() {
        let Some(protocol) = b.singleton_protocol else {
            continue;
        };
        assert_eq!(b.engine, BrowserEngine::Chromium, "{}", b.id);
        assert!(b.macos.is_some(), "{}", b.id);
        assert!(
            b.sources
                .iter()
                .any(|s| s.ends_with("/process_singleton_posix.cc")),
            "{}: no singleton protocol source",
            b.id
        );
        if protocol == SingletonProtocol::Edge {
            // Edge's header has no public source: the capture is the
            // evidence, and singleton.rs's tests replay it.
            let notes = b.notes.as_deref().unwrap_or_default();
            assert!(
                notes.contains("edge-156-singleton-message.bin"),
                "{}: Edge's protocol must cite its capture",
                b.id
            );
        }
    }
    // Every Edge channel speaks Edge's protocol.
    for id in ["edge", "edge-beta", "edge-dev", "edge-canary"] {
        assert_eq!(
            catalog().find(id).unwrap().singleton_protocol,
            Some(SingletonProtocol::Edge),
            "{id}"
        );
    }
    // Being a Chromium browser does not establish wire compatibility.
    assert_eq!(catalog().find("brave").unwrap().singleton_protocol, None);
}

/// Every identifier a config can use finds its own entry, so no entry
/// shadows another (Chrome's ids vs Chrome Beta's, Firefox's vs its
/// Developer Edition's).
#[test]
fn every_identifier_finds_its_own_entry() {
    for b in catalog().browsers() {
        let mut names = vec![b.id.clone(), b.name.clone()];
        names.extend(b.aliases.iter().cloned());
        if let Some(m) = &b.macos {
            names.extend(m.bundle_id.clone());
            names.push(m.app_name.clone());
        }
        if let Some(l) = &b.linux {
            names.extend(l.desktop_ids.iter().cloned());
        }
        if let Some(p) = b.windows.as_ref().and_then(|w| w.prog_id.as_ref()) {
            names.push(if p.ends_with('-') {
                format!("{p}0123456789ABCDEF")
            } else {
                p.clone()
            });
        }
        for name in names {
            let found = catalog().find(&name).map(|f| f.id.as_str());
            assert_eq!(found, Some(b.id.as_str()), "{name:?} should find {}", b.id);
        }
    }
}

#[test]
fn bundle_ids_find_only_macos_entries() {
    for b in catalog().browsers() {
        if let Some(id) = b.macos.as_ref().and_then(|m| m.bundle_id.as_deref()) {
            let found = catalog().find_bundle_id(id).map(|f| f.id.as_str());
            assert_eq!(found, Some(b.id.as_str()), "{id}");
        }
    }
    // Names and other platforms' ids are not bundle ids.
    for name in [
        "Google Chrome",
        "chrome",
        "ChromeHTML",
        "google-chrome.desktop",
    ] {
        assert!(catalog().find_bundle_id(name).is_none(), "{name}");
    }
    let old = catalog().find_bundle_id("io.gitlab.librewolf-community.librewolf");
    assert_eq!(old.map(|b| b.id.as_str()), Some("librewolf"));
}

#[test]
fn paths_find_their_own_entry() {
    let env = Env::fixture(Platform::Windows, "/fixture");
    for b in catalog().browsers() {
        if let Some(w) = &b.windows {
            for t in &w.exe {
                let path = env.expand(t).unwrap();
                let found = catalog().find_by_path(Platform::Windows, &path);
                assert_eq!(found.map(|f| f.id.as_str()), Some(b.id.as_str()), "{t}");
            }
        }
        if let Some(m) = &b.macos {
            let app = Path::new("/Applications").join(format!("{}.app", m.app_name));
            let found = catalog().find_by_path(Platform::MacOs, &app);
            assert_eq!(found.map(|f| f.id.as_str()), Some(b.id.as_str()));
        }
        if let Some(l) = &b.linux {
            for x in l.desktop_ids.iter().chain(&l.executables) {
                let path = Path::new("/usr/bin").join(x);
                let found = catalog().find_by_path(Platform::Linux, &path);
                assert_eq!(found.map(|f| f.id.as_str()), Some(b.id.as_str()), "{x}");
            }
        }
    }
}

#[test]
fn templates_use_their_platforms_tokens() {
    for b in catalog().browsers() {
        for platform in PLATFORMS {
            for t in templates(b, platform) {
                let (token, rest) =
                    split_token(t).unwrap_or_else(|| panic!("{}: {t:?} has no base token", b.id));
                assert!(
                    token_allowed(token, platform),
                    "{}: {{{token}}} is not a {platform:?} token in {t:?}",
                    b.id
                );
                assert!(
                    !rest.is_empty() && !t.contains('\\') && !t.ends_with('/'),
                    "{}: {t:?} must be a relative /-separated path after the token",
                    b.id
                );
            }
        }
    }
}

#[test]
fn per_platform_fields_are_well_formed() {
    for b in catalog().browsers() {
        if let Some(w) = &b.windows {
            assert!(!w.exe.is_empty(), "{}: no Windows exe", b.id);
            for e in &w.exe {
                assert!(e.ends_with(".exe"), "{}: {e:?}", b.id);
            }
        }
        if let Some(l) = &b.linux {
            assert!(
                !l.desktop_ids.is_empty() || !l.executables.is_empty(),
                "{}: Linux needs a desktop id or an executable",
                b.id
            );
            for d in &l.desktop_ids {
                assert!(d.ends_with(".desktop"), "{}: {d:?}", b.id);
            }
            for x in &l.executables {
                assert!(!x.contains('/'), "{}: {x:?} must be a bare name", b.id);
            }
        }
        if let Some(m) = &b.macos {
            assert!(
                m.bundle_id.as_ref().is_none_or(|id| id.contains('.'))
                    && !m.app_name.ends_with(".app"),
                "{}: macOS ids",
                b.id
            );
            if m.bundle_id.is_none() {
                assert!(
                    b.notes.as_ref().is_some_and(|n| n.contains("bundle id")),
                    "{}: say why the bundle id is missing",
                    b.id
                );
            }
        }
    }
}

/// Browsers that select profiles on the command line say where their
/// profiles live on every platform they support, and the rest say nothing.
#[test]
fn profile_directories_match_the_profile_flag() {
    for b in catalog().browsers() {
        for platform in PLATFORMS {
            if !b.supports(platform) {
                continue;
            }
            let dirs = b.profile_templates(platform);
            if b.profile_flag() == ProfileFlag::None {
                assert!(dirs.is_empty(), "{}: profiles without a flag", b.id);
            } else {
                assert!(!dirs.is_empty(), "{}: no {platform:?} profiles", b.id);
            }
        }
    }
}

/// Every browser says how it opens a private window: the engine's flag
/// unless the entry overrides it. The overrides are the ones the launcher
/// is documented with, and a Chromium browser that cannot pick a profile
/// from the command line says explicitly whether it can go private.
#[test]
fn private_window_flags_match_the_engine() {
    let flag = |id: &str| catalog().find(id).unwrap().private_flag();
    for b in catalog().browsers() {
        let expected = match b.engine {
            BrowserEngine::Chromium if b.id.starts_with("edge") => PrivateFlag::InPrivate,
            BrowserEngine::Chromium if b.profile_flag() == ProfileFlag::None => {
                assert!(
                    b.private_flag.is_some(),
                    "{}: say whether it can go private",
                    b.id
                );
                b.private_flag.unwrap()
            }
            BrowserEngine::Chromium => PrivateFlag::Incognito,
            BrowserEngine::Firefox => PrivateFlag::PrivateWindow,
            BrowserEngine::WebKit => PrivateFlag::None,
        };
        assert_eq!(b.private_flag(), expected, "{}", b.id);
        if let Some(arg) = b.private_flag().arg() {
            assert!(arg.starts_with("--"), "{}: {arg:?}", b.id);
            assert_eq!(PrivateFlag::from_arg(arg), Some(b.private_flag()));
        }
    }
    for id in ["chrome", "chromium", "brave", "vivaldi", "opera"] {
        assert_eq!(flag(id).arg(), Some("--incognito"), "{id}");
    }
    assert_eq!(flag("edge").arg(), Some("--inprivate"));
    for id in ["firefox", "zen", "librewolf", "waterfox", "floorp"] {
        assert_eq!(flag(id).arg(), Some("--private-window"), "{id}");
    }
    for id in ["safari", "orion", "arc"] {
        assert_eq!(flag(id), PrivateFlag::None, "{id}");
    }
    assert_eq!(PrivateFlag::from_arg("--profile-directory=Default"), None);
}

/// A private window is called what the browser's own menu calls it: the
/// flag's word unless the entry says otherwise, and nothing for a browser
/// without one. An override is never set on a browser that cannot go
/// private.
#[test]
fn private_windows_are_named_as_the_browser_names_them() {
    let label = |id: &str| catalog().find(id).unwrap().private_label();
    for b in catalog().browsers() {
        if b.private_label.is_some() {
            assert_ne!(b.private_flag(), PrivateFlag::None, "{}", b.id);
        }
        assert_eq!(
            b.private_label().is_some(),
            b.private_flag() != PrivateFlag::None,
            "{}",
            b.id
        );
    }
    assert_eq!(label("chrome"), Some("Incognito"));
    assert_eq!(label("edge"), Some("InPrivate"));
    assert_eq!(label("firefox"), Some("Private window"));
    for id in ["brave", "brave-beta", "vivaldi", "opera", "opera-gx"] {
        assert_eq!(label(id), Some("Private window"), "{id}");
    }
    for id in ["safari", "orion", "arc", "dia"] {
        assert_eq!(label(id), None, "{id}");
    }
}

#[derive(Deserialize)]
struct FinickyBrowser {
    config_dir_relative: String,
    id: String,
    #[serde(rename = "type")]
    kind: String,
    app_name: String,
}

/// The catalog keeps every Finicky entry as Finicky had it on macOS.
#[test]
fn covers_finickys_browsers_json() {
    let finicky: Vec<FinickyBrowser> =
        serde_json::from_str(include_str!("../../tests/fixtures/finicky-browsers.json")).unwrap();
    assert_eq!(finicky.len(), 19);
    for f in &finicky {
        let b = catalog()
            .find(&f.id)
            .unwrap_or_else(|| panic!("{} is missing", f.id));
        assert!(b.finicky, "{}: finicky flag", b.id);
        let m = b.macos.as_ref().unwrap();
        assert_eq!(m.bundle_id.as_deref(), Some(f.id.as_str()), "{}", b.id);
        assert_eq!(m.app_name, f.app_name, "{}", b.id);
        let engine = match f.kind.as_str() {
            "Chromium" => BrowserEngine::Chromium,
            "Firefox" => BrowserEngine::Firefox,
            "Safari" => BrowserEngine::WebKit,
            other => panic!("unknown Finicky type {other}"),
        };
        assert_eq!(b.engine, engine, "{}", b.id);
        let expected: Vec<String> = if f.config_dir_relative.is_empty() {
            Vec::new()
        } else {
            vec![format!("{{app_support}}/{}", f.config_dir_relative)]
        };
        assert_eq!(
            m.profiles, expected,
            "{}: Finicky's profile directory",
            b.id
        );
    }
    let flagged = catalog().browsers().iter().filter(|b| b.finicky).count();
    assert_eq!(
        flagged,
        finicky.len(),
        "only Finicky's entries carry the flag"
    );
}

#[test]
fn prog_id_prefixes() {
    assert!(prog_id_matches(
        "FirefoxURL-",
        "FirefoxURL-308046B0AF4A39CB"
    ));
    assert!(!prog_id_matches("FirefoxURL-", "FirefoxURL-"));
    assert!(prog_id_matches("ChromeHTML", "chromehtml"));
    assert!(!prog_id_matches("ChromeHTML", "ChromeHTML2"));
    assert!(prog_id_matches(
        "ChromeHTML",
        "ChromeHTML.ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    ));
    assert!(!prog_id_matches("ChromeHTML", "ChromeHTML."));
    assert!(!prog_id_matches("ChromeHTML", "Chrome"));
}
