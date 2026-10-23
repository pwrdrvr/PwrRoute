//! The Overview as plain data: what the landing screen says, built from
//! the files, the router's state and today's tally by [`build`], with no
//! window. `overview.rs` draws it; the tests here cover every state the
//! design draws (`design/PwrRoute Overview and Setup.dc.html`, 1a-3f).
//!
//! Four parts, top to bottom: the hero (is it working, and if not, what
//! to do), the setup card (what is left), today's tiles, and the route map
//! (the rules in evaluation order on the left, one card per destination on
//! the right). [`Map::lit`] is the map's hover model: which rows, cards
//! and pills light, and which curves are drawn, for what is under the
//! pointer.
//!
//! Nothing here holds a URL: rows show patterns (the rules' own text),
//! counts and browser names.

use std::collections::BTreeSet;

use pwrroute_browsers::Catalog;
use pwrroute_engine::{
    AppType, BrowserSpec, Description, FALLBACK_BROWSER, MatcherSpec, Precedence, Rule, RulesFile,
};

use crate::activity::{Counts, Filter, HitKey, Tally, thousands};
use crate::default_browser::DefaultState;
use crate::widgets::badge_for;

/// The most curves drawn at once: a card fed by more draws none.
pub const MAX_CURVES: usize = 6;

/// The most profile pills a card shows before "+N".
pub const MAX_PILLS: usize = 4;

/// One browser the scan found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub bundle_id: Option<String>,
    pub profiles: Vec<String>,
    /// Hidden in Settings › Browsers: never listed as unused.
    pub hidden: bool,
}

impl Installed {
    /// Whether a rule's `browser` value names this app, as the launcher
    /// matches it, or by the catalog's name for the value.
    fn is_named(&self, value: &str, canonical: &str) -> bool {
        let value = value.trim();
        !value.is_empty()
            && (value.eq_ignore_ascii_case(&self.name)
                || canonical.eq_ignore_ascii_case(&self.name)
                || self
                    .bundle_id
                    .as_deref()
                    .is_some_and(|id| id.eq_ignore_ascii_case(value)))
    }
}

/// Routing is paused: where every link goes until it resumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pause {
    /// "Google Chrome", for the rail foot.
    pub name: String,
    /// "Google Chrome · Personal", for the hero.
    pub display: String,
}

/// The config in charge is in trouble, as the router reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigTrouble {
    /// It has never loaded (the router's `fallback`): nothing it or
    /// `rules.json` says runs, and every link opens in the fallback browser
    /// until it loads. The first line of why.
    NotLoaded(String),
    /// A reload failed (the router's `error` alone): the last version that
    /// loaded still routes, with `rules.json`. The first line of why.
    ReloadFailed(String),
}

impl ConfigTrouble {
    pub fn message(&self) -> &str {
        match self {
            Self::NotLoaded(message) | Self::ReloadFailed(message) => message,
        }
    }
}

/// What the router and the OS say right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub paused: Option<Pause>,
    pub default: DefaultState,
    /// The config in charge did not load, or did not reload.
    pub config_error: Option<ConfigTrouble>,
}

impl Status {
    /// Routing, PwrRoute the default browser, nothing wrong.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the window harness and the screens example")
    )]
    pub fn routing() -> Self {
        Self {
            paused: None,
            default: DefaultState::PwrRoute,
            config_error: None,
        }
    }

    /// The hero's state, highest priority first: the config did not load
    /// (or reload), paused, not the default browser, unknown, routing.
    pub fn state(&self) -> State {
        if let Some(trouble) = &self.config_error {
            match trouble {
                ConfigTrouble::NotLoaded(_) => State::ConfigError,
                ConfigTrouble::ReloadFailed(_) => State::ConfigStale,
            }
        } else if self.paused.is_some() {
            State::Paused
        } else {
            match self.default {
                DefaultState::Other(_) => State::NotDefault,
                DefaultState::Unknown => State::Unknown,
                DefaultState::PwrRoute => State::Routing,
            }
        }
    }
}

/// The routing state, as the hero, the strip chip and the rail foot say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// The config never loaded: every link goes to the fallback browser.
    ConfigError,
    /// Its last reload failed: the version that loaded before still runs.
    ConfigStale,
    Paused,
    NotDefault,
    Unknown,
    Routing,
}

/// A surface's colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Warn,
    Neutral,
    Danger,
}

/// What a button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ShowError,
    Resume,
    MakeDefault,
    OpenBrowsersSettings,
    TestLink,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::ShowError => "Show the error",
            Self::Resume => "Resume routing",
            Self::MakeDefault => "Make PwrRoute the default browser",
            Self::OpenBrowsersSettings => "Open Settings \u{203a} Browsers",
            Self::TestLink => "Test a link\u{2026}",
        }
    }

    /// The hero's primary buttons; the rest are plain.
    pub fn primary(self) -> bool {
        matches!(
            self,
            Self::Resume | Self::MakeDefault | Self::OpenBrowsersSettings
        )
    }
}

/// The band across the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hero {
    pub state: State,
    pub tone: Tone,
    pub sentence: String,
    /// Parts of the detail line, drawn with subtle "·" between them.
    pub detail: Vec<String>,
    pub action: Option<Action>,
}

/// One row of the setup card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupStep {
    pub title: &'static str,
    pub done: bool,
    /// The help line, or the done line.
    pub body: Option<String>,
    /// Step 1 once done: the badge and "Google Chrome · Personal".
    pub result: Option<(String, String)>,
}

/// "Finish setting up": shown while `rules.json` names no default browser
/// or another app is the OS default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setup {
    /// Choose where everything else goes.
    pub everything_else: SetupStep,
    /// Make PwrRoute the default browser.
    pub default_browser: SetupStep,
}

impl Setup {
    pub fn done(&self) -> usize {
        usize::from(self.everything_else.done) + usize::from(self.default_browser.done)
    }
}

/// One of today's tiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub label: &'static str,
    pub count: u64,
    /// The Activity filter a click opens.
    pub filter: Filter,
    /// Failed, above zero.
    pub danger: bool,
}

/// The map's left caption.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caption {
    /// "In order · first match wins", "today".
    InOrder,
    /// Paused: "When routing resumes", in warning text.
    WhenResumes,
    /// Not the default browser: "What will happen · first match wins".
    WhatWillHappen,
}

/// Which list a row is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowSource {
    /// `rules.json`'s `rules[index]`.
    Rule(usize),
    /// The config's `handlers[index]`.
    Handler(usize),
}

/// A row's match column: its first matcher, and how many more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowMatch {
    Pattern {
        text: String,
        regex: bool,
        more: usize,
    },
    /// "from [Sl Slack]": a rule with no patterns, only a sending app.
    Opener {
        badge: String,
        name: String,
        more: usize,
    },
    /// "ƒ isWorkplace", or `None` for an anonymous function.
    Function { name: Option<String>, more: usize },
    /// A matcher Finicky would not match with, by its kind.
    Other { text: String, more: usize },
    /// No pattern and no app: every link.
    Any,
}

/// Where a row sends links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A browser card; `label` is the profile when the rule names one,
    /// else the browser.
    Browser {
        card: usize,
        badge: String,
        label: String,
        profile: Option<String>,
    },
    /// A JS function picks the browser per link.
    Function { card: usize },
    /// `browser: null`: the link is not opened.
    NotOpened { card: usize },
    /// A browser the scan did not find: its links fall through.
    Ghost {
        card: usize,
        badge: String,
        name: String,
    },
    /// A draft with no browser yet.
    Unset,
}

impl Target {
    pub fn card(&self) -> Option<usize> {
        match self {
            Self::Browser { card, .. }
            | Self::Function { card }
            | Self::NotOpened { card }
            | Self::Ghost { card, .. } => Some(*card),
            Self::Unset => None,
        }
    }

    fn profile(&self) -> Option<&str> {
        match self {
            Self::Browser { profile, .. } => profile.as_deref(),
            _ => None,
        }
    }
}

/// One ranked rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 1-based, across sections, in evaluation order.
    pub rank: usize,
    pub source: RowSource,
    pub matcher: RowMatch,
    pub target: Target,
    pub enabled: bool,
    /// Nothing runs it: the config never loaded, and the router's
    /// stand-in engine holds no rules. Drawn faded.
    pub idle: bool,
    /// Today's links, when there are any and counts are shown.
    pub count: Option<u64>,
    /// Names the row's own list in Activity's words: "rule 1", "config
    /// handler 2 of ~/.finicky.ts".
    pub tooltip: String,
}

/// One entry of the left column, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// "~/.finicky.ts · runs first", with its note.
    Header { label: String, note: String },
    /// An index into [`Map::rows`].
    Row(usize),
    /// The config did not load: its section is this one row.
    Error { title: String, detail: String },
}

/// The pinned last row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EverythingElse {
    /// Whose default: "PwrRoute's default", "~/.finicky.ts's default",
    /// "Safari until you choose".
    pub owner: String,
    pub target: Target,
    pub count: Option<u64>,
}

/// A card's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardKind {
    Browser,
    /// "Decided by JavaScript".
    Function,
    NotOpened,
    /// Not installed: dashed.
    Ghost,
}

/// A profile pill on a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    /// `None`: the browser's own default profile.
    pub profile: Option<String>,
    /// Takes Everything else: "∞".
    pub everything_else: bool,
}

impl Pill {
    pub fn label(&self) -> &str {
        self.profile.as_deref().unwrap_or("default profile")
    }
}

/// A card's share of today's links.
#[derive(Clone, Debug, PartialEq)]
pub struct Share {
    /// "67%", "<1%".
    pub label: String,
    /// 0..=1, for the bar.
    pub fraction: f32,
}

/// One destination.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub kind: CardKind,
    pub badge: String,
    pub name: String,
    /// What rules name it by: the identity rows resolve to.
    key: String,
    pub pills: Vec<Pill>,
    /// Pills past [`MAX_PILLS`].
    pub more_pills: usize,
    /// Enabled rules that send links here.
    pub rules: usize,
    /// "2 rules · 944 links today".
    pub meta: String,
    pub links: Option<u64>,
    pub share: Option<Share>,
    /// "Everything else · PwrRoute" on the card that takes it.
    pub ribbon: Option<String>,
    /// The only card: "Everything opens in Safari", and the count on a
    /// line of its own.
    pub alone: Option<(String, Option<String>)>,
    /// Only rows that nothing runs send links here ([`Row::idle`]): drawn
    /// faded.
    pub idle: bool,
}

/// The left column with no rules: the first-rule prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Empty {
    /// "Every link opens in Safari."
    pub title: String,
    /// "Import from ~/.finicky.ts…", when a config was found and is not
    /// in charge.
    pub import: Option<String>,
}

/// The route map.
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub caption: Caption,
    /// Row counts, "today" and "links today" are shown: not while links
    /// are not reaching PwrRoute.
    pub counts: bool,
    /// Paused: the body dims.
    pub dimmed: bool,
    /// "Not used: Safari, Microsoft Edge".
    pub not_used: Option<String>,
    /// "3 rewrites run on every link first", when there are any.
    pub rewrites: Option<String>,
    pub items: Vec<Item>,
    pub rows: Vec<Row>,
    pub everything_else: EverythingElse,
    pub cards: Vec<Card>,
    pub empty: Option<Empty>,
}

/// What is under the pointer, or pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Focus {
    /// An index into [`Map::rows`].
    Row(usize),
    Card(usize),
    /// A card and one of its pills.
    Pill(usize, usize),
    EverythingElse,
}

/// The left end of a curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum End {
    Row(usize),
    EverythingElse,
}

/// What lights for a [`Focus`]; everything else drops to 45%.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lit {
    pub rows: BTreeSet<usize>,
    pub everything_else: bool,
    pub cards: BTreeSet<usize>,
    pub pills: BTreeSet<(usize, usize)>,
    /// Row (or Everything else) to card: at most [`MAX_CURVES`].
    pub curves: Vec<(End, usize)>,
}

impl Lit {
    pub fn row(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }

    pub fn card(&self, card: usize) -> bool {
        self.cards.contains(&card)
    }

    pub fn pill(&self, card: usize, pill: usize) -> bool {
        self.pills.contains(&(card, pill))
    }
}

impl Map {
    /// The pill on `card` a target names: its profile's, else the
    /// browser's default profile.
    fn pill_for(&self, card: usize, profile: Option<&str>) -> Option<usize> {
        self.cards.get(card)?.pills.iter().position(|pill| {
            match (pill.profile.as_deref(), profile) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            }
        })
    }

    /// What lights for `focus`.
    pub fn lit(&self, focus: Focus) -> Lit {
        let mut lit = Lit::default();
        let ee_card = self.everything_else.target.card();
        let ee_profile = self.everything_else.target.profile();
        match focus {
            Focus::Row(row) => {
                let Some(r) = self.rows.get(row) else {
                    return lit;
                };
                lit.rows.insert(row);
                if let Some(card) = r.target.card() {
                    lit.cards.insert(card);
                    if let Some(pill) = self.pill_for(card, r.target.profile()) {
                        lit.pills.insert((card, pill));
                    }
                    lit.curves.push((End::Row(row), card));
                }
            }
            Focus::EverythingElse => {
                lit.everything_else = true;
                if let Some(card) = ee_card {
                    lit.cards.insert(card);
                    if let Some(pill) = self.pill_for(card, ee_profile) {
                        lit.pills.insert((card, pill));
                    }
                    lit.curves.push((End::EverythingElse, card));
                }
            }
            Focus::Card(card) | Focus::Pill(card, _) => {
                if card >= self.cards.len() {
                    return lit;
                }
                let pill = match focus {
                    Focus::Pill(_, pill) => self.cards[card].pills.get(pill).map(|_| pill),
                    _ => None,
                };
                let names_pill =
                    |profile: Option<&str>| pill.is_none() || self.pill_for(card, profile) == pill;
                lit.cards.insert(card);
                if let Some(pill) = pill {
                    lit.pills.insert((card, pill));
                }
                let mut ends = Vec::new();
                for (i, row) in self.rows.iter().enumerate() {
                    if row.enabled
                        && row.target.card() == Some(card)
                        && names_pill(row.target.profile())
                    {
                        lit.rows.insert(i);
                        ends.push(End::Row(i));
                    }
                }
                if ee_card == Some(card) && names_pill(ee_profile) {
                    lit.everything_else = true;
                    ends.push(End::EverythingElse);
                }
                if ends.len() <= MAX_CURVES {
                    lit.curves = ends.into_iter().map(|end| (end, card)).collect();
                }
            }
        }
        lit
    }
}

/// The callout under the map while a config is in charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callout {
    /// Precedence is the config's: offer PwrRoute's default (2a), with
    /// `describe()`'s warnings.
    Offer { body: String, warnings: Vec<String> },
    /// Just switched on: the confirmation with Undo (2b).
    Done { body: String },
    /// On, settled: the one-line note.
    Settled { body: String },
}

pub const OFFER_TITLE: &str = "Want your own Everything else?";
pub const OFFER_BUTTON: &str = "Let PwrRoute's default take everything else";
pub const DONE_TITLE: &str = "PwrRoute's default now takes everything else.";
pub const CHANGE_IN_RULES: &str = "Change in Rules \u{203a}";

/// The whole screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Overview {
    pub hero: Hero,
    pub setup: Option<Setup>,
    pub tiles: [Tile; 4],
    pub map: Map,
    pub callout: Option<Callout>,
}

/// The config in charge, as the window loaded it.
#[derive(Clone, Copy, Debug)]
pub struct ConfigInputs<'a> {
    /// "~/.finicky.ts".
    pub shown: &'a str,
    /// `None` when it did not load.
    pub description: Option<&'a Description>,
}

/// Everything [`build`] reads.
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// `rules.json` as edited.
    pub rules: &'a RulesFile,
    pub config: Option<ConfigInputs<'a>>,
    pub status: &'a Status,
    /// The scan; empty when it has not run, and then no target is a ghost.
    pub installed: &'a [Installed],
    pub today: Counts,
    pub tally: &'a Tally,
    /// A config found on disk that is not in charge: "~/.finicky.ts".
    pub found_config: Option<&'a str>,
    /// The callout's button was just used and Undo is still offered.
    pub just_overridden: bool,
}

/// "1 rule", "3 rules".
pub fn plural(n: usize, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {one}s")
    }
}

/// "Google Chrome · Personal", or the name alone.
fn display(name: &str, profile: Option<&str>) -> String {
    match profile {
        Some(profile) if !profile.is_empty() => format!("{name} \u{00b7} {profile}"),
        _ => name.to_owned(),
    }
}

/// A destination as the map resolves it, before it is a card.
struct Dest {
    kind: CardKind,
    key: String,
    name: String,
    badge: String,
}

/// The browser a rule names, as the reader knows it: the installed app's
/// name, else the catalog's, else the value as written (a path by its
/// stem). `ghost` when the scan ran and did not find it.
fn resolve(value: &str, installed: &[Installed]) -> Dest {
    let value = value.trim();
    let canonical = Catalog::builtin()
        .find(value)
        .map(|browser| browser.name.clone())
        .unwrap_or_else(|| {
            if value.contains('/') || value.contains('\\') {
                std::path::Path::new(value)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(value)
                    .trim_end_matches(".app")
                    .to_owned()
            } else {
                value.to_owned()
            }
        });
    let found = installed.iter().find(|app| app.is_named(value, &canonical));
    let name = found.map_or(canonical, |app| app.name.clone());
    let ghost = !installed.is_empty() && found.is_none();
    Dest {
        kind: if ghost {
            CardKind::Ghost
        } else {
            CardKind::Browser
        },
        key: name.to_lowercase(),
        badge: badge_for(&name).to_string(),
        name,
    }
}

const FUNCTION_KEY: &str = "\u{0}function";
const NOT_OPENED_KEY: &str = "\u{0}not-opened";

fn function_dest() -> Dest {
    Dest {
        kind: CardKind::Function,
        key: FUNCTION_KEY.into(),
        name: "Decided by JavaScript".into(),
        badge: "\u{0192}".into(),
    }
}

fn not_opened_dest() -> Dest {
    Dest {
        kind: CardKind::NotOpened,
        key: NOT_OPENED_KEY.into(),
        name: crate::widgets::NOT_OPENED.into(),
        badge: crate::widgets::NOT_OPENED_BADGE.into(),
    }
}

/// A config browser spec as a destination and a profile.
fn spec_dest(spec: &BrowserSpec, installed: &[Installed]) -> (Dest, Option<String>) {
    match spec {
        BrowserSpec::Function(_) => (function_dest(), None),
        BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => (not_opened_dest(), None),
        BrowserSpec::Browser(fixed) => (
            resolve(&fixed.name, installed),
            (!fixed.profile.is_empty()).then(|| fixed.profile.clone()),
        ),
    }
}

/// The cards, in order of first use, and the targets that point at them.
#[derive(Default)]
struct Cards {
    dests: Vec<Dest>,
    /// Per card: the profiles named, in order (`None`: the default one).
    pills: Vec<Vec<Option<String>>>,
}

impl Cards {
    fn add(&mut self, dest: Dest, profile: Option<String>) -> Target {
        let card = match self.dests.iter().position(|d| d.key == dest.key) {
            Some(card) => card,
            None => {
                self.dests.push(Dest {
                    kind: dest.kind,
                    key: dest.key.clone(),
                    name: dest.name.clone(),
                    badge: dest.badge.clone(),
                });
                self.pills.push(Vec::new());
                self.dests.len() - 1
            }
        };
        if matches!(dest.kind, CardKind::Browser | CardKind::Ghost) {
            let pills = &mut self.pills[card];
            let named = pills.iter().any(|p| match (p, &profile) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            });
            if !named {
                pills.push(profile.clone());
            }
        }
        match dest.kind {
            CardKind::Browser => Target::Browser {
                card,
                label: profile.clone().unwrap_or_else(|| dest.name.clone()),
                badge: dest.badge,
                profile,
            },
            CardKind::Ghost => Target::Ghost {
                card,
                badge: dest.badge,
                name: dest.name,
            },
            CardKind::Function => Target::Function { card },
            CardKind::NotOpened => Target::NotOpened { card },
        }
    }
}

/// A `rules.json` rule's match column.
fn rule_match(rule: &Rule) -> RowMatch {
    let patterns: Vec<&String> = rule
        .patterns
        .iter()
        .filter(|p| !p.trim().is_empty())
        .collect();
    let openers: Vec<&String> = rule
        .opener
        .iter()
        .filter(|o| !o.trim().is_empty())
        .collect();
    // "*" from an app reads as the app: the pattern says nothing more.
    let every_link = patterns.iter().all(|p| p.trim() == "*");
    if let Some(first) = patterns
        .first()
        .filter(|_| openers.is_empty() || !every_link)
    {
        RowMatch::Pattern {
            regex: first.len() > 1 && first.starts_with('/') && first.ends_with('/'),
            text: (*first).clone(),
            more: patterns.len() - 1,
        }
    } else if let Some(first) = openers.first() {
        let name = crate::default_browser::browser_name(first);
        // "Slack:com.tinyspeck.slackmacgap" names Slack.
        let name = name.split(':').next().unwrap_or(&name).to_owned();
        RowMatch::Opener {
            badge: badge_for(&name).to_string(),
            name,
            more: openers.len() - 1,
        }
    } else {
        RowMatch::Any
    }
}

/// A config handler's match column.
fn handler_match(matchers: &[MatcherSpec]) -> RowMatch {
    let more = matchers.len().saturating_sub(1);
    match matchers.first() {
        None => RowMatch::Any,
        Some(MatcherSpec::Wildcard(text)) => RowMatch::Pattern {
            text: text.clone(),
            regex: false,
            more,
        },
        Some(MatcherSpec::Regex(text)) => RowMatch::Pattern {
            text: text.clone(),
            regex: true,
            more,
        },
        Some(MatcherSpec::Function(function)) => RowMatch::Function {
            name: (!function.name.trim().is_empty()).then(|| function.name.clone()),
            more,
        },
        Some(MatcherSpec::Other(kind)) => RowMatch::Other {
            text: kind.clone(),
            more,
        },
    }
}

/// `describe()`'s reasons to think twice before PwrRoute's default takes
/// everything else (Rules 5c), in the banner's words.
pub fn override_warnings(description: &Description) -> Vec<String> {
    let mut warnings = Vec::new();
    match &description.default_browser {
        BrowserSpec::Function(_) => {
            warnings.push("Its defaultBrowser function will no longer run.".to_owned());
        }
        BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => {
            warnings.push("Unmatched links will now open instead of being dropped.".to_owned());
        }
        BrowserSpec::Browser(_) => {}
    }
    if let Some(number) = crate::settings::configuration_host::catch_all(&description.handlers) {
        warnings.push(crate::settings::configuration_host::catch_all_warning(
            number,
        ));
    }
    warnings
}

/// While a reload failed: a change to the order is written, but the router
/// takes it only with the config.
pub fn stale_note(shown: &str) -> String {
    format!("A change here applies once {shown} reloads.")
}

/// Builds the screen.
pub fn build(inputs: &Inputs<'_>) -> Overview {
    let status = inputs.status;
    let state = status.state();
    let rules = inputs.rules;
    let installed = inputs.installed;
    let config = inputs.config;
    // The config's trouble, as the router reports it. A config this window
    // could not read while the router reports none is the router's copy
    // routing: the map says so rather than that links skip it.
    let trouble = config.and_then(|config| {
        status.config_error.clone().or_else(|| {
            config
                .description
                .is_none()
                .then(|| ConfigTrouble::ReloadFailed("It did not load here.".to_owned()))
        })
    });
    // Never loaded: neither the config nor rules.json runs, and every link
    // goes to the fallback browser.
    let not_loaded = matches!(trouble, Some(ConfigTrouble::NotLoaded(_)));
    // A failed reload: the router routes with the files as they last
    // loaded, config and rules.json together, so neither the order nor a
    // rule's number on screen need be the one it uses (an edit since then
    // waits for the reload). Counts by number are not shown, and the
    // headers do not claim an order.
    let stale = matches!(trouble, Some(ConfigTrouble::ReloadFailed(_)));
    let description = config
        .and_then(|config| config.description)
        .filter(|_| !not_loaded);
    let rules_first = rules.precedence == Precedence::PwrRoute;
    // A default PwrRoute can open: not empty, and not PwrRoute itself, which
    // the router turns into its fallback.
    let has_default = crate::config_view::has_own_default(&rules.default_browser);
    let counts = state != State::NotDefault;

    // Everything else: PwrRoute's default, or the config's while it owns it.
    // As the engine decides: the config keeps its default unless the rules
    // run first with a default of their own.
    let config_owns_default = description.is_some() && !rules.owns_default();
    let mut cards = Cards::default();
    let mut rows: Vec<Row> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    let count_of = |key: HitKey| {
        let n = inputs.tally.rule(key);
        (counts && n > 0).then_some(n)
    };

    let rules_json_name = if has_default {
        rules.default_browser.clone()
    } else {
        FALLBACK_BROWSER.to_owned()
    };
    let pwrroute_dest = || {
        let dest = resolve(&rules_json_name, installed);
        let profile = (has_default && !rules.default_profile.is_empty())
            .then(|| rules.default_profile.clone());
        (dest, profile)
    };
    let pwrroute_display = {
        let (dest, profile) = pwrroute_dest();
        display(&dest.name, profile.as_deref())
    };

    // The config's handlers and the visual rules, in evaluation order.
    let config_section = |rows: &mut Vec<Row>, items: &mut Vec<Item>, cards: &mut Cards| {
        let Some(config) = config else {
            return;
        };
        let header = if not_loaded {
            format!("{} \u{00b7} not running", config.shown)
        } else if stale {
            format!("{} \u{00b7} as last loaded", config.shown)
        } else if rules_first {
            format!("{} handlers \u{00b7} run after", config.shown)
        } else {
            format!("{} \u{00b7} runs first", config.shown)
        };
        let Some(description) = description else {
            items.push(Item::Header {
                label: header,
                note: String::new(),
            });
            items.push(Item::Error {
                title: if not_loaded {
                    format!("{} didn't load: nothing runs until it does", config.shown)
                } else {
                    format!(
                        "{} didn't reload: the last version that loaded still runs",
                        config.shown
                    )
                },
                detail: trouble
                    .as_ref()
                    .map(|trouble| trouble.message().to_owned())
                    .unwrap_or_default(),
            });
            return;
        };
        items.push(Item::Header {
            label: header,
            note: format!(
                "{} \u{00b7} read-only",
                plural(description.handlers.len(), "handler")
            ),
        });
        for (index, handler) in description.handlers.iter().enumerate() {
            let (dest, profile) = spec_dest(&handler.browser, installed);
            let target = cards.add(dest, profile);
            let ghost = matches!(target, Target::Ghost { .. });
            items.push(Item::Row(rows.len()));
            rows.push(Row {
                rank: rows.len() + 1,
                source: RowSource::Handler(index),
                matcher: handler_match(&handler.matchers),
                target,
                enabled: true,
                idle: false,
                count: (!ghost && !stale)
                    .then(|| count_of(HitKey::Handler(index)))
                    .flatten(),
                tooltip: format!("config handler {} of {}", index + 1, config.shown),
            });
        }
    };
    let rules_section = |rows: &mut Vec<Row>, items: &mut Vec<Item>, cards: &mut Cards| {
        if rules.rules.is_empty() {
            return;
        }
        if config.is_some() {
            items.push(Item::Header {
                label: if not_loaded {
                    // The router's fallback engine holds no rules.
                    "Your rules \u{00b7} not running".to_owned()
                } else if stale {
                    "Your rules \u{00b7} as last loaded".to_owned()
                } else if rules_first {
                    "Your rules \u{00b7} run first".to_owned()
                } else {
                    "Your rules \u{00b7} run after".to_owned()
                },
                note: plural(rules.rules.len(), "rule"),
            });
        }
        for (index, rule) in rules.rules.iter().enumerate() {
            let target = if rule.browser.trim().is_empty() {
                Target::Unset
            } else {
                let profile = (!rule.profile.is_empty()).then(|| rule.profile.clone());
                cards.add(resolve(&rule.browser, installed), profile)
            };
            let ghost = matches!(target, Target::Ghost { .. });
            items.push(Item::Row(rows.len()));
            rows.push(Row {
                rank: rows.len() + 1,
                source: RowSource::Rule(index),
                matcher: rule_match(rule),
                target,
                enabled: rule.enabled,
                idle: not_loaded,
                count: (!ghost && rule.enabled && !not_loaded && !stale)
                    .then(|| count_of(HitKey::Rule(index)))
                    .flatten(),
                tooltip: format!("rule {}", index + 1),
            });
        }
    };

    // The Everything else card comes first; the rest follow first use.
    let (ee_dest, ee_profile) = match description {
        Some(description) if config_owns_default => {
            spec_dest(&description.default_browser, installed)
        }
        _ => pwrroute_dest(),
    };
    let ee_target = cards.add(ee_dest, ee_profile);
    if rules_first {
        rules_section(&mut rows, &mut items, &mut cards);
        config_section(&mut rows, &mut items, &mut cards);
    } else {
        config_section(&mut rows, &mut items, &mut cards);
        rules_section(&mut rows, &mut items, &mut cards);
    }
    let shown = config.map_or("", |config| config.shown);
    let ee_owner = if config_owns_default {
        format!("{shown}'s default")
    } else if !has_default {
        format!("{pwrroute_display} until you choose")
    } else {
        "PwrRoute's default".to_owned()
    };
    let everything_else = EverythingElse {
        owner: ee_owner,
        target: ee_target.clone(),
        count: (counts && inputs.today.unrouted > 0).then_some(inputs.today.unrouted),
    };

    // Each card's rules, links and share.
    let mut links = vec![0u64; cards.dests.len()];
    let mut function_links = 0;
    for (name, _, n) in inputs.tally.browsers() {
        let dest = resolve(name, installed);
        if let Some(card) = cards.dests.iter().position(|d| d.key == dest.key) {
            links[card] += n;
        }
    }
    for row in &rows {
        if let Target::Function { .. } = row.target {
            function_links += row.count.unwrap_or(0);
        }
    }
    let ee_card = ee_target.card();
    let total = inputs.today.routed;
    let alone = cards.dests.len() == 1;
    let ribbon = if config_owns_default {
        format!("Everything else \u{00b7} {shown}")
    } else if has_default {
        "Everything else \u{00b7} PwrRoute".to_owned()
    } else {
        "Everything else".to_owned()
    };
    let built: Vec<Card> = cards
        .dests
        .iter()
        .enumerate()
        .map(|(card, dest)| {
            let feeding = rows
                .iter()
                .filter(|row| row.enabled && row.target.card() == Some(card))
                .count();
            let idle = rows
                .iter()
                .any(|row| row.enabled && row.target.card() == Some(card))
                && rows
                    .iter()
                    .filter(|row| row.enabled && row.target.card() == Some(card))
                    .all(|row| row.idle);
            let is_ee = ee_card == Some(card);
            let n = match dest.kind {
                CardKind::Function => function_links,
                CardKind::Ghost => 0,
                _ => links[card],
            };
            let card_links = (counts && n > 0).then_some(n);
            let today = card_links.map(|n| format!("{} links today", thousands(n)));
            let rules_part = if feeding == 0 && is_ee {
                "Everything else".to_owned()
            } else {
                plural(feeding, "rule")
            };
            let meta = match dest.kind {
                CardKind::Browser => std::iter::once(rules_part)
                    .chain(today.clone())
                    .collect::<Vec<_>>(),
                CardKind::Function => std::iter::once(rules_part)
                    .chain(["picks a browser per link".to_owned()])
                    .chain(today.clone())
                    .collect(),
                CardKind::NotOpened => std::iter::once(rules_part)
                    .chain(["these links are not opened".to_owned()])
                    .chain(today.clone())
                    .collect(),
                CardKind::Ghost => std::iter::once(rules_part)
                    .chain(["its links fall through to Everything else".to_owned()])
                    .collect(),
            }
            .join(" \u{00b7} ");
            let share =
                (dest.kind == CardKind::Browser && counts && total > 0 && !alone).then(|| {
                    let fraction = (n as f32 / total as f32).clamp(0.0, 1.0);
                    let percent = fraction * 100.0;
                    Share {
                        label: if n > 0 && percent < 1.0 {
                            "<1%".to_owned()
                        } else {
                            format!("{}%", percent.round() as u64)
                        },
                        fraction,
                    }
                });
            let mut pills: Vec<Pill> = cards.pills[card]
                .iter()
                .map(|profile| Pill {
                    profile: profile.clone(),
                    everything_else: is_ee
                        && match (profile, ee_target.profile()) {
                            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                            (None, None) => true,
                            _ => false,
                        },
                })
                .collect();
            // The Everything else pill goes last, as the design draws it.
            pills.sort_by_key(|pill| pill.everything_else);
            // Past MAX_PILLS the rest fold into "+N", but the Everything
            // else pill stays: it is the one the ribbon points at.
            let more_pills = pills.len().saturating_sub(MAX_PILLS);
            if more_pills > 0 {
                let everything_else = pills.last().filter(|pill| pill.everything_else).cloned();
                pills.truncate(MAX_PILLS);
                if let Some(pill) = everything_else {
                    pills[MAX_PILLS - 1] = pill;
                }
            }
            let alone = (alone && is_ee).then(|| {
                let shown = match &ee_target {
                    Target::Browser { profile, .. } => display(&dest.name, profile.as_deref()),
                    _ => dest.name.clone(),
                };
                (format!("Everything opens in {shown}"), today.clone())
            });
            Card {
                kind: dest.kind,
                badge: dest.badge.clone(),
                name: dest.name.clone(),
                key: dest.key.clone(),
                pills,
                more_pills,
                rules: feeding,
                meta,
                links: card_links,
                share,
                ribbon: is_ee.then(|| ribbon.clone()),
                alone,
                idle: idle && !is_ee,
            }
        })
        .collect();

    // Ghost cards go last, after the browsers and the special cards.
    let (built, remap) = order_cards(built);
    for row in &mut rows {
        remap_target(&mut row.target, &remap);
    }
    let mut everything_else = everything_else;
    remap_target(&mut everything_else.target, &remap);

    let used: Vec<&str> = built.iter().map(|card| card.key.as_str()).collect();
    let not_used: Vec<&str> = installed
        .iter()
        .filter(|app| {
            !app.hidden
                && !app.name.eq_ignore_ascii_case("PwrRoute")
                && !used.contains(&app.name.to_lowercase().as_str())
        })
        .map(|app| app.name.as_str())
        .collect();

    let rewrites = if not_loaded {
        0
    } else {
        description.map_or(0, |d| d.rewrites.len()) + rules.rewrites.len()
    };
    let nothing = config.is_none() && rules.rules.is_empty();
    let empty = nothing.then(|| Empty {
        title: format!("Every link opens in {pwrroute_display}."),
        import: inputs
            .found_config
            .map(|shown| format!("Import from {shown}\u{2026}")),
    });

    let map = Map {
        caption: match state {
            State::Paused => Caption::WhenResumes,
            State::NotDefault => Caption::WhatWillHappen,
            _ => Caption::InOrder,
        },
        counts,
        dimmed: state == State::Paused,
        not_used: (!not_used.is_empty()).then(|| format!("Not used: {}", not_used.join(", "))),
        rewrites: (rewrites > 0).then(|| {
            if rewrites == 1 {
                "1 rewrite runs on every link first".to_owned()
            } else {
                format!("{rewrites} rewrites run on every link first")
            }
        }),
        items,
        rows,
        everything_else,
        cards: built,
        empty,
    };

    // The setup card: what is left.
    let other_default = match &status.default {
        DefaultState::Other(name) => Some(name.to_string()),
        _ => None,
    };
    let setup = (!has_default || other_default.is_some()).then(|| {
        let fallback = resolve(FALLBACK_BROWSER, installed).name;
        Setup {
            everything_else: SetupStep {
                title: "Choose where everything else goes",
                done: has_default,
                body: (!has_default).then(|| {
                    if config_owns_default {
                        // The config's default takes them; this browser is
                        // the router's fallback (First run 4e).
                        format!(
                            "{shown}'s defaultBrowser handles links its handlers don't \
                             claim. The browser below still catches config errors, \
                             browsers that can't open, and every link while paused."
                        )
                    } else {
                        format!(
                            "Links no rule claims open here. Until you choose, that's {fallback}."
                        )
                    }
                }),
                result: has_default.then(|| {
                    let (dest, _) = pwrroute_dest();
                    (dest.badge, pwrroute_display.clone())
                }),
            },
            default_browser: SetupStep {
                title: "Make PwrRoute the default browser",
                // Done only when the OS says so: an unknown default is not.
                done: status.default == DefaultState::PwrRoute,
                body: Some(match &status.default {
                    DefaultState::Other(name) => format!(
                        "macOS will ask you to confirm. Until then, links keep opening in \
                         {name} and skip your rules."
                    ),
                    DefaultState::Unknown => "macOS will ask you to confirm. PwrRoute can't \
                                              tell which browser is the default now."
                        .to_owned(),
                    DefaultState::PwrRoute => "PwrRoute is the default browser.".to_owned(),
                }),
                result: None,
            },
        }
    });

    // The hero.
    let source = match config {
        Some(config) if rules.rules.is_empty() => config.shown.to_owned(),
        Some(config) if rules_first => {
            format!("{} + {}", plural(rules.rules.len(), "rule"), config.shown)
        }
        Some(config) => format!("{} + {}", config.shown, plural(rules.rules.len(), "rule")),
        None => format!("rules.json \u{00b7} {}", plural(rules.rules.len(), "rule")),
    };
    let quiet_action = |action: Action| (setup.is_none()).then_some(action);
    let hero = match state {
        // The router's fallback engine routes nothing: every link opens in
        // the fallback browser (design 3c drew rules still running, which
        // the router does not do).
        State::ConfigError => Hero {
            state,
            tone: Tone::Danger,
            sentence: format!("{shown} didn't load."),
            detail: vec![format!(
                "Every link opens in {pwrroute_display} until it loads."
            )],
            action: Some(Action::ShowError),
        },
        // The last version that loaded still runs; the error is here, and
        // on Rules only when this window could not read the file either.
        State::ConfigStale => Hero {
            state,
            tone: Tone::Warn,
            sentence: format!("{shown} didn't reload."),
            detail: std::iter::once(
                "Links still route with the version that last loaded".to_owned(),
            )
            .chain(
                status
                    .config_error
                    .as_ref()
                    .map(|trouble| trouble.message().to_owned()),
            )
            .collect(),
            action: description.is_none().then_some(Action::ShowError),
        },
        State::Paused => Hero {
            state,
            tone: Tone::Warn,
            sentence: "Routing is paused.".to_owned(),
            detail: vec![format!(
                "Every link opens in {} until you resume. Rewrites still run.",
                status.paused.as_ref().map_or("", |p| p.display.as_str())
            )],
            action: Some(Action::Resume),
        },
        State::NotDefault => Hero {
            state,
            tone: Tone::Neutral,
            sentence: "Links aren't reaching PwrRoute yet.".to_owned(),
            detail: vec![format!(
                "{} is your default browser, so links open there and skip your rules.",
                other_default.clone().unwrap_or_default()
            )],
            action: quiet_action(Action::MakeDefault),
        },
        State::Unknown => Hero {
            state,
            tone: Tone::Neutral,
            sentence: "PwrRoute can't tell which browser is the default.".to_owned(),
            detail: Vec::new(),
            action: Some(Action::OpenBrowsersSettings),
        },
        State::Routing if nothing => Hero {
            state,
            tone: Tone::Accent,
            sentence: "PwrRoute is catching links.".to_owned(),
            detail: vec![format!(
                "No rules yet, so every link opens in {pwrroute_display}."
            )],
            action: quiet_action(Action::TestLink),
        },
        State::Routing => Hero {
            state,
            tone: Tone::Accent,
            sentence: "Links are routing.".to_owned(),
            detail: vec!["PwrRoute is your default browser".to_owned(), source],
            action: quiet_action(Action::TestLink),
        },
    };

    let tile = |label, count, filter| Tile {
        label,
        count,
        filter,
        danger: false,
    };
    let today = inputs.today;
    let tiles = [
        tile("Links today", today.routed, Filter::All),
        tile("Took Everything else", today.unrouted, Filter::Unrouted),
        tile("Rewritten", today.rewritten, Filter::Rewritten),
        Tile {
            danger: today.failed > 0,
            ..tile("Failed", today.failed, Filter::Failed)
        },
    ];

    // The callout: only while a config that loaded is in charge, and
    // PwrRoute has a default of its own to offer.
    let with_stale_note = |body: String| {
        if stale {
            format!("{body} {}", stale_note(shown))
        } else {
            body
        }
    };
    let callout = description.filter(|_| has_default).map(|description| {
        let config_default = match &description.default_browser {
            BrowserSpec::Browser(fixed) if fixed.app_type != AppType::None => {
                Some(resolve(&fixed.name, installed).name)
            }
            _ => None,
        };
        let theirs = match &config_default {
            Some(name) => format!("{shown}'s defaultBrowser, {name},"),
            None => format!("{shown}'s defaultBrowser"),
        };
        if !rules_first {
            let instead = match (&config_default, &description.default_browser) {
                (Some(name), _) => format!("instead of {name}"),
                (None, BrowserSpec::Function(_)) => {
                    "instead of its defaultBrowser function".to_owned()
                }
                (None, _) => "instead of leaving them unopened".to_owned(),
            };
            Callout::Offer {
                body: format!(
                    "Your rules would run first, then {shown}'s handlers, and \
                     {pwrroute_display} would take what is left {instead}."
                ),
                warnings: override_warnings(description)
                    .into_iter()
                    .chain(stale.then(|| stale_note(shown)))
                    .collect(),
            }
        } else if inputs.just_overridden {
            Callout::Done {
                body: with_stale_note(format!("{theirs} isn't used while this is on.")),
            }
        } else {
            Callout::Settled {
                body: with_stale_note(format!(
                    "{theirs} isn't used while PwrRoute's default takes everything else."
                )),
            }
        }
    });

    Overview {
        hero,
        setup,
        tiles,
        map,
        callout,
    }
}

/// Everything else's card first (built first), then the browser cards by
/// how many rules feed them and then today's links, then the special
/// cards, then ghosts; and where each old index went. Ties keep the order
/// of first use.
fn order_cards(cards: Vec<Card>) -> (Vec<Card>, Vec<usize>) {
    use std::cmp::Reverse;
    let rank = |kind: CardKind| match kind {
        CardKind::Browser => 0,
        CardKind::Function | CardKind::NotOpened => 1,
        CardKind::Ghost => 2,
    };
    let mut order: Vec<usize> = (0..cards.len()).collect();
    // The Everything else card keeps its place whatever it is.
    order.sort_by_key(|&i| {
        let card = &cards[i];
        (
            if i == 0 { 0 } else { 1 + rank(card.kind) },
            Reverse(card.rules),
            Reverse(card.links.unwrap_or(0)),
            i,
        )
    });
    let mut remap = vec![0; cards.len()];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    let mut slots: Vec<Option<Card>> = cards.into_iter().map(Some).collect();
    let ordered = order
        .iter()
        .map(|&old| slots[old].take().expect("each card once"))
        .collect();
    (ordered, remap)
}

fn remap_target(target: &mut Target, remap: &[usize]) {
    match target {
        Target::Browser { card, .. }
        | Target::Function { card }
        | Target::NotOpened { card }
        | Target::Ghost { card, .. } => *card = remap[*card],
        Target::Unset => {}
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::SystemTime;

    use pwrroute_engine::{
        DefaultHost, Engine, FixedBrowser, FunctionSpec, HandlerRule, Limits, RulesFile,
    };
    use pwrroute_router::{DispatchRecord, Matched, Owner};

    use super::*;
    use crate::activity::Feed;
    use crate::fixtures;

    /// Everything [`build`] reads, owned.
    struct World {
        rules: RulesFile,
        /// The config in charge: `Some(None)` when it did not load.
        config: Option<Option<Description>>,
        status: Status,
        installed: Vec<Installed>,
        day: Vec<DispatchRecord>,
        found_config: Option<&'static str>,
        just_overridden: bool,
    }

    impl World {
        fn new(rules: RulesFile, day: Vec<DispatchRecord>) -> Self {
            Self {
                rules,
                config: None,
                status: Status::routing(),
                installed: fixtures::overview_browsers(),
                day,
                found_config: None,
                just_overridden: false,
            }
        }

        /// 1a: the visual rules, routing.
        fn rules() -> Self {
            Self::new(fixtures::overview_rules(), fixtures::overview_rules_day())
        }

        /// 2a: the config in charge, its precedence.
        fn config() -> Self {
            Self {
                config: Some(Some(description())),
                ..Self::new(
                    fixtures::overview_config_rules(),
                    fixtures::overview_config_day(),
                )
            }
        }

        fn build(&self) -> Overview {
            let feed = Feed::from_records(self.day.clone());
            let now = SystemTime::now();
            let tally = feed.tally(now);
            build(&Inputs {
                rules: &self.rules,
                config: self.config.as_ref().map(|description| ConfigInputs {
                    shown: "~/.finicky.ts",
                    description: description.as_ref(),
                }),
                status: &self.status,
                installed: &self.installed,
                today: feed.today(now),
                tally: &tally,
                found_config: self.found_config,
                just_overridden: self.just_overridden,
            })
        }
    }

    fn description() -> Description {
        Engine::load(
            &fixtures::overview_config(),
            Rc::new(DefaultHost),
            Limits::default(),
        )
        .expect("the Overview config loads")
        .describe()
        .expect("it describes")
        .expect("it has a description")
    }

    fn names(map: &Map) -> Vec<&str> {
        map.cards.iter().map(|card| card.name.as_str()).collect()
    }

    fn headers(map: &Map) -> Vec<(&str, &str)> {
        map.items
            .iter()
            .filter_map(|item| match item {
                Item::Header { label, note } => Some((label.as_str(), note.as_str())),
                _ => None,
            })
            .collect()
    }

    fn browser(name: &str) -> BrowserSpec {
        BrowserSpec::Browser(FixedBrowser {
            name: name.into(),
            app_type: if name.is_empty() {
                AppType::None
            } else {
                AppType::AppName
            },
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
        })
    }

    fn function(name: &str) -> FunctionSpec {
        FunctionSpec {
            name: name.into(),
            source: None,
        }
    }

    fn rule(pattern: &str, browser: &str, profile: &str) -> Rule {
        Rule {
            patterns: vec![pattern.into()],
            browser: browser.into(),
            profile: profile.into(),
            ..Rule::default()
        }
    }

    #[test]
    fn plural_counts_one_and_many() {
        assert_eq!(plural(0, "rule"), "0 rules");
        assert_eq!(plural(1, "rule"), "1 rule");
        assert_eq!(plural(6, "handler"), "6 handlers");
    }

    #[test]
    fn the_tally_counts_each_rule_and_browser_today() {
        let feed = Feed::from_records(fixtures::overview_rules_day());
        let tally = feed.tally(SystemTime::now());
        assert_eq!(tally.rule(HitKey::Rule(0)), 214);
        assert_eq!(tally.rule(HitKey::Rule(3)), 402);
        assert_eq!(tally.rule(HitKey::Rule(5)), 0);
        assert_eq!(tally.rule(HitKey::Default(Some(Owner::PwrRoute))), 642);
        let work = tally
            .browsers()
            .find(|(name, profile, _)| *name == "Google Chrome" && *profile == "Work")
            .map(|(_, _, n)| n);
        assert_eq!(work, Some(302));
    }

    /// Records name the list that routed each link (`Matched::Rule` for
    /// `rules.json`, `Matched::Handler` for the config, the default by its
    /// owner), so each row counts its own links under either order, and
    /// the tooltip names it as Activity does.
    #[test]
    fn rows_count_rules_json_rules_and_config_handlers_apart() {
        for precedence in [Precedence::Config, Precedence::PwrRoute] {
            let mut world = World::config();
            world.rules.precedence = precedence;
            let feed = Feed::from_records(world.day.clone());
            let tally = feed.tally(SystemTime::now());
            assert_eq!(tally.rule(HitKey::Handler(0)), 57);
            assert_eq!(tally.rule(HitKey::Rule(0)), 402);
            assert_eq!(tally.rule(HitKey::Default(Some(Owner::Config))), 627);
            assert_eq!(tally.rule(HitKey::Default(Some(Owner::PwrRoute))), 0);
            let overview = world.build();
            let count = |tooltip: &str| {
                overview
                    .map
                    .rows
                    .iter()
                    .find(|row| row.tooltip == tooltip)
                    .unwrap_or_else(|| panic!("no row {tooltip}"))
                    .count
            };
            assert_eq!(count("config handler 1 of ~/.finicky.ts"), Some(57));
            assert_eq!(count("config handler 3 of ~/.finicky.ts"), Some(214));
            assert_eq!(count("rule 1"), Some(402));
            assert_eq!(count("rule 2"), Some(12));
            assert_eq!(count("rule 3"), Some(3));
        }
    }

    #[test]
    fn routing_1a_says_links_are_routing_with_todays_counts() {
        let overview = World::rules().build();
        assert_eq!(overview.hero.state, State::Routing);
        assert_eq!(overview.hero.tone, Tone::Accent);
        assert_eq!(overview.hero.sentence, "Links are routing.");
        assert_eq!(
            overview.hero.detail,
            [
                "PwrRoute is your default browser",
                "rules.json \u{b7} 6 rules"
            ]
        );
        assert_eq!(overview.hero.action, Some(Action::TestLink));
        assert!(overview.setup.is_none(), "nothing left to set up");
        assert!(overview.callout.is_none());

        let counts: Vec<u64> = overview.tiles.iter().map(|tile| tile.count).collect();
        assert_eq!(counts, [1412, 642, 1108, 0]);
        let filters: Vec<Filter> = overview.tiles.iter().map(|tile| tile.filter).collect();
        assert_eq!(
            filters,
            [
                Filter::All,
                Filter::Unrouted,
                Filter::Rewritten,
                Filter::Failed
            ]
        );
        assert!(!overview.tiles[3].danger);

        let map = &overview.map;
        assert_eq!(map.caption, Caption::InOrder);
        assert!(map.counts && !map.dimmed);
        assert_eq!(
            map.rewrites.as_deref(),
            Some("3 rewrites run on every link first")
        );
        assert!(headers(map).is_empty(), "one list needs no section headers");
        assert_eq!(map.rows.len(), 6);
        assert_eq!(
            map.rows[0].matcher,
            RowMatch::Pattern {
                text: "*.docs.example.com/*".into(),
                regex: false,
                more: 1,
            }
        );
        assert_eq!(map.rows[0].count, Some(214));
        assert_eq!(map.rows[0].tooltip, "rule 1");
        assert_eq!(
            map.rows[2].matcher,
            RowMatch::Opener {
                badge: badge_for("Slack").to_string(),
                name: "Slack".into(),
                more: 0,
            },
            "\"*\" from an app reads as the app"
        );
        assert!(!map.rows[5].enabled);
        assert_eq!(map.rows[5].count, None, "a rule that is off counts nothing");

        assert_eq!(names(map), ["Google Chrome", "Firefox", "Arc"]);
        assert_eq!(
            map.not_used.as_deref(),
            Some("Not used: Safari, Microsoft Edge")
        );
        let chrome = &map.cards[0];
        assert_eq!(
            chrome.ribbon.as_deref(),
            Some("Everything else \u{b7} PwrRoute")
        );
        let pills: Vec<(&str, bool)> = chrome
            .pills
            .iter()
            .map(|pill| (pill.label(), pill.everything_else))
            .collect();
        assert_eq!(pills, [("Work", false), ("Personal", true)]);
        assert_eq!(chrome.meta, "2 rules \u{b7} 944 links today");
        assert_eq!(chrome.share.as_ref().map(|s| s.label.as_str()), Some("67%"));
        assert_eq!(map.cards[2].meta, "1 rule \u{b7} 402 links today");

        assert_eq!(map.everything_else.owner, "PwrRoute's default");
        assert_eq!(map.everything_else.count, Some(642));
        assert_eq!(map.everything_else.target.card(), Some(0));
    }

    #[test]
    fn hovering_a_row_lights_its_card_pill_and_one_curve() {
        let map = World::rules().build().map;
        let lit = map.lit(Focus::Row(1));
        assert_eq!(lit.rows, BTreeSet::from([1]));
        assert!(lit.card(0) && !lit.card(1));
        assert!(lit.pill(0, 0), "rule 2 sends to Work");
        assert!(!lit.everything_else);
        assert_eq!(lit.curves, [(End::Row(1), 0)]);
    }

    #[test]
    fn hovering_a_card_lights_every_enabled_rule_and_everything_else() {
        let map = World::rules().build().map;
        let lit = map.lit(Focus::Card(0));
        assert_eq!(lit.rows, BTreeSet::from([0, 1]), "rule 6 is off");
        assert!(lit.everything_else);
        assert_eq!(
            lit.curves,
            [(End::Row(0), 0), (End::Row(1), 0), (End::EverythingElse, 0)]
        );

        let work = map.lit(Focus::Pill(0, 0));
        assert_eq!(work.rows, BTreeSet::from([0, 1]));
        assert!(!work.everything_else);
        assert!(work.pill(0, 0) && !work.pill(0, 1));

        let personal = map.lit(Focus::Pill(0, 1));
        assert!(personal.rows.is_empty());
        assert!(personal.everything_else);
        assert_eq!(personal.curves, [(End::EverythingElse, 0)]);

        let everything_else = map.lit(Focus::EverythingElse);
        assert!(everything_else.pill(0, 1));
        assert_eq!(everything_else.curves, [(End::EverythingElse, 0)]);
    }

    #[test]
    fn a_card_fed_by_more_than_six_rows_lights_them_without_curves() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: (0..7)
                .map(|i| rule(&format!("site{i}.example.com/*"), "Firefox", ""))
                .collect(),
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        let firefox = names(&map).iter().position(|n| *n == "Firefox").unwrap();
        let lit = map.lit(Focus::Card(firefox));
        assert_eq!(lit.rows.len(), 7);
        assert!(lit.curves.is_empty());
        assert_eq!(map.lit(Focus::Row(6)).curves.len(), 1);
    }

    #[test]
    fn the_config_in_charge_2a_runs_first_and_offers_pwrroutes_default() {
        let overview = World::config().build();
        assert_eq!(
            overview.hero.detail,
            [
                "PwrRoute is your default browser",
                "~/.finicky.ts + 3 rules"
            ]
        );
        let map = &overview.map;
        assert_eq!(
            headers(map),
            [
                (
                    "~/.finicky.ts \u{b7} runs first",
                    "5 handlers \u{b7} read-only"
                ),
                ("Your rules \u{b7} run after", "3 rules"),
            ]
        );
        assert_eq!(
            map.rewrites.as_deref(),
            Some("3 rewrites run on every link first")
        );
        assert_eq!(map.rows.len(), 8);
        assert_eq!(map.rows[0].source, RowSource::Handler(0));
        assert_eq!(map.rows[0].tooltip, "config handler 1 of ~/.finicky.ts");
        assert_eq!(
            map.rows[0].matcher,
            RowMatch::Function {
                name: Some("isWorkplace".into()),
                more: 0,
            }
        );
        assert_eq!(map.rows[5].source, RowSource::Rule(0));
        assert_eq!(map.rows[5].rank, 6);

        assert_eq!(map.everything_else.owner, "~/.finicky.ts's default");
        // Everything else's card, then browsers by rules and today's links.
        assert_eq!(
            names(map),
            [
                "Safari",
                "Google Chrome",
                "Arc",
                "Firefox",
                "Decided by JavaScript",
                "Orion"
            ]
        );
        assert_eq!(map.cards[1].meta, "3 rules \u{b7} 314 links today");
        assert_eq!(
            map.cards[0].ribbon.as_deref(),
            Some("Everything else \u{b7} ~/.finicky.ts")
        );
        let function = map
            .cards
            .iter()
            .find(|card| card.kind == CardKind::Function)
            .expect("a function card");
        assert_eq!(function.name, "Decided by JavaScript");
        assert_eq!(
            function.meta,
            "1 rule \u{b7} picks a browser per link \u{b7} 9 links today"
        );
        let ghost = map.cards.last().expect("cards");
        assert_eq!(
            (ghost.kind, ghost.name.as_str()),
            (CardKind::Ghost, "Orion")
        );
        assert_eq!(
            ghost.meta,
            "1 rule \u{b7} its links fall through to Everything else"
        );
        assert!(matches!(map.rows[4].target, Target::Ghost { .. }));

        match overview.callout {
            Some(Callout::Offer { body, warnings }) => {
                assert!(
                    body.contains(
                        "Google Chrome \u{b7} Personal would take what is left instead of Safari"
                    ),
                    "{body}"
                );
                assert!(warnings.is_empty());
            }
            other => panic!("expected the offer, got {other:?}"),
        }
    }

    #[test]
    fn pwrroutes_default_2b_puts_rules_first_with_undo_then_settles() {
        let mut world = World::config();
        world.rules.precedence = Precedence::PwrRoute;
        world.just_overridden = true;
        world.day = fixtures::overview_override_day();
        let overview = world.build();
        let chrome = &overview.map.cards[0];
        assert_eq!(chrome.meta, "3 rules \u{b7} 941 links today");
        assert_eq!(chrome.share.as_ref().map(|s| s.label.as_str()), Some("67%"));
        let safari = overview
            .map
            .cards
            .iter()
            .find(|card| card.name == "Safari")
            .expect("Safari's card");
        assert_eq!(safari.meta, "1 rule \u{b7} 3 links today");
        assert_eq!(safari.share.as_ref().map(|s| s.label.as_str()), Some("<1%"));
        assert_eq!(overview.map.everything_else.count, Some(627));
        assert_eq!(
            headers(&overview.map),
            [
                ("Your rules \u{b7} run first", "3 rules"),
                (
                    "~/.finicky.ts handlers \u{b7} run after",
                    "5 handlers \u{b7} read-only"
                ),
            ]
        );
        assert_eq!(overview.hero.detail[1], "3 rules + ~/.finicky.ts");
        assert_eq!(overview.map.everything_else.owner, "PwrRoute's default");
        assert_eq!(overview.map.cards[0].name, "Google Chrome");
        assert_eq!(
            overview.callout,
            Some(Callout::Done {
                body: "~/.finicky.ts's defaultBrowser, Safari, isn't used while this is on.".into()
            })
        );

        world.just_overridden = false;
        assert!(matches!(
            world.build().callout,
            Some(Callout::Settled { .. })
        ));
    }

    #[test]
    fn override_warnings_name_what_pwrroutes_default_changes() {
        let handler = |matcher: MatcherSpec| HandlerRule {
            matchers: vec![matcher],
            browser: browser("Firefox"),
        };
        let quiet = Description {
            rewrites: Vec::new(),
            handlers: vec![handler(MatcherSpec::Wildcard("a.example.com/*".into()))],
            default_browser: browser("Safari"),
        };
        assert!(override_warnings(&quiet).is_empty());

        let function = Description {
            default_browser: BrowserSpec::Function(function("pick")),
            ..quiet.clone()
        };
        assert_eq!(
            override_warnings(&function),
            ["Its defaultBrowser function will no longer run."]
        );

        let dropped = Description {
            default_browser: browser(""),
            handlers: vec![
                handler(MatcherSpec::Wildcard("a.example.com/*".into())),
                handler(MatcherSpec::Wildcard("*".into())),
            ],
            ..quiet
        };
        assert_eq!(
            override_warnings(&dropped),
            [
                "Unmatched links will now open instead of being dropped.",
                "Config handler 2 matches every link, so PwrRoute's default never applies.",
            ]
        );
    }

    #[test]
    fn paused_3a_dims_the_map_and_offers_resume() {
        let mut world = World::new(
            fixtures::overview_paused_rules(),
            fixtures::overview_paused_day(),
        );
        world.status.paused = Some(Pause {
            name: "Google Chrome".into(),
            display: "Google Chrome \u{b7} Personal".into(),
        });
        let overview = world.build();
        assert_eq!(overview.hero.state, State::Paused);
        assert_eq!(overview.hero.tone, Tone::Warn);
        assert_eq!(overview.hero.sentence, "Routing is paused.");
        assert_eq!(
            overview.hero.detail,
            [
                "Every link opens in Google Chrome \u{b7} Personal until you resume. Rewrites still run."
            ]
        );
        assert_eq!(overview.hero.action, Some(Action::Resume));
        assert!(overview.hero.action.unwrap().primary());
        assert_eq!(overview.tiles[0].count, 1_403);
        assert_eq!(overview.map.rows.len(), 4);
        assert_eq!(overview.map.caption, Caption::WhenResumes);
        assert!(overview.map.dimmed);
    }

    #[test]
    fn not_default_3b_hides_counts_and_shows_setup() {
        let mut world = World::rules();
        world.status.default = DefaultState::Other("Safari".into());
        let overview = world.build();
        assert_eq!(overview.hero.state, State::NotDefault);
        assert_eq!(
            overview.hero.sentence,
            "Links aren't reaching PwrRoute yet."
        );
        assert_eq!(
            overview.hero.detail,
            ["Safari is your default browser, so links open there and skip your rules."]
        );
        assert_eq!(overview.hero.action, None, "the setup card has the button");
        let setup = overview.setup.expect("the setup card");
        assert_eq!(setup.done(), 1);
        assert_eq!(
            setup.everything_else.result,
            Some((
                badge_for("Google Chrome").to_string(),
                "Google Chrome \u{b7} Personal".into()
            ))
        );
        assert!(!setup.default_browser.done);
        assert!(
            setup
                .default_browser
                .body
                .as_deref()
                .is_some_and(|body| body.contains("keep opening in Safari"))
        );
        let map = overview.map;
        assert_eq!(map.caption, Caption::WhatWillHappen);
        assert!(!map.counts);
        assert!(map.rows.iter().all(|row| row.count.is_none()));
        assert!(map.cards.iter().all(|card| card.share.is_none()));
        assert_eq!(map.everything_else.count, None);
    }

    #[test]
    fn unknown_default_offers_browsers_settings() {
        let mut world = World::rules();
        world.status.default = DefaultState::Unknown;
        let overview = world.build();
        assert_eq!(overview.hero.state, State::Unknown);
        assert_eq!(overview.hero.action, Some(Action::OpenBrowsersSettings));
        assert!(overview.setup.is_none(), "rules.json has a default");

        // With no default chosen, the setup card never calls an unknown
        // default done.
        world.rules.default_browser = String::new();
        world.rules.default_profile = String::new();
        let setup = world.build().setup.expect("the setup card");
        assert_eq!(setup.done(), 0);
        assert!(!setup.default_browser.done);
        assert!(
            setup
                .default_browser
                .body
                .as_deref()
                .is_some_and(|body| body.contains("can't tell")),
            "{:?}",
            setup.default_browser.body
        );
    }

    #[test]
    fn the_setup_step_names_the_configs_default_while_it_owns_everything_else() {
        let mut world = World::config();
        world.rules.default_browser = String::new();
        world.rules.default_profile = String::new();
        let setup = world.build().setup.expect("the setup card");
        assert_eq!(
            setup.everything_else.body.as_deref(),
            Some(
                "~/.finicky.ts's defaultBrowser handles links its handlers don't claim. The \
                 browser below still catches config errors, browsers that can't open, and every \
                 link while paused."
            )
        );
    }

    #[test]
    fn config_error_3c_sends_every_link_to_the_fallback_until_it_loads() {
        // The router's fallback engine routes nothing: not the config, not
        // rules.json's rules or rewrites.
        let mut world = World::config();
        world.config = Some(None);
        world.day = fixtures::overview_config_error_day();
        world.status.config_error = Some(ConfigTrouble::NotLoaded(
            "SyntaxError: Unexpected token \u{b7} line 42".into(),
        ));
        let overview = world.build();
        assert_eq!(overview.hero.state, State::ConfigError);
        assert_eq!(overview.hero.tone, Tone::Danger);
        assert_eq!(overview.hero.sentence, "~/.finicky.ts didn't load.");
        assert_eq!(
            overview.hero.detail,
            ["Every link opens in Google Chrome \u{b7} Personal until it loads."]
        );
        assert_eq!(overview.hero.action, Some(Action::ShowError));
        assert_eq!(overview.tiles[0].count, 212);
        assert!(overview.tiles[3].danger, "two failed");
        assert!(overview.callout.is_none());
        let map = overview.map;
        assert!(map.items.iter().any(|item| matches!(
            item,
            Item::Error { title, detail }
                if title == "~/.finicky.ts didn't load: nothing runs until it does"
                    && detail.starts_with("SyntaxError")
        )));
        assert_eq!(
            headers(&map),
            [
                ("~/.finicky.ts \u{b7} not running", ""),
                ("Your rules \u{b7} not running", "3 rules"),
            ]
        );
        assert_eq!(map.rows.len(), 3, "only the visual rules");
        assert!(map.rows.iter().all(|row| row.count.is_none()));
        // Nothing runs them: the rows, and the cards only they feed, fade.
        assert!(map.rows.iter().all(|row| row.idle));
        let ee = map.everything_else.target.card();
        for (index, card) in map.cards.iter().enumerate() {
            let fed = map
                .rows
                .iter()
                .any(|row| row.enabled && row.target.card() == Some(index));
            assert_eq!(card.idle, fed && Some(index) != ee, "{}", card.name);
        }
        assert!(map.cards.iter().any(|card| card.idle));
        assert_eq!(map.rewrites, None, "no rewrite runs either");
        assert_eq!(map.everything_else.owner, "PwrRoute's default");
        assert_eq!(map.everything_else.count, Some(212));
        assert_eq!(map.cards[0].name, "Google Chrome");
    }

    #[test]
    fn a_failed_reload_keeps_the_last_version_routing() {
        // The window read the config before it broke: the map is the one
        // that still routes, and the hero carries the error.
        let mut world = World::config();
        world.status.config_error = Some(ConfigTrouble::ReloadFailed(
            "SyntaxError: Unexpected token \u{b7} line 42".into(),
        ));
        let overview = world.build();
        assert_eq!(overview.hero.state, State::ConfigStale);
        assert_eq!(overview.hero.tone, Tone::Warn);
        assert_eq!(overview.hero.sentence, "~/.finicky.ts didn't reload.");
        assert_eq!(
            overview.hero.detail,
            [
                "Links still route with the version that last loaded",
                "SyntaxError: Unexpected token \u{b7} line 42",
            ]
        );
        assert_eq!(overview.hero.action, None, "Rules has no error to show");
        let map = &overview.map;
        assert!(
            !map.items
                .iter()
                .any(|item| matches!(item, Item::Error { .. }))
        );
        // The router routes with what last loaded, rules.json included: the
        // headers claim no order, and no count by a number that may have
        // moved since.
        assert_eq!(
            headers(map),
            [
                (
                    "~/.finicky.ts \u{b7} as last loaded",
                    "5 handlers \u{b7} read-only"
                ),
                ("Your rules \u{b7} as last loaded", "3 rules"),
            ]
        );
        assert!(map.rows.iter().all(|row| row.count.is_none()));
        assert!(map.rows.iter().all(|row| !row.idle));
        assert_eq!(map.everything_else.owner, "~/.finicky.ts's default");
        // The callout's switch is written now and taken with the reload.
        let Some(Callout::Offer { warnings, .. }) = &overview.callout else {
            panic!("{:?}", overview.callout);
        };
        assert_eq!(
            warnings.last().map(String::as_str),
            Some("A change here applies once ~/.finicky.ts reloads.")
        );

        // This window could not read it either: an error row that says the
        // last version still runs, and Show the error.
        world.config = Some(None);
        let overview = world.build();
        assert_eq!(overview.hero.action, Some(Action::ShowError));
        assert!(overview.map.items.iter().any(|item| matches!(
            item,
            Item::Error { title, .. }
                if title == "~/.finicky.ts didn't reload: the last version that loaded still runs"
        )));
        assert_eq!(
            headers(&overview.map)[1],
            ("Your rules \u{b7} as last loaded", "3 rules")
        );
    }

    /// rules.json's default naming PwrRoute itself owns nothing: the
    /// engine leaves the config's default in place, and so does the map.
    #[test]
    fn a_rules_json_default_of_pwrroute_leaves_the_configs_default() {
        let mut world = World::config();
        world.rules.precedence = Precedence::PwrRoute;
        world.rules.default_browser = "PwrRoute".into();
        assert!(!world.rules.owns_default());
        let overview = world.build();
        assert_eq!(
            overview.map.everything_else.owner,
            "~/.finicky.ts's default"
        );
        assert!(
            overview
                .map
                .cards
                .iter()
                .all(|card| !card.name.eq_ignore_ascii_case("PwrRoute")),
            "no PwrRoute card"
        );
    }

    /// Every surface finds a handler that takes every link the same way:
    /// `"*"`, `"*://*/*"`, an anything regex, a function that returns true.
    #[test]
    fn the_callout_warns_about_every_kind_of_catch_all() {
        for matcher in [
            MatcherSpec::Wildcard("*".into()),
            MatcherSpec::Wildcard("*://*/*".into()),
            MatcherSpec::Regex("/.*/".into()),
            MatcherSpec::Function(FunctionSpec {
                source: Some("() => true".into()),
                ..function("")
            }),
        ] {
            let description = Description {
                rewrites: Vec::new(),
                handlers: vec![HandlerRule {
                    matchers: vec![matcher.clone()],
                    browser: browser("Firefox"),
                }],
                default_browser: browser("Safari"),
            };
            assert_eq!(
                override_warnings(&description),
                ["Config handler 1 matches every link, so PwrRoute's default never applies."],
                "{matcher:?}"
            );
        }
    }

    /// 3f: rules.json names only a default, and today's links went there.
    #[test]
    fn default_only_3f_counts_the_links_on_its_card() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            ..RulesFile::default()
        };
        let overview = World::new(rules, fixtures::overview_default_only_day()).build();
        assert_eq!(overview.map.everything_else.owner, "PwrRoute's default");
        let chrome = &overview.map.cards[0];
        assert_eq!(chrome.name, "Google Chrome");
        assert_eq!(
            chrome.alone,
            Some((
                "Everything opens in Google Chrome \u{b7} Personal".into(),
                Some("23 links today".into())
            ))
        );
        assert!(
            overview
                .map
                .not_used
                .as_deref()
                .is_none_or(|not_used| !not_used.contains("Google Chrome")),
            "{:?}",
            overview.map.not_used
        );
    }

    #[test]
    fn the_hero_puts_a_config_that_did_not_load_before_a_pause() {
        let mut status = Status::routing();
        status.paused = Some(Pause {
            name: "Google Chrome".into(),
            display: "Google Chrome".into(),
        });
        assert_eq!(status.state(), State::Paused);
        status.config_error = Some(ConfigTrouble::ReloadFailed("x".into()));
        assert_eq!(status.state(), State::ConfigStale);
        status.config_error = Some(ConfigTrouble::NotLoaded("x".into()));
        assert_eq!(status.state(), State::ConfigError);
    }

    #[test]
    fn nothing_3d_prompts_for_a_first_rule_and_a_default() {
        let overview = World::new(RulesFile::default(), fixtures::overview_nothing_day()).build();
        assert_eq!(overview.hero.sentence, "PwrRoute is catching links.");
        assert_eq!(
            overview.hero.detail,
            ["No rules yet, so every link opens in Safari."]
        );
        assert_eq!(overview.hero.action, None);
        let setup = overview.setup.expect("the setup card");
        assert_eq!(setup.done(), 1);
        assert_eq!(
            setup.everything_else.body.as_deref(),
            Some("Links no rule claims open here. Until you choose, that's Safari.")
        );
        let map = overview.map;
        assert_eq!(
            map.empty,
            Some(Empty {
                title: "Every link opens in Safari.".into(),
                import: None,
            })
        );
        assert_eq!(map.everything_else.owner, "Safari until you choose");
        assert_eq!(map.cards.len(), 1);
        let safari = &map.cards[0];
        assert_eq!(safari.ribbon.as_deref(), Some("Everything else"));
        assert_eq!(
            safari.alone,
            Some((
                "Everything opens in Safari".into(),
                Some("23 links today".into())
            ))
        );
        assert!(safari.share.is_none(), "one card has no share to show");
    }

    #[test]
    fn default_only_3e_offers_to_import_the_config_found() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            ..RulesFile::default()
        };
        let mut world = World::new(rules, fixtures::overview_nothing_day());
        world.found_config = Some("~/.finicky.ts");
        let overview = world.build();
        assert!(overview.setup.is_none());
        assert_eq!(overview.hero.action, Some(Action::TestLink));
        assert_eq!(
            overview.hero.detail,
            ["No rules yet, so every link opens in Google Chrome \u{b7} Personal."]
        );
        let empty = overview.map.empty.expect("the first-rule prompt");
        assert_eq!(
            empty.import.as_deref(),
            Some("Import from ~/.finicky.ts\u{2026}")
        );
        assert_eq!(overview.map.everything_else.owner, "PwrRoute's default");
    }

    #[test]
    fn a_browser_is_a_ghost_only_once_the_scan_ran() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![rule("beta.example.net/*", "Orion", "")],
            ..RulesFile::default()
        };
        let mut world = World::new(rules, Vec::new());
        let map = world.build().map;
        assert!(matches!(map.rows[0].target, Target::Ghost { .. }));
        assert_eq!(
            map.cards.last().map(|card| card.kind),
            Some(CardKind::Ghost)
        );

        world.installed.clear();
        let map = world.build().map;
        assert!(matches!(map.rows[0].target, Target::Browser { .. }));
        assert!(map.not_used.is_none());
    }

    #[test]
    fn pills_past_four_fold_into_more_and_keep_everything_else() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            rules: ["A", "B", "C", "D", "E"]
                .iter()
                .map(|profile| rule("a.example.com/*", "Google Chrome", profile))
                .collect(),
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        let chrome = &map.cards[0];
        assert_eq!(chrome.pills.len(), MAX_PILLS);
        assert_eq!(chrome.more_pills, 2);
        let last = chrome.pills.last().expect("pills");
        assert!(last.everything_else);
        assert_eq!(last.label(), "Personal");
        assert!(map.lit(Focus::EverythingElse).pill(0, MAX_PILLS - 1));
    }

    #[test]
    fn a_tiny_share_reads_under_one_percent() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![rule("a.example.com/*", "Arc", "")],
            ..RulesFile::default()
        };
        let pwrroute = Matched::Default {
            owner: Some(Owner::PwrRoute),
        };
        let day = fixtures::overview_day(
            &[
                (Matched::Rule { index: 0 }, Some(("Arc", "")), 1),
                (pwrroute, Some(("Safari", "")), 200),
            ],
            0,
            0,
        );
        let map = World::new(rules, day).build().map;
        let arc = names(&map).iter().position(|n| *n == "Arc").unwrap();
        assert_eq!(
            map.cards[arc].share.as_ref().map(|s| s.label.as_str()),
            Some("<1%")
        );
        assert_eq!(
            map.cards[0].share.as_ref().map(|s| s.label.as_str()),
            Some("100%")
        );
    }

    #[test]
    fn a_rule_with_no_browser_has_no_card() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![rule("a.example.com/*", "", "")],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        assert_eq!(map.rows[0].target, Target::Unset);
        assert_eq!(map.lit(Focus::Row(0)).curves, []);
        assert_eq!(map.cards[0].rules, 0);
    }

    #[test]
    fn handler_matchers_read_by_their_first_entry() {
        assert_eq!(
            handler_match(&[MatcherSpec::Function(function(""))]),
            RowMatch::Function {
                name: None,
                more: 0
            },
            "an anonymous function is a bare \u{192}"
        );
        assert_eq!(
            handler_match(&[
                MatcherSpec::Regex("/^https:\\/\\/a\\.example\\.com/".into()),
                MatcherSpec::Wildcard("b.example.com/*".into()),
            ]),
            RowMatch::Pattern {
                text: "/^https:\\/\\/a\\.example\\.com/".into(),
                regex: true,
                more: 1,
            }
        );
        assert_eq!(handler_match(&[]), RowMatch::Any);
    }
}
