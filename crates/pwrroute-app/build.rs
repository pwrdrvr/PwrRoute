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

    let commit = git(&manifest, &["rev-parse", "HEAD"]);
    let toplevel = git(&manifest, &["rev-parse", "--show-toplevel"]);
    let subject = git(&manifest, &["log", "-1", "--format=%s"]);
    let commit_time = git(&manifest, &["log", "-1", "--format=%ct"]);
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
    let tag =
        git(&manifest, &["describe", "--tags", "--exact-match", "HEAD"]).or_else(|| ci_tag(on_ci));
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

    watch(&manifest);
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
fn watch(manifest: &Path) {
    for var in [
        "GITHUB_ACTIONS",
        "GITHUB_HEAD_REF",
        "GITHUB_REF_NAME",
        "GITHUB_REF_TYPE",
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
    let absolute = |path: String| {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            manifest.join(path)
        }
    };
    if let Some(git_dir) = git(manifest, &["rev-parse", "--git-dir"]).map(absolute) {
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    }
    if let Some(common) = git(manifest, &["rev-parse", "--git-common-dir"]).map(absolute) {
        for path in ["refs/heads", "refs/tags", "packed-refs"] {
            println!("cargo:rerun-if-changed={}", common.join(path).display());
        }
    }
}
