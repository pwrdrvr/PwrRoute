//! Visual rules: `rules.json`, the model the app's rules editor reads and
//! writes.
//!
//! The file is PwrFinicky's `rules.json` (its Go host's `rules.RulesFile`),
//! extended with fields that file never had. Every extension is optional and
//! its default is what the Go host did, so a Go `rules.json` loads unchanged
//! and resolves the same way, and a file that uses no extension is one the Go
//! host reads too.
//!
//! | Field | Go host | Meaning |
//! | --- | --- | --- |
//! | `defaultBrowser`, `defaultProfile` | yes | Where everything else goes. Used only when no JS config is in charge. |
//! | `options` | yes | Finicky's `options`, used only when no JS config is in charge. |
//! | `rules[].match`, `.browser`, `.profile` | yes | Handlers: the first match wins. `match` is a string or a list. |
//! | `rules[].opener` | no | Also require the link to come from one of these apps (name or bundle id). |
//! | `rules[].enabled` | no | `false` switches a rule off but keeps its place. |
//! | `rewrites` | no | Rewrites: every match applies, in order, before the handlers. |
//!
//! With a JS config, the config's rewrites and handlers run first and it owns
//! the default browser and options; these rules are appended after its own,
//! as the Go host appended JSON handlers. Without one, these rules are the
//! whole config.
//!
//! Match patterns are Finicky's: wildcard strings (`*.example.com/*`), matched
//! by Finicky's own `isMatch`. A regex or function matcher is JS-only.

use std::{fs, io, path::Path};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

/// The default browser the Go host used for a `rules.json` that names none.
pub const FALLBACK_BROWSER: &str = "com.apple.Safari";

/// The contents of `rules.json`. Field order is the file's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesFile {
    /// A browser name, bundle id or path. Empty means [`FALLBACK_BROWSER`].
    #[serde(default)]
    pub default_browser: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<RulesOptions>,
    /// PwrRoute extension: rewrites, applied in order before any handler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewrites: Vec<Rewrite>,
    /// Handlers, first match wins. The Go host calls them rules.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub rules: Vec<Rule>,
}

/// Finicky's `options`, as the Go host's `rules.json` carried them. Each is
/// left out of the config when unset, so Finicky's own default applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_icon: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_requests: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_for_updates: Option<bool>,
}

/// A handler: links matching any pattern open in `browser`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// Wildcard patterns; any one matching is a match. Written as a plain
    /// string when there is exactly one, as the Go host wrote it.
    #[serde(
        rename = "match",
        default,
        serialize_with = "one_or_many::serialize",
        deserialize_with = "one_or_many::deserialize"
    )]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub browser: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    /// PwrRoute extension: when not empty, the link must also come from one
    /// of these apps, compared with the opener's name and bundle id, ignoring
    /// case.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opener: Vec<String>,
    /// PwrRoute extension: a disabled rule is skipped but keeps its place.
    #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            browser: String::new(),
            profile: String::new(),
            opener: Vec::new(),
            enabled: true,
        }
    }
}

/// A rewrite: links matching any pattern have `actions` applied, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rewrite {
    #[serde(
        rename = "match",
        default,
        serialize_with = "one_or_many::serialize",
        deserialize_with = "one_or_many::deserialize"
    )]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub actions: Vec<RewriteAction>,
    #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
}

impl Default for Rewrite {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            actions: Vec::new(),
            enabled: true,
        }
    }
}

/// One change a [`Rewrite`] makes. These are the visual rewrites the design
/// offers; anything else is a JS `url` function in a config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RewriteAction {
    /// Replace the whole URL, as a Finicky string rewrite does.
    Url(String),
    /// Set the scheme, without the colon: `"https"`.
    SetScheme(String),
    /// Set the host, with an optional port: `"xcancel.com"`.
    SetHost(String),
    /// Set the path: `"/feed"`.
    ReplacePath(String),
    /// Remove query parameters whose name matches; `*` matches any run of
    /// characters (`"utm_*"`).
    DropQuery(Vec<String>),
}

/// Where a [`Problem`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "index")]
pub enum Place {
    DefaultBrowser,
    Rule(usize),
    Rewrite(usize),
}

/// Something the editor should not save, with the reason in words for the
/// user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub place: Place,
    pub message: String,
}

impl RulesFile {
    /// Reads `rules.json`. A missing file is an empty one, as the Go host
    /// treated it.
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("{}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("{}", path.display())),
        }
    }

    /// Writes `rules.json` with two-space indentation, creating its directory.
    /// The write goes to a sibling file first and is renamed into place, so a
    /// watcher never reads half a file.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("{}", dir.display()))?;
        }
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        let mut staging = path.as_os_str().to_owned();
        staging.push(".saving");
        fs::write(&staging, text).with_context(|| format!("{}", path.display()))?;
        fs::rename(&staging, path).with_context(|| format!("{}", path.display()))
    }

    /// Whether the file says anything at all. The Go host used a `rules.json`
    /// on its own only when it named a default browser or had rules.
    pub fn is_empty(&self) -> bool {
        self.default_browser.is_empty() && self.rules.is_empty() && self.rewrites.is_empty()
    }

    /// What the editor should flag before saving. The engine itself loads a
    /// file with problems the way the Go host did: incomplete rules are
    /// skipped and an empty default is [`FALLBACK_BROWSER`].
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        let mut flag = |place, message: &str| {
            problems.push(Problem {
                place,
                message: message.to_owned(),
            });
        };
        if self.default_browser.trim().is_empty() {
            flag(Place::DefaultBrowser, "Select a default browser.");
        } else if is_self(&self.default_browser) {
            flag(Place::DefaultBrowser, "PwrRoute cannot route to itself.");
        }
        for (i, rule) in self.rules.iter().enumerate() {
            let place = Place::Rule(i);
            if rule.browser.trim().is_empty() {
                flag(place, "Select a browser.");
            } else if is_self(&rule.browser) {
                flag(place, "Select a browser other than PwrRoute.");
            }
            check_patterns(&rule.patterns, place, &mut flag);
            if rule.opener.iter().any(|o| o.trim().is_empty()) {
                flag(place, "App names cannot be empty.");
            }
        }
        for (i, rewrite) in self.rewrites.iter().enumerate() {
            let place = Place::Rewrite(i);
            check_patterns(&rewrite.patterns, place, &mut flag);
            if rewrite.actions.is_empty() {
                flag(place, "Add at least one change.");
            }
            for action in &rewrite.actions {
                if let Some(message) = action.problem() {
                    flag(place, message);
                }
            }
        }
        problems
    }

    /// The rules as Finicky config objects, the shape the host's JS glue
    /// turns into `handlers` and `rewrite` entries. Handlers follow the Go
    /// host's `ToJSHandlers`: empty patterns are dropped, and a rule with no
    /// pattern or no browser is skipped. Disabled rules are skipped too.
    pub(crate) fn plan(&self) -> Value {
        let handlers: Vec<Value> = self
            .planned_rules()
            .into_iter()
            .filter_map(|i| {
                let r = &self.rules[i];
                let matches = non_empty(&r.patterns)?;
                let browser = if r.profile.is_empty() {
                    json!(r.browser)
                } else {
                    json!({ "name": r.browser, "profile": r.profile })
                };
                let mut handler = json!({ "match": matches, "browser": browser });
                if !r.opener.is_empty() {
                    handler["opener"] = json!(r.opener);
                }
                Some(handler)
            })
            .collect();
        let rewrites: Vec<Value> = self
            .planned_rewrites()
            .into_iter()
            .filter_map(|i| {
                let r = &self.rewrites[i];
                let matches = non_empty(&r.patterns)?;
                Some(json!({ "match": matches, "actions": r.actions }))
            })
            .collect();
        json!({ "handlers": handlers, "rewrites": rewrites })
    }

    /// Which of `rules` the plan keeps, by position in the file: enabled,
    /// with a pattern and a browser. The engine reports a rule by its plan
    /// position; this maps it back.
    pub(crate) fn planned_rules(&self) -> Vec<usize> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, r)| r.enabled && !r.browser.is_empty() && non_empty(&r.patterns).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// Which of `rewrites` the plan keeps: enabled, with a pattern and an
    /// action.
    pub(crate) fn planned_rewrites(&self) -> Vec<usize> {
        self.rewrites
            .iter()
            .enumerate()
            .filter(|(_, r)| r.enabled && !r.actions.is_empty() && non_empty(&r.patterns).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// The config a `rules.json` makes on its own, as the Go host's
    /// `ToJSConfigScript` built it (without the handlers and rewrites, which
    /// [`RulesFile::plan`] supplies).
    pub(crate) fn base_config(&self) -> Value {
        let name = if self.default_browser.is_empty() {
            FALLBACK_BROWSER
        } else {
            &self.default_browser
        };
        let default_browser = if self.default_profile.is_empty() {
            json!(name)
        } else {
            json!({ "name": name, "profile": self.default_profile })
        };
        let mut config = json!({ "defaultBrowser": default_browser });
        if let Some(options) = &self.options {
            config["options"] = serde_json::to_value(options).unwrap_or_default();
        }
        config
    }
}

impl RewriteAction {
    fn problem(&self) -> Option<&'static str> {
        match self {
            Self::Url(url) if url::Url::parse(url).is_err() => Some("Enter a full URL."),
            Self::SetScheme(s)
                if s.is_empty()
                    || !s.starts_with(|c: char| c.is_ascii_alphabetic())
                    || !s
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) =>
            {
                Some("Enter a scheme such as https.")
            }
            Self::SetHost(h) if h.trim().is_empty() || h.contains(['/', '?', '#', ' ']) => {
                Some("Enter a host such as example.com.")
            }
            Self::DropQuery(keys) if keys.is_empty() || keys.iter().any(|k| k.is_empty()) => {
                Some("Name the query parameters to drop.")
            }
            _ => None,
        }
    }
}

fn check_patterns(patterns: &[String], place: Place, flag: &mut impl FnMut(Place, &str)) {
    if patterns.is_empty() {
        flag(place, "Add at least one match pattern.");
    } else if patterns.iter().any(|p| p.trim().is_empty()) {
        flag(place, "Match patterns cannot be empty.");
    }
}

/// PwrRoute as a destination would hand the link straight back.
fn is_self(browser: &str) -> bool {
    let name = browser.split(':').next().unwrap_or(browser).trim();
    name.eq_ignore_ascii_case("PwrRoute") || name.eq_ignore_ascii_case("com.pwrdrvr.pwrroute")
}

/// The non-empty patterns, as one string when there is one (Go's shape).
fn non_empty(patterns: &[String]) -> Option<Value> {
    let kept: Vec<&String> = patterns.iter().filter(|p| !p.is_empty()).collect();
    match kept.as_slice() {
        [] => None,
        [one] => Some(json!(one)),
        many => Some(json!(many)),
    }
}

fn enabled() -> bool {
    true
}

fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

fn null_as_empty<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

/// `match` as the Go host reads and writes it: a string or a list of strings.
mod one_or_many {
    use super::*;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        Ok(match Option::<OneOrMany>::deserialize(d)? {
            None => Vec::new(),
            Some(OneOrMany::One(s)) => vec![s],
            Some(OneOrMany::Many(v)) => v,
        })
    }

    pub(super) fn serialize<S: Serializer>(v: &[String], s: S) -> Result<S::Ok, S::Error> {
        match v {
            [one] => one.serialize(s),
            many => many.serialize(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_go_host_schema() {
        let file: RulesFile = serde_json::from_str(
            r#"{
              "defaultBrowser": "Safari",
              "defaultProfile": "",
              "options": { "keepRunning": true },
              "rules": [
                { "match": "github.com/*", "browser": "Firefox" },
                { "match": ["a.example/*", "b.example/*"], "browser": "Google Chrome", "profile": "Work" },
                { "match": null, "browser": "Arc" }
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(file.rules.len(), 3);
        assert_eq!(file.rules[0].patterns, ["github.com/*"]);
        assert!(file.rules[1].enabled);
        assert!(file.rules[2].patterns.is_empty());
        assert_eq!(file.options.unwrap().keep_running, Some(true));
    }

    #[test]
    fn null_rules_are_empty() {
        let file: RulesFile =
            serde_json::from_str(r#"{"defaultBrowser": "Safari", "rules": null}"#).unwrap();
        assert!(file.rules.is_empty());
    }

    #[test]
    fn writes_what_the_go_host_writes_when_no_extension_is_used() {
        let file = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![
                Rule {
                    patterns: vec!["github.com/*".into()],
                    browser: "Firefox".into(),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["a/*".into(), "b/*".into()],
                    browser: "Arc".into(),
                    profile: "Work".into(),
                    ..Rule::default()
                },
            ],
            ..RulesFile::default()
        };
        assert_eq!(
            serde_json::to_value(&file).unwrap(),
            json!({
                "defaultBrowser": "Safari",
                "rules": [
                    { "match": "github.com/*", "browser": "Firefox" },
                    { "match": ["a/*", "b/*"], "browser": "Arc", "profile": "Work" }
                ]
            })
        );
    }

    #[test]
    fn extensions_round_trip() {
        let text = r#"{
          "defaultBrowser": "Firefox",
          "rewrites": [
            { "match": "x.com/*", "actions": [{ "setHost": "xcancel.com" }, { "dropQuery": ["utm_*"] }] },
            { "match": "old.example/*", "actions": [{ "url": "https://new.example/" }], "enabled": false }
          ],
          "rules": [
            { "match": "*.atlassian.net/*", "browser": "Google Chrome", "profile": "Work", "opener": ["Slack"] },
            { "match": "zoom.us/*", "browser": "us.zoom.xos", "enabled": false }
          ]
        }"#;
        let file: RulesFile = serde_json::from_str(text).unwrap();
        assert_eq!(
            file.rewrites[0].actions,
            [
                RewriteAction::SetHost("xcancel.com".into()),
                RewriteAction::DropQuery(vec!["utm_*".into()])
            ]
        );
        assert!(!file.rules[1].enabled);
        let again: RulesFile =
            serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(again, file);
    }

    #[test]
    fn plan_follows_go_to_js_handlers() {
        let file = RulesFile {
            rules: vec![
                Rule {
                    patterns: vec!["".into(), "a/*".into()],
                    browser: "Firefox".into(),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["".into()],
                    browser: "Firefox".into(),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["b/*".into()],
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["c/*".into(), "d/*".into()],
                    browser: "Arc".into(),
                    profile: "Work".into(),
                    ..Rule::default()
                },
                Rule {
                    patterns: vec!["e/*".into()],
                    browser: "Arc".into(),
                    enabled: false,
                    ..Rule::default()
                },
            ],
            ..RulesFile::default()
        };
        assert_eq!(
            file.plan()["handlers"],
            json!([
                { "match": "a/*", "browser": "Firefox" },
                { "match": ["c/*", "d/*"], "browser": { "name": "Arc", "profile": "Work" } }
            ])
        );
    }

    #[test]
    fn base_config_follows_go_to_js_config_script() {
        assert_eq!(
            RulesFile::default().base_config(),
            json!({ "defaultBrowser": "com.apple.Safari" })
        );
        let file = RulesFile {
            default_browser: "Firefox".into(),
            default_profile: "Work".into(),
            options: Some(RulesOptions {
                log_requests: Some(false),
                ..RulesOptions::default()
            }),
            ..RulesFile::default()
        };
        assert_eq!(
            file.base_config(),
            json!({
                "defaultBrowser": { "name": "Firefox", "profile": "Work" },
                "options": { "logRequests": false }
            })
        );
    }

    #[test]
    fn problems_name_the_place() {
        let file = RulesFile {
            default_browser: "PwrRoute".into(),
            rules: vec![Rule {
                patterns: vec![" ".into()],
                browser: "Firefox".into(),
                ..Rule::default()
            }],
            rewrites: vec![Rewrite {
                patterns: vec!["x/*".into()],
                actions: vec![RewriteAction::SetScheme("ht tp".into())],
                ..Rewrite::default()
            }],
            ..RulesFile::default()
        };
        let places: Vec<Place> = file.problems().iter().map(|p| p.place).collect();
        assert_eq!(
            places,
            [Place::DefaultBrowser, Place::Rule(0), Place::Rewrite(0)]
        );
    }

    #[test]
    fn a_missing_file_is_empty_and_save_round_trips() {
        let dir = std::env::temp_dir().join(format!("pwrroute-rules-{}", std::process::id()));
        let path = dir.join("nested/rules.json");
        assert!(RulesFile::load(&path).unwrap().is_empty());
        let file = RulesFile {
            default_browser: "Firefox".into(),
            ..RulesFile::default()
        };
        file.save(&path).unwrap();
        assert_eq!(RulesFile::load(&path).unwrap(), file);
        fs::remove_dir_all(dir).unwrap();
    }
}
