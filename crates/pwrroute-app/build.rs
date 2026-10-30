//! Captures where this binary was built from, for the rail's build block
//! and Settings › About (`src/build_info.rs`): the checkout, branch, commit
//! and whether it had uncommitted changes. Read from `git` here, once per
//! build; the app never looks at a checkout while it runs.
//!
//! Every value is an environment variable for `env!`, empty when unknown
//! (no `git`, or a source tree that is not a checkout).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let on_ci = std::env::var_os("GITHUB_ACTIONS").is_some();

    // One call for HEAD, the checkout and the git dirs `watch` needs.
    let parsed = git(
        &manifest,
        &[
            "rev-parse",
            "HEAD",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ],
    )
    .map(|out| out.lines().map(str::to_owned).collect::<Vec<_>>())
    .unwrap_or_default();
    let head = parsed.first().cloned();
    let toplevel = parsed.get(1).cloned();
    // A pull request's CI checkout is GitHub's merge commit, on no branch:
    // the workflow names the PR's head instead (preview-build.yml).
    let commit = std::env::var("PWRROUTE_SOURCE_SHA")
        .ok()
        .map(|sha| sha.trim().to_owned())
        .filter(|sha| !sha.is_empty())
        .or(head);
    // The subject and time of that commit; a shallow checkout may not
    // have the PR's head, and then neither is known.
    let (subject, commit_time) = commit
        .as_deref()
        .and_then(|commit| git(&manifest, &["log", "-1", "--format=%ct%n%s", commit]))
        .and_then(|out| {
            let (time, subject) = out.split_once('\n')?;
            Some((Some(subject.to_owned()), Some(time.to_owned())))
        })
        .unwrap_or((None, None));
    // `--no-optional-locks`: a build never rewrites the index.
    let dirty = git(
        &manifest,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--untracked-files=normal",
        ],
    )
    .is_some_and(|status| !status.is_empty());
    let branch = git(&manifest, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .or_else(|| ci_branch(on_ci));
    // HEAD may carry more than one tag; this version's is the one that
    // makes it a release.
    let release_tag = format!(
        "v{}",
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    );
    let tags = git(&manifest, &["tag", "--points-at", "HEAD"]).unwrap_or_default();
    let tag = tags
        .lines()
        .find(|tag| *tag == release_tag)
        .or_else(|| tags.lines().next())
        .map(str::to_owned)
        .or_else(|| ci_tag(on_ci));
    // A runner's path says nothing about the build, so CI leaves it out.
    let checkout = if on_ci { None } else { toplevel };
    // The time only means something when the tree had changes: otherwise
    // the commit says it all, and a time would set two identical builds
    // apart. SOURCE_DATE_EPOCH wins, for reproducible builds.
    let built_at = dirty.then(|| {
        std::env::var("SOURCE_DATE_EPOCH").ok().unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs().to_string())
                .unwrap_or_default()
        })
    });

    emit("PWRROUTE_BUILD_COMMIT", commit.as_deref());
    emit("PWRROUTE_BUILD_SUBJECT", subject.as_deref());
    emit("PWRROUTE_BUILD_COMMIT_TIME", commit_time.as_deref());
    emit("PWRROUTE_BUILD_BRANCH", branch.as_deref());
    emit("PWRROUTE_BUILD_TAG", tag.as_deref());
    emit("PWRROUTE_BUILD_DIRTY", dirty.then_some("1"));
    emit("PWRROUTE_BUILD_CHECKOUT", checkout.as_deref());
    emit("PWRROUTE_BUILD_TIME", built_at.as_deref());

    watch(&manifest, parsed.get(2), parsed.get(3));

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources(&manifest);
    }
}

/// Embeds PwrRoute's icon in `pwrroute.exe` as icon resource 1, which GPUI
/// loads for every window (the taskbar, Alt+Tab) and Explorer shows for the
/// file, plus the version block Task Manager and the file's Properties
/// name it by. GPUI's own resources (its manifest) are separate.
fn windows_resources(manifest: &Path) {
    let icon = manifest.join("../../packaging/windows/PwrRoute.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let icon = icon
        .canonicalize()
        .unwrap_or(icon)
        .display()
        .to_string()
        // A path from canonicalize carries `\\?\`, which rc does not read.
        .trim_start_matches(r"\\?\")
        .replace('\\', r"\\");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    // FILEVERSION wants four numbers; a pre-release suffix is dropped.
    let mut numbers: Vec<u16> = version
        .split(['-', '+'])
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    numbers.resize(4, 0);
    let numeric = numbers
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let rc = format!(
        r#"1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "PwrRoute"
      VALUE "ProductName", "PwrRoute"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "pwrroute.exe"
      VALUE "InternalName", "pwrroute"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let rc_path = out.join("pwrroute.rc");
    std::fs::write(&rc_path, rc).expect("write pwrroute.rc");
    embed_resource::compile_for(&rc_path, ["pwrroute"], embed_resource::NONE)
        .manifest_required()
        .expect("embed PwrRoute's icon in pwrroute.exe");
}

fn emit(name: &str, value: Option<&str>) {
    // One line per value: a newline would end the directive.
    let value = value.unwrap_or_default().replace(['\r', '\n'], " ");
    println!("cargo:rustc-env={name}={value}");
}

/// Trimmed stdout of `git -C dir args`, or `None` when git fails or says
/// nothing.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// A pull request's checkout is a detached merge commit; the branch it
/// builds is the PR's head.
fn ci_branch(on_ci: bool) -> Option<String> {
    if !on_ci {
        return None;
    }
    let head = std::env::var("GITHUB_HEAD_REF")
        .ok()
        .filter(|v| !v.is_empty());
    head.or_else(|| {
        (std::env::var("GITHUB_REF_TYPE").ok().as_deref() == Some("branch"))
            .then(|| std::env::var("GITHUB_REF_NAME").ok())
            .flatten()
            .filter(|v| !v.is_empty())
    })
}

/// A tag push's shallow checkout may not carry the tag object.
fn ci_tag(on_ci: bool) -> Option<String> {
    if !on_ci || std::env::var("GITHUB_REF_TYPE").ok().as_deref() != Some("tag") {
        return None;
    }
    std::env::var("GITHUB_REF_NAME")
        .ok()
        .filter(|v| !v.is_empty())
}

/// Run again when HEAD moves (a checkout, a commit on a detached HEAD), a
/// branch or tag changes (a commit) or anything under `crates/` does (an
/// edit that makes or clears uncommitted changes). Not on the index: tools
/// that only refresh it would rebuild the app for nothing.
fn watch(manifest: &Path, git_dir: Option<&String>, common_dir: Option<&String>) {
    for var in [
        "GITHUB_ACTIONS",
        "GITHUB_HEAD_REF",
        "GITHUB_REF_NAME",
        "GITHUB_REF_TYPE",
        "PWRROUTE_SOURCE_SHA",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rerun-if-changed=build.rs");
    if let Some(crates) = manifest.parent() {
        println!("cargo:rerun-if-changed={}", crates.display());
        if let Some(root) = crates.parent() {
            for file in ["Cargo.toml", "Cargo.lock"] {
                println!("cargo:rerun-if-changed={}", root.join(file).display());
            }
        }
    }
    let absolute = |path: &String| {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            manifest.join(path)
        }
    };
    if let Some(git_dir) = git_dir.map(absolute) {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    }
    if let Some(common) = common_dir.map(absolute) {
        for path in ["refs/heads", "refs/tags", "packed-refs"] {
            println!("cargo:rerun-if-changed={}", common.join(path).display());
        }
    }
}
