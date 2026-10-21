//! Permissions and the roles that bundle them (the brief's 2d matrix,
//! Settings 1a). Built-in roles are additive: Opener is Reader plus opening
//! links, Editor is Opener plus writing rules.

use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// One thing an agent may be allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// `pwrroute_resolve`.
    #[serde(rename = "links.resolve")]
    Resolve,
    /// `pwrroute_browsers`.
    #[serde(rename = "browsers.read")]
    Browsers,
    /// `pwrroute_rules_list`, `pwrroute_rules_test` and
    /// `pwrroute_suggest_rule`.
    #[serde(rename = "rules.read")]
    RulesRead,
    /// `pwrroute_activity`, with every URL cut to its host.
    #[serde(rename = "activity.hosts")]
    ActivityHosts,
    /// `pwrroute_activity` with full URLs.
    #[serde(rename = "activity.urls")]
    ActivityUrls,
    /// `pwrroute_activity` over the on-disk journal (`source: "journal"`),
    /// when the user turned it on.
    #[serde(rename = "activity.journal")]
    ActivityJournal,
    /// `pwrroute_open`.
    #[serde(rename = "links.open")]
    Open,
    /// `pwrroute_rules_apply`.
    #[serde(rename = "rules.write")]
    RulesWrite,
}

impl Permission {
    pub const ALL: [Self; 8] = [
        Self::Resolve,
        Self::Browsers,
        Self::RulesRead,
        Self::ActivityHosts,
        Self::ActivityUrls,
        Self::Open,
        Self::RulesWrite,
        Self::ActivityJournal,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Resolve => "links.resolve",
            Self::Browsers => "browsers.read",
            Self::RulesRead => "rules.read",
            Self::ActivityHosts => "activity.hosts",
            Self::ActivityUrls => "activity.urls",
            Self::ActivityJournal => "activity.journal",
            Self::Open => "links.open",
            Self::RulesWrite => "rules.write",
        }
    }

    /// How the approval window names it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Resolve => "Ask where a link would go",
            Self::Browsers => "List browsers and profiles",
            Self::RulesRead => "Read the rules",
            Self::ActivityHosts => "See recent activity, hosts only",
            Self::ActivityUrls => "See recent activity with full URLs",
            Self::ActivityJournal => "Read the activity journal on disk, when it is on",
            Self::Open => "Open links through PwrRoute",
            Self::RulesWrite => "Change rules.json",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of permissions.
pub type Permissions = BTreeSet<Permission>;

/// The built-in roles. Each is the one before it plus more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Can ask where a link would go. Sees hosts, not full URLs.
    Reader,
    /// Reader, plus opening links through the router.
    Opener,
    /// Opener, plus writing `rules.json` and reading the activity journal.
    Editor,
}

impl Role {
    pub const ALL: [Self; 3] = [Self::Reader, Self::Opener, Self::Editor];

    #[must_use]
    pub fn permissions(self) -> Permissions {
        use Permission as P;
        let mut set = Permissions::from([P::Resolve, P::Browsers, P::RulesRead, P::ActivityHosts]);
        if self >= Self::Opener {
            set.extend([P::ActivityUrls, P::Open]);
        }
        if self >= Self::Editor {
            set.extend([P::RulesWrite, P::ActivityJournal]);
        }
        set
    }

    #[must_use]
    pub fn allows(self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }

    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Reader => "Can ask where a link would go. Sees hosts, not full URLs.",
            Self::Opener => {
                "Reader, plus opening links through the router. Every open is logged against the session."
            }
            Self::Editor => {
                "Opener, plus writing rules.json. Refused while a Finicky config is selected."
            }
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reader => "Reader",
            Self::Opener => "Opener",
            Self::Editor => "Editor",
        })
    }
}

impl FromStr for Role {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|r| r.to_string().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unknown role `{s}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Permission as P, Role};

    #[test]
    fn roles_match_the_brief() {
        let rows = [
            (P::Resolve, [true, true, true]),
            (P::Browsers, [true, true, true]),
            (P::RulesRead, [true, true, true]),
            (P::ActivityHosts, [true, true, true]),
            (P::ActivityUrls, [false, true, true]),
            (P::Open, [false, true, true]),
            (P::RulesWrite, [false, false, true]),
            (P::ActivityJournal, [false, false, true]),
        ];
        for (permission, expected) in rows {
            for (role, allowed) in Role::ALL.into_iter().zip(expected) {
                assert_eq!(role.allows(permission), allowed, "{role} {permission}");
            }
        }
    }

    #[test]
    fn names_are_the_serialized_form() {
        for p in P::ALL {
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::json!(p.name())
            );
        }
    }
}
