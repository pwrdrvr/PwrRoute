//! What one dispatch did, step by step: the record Activity lists, Test a
//! link shows, and an agent's `pwrroute_resolve` returns.

use std::time::{Duration, SystemTime};

use pwrroute_engine::Browser;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::link::{Link, Source};

/// How much of a link a record keeps, in memory and in the journal: the
/// engine's level, re-exported so the router and the engine cannot disagree.
///
/// The rule that routed the link decides (the engine's `Browser` carries the
/// tightest level of the `rules.json` rewrites that matched and the handler
/// that won); a link no rule matched is kept in full. The router reduces
/// the record to this level before anything stores it, sees it or writes
/// it, so a `Host` record never held the full URL anywhere outside the
/// dispatch thread. What each level keeps of a record:
///
/// - `Full`: the URL, the opener and every step, as received.
/// - `Host`: scheme and host only, in the link, the final URL and every
///   step (where a short link landed included); no error text, which can
///   quote the URL. The opener's name,
///   bundle id and path stay; its `window_title` goes, since an app can
///   fill it with the page's title or the URL itself.
/// - `None`: no URL, no host, no opener, no steps: the dispatch is only
///   counted, with its outcome and timings.
pub use pwrroute_engine::Privacy;

/// What the router asks of a [`Privacy`] level beyond the engine's
/// `tighter`: capping a record and naming the level for logs.
pub trait PrivacyExt: Copy {
    /// The lesser of two levels: what a record kept at `self` may become
    /// under a cap such as the journal's mode. The engine's `tighter`
    /// decides the order, so it is the same here as in `rules.json`.
    #[must_use]
    fn at_most(self, cap: Privacy) -> Privacy;

    /// A fixed label, for logs and descriptions. Never carries link data.
    fn label(self) -> &'static str;
}

impl PrivacyExt for Privacy {
    fn at_most(self, cap: Privacy) -> Privacy {
        self.tighter(cap)
    }

    fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Host => "host",
            Self::None => "none",
        }
    }
}

/// What replaces error text in a reduced record: a config or launcher
/// message can quote the URL, so a `Host` record keeps the step, not the
/// words.
pub const WITHHELD: &str = "withheld: this link is kept as host only";

/// A dispatch's number, unique within one router process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DispatchId(pub u64);

/// One dispatch: the link, every step that fired, and how long each part took.
///
/// It also reads back from the journal (one record per JSON line), so the
/// Activity screen and the MCP tool show journaled dispatches the same way.
/// `id` is unique within one router process only; journaled records from
/// earlier runs can repeat one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchRecord {
    pub id: DispatchId,
    #[serde(serialize_with = "unix_millis", deserialize_with = "from_unix_millis")]
    pub received_at: SystemTime,
    /// The level this record is kept at. A `Host` or `None` record was
    /// reduced before it was stored, journaled or handed to `on_dispatch`.
    /// A Test a link record stores nothing, so it is not reduced; its level
    /// says what a dispatch of the same link would keep.
    #[serde(default)]
    pub privacy: Privacy,
    pub link: Link,
    /// The link came from a second launch, over the IPC socket.
    pub forwarded: bool,
    /// The URL after every rewrite: what the browser was asked to open.
    pub final_url: Option<String>,
    /// The browser the link went to, or would have, when one was chosen.
    pub browser: Option<Browser>,
    pub steps: Vec<Step>,
    pub outcome: Outcome,
    pub timings: Timings,
}

/// One step in a dispatch's trace, in the order it happened.
///
/// Rewrites and handlers are two lists, as in Finicky v4: every matching
/// rewrite runs in order, then the first matching handler wins, then the
/// default. Rule numbers are indexes into the config's `rewrite` and
/// `handlers` arrays, present when the resolver reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Step {
    /// The link as received, before any rule ran.
    Received {
        url: String,
    },
    /// The link was on a shortener's domain and was followed to where it
    /// lands; `url` is that destination, which the rules then saw. Absent
    /// when the link was not short, expansion is off, or the shortener did
    /// not answer (the short link then goes to the rules as it is).
    Expanded {
        url: String,
    },
    /// A rewrite rule matched and ran; `url` is the URL after it, which can
    /// equal the URL before. `rule` is `None` when the resolver only reports
    /// the final URL (the step then means the URL changed, and `url` is the
    /// URL after every rewrite), or for a `rules.json` rewrite, which has no
    /// number in the config's list.
    Rewrite {
        rule: Option<usize>,
        url: String,
    },
    /// Which rule chose the browser.
    Matched {
        by: Matched,
    },
    /// The config threw or timed out. Finicky still returns the default
    /// browser in that case, and so does the router.
    ConfigError {
        message: String,
    },
    /// The fallback browser applies: no config is loaded, the config could
    /// not resolve the link, or (after a [`LaunchFailed`](Self::LaunchFailed))
    /// the chosen browser could not be opened, so the fallback opens it.
    Fallback {
        reason: String,
    },
    /// The config sent the link back to PwrRoute, which would loop. The
    /// fallback browser opens it instead.
    RefusedLoop {
        browser: String,
    },
    /// Routing was paused: every handler is ignored and the link goes to the
    /// pause target instead, after the rewrites. `skipped` is the config
    /// handler that matched and was passed over, when the resolver numbers
    /// it; a `rules.json` rule passed over is the record's
    /// [`Matched::Rule`], and `skipped` is then `None`.
    Paused {
        skipped: Option<usize>,
    },
    /// The chosen browser.
    Browser {
        browser: Browser,
    },
    /// `browser: null` in the config: the link is deliberately not opened.
    NotOpened,
    /// The launcher was asked to open it.
    Launched {
        dry_run: bool,
    },
    LaunchFailed {
        message: String,
    },
    /// The link was refused before routing: not a URL the router accepts.
    Rejected {
        reason: String,
    },
}

/// The rule that chose the browser.
///
/// Serialized inside [`Step::Matched`] as an object tagged by `kind`:
/// `{"kind":"handler","index":2}`, `{"kind":"rule","index":0}`,
/// `{"kind":"default","owner":"pwrroute"}` (`owner` left out when `None`)
/// and `{"kind":"unreported"}`. A record journaled before `rule` and
/// `owner` existed reads back unchanged: its `{"kind":"default"}` is
/// `Default { owner: None }`. An older build reading a newer journal reads
/// `default` with an owner too (it ignores the field), but not `rule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Matched {
    /// The JS config's `handlers[index]` matched first (0-based).
    Handler { index: usize },
    /// `rules.json`'s `rules[index]` matched first: its position in the
    /// file, 0-based, counting disabled rules (the engine's trace index for
    /// a `rules.json` step), whether it ran before or after the config's.
    Rule { index: usize },
    /// No handler matched; a default browser applied. `owner` says whose:
    /// the config's `defaultBrowser` or PwrRoute's (`rules.json`'s). `None`
    /// when the resolver did not say, and in records written before owners
    /// were recorded.
    Default {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<Owner>,
    },
    /// The resolver did not say: a resolver that returns no rule steps.
    /// `Resolver for Engine` always says; records journaled before
    /// `rules.json` rules were numbered carry this for those.
    Unreported,
}

impl Matched {
    /// A default browser applied, whoever owned it.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default { .. })
    }
}

/// Whose default browser took a link no handler matched
/// ([`Matched::Default`]). Serialized `config` or `pwrroute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    /// The JS config's `defaultBrowser`.
    Config,
    /// `rules.json`'s `defaultBrowser` and `defaultProfile`: with no JS
    /// config in charge, or under `"precedence": "pwrroute"`.
    PwrRoute,
}

impl From<pwrroute_engine::Source> for Owner {
    /// The owner of a default step, from the engine trace step's source.
    fn from(source: pwrroute_engine::Source) -> Self {
        match source {
            pwrroute_engine::Source::Config => Self::Config,
            pwrroute_engine::Source::Rules => Self::PwrRoute,
        }
    }
}

/// How a dispatch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Handed to the launcher, which reported success.
    Opened,
    /// Resolved and handed to a dry-run launcher.
    DryRun,
    /// Resolved only, for Test a link. Nothing opened, nothing recorded.
    Tested,
    /// The config chose `browser: null`.
    NotOpened,
    /// Rejected before routing, or the launcher failed.
    Failed,
}

/// Durations, in microseconds, measured from the moment the OS edge handed
/// the link to the router.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Waiting for the dispatch thread.
    pub queue_us: u64,
    pub resolve_us: u64,
    pub launch_us: u64,
    pub total_us: u64,
}

impl Timings {
    pub(crate) fn micros(d: Duration) -> u64 {
        u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
    }
}

/// A dispatch without its link: what may be written to disk. It carries
/// timings and fixed labels only, never URL contents, opener names or error
/// text (which can quote the URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchTiming {
    pub id: DispatchId,
    pub source: &'static str,
    pub forwarded: bool,
    pub outcome: Outcome,
    pub rewrites: usize,
    pub config_error: bool,
    pub timings: Timings,
}

impl DispatchRecord {
    /// Reduces the record to `level`, in place. `Full` changes nothing;
    /// `Host` keeps scheme and host of every URL, withholds error text and
    /// drops the opener's window title; `None` keeps the count: id, time,
    /// source, outcome and timings. A record already below `level` stays
    /// as it is.
    pub fn reduce(&mut self, level: Privacy) {
        let level = self.privacy.at_most(level);
        self.privacy = level;
        match level {
            Privacy::Full => {}
            Privacy::Host => {
                self.link.url = host_only(&self.link.url);
                if let Some(opener) = &mut self.link.opener {
                    // Free text from the opener: a page title, or the URL.
                    opener.window_title = None;
                }
                if let Some(url) = &mut self.final_url {
                    *url = host_only(url);
                }
                if let Some(browser) = &mut self.browser {
                    reduce_browser(browser);
                }
                for step in &mut self.steps {
                    reduce_step(step);
                }
            }
            Privacy::None => {
                self.link.url = String::new();
                self.link.opener = None;
                self.final_url = None;
                self.browser = None;
                self.steps.clear();
            }
        }
    }

    /// A copy reduced to `level`; see [`reduce`](Self::reduce).
    #[must_use]
    pub fn reduced(&self, level: Privacy) -> Self {
        let mut record = self.clone();
        record.reduce(level);
        record
    }

    /// The URL-free view of this dispatch, for logs.
    pub fn timing(&self) -> DispatchTiming {
        DispatchTiming {
            id: self.id,
            source: Source::label(self.link.source),
            forwarded: self.forwarded,
            outcome: self.outcome,
            rewrites: self
                .steps
                .iter()
                .filter(|s| matches!(s, Step::Rewrite { .. }))
                .count(),
            config_error: self
                .steps
                .iter()
                .any(|s| matches!(s, Step::ConfigError { .. })),
            timings: self.timings,
        }
    }
}

/// Scheme and host of a URL, the port included when one is given:
/// `https://example.com:8443`. A URL without a host (`mailto:`) keeps the
/// scheme alone, and one that does not parse keeps nothing.
pub fn host_only(url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return String::new();
    };
    let mut out = format!("{}:", parsed.scheme());
    if let Some(host) = parsed.host_str() {
        out.push_str("//");
        out.push_str(host);
        if let Some(port) = parsed.port() {
            out.push_str(&format!(":{port}"));
        }
    }
    out
}

fn reduce_browser(browser: &mut Browser) {
    browser.url = host_only(&browser.url);
    // Extra arguments come from the config and can carry the URL.
    browser.args.clear();
}

fn reduce_step(step: &mut Step) {
    match step {
        Step::Received { url } | Step::Expanded { url } | Step::Rewrite { url, .. } => {
            *url = host_only(url);
        }
        Step::Browser { browser } => reduce_browser(browser),
        Step::ConfigError { message } | Step::LaunchFailed { message } => {
            *message = WITHHELD.to_owned();
        }
        Step::Matched { .. }
        | Step::Fallback { .. }
        | Step::RefusedLoop { .. }
        | Step::Paused { .. }
        | Step::NotOpened
        | Step::Launched { .. }
        | Step::Rejected { .. } => {}
    }
}

fn unix_millis<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let ms = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    s.serialize_u64(ms)
}

fn from_unix_millis<'de, D: Deserializer<'de>>(d: D) -> Result<SystemTime, D::Error> {
    let ms = u64::deserialize(d)?;
    Ok(SystemTime::UNIX_EPOCH + Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use pwrroute_engine::AppType;

    use super::*;
    use crate::link::OpenerInfo;

    fn browser(url: &str) -> Browser {
        Browser {
            name: "Firefox".into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: "Work".into(),
            args: vec![format!("--open={url}")],
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        }
    }

    fn record() -> DispatchRecord {
        let url = "https://bank.example.com:8443/account?token=hunter2#x";
        let rewritten = "https://bank.example.com/new?token=hunter2";
        DispatchRecord {
            id: DispatchId(3),
            received_at: SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_123),
            privacy: Privacy::Full,
            link: Link::new(url, Source::AppleEvent).with_opener(Some(OpenerInfo {
                name: "Mail".into(),
                bundle_id: "com.apple.mail".into(),
                path: "/System/Applications/Mail.app".into(),
                window_title: Some("Inbox".into()),
            })),
            forwarded: false,
            final_url: Some(rewritten.into()),
            browser: Some(browser(rewritten)),
            steps: vec![
                Step::Received { url: url.into() },
                Step::Rewrite {
                    rule: Some(0),
                    url: rewritten.into(),
                },
                Step::ConfigError {
                    message: format!("matcher threw on {url}"),
                },
                Step::Matched {
                    by: Matched::Handler { index: 2 },
                },
                Step::Paused { skipped: Some(2) },
                Step::Browser {
                    browser: browser(rewritten),
                },
                Step::LaunchFailed {
                    message: format!("firefox: cannot open {rewritten}"),
                },
            ],
            outcome: Outcome::Failed,
            timings: Timings {
                queue_us: 1,
                resolve_us: 2,
                launch_us: 3,
                total_us: 6,
            },
        }
    }

    #[test]
    fn matched_reads_what_older_builds_wrote_and_they_read_its_defaults() {
        // Lines from builds before `rule` and `owner`.
        for (text, by) in [
            (
                r#"{"kind":"matched","by":{"kind":"default"}}"#,
                Matched::Default { owner: None },
            ),
            (
                r#"{"kind":"matched","by":{"kind":"handler","index":2}}"#,
                Matched::Handler { index: 2 },
            ),
            (
                r#"{"kind":"matched","by":{"kind":"unreported"}}"#,
                Matched::Unreported,
            ),
        ] {
            let step: Step = serde_json::from_str(text).unwrap();
            assert_eq!(step, Step::Matched { by });
            // And written back the same way.
            assert_eq!(serde_json::to_string(&step).unwrap(), text);
        }

        // The new forms round-trip.
        for (by, text) in [
            (Matched::Rule { index: 0 }, r#"{"kind":"rule","index":0}"#),
            (
                Matched::Default {
                    owner: Some(Owner::Config),
                },
                r#"{"kind":"default","owner":"config"}"#,
            ),
            (
                Matched::Default {
                    owner: Some(Owner::PwrRoute),
                },
                r#"{"kind":"default","owner":"pwrroute"}"#,
            ),
        ] {
            assert_eq!(serde_json::to_string(&by).unwrap(), text);
            assert_eq!(serde_json::from_str::<Matched>(text).unwrap(), by);
        }
        assert!(Matched::Default { owner: None }.is_default());
        assert!(!Matched::Rule { index: 0 }.is_default());
        assert_eq!(Owner::from(pwrroute_engine::Source::Rules), Owner::PwrRoute);
        assert_eq!(Owner::from(pwrroute_engine::Source::Config), Owner::Config);

        // The enum as older builds had it: a default with an owner still
        // reads there (the field is ignored); a `rule` does not, and their
        // journal reader skips a line that does not parse.
        #[derive(Debug, PartialEq, Deserialize)]
        #[serde(tag = "kind", rename_all = "camelCase")]
        enum Old {
            Handler {
                #[allow(dead_code, reason = "only the shape is read")]
                index: usize,
            },
            Default,
            Unreported,
        }
        assert_eq!(
            serde_json::from_str::<Old>(r#"{"kind":"default","owner":"pwrroute"}"#).unwrap(),
            Old::Default
        );
        assert!(serde_json::from_str::<Old>(r#"{"kind":"rule","index":0}"#).is_err());
    }

    #[test]
    fn host_only_keeps_scheme_host_and_port() {
        assert_eq!(
            host_only("https://bank.example.com:8443/account?token=hunter2#x"),
            "https://bank.example.com:8443"
        );
        assert_eq!(host_only("http://example.com/"), "http://example.com");
        assert_eq!(host_only("mailto:someone@example.com"), "mailto:");
        assert_eq!(host_only("not a url"), "");
    }

    #[test]
    fn a_host_record_keeps_hosts_and_the_opener_and_nothing_that_quotes_the_url() {
        let reduced = record().reduced(Privacy::Host);
        assert_eq!(reduced.privacy, Privacy::Host);
        assert_eq!(reduced.link.url, "https://bank.example.com:8443");
        assert_eq!(
            reduced.final_url.as_deref(),
            Some("https://bank.example.com")
        );
        let opener = reduced.link.opener.as_ref().unwrap();
        assert_eq!(opener.name, "Mail");
        assert_eq!(opener.bundle_id, "com.apple.mail");
        assert_eq!(opener.path, "/System/Applications/Mail.app");
        assert_eq!(opener.window_title, None);
        let browser = reduced.browser.as_ref().unwrap();
        assert_eq!(browser.url, "https://bank.example.com");
        assert_eq!(browser.name, "Firefox");
        assert_eq!(browser.profile, "Work");
        assert!(browser.args.is_empty());
        assert_eq!(
            reduced.steps,
            vec![
                Step::Received {
                    url: "https://bank.example.com:8443".into()
                },
                Step::Rewrite {
                    rule: Some(0),
                    url: "https://bank.example.com".into()
                },
                Step::ConfigError {
                    message: WITHHELD.into()
                },
                Step::Matched {
                    by: Matched::Handler { index: 2 }
                },
                Step::Paused { skipped: Some(2) },
                Step::Browser {
                    browser: browser.clone()
                },
                Step::LaunchFailed {
                    message: WITHHELD.into()
                },
            ]
        );
        let json = serde_json::to_string(&reduced).unwrap();
        for private in ["hunter2", "/account", "/new", "--open", "#x", "Inbox"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
        assert_eq!(reduced.outcome, Outcome::Failed);
        assert_eq!(reduced.timings, record().timings);
    }

    #[test]
    fn a_host_record_keeps_only_the_host_of_where_a_short_link_landed() {
        let mut full = record();
        full.link.url = "https://go.example/s3cr3t".into();
        full.steps.insert(
            1,
            Step::Expanded {
                url: "https://bank.example.com:8443/account?token=hunter2#x".into(),
            },
        );
        let reduced = full.reduced(Privacy::Host);
        assert_eq!(reduced.link.url, "https://go.example");
        assert_eq!(
            reduced.steps[1],
            Step::Expanded {
                url: "https://bank.example.com:8443".into()
            }
        );
        let json = serde_json::to_string(&reduced).unwrap();
        for private in ["s3cr3t", "hunter2", "/account"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
        assert!(full.reduced(Privacy::None).steps.is_empty());
    }

    #[test]
    fn a_host_record_drops_the_openers_window_title() {
        // A window title is the opener's free text: Mail puts the message's
        // subject there, a browser the page's title, and a title can be the
        // URL itself. At `Host` it goes the way the path and query go.
        let mut full = record();
        full.link.opener.as_mut().unwrap().window_title =
            Some("Reset your password - https://bank.example.com/account?token=hunter2".into());
        let reduced = full.reduced(Privacy::Host);
        let opener = reduced.link.opener.as_ref().unwrap();
        assert_eq!(opener.window_title, None);
        assert_eq!(opener.name, "Mail");
        let json = serde_json::to_string(&reduced).unwrap();
        assert!(!json.contains("windowTitle"), "{json}");
        assert!(!json.contains("Reset your password"), "{json}");
        // `Full` keeps it, and a record without an opener reduces as before.
        assert!(
            full.reduced(Privacy::Full)
                .link
                .opener
                .unwrap()
                .window_title
                .is_some()
        );
        full.link.opener = None;
        assert!(full.reduced(Privacy::Host).link.opener.is_none());
    }

    #[test]
    fn a_none_record_is_only_counted() {
        let reduced = record().reduced(Privacy::None);
        assert_eq!(reduced.privacy, Privacy::None);
        assert_eq!(reduced.id, DispatchId(3));
        assert_eq!(reduced.link.url, "");
        assert!(reduced.link.opener.is_none());
        assert_eq!(reduced.link.source, Source::AppleEvent);
        assert!(reduced.final_url.is_none());
        assert!(reduced.browser.is_none());
        assert!(reduced.steps.is_empty());
        assert_eq!(reduced.outcome, Outcome::Failed);
        assert_eq!(reduced.timings.total_us, 6);
        let json = serde_json::to_string(&reduced).unwrap();
        for private in ["example.com", "hunter2", "Mail", "Firefox", "Inbox"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }

    #[test]
    fn reducing_never_raises_the_level() {
        let mut reduced = record().reduced(Privacy::None);
        reduced.reduce(Privacy::Full);
        assert_eq!(reduced.privacy, Privacy::None);
        assert!(reduced.steps.is_empty());
        assert_eq!(Privacy::Host.at_most(Privacy::Full), Privacy::Host);
        assert_eq!(Privacy::Full.at_most(Privacy::Host), Privacy::Host);
        assert_eq!(Privacy::Host.at_most(Privacy::None), Privacy::None);
    }

    #[test]
    fn the_level_is_the_engines_and_serializes_as_lowercase_words() {
        // One type: a rule's level is a record's level with no conversion.
        let level: pwrroute_engine::Privacy = Privacy::Host;
        assert_eq!(level, Privacy::Host);
        for (level, word) in [
            (Privacy::Full, "full"),
            (Privacy::Host, "host"),
            (Privacy::None, "none"),
        ] {
            assert_eq!(level.label(), word);
            assert_eq!(serde_json::to_string(&level).unwrap(), format!("{word:?}"));
            let back: Privacy = serde_json::from_str(&format!("{word:?}")).unwrap();
            assert_eq!(back, level);
        }
        // The order is the engine's, so the two can never disagree.
        assert_eq!(Privacy::Full.at_most(Privacy::None), Privacy::None);
        assert_eq!(Privacy::None.at_most(Privacy::Full), Privacy::None);
        assert_eq!(
            Privacy::Full.tighter(Privacy::Host),
            Privacy::Full.at_most(Privacy::Host)
        );
    }

    #[test]
    fn a_record_round_trips_through_json() {
        let full = record();
        let json = serde_json::to_string(&full).unwrap();
        let back: DispatchRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, full.id);
        assert_eq!(back.received_at, full.received_at);
        assert_eq!(back.privacy, Privacy::Full);
        assert_eq!(back.link, full.link);
        assert_eq!(back.final_url, full.final_url);
        assert_eq!(back.browser, full.browser);
        assert_eq!(back.steps, full.steps);
        assert_eq!(back.outcome, full.outcome);
        assert_eq!(back.timings, full.timings);
        // A line written before `privacy` existed reads as full.
        let json = json.replace(r#""privacy":"full","#, "");
        let back: DispatchRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.privacy, Privacy::Full);
    }
}
