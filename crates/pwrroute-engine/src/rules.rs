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
//! | `defaultBrowser`, `defaultProfile` | yes | Where everything else goes. Used only when no JS config is in charge, or when `precedence` is `pwrroute`. |
//! | `precedence` | no | Who goes first while a JS config is in charge ([`Precedence`]). Absent (or `config`, or any word but `pwrroute`): the Go host's order, the config's handlers, then these rules, then the config's `defaultBrowser`. `pwrroute`: these rules, then the config's handlers, then `defaultBrowser`/`defaultProfile` above instead of the config's. Rewrites run config first either way. Without a config it changes nothing. |
//! | `options` | yes | Finicky's `options`, used only when no JS config is in charge. |
//! | `rules[].match`, `.browser`, `.profile` | yes | Handlers: the first match wins. `match` is a string or a list. `browser` is an app name (`Google Chrome`), a bundle id (`com.google.Chrome`) or a `.app` path (`/Applications/Google Chrome.app`, `~/Applications/…`), told apart as Finicky tells them apart (`app_type`); `defaultBrowser` takes the same. |
//! | `rules[].opener` | no | Also require the link to come from one of these apps (name or bundle id). |
//! | `rules[].enabled` | no | `false` switches a rule off but keeps its place. |
//! | `rewrites` | no | Rewrites: every match applies, in order, before the handlers. |
//! | `rules[].privacy`, `rewrites[].privacy` | no | What the router may keep of a link this entry routed: `full` (default) the URL, `host` its host only, `none` nothing. The tightest level of every rewrite that matched and the handler that won applies. An unknown word is kept as written, flagged by [`RulesFile::problems`], and treated as `none`. |
//! | `rules[].private` | no | `true` opens the link in a private (incognito) window. When `privacy` is absent, it then defaults to `host`. |
//! | `rules[].examples` | no | URLs that must resolve to this rule. Run by [`RulesFile::run_tests`]; a new rule cannot take them over unnoticed. |
//! | `rewrites[].examples` | no | `{ "url", "becomes" }` pairs: this rewrite must match `url` and turn it into `becomes`. |
//! | `tests` | no | `{ "url", "opener"?, "expect": { "browser", "profile"?, "url"? } }`: the whole config, JS included, must send `url` there. `opener` is an app name or bundle id (`Name:bundle.id` sets both). |
//!
//! With a JS config, the config's rewrites and handlers run first and it owns
//! the default browser and options; these rules are appended after its own,
//! as the Go host appended JSON handlers. `"precedence": "pwrroute"` (opt-in,
//! [`Precedence::PwrRoute`]) moves these handlers ahead of the config's and
//! gives this file's default browser everything else. Without a config,
//! these rules are the whole config.
//!
//! Match patterns are Finicky's: wildcard strings (`*.example.com/*`), matched
//! by Finicky's own `isMatch`. A regex or function matcher is JS-only.
//!
//! Examples and tests are URLs the user chose to save with the rules. They
//! are the one place a URL goes to disk: nothing else in the engine writes
//! one, and the router keeps the links it routes in memory only.

use std::{fs, io, path::Path};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::Privacy;

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
    /// PwrRoute extension: whether these rules and this default browser go
    /// before a JS config's handlers and default. Absent is
    /// [`Precedence::Config`], the Go host's order, and is never written, so
    /// a Go file round-trips unchanged.
    #[serde(default, skip_serializing_if = "Precedence::is_config")]
    pub precedence: Precedence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<RulesOptions>,
    /// PwrRoute extension: rewrites, applied in order before any handler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewrites: Vec<Rewrite>,
    /// Handlers, first match wins. The Go host calls them rules.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub rules: Vec<Rule>,
    /// PwrRoute extension: links with the browser (and profile, and URL)
    /// the whole config must send them to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<RouteTest>,
}

/// Who decides a link while a JS config is in charge: `rules.json`'s
/// `precedence`. Opt-in: only the exact word `"pwrroute"` changes anything.
///
/// | Value | Handlers | Everything else |
/// | --- | --- | --- |
/// | absent, `"config"`, anything else | the config's, then `rules.json`'s | the config's `defaultBrowser` |
/// | `"pwrroute"` | `rules.json`'s, then the config's | `rules.json`'s `defaultBrowser`/`defaultProfile` |
///
/// Rewrites run the config's first, then `rules.json`'s, under both: they
/// compose rather than compete. With no config in charge `rules.json` is the
/// whole config and the value changes nothing. Under `PwrRoute`, an empty
/// `defaultBrowser`, or one naming PwrRoute, leaves the config's default in
/// place: the override needs a browser to give the links to.
///
/// A value the engine does not know (a typo, another case, a word from a
/// newer PwrRoute, a non-string) reads as [`Precedence::Config`] rather than
/// failing the whole file: the rules keep routing as the Go host routed
/// them. It is not kept, so the next save leaves the field out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Precedence {
    /// The Go host's order: the config's handlers run first and its
    /// `defaultBrowser` takes everything else. Written as `"config"`, but
    /// [`RulesFile`] leaves it out.
    #[default]
    Config,
    /// `"pwrroute"`: `rules.json`'s handlers run before the config's, and
    /// `rules.json`'s default browser takes everything else.
    PwrRoute,
}

impl Precedence {
    /// The Go host's order, the default; `RulesFile` does not write it.
    pub fn is_config(&self) -> bool {
        *self == Self::Config
    }

    /// The word `rules.json` uses: `"config"` or `"pwrroute"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::PwrRoute => "pwrroute",
        }
    }
}

impl<'de> Deserialize<'de> for Precedence {
    /// `"pwrroute"` exactly is [`Precedence::PwrRoute`]; any other value,
    /// of any type, is [`Precedence::Config`] (see the type's docs).
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(word) if word == "pwrroute" => Self::PwrRoute,
            _ => Self::Config,
        })
    }
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
    /// PwrRoute extension: open the link in a private (incognito) window.
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    /// PwrRoute extension: what the router may keep of a link this rule
    /// routed. Absent means [`Privacy::Full`], or [`Privacy::Host`] for a
    /// private rule; [`Rule::effective_privacy`] says which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacySetting>,
    /// PwrRoute extension: URLs that must resolve to this rule.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            browser: String::new(),
            profile: String::new(),
            opener: Vec::new(),
            enabled: true,
            private: false,
            privacy: None,
            examples: Vec::new(),
        }
    }
}

impl Rule {
    /// The privacy level this rule gives a link: `privacy` as written, or
    /// [`Privacy::Host`] for a private rule and [`Privacy::Full`] otherwise.
    pub fn effective_privacy(&self) -> Privacy {
        match &self.privacy {
            Some(setting) => setting.level(),
            None if self.private => Privacy::Host,
            None => Privacy::Full,
        }
    }
}

/// A `privacy` value as the file has it: one of the three levels, or a word
/// the engine does not know, kept as written so [`RulesFile::problems`] can
/// name it and [`RulesFile::save`] does not drop it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PrivacySetting {
    Level(Privacy),
    Unknown(String),
}

impl PrivacySetting {
    /// The level the engine applies. An unknown word asked for privacy of
    /// some kind, so it keeps nothing.
    pub fn level(&self) -> Privacy {
        match self {
            Self::Level(level) => *level,
            Self::Unknown(_) => Privacy::None,
        }
    }

    fn problem(&self) -> Option<&'static str> {
        match self {
            Self::Level(_) => None,
            Self::Unknown(_) => Some("Privacy must be full, host or none."),
        }
    }
}

impl From<Privacy> for PrivacySetting {
    fn from(level: Privacy) -> Self {
        Self::Level(level)
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
    /// What the router may keep of a link this rewrite touched. Absent means
    /// [`Privacy::Full`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<PrivacySetting>,
    /// URLs this rewrite must match, and what it must turn each into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<RewriteExample>,
}

impl Default for Rewrite {
    fn default() -> Self {
        Self {
            patterns: Vec::new(),
            actions: Vec::new(),
            enabled: true,
            privacy: None,
            examples: Vec::new(),
        }
    }
}

impl Rewrite {
    /// The privacy level this rewrite gives a link it touched.
    pub fn effective_privacy(&self) -> Privacy {
        self.privacy
            .as_ref()
            .map_or(Privacy::Full, PrivacySetting::level)
    }
}

/// One URL a [`Rewrite`] must match, and the URL it must produce from it
/// (the URL after this rewrite alone, before any later rewrite).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteExample {
    pub url: String,
    pub becomes: String,
}

/// A link the whole config must route somewhere in particular.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteTest {
    pub url: String,
    /// The app the link comes from: a name or bundle id, or `Name:bundle.id`
    /// for both. The test's opener has that name and that bundle id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<String>,
    pub expect: Expectation,
}

/// Where a [`RouteTest`] must end. Only what is given is compared.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Expectation {
    /// The browser as the rule or config that should win names it: its
    /// `browser` field as written, compared ignoring case. The engine does
    /// not know that `Safari` and `com.apple.Safari` are one app, so write
    /// the same spelling the rule uses. `Name:profile` names the profile
    /// too, as Finicky reads it.
    pub browser: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The URL the browser must receive, after every rewrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
    Test(usize),
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
            flag(
                Place::DefaultBrowser,
                match self.precedence {
                    Precedence::Config => "Select a default browser.",
                    // The override is inert without a browser: the
                    // config's default keeps everything else.
                    Precedence::PwrRoute => {
                        "Pick a default browser for PwrRoute to take everything else."
                    }
                },
            );
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
            if let Some(message) = rule.privacy.as_ref().and_then(PrivacySetting::problem) {
                flag(place, message);
            }
            if !rule.examples.iter().all(|u| is_absolute_url(u)) {
                flag(place, EXAMPLE_URL);
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
            if let Some(message) = rewrite.privacy.as_ref().and_then(PrivacySetting::problem) {
                flag(place, message);
            }
            if !rewrite
                .examples
                .iter()
                .all(|e| is_absolute_url(&e.url) && is_absolute_url(&e.becomes))
            {
                flag(place, EXAMPLE_URL);
            }
        }
        for (i, test) in self.tests.iter().enumerate() {
            let place = Place::Test(i);
            if !is_absolute_url(&test.url) {
                flag(place, EXAMPLE_URL);
            }
            if test.expect.browser.trim().is_empty() {
                flag(place, "Name the browser the link must open in.");
            }
            if test
                .expect
                .url
                .as_deref()
                .is_some_and(|u| !is_absolute_url(u))
            {
                flag(place, "The expected URL must be a full URL.");
            }
            if test.opener.as_deref().is_some_and(|o| o.trim().is_empty()) {
                flag(place, "App names cannot be empty.");
            }
        }
        problems
    }

    /// Whether these rules run before a JS config's handlers:
    /// `"precedence": "pwrroute"`. Without a config it changes nothing.
    pub fn rules_first(&self) -> bool {
        self.precedence == Precedence::PwrRoute
    }

    /// Whether this file's `defaultBrowser` (and `defaultProfile`) takes
    /// everything else while a JS config is in charge, in place of the
    /// config's own `defaultBrowser`: `"precedence": "pwrroute"` with a
    /// default browser that is not empty and not PwrRoute itself. Otherwise
    /// the config keeps its default, and [`RulesFile::problems`] says why.
    /// Without a config this file's default always applies.
    pub fn owns_default(&self) -> bool {
        self.rules_first()
            && !self.default_browser.trim().is_empty()
            && !is_self(&self.default_browser)
    }

    /// The rules as Finicky config objects, the shape the host's JS glue
    /// turns into `handlers` and `rewrite` entries. Handlers follow the Go
    /// host's `ToJSHandlers`: empty patterns are dropped, and a rule with no
    /// pattern or no browser is skipped. Disabled rules are skipped too.
    ///
    /// With a JS config in charge the glue also reads `rulesFirst` (put
    /// these handlers before the config's) and, when present,
    /// `defaultBrowser` (use it instead of the config's): see
    /// [`RulesFile::rules_first`] and [`RulesFile::owns_default`]. Without a
    /// config, [`RulesFile::base_config`] is the whole config and both are
    /// ignored.
    pub(crate) fn plan(&self) -> Value {
        let handlers: Vec<Value> = self
            .planned_rules()
            .into_iter()
            .filter_map(|i| {
                let r = &self.rules[i];
                let matches = non_empty(&r.patterns)?;
                let browser = browser_spec(&r.browser, &r.profile);
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
        let mut plan = json!({
            "handlers": handlers,
            "rewrites": rewrites,
            "rulesFirst": self.rules_first(),
        });
        if self.owns_default() {
            plan["defaultBrowser"] = browser_spec(&self.default_browser, &self.default_profile);
        }
        plan
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
        let mut config = json!({ "defaultBrowser": browser_spec(name, &self.default_profile) });
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

/// A browser as a Finicky handler names one: the plain string when there is
/// no profile (the API classifies it itself), else the object form with
/// `appType` set by [`app_type`]. The API's `createBrowserConfig` leaves an
/// object's `appType` at `appName` when it is absent, which turns a bundle
/// id into `open -a com.google.Chrome`; and the `"name:profile"` string form
/// splits on the first `:`, so it is never used.
fn browser_spec(browser: &str, profile: &str) -> Value {
    if profile.is_empty() {
        json!(browser)
    } else {
        json!({ "name": browser, "appType": app_type(browser), "profile": profile })
    }
}

/// How Finicky reads a browser string (`autodetectAppStringType` in its
/// configuration API): letters, digits and spaces are an app name; letters,
/// digits, `.` and `-` a bundle id; `/x/y.app`, optionally after `~`, a
/// path; anything else an app name. The words are the API's `appType`
/// values and [`crate::AppType`]'s serialized names.
pub(crate) fn app_type(app: &str) -> &'static str {
    let all = |ok: fn(char) -> bool| !app.is_empty() && app.chars().all(ok);
    if all(|c| c.is_ascii_alphanumeric() || c == ' ') {
        "appName"
    } else if all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        "bundleId"
    } else if is_app_path(app) {
        "path"
    } else {
        "appName"
    }
}

/// Finicky's path shape: `^(~?(?:\/[^/\n]+)+\/[^/\n]+\.app)$`, that is at
/// least two non-empty `/`-separated components, the last ending in `.app`.
fn is_app_path(app: &str) -> bool {
    let rest = app.strip_prefix('~').unwrap_or(app);
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    let parts: Vec<&str> = rest.split('/').collect();
    parts.len() >= 2
        && parts.iter().all(|p| !p.is_empty() && !p.contains('\n'))
        && parts.last().is_some_and(|p| p.ends_with(".app"))
}

const EXAMPLE_URL: &str = "Examples must be full URLs, such as https://example.com/path.";

/// A URL as the engine receives links: `http` or `https`, a scheme Finicky
/// matches without an authority (`mailto:`, `tel:`, `sms:`, `data:`,
/// `file:`), or an app's own scheme with `://`. `github.com/x` is not one,
/// and neither is `localhost:3000/x`, which parses with `localhost` as its
/// scheme.
fn is_absolute_url(text: &str) -> bool {
    let text = text.trim();
    let Ok(url) = url::Url::parse(text) else {
        return false;
    };
    let scheme = url.scheme();
    matches!(
        scheme,
        "http" | "https" | "mailto" | "tel" | "sms" | "data" | "file"
    ) || text[scheme.len()..].starts_with("://")
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

fn is_false(value: &bool) -> bool {
    !*value
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
                { "match": ["c/*", "d/*"], "browser": { "name": "Arc", "appType": "appName", "profile": "Work" } }
            ])
        );
    }

    #[test]
    fn a_profiled_browser_carries_the_app_type_finicky_would_detect() {
        // The object form is the only one with a profile that keeps a bundle
        // id or path whole; without `appType`, the API reads it as a name.
        let with_profile = |browser: &str| Rule {
            patterns: vec!["a/*".into()],
            browser: browser.into(),
            profile: "Work".into(),
            ..Rule::default()
        };
        let file = RulesFile {
            default_browser: "com.google.Chrome".into(),
            default_profile: "Personal".into(),
            rules: vec![
                with_profile("com.google.Chrome"),
                with_profile("/Applications/Google Chrome.app"),
                with_profile("~/Applications/Google Chrome.app"),
                with_profile("Google Chrome"),
            ],
            ..RulesFile::default()
        };
        let browsers: Vec<Value> = file.plan()["handlers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["browser"].clone())
            .collect();
        assert_eq!(
            browsers,
            [
                json!({ "name": "com.google.Chrome", "appType": "bundleId", "profile": "Work" }),
                json!({ "name": "/Applications/Google Chrome.app", "appType": "path", "profile": "Work" }),
                json!({ "name": "~/Applications/Google Chrome.app", "appType": "path", "profile": "Work" }),
                json!({ "name": "Google Chrome", "appType": "appName", "profile": "Work" }),
            ]
        );
        assert_eq!(
            file.base_config()["defaultBrowser"],
            json!({ "name": "com.google.Chrome", "appType": "bundleId", "profile": "Personal" })
        );
        // Without a profile the string form is kept, as the Go host wrote it,
        // and the API classifies it the same way.
        let file = RulesFile {
            default_browser: "com.google.Chrome".into(),
            ..RulesFile::default()
        };
        assert_eq!(
            file.base_config()["defaultBrowser"],
            json!("com.google.Chrome")
        );
    }

    #[test]
    fn app_type_matches_finickys_autodetect() {
        for (app, expected) in [
            ("Safari", "appName"),
            ("Google Chrome", "appName"),
            ("Firefox Developer Edition", "appName"),
            ("com.apple.Safari", "bundleId"),
            ("org.mozilla.firefox", "bundleId"),
            ("company.thebrowser.Browser", "bundleId"),
            ("us.zoom.xos", "bundleId"),
            ("/Applications/Firefox.app", "path"),
            ("/Applications/Google Chrome.app", "path"),
            ("~/Applications/Firefox.app", "path"),
            ("/Users/me/Applications/Beta/Firefox Nightly.app", "path"),
            // Finicky's shape needs two components and a `.app` end.
            ("/Firefox.app", "appName"),
            ("~/Firefox.app", "appName"),
            ("/Applications/Firefox.app/", "appName"),
            ("/Applications//Firefox.app", "appName"),
            ("/usr/bin/firefox", "appName"),
            ("Applications/Firefox.app", "appName"),
            // Anything else is a name, as in the API.
            ("Brave Browser (Beta)", "appName"),
            ("Firefox:Work", "appName"),
            ("", "appName"),
        ] {
            assert_eq!(app_type(app), expected, "{app:?}");
        }
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
                "defaultBrowser": { "name": "Firefox", "appType": "appName", "profile": "Work" },
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
    fn privacy_examples_and_tests_round_trip() {
        let text = r#"{
          "defaultBrowser": "Firefox",
          "rewrites": [
            { "match": "x.com/*", "actions": [{ "setHost": "xcancel.com" }], "privacy": "none",
              "examples": [{ "url": "https://x.com/a", "becomes": "https://xcancel.com/a" }] }
          ],
          "rules": [
            { "match": "bank.example/*", "browser": "Safari", "private": true, "examples": ["https://bank.example/"] },
            { "match": "a.example/*", "browser": "Safari", "privacy": "host" },
            { "match": "b.example/*", "browser": "Safari", "private": true, "privacy": "full" }
          ],
          "tests": [
            { "url": "https://bank.example/", "expect": { "browser": "Safari" } },
            { "url": "https://x.com/a", "opener": "Slack", "expect": { "browser": "Firefox", "profile": "Work", "url": "https://xcancel.com/a" } }
          ]
        }"#;
        let file: RulesFile = serde_json::from_str(text).unwrap();
        assert_eq!(file.rewrites[0].effective_privacy(), Privacy::None);
        assert_eq!(
            file.rewrites[0].examples,
            [RewriteExample {
                url: "https://x.com/a".into(),
                becomes: "https://xcancel.com/a".into()
            }]
        );
        let levels: Vec<_> = file.rules.iter().map(Rule::effective_privacy).collect();
        assert_eq!(levels, [Privacy::Host, Privacy::Host, Privacy::Full]);
        assert_eq!(file.rules[0].examples, ["https://bank.example/"]);
        assert_eq!(file.tests[1].opener.as_deref(), Some("Slack"));
        assert_eq!(file.tests[1].expect.profile.as_deref(), Some("Work"));
        let again: RulesFile =
            serde_json::from_str(&serde_json::to_string(&file).unwrap()).unwrap();
        assert_eq!(again, file);
        // What was absent stays absent, and `private: true` is written as such.
        let value = serde_json::to_value(&file).unwrap();
        assert_eq!(
            value["rules"][0],
            json!({ "match": "bank.example/*", "browser": "Safari", "private": true, "examples": ["https://bank.example/"] })
        );
        assert_eq!(
            value["rules"][1],
            json!({ "match": "a.example/*", "browser": "Safari", "privacy": "host" })
        );
        assert_eq!(
            value["tests"][0],
            json!({ "url": "https://bank.example/", "expect": { "browser": "Safari" } })
        );
    }

    #[test]
    fn an_example_needs_a_scheme_the_engine_would_receive() {
        for ok in [
            "https://example.com/x",
            "http://localhost:3000/x",
            " https://example.com ",
            "mailto:alice@example.com",
            "tel:+15555550100",
            "data:text/plain,hi",
            "file:///tmp/x.html",
            "vscode://file/x.rs",
            "slack://channel?id=1",
        ] {
            assert!(is_absolute_url(ok), "{ok}");
        }
        for bad in [
            "github.com/x",
            "localhost:3000/x",
            "example.com:443/x",
            "",
            "not a url",
        ] {
            assert!(!is_absolute_url(bad), "{bad}");
        }
    }

    #[test]
    fn an_unknown_privacy_level_loads_and_is_a_problem() {
        let file: RulesFile = serde_json::from_str(
            r#"{ "defaultBrowser": "Firefox", "rules": [{ "match": "a/*", "browser": "Safari", "privacy": "hidden" }] }"#,
        )
        .unwrap();
        assert_eq!(
            file.rules[0].privacy,
            Some(PrivacySetting::Unknown("hidden".into()))
        );
        assert_eq!(file.rules[0].effective_privacy(), Privacy::None);
        let problems = file.problems();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].place, Place::Rule(0));
        assert_eq!(problems[0].message, "Privacy must be full, host or none.");
        // A wrong type is still a load error, as for every other field.
        assert!(
            serde_json::from_str::<RulesFile>(
                r#"{ "rules": [{ "match": "a/*", "browser": "Safari", "privacy": 2 }] }"#
            )
            .is_err()
        );
    }

    #[test]
    fn an_absent_precedence_stays_absent() {
        let text = r#"{ "defaultBrowser": "Safari", "rules": [{ "match": "a.example/*", "browser": "Firefox" }] }"#;
        let file: RulesFile = serde_json::from_str(text).unwrap();
        assert_eq!(file.precedence, Precedence::Config);
        assert!(file.precedence.is_config());
        let written = serde_json::to_value(&file).unwrap();
        assert!(written.get("precedence").is_none(), "{written}");
        assert_eq!(
            written,
            serde_json::from_str::<serde_json::Value>(text).unwrap()
        );

        // "config" says the same as absent, so it is not written back.
        let file: RulesFile =
            serde_json::from_str(r#"{ "defaultBrowser": "Safari", "precedence": "config" }"#)
                .unwrap();
        assert_eq!(file.precedence, Precedence::Config);
        assert_eq!(
            serde_json::to_value(&file).unwrap(),
            json!({ "defaultBrowser": "Safari", "rules": [] })
        );
    }

    #[test]
    fn pwrroute_precedence_round_trips() {
        let text = r#"{
          "defaultBrowser": "Firefox",
          "defaultProfile": "Work",
          "precedence": "pwrroute",
          "rules": [{ "match": "a.example/*", "browser": "Safari" }]
        }"#;
        let file: RulesFile = serde_json::from_str(text).unwrap();
        assert_eq!(file.precedence, Precedence::PwrRoute);
        assert!(!file.precedence.is_config());
        assert_eq!(file.precedence.as_str(), "pwrroute");
        let written = serde_json::to_value(&file).unwrap();
        assert_eq!(written["precedence"], "pwrroute");
        assert_eq!(
            written,
            serde_json::from_str::<serde_json::Value>(text).unwrap()
        );
        // Written next to the default browser it is about.
        let keys: Vec<&str> = written
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["defaultBrowser", "defaultProfile", "precedence", "rules"]
        );
        assert_eq!(serde_json::from_value::<RulesFile>(written).unwrap(), file);

        let dir = std::env::temp_dir().join(format!("pwrroute-precedence-{}", std::process::id()));
        let path = dir.join("rules.json");
        file.save(&path).unwrap();
        assert_eq!(RulesFile::load(&path).unwrap(), file);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unknown_precedence_keeps_the_go_order_and_the_file_loads() {
        for value in [
            json!("PwrRoute"),
            json!("rules-first"),
            json!(""),
            json!(true),
            json!(1),
            json!(null),
            json!({ "mode": "pwrroute" }),
        ] {
            let file: RulesFile = serde_json::from_value(json!({
                "defaultBrowser": "Firefox",
                "precedence": value,
                "rules": [{ "match": "a.example/*", "browser": "Safari" }]
            }))
            .unwrap_or_else(|e| panic!("{value}: {e}"));
            assert_eq!(file.precedence, Precedence::Config, "{value}");
            assert_eq!(file.rules.len(), 1, "{value}");
            // Not kept: the next save leaves the field out.
            assert!(
                serde_json::to_value(&file)
                    .unwrap()
                    .get("precedence")
                    .is_none(),
                "{value}"
            );
        }
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

    #[test]
    fn the_plan_carries_the_precedence_flags() {
        let file = RulesFile {
            default_browser: "Firefox".into(),
            default_profile: "Work".into(),
            ..RulesFile::default()
        };
        // The Go order: nothing for the glue to change.
        let plan = file.plan();
        assert_eq!(plan["rulesFirst"], json!(false));
        assert!(plan.get("defaultBrowser").is_none(), "{plan}");

        let file = RulesFile {
            precedence: Precedence::PwrRoute,
            ..file
        };
        assert!(file.rules_first() && file.owns_default());
        let plan = file.plan();
        assert_eq!(plan["rulesFirst"], json!(true));
        assert_eq!(
            plan["defaultBrowser"],
            json!({ "name": "Firefox", "appType": "appName", "profile": "Work" })
        );
        let plain = RulesFile {
            default_profile: String::new(),
            ..file.clone()
        };
        assert_eq!(plain.plan()["defaultBrowser"], json!("Firefox"));

        // No usable default: the handlers still go first, the config keeps
        // its default.
        for default_browser in ["", " ", "PwrRoute", "PwrRoute:Work"] {
            let file = RulesFile {
                default_browser: default_browser.into(),
                ..file.clone()
            };
            assert!(
                file.rules_first() && !file.owns_default(),
                "{default_browser:?}"
            );
            let plan = file.plan();
            assert_eq!(plan["rulesFirst"], json!(true));
            assert!(plan.get("defaultBrowser").is_none(), "{plan}");
        }
    }

    #[test]
    fn an_empty_default_under_pwrroute_asks_for_one_to_take_everything_else() {
        let message = |precedence| {
            RulesFile {
                precedence,
                ..RulesFile::default()
            }
            .problems()
            .into_iter()
            .find(|p| p.place == Place::DefaultBrowser)
            .map(|p| p.message)
        };
        assert_eq!(
            message(Precedence::Config).as_deref(),
            Some("Select a default browser.")
        );
        assert_eq!(
            message(Precedence::PwrRoute).as_deref(),
            Some("Pick a default browser for PwrRoute to take everything else.")
        );
    }
}
