//! Which build this is (design: Build Identity): the version, profile and
//! architecture, and the checkout, branch and commit `build.rs` read from
//! git when the binary was built. The rail's build block, Settings › About
//! and Diagnostics all draw from one [`BuildInfo`], and their copy buttons
//! write one [`BuildInfo::report`].
//!
//! Nothing here reads a checkout at run time, and nothing here is a link.

use std::path::Path;

/// Where this binary came from. Strings are empty-free: an unknown value is
/// `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// "0.5.0".
    pub version: String,
    /// "debug" or "release".
    pub profile: &'static str,
    /// "aarch64".
    pub arch: &'static str,
    /// All 40 hex characters.
    pub commit: Option<String>,
    /// The commit's subject line.
    pub subject: Option<String>,
    /// The commit's time, in seconds since the epoch.
    pub commit_time: Option<i64>,
    /// `None` for a detached HEAD.
    pub branch: Option<String>,
    /// The tag on HEAD, if any.
    pub tag: Option<String>,
    /// The tree had uncommitted changes.
    pub dirty: bool,
    /// The checkout's absolute path; never set for a CI build.
    pub checkout: Option<String>,
    /// When it was built, only when `dirty` (seconds since the epoch).
    pub built_at: Option<i64>,
}

/// This binary's: [`BuildInfo::current`].
impl Default for BuildInfo {
    fn default() -> Self {
        Self::current()
    }
}

/// Each row of the rail's block, and what its click copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    /// The whole block: [`BuildInfo::report`].
    All,
    Checkout,
    Branch,
    Commit,
}

fn known(value: &'static str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

impl BuildInfo {
    /// This binary, as `build.rs` saw it.
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            arch: std::env::consts::ARCH,
            commit: known(env!("PWRROUTE_BUILD_COMMIT")),
            subject: known(env!("PWRROUTE_BUILD_SUBJECT")),
            commit_time: known(env!("PWRROUTE_BUILD_COMMIT_TIME")).and_then(|t| t.parse().ok()),
            branch: known(env!("PWRROUTE_BUILD_BRANCH")),
            tag: known(env!("PWRROUTE_BUILD_TAG")),
            dirty: !env!("PWRROUTE_BUILD_DIRTY").is_empty(),
            checkout: known(env!("PWRROUTE_BUILD_CHECKOUT")),
            built_at: known(env!("PWRROUTE_BUILD_TIME")).and_then(|t| t.parse().ok()),
        }
    }

    /// Contrived, for tests and the `screens` example: a worktree on a
    /// branch with nothing uncommitted.
    pub fn fixture() -> Self {
        Self {
            version: "0.5.0".to_owned(),
            profile: "debug",
            arch: "aarch64",
            commit: Some("5e1c07a2f19d4b6c08e3a7d5b2c91f40e6a8d317".to_owned()),
            subject: Some("show which build is running".to_owned()),
            commit_time: Some(1_793_196_120),
            branch: Some("feat/build-identity".to_owned()),
            tag: None,
            dirty: false,
            checkout: Some("/Users/you/.pwragent/worktrees/k3x9q2ab/PwrRoute".to_owned()),
            built_at: None,
        }
    }

    /// A release: the release profile, built from exactly its `vX.Y.Z`
    /// tag with nothing uncommitted. Everything else is a dev build.
    pub fn is_release(&self) -> bool {
        self.profile == "release"
            && !self.dirty
            && self.tag.as_deref() == Some(format!("v{}", self.version).as_str())
    }

    /// The first 8 characters of the commit.
    pub fn short_commit(&self) -> Option<&str> {
        self.commit
            .as_deref()
            .map(|commit| commit.get(..8).unwrap_or(commit))
    }

    /// "5e1c07a2", with "+" for uncommitted changes: Diagnostics' Version.
    pub fn short_commit_marked(&self) -> Option<String> {
        self.short_commit()
            .map(|short| format!("{short}{}", if self.dirty { "+" } else { "" }))
    }

    /// "0.5.0 · debug": the rail block's header.
    pub fn version_and_profile(&self) -> String {
        format!("{} \u{00b7} {}", self.version, self.profile)
    }

    /// "v0.5.0 · 42a50803": a release's one line in the rail.
    pub fn release_line(&self) -> String {
        match self.short_commit() {
            Some(short) => format!("v{} \u{00b7} {short}", self.version),
            None => format!("v{}", self.version),
        }
    }

    /// The checkout as the rail shows it: `~` for home, and past
    /// `max_chars` its last two folders (a worktree's id is the part that
    /// tells checkouts apart).
    pub fn checkout_label(&self, home: Option<&Path>, max_chars: usize) -> Option<String> {
        let checkout = self.checkout.as_deref()?;
        let shown = match home.and_then(|home| Path::new(checkout).strip_prefix(home).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Some(rest) => Path::new("~").join(rest).display().to_string(),
            None => checkout.to_owned(),
        };
        if shown.chars().count() <= max_chars {
            return Some(shown);
        }
        let parts: Vec<&str> = Path::new(checkout)
            .components()
            .filter_map(|part| part.as_os_str().to_str())
            .collect();
        Some(match parts.as_slice() {
            [.., parent, name] => format!("{parent}/{name}"),
            _ => shown,
        })
    }

    /// The full value a row's click copies.
    pub fn copy_text(&self, part: Part) -> Option<String> {
        match part {
            Part::All => Some(self.report()),
            Part::Checkout => self.checkout.clone(),
            Part::Branch => self.branch.clone(),
            Part::Commit => self.commit.clone(),
        }
    }

    /// "debug build for aarch64".
    pub fn build_line(&self) -> String {
        format!("{} build for {}", self.profile, self.arch)
    }

    /// "“show which build is running” · committed Oct 28 14:02".
    pub fn commit_line(&self) -> Option<String> {
        let subject = self
            .subject
            .as_deref()
            .map(|subject| format!("\u{201c}{subject}\u{201d}"));
        let when = self
            .commit_time
            .and_then(local)
            .map(|time| format!("committed {}", time.format("%b %-d %H:%M")));
        match (subject, when) {
            (Some(subject), Some(when)) => Some(format!("{subject} \u{00b7} {when}")),
            (one, other) => one.or(other),
        }
    }

    /// "Built at 14:32 with uncommitted changes, …", only when there were.
    pub fn changes_line(&self) -> Option<String> {
        if !self.dirty {
            return None;
        }
        Some(match self.built_at.and_then(local) {
            Some(time) => format!(
                "Built {} with uncommitted changes, so it is more than this commit.",
                time.format("%b %-d at %H:%M")
            ),
            None => "Built with uncommitted changes, so it is more than this commit.".to_owned(),
        })
    }

    /// The text every "copy the build" button writes: the rail's, About's
    /// Copy build info, and the top of Diagnostics' Copy. The checkout is
    /// the full path, one you can `cd` to.
    pub fn report(&self) -> String {
        const PAD: &str = "          ";
        let mut lines = vec![format!(
            "PwrRoute {} ({}, {})",
            self.version, self.profile, self.arch
        )];
        if let Some(checkout) = &self.checkout {
            lines.push(format!("Checkout: {checkout}"));
        }
        if self.is_release()
            && let Some(tag) = &self.tag
        {
            lines.push(format!("Tag:      {tag}"));
        } else {
            lines.push(format!(
                "Branch:   {}",
                self.branch.as_deref().unwrap_or("detached HEAD")
            ));
        }
        let changes = if self.dirty {
            " + uncommitted changes"
        } else {
            ""
        };
        lines.push(format!(
            "Commit:   {}{changes}",
            self.commit.as_deref().unwrap_or("unknown")
        ));
        if let Some(subject) = &self.subject {
            lines.push(format!("{PAD}{subject}"));
        }
        if let Some(built) = self.built_at.and_then(local) {
            lines.push(format!("Built:    {}", built.format("%Y-%m-%d %H:%M")));
        }
        let mut report = lines.join("\n");
        report.push('\n');
        report
    }
}

fn local(seconds: i64) -> Option<chrono::DateTime<chrono::Local>> {
    chrono::DateTime::from_timestamp(seconds, 0).map(|time| time.with_timezone(&chrono::Local))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> BuildInfo {
        BuildInfo {
            profile: "release",
            tag: Some("v0.5.0".to_owned()),
            branch: None,
            checkout: None,
            commit: Some("42a50803e6b1c9d27f0a4c35e8d1b6a0f9c2e471".to_owned()),
            subject: Some("chore(release): v0.5.0".to_owned()),
            ..BuildInfo::fixture()
        }
    }

    #[test]
    fn the_current_build_has_a_version_and_a_profile() {
        let build = BuildInfo::current();
        assert_eq!(build.version, env!("CARGO_PKG_VERSION"));
        assert!(["debug", "release"].contains(&build.profile));
        if let Some(commit) = &build.commit {
            assert_eq!(commit.len(), 40, "{commit}");
        }
        assert_eq!(build.built_at.is_some(), build.dirty);
    }

    #[test]
    fn only_a_clean_release_build_of_its_own_tag_is_a_release() {
        assert!(release().is_release());
        assert!(!BuildInfo::fixture().is_release());
        let dirty = BuildInfo {
            dirty: true,
            ..release()
        };
        assert!(!dirty.is_release());
        let debug = BuildInfo {
            profile: "debug",
            ..release()
        };
        assert!(!debug.is_release());
        let other_tag = BuildInfo {
            tag: Some("v0.4.0".to_owned()),
            ..release()
        };
        assert!(!other_tag.is_release());
    }

    #[test]
    fn the_commit_is_cut_to_eight_and_marked_for_changes() {
        let build = BuildInfo::fixture();
        assert_eq!(build.short_commit(), Some("5e1c07a2"));
        assert_eq!(build.short_commit_marked().as_deref(), Some("5e1c07a2"));
        let dirty = BuildInfo {
            dirty: true,
            ..build
        };
        assert_eq!(dirty.short_commit_marked().as_deref(), Some("5e1c07a2+"));
        assert_eq!(release().release_line(), "v0.5.0 \u{00b7} 42a50803");
    }

    #[test]
    fn the_checkout_shows_home_as_a_tilde_then_its_last_two_folders() {
        let build = BuildInfo::fixture();
        let home = Path::new("/Users/you");
        assert_eq!(
            build.checkout_label(Some(home), 80).as_deref(),
            Some("~/.pwragent/worktrees/k3x9q2ab/PwrRoute")
        );
        assert_eq!(
            build.checkout_label(Some(home), 24).as_deref(),
            Some("k3x9q2ab/PwrRoute")
        );
        let main = BuildInfo {
            checkout: Some("/Users/you/pwrdrvr/PwrRoute".to_owned()),
            ..BuildInfo::fixture()
        };
        assert_eq!(
            main.checkout_label(Some(home), 24).as_deref(),
            Some("~/pwrdrvr/PwrRoute")
        );
        assert_eq!(release().checkout_label(Some(home), 24), None);
    }

    #[test]
    fn rows_copy_their_full_values() {
        let build = BuildInfo::fixture();
        assert_eq!(
            build.copy_text(Part::Checkout).as_deref(),
            Some("/Users/you/.pwragent/worktrees/k3x9q2ab/PwrRoute")
        );
        assert_eq!(
            build.copy_text(Part::Branch).as_deref(),
            Some("feat/build-identity")
        );
        assert_eq!(
            build.copy_text(Part::Commit).as_deref(),
            Some("5e1c07a2f19d4b6c08e3a7d5b2c91f40e6a8d317")
        );
        assert_eq!(build.copy_text(Part::All), Some(build.report()));
        let detached = BuildInfo {
            branch: None,
            ..build
        };
        assert_eq!(detached.copy_text(Part::Branch), None);
    }

    #[test]
    fn the_report_names_the_checkout_branch_and_commit() {
        assert_eq!(
            BuildInfo::fixture().report(),
            "PwrRoute 0.5.0 (debug, aarch64)\n\
             Checkout: /Users/you/.pwragent/worktrees/k3x9q2ab/PwrRoute\n\
             Branch:   feat/build-identity\n\
             Commit:   5e1c07a2f19d4b6c08e3a7d5b2c91f40e6a8d317\n\
             \x20         show which build is running\n"
        );
    }

    #[test]
    fn a_release_report_names_its_tag_and_no_checkout() {
        assert_eq!(
            release().report(),
            "PwrRoute 0.5.0 (release, aarch64)\n\
             Tag:      v0.5.0\n\
             Commit:   42a50803e6b1c9d27f0a4c35e8d1b6a0f9c2e471\n\
             \x20         chore(release): v0.5.0\n"
        );
    }

    #[test]
    fn a_detached_build_with_changes_says_so_and_when_it_was_built() {
        let build = BuildInfo {
            branch: None,
            dirty: true,
            built_at: Some(1_793_197_920),
            ..BuildInfo::fixture()
        };
        let report = build.report();
        assert!(report.contains("Branch:   detached HEAD\n"), "{report}");
        assert!(
            report.contains("5e1c07a2f19d4b6c08e3a7d5b2c91f40e6a8d317 + uncommitted changes\n"),
            "{report}"
        );
        assert!(report.contains("\nBuilt:    20"), "{report}");
        assert!(build.changes_line().is_some());
        assert_eq!(BuildInfo::fixture().changes_line(), None);
    }

    #[test]
    fn nothing_known_still_reports_the_version() {
        let bare = BuildInfo {
            commit: None,
            subject: None,
            commit_time: None,
            branch: None,
            checkout: None,
            ..BuildInfo::fixture()
        };
        assert_eq!(
            bare.report(),
            "PwrRoute 0.5.0 (debug, aarch64)\nBranch:   detached HEAD\nCommit:   unknown\n"
        );
        assert_eq!(bare.release_line(), "v0.5.0");
        assert_eq!(bare.commit_line(), None);
    }
}
