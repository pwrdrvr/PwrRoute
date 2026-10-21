//! Launch plans for every platform, from fixture machines. Nothing here
//! starts a browser: plans are only built, and the one real spawn runs this
//! test binary as a stand-in.

mod common;

use std::{fs, path::PathBuf, time::Duration};

use common::{Fixture, LOCAL_STATE, PROFILES_INI, URL, browser, strings};
use pwrroute_browsers::{LaunchError, LaunchOptions, Launcher, Note, Plan, Platform};
use pwrroute_engine::{AppType, Browser};

fn plan(fx: &Fixture, b: &Browser) -> Plan {
    plan_with(fx, b, LaunchOptions::default())
}

fn plan_with(fx: &Fixture, b: &Browser, options: LaunchOptions) -> Plan {
    Launcher::with_env(fx.env.clone(), options)
        .plan(b)
        .unwrap()
        .expect("a plan")
}

fn open_args(p: &Plan) -> Vec<&str> {
    assert_eq!(p.program, PathBuf::from("/usr/bin/open"));
    p.args.iter().map(String::as_str).collect()
}

fn mac_with_chrome_profiles() -> Fixture {
    let fx = Fixture::new(Platform::MacOs);
    fx.write(
        "home/Library/Application Support/Google/Chrome/Local State",
        LOCAL_STATE,
    );
    fx.write(
        "home/Library/Application Support/Firefox/profiles.ini",
        PROFILES_INI,
    );
    fx
}

#[test]
fn none_means_do_not_open() {
    for platform in [Platform::MacOs, Platform::Windows, Platform::Linux] {
        let fx = Fixture::new(platform);
        let launcher = Launcher::with_env(fx.env.clone(), LaunchOptions::default());
        let b = browser("", AppType::None);
        assert!(launcher.plan(&b).unwrap().is_none());
        assert!(launcher.launch(&b).unwrap().is_none());
    }
}

#[test]
fn macos_by_name_bundle_id_and_path() {
    let fx = Fixture::new(Platform::MacOs);
    let p = plan(&fx, &browser("Google Chrome", AppType::AppName));
    assert_eq!(open_args(&p), ["-a", "Google Chrome", URL]);
    assert!(p.notes.is_empty());

    let p = plan(&fx, &browser("com.apple.Safari", AppType::BundleId));
    assert_eq!(open_args(&p), ["-b", "com.apple.Safari", URL]);

    let p = plan(&fx, &browser("/Applications/Firefox.app", AppType::Path));
    assert_eq!(open_args(&p), ["-a", "/Applications/Firefox.app", URL]);
}

#[test]
fn macos_background() {
    let fx = Fixture::new(Platform::MacOs);
    let mut b = browser("Safari", AppType::AppName);
    b.open_in_background = Some(true);
    assert_eq!(open_args(&plan(&fx, &b)), ["-a", "Safari", "-g", URL]);

    let by_default = LaunchOptions {
        background_by_default: true,
    };
    let b = browser("Safari", AppType::AppName);
    assert_eq!(
        open_args(&plan_with(&fx, &b, by_default)),
        ["-a", "Safari", "-g", URL]
    );
    let mut b = browser("Safari", AppType::AppName);
    b.open_in_background = Some(false);
    assert_eq!(
        open_args(&plan_with(&fx, &b, by_default)),
        ["-a", "Safari", URL]
    );
}

#[test]
fn macos_chromium_profile_by_name_and_directory() {
    let fx = mac_with_chrome_profiles();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.profile = "Personal".into();
    let p = plan(&fx, &b);
    assert_eq!(
        open_args(&p),
        [
            "-a",
            "Google Chrome",
            "-n",
            "--args",
            "--profile-directory=Profile 1",
            URL
        ]
    );
    assert!(p.notes.is_empty());

    // Finicky's fallback: the directory itself.
    b.profile = "Profile 3".into();
    assert!(open_args(&plan(&fx, &b)).contains(&"--profile-directory=Profile 3"));

    // By bundle id and by path, too.
    let mut b = browser("com.google.Chrome", AppType::BundleId);
    b.profile = "Work".into();
    assert!(open_args(&plan(&fx, &b)).contains(&"--profile-directory=Default"));
    let mut b = browser("/Applications/Google Chrome.app", AppType::Path);
    b.profile = "Work".into();
    assert!(open_args(&plan(&fx, &b)).contains(&"--profile-directory=Default"));
}

#[test]
fn macos_firefox_profile() {
    let fx = mac_with_chrome_profiles();
    let mut b = browser("org.mozilla.firefox", AppType::BundleId);
    b.profile = "Work Stuff".into();
    assert_eq!(
        open_args(&plan(&fx, &b)),
        [
            "-b",
            "org.mozilla.firefox",
            "-n",
            "--args",
            "-P",
            "Work Stuff",
            URL
        ]
    );
}

#[test]
fn missing_and_unsupported_profiles_open_without_one() {
    let fx = mac_with_chrome_profiles();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.profile = "Nope".into();
    let p = plan(&fx, &b);
    assert_eq!(open_args(&p), ["-a", "Google Chrome", URL]);
    assert_eq!(
        p.notes,
        [Note::ProfileNotFound {
            profile: "Nope".into(),
            available: strings(&["Work", "Personal", "Side Project"]),
        }]
    );

    // Installed nowhere in the fixture: no Local State, so no profiles.
    let mut b = browser("Brave Browser", AppType::AppName);
    b.profile = "Work".into();
    let p = plan(&fx, &b);
    assert!(
        matches!(&p.notes[..], [Note::ProfileNotFound { available, .. }] if available.is_empty())
    );

    for name in ["Safari", "Some Unknown Browser"] {
        let mut b = browser(name, AppType::AppName);
        b.profile = "Work".into();
        let p = plan(&fx, &b);
        assert_eq!(open_args(&p), ["-a", name, URL]);
        assert_eq!(
            p.notes,
            [Note::ProfileUnsupported {
                profile: "Work".into()
            }]
        );
    }
}

#[test]
fn macos_args_replace_the_url() {
    let fx = mac_with_chrome_profiles();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.args = strings(&["--incognito", URL]);
    assert_eq!(
        open_args(&plan(&fx, &b)),
        ["-a", "Google Chrome", "--args", "--incognito", URL]
    );

    // A config's own `--args`: what precedes it is for `open`, and the
    // profile flag goes after it.
    b.profile = "Work".into();
    b.args = strings(&["-F", "--args", "--incognito", URL]);
    assert_eq!(
        open_args(&plan(&fx, &b)),
        [
            "-a",
            "Google Chrome",
            "-n",
            "-F",
            "--args",
            "--profile-directory=Default",
            "--incognito",
            URL
        ]
    );
}

#[test]
fn private_window_adds_the_catalog_flag() {
    let fx = Fixture::new(Platform::MacOs);
    for (name, flag) in [
        ("Google Chrome", "--incognito"),
        ("Microsoft Edge", "--inprivate"),
        ("Firefox", "--private-window"),
    ] {
        let mut b = browser(name, AppType::AppName);
        b.private = true;
        let p = plan(&fx, &b);
        assert_eq!(
            open_args(&p),
            ["-a", name, "-n", "--args", flag, URL],
            "{name}"
        );
        assert!(p.notes.is_empty(), "{name}");
    }

    // The config's args already carry the flag: it is not doubled.
    let mut b = browser("Google Chrome", AppType::AppName);
    b.private = true;
    b.args = strings(&["--incognito", URL]);
    let p = plan(&fx, &b);
    assert_eq!(
        open_args(&p),
        ["-a", "Google Chrome", "--args", "--incognito", URL]
    );
    assert!(p.notes.is_empty());

    // Safari has no private-window flag: the link still opens, with a note.
    let mut b = browser("Safari", AppType::AppName);
    b.private = true;
    let p = plan(&fx, &b);
    assert_eq!(open_args(&p), ["-a", "Safari", URL]);
    assert_eq!(
        p.notes,
        [Note::PrivateUnsupported {
            browser: "Safari".into()
        }]
    );

    // Without the request, nothing is added and nothing is noted.
    let b = browser("Google Chrome", AppType::AppName);
    assert!(!b.private);
    let p = plan(&fx, &b);
    assert_eq!(open_args(&p), ["-a", "Google Chrome", URL]);
    assert!(p.notes.is_empty());
}

#[test]
fn private_window_on_windows_and_linux() {
    let fx = windows_with_chrome();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.private = true;
    let p = plan(&fx, &b);
    assert!(p.program.ends_with("chrome.exe"));
    assert_eq!(p.args, strings(&["--incognito", URL]));
    assert!(p.notes.is_empty());

    let fx = linux_fixture();
    let mut b = browser("Firefox", AppType::AppName);
    b.private = true;
    let p = plan(&fx, &b);
    assert_eq!(p.program, PathBuf::from("/usr/bin/flatpak"));
    assert_eq!(
        p.args,
        strings(&[
            "run",
            "--branch=stable",
            "--arch=x86_64",
            "--command=firefox",
            "--file-forwarding",
            "org.mozilla.firefox",
            "--private-window",
            URL
        ])
    );
    assert!(p.notes.is_empty());
}

fn windows_with_chrome() -> Fixture {
    let fx = Fixture::new(Platform::Windows);
    fx.write("Program Files/Google/Chrome/Application/chrome.exe", "");
    fx.write(
        "home/AppData/Local/Google/Chrome/User Data/Local State",
        LOCAL_STATE,
    );
    fx.write("home/AppData/Local/Mozilla Firefox/firefox.exe", "");
    fx.write(
        "home/AppData/Roaming/Mozilla/Firefox/profiles.ini",
        PROFILES_INI,
    );
    fx
}

#[test]
fn windows_runs_the_executable_with_profile_args() {
    let fx = windows_with_chrome();
    let chrome = fx.path("Program Files/Google/Chrome/Application/chrome.exe");

    // A macOS config's bundle id or app name finds the Windows install.
    for (name, app_type) in [
        ("com.google.Chrome", AppType::BundleId),
        ("Google Chrome", AppType::AppName),
    ] {
        let mut b = browser(name, app_type);
        b.profile = "Side Project".into();
        let p = plan(&fx, &b);
        assert_eq!(p.program, chrome);
        assert_eq!(p.args, strings(&["--profile-directory=Profile 3", URL]));
        assert!(p.notes.is_empty());
    }

    let mut b = browser("Firefox", AppType::AppName);
    b.profile = "Personal".into();
    b.args = strings(&["-new-window", URL]);
    let p = plan(&fx, &b);
    assert_eq!(
        p.program,
        fx.path("home/AppData/Local/Mozilla Firefox/firefox.exe")
    );
    assert_eq!(p.args, strings(&["-P", "Personal", "-new-window", URL]));
}

#[test]
fn windows_cannot_open_in_background() {
    let fx = windows_with_chrome();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.open_in_background = Some(true);
    let p = plan(&fx, &b);
    assert_eq!(p.args, strings(&[URL]));
    assert_eq!(p.notes, [Note::BackgroundUnsupported]);
}

#[test]
fn windows_channels_and_missing_browsers_are_not_confused() {
    let fx = windows_with_chrome();
    let launcher = Launcher::with_env(fx.env.clone(), LaunchOptions::default());
    for name in ["Google Chrome Beta", "Microsoft Edge", "Nonexistent"] {
        let err = launcher.plan(&browser(name, AppType::AppName)).unwrap_err();
        assert!(
            matches!(&err, LaunchError::NotInstalled { name: n } if n == name),
            "{err}"
        );
    }
    let missing = fx.path("Program Files/Nope/nope.exe");
    let b = browser(&missing.to_string_lossy(), AppType::Path);
    assert!(launcher.plan(&b).is_err());

    // A path outside the catalog runs as is, with no profile support.
    let other = fx.write("Tools/other.exe", "");
    let mut b = browser(&other.to_string_lossy(), AppType::Path);
    b.profile = "Work".into();
    let p = plan(&fx, &b);
    assert_eq!(p.program, other);
    assert_eq!(p.args, strings(&[URL]));
    assert_eq!(
        p.notes,
        [Note::ProfileUnsupported {
            profile: "Work".into()
        }]
    );
}

fn linux_fixture() -> Fixture {
    let fx = Fixture::new(Platform::Linux);
    fx.write(
        "usr/share/applications/google-chrome.desktop",
        "[Desktop Entry]\nType=Application\nName=Google Chrome\n\
         Exec=/usr/bin/google-chrome-stable %U\n\
         MimeType=x-scheme-handler/http;x-scheme-handler/https;\n",
    );
    fx.write("home/.config/google-chrome/Local State", LOCAL_STATE);
    fx.write(
        "var/lib/flatpak/exports/share/applications/org.mozilla.firefox.desktop",
        "[Desktop Entry]\nType=Application\nName=Firefox\n\
         Exec=/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=firefox \
         --file-forwarding org.mozilla.firefox @@u %u @@\n",
    );
    fx.write(
        "home/.var/app/org.mozilla.firefox/.mozilla/firefox/profiles.ini",
        PROFILES_INI,
    );
    fx.write("bin/brave-browser-stable", "");
    fx
}

#[test]
fn linux_runs_the_desktop_entry() {
    let fx = linux_fixture();
    let mut b = browser("Google Chrome", AppType::AppName);
    b.profile = "Work".into();
    let p = plan(&fx, &b);
    assert_eq!(p.program, PathBuf::from("/usr/bin/google-chrome-stable"));
    assert_eq!(p.args, strings(&["--profile-directory=Default", URL]));

    // Flatpak: the profile lives in the sandbox's home.
    let mut b = browser("org.mozilla.firefox", AppType::BundleId);
    b.profile = "Work Stuff".into();
    b.open_in_background = Some(true);
    let p = plan(&fx, &b);
    assert_eq!(p.program, PathBuf::from("/usr/bin/flatpak"));
    assert_eq!(
        p.args,
        strings(&[
            "run",
            "--branch=stable",
            "--arch=x86_64",
            "--command=firefox",
            "--file-forwarding",
            "org.mozilla.firefox",
            "-P",
            "Work Stuff",
            URL
        ])
    );
    assert_eq!(p.notes, [Note::BackgroundUnsupported]);
}

#[test]
fn linux_falls_back_to_path_and_desktop_ids() {
    let fx = linux_fixture();
    let p = plan(&fx, &browser("Brave Browser", AppType::AppName));
    assert_eq!(p.program, fx.path("bin/brave-browser-stable"));
    assert_eq!(p.args, strings(&[URL]));

    // Not in the catalog: a desktop id, then an executable on PATH.
    fx.write(
        "usr/share/applications/org.example.Browser.desktop",
        "[Desktop Entry]\nType=Application\nName=Example\nExec=example-browser --x %u\n",
    );
    let p = plan(&fx, &browser("org.example.Browser", AppType::BundleId));
    assert_eq!(p.program, PathBuf::from("example-browser"));
    assert_eq!(p.args, strings(&["--x", URL]));
    fx.write("bin/surf", "");
    let p = plan(&fx, &browser("surf", AppType::AppName));
    assert_eq!(p.program, fx.path("bin/surf"));

    // A desktop file given as a path.
    let desktop = fx.path("usr/share/applications/google-chrome.desktop");
    let p = plan(&fx, &browser(&desktop.to_string_lossy(), AppType::Path));
    assert_eq!(p.program, PathBuf::from("/usr/bin/google-chrome-stable"));

    let launcher = Launcher::with_env(fx.env.clone(), LaunchOptions::default());
    let err = launcher
        .plan(&browser("Vivaldi", AppType::AppName))
        .unwrap_err();
    assert!(matches!(err, LaunchError::NotInstalled { .. }));
}

#[test]
fn linux_skips_desktop_entries_whose_program_is_gone() {
    let fx = Fixture::new(Platform::Linux);
    fx.write(
        "home/.local/share/applications/chromium.desktop",
        "[Desktop Entry]\nType=Application\nName=Chromium\n\
         Exec=chromium %U\nTryExec=chromium\n",
    );
    fx.write(
        "var/lib/flatpak/exports/share/applications/org.chromium.Chromium.desktop",
        "[Desktop Entry]\nType=Application\nName=Chromium\n\
         Exec=/usr/bin/flatpak run org.chromium.Chromium @@u %U @@\n",
    );
    let p = plan(&fx, &browser("Chromium", AppType::AppName));
    assert_eq!(p.program, PathBuf::from("/usr/bin/flatpak"));
    assert_eq!(p.args, strings(&["run", "org.chromium.Chromium", URL]));
}

#[test]
fn command_line_quotes_for_reading() {
    let fx = Fixture::new(Platform::MacOs);
    let mut b = browser("Google Chrome", AppType::AppName);
    b.url = "https://example.com/".into();
    assert_eq!(
        plan(&fx, &b).command_line(),
        r#"/usr/bin/open -a "Google Chrome" https://example.com/"#
    );
}

/// The stand-in browser. In a normal run there is no `--` in the test
/// binary's arguments and this does nothing; when [`launch_spawns_the_plan`]
/// starts this binary as a "browser", libtest runs only this test and the
/// arguments after `--` (output file first) are recorded.
#[test]
fn fake_browser() {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--") else {
        return;
    };
    let (out, rest) = args[i + 1..].split_first().expect("output path");
    fs::write(out, serde_json::to_string(rest).unwrap()).unwrap();
}

#[test]
fn launch_spawns_the_plan() {
    // Windows plans on Windows; everywhere else the direct-exec path that
    // Linux uses (the macOS path runs `open`, which would start an app).
    let platform = if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    };
    let fx = Fixture::new(platform);
    let out = fx.path("args.json");
    let exe = std::env::current_exe().unwrap();
    let mut b = browser(&exe.to_string_lossy(), AppType::Path);
    let url = r#"https://example.com/private?token=SECRET&text="quoted space""#;
    b.args = vec![
        "--exact".into(),
        "fake_browser".into(),
        "--".into(),
        out.to_string_lossy().into_owned(),
        "--custom=two words".into(),
        url.into(),
    ];
    let launcher = Launcher::with_env(fx.env.clone(), LaunchOptions::default());
    let plan = launcher.launch(&b).unwrap().unwrap();
    assert_eq!(plan.program, exe);

    let mut recorded = None;
    for _ in 0..500 {
        if let Ok(text) = fs::read_to_string(&out)
            && let Ok(args) = serde_json::from_str::<Vec<String>>(&text)
        {
            recorded = Some(args);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(recorded, Some(strings(&["--custom=two words", url])));
}

#[test]
fn errors_do_not_leak_urls() {
    let fx = Fixture::new(Platform::Linux);
    let launcher = Launcher::with_env(fx.env.clone(), LaunchOptions::default());
    let err = launcher
        .launch(&browser("Nonexistent", AppType::AppName))
        .unwrap_err();
    assert!(!err.to_string().contains("example.com"));
}
