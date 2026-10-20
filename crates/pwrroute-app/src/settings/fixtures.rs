//! Contrived Settings data, transcribed from the design's three panes
//! (`design/PwrRoute Settings.dc.html`). Nothing here is read from this
//! machine.

use super::model::{
    AgentAction, AgentRole, AgentSession, BrowserApp, Browsers, Configuration, DefaultBrowser,
    DefaultState, FinickyConfig, Grant, ImportCandidate, ImportPlan, Listener, LocalAgents,
    Permission, Piece, RoleId, RulesSource, SessionId, SettingsModel, Status, Usage,
};

pub fn model() -> SettingsModel {
    SettingsModel {
        local_agents: local_agents(),
        configuration: configuration(),
        browsers: browsers(),
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
        endpoint: "http://127.0.0.1:51731/mcp".into(),
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
                summary: "Opener, plus writing rules.json. Refused while a Finicky config is \
                          selected."
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
        checked: importable,
    }
}

pub fn configuration() -> Configuration {
    Configuration {
        active: RulesSource::Visual,
        visual_summary: "rules.json \u{00b7} 3 rewrites \u{00b7} 7 handlers".into(),
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
        expand_short_links: true,
        shorteners: ["t.co", "bit.ly", "tinyurl.com", "lnkd.in", "buff.ly"]
            .into_iter()
            .map(Into::into)
            .collect(),
        rules_path: "~/Library/Application Support/PwrRoute/rules.json".into(),
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
        badge: badge.into(),
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
