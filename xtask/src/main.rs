//! Repository tasks: `cargo xtask <task>`. CI runs these same commands, so a
//! green local run means a green CI run.

mod e2e;
mod linux;
mod macos;

use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, Result, bail, ensure};

const USAGE: &str = "\
usage: cargo xtask <task>

  lint                 rustfmt --check, CI classifier tests, clippy, cargo-deny, notices
  test                 every workspace test
  fmt                  format in place
  licenses [--check]   write THIRD_PARTY_LICENSES, or check it is current
  release-check --tag vX.Y.Z [--notes-file PATH]
                       the tag, Cargo.toml version and CHANGELOG.md agree;
                       optionally write that version's notes
  bundle [--universal] [--sign IDENTITY] [--notarize] [--app-only]
         [--require-icon-catalog] [--debug]
                       macOS: <target>/bundle/PwrRoute.app, its zip and DMG
                       (--app-only: the app alone, for vpk pack;
                       --require-icon-catalog: fail, not fall back to the
                       legacy .icns, without Xcode 26's actool;
                       --debug --app-only: native debug app at
                       <workspace>/target/dev/PwrRoute.app, even with a shared target)
  package-linux        Linux: <target>/package/*.tar.gz and *.deb
  e2e [--release] [--pending] [--screenshots] [-- TEST-ARGS]
                       build the app and run the Desktop E2E suite against
                       it (--pending: also the scenarios not yet met)

<target> is $CARGO_TARGET_DIR when set, else target/ in the workspace.";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some((task, rest)) = args.split_first() else {
        println!("{USAGE}");
        return Ok(());
    };
    match task.as_str() {
        "lint" => lint(),
        "test" => cargo(&["test", "--workspace", "--locked"]),
        "fmt" => cargo(&["fmt", "--all"]),
        "licenses" => licenses(rest.iter().any(|a| a == "--check")),
        "release-check" => release_check(rest),
        "bundle" => macos::bundle(rest),
        "package-linux" => linux::package(),
        "e2e" => e2e::e2e(rest),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => bail!("unknown task `{other}`\n\n{USAGE}"),
    }
}

/// Cheapest first, so a formatting slip fails in a second, not after clippy.
fn lint() -> Result<()> {
    cargo(&["fmt", "--all", "--", "--check"])?;
    let mut node = Command::new("node");
    node.args(["--test", "scripts/ci/classify-changes.test.cjs"]);
    // Start's subprocess mocks require Bash and POSIX executable permissions.
    if cfg!(unix) {
        node.arg("scripts/dev/start.test.cjs");
    }
    run(node.current_dir(root()))
        .context("Node.js is required to run the repository's JavaScript tests")?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    run(Command::new("cargo")
        .args(["deny", "--log-level", "error", "check"])
        .current_dir(root()))
    .context("cargo-deny is required: `cargo install --locked cargo-deny`")?;
    licenses(true)
}

/// THIRD_PARTY_LICENSES: a header, the components cargo can't see
/// (`licenses/bundled.txt`), the crates in Velopack's native binaries
/// (`licenses/velopack.txt`, from scripts/release/velopack-notices.sh), then
/// every shipped crate from cargo-about.
fn licenses(check: bool) -> Result<()> {
    let root = root();
    let target = target_dir();
    // Written to a file, not read from stdout: under PowerShell, cargo-about
    // refuses to write to a pipe (to dodge PowerShell's re-encoding).
    let generated = target.join("cargo-about.txt");
    fs::create_dir_all(&target)?;
    let out = Command::new("cargo")
        .args([
            "about",
            "generate",
            "-c",
            "about.toml",
            "licenses/about.hbs",
            "--workspace",
            // --locked, not --frozen: on a fresh runner the crates for the
            // other release targets are not downloaded yet, and their
            // license files are what this reads. Cargo.lock still pins them.
            "--locked",
            "--fail",
            "-o",
        ])
        .arg(&generated)
        .current_dir(&root)
        .output()
        .context("cargo-about is required: `cargo install --locked cargo-about`")?;
    if !out.status.success() {
        bail!(
            "cargo about generate failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let crates = read(&generated)?;
    let bundled = read(&root.join("licenses/bundled.txt"))?;
    let velopack = read(&root.join("licenses/velopack.txt"))?;
    let notice = format!(
        "PwrRoute Third-Party Licenses\n\
         =============================\n\
         \n\
         Generated by `cargo xtask licenses` from Cargo.lock (cargo-about),\n\
         licenses/bundled.txt and licenses/velopack.txt. Do not edit this file\n\
         by hand.\n\
         \n\
         PwrRoute's own license, and Finicky's for the code derived from it, are\n\
         in LICENSE.\n\
         \n\
         {bundled}\n{velopack}\n{crates}"
    );
    let notice = normalize(&notice);
    let path = root.join("THIRD_PARTY_LICENSES");
    if check {
        // Normalized the same way, in case a checkout has CRLF.
        let current = normalize(&read(&path)?);
        if current != notice {
            let fresh = target.join("THIRD_PARTY_LICENSES.generated");
            fs::write(&fresh, &notice)?;
            eprintln!("generated copy for comparison: {}", fresh.display());
        }
        ensure!(
            current == notice,
            "THIRD_PARTY_LICENSES is out of date: run `cargo xtask licenses` and commit it"
        );
        println!("THIRD_PARTY_LICENSES is current");
    } else {
        fs::write(&path, notice)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// LF line endings and exactly one trailing newline. Some crates ship CRLF
/// license files, and a Windows checkout without .gitattributes would too.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_owned() + "\n"
}

fn release_check(args: &[String]) -> Result<()> {
    let mut tag = env::var("RELEASE_TAG").ok();
    let mut notes_file = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tag" => tag = it.next().cloned(),
            "--notes-file" => notes_file = it.next().map(PathBuf::from),
            other => bail!("release-check: unknown argument `{other}`"),
        }
    }
    let tag = tag.context("release-check needs --tag vX.Y.Z (or RELEASE_TAG)")?;
    let version = workspace_version()?;
    ensure!(
        tag == format!("v{version}"),
        "tag {tag} does not match the Cargo.toml workspace version {version}"
    );
    let changelog = read(&root().join("CHANGELOG.md"))?;
    let notes = changelog_section(&changelog, &version)
        .with_context(|| format!("CHANGELOG.md has no `## v{version}` section"))?;
    ensure!(
        !notes.trim().is_empty(),
        "CHANGELOG.md's `## v{version}` section is empty"
    );
    if let Some(path) = notes_file {
        fs::write(&path, notes)?;
    }
    println!("release metadata agrees for {tag}");
    Ok(())
}

/// The lines under `## vX.Y.Z` (or `## X.Y.Z`), up to the next `## `.
fn changelog_section(changelog: &str, version: &str) -> Option<String> {
    let mut lines = changelog.lines();
    lines.find(|line| {
        line.strip_prefix("## ")
            .map(|h| h.trim_start_matches('v'))
            .and_then(|h| h.split_whitespace().next())
            == Some(version)
    })?;
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("## ")).collect();
    Some(body.join("\n").trim().to_owned() + "\n")
}

/// The version packages are named and stamped with: `PWRROUTE_VERSION` when
/// set (preview builds use `0.0.1-pr.12`), else the workspace version.
/// Cargo.toml is never edited for a preview, so `--locked` keeps holding.
pub(crate) fn package_version() -> Result<String> {
    match env::var("PWRROUTE_VERSION") {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => workspace_version(),
    }
}

pub(crate) fn workspace_version() -> Result<String> {
    let manifest = read(&root().join("Cargo.toml"))?;
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
        } else if in_package && let Some(v) = line.strip_prefix("version = ") {
            return Ok(v.trim_matches('"').to_owned());
        }
    }
    bail!("no version in [workspace.package]")
}

pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

/// Cargo's target directory: `CARGO_TARGET_DIR` when set, else `target/` in
/// the workspace. Where `cargo build` puts the binaries `bundle` and
/// `package-linux` copy, and where they and `licenses` write their output.
pub(crate) fn target_dir() -> PathBuf {
    target_dir_from(&root(), env::var_os("CARGO_TARGET_DIR"))
}

/// A relative `CARGO_TARGET_DIR` is taken from the workspace root, which is
/// where xtask runs cargo, so it names the directory cargo builds into.
fn target_dir_from(root: &Path, var: Option<OsString>) -> PathBuf {
    match var {
        Some(dir) if !dir.is_empty() => root.join(dir),
        _ => root.join("target"),
    }
}

pub(crate) fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| path.display().to_string())
}

fn cargo(args: &[&str]) -> Result<()> {
    run(Command::new("cargo").args(args).current_dir(root()))
}

pub(crate) fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("could not run {command:?}"))?;
    ensure!(status.success(), "{command:?} failed: {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANGELOG: &str =
        "# Changelog\n\n## v0.2.0\n\n- Second.\n\n## v0.1.0 - 2026-10-13\n\n- First.\n";

    #[test]
    fn finds_a_changelog_section() {
        assert_eq!(
            changelog_section(CHANGELOG, "0.1.0").as_deref(),
            Some("- First.\n")
        );
        assert_eq!(
            changelog_section(CHANGELOG, "0.2.0").as_deref(),
            Some("- Second.\n")
        );
        assert_eq!(changelog_section(CHANGELOG, "0.3.0"), None);
    }

    #[test]
    fn notices_compare_equal_across_line_endings() {
        let lf = "A\n\nB\n";
        assert_eq!(normalize("A\r\n\r\nB\r\n"), lf);
        assert_eq!(normalize("A\n\nB\n\n\n"), lf);
        assert_eq!(normalize(lf), lf);
    }

    #[test]
    fn a_prefix_is_not_a_match() {
        assert_eq!(changelog_section(CHANGELOG, "0.1"), None);
    }

    #[test]
    fn the_target_dir_follows_cargo_target_dir() {
        let root = Path::new("/work/PwrRoute");
        assert_eq!(target_dir_from(root, None), root.join("target"));
        assert_eq!(
            target_dir_from(root, Some(OsString::new())),
            root.join("target"),
            "an empty value is unset, as cargo treats it"
        );
        let elsewhere = env::temp_dir().join("pwrroute-target");
        assert_eq!(
            target_dir_from(root, Some(elsewhere.clone().into_os_string())),
            elsewhere
        );
        assert_eq!(
            target_dir_from(root, Some("build/out".into())),
            root.join("build/out")
        );
    }

    #[test]
    fn reads_the_workspace_version() {
        let version = workspace_version().unwrap();
        assert!(
            version.starts_with(|c: char| c.is_ascii_digit()),
            "{version}"
        );
    }
}
