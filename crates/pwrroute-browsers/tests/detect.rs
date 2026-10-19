//! Installed-browser detection against fixture machines.

mod common;

use common::Fixture;
use pwrroute_browsers::{InstalledBrowser, Platform, detect_installed};

fn plist(bundle_id: &str, schemes: &[&str], rank: Option<&str>) -> String {
    let schemes: String = schemes
        .iter()
        .map(|s| format!("<string>{s}</string>"))
        .collect();
    let rank = rank
        .map(|r| format!("<key>LSHandlerRank</key><string>{r}</string>"))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{bundle_id}</string>
<key>CFBundleURLTypes</key><array><dict>{rank}
<key>CFBundleURLSchemes</key><array>{schemes}</array></dict></array>
</dict></plist>"#
    )
}

fn summary(found: &[InstalledBrowser]) -> Vec<(String, Option<String>)> {
    found
        .iter()
        .map(|b| (b.name.clone(), b.browser.map(|e| e.id.clone())))
        .collect()
}

fn pair(name: &str, id: Option<&str>) -> (String, Option<String>) {
    (name.to_owned(), id.map(str::to_owned))
}

#[test]
fn macos_bundles() {
    let fx = Fixture::new(Platform::MacOs);
    let app = |dir: &str, name: &str, body: &str| {
        fx.write(&format!("{dir}/{name}.app/Contents/Info.plist"), body);
    };
    app(
        "Applications",
        "Google Chrome",
        &plist("com.google.Chrome", &["http", "https"], None),
    );
    // Known by bundle id even under another name; duplicates collapse.
    app(
        "home/Applications",
        "Chrome Copy",
        &plist("com.google.Chrome", &["https"], None),
    );
    app(
        "Applications",
        "Example Browser",
        &plist("com.example.browser", &["https"], Some("Alternate")),
    );
    app(
        "Applications",
        "Deep Linker",
        &plist("com.example.links", &["https"], Some("None")),
    );
    app(
        "Applications",
        "Notes",
        &plist("com.apple.Notes", &["notes"], None),
    );
    app(
        "Applications",
        "Finicky",
        &plist("se.johnste.finicky", &["http", "https"], None),
    );
    // A binary Info.plist: known only by the bundle's name.
    app("Applications", "Firefox", "bplist00");
    app("System/Applications", "Unknown Binary", "bplist00");
    app(
        "System/Applications",
        "Safari",
        &plist("com.apple.Safari", &["http", "https"], None),
    );

    let found = detect_installed(&fx.env);
    assert_eq!(
        summary(&found),
        [
            pair("Example Browser", None),
            pair("Firefox", Some("firefox")),
            pair("Google Chrome", Some("chrome")),
            pair("Safari", Some("safari")),
        ]
    );
    assert_eq!(found[1].bundle_id.as_deref(), Some("org.mozilla.firefox"));
    assert_eq!(found[2].location, fx.path("Applications/Google Chrome.app"));
}

#[test]
fn windows_install_paths() {
    let fx = Fixture::new(Platform::Windows);
    fx.write("Program Files/Mozilla Firefox/firefox.exe", "");
    fx.write(
        "home/AppData/Local/Google/Chrome SxS/Application/chrome.exe",
        "",
    );
    fx.write(
        "Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
        "",
    );
    fx.write(
        "Program Files/Google/Chrome Beta/Application/README.txt",
        "",
    );
    let found = detect_installed(&fx.env);
    let mut s = summary(&found);
    s.sort();
    assert_eq!(
        s,
        [
            pair("Firefox", Some("firefox")),
            pair("Google Chrome Canary", Some("chrome-canary")),
            pair("Microsoft Edge", Some("edge")),
        ]
    );
}

#[test]
fn linux_desktop_entries_and_path() {
    let fx = Fixture::new(Platform::Linux);
    let entry = |name: &str, exec: &str, mime: &str| {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nMimeType={mime}\n")
    };
    let https = "text/html;x-scheme-handler/http;x-scheme-handler/https;";
    fx.write(
        "usr/share/applications/firefox.desktop",
        &entry("Firefox Web Browser", "firefox %u", https),
    );
    fx.write(
        "usr/share/applications/surf.desktop",
        &entry("Surf", "surf %u", https),
    );
    fx.write(
        "usr/share/applications/editor.desktop",
        &entry("Editor", "editor %F", "text/plain;"),
    );
    fx.write(
        "usr/share/applications/pwrroute.desktop",
        &entry("PwrRoute", "pwrroute %u", https),
    );
    // Hidden in the user's data dir: hidden everywhere.
    fx.write(
        "usr/share/applications/vivaldi-stable.desktop",
        &entry("Vivaldi", "/usr/bin/vivaldi-stable %U", https),
    );
    fx.write(
        "home/.local/share/applications/vivaldi-stable.desktop",
        "[Desktop Entry]\nType=Application\nName=Vivaldi\nExec=vivaldi\nHidden=true\n",
    );
    // Flatpak exports, found even when XDG_DATA_DIRS leaves them out.
    fx.write(
        "var/lib/flatpak/exports/share/applications/com.brave.Browser.desktop",
        &entry(
            "Brave",
            "/usr/bin/flatpak run --command=brave com.brave.Browser @@u %U @@",
            https,
        ),
    );
    // ...and not brought back by the PATH fallback.
    fx.write("bin/vivaldi-stable", "");
    // A stale copy whose TryExec program is gone.
    fx.write(
        "home/.local/share/applications/google-chrome.desktop",
        "[Desktop Entry]\nType=Application\nName=Google Chrome\n\
         Exec=/usr/bin/google-chrome-stable %U\nTryExec=google-chrome-stable\n",
    );
    // On PATH only.
    fx.write("bin/chromium", "");

    let found = detect_installed(&fx.env);
    assert_eq!(
        summary(&found),
        [
            pair("Firefox", Some("firefox")),
            pair("Surf", None),
            pair("Brave Browser", Some("brave")),
            pair("Chromium", Some("chromium")),
        ]
    );
    assert_eq!(found[3].location, fx.path("bin/chromium"));
}
