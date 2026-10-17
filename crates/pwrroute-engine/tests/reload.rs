//! Hot reload: a new engine is swapped in only once it loads and validates;
//! a failure keeps the engine that worked and says why.

use std::{fs, path::PathBuf, rc::Rc, sync::mpsc, time::Duration};

use pwrroute_engine::{
    ConfigChoice, DefaultHost, FALLBACK_BROWSER, Limits, Reloader, Watcher,
    discovery::{self, Environment},
};

/// A scratch home directory, removed on drop.
struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-reload-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn candidates(&self) -> ConfigChoice {
        ConfigChoice::Discover(discovery::config_candidates(&Environment {
            home: Some(self.0.clone()),
            ..Environment::default()
        }))
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config(browser: &str) -> String {
    format!("export default {{ defaultBrowser: \"{browser}\" }};\n")
}

fn browser(reloader: &Reloader) -> String {
    reloader
        .engine()
        .resolve("https://example.com/", None)
        .unwrap()
        .browser
        .name
}

fn reloader(choice: ConfigChoice, rules: Option<PathBuf>) -> Reloader {
    let limits = Limits {
        load: Duration::from_millis(500),
        ..Limits::default()
    };
    Reloader::new(choice, rules, Rc::new(DefaultHost), limits).unwrap()
}

/// `poll` waits for a change to settle: the first call sees it, the second
/// reloads.
fn settle(reloader: &mut Reloader) -> Option<anyhow::Result<()>> {
    assert!(reloader.poll().is_none(), "the first poll only notices");
    reloader.poll()
}

#[test]
fn a_broken_edit_keeps_the_working_engine() {
    let home = Home::new("broken-edit");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    assert_eq!(browser(&r), "Firefox");
    assert_eq!(r.status().config.as_deref(), Some(path.as_path()));
    assert_eq!(r.status().generation, 1);

    fs::write(&path, "export default { defaultBrowser: ").unwrap();
    let err = settle(&mut r).unwrap().unwrap_err().to_string();
    assert!(err.contains(".finicky.js"), "{err}");
    assert_eq!(r.status().error.as_deref(), Some(err.as_str()));
    assert_eq!(r.status().generation, 1);
    assert_eq!(browser(&r), "Firefox", "the old engine stays");

    fs::write(&path, config("Arc")).unwrap();
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), "Arc");
    assert_eq!(r.status().error, None);
    assert_eq!(r.status().generation, 2);
}

#[test]
fn an_invalid_config_keeps_the_working_engine() {
    let home = Home::new("invalid");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    fs::write(&path, "export default { handlers: 42 };").unwrap();
    let err = settle(&mut r).unwrap().unwrap_err().to_string();
    assert!(err.starts_with("validating the config:"), "{err}");
    assert_eq!(browser(&r), "Firefox");
}

#[test]
fn a_config_that_hangs_on_load_keeps_the_working_engine() {
    let home = Home::new("hang");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    fs::write(&path, "while (true) {}\nexport default {};").unwrap();
    let err = settle(&mut r).unwrap().unwrap_err().to_string();
    assert!(err.contains("timed out"), "{err}");
    assert_eq!(browser(&r), "Firefox");
}

#[test]
fn a_broken_start_falls_back_then_recovers() {
    let home = Home::new("broken-start");
    let path = home.write(".finicky.ts", "export default {");
    let mut r = reloader(home.candidates(), None);
    assert!(r.status().error.is_some());
    assert_eq!(r.status().config, None);
    assert_eq!(browser(&r), FALLBACK_BROWSER);

    fs::write(&path, config("Firefox")).unwrap();
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), "Firefox");
    assert_eq!(r.status().config.as_deref(), Some(path.as_path()));
}

#[test]
fn a_removed_config_is_kept_until_it_returns() {
    let home = Home::new("removed");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    fs::remove_file(&path).unwrap();
    let err = settle(&mut r).unwrap().unwrap_err().to_string();
    assert!(err.contains("is gone"), "{err}");
    assert_eq!(browser(&r), "Firefox");

    // Saved again (here under another candidate name): picked up.
    let moved = home.write(".config/finicky/finicky.ts", &config("Arc"));
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), "Arc");
    assert_eq!(r.status().config.as_deref(), Some(moved.as_path()));
}

#[test]
fn a_new_config_is_discovered() {
    let home = Home::new("discovered");
    let rules = home.write(
        "rules.json",
        r#"{ "defaultBrowser": "Vivaldi", "rules": [] }"#,
    );
    let mut r = reloader(home.candidates(), Some(rules));
    assert_eq!(browser(&r), "Vivaldi", "rules.json alone");
    assert_eq!(r.status().config, None);

    home.write(".config/finicky.js", &config("Firefox"));
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), "Firefox", "the config owns the default");
}

#[test]
fn stop_using_the_config() {
    let home = Home::new("stop-using");
    home.write(".finicky.js", &config("Firefox"));
    let rules = home.write(
        "rules.json",
        r#"{ "defaultBrowser": "Vivaldi", "rules": [] }"#,
    );
    let mut r = reloader(home.candidates(), Some(rules));
    assert_eq!(browser(&r), "Firefox");
    r.set_choice(ConfigChoice::None).unwrap();
    assert_eq!(browser(&r), "Vivaldi");
    assert_eq!(r.status().config, None);
}

#[test]
fn broken_rules_json_keeps_the_working_engine() {
    let home = Home::new("rules");
    let rules = home.write(
        "PwrRoute/rules.json",
        r#"{ "defaultBrowser": "Firefox", "rules": [ { "match": "github.com/*", "browser": "Arc" } ] }"#,
    );
    let mut r = reloader(ConfigChoice::None, Some(rules.clone()));
    let github = |r: &Reloader| {
        r.engine()
            .resolve("https://github.com/x", None)
            .unwrap()
            .browser
            .name
    };
    assert_eq!(github(&r), "Arc");

    fs::write(&rules, r#"{ "defaultBrowser": "Firefox", "rules": [ "#).unwrap();
    let err = settle(&mut r).unwrap().unwrap_err().to_string();
    assert!(err.contains("rules.json"), "{err}");
    assert_eq!(github(&r), "Arc");

    fs::write(
        &rules,
        r#"{ "defaultBrowser": "Firefox", "rules": [ { "match": "github.com/*", "browser": "Safari" } ] }"#,
    )
    .unwrap();
    settle(&mut r).unwrap().unwrap();
    assert_eq!(github(&r), "Safari");

    // Deleting rules.json is an empty one, as the Go host read it.
    fs::remove_file(&rules).unwrap();
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), FALLBACK_BROWSER);
}

#[test]
fn an_imported_file_is_watched() {
    let home = Home::new("imports");
    home.write(
        "helpers.ts",
        "export const BROWSER: string = \"Firefox\";\n",
    );
    let path = home.write(
        ".finicky.ts",
        "import { BROWSER } from \"./helpers\";\nexport default { defaultBrowser: BROWSER };\n",
    );
    let mut r = reloader(home.candidates(), None);
    assert_eq!(browser(&r), "Firefox");
    assert!(r.watch_set().paths().contains(&home.path("helpers.ts")));
    assert!(r.engine().dependencies().contains(&path));

    home.write("helpers.ts", "export const BROWSER: string = \"Arc\";\n");
    settle(&mut r).unwrap().unwrap();
    assert_eq!(browser(&r), "Arc");
}

#[test]
fn nothing_changed_means_no_reload() {
    let home = Home::new("quiet");
    home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    assert!(r.poll().is_none());
    assert!(r.poll().is_none());
    assert!(r.reload_if_changed().is_none());
    assert_eq!(r.status().generation, 1);
}

#[test]
fn the_watcher_reports_a_settled_change() {
    let home = Home::new("watcher");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::spawn(r.watch_set(), Duration::from_millis(20), move || {
        let _ = tx.send(());
    })
    .unwrap();
    fs::write(&path, config("Arc")).unwrap();
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the watcher reports the change");
    r.reload_if_changed().unwrap().unwrap();
    assert_eq!(browser(&r), "Arc");
    drop(watcher);
}

#[test]
fn an_explicit_path_is_used_even_outside_the_candidates() {
    let home = Home::new("explicit");
    home.write(".finicky.js", &config("Firefox"));
    let chosen = home.write("elsewhere/routes.ts", &config("Arc"));
    let r = reloader(ConfigChoice::Path(chosen.clone()), None);
    assert_eq!(browser(&r), "Arc");
    assert_eq!(r.status().config.as_deref(), Some(chosen.as_path()));
}

#[test]
fn the_watcher_reports_a_change_made_before_it_started() {
    let home = Home::new("watcher-late");
    let path = home.write(".finicky.js", &config("Firefox"));
    let mut r = reloader(home.candidates(), None);
    fs::write(&path, config("Arc")).unwrap();
    let (tx, rx) = mpsc::channel();
    let _watcher = Watcher::spawn(r.watch_set(), Duration::from_millis(20), move || {
        let _ = tx.send(());
    })
    .unwrap();
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the change since the last load is reported");
    r.reload_if_changed().unwrap().unwrap();
    assert_eq!(browser(&r), "Arc");
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "a loaded state is not reported again"
    );
}
