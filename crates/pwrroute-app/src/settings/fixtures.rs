//! Contrived Settings data, transcribed from the design's panes
//! (`design/PwrRoute Settings.dc.html`, `design/PwrRoute Updates and
//! Default Browser.dc.html`). Nothing here is read from this machine: the
//! shipped app builds every pane from `live.rs`, and these draw the design
//! canvas (the `screens` example) and the window's tests.

use crate::build_info::BuildInfo;

use super::model::{
    About, AgentAction, AgentRole, AgentSession, BrowserApp, Browsers, Configuration,
    DefaultBrowser, DefaultState, Diagnostics, FinickyConfig, Grant, ImportCandidate, ImportPlan,
    JournalFiles, JournalLevel, Listener, LocalAgents, Permission, Piece, Privacy, RoleId,
    RulesSource, SessionId, SettingsModel, Slot, SlotTile, Status, UpdateState, Updates, Usage,
};

pub fn model() -> SettingsModel {
    SettingsModel {
        local_agents: local_agents(),
        configuration: configuration(),
        sync: sync(),
        privacy: privacy(),
        browsers: browsers(),
        updates: updates(),
        experimental: super::model::Experimental {
            yield_browser_focus: Some(false),
        },
        diagnostics: diagnostics(),
        about: about(),
    }
}

/// Sync as the design's 1b draws it (Synced Rules): two folders, one
/// waiting for iCloud, Google Drive offered, and the machine card's
/// destinations and sets from 2a. Contrived throughout.
pub fn sync() -> super::model::Sync {
    use super::model::{DestinationRow, RuleSetRow, Sync, SyncFolder, SyncPlace, SyncTone};
    use std::path::PathBuf;
    let row = |name: &str, here: &str, this_machine: bool, uses: usize, warning: Option<&str>| {
        DestinationRow {
            name: name.to_owned().into(),
            here: here.to_owned().into(),
            this_machine,
            uses,
            warning: warning.map(|w| w.to_owned().into()),
            lanes: Vec::new(),
        }
    };
    Sync {
        machine_name: "Studio Mac".into(),
        computer_name: "Studio Mac".into(),
        folders: vec![
            SyncFolder {
                label: "Dropbox".into(),
                shown: "~/Library/CloudStorage/Dropbox/PwrRoute".into(),
                root: PathBuf::from("/Users/jo/Library/CloudStorage/Dropbox"),
                chip: "In sync".into(),
                tone: SyncTone::Ok,
                line: "09:41 \u{00b7} saved by MacBook Air".into(),
                conflict: None,
            },
            SyncFolder {
                label: "iCloud Drive".into(),
                shown: "~/Library/Mobile Documents/com~apple~CloudDocs/PwrRoute".into(),
                root: PathBuf::from("/Users/jo/Library/Mobile Documents/com~apple~CloudDocs"),
                chip: "Waiting".into(),
                tone: SyncTone::Neutral,
                line: "Waiting for iCloud to download rules.json".into(),
                conflict: None,
            },
        ],
        offered: vec![SyncPlace {
            label: "Google Drive \u{00b7} jo@example.com".into(),
            shown: "~/Library/CloudStorage/GoogleDrive-jo@example.com/My Drive/PwrRoute".into(),
            root: PathBuf::from(
                "/Users/jo/Library/CloudStorage/GoogleDrive-jo@example.com/My Drive",
            ),
        }],
        config: Some("~/.finicky.ts".into()),
        config_on: true,
        summary: "2 folders".into(),
        summary_tone: SyncTone::Neutral,
        last_synced: "09:41".into(),
        backups: 3,
        destinations: vec![
            row("Work 1", "Google Chrome \u{00b7} Profile 3", true, 4, None),
            row("Work 2", "Google Chrome \u{00b7} Work 2", false, 4, None),
            row("Personal", "Safari \u{00b7} Personal", false, 2, None),
            row(
                "PwrDrvr",
                "Arc",
                false,
                3,
                Some(
                    "Arc isn\u{2019}t installed on this Mac. Links for PwrDrvr open in the fallback browser, Safari, until you choose a browser here.",
                ),
            ),
        ],
        sets: vec![
            RuleSetRow {
                name: "Work 2".into(),
                entries: 4,
                on: true,
                synced_on: true,
                overridden: false,
            },
            RuleSetRow {
                name: "PwrDrvr".into(),
                entries: 3,
                on: false,
                synced_on: true,
                overridden: true,
            },
        ],
        problem: None,
    }
}

/// Synced Rules 1c: Dropbox in conflict with MacBook Air, iCloud in sync.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "for the window's tests and the screens example")
)]
pub fn sync_conflict() -> super::model::Sync {
    use super::model::{SyncConflict, SyncTone};
    let mut sync = sync();
    sync.offered.clear();
    sync.summary = "1 conflict".into();
    sync.summary_tone = SyncTone::Bad;
    sync.last_synced = "09:12".into();
    let dropbox = &mut sync.folders[0];
    dropbox.chip = "Conflict".into();
    dropbox.tone = SyncTone::Bad;
    dropbox.line = "Both changed since the last sync".into();
    dropbox.conflict = Some(SyncConflict {
        file: "rules.json".into(),
        headline: "Links keep routing with this Mac\u{2019}s rules until you choose. Nothing is \
                   written to Dropbox meanwhile."
            .into(),
        mine: "14 rules \u{00b7} this Mac".into(),
        theirs: "15 rules \u{00b7} saved 09:20".into(),
        theirs_name: "MacBook Air".into(),
        differences: vec![
            "+ rule 15 *.example.org/* \u{2192} Work 2".into(),
            "\u{2212} rule 3 tickets.example.net/* \u{2192} Work 1".into(),
        ],
        remote_sha: "0123abcd".into(),
        join: false,
    });
    let icloud = &mut sync.folders[1];
    icloud.chip = "In sync".into();
    icloud.tone = SyncTone::Ok;
    icloud.line = "09:12 \u{00b7} saved by Studio Mac".into();
    sync
}

/// Updates as the design's 1a draws it: a release downloaded and waiting.
pub fn updates() -> Updates {
    let tile = |slot: Slot, release: &str, note: &str, selected: bool, installed: bool| SlotTile {
        slot,
        release: Some(release.into()),
        note: note.into(),
        selected,
        installed,
    };
    Updates {
        version: "0.2.0".into(),
        channel: "Stable \u{00b7} Latest".into(),
        install: "Installed by Velopack; updates apply on a restart.".into(),
        state: UpdateState::Ready("0.2.1".into()),
        last_checked: Some("Today 08:13".into()),
        slots: vec![
            tile(
                Slot::STABLE_LATEST,
                "v0.2.1",
                "Smoke-checked. The default for everyone.",
                true,
                false,
            ),
            tile(
                Slot::STABLE_PRERELEASE,
                "v0.2.2-rc.1",
                "Release candidates for the stable line.",
                false,
                false,
            ),
            tile(
                Slot::BETA_LATEST,
                "v0.3.0-beta.2",
                "Beta builds off main.",
                false,
                false,
            ),
            tile(
                Slot::BETA_PRERELEASE,
                "v0.3.0-alpha.9",
                "Newest alpha off main. May not install.",
                false,
                false,
            ),
        ],
        inferred: true,
    }
}

/// Diagnostics for a contrived machine; nothing here is read from this one.
pub fn diagnostics() -> Diagnostics {
    Diagnostics {
        version: "0.2.0 \u{00b7} debug \u{00b7} aarch64 \u{00b7} 5e1c07a2+".into(),
        os: "macOS 15.6 (arm64)".into(),
        data_dir: "~/Library/Application Support/PwrRoute".into(),
        data_dir_exists: true,
        router: "Listening on ~/Library/Application Support/PwrRoute/router.sock \u{00b7} pid 4242"
            .into(),
        router_listening: true,
        routing: "~/.finicky.ts runs first; rules.json follows it.".into(),
        dry_run: false,
        agents: "Listening on ~/Library/Application Support/PwrRoute/mcp.sock".into(),
        log: "~/Library/Logs/PwrRoute/main.log".into(),
        log_exists: true,
        build: build(),
        yield_browser_focus: Some(false),
    }
}

/// A contrived dev build of 0.2.0: a worktree on a branch, built with
/// uncommitted changes (design: Build Identity 2a).
pub fn build() -> BuildInfo {
    BuildInfo {
        version: "0.2.0".to_owned(),
        dirty: true,
        built_at: Some(1_793_197_920),
        ..BuildInfo::fixture()
    }
}

/// About for the contrived 0.2.0 copy.
pub fn about() -> About {
    About {
        source: build(),
        license: "MIT. Includes code from Finicky, copyright John Sterling, under the MIT \
                  license; see LICENSE."
            .into(),
        links: vec![
            ("Source".into(), "github.com/pwrdrvr/PwrRoute".into()),
            (
                "Releases".into(),
                "github.com/pwrdrvr/PwrRoute/releases".into(),
            ),
            ("Issues".into(), "github.com/pwrdrvr/PwrRoute/issues".into()),
        ],
    }
}

/// Privacy as the design's 1b draws it: hosts only, with files on disk.
/// The sentences are the router's for these values, transcribed; the real
/// pane takes them from `RouterHandle::retention()`.
pub fn privacy() -> Privacy {
    const FILE: &str = "~/Library/Application Support/PwrRoute/journal/activity-YYYY-MM-DD.jsonl";
    Privacy {
        keep_count: 500,
        keep_hours: 72,
        journal: JournalLevel::Hosts,
        journal_days: 7,
        description: format!(
            "Kept in memory for the last 500 links or 72 hours, cleared when PwrRoute quits, \
             and journaled as scheme and host only to {FILE} for 7 days."
        )
        .into(),
        full_description: format!(
            "Kept in memory for the last 500 links or 72 hours, cleared when PwrRoute quits, \
             and journaled with full URLs (except links whose rule keeps less) to {FILE} for \
             7 days."
        )
        .into(),
        journal_path: FILE.into(),
        journal_files: Some(JournalFiles {
            summary: "1.2 MB \u{00b7} 8,114 lines \u{00b7} 6 days".into(),
            since: "2026-10-14".into(),
        }),
        journal_error: None,
        rules_keep_less: Some("Rule 9 keeps host only and rule 10 keeps nothing".into()),
        can_save: true,
    }
}

fn role_id(id: &'static str) -> RoleId {
    RoleId(id.into())
}

fn permissions(grants: &[(&'static str, Grant)]) -> Vec<Permission> {
    grants
        .iter()
        .map(|(tool, grant)| Permission {
            tool: (*tool).into(),
            grant: grant.clone(),
        })
        .collect()
}

pub fn local_agents() -> LocalAgents {
    LocalAgents {
        enabled: true,
        listener: Listener::Listening,
        endpoint: "~/Library/Application Support/PwrRoute/mcp.sock".into(),
        command: "pwrroute mcp".into(),
        sessions: vec![
            AgentSession {
                id: SessionId("s1".into()),
                name: "Claude Code \u{00b7} MacBook Pro".into(),
                detail: "approved 2026-10-02 \u{00b7} 14 calls today".into(),
                role: role_id("opener"),
                last_used: "2 min ago".into(),
                revoked: None,
            },
            AgentSession {
                id: SessionId("s2".into()),
                name: "PwrAgnt \u{00b7} thread tools".into(),
                detail: "approved 2026-10-09 \u{00b7} 3 calls today".into(),
                role: role_id("editor"),
                last_used: "31 min ago".into(),
                revoked: None,
            },
            AgentSession {
                id: SessionId("s3".into()),
                name: "Codex CLI \u{00b7} lab-mini".into(),
                detail: "revoked 2026-10-11".into(),
                role: role_id("reader"),
                last_used: String::new().into(),
                revoked: Some("2026-10-11".into()),
            },
        ],
        roles: vec![
            AgentRole {
                id: role_id("reader"),
                name: "Reader".into(),
                built_in: true,
                summary: "Can ask where a link would go. Sees hosts, not full URLs.".into(),
                permissions: permissions(&[
                    ("resolve", Grant::Full),
                    ("browsers", Grant::Full),
                    ("rules_list", Grant::Full),
                    ("activity", Grant::Partial("hosts only".into())),
                    ("open", Grant::None),
                    ("rules_apply", Grant::None),
                ]),
            },
            AgentRole {
                id: role_id("opener"),
                name: "Opener".into(),
                built_in: true,
                summary: "Reader, plus opening links through the router. Every open is logged \
                          against the session."
                    .into(),
                permissions: permissions(&[
                    ("resolve", Grant::Full),
                    ("browsers", Grant::Full),
                    ("rules_list", Grant::Full),
                    ("activity", Grant::Full),
                    ("open", Grant::Full),
                    ("rules_apply", Grant::None),
                ]),
            },
            AgentRole {
                id: role_id("editor"),
                name: "Editor".into(),
                built_in: true,
                summary: "Opener, plus changing rules.json and reading the activity journal \
                          on disk."
                    .into(),
                permissions: permissions(&[
                    ("resolve", Grant::Full),
                    ("browsers", Grant::Full),
                    ("rules_list", Grant::Full),
                    ("activity", Grant::Full),
                    ("open", Grant::Full),
                    ("rules_apply", Grant::Full),
                ]),
            },
        ],
        selected_role: Some(role_id("opener")),
        recent: vec![
            AgentAction {
                time: "09:40".into(),
                client: "Claude Code".into(),
                call: "pwrroute_open \u{00b7} https://github.com/pwrdrvr/PwrGit/pull/361".into(),
                outcome: "Google Chrome \u{00b7} Work".into(),
                status: Status::Ok,
            },
            AgentAction {
                time: "09:12".into(),
                client: "PwrAgnt".into(),
                call: "pwrroute_rules_apply \u{00b7} added handler 6 (zoom.us/j/*)".into(),
                outcome: "saved \u{00b7} reloaded".into(),
                status: Status::Ok,
            },
            AgentAction {
                time: "08:55".into(),
                client: "Claude Code".into(),
                call: "pwrroute_resolve \u{00b7} https://x.com/\u{2026} \u{00b7} from Slack".into(),
                outcome: "Firefox \u{00b7} dry run".into(),
                status: Status::Ok,
            },
        ],
    }
}

fn text(words: &'static str) -> Piece {
    Piece::Text(words.into())
}

fn tag(pattern: &'static str) -> Piece {
    Piece::Tag(pattern.into())
}

fn candidate(importable: bool, summary: Vec<Piece>) -> ImportCandidate {
    ImportCandidate {
        summary,
        importable,
    }
}

pub fn configuration() -> Configuration {
    Configuration {
        active: RulesSource::Visual,
        can_switch: true,
        visual_summary: "rules.json \u{00b7} 3 rewrites \u{00b7} 7 rules".into(),
        finicky: Some(FinickyConfig {
            path: "~/.finicky.ts".into(),
            summary: "~/.finicky.ts \u{00b7} 2 rewrites \u{00b7} 6 handlers".into(),
            found: "Found in the home folder. Runs as written, with its imports watched for \
                    changes."
                .into(),
        }),
        import: Some(ImportPlan {
            candidates: vec![
                candidate(
                    true,
                    vec![
                        text("rewrite \u{00b7}"),
                        tag("x.com/*"),
                        text("\u{2192}"),
                        Piece::Action("host xcancel.com".into()),
                    ],
                ),
                candidate(
                    true,
                    vec![
                        text("rewrite \u{00b7}"),
                        tag("*"),
                        text("\u{2192}"),
                        Piece::Action("drop utm_*".into()),
                        Piece::Aside("proved equal to".into()),
                        Piece::Code("url.searchParams.delete(\u{2026})".into()),
                    ],
                ),
                candidate(
                    true,
                    vec![
                        text("handler \u{00b7}"),
                        tag("bsky.app/*"),
                        text("\u{2192} Firefox"),
                    ],
                ),
                candidate(
                    true,
                    vec![
                        text("handler \u{00b7}"),
                        tag("google.com/*"),
                        tag("*.google.com*"),
                        text("\u{2192} Google Chrome"),
                    ],
                ),
                candidate(
                    false,
                    vec![Piece::Source(
                        "(url, { opener }) => url.host.includes(\"workplace\") \u{2192} Firefox"
                            .into(),
                    )],
                ),
                candidate(
                    false,
                    vec![
                        Piece::Source(
                            "(url, { opener }) => opener?.name.includes(\"Slack\") \u{2192} \
                             Firefox"
                                .into(),
                        ),
                        Piece::Aside("could be \u{201c}sent from Slack\u{201d}; not proved".into()),
                    ],
                ),
                candidate(true, vec![text("defaultBrowser \u{2192} Google Chrome")]),
            ],
        }),
        shorteners: ["t.co", "bit.ly", "tinyurl.com", "lnkd.in", "buff.ly"]
            .into_iter()
            .map(Into::into)
            .collect(),
        rules_path: "~/Library/Application Support/PwrRoute/rules.json".into(),
        rules_status: "Loaded \u{00b7} edited 12 min ago.".into(),
        everything_else: None,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "a fixture table row; each argument is one column"
)]
fn app(
    id: &'static str,
    name: &'static str,
    badge: &'static str,
    location: &'static str,
    profiles: &[&'static str],
    note: Option<&'static str>,
    usage: Usage,
    visible: bool,
) -> BrowserApp {
    BrowserApp {
        id: id.into(),
        name: name.into(),
        badge: AppBadge::new(badge, id),
        location: location.into(),
        profiles: profiles.iter().map(|profile| (*profile).into()).collect(),
        note: note.map(Into::into),
        usage,
        visible,
    }
}

fn in_rules(count: usize) -> Usage {
    Usage::Rules {
        count,
        all_off: false,
    }
}

pub fn browsers() -> Browsers {
    Browsers {
        default: DefaultBrowser {
            state: DefaultState::PwrRoute,
            note: Some("Confirmed 2026-10-13 09:02.".into()),
        },
        detected: vec![
            app(
                "com.google.Chrome",
                "Google Chrome",
                "Ch",
                "com.google.Chrome \u{00b7} /Applications/Google Chrome.app",
                &["Personal", "Work"],
                None,
                in_rules(4),
                true,
            ),
            app(
                "org.mozilla.firefox",
                "Firefox",
                "Ff",
                "org.mozilla.firefox",
                &["default-release", "dev-edition"],
                None,
                in_rules(2),
                true,
            ),
            app(
                "company.thebrowser.Browser",
                "Arc",
                "Ar",
                "company.thebrowser.Browser",
                &[],
                Some("no profiles"),
                in_rules(1),
                true,
            ),
            app(
                "com.apple.Safari",
                "Safari",
                "Sa",
                "com.apple.Safari",
                &["Personal", "Work"],
                None,
                Usage::Unused,
                true,
            ),
            app(
                "com.brave.Browser.nightly",
                "Brave Browser Nightly",
                "Bn",
                "com.brave.Browser.nightly",
                &["Default"],
                None,
                Usage::Unused,
                false,
            ),
        ],
        other_apps: vec![
            app(
                "us.zoom.xos",
                "zoom.us",
                "Zm",
                "us.zoom.xos",
                &[],
                Some("opens meeting links natively"),
                Usage::NotInstalled,
                true,
            ),
            app(
                "com.figma.Desktop",
                "Figma",
                "Fg",
                "com.figma.Desktop",
                &[],
                None,
                Usage::Rules {
                    count: 1,
                    all_off: true,
                },
                true,
            ),
        ],
    }
}

// Settings Everything else (WP-F)

use super::model::{DefaultChoice, DefaultOwner, EverythingElse};
use crate::widgets::AppBadge;

/// Overview and Setup 5d: `~/.finicky.ts` in charge, and PwrRoute's
/// default, Google Chrome · Personal, takes everything else.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "for the window's tests and the screens example")
)]
pub fn configuration_everything_else() -> Configuration {
    let chrome = DefaultChoice {
        value: "com.google.Chrome".into(),
        name: "Google Chrome".into(),
        badge: AppBadge::new("Ch", "com.google.Chrome"),
        profiles: vec!["Personal".into(), "Work".into()],
    };
    let safari = DefaultChoice {
        value: "com.apple.Safari".into(),
        name: "Safari".into(),
        badge: AppBadge::new("Sf", "com.apple.Safari"),
        profiles: Vec::new(),
    };
    let firefox = DefaultChoice {
        value: "org.mozilla.firefox".into(),
        name: "Firefox".into(),
        badge: AppBadge::new("Ff", "org.mozilla.firefox"),
        profiles: vec!["default-release".into()],
    };
    Configuration {
        active: RulesSource::Finicky,
        can_switch: true,
        visual_summary: "rules.json \u{00b7} 1 rewrite \u{00b7} 3 rules".into(),
        finicky: Some(FinickyConfig {
            path: "~/.finicky.ts".into(),
            summary: "~/.finicky.ts \u{00b7} 5 handlers".into(),
            found: "Found at ~/.finicky.ts; its rewrites run first, then your rules, then its \
                    handlers. Edited 4 min ago."
                .into(),
        }),
        import: None,
        everything_else: Some(EverythingElse {
            owner: DefaultOwner::PwrRoute,
            config_label: "~/.finicky.ts's default \u{00b7} Safari".into(),
            config_detail: "~/.finicky.ts's defaultBrowser handles links its handlers don't \
                            claim. The browser below still catches config errors, browsers \
                            that can't open, and every link while paused."
                .into(),
            pwrroute_detail: "Your rules run first, then ~/.finicky.ts's handlers. Links \
                              nothing claims go here instead of Safari."
                .into(),
            warnings: Vec::new(),
            browsers: vec![chrome, safari, firefox],
            browser: Some(0),
            profile: Some("Personal".into()),
            problem: None,
        }),
        ..configuration()
    }
}

/// [`configuration_everything_else`] over a config whose default is a
/// function and whose handler 5 matches every link (design 5c's warnings).
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "for the window's tests and the screens example")
)]
pub fn configuration_everything_else_warnings() -> Configuration {
    let mut configuration = configuration_everything_else();
    if let Some(field) = configuration.everything_else.as_mut() {
        field.config_label = "~/.finicky.ts's default".into();
        field.pwrroute_detail =
            "Your rules run first, then ~/.finicky.ts's handlers. Links nothing claims go here."
                .into();
        field.warnings = vec![
            "Its defaultBrowser function will no longer run.".into(),
            "Config handler 5 matches every link, so PwrRoute's default never applies.".into(),
        ];
    }
    configuration
}

/// [`configuration_everything_else`] after a change that was not saved:
/// the error line under the selects.
#[cfg_attr(not(test), allow(dead_code, reason = "for the screens example"))]
pub fn configuration_everything_else_not_saved() -> Configuration {
    let mut configuration = configuration_everything_else();
    if let Some(field) = configuration.everything_else.as_mut() {
        field.problem =
            Some("Not saved: rules.json did not load, so it is not overwritten.".into());
    }
    configuration
}
