//! `cargo xtask e2e`: build the app, then run the Desktop E2E suite in `e2e/`
//! against it, one test at a time.
//!
//! On macOS the binary is wrapped in an ad hoc signed `target/e2e/PwrRoute.app`
//! first, so the tests launch a real bundle: its bundle id, its Apple Events.
//! A dev profile build by default, which is what CI already caches; pass
//! `--release` for the shipping profile.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Result, bail};

use crate::{macos, package_version, read, root, run};

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    release: bool,
    /// Also run the `#[ignore]`d scenarios whose contract is not met yet.
    pending: bool,
    screenshots: bool,
    /// Passed to the test binary after `--`: a name filter, `--exact`, ….
    test_args: Vec<String>,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--release" => options.release = true,
            "--pending" => options.pending = true,
            "--screenshots" => options.screenshots = true,
            "--" => options.test_args.extend(it.by_ref().cloned()),
            other => bail!("unknown e2e argument `{other}`"),
        }
    }
    Ok(options)
}

pub(crate) fn e2e(args: &[String]) -> Result<()> {
    let options = parse(args)?;
    let root = root();
    let profile = if options.release { "release" } else { "debug" };

    let mut build = Command::new("cargo");
    build.args(["build", "-p", "pwrroute-app", "--locked"]);
    if options.release {
        build.arg("--release");
    }
    run(build.current_dir(&root))?;

    let target = target_dir(&root);
    let binary = target
        .join(profile)
        .join(format!("pwrroute{}", env::consts::EXE_SUFFIX));
    let app = if cfg!(target_os = "macos") {
        dev_bundle(&root, &target, &binary)?
    } else {
        binary
    };

    let artifacts = target.join("e2e-artifacts");
    if artifacts.exists() {
        fs::remove_dir_all(&artifacts)?;
    }
    fs::create_dir_all(&artifacts)?;

    let mut test = Command::new("cargo");
    test.args([
        "test",
        "-p",
        "pwrroute-e2e",
        "--locked",
        "--",
        "--test-threads=1",
    ]);
    if options.pending {
        test.arg("--include-ignored");
    }
    test.args(&options.test_args)
        .env("PWRROUTE_E2E_APP", &app)
        .env("PWRROUTE_E2E_ARTIFACTS", &artifacts)
        .current_dir(&root);
    // CI's desktops are throwaway, so screenshots there are safe to keep.
    if options.screenshots || env::var_os("CI").is_some() {
        test.env("PWRROUTE_E2E_SCREENSHOTS", "1");
    }
    let result = run(&mut test);
    println!("E2E artifacts: {}", artifacts.display());
    result
}

/// Where the build above put the binary: `CARGO_TARGET_DIR` when a developer
/// keeps several checkouts' builds apart, else the workspace's `target/`.
fn target_dir(root: &Path) -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(|dir| root.join(dir))
        .unwrap_or_else(|| root.join("target"))
}

/// `target/e2e/PwrRoute.app` around `binary`: the release bundle's
/// Info.plist and ad hoc signature (`macos.rs`), without the notices or DMG.
fn dev_bundle(root: &Path, target: &Path, binary: &Path) -> Result<PathBuf> {
    let app = target.join("e2e/PwrRoute.app");
    if app.exists() {
        fs::remove_dir_all(&app)?;
    }
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::copy(binary, contents.join("MacOS/pwrroute"))?;
    let plist = read(&root.join("packaging/macos/Info.plist.in"))?
        .replace("@SHORT_VERSION@", macos::short_version(&package_version()?));
    fs::write(contents.join("Info.plist"), plist)?;
    macos::sign(&app, None)?;
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn test_arguments_follow_the_separator() {
        let options = parse(&args(&["--pending", "--", "quits", "--exact"])).unwrap();
        assert!(options.pending && !options.release);
        assert_eq!(options.test_args, ["quits", "--exact"]);
        assert!(parse(&args(&["--bogus"])).is_err());
    }
}
