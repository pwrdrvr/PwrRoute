//! `cargo xtask bundle`: PwrRoute.app, signed (ad hoc unless an identity is
//! given), optionally notarized, then zipped and put in a DMG.
//!
//! `--app-only` stops at the ad hoc signed app. release.yml hands that app to
//! `vpk pack`, which adds Velopack's updater, signs and notarizes it.
//!
//! The icon is compiled from the Icon Composer package with Xcode 26's actool,
//! or, without one, copied from the legacy .icns with a warning;
//! `--require-icon-catalog` (CI, release and preview builds) makes that an
//! error instead.
//!
//! Notarizing reads an App Store Connect API key from the environment:
//! `APPLE_API_KEY_PATH` (the .p8 file), `APPLE_API_KEY_ID` and
//! `APPLE_API_ISSUER`.

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail, ensure};

use crate::{package_version, read, root, run, target_dir};

const TARGETS: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    universal: bool,
    identity: Option<String>,
    notarize: bool,
    app_only: bool,
    require_icon_catalog: bool,
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
            "--require-icon-catalog" => options.require_icon_catalog = true,
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
    let built = target_dir();
    let out = built.join("bundle");
    fs::create_dir_all(&out)?;

    let (binaries, arch) = if options.universal {
        for triple in TARGETS {
            build(&root, Some(triple))?;
        }
        let lipo = |name: &str| -> Result<PathBuf> {
            let output = out.join(format!("{name}-universal"));
            let mut lipo = Command::new("lipo");
            lipo.arg("-create").arg("-output").arg(&output);
            for triple in TARGETS {
                lipo.arg(built.join(triple).join("release").join(name));
            }
            run(&mut lipo)?;
            Ok(output)
        };
        let binaries = Binaries {
            app: lipo(APP_EXE)?,
            helper: lipo(HELPER_EXE)?,
        };
        (binaries, "universal")
    } else {
        build(&root, None)?;
        let release = built.join("release");
        (
            Binaries {
                app: release.join(APP_EXE),
                helper: release.join(HELPER_EXE),
            },
            std::env::consts::ARCH,
        )
    };

    let app = out.join("PwrRoute.app");
    if app.exists() {
        // Replaced whole: a stale file inside a signed bundle breaks its
        // signature.
        fs::remove_dir_all(&app)?;
    }
    let short = short_version(&version);
    let plist = read(&root.join(APP_PLIST))?.replace("@SHORT_VERSION@", short);
    let minimum = plist_string(&plist, "LSMinimumSystemVersion")
        .context("Info.plist has no LSMinimumSystemVersion")?
        .to_owned();
    let icon = match icon_catalog(
        &root.join(ICON_PACKAGE),
        &out.join("icon-catalog"),
        &minimum,
    ) {
        Ok(compiled) => Icon::Catalog(compiled),
        Err(err) if !options.require_icon_catalog => {
            eprintln!(
                "warning: bundling the legacy {LEGACY_ICNS} without an asset catalog: {err:#}\n\
                 warning: macOS 26 redraws a bare .icns itself; release builds pass \
                 --require-icon-catalog and need Xcode 26 or newer"
            );
            Icon::Legacy(root.join(LEGACY_ICNS))
        }
        Err(err) => return Err(err.context("--require-icon-catalog: no asset catalog")),
    };
    let contents = app.join("Contents");
    lay_out(&contents, &binaries.app, APP_EXE, &plist, &icon)?;
    let resources = contents.join("Resources");
    for file in ["LICENSE", "THIRD_PARTY_LICENSES", "CHANGELOG.md"] {
        fs::copy(root.join(file), resources.join(file))?;
    }
    let helper_plist = read(&root.join(HELPER_PLIST))?.replace("@SHORT_VERSION@", short);
    check_helper_plist(&helper_plist)?;
    let helper = app.join(LINK_HELPER_PATH);
    lay_out(
        &helper.join("Contents"),
        &binaries.helper,
        HELPER_EXE,
        &helper_plist,
        &icon,
    )?;
    for contents in [&contents, &helper.join("Contents")] {
        run(Command::new("plutil")
            .arg("-lint")
            .arg(contents.join("Info.plist")))?;
        check_icon(contents, options.require_icon_catalog)?;
    }

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
    disk_image(&app, &dmg, &version)?;
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
        .args([
            "build",
            "--release",
            "--locked",
            "-p",
            "pwrroute-app",
            "-p",
            "pwrroute-link-helper",
        ])
        .current_dir(root);
    if let Some(target) = target {
        cargo.args(["--target", target]);
    }
    run(&mut cargo)
}

/// The app's executable and its Info.plist template.
const APP_EXE: &str = "pwrroute";
const APP_PLIST: &str = "packaging/macos/Info.plist.in";

/// The link helper: its executable (`crates/pwrroute-link-helper`), its
/// Info.plist template, its bundle id, and where it is nested. Contents/Helpers
/// is the place Apple's code signing guide ("Placing Content in a Bundle")
/// gives helper apps and tools; Contents/Library/LoginItems is for apps the
/// system starts at login, which this is not. Codesign treats both as nested
/// code, so the helper is signed on its own and sealed into the app's
/// signature. `pwrroute_os::LINK_HELPER_PATH` must say the same (its tests
/// read this file).
const HELPER_EXE: &str = "pwrroute-links";
const HELPER_PLIST: &str = "packaging/macos/LinkHelper.Info.plist.in";
const HELPER_ID: &str = "com.pwrdrvr.pwrroute.links";
const LINK_HELPER_PATH: &str = "Contents/Helpers/PwrRoute.app";

/// The two release binaries a bundle is made of.
struct Binaries {
    app: PathBuf,
    helper: PathBuf,
}

/// The icon both bundles carry: actool's output, or the legacy .icns.
enum Icon {
    /// The directory holding Assets.car and Icon.icns.
    Catalog(PathBuf),
    /// The .icns to copy in as Icon.icns, with CFBundleIconName dropped.
    Legacy(PathBuf),
}

/// One bundle's `Contents`: the executable as `MacOS/<exe>`, the Info.plist
/// (without CFBundleIconName for a legacy icon) and the icon in Resources.
fn lay_out(contents: &Path, binary: &Path, exe: &str, plist: &str, icon: &Icon) -> Result<()> {
    let resources = contents.join("Resources");
    fs::create_dir_all(contents.join("MacOS"))?;
    fs::create_dir_all(&resources)?;
    fs::copy(binary, contents.join("MacOS").join(exe))
        .with_context(|| binary.display().to_string())?;
    let plist = match icon {
        Icon::Catalog(compiled) => {
            for file in [ASSET_CATALOG, &format!("{APP_ICON}.icns")] {
                fs::copy(compiled.join(file), resources.join(file))?;
            }
            plist.to_owned()
        }
        Icon::Legacy(icns) => {
            fs::copy(icns, resources.join(format!("{APP_ICON}.icns")))?;
            without_key(plist, "CFBundleIconName")
        }
    };
    ensure!(
        plist_string(&plist, "CFBundleExecutable") == Some(exe),
        "Info.plist's CFBundleExecutable is not {exe}"
    );
    fs::write(contents.join("Info.plist"), plist)?;
    Ok(())
}

/// What the helper's Info.plist must say: its own id, PwrRoute's names (the
/// default-browser dialog and System Settings show them), no Dock icon, and
/// both web schemes.
fn check_helper_plist(plist: &str) -> Result<()> {
    for (key, want) in [
        ("CFBundleIdentifier", HELPER_ID),
        ("CFBundleName", "PwrRoute"),
        ("CFBundleDisplayName", "PwrRoute"),
        ("CFBundleExecutable", HELPER_EXE),
        ("CFBundlePackageType", "APPL"),
    ] {
        ensure!(
            plist_string(plist, key) == Some(want),
            "the link helper's Info.plist does not set {key} to {want}"
        );
    }
    ensure!(
        plist_true(plist, "LSUIElement"),
        "the link helper's Info.plist does not set LSUIElement"
    );
    let (_, url_types) = plist
        .split_once("<key>CFBundleURLTypes</key>")
        .context("the link helper's Info.plist has no CFBundleURLTypes")?;
    // Up to the next top-level key, which is indented one tab.
    let url_types = url_types.split("\n\t<key>").next().unwrap_or(url_types);
    for scheme in ["http", "https"] {
        ensure!(
            url_types.contains(&format!("<string>{scheme}</string>")),
            "the link helper's Info.plist does not declare {scheme}"
        );
    }
    Ok(())
}

/// `<key>KEY</key>` followed by `<true/>`.
fn plist_true(plist: &str, key: &str) -> bool {
    plist
        .split_once(&format!("<key>{key}</key>"))
        .is_some_and(|(_, after)| after.trim_start().starts_with("<true/>"))
}

/// `0.0.1-alpha.0` becomes `0.0.1`: macOS wants three numbers.
pub(crate) fn short_version(version: &str) -> &str {
    version.split(['-', '+']).next().unwrap_or(version)
}

/// The Icon Composer package `scripts/release/make-icns.sh` writes from
/// packaging/icon/PwrRoute.svg; committed, because CI does not rasterize.
const ICON_PACKAGE: &str = "packaging/icon/PwrRoute.icon";

/// The icon set the same script renders: the bundle's icon when there is no
/// actool 26 to compile the package. release.yml also passes it to `vpk
/// pack --icon`, which reads it only when vpk builds the .app itself.
const LEGACY_ICNS: &str = "packaging/macos/PwrRoute.icns";

/// The name Info.plist's CFBundleIconName and CFBundleIconFile carry, and the
/// basename the package is compiled under: actool resolves `--app-icon` by
/// the package's file name, so compiling `PwrRoute.icon` with `--app-icon
/// Icon` writes Assets.car and silently no Icon.icns.
const APP_ICON: &str = "Icon";
const ASSET_CATALOG: &str = "Assets.car";

/// actool learned to compile `.icon` packages in Xcode 26.
const MIN_ACTOOL_MAJOR: u32 = 26;

/// Compiles `package` the way Xcode and electron-builder do, into
/// `work/out`: Assets.car (what macOS 26 draws) and the Icon.icns actool
/// derives from the same source (what older macOS draws). Returns `work/out`
/// once both are there and actool's partial Info.plist names them as
/// Info.plist.in does.
fn icon_catalog(package: &Path, work: &Path, minimum_system: &str) -> Result<PathBuf> {
    let version = actool_version()?;
    ensure!(
        actool_major(&version).is_some_and(|major| major >= MIN_ACTOOL_MAJOR),
        "actool {version} cannot compile an Icon Composer package; it needs Xcode \
         {MIN_ACTOOL_MAJOR} or newer"
    );
    if work.exists() {
        fs::remove_dir_all(work)?;
    }
    let out = work.join("out");
    fs::create_dir_all(&out)?;
    copy_dir(package, &work.join(format!("{APP_ICON}.icon")))?;
    run(Command::new("xcrun")
        .arg("actool")
        .args(actool_args(work, minimum_system)))?;
    for file in [ASSET_CATALOG, &format!("{APP_ICON}.icns")] {
        ensure!(
            out.join(file).is_file(),
            "actool wrote no {file}; was the package compiled as {APP_ICON}.icon?"
        );
    }
    let partial = read(&out.join("partial-info.plist"))?;
    for key in ["CFBundleIconName", "CFBundleIconFile"] {
        ensure!(
            plist_string(&partial, key) == Some(APP_ICON),
            "actool's partial Info.plist does not set {key} to {APP_ICON}:\n{partial}"
        );
    }
    Ok(out)
}

/// actool's command line for `work/Icon.icon`, as electron-builder's
/// macosIconComposer.js builds it, minus its `--accent-color` (PwrRoute
/// has none) and with the app's own minimum macOS.
fn actool_args(work: &Path, minimum_system: &str) -> Vec<OsString> {
    let out = work.join("out");
    let mut args: Vec<OsString> = vec![work.join(format!("{APP_ICON}.icon")).into()];
    args.push("--compile".into());
    args.push(out.clone().into());
    for arg in [
        "--output-format",
        "human-readable-text",
        "--notices",
        "--warnings",
        "--output-partial-info-plist",
    ] {
        args.push(arg.into());
    }
    args.push(out.join("partial-info.plist").into());
    for arg in [
        "--app-icon",
        APP_ICON,
        "--include-all-app-icons",
        "--enable-on-demand-resources",
        "NO",
        "--development-region",
        "en",
        "--target-device",
        "mac",
        "--minimum-deployment-target",
        minimum_system,
        "--platform",
        "macosx",
    ] {
        args.push(arg.into());
    }
    args
}

/// `actool --version` prints a plist; the version is its
/// `short-bundle-version`, e.g. `26.5`.
fn actool_version() -> Result<String> {
    let output = Command::new("xcrun")
        .args(["actool", "--version"])
        .output()
        .context("no xcrun: install Xcode 26 or newer for the app icon's asset catalog")?;
    ensure!(
        output.status.success(),
        "`xcrun actool --version` failed ({}): install Xcode 26 or newer and select it \
         with xcode-select\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let plist = String::from_utf8_lossy(&output.stdout);
    plist_string(&plist, "short-bundle-version")
        .map(str::to_owned)
        .with_context(|| format!("no short-bundle-version in `actool --version`:\n{plist}"))
}

fn actool_major(version: &str) -> Option<u32> {
    version.split('.').next()?.trim().parse().ok()
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from).with_context(|| from.display().to_string())? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The bundle shows an icon only when Info.plist names files that are in
/// Contents/Resources; a slip leaves the generic app icon in the Dock and
/// the default-browser chooser. CFBundleIconFile must name an .icns there;
/// CFBundleIconName, which `require_catalog` insists on, an Assets.car.
fn check_icon(contents: &Path, require_catalog: bool) -> Result<()> {
    let plist = read(&contents.join("Info.plist"))?;
    let resources = contents.join("Resources");
    let name =
        plist_string(&plist, "CFBundleIconFile").context("Info.plist has no CFBundleIconFile")?;
    let icns = resources.join(format!("{}.icns", name.trim_end_matches(".icns")));
    let header = fs::read(&icns).with_context(|| icns.display().to_string())?;
    ensure!(
        header.starts_with(b"icns"),
        "{} is not an icns file",
        icns.display()
    );
    match plist_string(&plist, "CFBundleIconName") {
        Some(_) => {
            let car = resources.join(ASSET_CATALOG);
            let header = fs::read(&car).with_context(|| car.display().to_string())?;
            ensure!(
                header.starts_with(b"BOMStore"),
                "{} is not an asset catalog",
                car.display()
            );
        }
        None => ensure!(
            !require_catalog,
            "Info.plist has no CFBundleIconName, so macOS 26 would redraw the .icns"
        ),
    }
    Ok(())
}

/// The `<string>` after `<key>KEY</key>`.
fn plist_string<'a>(plist: &'a str, key: &str) -> Option<&'a str> {
    let (_, after_key) = plist.split_once(&format!("<key>{key}</key>"))?;
    let (between, after_tag) = after_key.split_once("<string>")?;
    if !between.trim().is_empty() {
        return None;
    }
    let (value, _) = after_tag.split_once("</string>")?;
    Some(value.trim()).filter(|v| !v.is_empty())
}

/// The plist without `<key>KEY</key>` and the `<string>` after it, each line
/// removed whole.
fn without_key(plist: &str, key: &str) -> String {
    let key_line = format!("<key>{key}</key>");
    let mut kept = Vec::new();
    let mut lines = plist.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() == key_line {
            lines.next_if(|value| value.trim_start().starts_with("<string>"));
        } else {
            kept.push(line);
        }
    }
    kept.join("\n") + if plist.ends_with('\n') { "\n" } else { "" }
}

/// Signs inside out, never with `--deep`: the link helper first, as code of
/// its own, then the app, whose signature seals the helper's. A bundle
/// without a helper (`cargo xtask e2e`'s) is just the app.
pub(crate) fn sign(app: &Path, identity: Option<&str>) -> Result<()> {
    let helper = app.join(LINK_HELPER_PATH);
    if helper.exists() {
        sign_code(&helper, identity)?;
    }
    sign_code(app, identity)?;
    // Verifying deep is fine: it checks each nested signature, it signs
    // nothing.
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(app))
}

fn sign_code(bundle: &Path, identity: Option<&str>) -> Result<()> {
    let mut codesign = Command::new("codesign");
    codesign.arg("--force");
    match identity {
        // Hardened runtime and a secure timestamp: what notarization requires.
        Some(identity) => {
            codesign.args(["--timestamp", "--options", "runtime", "--sign", identity])
        }
        None => codesign.args(["--sign", "-"]),
    };
    run(codesign.arg(bundle))
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
/// without an identity: signing and notarizing the DMG stay here. The full
/// version (prerelease included) goes in the volume name; the app's
/// Info.plist has only the short one.
fn disk_image(app: &Path, dmg: &Path, version: &str) -> Result<()> {
    run(Command::new(root().join(DMG_SCRIPT))
        .env("DMG_VERSION", version)
        .arg(app)
        .arg(dmg))
}

const DMG_SCRIPT: &str = "scripts/release/macos-dmg.sh";

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

    #[test]
    fn the_icon_catalog_is_opt_in_strict() {
        assert!(!parse(&[]).unwrap().require_icon_catalog);
        let strict = parse(&args(&[
            "--universal",
            "--app-only",
            "--require-icon-catalog",
        ]))
        .unwrap();
        assert!(strict.require_icon_catalog && strict.app_only && strict.universal);
    }

    #[test]
    fn reads_strings_from_a_plist() {
        let plist = "<dict>\n\t<key>CFBundleIconFile</key>\n\t<string>Icon</string>\n</dict>";
        assert_eq!(plist_string(plist, "CFBundleIconFile"), Some("Icon"));
        assert_eq!(plist_string(plist, "CFBundleIconName"), None);
        assert_eq!(plist_string("<dict></dict>", "CFBundleIconFile"), None);
        assert_eq!(
            plist_string(
                "<key>CFBundleIconFile</key><string></string>",
                "CFBundleIconFile"
            ),
            None
        );
        assert_eq!(
            plist_string("<key>A</key><true/><key>B</key><string>b</string>", "A"),
            None,
            "a key's value is the element right after it"
        );
    }

    #[test]
    fn the_fallback_drops_only_the_icon_name() {
        let template = read(&root().join("packaging/macos/Info.plist.in")).unwrap();
        assert_eq!(plist_string(&template, "CFBundleIconName"), Some(APP_ICON));
        let fallback = without_key(&template, "CFBundleIconName");
        assert_eq!(plist_string(&fallback, "CFBundleIconName"), None);
        assert!(
            !fallback.contains("<key>CFBundleIconName</key>"),
            "{fallback}"
        );
        assert_eq!(plist_string(&fallback, "CFBundleIconFile"), Some(APP_ICON));
        assert_eq!(
            plist_string(&fallback, "CFBundleIdentifier"),
            Some("com.pwrdrvr.pwrroute")
        );
        assert_eq!(
            template.lines().count() - 2,
            fallback.lines().count(),
            "the key and its string, nothing else"
        );
        assert!(fallback.ends_with("</plist>\n"));
        assert_eq!(without_key(&fallback, "CFBundleIconName"), fallback);
    }

    #[test]
    fn actool_compiles_the_package_as_icon() {
        let work = Path::new("work").join("icon-catalog");
        let out = work.join("out");
        let args = actool_args(&work, "11.0");
        assert_eq!(
            Path::new(&args[0]),
            work.join("Icon.icon"),
            "the input comes first, named for --app-icon"
        );
        let value = |flag: &str| {
            let at = args
                .iter()
                .position(|a| a == flag)
                .unwrap_or_else(|| panic!("no {flag}"));
            PathBuf::from(&args[at + 1])
        };
        assert_eq!(value("--compile"), out);
        assert_eq!(
            value("--output-partial-info-plist"),
            out.join("partial-info.plist")
        );
        assert_eq!(value("--app-icon"), Path::new(APP_ICON));
        assert_eq!(value("--platform"), Path::new("macosx"));
        assert_eq!(value("--target-device"), Path::new("mac"));
        assert_eq!(value("--minimum-deployment-target"), Path::new("11.0"));
        assert!(args.iter().any(|a| a == "--include-all-app-icons"));
    }

    #[test]
    fn reads_the_actool_major_version() {
        let output = "<plist version=\"1.0\">\n<dict>\n\t<key>com.apple.actool.version</key>\n\t<dict>\n\t\t<key>bundle-version</key>\n\t\t<string>24765</string>\n\t\t<key>short-bundle-version</key>\n\t\t<string>26.5</string>\n\t</dict>\n</dict>\n</plist>";
        let version = plist_string(output, "short-bundle-version").unwrap();
        assert_eq!(actool_major(version), Some(26));
        assert_eq!(actool_major("16.4"), Some(16));
        assert_eq!(actool_major("26"), Some(26));
        assert_eq!(actool_major(""), None);
    }

    /// The committed artwork: the package make-icns.sh writes (its fill from
    /// the SVG's ground, a 1024px glyph), the fallback .icns and the .ico.
    #[test]
    fn the_committed_icons_are_whole() {
        let package = root().join(ICON_PACKAGE);
        let json = read(&package.join("icon.json")).unwrap();
        assert!(json.contains("\"image-name\" : \"glyph.png\""), "{json}");
        let svg = read(&root().join("packaging/icon/PwrRoute.svg")).unwrap();
        for stop in ["#1e1a14", "#0a0908"] {
            assert!(svg.contains(&format!("stop-color=\"{stop}\"")), "{stop}");
            let channel =
                |i: usize| f64::from(u8::from_str_radix(&stop[i..i + 2], 16).unwrap()) / 255.0;
            let srgb = format!(
                "srgb:{:.5},{:.5},{:.5},1.00000",
                channel(1),
                channel(3),
                channel(5)
            );
            assert!(
                json.contains(&srgb),
                "icon.json lacks {srgb}: rerun make-icns.sh"
            );
        }
        let glyph = fs::read(package.join("Assets/glyph.png")).unwrap();
        assert!(
            glyph.starts_with(b"\x89PNG\r\n\x1a\n"),
            "glyph.png is not a PNG"
        );
        let (width, height) = (&glyph[16..20], &glyph[20..24]);
        assert_eq!(
            (width, height),
            (&[0, 0, 4, 0][..], &[0, 0, 4, 0][..]),
            "1024x1024"
        );
        let icns = fs::read(root().join(LEGACY_ICNS)).unwrap();
        assert!(
            icns.starts_with(b"icns"),
            "{LEGACY_ICNS} is not an icns file"
        );
        let ico = fs::read(root().join("packaging/windows/PwrRoute.ico")).unwrap();
        assert!(
            ico.starts_with(&[0, 0, 1, 0]),
            "the .ico has no icon header"
        );
    }

    /// macos-dmg.sh places the icons and generate-dmg-background.swift paints
    /// the arrow between them; the committed PNG is the size of the window.
    #[test]
    fn the_dmg_layout_matches_its_background() {
        let script = read(&root().join(DMG_SCRIPT)).unwrap();
        let swift = read(&root().join("scripts/release/generate-dmg-background.swift")).unwrap();
        for (shell, swift_name, value) in [
            ("window_width", "width", 660),
            ("window_height", "height", 400),
            ("icon_size", "iconSize", 112),
            ("app_x", "appIconX", 170),
            ("applications_x", "applicationsX", 500),
            ("icon_y", "iconY", 230),
        ] {
            assert!(
                script.lines().any(|l| l == format!("{shell}={value}")),
                "macos-dmg.sh: {shell} is not {value}"
            );
            assert!(
                swift
                    .lines()
                    .any(|l| l == format!("let {swift_name} = {value}")),
                "generate-dmg-background.swift: {swift_name} is not {value}"
            );
        }
        let png = fs::read(root().join("packaging/dmg/background.png")).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
        // IHDR's width and height, big-endian, at bytes 16..24.
        let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        assert_eq!((size(16), size(20)), (660, 400));
    }

    #[cfg(unix)]
    #[test]
    fn the_dmg_script_parses_and_wants_two_or_three_arguments() {
        let script = root().join(DMG_SCRIPT);
        let syntax = Command::new("bash")
            .arg("-n")
            .arg(&script)
            .status()
            .unwrap();
        assert!(syntax.success(), "bash -n {DMG_SCRIPT}");
        let usage = Command::new("bash")
            .arg(&script)
            .arg("only-one")
            .output()
            .unwrap();
        assert_eq!(usage.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&usage.stderr).contains("usage: macos-dmg.sh"));
    }

    /// What `bundle` runs after writing Contents: the .icns the plist
    /// names, and an Assets.car when it names a catalog or the release
    /// wants one.
    #[test]
    fn the_bundle_check_wants_what_the_plist_names() {
        let dir = env::temp_dir().join(format!("pwrroute-icon-check-{}", std::process::id()));
        let contents = dir.join("Contents");
        let resources = contents.join("Resources");
        fs::create_dir_all(&resources).unwrap();
        let plist = contents.join("Info.plist");
        let both = "<key>CFBundleIconFile</key>\n<string>Icon</string>\n\
                    <key>CFBundleIconName</key>\n<string>Icon</string>\n";
        fs::write(&plist, both).unwrap();
        assert!(check_icon(&contents, false).is_err(), "no .icns yet");
        fs::copy(root().join(LEGACY_ICNS), resources.join("Icon.icns")).unwrap();
        assert!(check_icon(&contents, false).is_err(), "no Assets.car yet");
        fs::write(resources.join(ASSET_CATALOG), "not a catalog").unwrap();
        assert!(check_icon(&contents, false).is_err(), "not a BOMStore");
        fs::write(resources.join(ASSET_CATALOG), "BOMStore\0\0\0\x01").unwrap();
        check_icon(&contents, true).unwrap();

        let fallback = without_key(both, "CFBundleIconName");
        fs::write(&plist, fallback).unwrap();
        check_icon(&contents, false).unwrap();
        assert!(
            check_icon(&contents, true).is_err(),
            "the release wants the catalog"
        );
        fs::write(&plist, "<dict></dict>").unwrap();
        assert!(check_icon(&contents, false).is_err(), "the keys are gone");
        fs::remove_dir_all(dir).unwrap();
    }

    /// The helper's template says what the default-browser dialog, the Dock
    /// and LaunchServices need; a slip in any of them is caught here.
    #[test]
    fn the_helper_plist_is_a_windowless_web_handler_named_pwrroute() {
        let template = read(&root().join(HELPER_PLIST)).unwrap();
        check_helper_plist(&template).unwrap();
        assert_eq!(plist_string(&template, "CFBundleIconName"), Some(APP_ICON));
        assert_eq!(plist_string(&template, "CFBundleIconFile"), Some(APP_ICON));
        // The same minimum macOS as the app it lives in.
        let app = read(&root().join(APP_PLIST)).unwrap();
        assert_eq!(
            plist_string(&template, "LSMinimumSystemVersion"),
            plist_string(&app, "LSMinimumSystemVersion")
        );
        // The app keeps its own handler, which earlier registrations name.
        assert!(app.contains("<string>https</string>"));

        let broken = [
            template.replace(HELPER_ID, "com.pwrdrvr.pwrroute"),
            template.replace(
                "<key>CFBundleDisplayName</key>\n\t<string>PwrRoute</string>",
                "<key>CFBundleDisplayName</key>\n\t<string>PwrRoute Links</string>",
            ),
            template.replace("<key>LSUIElement</key>\n\t<true/>", ""),
            template.replace("\t\t\t\t<string>https</string>\n", ""),
            template.replace(
                "<string>pwrroute-links</string>",
                "<string>pwrroute</string>",
            ),
        ];
        for plist in broken {
            assert_ne!(plist, template, "the replacement must apply");
            assert!(check_helper_plist(&plist).is_err());
        }
    }

    /// The app with its helper nested at Contents/Helpers, each with its
    /// executable, Info.plist and icon: what `bundle` signs.
    #[test]
    fn the_helper_is_nested_as_a_bundle_of_its_own() {
        let dir = env::temp_dir().join(format!("pwrroute-nest-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("binary");
        fs::write(&binary, b"\xcf\xfa\xed\xfe").unwrap();
        let icon = Icon::Legacy(root().join(LEGACY_ICNS));
        let app = dir.join("PwrRoute.app");
        let app_plist = read(&root().join(APP_PLIST)).unwrap();
        lay_out(&app.join("Contents"), &binary, APP_EXE, &app_plist, &icon).unwrap();
        let helper_plist = read(&root().join(HELPER_PLIST)).unwrap();
        let helper = app.join(LINK_HELPER_PATH);
        lay_out(
            &helper.join("Contents"),
            &binary,
            HELPER_EXE,
            &helper_plist,
            &icon,
        )
        .unwrap();

        assert_eq!(
            helper,
            app.join("Contents").join("Helpers").join("PwrRoute.app")
        );
        for file in [
            "Contents/MacOS/pwrroute",
            "Contents/Info.plist",
            "Contents/Resources/Icon.icns",
            "Contents/Helpers/PwrRoute.app/Contents/MacOS/pwrroute-links",
            "Contents/Helpers/PwrRoute.app/Contents/Info.plist",
            "Contents/Helpers/PwrRoute.app/Contents/Resources/Icon.icns",
        ] {
            assert!(app.join(file).is_file(), "{file}");
        }
        let written = read(&helper.join("Contents/Info.plist")).unwrap();
        check_helper_plist(&written).unwrap();
        assert_eq!(
            plist_string(&written, "CFBundleIconName"),
            None,
            "legacy icon"
        );
        check_icon(&helper.join("Contents"), false).unwrap();
        check_icon(&app.join("Contents"), false).unwrap();
        // Executables only where each Info.plist says.
        assert!(
            lay_out(
                &dir.join("x/Contents"),
                &binary,
                "other",
                &helper_plist,
                &icon
            )
            .is_err()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// The committed package through the real actool, when there is one
    /// that can compile it: Assets.car, the derived Icon.icns, and the
    /// partial plist naming both. Skipped, saying why, elsewhere (Linux CI,
    /// a Mac without Xcode 26).
    #[test]
    fn the_committed_package_compiles() {
        match actool_version() {
            Ok(version) if actool_major(&version).is_some_and(|m| m >= MIN_ACTOOL_MAJOR) => {}
            Ok(version) => {
                println!("skipped: actool {version} is older than {MIN_ACTOOL_MAJOR}");
                return;
            }
            Err(err) => {
                println!("skipped: no actool ({err:#})");
                return;
            }
        }
        let work = env::temp_dir().join(format!("pwrroute-icon-catalog-{}", std::process::id()));
        let out = icon_catalog(&root().join(ICON_PACKAGE), &work, "11.0").unwrap();
        let car = fs::read(out.join(ASSET_CATALOG)).unwrap();
        assert!(car.starts_with(b"BOMStore"));
        let icns = fs::read(out.join("Icon.icns")).unwrap();
        assert!(icns.starts_with(b"icns"));
        fs::remove_dir_all(work).unwrap();
    }
}
