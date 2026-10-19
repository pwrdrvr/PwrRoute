//! Profile listing from fixture `Local State` and `profiles.ini` files in
//! each platform's profile directory.

mod common;

use common::{Fixture, LOCAL_STATE, PROFILES_INI};
use pwrroute_browsers::{Catalog, Platform, Profile, list_profiles};

fn names(profiles: &[Profile]) -> Vec<(&str, &str, bool)> {
    profiles
        .iter()
        .map(|p| (p.name.as_str(), p.directory.as_str(), p.default))
        .collect()
}

const CHROME: [(&str, &str, bool); 3] = [
    ("Work", "Default", false),
    ("Personal", "Profile 1", true),
    ("Side Project", "Profile 3", false),
];

const FIREFOX: [(&str, &str, bool); 2] = [
    ("Personal", "Profiles/8fz3m2q7.default-release", false),
    ("Work Stuff", "Profiles/k2x9c1a0.work", true),
];

#[test]
fn chromium_profiles_on_every_platform() {
    let chrome = Catalog::builtin().find("chrome").unwrap();
    for (platform, rel) in [
        (
            Platform::MacOs,
            "home/Library/Application Support/Google/Chrome",
        ),
        (
            Platform::Windows,
            "home/AppData/Local/Google/Chrome/User Data",
        ),
        (Platform::Linux, "home/.config/google-chrome"),
    ] {
        let fx = Fixture::new(platform);
        assert!(list_profiles(chrome, &fx.env).is_empty(), "{platform:?}");
        fx.write(&format!("{rel}/Local State"), LOCAL_STATE);
        assert_eq!(
            names(&list_profiles(chrome, &fx.env)),
            CHROME,
            "{platform:?}"
        );
        assert_eq!(chrome.profile_dir(&fx.env), Some(fx.path(rel)));
    }
}

#[test]
fn firefox_profiles_on_every_platform() {
    let firefox = Catalog::builtin().find("firefox").unwrap();
    for (platform, rel) in [
        (Platform::MacOs, "home/Library/Application Support/Firefox"),
        (Platform::Windows, "home/AppData/Roaming/Mozilla/Firefox"),
        (Platform::Linux, "home/.mozilla/firefox"),
    ] {
        let fx = Fixture::new(platform);
        fx.write(&format!("{rel}/profiles.ini"), PROFILES_INI);
        assert_eq!(
            names(&list_profiles(firefox, &fx.env)),
            FIREFOX,
            "{platform:?}"
        );
    }
}

/// Linux has several candidate directories; the one with a profile index
/// wins, so a Snap install's profiles are found.
#[test]
fn linux_picks_the_directory_that_has_profiles() {
    let firefox = Catalog::builtin().find("firefox").unwrap();
    let fx = Fixture::new(Platform::Linux);
    assert_eq!(
        firefox.profile_dir(&fx.env),
        Some(fx.path("home/.mozilla/firefox"))
    );
    fx.write(
        "home/snap/firefox/common/.mozilla/firefox/profiles.ini",
        PROFILES_INI,
    );
    assert_eq!(
        firefox.profile_dir(&fx.env),
        Some(fx.path("home/snap/firefox/common/.mozilla/firefox"))
    );
    assert_eq!(names(&list_profiles(firefox, &fx.env)), FIREFOX);
}

#[test]
fn browsers_without_a_profile_flag_list_nothing() {
    let safari = Catalog::builtin().find("Safari").unwrap();
    let fx = Fixture::new(Platform::MacOs);
    assert!(safari.profile_dir(&fx.env).is_none());
    assert!(list_profiles(safari, &fx.env).is_empty());
}
