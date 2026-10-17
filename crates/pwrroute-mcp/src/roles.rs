//! Permissions, the roles that bundle them, and the OAuth scopes that name
//! them (the brief's 2d matrix, Settings 1a).
//!
//! As in PwrSnap and PwrGit, a scope is one permission, and a role is a named
//! set of permissions. A client asks for scopes, the person picks a role in
//! the approval window, and the session can do what the role allows within
//! what was asked: `role ∩ consented scopes`. Changing a session's role later
//! can narrow it but never widen it past that consent.

use std::{collections::BTreeSet, fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// One thing a session may be allowed to do. The serialized form is the
/// OAuth scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// `pwrroute_resolve`.
    #[serde(rename = "links.resolve")]
    Resolve,
    /// `pwrroute_browsers`.
    #[serde(rename = "browsers.read")]
    Browsers,
    /// `pwrroute_rules_list`.
    #[serde(rename = "rules.read")]
    RulesRead,
    /// `pwrroute_activity`, with every URL cut to its host.
    #[serde(rename = "activity.hosts")]
    ActivityHosts,
    /// `pwrroute_activity` with full URLs.
    #[serde(rename = "activity.urls")]
    ActivityUrls,
    /// `pwrroute_open`.
    #[serde(rename = "links.open")]
    Open,
    /// `pwrroute_rules_apply`.
    #[serde(rename = "rules.write")]
    RulesWrite,
}

impl Permission {
    pub const ALL: [Self; 7] = [
        Self::Resolve,
        Self::Browsers,
        Self::RulesRead,
        Self::ActivityHosts,
        Self::ActivityUrls,
        Self::Open,
        Self::RulesWrite,
    ];

    /// The scopes a client gets when it asks for none: what a Reader needs.
    pub const DEFAULT_SCOPES: [Self; 4] = [
        Self::Resolve,
        Self::Browsers,
        Self::RulesRead,
        Self::ActivityHosts,
    ];

    #[must_use]
    pub fn scope(self) -> &'static str {
        match self {
            Self::Resolve => "links.resolve",
            Self::Browsers => "browsers.read",
            Self::RulesRead => "rules.read",
            Self::ActivityHosts => "activity.hosts",
            Self::ActivityUrls => "activity.urls",
            Self::Open => "links.open",
            Self::RulesWrite => "rules.write",
        }
    }

    #[must_use]
    pub fn from_scope(scope: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.scope() == scope)
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
            Self::Open => "Open links through PwrRoute",
            Self::RulesWrite => "Change rules.json",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.scope())
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
    /// Opener, plus writing `rules.json`.
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
            set.insert(P::RulesWrite);
        }
        set
    }

    #[must_use]
    pub fn allows(self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }

    /// Roles a client that asked for `scopes` can be given: those that ask
    /// for nothing beyond them.
    #[must_use]
    pub fn offered_for(scopes: &Permissions) -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(|r| r.permissions().is_subset(scopes))
            .collect()
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
    use super::{Permission as P, Permissions, Role};

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
        ];
        for (permission, expected) in rows {
            for (role, allowed) in Role::ALL.into_iter().zip(expected) {
                assert_eq!(role.allows(permission), allowed, "{role} {permission}");
            }
        }
    }

    #[test]
    fn scopes_round_trip() {
        for p in P::ALL {
            assert_eq!(P::from_scope(p.scope()), Some(p));
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::json!(p.scope())
            );
        }
    }

    #[test]
    fn only_roles_within_the_request_are_offered() {
        let reader = Permissions::from(P::DEFAULT_SCOPES);
        assert_eq!(Role::offered_for(&reader), [Role::Reader]);
        let all = Permissions::from(P::ALL);
        assert_eq!(Role::offered_for(&all), Role::ALL);
        assert!(Role::offered_for(&Permissions::from([P::Open])).is_empty());
    }
}
