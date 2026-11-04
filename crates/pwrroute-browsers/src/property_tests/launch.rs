use super::*;
use proptest::prelude::*;

fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-zA-Z0-9 ._/-]{0,16}",
        Just("--args".into()),
        Just("--incognito".into()),
        Just("--url=a b;%'\"".into())
    ]
}

fn check(
    app_type: AppType,
    url: String,
    args: Vec<String>,
    flags: Option<Vec<String>>,
    background: bool,
    hand_off: bool,
) -> Result<(), TestCaseError> {
    let browser = Browser {
        name: "Example Browser".into(),
        app_type,
        url,
        args,
        profile: String::new(),
        open_in_background: None,
        private: false,
        privacy: pwrroute_engine::Privacy::Full,
    };
    let has_flags = flags.is_some();
    let mut expected_payload = flags.clone().unwrap_or_default();
    let (_, argv) = macos_plan(&browser, background, hand_off, flags);
    if let Some(i) = argv.iter().position(|a| a == "--args")
        && !argv[i + 1..].is_empty()
    {
        prop_assert!(argv[..i].iter().any(|a| a == "-n"), "{:?}", argv);
    }
    if browser.args.is_empty() {
        // Equal text may belong to separate arguments: a profile flag, the
        // open delimiter, and the URL. The payload must preserve every flag
        // in order, followed by exactly one URL argument.
        expected_payload.push(browser.url.clone());
        let payload_start = if has_flags {
            argv.iter().position(|arg| arg == "--args").unwrap() + 1
        } else {
            // Without profile/config args, open receives its app selector,
            // app name, an optional background option, then the URL.
            2 + usize::from(background)
        };
        prop_assert_eq!(&argv[payload_start..], expected_payload.as_slice());
    }
    Ok(())
}

proptest! {
    #[test]
    fn every_open_argument_survives_a_running_browser(
        app_type in prop::sample::select(vec![AppType::AppName, AppType::BundleId, AppType::Path]),
        url in prop_oneof!["https://example\\.org/[a-z0-9 &;%'\"=-]{0,24}", "--[a-z &;%'\"]{0,16}"],
        args in prop::collection::vec(token(), 0..4),
        flags in prop::option::of(prop::collection::vec(token().prop_filter("profile flags cannot be open's delimiter", |s| s != "--args"), 0..3)),
        background in any::<bool>(), hand_off in any::<bool>(),
    ) { check(app_type, url, args, flags, background, hand_off)?; }
}

#[test]
fn config_arguments_alone_require_a_new_open_instance() {
    check(
        AppType::AppName,
        "https://example.org/".into(),
        vec!["--incognito".into()],
        None,
        false,
        false,
    )
    .unwrap();
}

proptest! {
    #[test]
    fn url_text_can_also_appear_in_flags(
        url in "--[a-z]{0,12}", copies in 0usize..4,
        app_type in prop::sample::select(vec![AppType::AppName, AppType::BundleId, AppType::Path]),
        background in any::<bool>(), hand_off in any::<bool>(),
    ) {
        // The open delimiter cannot be a profile flag, but it can still
        // be the URL text when there are no profile flags to prepend.
        let flags = if url == "--args" { Vec::new() } else { vec![url.clone(); copies] };
        check(app_type, url, Vec::new(), Some(flags), background, hand_off)?;
    }
}

#[test]
fn a_flag_equal_to_the_url_is_preserved_beside_the_url() {
    check(
        AppType::AppName,
        "--".into(),
        Vec::new(),
        Some(vec!["--".into()]),
        false,
        false,
    )
    .unwrap();
}

#[test]
fn delimiter_text_in_the_url_is_still_a_single_payload_argument() {
    for flags in [None, Some(Vec::new())] {
        check(
            AppType::AppName,
            "--args".into(),
            Vec::new(),
            flags,
            false,
            false,
        )
        .unwrap();
    }
}

// The differential needs the actual command builder, not a reimplementation
// of browser_args. This seam plans only; it never spawns open.
pub(crate) fn open_browser_args(name: &str, url: String, args: Vec<String>) -> Vec<String> {
    let b = Browser {
        name: name.into(),
        app_type: AppType::AppName,
        url,
        args,
        profile: String::new(),
        open_in_background: None,
        private: false,
        privacy: pwrroute_engine::Privacy::Full,
    };
    let (_, argv) = macos_plan(
        &b,
        false,
        true,
        Some(vec!["--profile-directory=Profile 1".into()]),
    );
    let split = argv.iter().position(|a| a == "--args").unwrap();
    let mut result = vec![macos_program(&b, name)];
    result.extend_from_slice(&argv[split + 1..]);
    result
}

#[test]
fn socket_admission_requires_a_catalog_browser_with_a_reader_oracle() {
    let root = tempfile::tempdir().unwrap();
    let launcher = Launcher::with_env(
        Env::fixture(Platform::MacOs, root.path()),
        LaunchOptions::default(),
    )
    .with_running(|_| true)
    .with_uptime(|| Some(Duration::from_secs(7)));
    for entry in Catalog::builtin().browsers() {
        let Some(mac) = &entry.macos else { continue };
        let b = Browser {
            name: mac.app_name.clone(),
            app_type: AppType::AppName,
            url: "https://example.org/".into(),
            profile: String::new(),
            args: vec!["--model-argument".into()],
            open_in_background: None,
            private: false,
            privacy: pwrroute_engine::Privacy::Full,
        };
        let plan = launcher.plan(&b).unwrap().unwrap();
        if plan.socket.is_some() {
            let edge = crate::singleton::properties::reader(&entry.id);
            assert_eq!(
                entry.singleton_protocol,
                Some(if edge {
                    SingletonProtocol::Edge
                } else {
                    SingletonProtocol::Chromium
                })
            );
        }
        if entry.engine == BrowserEngine::Chromium && entry.singleton_protocol.is_some() {
            assert!(
                plan.socket.is_some(),
                "{} should try its verified reader",
                entry.id
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn planned_socket_argv_matches_the_actual_open_plan() {
    use proptest::test_runner::TestRunner;
    use std::os::unix::fs::symlink;
    let fixtures: Vec<_> = Catalog::builtin()
        .browsers()
        .iter()
        .filter(|entry| entry.singleton_protocol.is_some())
        .map(|entry| {
            let fake = crate::singleton::fake::Fake::new("data");
            let mut env = Env::fixture(Platform::MacOs, fake.root());
            env.set_var("TMPDIR", fake.temp.to_string_lossy().into_owned());
            let data = entry.profile_dir(&env).unwrap();
            std::fs::create_dir_all(&data).unwrap();
            symlink(
                fake.sock_dir.join("SingletonSocket"),
                data.join("SingletonSocket"),
            )
            .unwrap();
            symlink("4242", data.join("SingletonCookie")).unwrap();
            std::fs::write(
                data.join("Local State"),
                include_str!("../../tests/fixtures/chromium-local-state.json"),
            )
            .unwrap();
            let launcher = Launcher::with_env(env, LaunchOptions::default())
                .with_running(|_| true)
                .with_uptime(|| Some(Duration::from_secs(7)));
            (entry, fake, launcher)
        })
        .collect();
    let strategy = (
        0usize..3,
        any::<bool>(),
        any::<bool>(),
        any::<String>(),
        prop::collection::vec(
            token().prop_filter("custom open options have their own transport", |s| {
                s != "--args" && !s.starts_with("--user-data-dir")
            }),
            0..4,
        ),
    );
    TestRunner::default()
        .run(&strategy, |(kind, background, private, url, args)| {
            for (entry, _fake, launcher) in &fixtures {
                let mac = entry.macos.as_ref().unwrap();
                let (app_type, name) = match kind {
                    0 => (AppType::AppName, mac.app_name.clone()),
                    1 => (AppType::BundleId, mac.bundle_id.clone().unwrap()),
                    _ => (AppType::Path, format!("/Applications/{}.app", mac.app_name)),
                };
                let browser = Browser {
                    name,
                    app_type,
                    url: url.clone(),
                    args: args.clone(),
                    profile: "Personal".into(),
                    open_in_background: Some(background),
                    private,
                    privacy: pwrroute_engine::Privacy::Full,
                };
                let plan = launcher.plan(&browser).unwrap().unwrap();
                let split = plan.args.iter().position(|a| a == "--args").unwrap();
                let mut expected = vec![macos_program(&browser, &mac.app_name)];
                expected.extend_from_slice(&plan.args[split + 1..]);
                let handoff = plan.socket.unwrap().unwrap();
                prop_assert_eq!(&handoff.argv, &expected, "{}", entry.id);
                let h = singleton::header(handoff.protocol, Some(Duration::from_secs(7))).unwrap();
                if let Ok(bytes) = singleton::encode("/", &h, &handoff.argv) {
                    let edge = crate::singleton::properties::reader(&entry.id);
                    prop_assert_eq!(
                        crate::singleton::properties::decode(&bytes, edge),
                        Some(expected)
                    );
                }
            }
            Ok(())
        })
        .unwrap();
}
