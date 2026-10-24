//! Which release a copy updates to: the Pwr family's release channel, as
//! PwrSnap, PwrGit and PwrAgnt have it.
//!
//! Two trains, two tracks, four slots:
//!
//! | Slot                | Example tag               | What it is                          |
//! | ------------------- | ------------------------- | ----------------------------------- |
//! | Stable · Latest     | `v0.1.0` (promoted)       | Smoke-checked; the default          |
//! | Stable · Prerelease | `v0.1.1` staged, `-rc.1`  | Candidates for the stable line      |
//! | Beta · Latest       | `v0.2.0-beta.3`           | Beta builds off main                |
//! | Beta · Prerelease   | `v0.2.0-alpha.7`          | Newest alpha off main; may not work |
//!
//! Every CI release is born a GitHub Pre-release and promoted to Latest by
//! hand (`gh release edit vX.Y.Z --latest --prerelease=false`). Until it is
//! promoted, a suffix-free tag is offered to the Prerelease slots only, so
//! Stable · Latest keeps the previous promoted release.
//!
//! The slot follows the installed build ([`infer`]) until the user picks one
//! in Settings › Updates ([`UpdateSettings::resolve`]). [`select`] picks each
//! slot's release from the published list, by SemVer precedence, never by
//! publish date. Nothing here touches the network.

use std::fmt;

use semver::Version;
use serde::{Deserialize, Serialize};

/// The version line: the smoke-checked releases, or builds off main.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Train {
    #[default]
    Stable,
    Beta,
}

/// Within a train: the smoke-checked build, or the newest one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Track {
    #[default]
    Latest,
    Prerelease,
}

/// One of the four choices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Slot {
    pub train: Train,
    pub track: Track,
}

impl Slot {
    pub const STABLE_LATEST: Self = Self::new(Train::Stable, Track::Latest);
    pub const STABLE_PRERELEASE: Self = Self::new(Train::Stable, Track::Prerelease);
    pub const BETA_LATEST: Self = Self::new(Train::Beta, Track::Latest);
    pub const BETA_PRERELEASE: Self = Self::new(Train::Beta, Track::Prerelease);
    /// In the order Settings lays them out: by train, then by track.
    pub const ALL: [Self; 4] = [
        Self::STABLE_LATEST,
        Self::STABLE_PRERELEASE,
        Self::BETA_LATEST,
        Self::BETA_PRERELEASE,
    ];

    pub const fn new(train: Train, track: Track) -> Self {
        Self { train, track }
    }

    /// "stable:latest": what a download is recorded against.
    pub fn key(self) -> String {
        format!(
            "{}:{}",
            match self.train {
                Train::Stable => "stable",
                Train::Beta => "beta",
            },
            match self.track {
                Track::Latest => "latest",
                Track::Prerelease => "prerelease",
            }
        )
    }

    /// The inverse of [`Slot::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.key() == key)
    }
}

/// "Stable · Latest".
impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let train = match self.train {
            Train::Stable => "Stable",
            Train::Beta => "Beta",
        };
        let track = match self.track {
            Track::Latest => "Latest",
            Track::Prerelease => "Prerelease",
        };
        write!(f, "{train} \u{00b7} {track}")
    }
}

/// Whether the slot was picked in Settings or follows the installed build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SelectionSource {
    /// Re-derived from the running build every time it is read, so
    /// installing an alpha over a stable build moves the feed with it.
    #[default]
    Inferred,
    /// Picked in Settings › Updates; kept until another pick.
    User,
}

/// `settings.json`'s `updates`: the slot the user picked, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub train: Option<Train>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<Track>,
    pub selection_source: SelectionSource,
}

impl UpdateSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// A pick in Settings: both axes, kept.
    pub fn picked(slot: Slot) -> Self {
        Self {
            train: Some(slot.train),
            channel: Some(slot.track),
            selection_source: SelectionSource::User,
        }
    }

    /// The slot in force for a copy at `installed`: the user's pick, each
    /// axis falling back to its default on its own, or the one the build
    /// implies.
    pub fn resolve(&self, installed: &str) -> Slot {
        match self.selection_source {
            SelectionSource::User => Slot::new(
                self.train.unwrap_or_default(),
                self.channel.unwrap_or_default(),
            ),
            SelectionSource::Inferred => infer(installed),
        }
    }

    /// The slot follows the build, rather than a pick.
    pub fn is_inferred(&self) -> bool {
        self.selection_source == SelectionSource::Inferred
    }
}

/// The slot a build at `version` belongs to, by its prerelease suffix:
/// none is Stable · Latest, `-alpha` Beta · Prerelease, `-beta` Beta ·
/// Latest, and anything else (`-rc`, `-prerelease`) Stable · Prerelease. A
/// version that does not parse is Stable · Latest.
pub fn infer(version: &str) -> Slot {
    let Some(version) = parse(version) else {
        return Slot::STABLE_LATEST;
    };
    match pre0(&version).as_deref() {
        None => Slot::STABLE_LATEST,
        Some("alpha") => Slot::BETA_PRERELEASE,
        Some("beta") => Slot::BETA_LATEST,
        Some(_) => Slot::STABLE_PRERELEASE,
    }
}

/// A published release, as far as choosing one goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// "v0.1.0".
    pub tag: String,
    /// GitHub's Pre-release flag: not promoted yet.
    pub prerelease: bool,
}

impl Release {
    pub fn new(tag: impl Into<String>, prerelease: bool) -> Self {
        Self {
            tag: tag.into(),
            prerelease,
        }
    }

    /// The tag as SemVer, without its `v`.
    pub fn version(&self) -> Option<Version> {
        parse(&self.tag)
    }
}

/// `version`, with or without a leading `v`, as SemVer.
pub fn parse(version: &str) -> Option<Version> {
    Version::parse(version.trim().trim_start_matches('v')).ok()
}

/// The first prerelease identifier, lowercased: "alpha" for `-alpha.7`.
fn pre0(version: &Version) -> Option<String> {
    let pre = version.pre.as_str();
    (!pre.is_empty()).then(|| {
        pre.split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    })
}

fn core(version: &Version) -> (u64, u64, u64) {
    (version.major, version.minor, version.patch)
}

/// The release each slot resolves to, from `releases` (drafts and releases
/// without this platform's update feed already left out). A slot with no
/// release is `None`.
pub fn select(releases: &[Release], slot: Slot) -> Option<Release> {
    let parsed: Vec<(Version, &Release)> = releases
        .iter()
        .filter_map(|release| Some((release.version()?, release)))
        .collect();
    let highest = |keep: &dyn Fn(&Version, &Release) -> bool| {
        parsed
            .iter()
            .filter(|(version, release)| keep(version, release))
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(version, release)| (version.clone(), *release))
    };
    // The promoted release. A suffix-free tag first, so a suffixed tag
    // promoted by mistake cannot take the slot while a real one exists.
    let stable_latest = highest(&|v, r| !r.prerelease && v.pre.is_empty())
        .or_else(|| highest(&|_, r| !r.prerelease));
    let stable_core = stable_latest.as_ref().map(|(v, _)| core(v));
    let stable_final = stable_latest
        .as_ref()
        .filter(|(v, _)| v.pre.is_empty())
        .cloned();
    let is_beta_train = |v: &Version, r: &Release| {
        let pre = pre0(v);
        r.prerelease
            && matches!(pre.as_deref(), Some("alpha" | "beta"))
            && match stable_core {
                // Ahead of the promoted line, or it is that line's past.
                Some(stable) => core(v) > stable,
                None => {
                    pre.as_deref() == Some("alpha")
                        || parsed.iter().any(|(other, other_release)| {
                            other_release.prerelease
                                && core(other) == core(v)
                                && pre0(other).as_deref() == Some("alpha")
                        })
                }
            }
    };
    let is_beta_latest =
        |v: &Version, r: &Release| pre0(v).as_deref() == Some("beta") && is_beta_train(v, r);
    let ahead_of_stable = |v: &Version| stable_latest.as_ref().is_none_or(|(s, _)| v > s);
    let is_stable_candidate_ahead = |v: &Version, r: &Release| {
        r.prerelease
            && ahead_of_stable(v)
            && matches!(pre0(v).as_deref(), None | Some("prerelease" | "rc"))
    };
    let chosen = match (slot.train, slot.track) {
        (Train::Stable, Track::Latest) => stable_latest,
        // Staged finals, -rc, -prerelease and the like; never an alpha or a
        // beta-train build, and never below the promoted release.
        (Train::Stable, Track::Prerelease) => at_least(
            highest(&|v, r| {
                r.prerelease && pre0(v).as_deref() != Some("alpha") && !is_beta_latest(v, r)
            }),
            stable_latest,
        ),
        (Train::Beta, Track::Latest) => highest(&is_beta_latest).or(stable_final),
        (Train::Beta, Track::Prerelease) => at_least(
            highest(&|v, r| is_beta_train(v, r) || is_stable_candidate_ahead(v, r)),
            stable_final,
        ),
    };
    chosen.map(|(_, release)| release.clone())
}

/// The higher of the two, by precedence.
fn at_least<'a>(
    candidate: Option<(Version, &'a Release)>,
    floor: Option<(Version, &'a Release)>,
) -> Option<(Version, &'a Release)> {
    match (candidate, floor) {
        (Some(c), Some(f)) => Some(if c.0 >= f.0 { c } else { f }),
        (c, f) => c.or(f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promoted(tag: &str) -> Release {
        Release::new(tag, false)
    }

    fn pre(tag: &str) -> Release {
        Release::new(tag, true)
    }

    fn tags(releases: &[Release]) -> [Option<String>; 4] {
        Slot::ALL.map(|slot| select(releases, slot).map(|r| r.tag))
    }

    fn some(tag: &str) -> Option<String> {
        Some(tag.to_owned())
    }

    #[test]
    fn the_build_implies_its_slot() {
        assert_eq!(infer("0.1.0"), Slot::STABLE_LATEST);
        assert_eq!(infer("v1.2.3"), Slot::STABLE_LATEST);
        assert_eq!(infer("0.1.0-rc.3"), Slot::STABLE_PRERELEASE);
        assert_eq!(infer("0.1.1-prerelease.1"), Slot::STABLE_PRERELEASE);
        assert_eq!(infer("0.2.0-beta.3"), Slot::BETA_LATEST);
        assert_eq!(infer("0.2.0-alpha.7"), Slot::BETA_PRERELEASE);
        assert_eq!(infer("0.0.1-Alpha.0"), Slot::BETA_PRERELEASE);
        assert_eq!(infer("0.2.0-nightly.1"), Slot::STABLE_PRERELEASE);
        assert_eq!(infer("not a version"), Slot::STABLE_LATEST);
    }

    #[test]
    fn a_pick_is_kept_and_an_inferred_slot_follows_the_build() {
        let inferred = UpdateSettings::default();
        assert!(inferred.is_inferred());
        assert_eq!(inferred.resolve("0.1.0-rc.3"), Slot::STABLE_PRERELEASE);
        assert_eq!(inferred.resolve("0.1.0"), Slot::STABLE_LATEST);
        let picked = UpdateSettings::picked(Slot::BETA_PRERELEASE);
        assert!(!picked.is_inferred());
        assert_eq!(picked.resolve("0.1.0"), Slot::BETA_PRERELEASE);
        // A half pick falls back per axis.
        let half = UpdateSettings {
            train: Some(Train::Beta),
            channel: None,
            selection_source: SelectionSource::User,
        };
        assert_eq!(half.resolve("0.1.0-alpha.1"), Slot::BETA_LATEST);
    }

    #[test]
    fn the_settings_read_and_write_back_as_the_siblings_store_them() {
        let picked = UpdateSettings::picked(Slot::STABLE_PRERELEASE);
        let json = serde_json::to_value(picked).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "train": "stable",
                "channel": "prerelease",
                "selectionSource": "user"
            })
        );
        assert_eq!(
            serde_json::from_value::<UpdateSettings>(serde_json::json!({})).unwrap(),
            UpdateSettings::default()
        );
        for slot in Slot::ALL {
            assert_eq!(Slot::from_key(&slot.key()), Some(slot));
        }
        assert_eq!(Slot::BETA_LATEST.to_string(), "Beta \u{00b7} Latest");
    }

    #[test]
    fn a_staged_final_waits_for_promotion_on_stable_latest() {
        // v0.1.0 is published (born a Pre-release) over the rc line, and
        // nothing has been promoted yet.
        let releases = [pre("v0.1.0"), pre("v0.1.0-rc.3"), pre("v0.1.0-rc.2")];
        assert_eq!(
            tags(&releases),
            [None, some("v0.1.0"), None, some("v0.1.0")]
        );
        // Promoted: every slot but the beta ones' own builds has it.
        let releases = [promoted("v0.1.0"), pre("v0.1.0-rc.3")];
        assert_eq!(
            tags(&releases),
            [
                some("v0.1.0"),
                some("v0.1.0"),
                some("v0.1.0"),
                some("v0.1.0")
            ]
        );
        // The next final staged: Stable · Latest keeps the promoted one.
        let releases = [pre("v0.1.1"), promoted("v0.1.0")];
        assert_eq!(
            tags(&releases),
            [
                some("v0.1.0"),
                some("v0.1.1"),
                some("v0.1.0"),
                some("v0.1.1")
            ]
        );
    }

    #[test]
    fn each_train_takes_its_own_builds() {
        // The next line's alphas, this line's betas.
        let releases = [
            pre("v0.3.0-alpha.7"),
            pre("v0.2.0-beta.3"),
            pre("v0.1.1-rc.1"),
            promoted("v0.1.0"),
            pre("v0.1.0-rc.3"),
        ];
        assert_eq!(
            tags(&releases),
            [
                some("v0.1.0"),
                some("v0.1.1-rc.1"),
                some("v0.2.0-beta.3"),
                some("v0.3.0-alpha.7"),
            ]
        );
        // Within one line a beta comes after its alphas.
        let releases = [
            pre("v0.2.0-alpha.7"),
            pre("v0.2.0-beta.3"),
            promoted("v0.1.0"),
        ];
        assert_eq!(tags(&releases)[3], some("v0.2.0-beta.3"));
        // Precedence, not publish order: a newer alpha listed last wins.
        let releases = [
            pre("v0.2.0-alpha.2"),
            promoted("v0.1.0"),
            pre("v0.2.0-alpha.10"),
        ];
        assert_eq!(tags(&releases)[3], some("v0.2.0-alpha.10"));
    }

    #[test]
    fn a_promoted_line_retires_its_betas() {
        // v0.2.0 promoted: the leftover v0.2.0 betas and alphas are its
        // past, and Beta falls back to the release.
        let releases = [
            promoted("v0.2.0"),
            pre("v0.2.0-beta.3"),
            pre("v0.2.0-alpha.7"),
            promoted("v0.1.0"),
        ];
        assert_eq!(
            tags(&releases),
            [
                some("v0.2.0"),
                some("v0.2.0"),
                some("v0.2.0"),
                some("v0.2.0")
            ]
        );
    }

    #[test]
    fn a_mis_promoted_suffix_does_not_take_stable_latest() {
        let releases = [promoted("v0.2.0-alpha.1"), promoted("v0.1.0")];
        assert_eq!(tags(&releases)[0], some("v0.1.0"));
        // With no suffix-free release promoted at all, it is all there is.
        assert_eq!(
            select(&[promoted("v0.2.0-alpha.1")], Slot::STABLE_LATEST).map(|r| r.tag),
            some("v0.2.0-alpha.1")
        );
    }

    #[test]
    fn nothing_published_leaves_a_slot_empty_and_bad_tags_are_skipped() {
        assert_eq!(tags(&[]), [None, None, None, None]);
        // A release candidate is offered to both Prerelease slots.
        let releases = [pre("nightly"), pre("v0.1.0-rc.1")];
        assert_eq!(
            tags(&releases),
            [None, some("v0.1.0-rc.1"), None, some("v0.1.0-rc.1")]
        );
        // Before any stable release, an alpha line is the beta train.
        let releases = [pre("v0.0.1-alpha.3")];
        assert_eq!(tags(&releases), [None, None, None, some("v0.0.1-alpha.3")]);
    }
}
