//! `cargo xtask bundle`: PwrRoute.app, signed (ad hoc unless an identity is
//! given), optionally notarized, then zipped and put in a DMG.
//!
//! `--app-only` stops at the ad hoc signed app. release.yml hands that app to
//! `vpk pack`, which adds Velopack's updater, signs and notarizes it.
//!
//! Notarizing reads an App Store Connect API key from the environment:
//! `APPLE_API_KEY_PATH` (the .p8 file), `APPLE_API_KEY_ID` and
//! `APPLE_API_ISSUER`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail, ensure};

use crate::{package_version, read, root, run};

const TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    universal: bool,
    identity: Option<String>,
    notarize: bool,
    app_only: bool,
}

fn parse(args: &[String]) -> Result<Options> {
    let mut options = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--universal" => options.universal = true,
            "--sign" => {
                let identity = it.next().filter(|s| !s.is_empty()).context(
                    "--sign needs an identity, e.g. \"Developer ID Application: Name (TEAMID)\"",
                )?;
                options.identity = Some(identity.clone());
            }
            "--notarize" => options.notarize = true,
            "--app-only" => options.app_only = true,
            other => bail!("bundle: unknown argument `{other}`"),
        }
    }
    ensure!(
        !options.notarize || options.identity.is_some(),
        "--notarize needs --sign: Apple notarizes Developer ID signatures only"
    );
    ensure!(
        !(options.app_only && options.notarize),
        "--app-only stops before notarizing; drop --notarize"
    );
    Ok(options)
}

pub(crate) fn bundle(args: &[String]) -> Result<()> {
    ensure!(cfg!(target_os = "macos"), "bundle runs on macOS only");
    let options = parse(args)?;
    let root = root();
    let version = package_version()?;
    let out = root.join("target/bundle");
    fs::create_dir_all(&out)?;

    let (binary, arch) = if options.universal {
        for target in TARGETS {
            build(&root, Some(target))?;
        }
        let universal = out.join("pwrroute-universal");
        let mut lipo = Command::new("lipo");
        lipo.arg("-create").arg("-output").arg(&universal);
        for target in TARGETS {
            lipo.arg(root.join("target").join(target).join("release/pwrroute"));
        }
        run(&mut lipo)?;
        (universal, "universal")
    } else {
        build(&root, None)?;
        (root.join("target/release/pwrroute"), std::env::consts::ARCH)
    };

    let app = out.join("PwrRoute.app");
    if app.exists() {
        // Replaced whole: a stale file inside a signed bundle breaks its
        // signature.
        fs::remove_dir_all(&app)?;
    }
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(contents.join("Resources"))?;
    fs::copy(&binary, contents.join("MacOS/pwrroute"))?;
    for file in ["LICENSE", "THIRD_PARTY_LICENSES", "CHANGELOG.md"] {
        fs::copy(root.join(file), contents.join("Resources").join(file))?;
    }
    let plist = read(&root.join("packaging/macos/Info.plist.in"))?
        .replace("@SHORT_VERSION@", short_version(&version));
    fs::write(contents.join("Info.plist"), plist)?;
    run(Command::new("plutil")
        .arg("-lint")
        .arg(contents.join("Info.plist")))?;

    sign(&app, options.identity.as_deref())?;
    if options.app_only {
        println!("{}", app.display());
        return Ok(());
    }

    let stem = format!("PwrRoute-{version}-{arch}-mac");
    let zip = out.join(format!("{stem}.zip"));
    archive(&app, &zip)?;
    if options.notarize {
        notarize(&zip)?;
        run(Command::new("xcrun").args(["stapler", "staple"]).arg(&app))?;
        // Re-zip so the archive carries the stapled ticket.
        archive(&app, &zip)?;
    }
    let dmg = out.join(format!("PwrRoute-{version}-{arch}.dmg"));
    disk_image(&app, &dmg)?;
    if let Some(identity) = &options.identity {
        run(Command::new("codesign")
            .args(["--force", "--timestamp", "--sign", identity])
            .arg(&dmg))?;
        if options.notarize {
            notarize(&dmg)?;
            run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
        }
    }
    println!("{}\n{}\n{}", app.display(), zip.display(), dmg.display());
    Ok(())
}

fn build(root: &Path, target: Option<&str>) -> Result<()> {
    let mut cargo = Command::new("cargo");
    cargo
        .args(["build", "--release", "--locked", "-p", "pwrroute-app"])
        .current_dir(root);
    if let Some(target) = target {
        cargo.args(["--target", target]);
    }
    run(&mut cargo)
}

/// `0.0.1-alpha.0` becomes `0.0.1`: macOS wants three numbers.
fn short_version(version: &str) -> &str {
    version.split(['-', '+']).next().unwrap_or(version)
}

fn sign(app: &Path, identity: Option<&str>) -> Result<()> {
    let mut codesign = Command::new("codesign");
    codesign.arg("--force");
    match identity {
        // Hardened runtime and a secure timestamp: what notarization requires.
        Some(identity) => {
            codesign.args(["--timestamp", "--options", "runtime", "--sign", identity])
        }
        None => codesign.args(["--sign", "-"]),
    };
    run(codesign.arg(app))?;
    run(Command::new("codesign")
        .args(["--verify", "--strict", "--verbose=2"])
        .arg(app))
}

fn archive(app: &Path, zip: &Path) -> Result<()> {
    if zip.exists() {
        fs::remove_file(zip)?;
    }
    run(Command::new("ditto")
        .args(["-c", "-k", "--sequesterRsrc", "--keepParent"])
        .arg(app)
        .arg(zip))
}

/// The DMG layout comes from scripts/release/macos-dmg.sh, which release.yml
/// also runs on the vpk-packed app, so both build the same image. Called
/// without an identity: signing and notarizing the DMG stay here.
fn disk_image(app: &Path, dmg: &Path) -> Result<()> {
    run(Command::new(root().join("scripts/release/macos-dmg.sh"))
        .arg(app)
        .arg(dmg))
}

fn notarize(file: &Path) -> Result<()> {
    let key: PathBuf = env::var_os("APPLE_API_KEY_PATH")
        .context("--notarize needs APPLE_API_KEY_PATH")?
        .into();
    let key_id = env::var("APPLE_API_KEY_ID").context("--notarize needs APPLE_API_KEY_ID")?;
    let issuer = env::var("APPLE_API_ISSUER").context("--notarize needs APPLE_API_ISSUER")?;
    run(Command::new("xcrun")
        .args(["notarytool", "submit"])
        .arg(file)
        .arg("--key")
        .arg(key)
        .args(["--key-id", &key_id, "--issuer", &issuer, "--wait"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn ad_hoc_unless_an_identity_is_given() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        let signed = parse(&args(&["--sign", "Developer ID Application: X"])).unwrap();
        assert_eq!(
            signed.identity.as_deref(),
            Some("Developer ID Application: X")
        );
    }

    #[test]
    fn notarizing_needs_a_real_signature() {
        assert!(parse(&args(&["--notarize"])).is_err());
        assert!(parse(&args(&["--sign"])).is_err());
        assert!(parse(&args(&["--sign", ""])).is_err());
    }

    #[test]
    fn app_only_stops_before_notarizing() {
        assert!(
            parse(&args(&["--universal", "--app-only"]))
                .unwrap()
                .app_only
        );
        assert!(parse(&args(&["--app-only", "--sign", "X", "--notarize"])).is_err());
    }

    #[test]
    fn short_version_drops_the_prerelease() {
        assert_eq!(short_version("0.0.1-alpha.0"), "0.0.1");
        assert_eq!(short_version("1.2.3"), "1.2.3");
    }
}
