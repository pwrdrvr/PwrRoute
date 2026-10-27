//! The Overview as plain data: what the landing screen says, built from
//! the files, the router's state and today's tally by [`build`], with no
//! window. `overview.rs` draws it; the tests here cover every state the
//! design draws (`design/PwrRoute Overview and Setup.dc.html`, 1a-3f).
//!
//! Four parts, top to bottom: the hero (is it working, and if not, what
//! to do), the setup card (what is left), today's tiles, and the route map:
//! a graph from the rules, in evaluation order on the left, to one node per
//! profile on one card per browser on the right. Every installed browser
//! the Settings don't hide is on it, routed or not. [`Map::lit`] is the
//! map's trace model: for what is hovered, pinned or focused, which rows,
//! cards and nodes light, and every route with its state.
//!
//! Nothing here holds a URL: rows show patterns (the rules' own text),
//! counts and browser names.

use std::collections::{BTreeSet, HashMap};
use std::time::SystemTime;

use pwrroute_browsers::Catalog;
use pwrroute_engine::{
    AppType, BrowserSpec, Description, FALLBACK_BROWSER, HostnameSpec, MatcherSpec, Precedence,
    Rule, RulesFile,
};

use crate::activity::{Counts, Filter, HitKey, Tally, thousands};
use crate::default_browser::DefaultState;
use crate::widgets::{AppBadge, badge_for};

/// A routed card shows every profile that has a route, then idle ones
/// until it holds this many nodes; the rest fold into "+N more profiles".
pub const ROUTED_NODES: usize = 4;

/// An idle card (under "Also installed") shows this many nodes folded.
pub const IDLE_NODES: usize = 2;

/// A ƒ row's excerpt is cut here before layout.
pub const EXCERPT_MAX: usize = 200;

/// One browser the scan found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub bundle_id: Option<String>,
    pub profiles: Vec<String>,
    /// Each profile's directory, by `profiles`' index (Chromium's
    /// "Profile 1"); empty where the browser has none.
    pub directories: Vec<String>,
    /// Hidden in Settings › Browsers: never drawn unless a rule names it.
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

    /// The profile a rule's `profile` value names, by its name: as the
    /// launcher looks it up, by name, else by directory (a config's
    /// `profile: "Profile 1"` is the profile the scan calls "Work").
    fn profile_named(&self, wanted: &str) -> Option<&str> {
        let wanted = wanted.trim();
        self.profiles
            .iter()
            .find(|name| name.eq_ignore_ascii_case(wanted))
            .or_else(|| {
                self.directories
                    .iter()
                    .position(|dir| !dir.is_empty() && dir.eq_ignore_ascii_case(wanted))
                    .and_then(|i| self.profiles.get(i))
            })
            .map(String::as_str)
    }
}

/// A profile as the map names it: the scan's name for it when the browser
/// (by its card key) has a profile by that name or directory, else as
/// written.
fn canonical_profile(
    installed: &[Installed],
    key: &str,
    profile: Option<String>,
) -> Option<String> {
    let profile = profile?;
    let named = installed
        .iter()
        .find(|app| app.name.to_lowercase() == key)
        .and_then(|app| app.profile_named(&profile))
        .map(str::to_owned);
    Some(named.unwrap_or(profile))
}

/// Routing is paused: where every link goes until it resumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pause {
    /// "Google Chrome", for the rail foot.
    pub name: String,
    /// "Google Chrome · Personal", for the hero.
    pub display: String,
}

/// The user sent links somewhere without a rule (design: Send Links 2a,
/// 2b): the tray's Send links, or a node's Next link here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sending {
    /// Next link's count, or All links' time.
    pub scope: pwrroute_router::SendScope,
    /// "Microsoft Edge", for the strip chip and the rail foot.
    pub name: String,
    /// "Microsoft Edge · Work", for the hero.
    pub display: String,
    /// The app as the router opens it, and its name: a card matches either.
    pub app: String,
    /// The profile, `None` for the browser's own or a private window.
    pub profile: Option<String>,
    /// A private window.
    pub private: bool,
}

impl Sending {
    /// "next link" or "all links".
    pub fn short(&self) -> &'static str {
        match self.scope {
            pwrroute_router::SendScope::Next { .. } => "next link",
            pwrroute_router::SendScope::All { .. } => "all links",
        }
    }

    /// Whether `node` of `card` is where links are sent.
    pub fn is_node(&self, card: &Card, node: &Node) -> bool {
        let names = [self.app.as_str(), self.name.as_str()];
        let card_matches = names.iter().any(|name| {
            name.eq_ignore_ascii_case(&card.name) || name.eq_ignore_ascii_case(card.key())
        });
        card_matches
            && node.private.is_some() == self.private
            && (self.private
                || node.profile.as_deref().map(str::to_lowercase)
                    == self.profile.as_deref().map(str::to_lowercase))
    }
}

/// The config in charge is in trouble, as the router reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigTrouble {
    /// It has not loaded (the router's `config_not_loaded`): the router
    /// leaves it out, so `rules.json`'s rules and rewrites run alone and
    /// PwrRoute's default takes the rest until it loads. The first line of
    /// why.
    NotLoaded(String),
    /// It has not loaded and nothing else did either (the router's
    /// `fallback`): nothing it or `rules.json` says runs, and every link
    /// opens in the fallback browser. The first line of why.
    NothingLoaded(String),
    /// A reload failed (the router's `error` alone): the last version that
    /// loaded still routes, with `rules.json`. The first line of why.
    ReloadFailed(String),
}

impl ConfigTrouble {
    pub fn message(&self) -> &str {
        match self {
            Self::NotLoaded(message)
            | Self::NothingLoaded(message)
            | Self::ReloadFailed(message) => message,
        }
    }
}

/// What the router and the OS say right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub paused: Option<Pause>,
    /// Links sent without a rule; never with `paused` (one slot).
    pub sending: Option<Sending>,
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
            sending: None,
            default: DefaultState::PwrRoute,
            config_error: None,
        }
    }

    /// The hero's state, highest priority first: the config did not load
    /// (or reload), paused, sending, not the default browser, unknown,
    /// routing.
    pub fn state(&self) -> State {
        if let Some(trouble) = &self.config_error {
            match trouble {
                ConfigTrouble::NotLoaded(_) | ConfigTrouble::NothingLoaded(_) => State::ConfigError,
                ConfigTrouble::ReloadFailed(_) => State::ConfigStale,
            }
        } else if self.paused.is_some() {
            State::Paused
        } else if self.sending.is_some() {
            State::Sending
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
    /// The config never loaded: rules.json routes without it (or, when it
    /// has nothing either, every link goes to the fallback browser).
    ConfigError,
    /// Its last reload failed: the version that loaded before still runs.
    ConfigStale,
    Paused,
    /// The user sent links somewhere: Next link or All links.
    Sending,
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
    /// End a send: the rules decide again.
    BackToRules,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::ShowError => "Show the error",
            Self::Resume => "Resume routing",
            Self::MakeDefault => "Make PwrRoute the default browser",
            Self::OpenBrowsersSettings => "Open Settings \u{203a} Browsers",
            Self::TestLink => "Test a link\u{2026}",
            Self::BackToRules => "Back to rules",
        }
    }

    /// The hero's primary buttons; the rest are plain.
    pub fn primary(self) -> bool {
        matches!(
            self,
            Self::Resume | Self::MakeDefault | Self::OpenBrowsersSettings | Self::BackToRules
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
    pub result: Option<(AppBadge, String)>,
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
    /// All links sent: "When rules decide again", in accent text.
    WhenRulesDecide,
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

/// One pattern tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub text: String,
    pub regex: bool,
}

/// "from [Sl Slack]": the first sending app a rule names, and how many more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opener {
    pub badge: AppBadge,
    pub name: String,
    pub more: usize,
}

/// A row's match column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowMatch {
    /// The patterns as whole tags, in order: as many as fit, then "+N" for
    /// the rest and `more` (matchers that are not patterns). A rule with an
    /// app shows "from [app]" after them.
    Patterns {
        patterns: Vec<Pattern>,
        more: usize,
        opener: Option<Opener>,
    },
    /// A rule with no patterns, only a sending app.
    Opener(Opener),
    /// A function matcher: its name when it says something
    /// ([`telling_name`]), what it tests ([`function_excerpt`]), and its
    /// whole source for the tooltip. No source (native or bound): "unnamed
    /// function", or the name.
    Function {
        name: Option<String>,
        excerpt: Option<String>,
        source: Option<String>,
        more: usize,
    },
    /// A matcher Finicky would not match with, by its kind.
    Other { text: String, more: usize },
    /// No pattern and no app: every link.
    Any,
}

/// Whether a row's link opens in a private window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Private {
    /// A normal window.
    No,
    /// A private window, by the browser's word for it ("Incognito"): the
    /// route ends at the card's private-window node, whatever profile the
    /// rule names.
    Window(String),
    /// Asked for one, but the browser cannot open a private window from a
    /// link (`Note::PrivateUnsupported`): a normal window opens, and the
    /// route ends at the profile.
    Unsupported,
}

/// Where a row sends links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A browser card; `label` is the profile when the rule names one,
    /// else the browser, and "Incognito" or "Incognito · Work" for a
    /// private window.
    Browser {
        card: usize,
        badge: AppBadge,
        label: String,
        profile: Option<String>,
        private: Private,
    },
    /// A JS function picks the browser per link.
    Function { card: usize },
    /// `browser: null`: the link is not opened.
    NotOpened { card: usize },
    /// A browser the scan did not find: its links fall through.
    Ghost {
        card: usize,
        badge: AppBadge,
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

    pub fn profile(&self) -> Option<&str> {
        match self {
            Self::Browser { profile, .. } => profile.as_deref(),
            _ => None,
        }
    }

    /// A browser that is not installed: its route is not running.
    pub fn ghost(&self) -> bool {
        matches!(self, Self::Ghost { .. })
    }

    /// Opens a private window: its route ends at the private-window node.
    pub fn private_window(&self) -> bool {
        matches!(
            self,
            Self::Browser {
                private: Private::Window(_),
                ..
            }
        )
    }

    /// Asked for a private window the browser cannot open.
    pub fn not_private(&self) -> bool {
        matches!(
            self,
            Self::Browser {
                private: Private::Unsupported,
                ..
            }
        )
    }
}

/// A node: a card's index, and the node's on it.
pub type NodeRef = (usize, usize);

/// Where on a browser card a route ends: a profile (`None`: the default
/// one), or the private-window node.
#[derive(Clone, Copy, Debug)]
enum Slot<'a> {
    Profile(Option<&'a str>),
    Private,
}

/// One ranked rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// 1-based, across sections, in evaluation order.
    pub rank: usize,
    pub source: RowSource,
    pub matcher: RowMatch,
    pub target: Target,
    /// The node its route ends at.
    pub node: Option<NodeRef>,
    pub enabled: bool,
    /// Nothing runs it: neither the config nor rules.json loaded, and the
    /// router's stand-in engine holds no rules. Drawn faded.
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
    /// The node its route ends at.
    pub node: Option<NodeRef>,
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

/// One profile of a browser card, its private-window node, or the one node
/// of a special card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// `None`: the browser's own default profile (or the private-window
    /// node, which is no profile's).
    pub profile: Option<String>,
    /// The private-window node, by the browser's word for it: "Incognito",
    /// "InPrivate", "Private window". Always last on its card, never
    /// folded.
    pub private: Option<String>,
    /// Takes Everything else: "∞".
    pub everything_else: bool,
    /// Enabled rules that send links here.
    pub rules: usize,
    /// Today's links, when there are any and counts are shown.
    pub links: Option<u64>,
    /// No rule names it and Everything else does not go there: "no rules
    /// yet" and an empty port.
    pub unused: bool,
    /// Unused, and "+ Add a rule…" on hover can start a rule that lands
    /// here: on a private-window node, only when a rule naming the card's
    /// browser opens one (not when the node's word came from a route that
    /// names the browser another way).
    pub addable: bool,
    /// "1 rule", "7 rules", "1 rule · falls through", "no rules yet".
    pub meta: Option<String>,
    /// "Google Chrome · Work: 2 rules, 302 links today".
    pub tooltip: String,
}

impl Node {
    pub fn label(&self) -> &str {
        self.private
            .as_deref()
            .or(self.profile.as_deref())
            .unwrap_or("default profile")
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
    pub badge: AppBadge,
    pub name: String,
    /// What rules name it by: the identity rows resolve to, and the key
    /// the window remembers an expanded card by.
    key: String,
    /// Routed profiles first, by the rank of the first row that feeds
    /// them (Everything else after every rule), then the rest in scan
    /// order, then the private-window node when the browser has one. A
    /// special card has one.
    pub nodes: Vec<Node>,
    /// How many profile nodes show while the card is folded: all of them,
    /// or the rest fold into "+N more profiles" (never "+1"). The
    /// private-window node shows after them either way.
    pub shown: usize,
    /// Enabled rules that send links here.
    pub rules: usize,
    /// "2 rules · 944 links today", "4 profiles · no rules yet".
    pub meta: String,
    pub links: Option<u64>,
    pub share: Option<Share>,
    /// "Everything else · PwrRoute" on the card that takes it.
    pub ribbon: Option<String>,
    /// Nothing sends links here yet: under "Also installed", quieter.
    pub unused: bool,
    /// Only rows that nothing runs send links here ([`Row::idle`]): drawn
    /// faded.
    pub idle: bool,
}

impl Card {
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Decided by JavaScript, Not opened, not installed: the whole card is
    /// its one node.
    pub fn single(&self) -> bool {
        self.kind != CardKind::Browser
    }

    /// The private-window node's index: the last one, when there is one.
    pub fn private_node(&self) -> Option<usize> {
        self.nodes
            .last()
            .filter(|node| node.private.is_some())
            .map(|_| self.nodes.len() - 1)
    }

    /// The profile nodes: every node but the private-window one.
    pub fn profiles(&self) -> usize {
        self.nodes.len() - usize::from(self.private_node().is_some())
    }

    /// The nodes "+N more profiles" holds.
    pub fn folded(&self) -> usize {
        self.profiles() - self.shown
    }

    /// The nodes drawn, in order: the profiles that show (all of them
    /// while `expanded`), then the private-window node.
    pub fn visible(&self, expanded: bool) -> Vec<usize> {
        let profiles = if expanded {
            self.profiles()
        } else {
            self.shown
        };
        (0..profiles).chain(self.private_node()).collect()
    }
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
    /// Paused: the body dims and no route is running.
    pub dimmed: bool,
    /// "3 in use · 2 more installed": routed browsers, then idle ones.
    pub in_use: String,
    /// "Also installed"'s note, "2 browsers · no rules yet", when any
    /// browser has no rules yet.
    pub also: Option<String>,
    /// "3 rewrites run on every link first", when there are any.
    pub rewrites: Option<String>,
    pub items: Vec<Item>,
    pub rows: Vec<Row>,
    pub everything_else: EverythingElse,
    /// Routed browsers, the special cards, then the idle browsers A-Z.
    pub cards: Vec<Card>,
    pub empty: Option<Empty>,
}

/// What is hovered, pinned or has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Focus {
    /// An index into [`Map::rows`].
    Row(usize),
    /// A card's header.
    Card(usize),
    /// A card and one of its nodes.
    Node(usize, usize),
    EverythingElse,
}

/// What is traced, by what it is rather than where it is drawn: the
/// cards re-sort as today's links come in, and a pin stays on its profile.
/// The window keeps these and [`Map::resolve`]s them each frame.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FocusKey {
    Row(RowSource),
    /// A card's header, by its key.
    Card(String),
    /// A card by its key, and one of its profiles (`None`: the default
    /// one, or a special card's one node).
    Node(String, Option<String>),
    /// A card's private-window node, by the card's key.
    Private(String),
    EverythingElse,
}

/// The left end of a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum End {
    Row(usize),
    EverythingElse,
}

/// How a route is drawn against the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trace {
    /// Nothing is traced.
    Rest,
    /// Something else is.
    Dim,
    /// This route is.
    Lit,
}

/// One route, from a row (or Everything else) to its node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub from: End,
    pub to: NodeRef,
    pub trace: Trace,
    /// Links take it now. Not: the config never loaded, the browser is not
    /// installed, routing is paused, or the rule is off. Drawn dashed.
    pub running: bool,
    /// Drawn while nothing traces it. Not: a rule that is off, whose
    /// would-be route shows only while it is traced, outside the fan.
    pub at_rest: bool,
    /// Its place among the routes into its node that are drawn at rest,
    /// top to bottom by the row it comes from, and how many there are:
    /// they fan into the port.
    pub slot: usize,
    pub fan: usize,
}

/// The trace for a [`Focus`]: what lights, and every route. Whatever is
/// not lit drops to 45% while something is traced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lit {
    /// Something is traced: the rest dims.
    pub traced: bool,
    pub rows: BTreeSet<usize>,
    pub everything_else: bool,
    /// Traced by their header: the full lit style.
    pub cards: BTreeSet<usize>,
    /// Holding a lit node: an accent border, at full strength.
    pub warm: BTreeSet<usize>,
    pub nodes: BTreeSet<NodeRef>,
    /// Every route: rows in order, then Everything else.
    pub routes: Vec<Route>,
    /// The routes drawn at rest into each node: how wide its port fans.
    fans: HashMap<NodeRef, usize>,
}

impl Lit {
    pub fn row(&self, row: usize) -> bool {
        self.rows.contains(&row)
    }

    pub fn card(&self, card: usize) -> bool {
        self.cards.contains(&card)
    }

    pub fn warm(&self, card: usize) -> bool {
        self.warm.contains(&card)
    }

    pub fn node(&self, card: usize, node: usize) -> bool {
        self.nodes.contains(&(card, node))
    }

    /// The routes drawn at rest into a node: how wide its port fans.
    pub fn fan(&self, card: usize, node: usize) -> usize {
        self.fans.get(&(card, node)).copied().unwrap_or(0)
    }

    /// Some route drawn at rest is dashed: the legend says what that
    /// means. A rule that is off, traced, does not add the swatch.
    pub fn any_not_running(&self) -> bool {
        self.routes
            .iter()
            .any(|route| route.at_rest && !route.running)
    }

    /// The route from a row, or Everything else.
    pub fn route(&self, from: End) -> Option<&Route> {
        self.routes.iter().find(|route| route.from == from)
    }

    /// Some highlighted route into the node runs: its port fills accent.
    /// Lit with only routes that are not running, it rings instead.
    pub fn node_runs(&self, card: usize, node: usize) -> bool {
        self.routes
            .iter()
            .any(|route| route.to == (card, node) && route.trace == Trace::Lit && route.running)
    }
}

/// Whether two profiles are one: by name without regard to case, or both
/// the default.
fn same_profile(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

impl Map {
    fn node_of(&self, focus: End) -> Option<NodeRef> {
        match focus {
            End::Row(row) => self.rows.get(row).and_then(|row| row.node),
            End::EverythingElse => self.everything_else.node,
        }
    }

    /// The trace for `focus`, or the map at rest.
    pub fn lit(&self, focus: Option<Focus>) -> Lit {
        let mut lit = Lit::default();
        let ee = self.everything_else.node;
        // Light a node and every enabled row (and Everything else) into it.
        let light_node = |lit: &mut Lit, node: NodeRef| {
            lit.nodes.insert(node);
            lit.warm.insert(node.0);
            for (i, row) in self.rows.iter().enumerate() {
                if row.enabled && row.node == Some(node) {
                    lit.rows.insert(i);
                }
            }
            if ee == Some(node) {
                lit.everything_else = true;
            }
        };
        match focus {
            None => {}
            Some(Focus::Row(row)) => {
                if row < self.rows.len() {
                    lit.rows.insert(row);
                    if let Some(node) = self.node_of(End::Row(row)) {
                        lit.nodes.insert(node);
                        lit.warm.insert(node.0);
                    }
                }
            }
            Some(Focus::EverythingElse) => {
                lit.everything_else = true;
                if let Some(node) = ee {
                    lit.nodes.insert(node);
                    lit.warm.insert(node.0);
                }
            }
            // A node with no rules yet traces nothing: it offers "+ Add a
            // rule…".
            Some(Focus::Node(card, node)) => {
                if self
                    .cards
                    .get(card)
                    .and_then(|c| c.nodes.get(node))
                    .is_some_and(|n| !n.unused)
                {
                    light_node(&mut lit, (card, node));
                }
            }
            Some(Focus::Card(card)) => {
                if let Some(c) = self.cards.get(card) {
                    lit.cards.insert(card);
                    lit.warm.insert(card);
                    for (node, n) in c.nodes.iter().enumerate() {
                        if !n.unused {
                            light_node(&mut lit, (card, node));
                        }
                    }
                }
            }
        }
        lit.traced = !lit.rows.is_empty()
            || lit.everything_else
            || !lit.cards.is_empty()
            || !lit.nodes.is_empty();

        let trace = |lit_here: bool| {
            if lit_here {
                Trace::Lit
            } else if lit.traced {
                Trace::Dim
            } else {
                Trace::Rest
            }
        };
        let mut routes = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            let Some(to) = row.node else {
                continue;
            };
            // A rule that is off has no route, but its would-be route shows
            // while it is the one traced.
            if !row.enabled && focus != Some(Focus::Row(i)) {
                continue;
            }
            routes.push(Route {
                from: End::Row(i),
                to,
                trace: trace(lit.rows.contains(&i)),
                running: row.enabled && !row.idle && !self.dimmed && !row.target.ghost(),
                at_rest: row.enabled,
                slot: 0,
                fan: 0,
            });
        }
        if let Some(to) = ee {
            routes.push(Route {
                from: End::EverythingElse,
                to,
                trace: trace(lit.everything_else),
                running: !self.dimmed && !self.everything_else.target.ghost(),
                at_rest: true,
                slot: 0,
                fan: 0,
            });
        }
        // Each node's routes fan into its port in the order of their rows,
        // so they never cross there. Only routes drawn at rest take a
        // place: tracing a rule that is off moves nothing; its route ends
        // where it would sit, within the fan.
        let mut fans: HashMap<NodeRef, usize> = HashMap::new();
        for route in &mut routes {
            let seen = fans.entry(route.to).or_insert(0);
            route.slot = *seen;
            if route.at_rest {
                *seen += 1;
            }
        }
        for route in &mut routes {
            let fan = fans.get(&route.to).copied().unwrap_or(0);
            route.fan = fan.max(1);
            route.slot = route.slot.min(route.fan - 1);
        }
        lit.routes = routes;
        lit.fans = fans;
        lit
    }

    /// What is at `focus`, by what it is.
    pub fn key_of(&self, focus: Focus) -> Option<FocusKey> {
        match focus {
            Focus::Row(row) => self.rows.get(row).map(|row| FocusKey::Row(row.source)),
            Focus::EverythingElse => Some(FocusKey::EverythingElse),
            Focus::Card(card) => self
                .cards
                .get(card)
                .map(|card| FocusKey::Card(card.key.clone())),
            Focus::Node(card, node) => {
                let card = self.cards.get(card)?;
                let node = card.nodes.get(node)?;
                Some(if node.private.is_some() {
                    FocusKey::Private(card.key.clone())
                } else {
                    FocusKey::Node(card.key.clone(), node.profile.clone())
                })
            }
        }
    }

    /// Where `key` is on this map, if it still is.
    pub fn resolve(&self, key: &FocusKey) -> Option<Focus> {
        let card_at = |key: &str| self.cards.iter().position(|card| card.key == key);
        match key {
            FocusKey::Row(source) => self
                .rows
                .iter()
                .position(|row| row.source == *source)
                .map(Focus::Row),
            FocusKey::EverythingElse => Some(Focus::EverythingElse),
            FocusKey::Card(key) => card_at(key).map(Focus::Card),
            FocusKey::Node(key, profile) => {
                let card = card_at(key)?;
                let node = self.cards[card].nodes.iter().position(|node| {
                    node.private.is_none()
                        && same_profile(node.profile.as_deref(), profile.as_deref())
                })?;
                Some(Focus::Node(card, node))
            }
            FocusKey::Private(key) => {
                let card = card_at(key)?;
                Some(Focus::Node(card, self.cards[card].private_node()?))
            }
        }
    }

    /// What the legend calls a pin: "Google Chrome · Work", "rule 3".
    pub fn focus_label(&self, focus: Focus) -> Option<String> {
        match focus {
            Focus::Row(row) => self.rows.get(row).map(|row| row.tooltip.clone()),
            Focus::EverythingElse => Some("Everything else".to_owned()),
            Focus::Card(card) => self.cards.get(card).map(|card| card.name.clone()),
            Focus::Node(card, node) => {
                let card = self.cards.get(card)?;
                let node = card.nodes.get(node)?;
                Some(if card.single() {
                    card.name.clone()
                } else {
                    display(&card.name, Some(node.label()))
                })
            }
        }
    }

    /// What the keyboard steps through, top to bottom: the rows, Everything
    /// else, then each card's nodes that show (`expanded` says which cards
    /// show all of theirs), its private-window node after its profiles.
    pub fn keyboard_order(&self, expanded: impl Fn(&Card) -> bool) -> Vec<Focus> {
        let mut order: Vec<Focus> = self
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Row(row) => Some(Focus::Row(*row)),
                _ => None,
            })
            .collect();
        order.push(Focus::EverythingElse);
        for (c, card) in self.cards.iter().enumerate() {
            order.extend(
                card.visible(expanded(card))
                    .into_iter()
                    .map(|n| Focus::Node(c, n)),
            );
        }
        order
    }
}

/// How many of a row's pattern tags show in `room` pixels: whole tags,
/// 5 apart, as many as fit with "+N" after them for the rest (`plus(n)` is
/// its width). The first tag always shows, cut short when it is alone.
pub fn fit_tags(widths: &[f32], more: usize, room: f32, plus: impl Fn(usize) -> f32) -> usize {
    const GAP: f32 = 5.0;
    let n = widths.len();
    for k in (1..=n).rev() {
        let rest = n - k + more;
        let need = widths[..k].iter().sum::<f32>()
            + GAP * (k - 1) as f32
            + if rest > 0 { GAP + plus(rest) } else { 0.0 };
        if need <= room {
            return k;
        }
    }
    n.min(1)
}

/// The map's columns: each column grows to a cap, and the routes' gutter
/// takes every pixel past them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapWidths {
    pub rule: f32,
    pub gutter: f32,
    pub dest: f32,
}

/// The routes' gutter is never narrower.
pub const GUTTER_MIN: f32 = 96.0;
/// The destinations stop growing here (a 1424 window).
pub const DEST_MAX: f32 = 420.0;
/// The rules stop growing here (a 1514 window); past it, all extra width
/// is gutter.
pub const RULE_MAX: f32 = 720.0;

impl MapWidths {
    /// For the map's content width (less its borders and padding): the
    /// destinations are 40% of what the narrowest gutter leaves, within
    /// 266-420, the rules what is left up to 720, and the gutter the rest.
    /// The caps are fixed rather than measured, so the columns stay put
    /// when a rule is added or a count grows.
    pub fn of_content(content: f32) -> Self {
        let content = content.max(0.0);
        let dest = ((content - GUTTER_MIN) * 0.4)
            .clamp(266.0, DEST_MAX)
            .round();
        let rule = (content - GUTTER_MIN - dest).clamp(0.0, RULE_MAX);
        Self {
            rule,
            gutter: (content - rule - dest).max(GUTTER_MIN),
            dest,
        }
    }
}

/// How `fan` routes into one node spread on its port: the step between
/// them, at most 3, and the spread, at most 16 (the capsule is 9 plus it).
pub fn fan_spread(fan: usize) -> (f32, f32) {
    if fan < 2 {
        return (0.0, 0.0);
    }
    let gaps = (fan - 1) as f32;
    let step = (16.0 / gaps).min(3.0);
    (step, step * gaps)
}

/// How many of `len` nodes show folded: at least `floor`, and all of them
/// when only one would fold (never "+1 more").
fn folded_to(len: usize, floor: usize) -> usize {
    if len <= floor + 1 { len } else { floor }
}

/// Function names that say nothing about what the function tests.
const GENERIC_NAMES: [&str; 8] = [
    "match",
    "url",
    "handler",
    "matcher",
    "test",
    "fn",
    "anonymous",
    "default",
];

/// A function's name when it says something: not empty, and not one of
/// the generic names an inline arrow gets from its property.
pub fn telling_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && !GENERIC_NAMES
            .iter()
            .any(|generic| generic.eq_ignore_ascii_case(name)))
    .then(|| name.to_owned())
}

/// Where a lexer is between tokens: the last character that was not
/// whitespace, and the word it ended, so a `/` can be told apart as a
/// regex literal's start or a division.
#[derive(Default)]
struct Before {
    prev: Option<char>,
    word: String,
    spaced: bool,
}

impl Before {
    fn ident(c: char) -> bool {
        c.is_alphanumeric() || c == '_' || c == '$'
    }

    fn push(&mut self, c: char) {
        if c.is_whitespace() {
            self.spaced = true;
            return;
        }
        if Self::ident(c) {
            if self.spaced || !self.prev.is_some_and(Self::ident) {
                self.word.clear();
            }
            self.word.push(c);
        } else {
            self.word.clear();
        }
        self.prev = Some(c);
        self.spaced = false;
    }

    /// A `/` here starts a regex literal: at the start, after an operator
    /// or an opening bracket, or after a keyword such as `return`.
    fn regex_next(&self) -> bool {
        match self.prev {
            None => true,
            Some(c) if "(,=:[!&|?{};+-*%<>~^".contains(c) => true,
            Some(c) if Self::ident(c) => matches!(
                self.word.as_str(),
                "return" | "typeof" | "case" | "in" | "of" | "delete" | "void" | "throw" | "new"
            ),
            Some(_) => false,
        }
    }
}

/// The rest of a regex literal after its opening `/`: through the `/` that
/// closes it (not one inside a `[…]` class), or the end of the line.
fn regex_body(chars: &mut std::iter::Peekable<impl Iterator<Item = (usize, char)>>) -> String {
    let mut body = String::new();
    let mut class = false;
    while let Some(&(_, c)) = chars.peek() {
        if c == '\n' {
            break;
        }
        chars.next();
        body.push(c);
        match c {
            '\\' => {
                if let Some((_, next)) = chars.next() {
                    body.push(next);
                }
            }
            '[' => class = true,
            ']' => class = false,
            '/' if !class => break,
            _ => {}
        }
    }
    body
}

/// The source without its comments, strings and regex literals kept whole.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.char_indices().peekable();
    let mut quote: Option<char> = None;
    let mut before = Before::default();
    while let Some((_, c)) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' {
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
            } else if c == q {
                quote = None;
                before.push(')');
            }
            continue;
        }
        let next = chars.peek().map(|&(_, next)| next);
        match c {
            '"' | '\'' | '`' => {
                quote = Some(c);
                out.push(c);
            }
            '/' if next == Some('/') => {
                while chars.peek().is_some_and(|&(_, next)| next != '\n') {
                    chars.next();
                }
            }
            '/' if next == Some('*') => {
                chars.next();
                let mut previous = ' ';
                for (_, next) in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
                out.push(' ');
            }
            '/' if before.regex_next() => {
                out.push(c);
                out.push_str(&regex_body(&mut chars));
                // What follows a regex divides, as after a value.
                before.push(')');
            }
            '\\' => {
                out.push(c);
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
            }
            _ => {
                out.push(c);
                before.push(c);
            }
        }
    }
    out
}

/// Each character outside strings and regex literals, by byte offset, with
/// the bracket depth it sits at (a bracket at the depth of the pair it
/// opens or closes).
fn top_level(text: &str) -> Vec<(usize, char, usize)> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut before = Before::default();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if let Some(q) = quote {
            if c == '\\' {
                chars.next();
            } else if c == q {
                quote = None;
                before.push(')');
            }
            continue;
        }
        match c {
            '\\' => {
                chars.next();
            }
            '"' | '\'' | '`' => quote = Some(c),
            '/' if before.regex_next() && !matches!(chars.peek(), Some((_, '/' | '*'))) => {
                regex_body(&mut chars);
                before.push(')');
            }
            '(' | '[' | '{' => {
                out.push((i, c, depth));
                depth += 1;
                before.push(c);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                out.push((i, c, depth));
                before.push(c);
            }
            _ => {
                out.push((i, c, depth));
                before.push(c);
            }
        }
    }
    out
}

/// The close of the bracket opening `text` when it is the last character:
/// the whole of `text` is wrapped in one pair.
fn wrapped(text: &str, open: char, close: char) -> Option<&str> {
    if !text.starts_with(open) {
        return None;
    }
    let end = top_level(text)
        .into_iter()
        .skip(1)
        .find(|&(_, c, depth)| c == close && depth == 0)?
        .0;
    (end + close.len_utf8() == text.len()).then(|| &text[open.len_utf8()..end])
}

/// A value that says nothing of what is tested: `true`, `false`, `null`,
/// `undefined`, a number.
fn is_literal(expression: &str) -> bool {
    let e = expression.trim();
    matches!(e, "true" | "false" | "null" | "undefined" | "!0" | "!1") || e.parse::<f64>().is_ok()
}

/// A literal that is false to JavaScript.
fn is_falsy(expression: &str) -> bool {
    matches!(
        expression.trim(),
        "false" | "null" | "undefined" | "0" | "!1" | "\"\"" | "''"
    )
}

/// Each `return` in `text` at `depth` (`chars` is the [`top_level`] of
/// `text`), in order, that is the keyword, not part of a name or a
/// property (`x.return`): the expression it returns, to the `;` that ends
/// its statement or the `}` that closes its block.
fn returns(text: &str, chars: &[(usize, char, usize)], at_depth: Option<usize>) -> Vec<String> {
    // A line break ends the statement too (as JavaScript inserts a `;`)
    // unless the line ends, or the next begins, mid-expression.
    let line_ends = |at: usize| {
        let last = text[..at].trim_end().chars().last();
        let next = text[at..].trim_start().chars().next();
        !last.is_some_and(|c| "&|+-*/%=<>?:,(.[!~^".contains(c))
            && !next.is_some_and(|c| ".?:&|+-*/%=<>,)]".contains(c))
    };
    let statement_end = |from: usize, depth: usize| {
        chars
            .iter()
            .find(|&&(i, c, d)| {
                i >= from
                    && ((d == depth && c == ';')
                        || (d + 1 == depth && c == '}')
                        || (d == depth && c == '\n' && i > from && line_ends(i)))
            })
            .map_or(text.len(), |&(i, _, _)| i)
    };
    let mut found = Vec::new();
    for (n, &(i, _, depth)) in chars.iter().enumerate() {
        if at_depth.is_some_and(|want| want != depth) || !text[i..].starts_with("return") {
            continue;
        }
        let before = n.checked_sub(1).map(|p| chars[p].1);
        let after = text[i + 6..].chars().next();
        if before.is_some_and(|c| Before::ident(c) || c == '.') || after.is_some_and(Before::ident)
        {
            continue;
        }
        let start = i + 6;
        found.push(text[start..statement_end(start, depth)].trim().to_owned());
    }
    found
}

/// A leading `if (condition) …` whose branch returns a literal: the
/// condition, negated when the branch returns false (`if (x) return false`
/// tests `!(x)`).
fn guard_condition(inner: &str, chars: &[(usize, char, usize)]) -> Option<String> {
    let first = inner.trim_start();
    let start = inner.len() - first.len();
    let rest = first.strip_prefix("if")?;
    if !rest.trim_start().starts_with('(') {
        return None;
    }
    let open = chars
        .iter()
        .find(|&&(i, c, depth)| i >= start + 2 && depth == 0 && c == '(')?
        .0;
    let close = chars
        .iter()
        .find(|&&(i, c, depth)| i > open && depth == 0 && c == ')')?
        .0;
    let condition = inner[open + 1..close].trim();
    // The branch: a block, or one statement.
    let after = &inner[close + 1..];
    let branch_chars = top_level(after);
    let returned = returns(after, &branch_chars, None).into_iter().next()?;
    // `if (x) return true` with no `;` ends at the line.
    let returned = returned.lines().next().unwrap_or_default().to_owned();
    if !is_literal(&returned) || condition.is_empty() {
        return None;
    }
    Some(if is_falsy(&returned) {
        format!("!({condition})")
    } else {
        condition.to_owned()
    })
}

/// A block's body (`{ … }`, braces included) as one expression: its first
/// top-level `return` that says something (not a literal); else, when it
/// opens with an `if` guard that returns a literal, the guard's
/// condition; else its first `return`'s literal, or its first statement
/// and " …".
fn block_expression(block: &str) -> String {
    let inner = &block[1..block.len() - 1];
    let chars = top_level(inner);
    let found = returns(inner, &chars, Some(0));
    if let Some(expression) = found.iter().find(|expression| !is_literal(expression)) {
        return expression.clone();
    }
    if let Some(condition) = guard_condition(inner, &chars) {
        return condition;
    }
    if let Some(expression) = found.into_iter().next() {
        return expression;
    }
    let first = inner.trim_start();
    let start = inner.len() - first.len();
    let end = chars
        .iter()
        .find(|&&(i, c, depth)| i >= start && depth == 0 && (c == ';' || c == '\n'))
        .map_or(inner.len(), |&(i, _, _)| i);
    let statement = inner[start..end].trim();
    if statement.is_empty() {
        String::new()
    } else {
        format!("{statement} \u{2026}")
    }
}

/// What a function matcher tests, from its source, for a ƒ row:
/// `({ url }) => url.host === "tickets.example.com"` reads
/// `url.host === "tickets.example.com"`. The text after the first
/// top-level `=>`, or the body of `function (…) {…}`; a block gives its
/// first `return`'s expression (or its first statement and " …"); one
/// wrapping pair of parentheses and of braces and a trailing `;` go,
/// comments go, whitespace collapses, and it is cut at [`EXCERPT_MAX`].
pub fn function_excerpt(source: &str) -> Option<String> {
    let clean = strip_comments(source);
    let mut text = clean.trim();
    if let Some(rest) = text.strip_prefix("async")
        && rest.starts_with(|c: char| c.is_whitespace() || c == '(')
    {
        text = rest.trim_start();
    }
    let chars = top_level(text);
    let arrow = chars
        .iter()
        .find(|&&(i, c, depth)| depth == 0 && c == '=' && text[i + 1..].starts_with('>'))
        .map(|&(i, _, _)| i);
    let body = match arrow {
        Some(at) => text[at + 2..].trim(),
        None => {
            // `function name(…) { … }`, or a method: the block after the
            // parameters.
            let params = chars
                .iter()
                .find(|&&(_, c, depth)| depth == 0 && c == ')')
                .map(|&(i, _, _)| i);
            let open = params.and_then(|params| {
                chars
                    .iter()
                    .find(|&&(i, c, depth)| i > params && depth == 0 && c == '{')
                    .map(|&(i, _, _)| i)
            });
            let close = open.and_then(|open| {
                chars
                    .iter()
                    .find(|&&(i, c, depth)| i > open && depth == 0 && c == '}')
                    .map(|&(i, _, _)| i)
            });
            match (open, close) {
                (Some(open), Some(close)) => &text[open..=close],
                _ => text,
            }
        }
    };
    let mut expression = if body.starts_with('{') && wrapped(body, '{', '}').is_some() {
        block_expression(body)
    } else {
        body.to_owned()
    };
    let trimmed = expression.trim().trim_end_matches(';').trim();
    let mut unwrapped = trimmed;
    if let Some(inner) = wrapped(unwrapped, '(', ')') {
        unwrapped = inner.trim();
    }
    if let Some(inner) = wrapped(unwrapped, '{', '}') {
        unwrapped = inner.trim();
    }
    expression = unwrapped.trim_end_matches(';').to_owned();
    let collapsed = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    Some(if collapsed.chars().count() > EXCERPT_MAX {
        let cut: String = collapsed.chars().take(EXCERPT_MAX - 1).collect();
        format!("{}\u{2026}", cut.trim_end())
    } else {
        collapsed
    })
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
    badge: AppBadge,
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
        badge: badge_for(&name),
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

/// A config browser spec as a destination, a profile, and whether its
/// `args` ask for a private window (the Finicky API has no field of its
/// own for it).
fn spec_dest(spec: &BrowserSpec, installed: &[Installed]) -> (Dest, Option<String>, Ask) {
    match spec {
        BrowserSpec::Function(_) => (function_dest(), None, Ask::No),
        BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => {
            (not_opened_dest(), None, Ask::No)
        }
        BrowserSpec::Browser(fixed) => (
            resolve(&fixed.name, installed),
            (!fixed.profile.is_empty()).then(|| fixed.profile.clone()),
            if fixed
                .args
                .iter()
                .any(|arg| pwrroute_engine::is_private_flag(arg))
            {
                Ask::Yes {
                    name: fixed.name.clone(),
                    app_type: fixed.app_type,
                    args: fixed.args.clone(),
                }
            } else {
                Ask::No
            },
        ),
    }
}

/// Whether a route asks for a private window: a `rules[].private` rule, or
/// a config whose `args` carry a private flag. The browser is kept as the
/// launcher gets it, which decides what opens.
#[derive(Clone, Debug)]
enum Ask {
    No,
    Yes {
        name: String,
        app_type: AppType,
        args: Vec<String>,
    },
}

impl Ask {
    /// A `rules.json` rule's: its browser string, typed as the engine
    /// hands it to the launcher.
    fn rule(rule: &Rule) -> Self {
        if rule.private {
            Self::Yes {
                name: rule.browser.clone(),
                app_type: pwrroute_engine::app_type_of(rule.browser.trim()),
                args: Vec::new(),
            }
        } else {
            Self::No
        }
    }

    /// How the window opens on a browser card, as the launcher opens it.
    fn private(&self) -> Private {
        match self {
            Self::No => Private::No,
            Self::Yes {
                name,
                app_type,
                args,
            } => match pwrroute_browsers::private_window(
                name.trim(),
                *app_type,
                args,
                pwrroute_browsers::Platform::current(),
            ) {
                Some(term) => Private::Window(term.to_owned()),
                None => Private::Unsupported,
            },
        }
    }
}

/// What a browser, as a rule names it, calls its private window
/// ("Incognito", "InPrivate", "Private window"); `None` when a rule naming
/// it would open a normal one (Safari, Arc, or a name the catalog does not
/// know, so the launcher could not add the flag).
pub(crate) fn private_term(name: &str) -> Option<String> {
    let name = name.trim();
    pwrroute_browsers::private_window(
        name,
        pwrroute_engine::app_type_of(name),
        &[],
        pwrroute_browsers::Platform::current(),
    )
    .map(str::to_owned)
}

/// The hero's sentence while links are sent (design: Send Links 2a, 2b):
/// "Next link → Microsoft Edge · Work.", "Next 3 links → …", "All links
/// → …".
pub fn sending_sentence(sending: &Sending) -> String {
    let what = match sending.scope {
        pwrroute_router::SendScope::Next { left: 1, .. } => "Next link".to_owned(),
        pwrroute_router::SendScope::Next { left, .. } => format!("Next {left} links"),
        pwrroute_router::SendScope::All { .. } => "All links".to_owned(),
    };
    format!("{what} \u{2192} {}.", sending.display)
}

/// The hero's line under it: what happens after, and that rewrites run.
pub fn sending_detail(sending: &Sending, now: SystemTime) -> String {
    let after = match sending.scope {
        pwrroute_router::SendScope::Next { left, of } if left < of => {
            format!("{left} of {of} left, then your rules decide again.")
        }
        pwrroute_router::SendScope::Next { .. } => "Then your rules decide again.".to_owned(),
        pwrroute_router::SendScope::All { until: None } => "Until you go back to rules.".to_owned(),
        pwrroute_router::SendScope::All { until: Some(until) } => {
            let minutes = until
                .duration_since(now)
                .unwrap_or_default()
                .as_secs()
                .div_ceil(60);
            match minutes {
                0 => "For less than a minute, then your rules decide again.".to_owned(),
                1 => "For 1 more minute, then your rules decide again.".to_owned(),
                n => format!("For {n} more minutes, then your rules decide again."),
            }
        }
    };
    format!("{after} Rewrites still run.")
}

/// "an Incognito window", "an InPrivate window", "a private window".
pub(crate) fn window_phrase(term: &str) -> String {
    let lower = term.to_lowercase();
    if lower.ends_with("window") {
        format!("a {lower}")
    } else if term.starts_with(['A', 'E', 'I', 'O', 'U', 'a', 'e', 'i', 'o', 'u']) {
        format!("an {term} window")
    } else {
        format!("a {term} window")
    }
}

/// The cards, in order of first use, and the targets that point at them.
struct Cards<'a> {
    /// The scan, which names a profile written by its directory.
    installed: &'a [Installed],
    dests: Vec<Dest>,
    /// Per browser card: the profiles rows and Everything else name, in
    /// order of use (`None`: the default one).
    profiles: Vec<Vec<Option<String>>>,
}

impl<'a> Cards<'a> {
    fn new(installed: &'a [Installed]) -> Self {
        Self {
            installed,
            dests: Vec::new(),
            profiles: Vec::new(),
        }
    }

    /// The target for `dest` and `profile`, in a private window when
    /// `private` asks for one: its route then ends at the card's
    /// private-window node, or at the profile when the browser has none.
    fn add(&mut self, dest: Dest, profile: Option<String>, ask: &Ask) -> Target {
        let profile = canonical_profile(self.installed, &dest.key, profile);
        let private = match dest.kind {
            CardKind::Browser => ask.private(),
            _ => Private::No,
        };
        let card = match self.dests.iter().position(|d| d.key == dest.key) {
            Some(card) => card,
            None => {
                self.dests.push(Dest {
                    kind: dest.kind,
                    key: dest.key.clone(),
                    name: dest.name.clone(),
                    badge: dest.badge.clone(),
                });
                self.profiles.push(Vec::new());
                self.dests.len() - 1
            }
        };
        if dest.kind == CardKind::Browser && !matches!(private, Private::Window(_)) {
            let profiles = &mut self.profiles[card];
            if !profiles
                .iter()
                .any(|p| same_profile(p.as_deref(), profile.as_deref()))
            {
                profiles.push(profile.clone());
            }
        }
        match dest.kind {
            CardKind::Browser => Target::Browser {
                card,
                label: match &private {
                    Private::Window(term) => display(term, profile.as_deref()),
                    _ => profile.clone().unwrap_or_else(|| dest.name.clone()),
                },
                badge: dest.badge,
                profile,
                private,
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

    /// A row's tooltip, with what its private window does: "rule 5 · opens
    /// an Incognito window as Work", or why it opens a normal one.
    fn with_private_tip(&self, tooltip: String, target: &Target) -> String {
        let tip = match target {
            Target::Browser {
                private: Private::Window(term),
                profile,
                ..
            } => match profile {
                Some(profile) => format!("opens {} as {profile}", window_phrase(term)),
                None => format!("opens {}", window_phrase(term)),
            },
            Target::Browser {
                private: Private::Unsupported,
                card,
                ..
            } => not_private_tip(&self.dests[*card].name),
            _ => return tooltip,
        };
        format!("{tooltip} \u{00b7} {tip}")
    }
}

/// Why a private rule to `browser` opens a normal window.
pub fn not_private_tip(browser: &str) -> String {
    format!("{browser} can't open a private window from a link, so this opens a normal one.")
}

fn is_regex(pattern: &str) -> bool {
    pattern.len() > 1 && pattern.starts_with('/') && pattern.ends_with('/')
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
    let opener = openers.first().map(|first| {
        let name = crate::default_browser::browser_name(first);
        // "Slack:com.tinyspeck.slackmacgap" names Slack.
        let name = name.split(':').next().unwrap_or(&name).to_owned();
        Opener {
            badge: badge_for(&name),
            name,
            more: openers.len() - 1,
        }
    });
    // "*" from an app reads as the app: the pattern says nothing more.
    let every_link = patterns.iter().all(|p| p.trim() == "*");
    if !patterns.is_empty() && (opener.is_none() || !every_link) {
        RowMatch::Patterns {
            patterns: patterns
                .iter()
                .map(|text| Pattern {
                    regex: is_regex(text),
                    text: (*text).clone(),
                })
                .collect(),
            more: 0,
            opener,
        }
    } else if let Some(opener) = opener {
        RowMatch::Opener(opener)
    } else {
        RowMatch::Any
    }
}

/// A matcher as pattern tags: a wildcard or regex as itself, and a
/// function from `finicky.matchHostnames` as the hostnames it was made
/// with (a regex tested on the hostname as a regex tag).
fn as_tags(matcher: &MatcherSpec) -> Option<Vec<Pattern>> {
    match matcher {
        MatcherSpec::Wildcard(text) => Some(vec![Pattern {
            text: text.clone(),
            regex: false,
        }]),
        MatcherSpec::Regex(text) => Some(vec![Pattern {
            text: text.clone(),
            regex: true,
        }]),
        MatcherSpec::Function(function) => function
            .hostnames
            .as_ref()
            .filter(|list| !list.is_empty())
            .map(|list| {
                list.iter()
                    .map(|entry| match entry {
                        HostnameSpec::Hostname(text) => Pattern {
                            text: text.clone(),
                            regex: false,
                        },
                        HostnameSpec::Regex(text) => Pattern {
                            text: text.clone(),
                            regex: true,
                        },
                    })
                    .collect()
            }),
        MatcherSpec::Other(_) => None,
    }
}

/// A config handler's match column: tags when its first matcher reads as
/// them ([`as_tags`]), else by its first matcher's kind.
fn handler_match(matchers: &[MatcherSpec]) -> RowMatch {
    let more = matchers.len().saturating_sub(1);
    let Some(first) = matchers.first() else {
        return RowMatch::Any;
    };
    if as_tags(first).is_some() {
        let mut patterns = Vec::new();
        let mut rest = 0;
        for matcher in matchers {
            match as_tags(matcher) {
                Some(tags) => patterns.extend(tags),
                None => rest += 1,
            }
        }
        return RowMatch::Patterns {
            patterns,
            more: rest,
            opener: None,
        };
    }
    match first {
        MatcherSpec::Function(function) => RowMatch::Function {
            name: telling_name(&function.name),
            excerpt: function.source.as_deref().and_then(cached_excerpt),
            source: function.source.clone(),
            more,
        },
        MatcherSpec::Other(kind) => RowMatch::Other {
            text: kind.clone(),
            more,
        },
        // Read as tags above.
        MatcherSpec::Wildcard(_) | MatcherSpec::Regex(_) => RowMatch::Any,
    }
}

/// [`function_excerpt`], once per source: the map is built on every frame
/// of a fade, and a config's function can be long.
fn cached_excerpt(source: &str) -> Option<String> {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static EXCERPTS: RefCell<HashMap<String, Option<String>>> =
            RefCell::new(HashMap::new());
    }
    EXCERPTS.with(|cache| {
        if let Some(found) = cache.borrow().get(source) {
            return found.clone();
        }
        let excerpt = function_excerpt(source);
        let mut cache = cache.borrow_mut();
        // A config edited over a long session: start over now and then.
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(source.to_owned(), excerpt.clone());
        excerpt
    })
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
    // Never loaded: the router leaves the config out and routes with
    // rules.json alone, or, when nothing loaded at all, sends every link
    // to the fallback browser.
    let nothing_runs = matches!(trouble, Some(ConfigTrouble::NothingLoaded(_)));
    let not_loaded = nothing_runs || matches!(trouble, Some(ConfigTrouble::NotLoaded(_)));
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
    let mut cards = Cards::new(installed);
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
        let profile = canonical_profile(installed, &dest.key, profile);
        (dest, profile)
    };
    let pwrroute_display = {
        let (dest, profile) = pwrroute_dest();
        display(&dest.name, profile.as_deref())
    };

    // The config's handlers and the visual rules, in evaluation order.
    let config_section = |rows: &mut Vec<Row>, items: &mut Vec<Item>, cards: &mut Cards<'_>| {
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
                title: if nothing_runs {
                    format!("{} didn't load: nothing runs until it does", config.shown)
                } else if not_loaded {
                    format!("{} didn't load: links skip it", config.shown)
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
            let (dest, profile, private) = spec_dest(&handler.browser, installed);
            let target = cards.add(dest, profile, &private);
            let ghost = matches!(target, Target::Ghost { .. });
            items.push(Item::Row(rows.len()));
            rows.push(Row {
                rank: rows.len() + 1,
                source: RowSource::Handler(index),
                matcher: handler_match(&handler.matchers),
                node: None,
                enabled: true,
                idle: false,
                count: (!ghost && !stale)
                    .then(|| count_of(HitKey::Handler(index)))
                    .flatten(),
                tooltip: cards.with_private_tip(
                    format!("config handler {} of {}", index + 1, config.shown),
                    &target,
                ),
                target,
            });
        }
    };
    let rules_section = |rows: &mut Vec<Row>, items: &mut Vec<Item>, cards: &mut Cards<'_>| {
        if rules.rules.is_empty() {
            return;
        }
        if config.is_some() {
            items.push(Item::Header {
                label: if nothing_runs {
                    // The router's fallback engine holds no rules.
                    "Your rules \u{00b7} not running".to_owned()
                } else if not_loaded {
                    // The router routes with rules.json alone.
                    "Your rules \u{00b7} run without it".to_owned()
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
                cards.add(resolve(&rule.browser, installed), profile, &Ask::rule(rule))
            };
            let ghost = matches!(target, Target::Ghost { .. });
            items.push(Item::Row(rows.len()));
            rows.push(Row {
                rank: rows.len() + 1,
                source: RowSource::Rule(index),
                matcher: rule_match(rule),
                node: None,
                enabled: rule.enabled,
                idle: nothing_runs,
                count: (!ghost && rule.enabled && !nothing_runs && !stale)
                    .then(|| count_of(HitKey::Rule(index)))
                    .flatten(),
                tooltip: cards.with_private_tip(format!("rule {}", index + 1), &target),
                target,
            });
        }
    };

    // The Everything else card comes first; the rest follow first use.
    let (ee_dest, ee_profile, ee_private) = match description {
        Some(description) if config_owns_default => {
            spec_dest(&description.default_browser, installed)
        }
        _ => {
            let (dest, profile) = pwrroute_dest();
            (dest, profile, Ask::No)
        }
    };
    let ee_target = cards.add(ee_dest, ee_profile, &ee_private);
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
        node: None,
        count: (counts && inputs.today.unrouted > 0).then_some(inputs.today.unrouted),
    };

    // Today's links per card, and per card, profile (`None`: the default
    // one) and whether the window was private.
    let mut links = vec![0u64; cards.dests.len()];
    let mut profile_links: Vec<Vec<(Option<String>, bool, u64)>> =
        vec![Vec::new(); cards.dests.len()];
    let mut function_links = 0;
    for (name, profile, private, n) in inputs.tally.browsers() {
        let dest = resolve(name, installed);
        if let Some(card) = cards.dests.iter().position(|d| d.key == dest.key) {
            links[card] += n;
            let profile = canonical_profile(
                installed,
                &dest.key,
                (!profile.is_empty()).then(|| profile.to_owned()),
            );
            match profile_links[card]
                .iter_mut()
                .find(|(p, q, _)| *q == private && same_profile(p.as_deref(), profile.as_deref()))
            {
                Some((_, _, total)) => *total += n,
                None => profile_links[card].push((profile, private, n)),
            }
        }
    }
    for row in &rows {
        if let Target::Function { .. } = row.target {
            function_links += row.count.unwrap_or(0);
        }
    }
    let ee_card = ee_target.card();
    let total = inputs.today.routed;
    let ribbon = if config_owns_default {
        format!("Everything else \u{00b7} {shown}")
    } else if has_default {
        "Everything else \u{00b7} PwrRoute".to_owned()
    } else {
        "Everything else".to_owned()
    };
    // Everything else counts after every rule.
    let ee_rank = rows.len();
    let mut first_ranks = Vec::new();
    let built: Vec<Card> = cards
        .dests
        .iter()
        .enumerate()
        .map(|(card, dest)| {
            let single = dest.kind != CardKind::Browser;
            let is_ee = ee_card == Some(card);
            // The browser's private-window node, by its word for it: the
            // catalog's for the card's name (which "+ Add a rule…" writes),
            // else the word a route into a private window uses (a browser
            // as written in another way, or a config's flag).
            let own_term = (!single).then(|| private_term(&dest.name)).flatten();
            let term = (!single)
                .then(|| {
                    own_term.clone().or_else(|| {
                        rows.iter()
                            .map(|row| &row.target)
                            .chain(is_ee.then_some(&ee_target))
                            .find_map(|target| match target {
                                Target::Browser {
                                    card: at,
                                    private: Private::Window(term),
                                    ..
                                } if *at == card => Some(term.clone()),
                                _ => None,
                            })
                    })
                })
                .flatten();
            // Whether a target's route ends at a profile of this card (or
            // at its private-window node).
            let lands = |target: &Target, at: Slot<'_>| {
                target.card() == Some(card)
                    && (single
                        || match at {
                            Slot::Private => target.private_window(),
                            Slot::Profile(profile) => {
                                !target.private_window() && same_profile(target.profile(), profile)
                            }
                        })
            };
            let feeds = |row: &Row, at: Slot<'_>| lands(&row.target, at);
            let ee_feeds = |at: Slot<'_>| is_ee && lands(&ee_target, at);
            let first = |at: Slot<'_>| {
                rows.iter()
                    .position(|row| feeds(row, at))
                    .or_else(|| ee_feeds(at).then_some(ee_rank))
                    .unwrap_or(usize::MAX)
            };
            first_ranks.push(
                rows.iter()
                    .position(|row| row.target.card() == Some(card))
                    .or_else(|| is_ee.then_some(ee_rank))
                    .unwrap_or(usize::MAX),
            );

            // Something runs into it: an enabled row, or Everything else. A
            // profile only rules that are off name is idle, kept so their
            // would-be route has an end.
            let live =
                |at: Slot<'_>| ee_feeds(at) || rows.iter().any(|row| row.enabled && feeds(row, at));
            // The routed profiles by their first row, then those only rules
            // that are off name, then the rest the scan found, in its order.
            let mut profiles: Vec<Option<String>> = if single {
                vec![None]
            } else {
                cards.profiles[card].clone()
            };
            profiles.sort_by_key(|profile| {
                let at = Slot::Profile(profile.as_deref());
                (!live(at), first(at))
            });
            let routed = profiles
                .iter()
                .filter(|p| live(Slot::Profile(p.as_deref())))
                .count();
            if !single
                && let Some(app) = installed
                    .iter()
                    .find(|app| app.name.to_lowercase() == dest.key)
            {
                for profile in &app.profiles {
                    if !profiles
                        .iter()
                        .any(|p| same_profile(p.as_deref(), Some(profile)))
                    {
                        profiles.push(Some(profile.clone()));
                    }
                }
            }
            // A browser that has no profile yet still has its default one
            // beside its private-window node.
            if !single && profiles.is_empty() {
                profiles.push(None);
            }
            let card_n = match dest.kind {
                CardKind::Function => function_links,
                CardKind::Ghost => 0,
                _ => links[card],
            };
            // A private link opened in a normal window counts where it
            // opened: on its profile, when the card has no private node.
            let links_at = |at: Slot<'_>| -> u64 {
                profile_links[card]
                    .iter()
                    .filter(|(p, private, _)| match at {
                        Slot::Private => *private,
                        Slot::Profile(profile) => {
                            (!*private || term.is_none()) && same_profile(p.as_deref(), profile)
                        }
                    })
                    .map(|(_, _, n)| n)
                    .sum()
            };
            let slots: Vec<(Option<String>, Option<String>)> = profiles
                .iter()
                .map(|profile| (profile.clone(), None))
                .chain(term.clone().map(|term| (None, Some(term))))
                .collect();
            let nodes: Vec<Node> = slots
                .into_iter()
                .map(|(profile, private)| {
                    let at = if private.is_some() {
                        Slot::Private
                    } else {
                        Slot::Profile(profile.as_deref())
                    };
                    let rules_n = rows
                        .iter()
                        .filter(|row| row.enabled && feeds(row, at))
                        .count();
                    let everything_else = ee_feeds(at);
                    let unused = !live(at);
                    let n = if single { card_n } else { links_at(at) };
                    let node_links = (counts && n > 0 && !unused).then_some(n);
                    let rules_part = (rules_n > 0).then(|| plural(rules_n, "rule"));
                    let meta = if unused {
                        Some("no rules yet".to_owned())
                    } else if dest.kind == CardKind::Ghost {
                        Some(match &rules_part {
                            Some(rules) => format!("{rules} \u{00b7} falls through"),
                            None => "falls through".to_owned(),
                        })
                    } else {
                        rules_part.clone()
                    };
                    let named = if single {
                        dest.name.clone()
                    } else {
                        display(
                            &dest.name,
                            Some(
                                private
                                    .as_deref()
                                    .or(profile.as_deref())
                                    .unwrap_or("default profile"),
                            ),
                        )
                    };
                    let tooltip = match &private {
                        Some(term) if unused => {
                            format!("No rules open {} yet", window_phrase(term))
                        }
                        _ if unused => "No rules send links here yet".to_owned(),
                        _ => {
                            let parts: Vec<String> = rules_part
                                .into_iter()
                                .chain(everything_else.then(|| "Everything else".to_owned()))
                                .chain(node_links.map(|n| format!("{} links today", thousands(n))))
                                .collect();
                            let tooltip = if parts.is_empty() {
                                named
                            } else {
                                format!("{named}: {}", parts.join(", "))
                            };
                            // Whose profiles a private window opens as.
                            let mut named_profiles: Vec<&str> = Vec::new();
                            if private.is_some() {
                                let targets = rows
                                    .iter()
                                    .filter(|row| row.enabled)
                                    .map(|row| &row.target)
                                    .chain(is_ee.then_some(&ee_target));
                                for target in targets.filter(|t| lands(t, Slot::Private)) {
                                    if let Some(profile) = target.profile()
                                        && !named_profiles
                                            .iter()
                                            .any(|p| same_profile(Some(p), Some(profile)))
                                    {
                                        named_profiles.push(profile);
                                    }
                                }
                            }
                            if named_profiles.is_empty() {
                                tooltip
                            } else {
                                format!("{tooltip} \u{00b7} as {}", named_profiles.join(", "))
                            }
                        }
                    };
                    Node {
                        addable: unused && (private.is_none() || own_term.is_some()),
                        profile,
                        private,
                        everything_else,
                        rules: rules_n,
                        links: node_links,
                        unused,
                        meta,
                        tooltip,
                    }
                })
                .collect();

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
            // A browser only rules that are off send links to: with the
            // browsers that have no rules yet.
            let unused_card = dest.kind == CardKind::Browser && feeding == 0 && !is_ee;
            let card_links = (counts && card_n > 0 && !unused_card).then_some(card_n);
            let today = card_links.map(|n| format!("{} links today", thousands(n)));
            let rules_part = if feeding == 0 && is_ee {
                "Everything else".to_owned()
            } else {
                plural(feeding, "rule")
            };
            let meta = match dest.kind {
                CardKind::Browser if unused_card => vec![idle_meta(&nodes)],
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
            let share = (dest.kind == CardKind::Browser && !unused_card && counts && total > 0)
                .then(|| {
                    let fraction = (card_n as f32 / total as f32).clamp(0.0, 1.0);
                    let percent = fraction * 100.0;
                    Share {
                        label: if card_n > 0 && percent < 1.0 {
                            "<1%".to_owned()
                        } else {
                            format!("{}%", percent.round() as u64)
                        },
                        fraction,
                    }
                });
            let shown = if single {
                1
            } else if unused_card {
                folded_to(profiles.len(), routed.max(IDLE_NODES))
            } else {
                folded_to(profiles.len(), routed.max(ROUTED_NODES))
            };
            Card {
                kind: dest.kind,
                badge: dest.badge.clone(),
                name: dest.name.clone(),
                key: dest.key.clone(),
                nodes,
                shown,
                rules: feeding,
                meta,
                links: card_links,
                share,
                ribbon: is_ee.then(|| ribbon.clone()),
                unused: unused_card,
                idle: idle && !is_ee,
            }
        })
        .collect();

    // Every other browser the Settings don't hide, as a place links could
    // go, with no rules yet.
    let mut built = built;
    let spare: Vec<&Installed> = installed
        .iter()
        .filter(|app| {
            !app.hidden
                && !app.name.eq_ignore_ascii_case("PwrRoute")
                && !built.iter().any(|card| card.key == app.name.to_lowercase())
        })
        .collect();
    for app in spare {
        let profiles: Vec<Option<String>> = if app.profiles.is_empty() {
            vec![None]
        } else {
            app.profiles.iter().cloned().map(Some).collect()
        };
        let shown = folded_to(profiles.len(), IDLE_NODES);
        let term = private_term(&app.name);
        let nodes: Vec<Node> = profiles
            .into_iter()
            .map(|profile| (profile, None))
            .chain(term.map(|term| (None, Some(term))))
            .map(|(profile, private)| Node {
                tooltip: match &private {
                    Some(term) => format!("No rules open {} yet", window_phrase(term)),
                    None => "No rules send links here yet".to_owned(),
                },
                profile,
                private,
                everything_else: false,
                rules: 0,
                links: None,
                unused: true,
                addable: true,
                meta: Some("no rules yet".to_owned()),
            })
            .collect();
        first_ranks.push(usize::MAX);
        built.push(Card {
            kind: CardKind::Browser,
            badge: badge_for(&app.name),
            name: app.name.clone(),
            key: app.name.to_lowercase(),
            shown,
            meta: idle_meta(&nodes),
            nodes,
            rules: 0,
            links: None,
            share: None,
            ribbon: None,
            unused: true,
            idle: false,
        });
    }

    // Each route's node, then the cards in order: routed browsers by
    // today's share, the special cards, ghosts, then the browsers with no
    // rules yet, A-Z.
    let mut everything_else = everything_else;
    for row in &mut rows {
        row.node = node_ref(&built, &row.target);
    }
    everything_else.node = node_ref(&built, &everything_else.target);
    let (built, remap) = order_cards(built, &first_ranks);
    for row in &mut rows {
        remap_target(&mut row.target, &remap);
        row.node = row.node.map(|(card, node)| (remap[card], node));
    }
    remap_target(&mut everything_else.target, &remap);
    everything_else.node = everything_else.node.map(|(card, node)| (remap[card], node));

    let in_use = built
        .iter()
        .filter(|card| card.kind == CardKind::Browser && !card.unused)
        .count();
    let spare_count = built.iter().filter(|card| card.unused).count();
    let in_use_line = if spare_count > 0 {
        format!("{in_use} in use \u{00b7} {spare_count} more installed")
    } else {
        format!("{in_use} in use")
    };
    let also = (spare_count > 0)
        .then(|| format!("{} \u{00b7} no rules yet", plural(spare_count, "browser")));

    // The config's rewrites run only once it loads (its description is
    // left out until then); rules.json's run whenever it does.
    let rewrites = if nothing_runs {
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

    let all_sent = matches!(
        status.sending.as_ref().map(|sending| sending.scope),
        Some(pwrroute_router::SendScope::All { .. })
    );
    let map = Map {
        caption: match state {
            State::Paused => Caption::WhenResumes,
            State::Sending if all_sent => Caption::WhenRulesDecide,
            State::NotDefault => Caption::WhatWillHappen,
            _ => Caption::InOrder,
        },
        counts,
        // Paused, or every link sent elsewhere: the rules wait. Next link
        // leaves them lit, since they are back after it.
        dimmed: state == State::Paused || (state == State::Sending && all_sent),
        in_use: in_use_line,
        also,
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
        // The router leaves the config out: the rules still run, and
        // PwrRoute's default takes the rest (design 3c). With no rules
        // running, or nothing loaded at all, every link opens there.
        State::ConfigError => Hero {
            state,
            tone: Tone::Danger,
            sentence: format!("{shown} didn't load."),
            detail: vec![
                if !nothing_runs && rules.rules.iter().any(|rule| rule.enabled) {
                    format!(
                        "Your rules still run. Everything else goes to {pwrroute_display} \
                         until it loads."
                    )
                } else {
                    format!("Every link opens in {pwrroute_display} until it loads.")
                },
            ],
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
        State::Sending => {
            let sending = status.sending.as_ref().expect("Sending has a send");
            Hero {
                state,
                tone: Tone::Accent,
                sentence: sending_sentence(sending),
                detail: vec![sending_detail(sending, SystemTime::now())],
                action: Some(Action::BackToRules),
            }
        }
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
        // Each side as the map names a node. When both are one browser, a
        // side with no profile is its "default profile", so the two read
        // apart; a config default whose `args` ask for a private window is
        // named by it ("Microsoft Edge · InPrivate"), as its ∞ sits on the
        // private-window node. PwrRoute's own default is never private.
        let (ours_dest, ours_profile) = pwrroute_dest();
        let config_default = match &description.default_browser {
            spec @ BrowserSpec::Browser(fixed) if fixed.app_type != AppType::None => {
                let (dest, profile, ask) = spec_dest(spec, installed);
                let profile = canonical_profile(installed, &dest.key, profile);
                let private = match (dest.kind, ask.private()) {
                    (CardKind::Browser, Private::Window(term)) => Some(term),
                    _ => None,
                };
                Some((dest, profile, private))
            }
            _ => None,
        };
        let same_browser = config_default
            .as_ref()
            .is_some_and(|(dest, ..)| dest.key == ours_dest.key);
        let side = |name: &str, profile: Option<&str>| {
            if same_browser {
                display(name, Some(profile.unwrap_or("default profile")))
            } else {
                display(name, profile)
            }
        };
        let ours = side(&ours_dest.name, ours_profile.as_deref());
        // The same browser and profile, and not private: what is left
        // opens where it did.
        let unchanged = same_browser
            && config_default
                .as_ref()
                .is_some_and(|(_, profile, private)| {
                    private.is_none() && same_profile(profile.as_deref(), ours_profile.as_deref())
                });
        let theirs_name = config_default
            .as_ref()
            .map(|(dest, profile, private)| match private {
                Some(term) => display(&dest.name, Some(&display(term, profile.as_deref()))),
                None => side(&dest.name, profile.as_deref()),
            });
        let theirs = match &theirs_name {
            Some(name) => format!("{shown}'s defaultBrowser, {name},"),
            None => format!("{shown}'s defaultBrowser"),
        };
        if !rules_first {
            let instead = match (&theirs_name, &description.default_browser) {
                (Some(name), _) => format!("instead of {name}"),
                (None, BrowserSpec::Function(_)) => {
                    "instead of its defaultBrowser function".to_owned()
                }
                (None, _) => "instead of leaving them unopened".to_owned(),
            };
            Callout::Offer {
                body: if unchanged {
                    format!(
                        "Your rules would run first, then {shown}'s handlers, and what is \
                         left would still open in {pwrroute_display}."
                    )
                } else {
                    format!(
                        "Your rules would run first, then {shown}'s handlers, and {ours} \
                         would take what is left {instead}."
                    )
                },
                warnings: override_warnings(description)
                    .into_iter()
                    .chain(stale.then(|| stale_note(shown)))
                    .collect(),
            }
        } else if inputs.just_overridden {
            Callout::Done {
                body: with_stale_note(if unchanged {
                    format!(
                        "Your rules now run first. What is left still opens in {pwrroute_display}."
                    )
                } else {
                    format!("{theirs} isn't used while this is on.")
                }),
            }
        } else {
            Callout::Settled {
                body: with_stale_note(if unchanged {
                    format!(
                        "Your rules run first. What is left opens in {pwrroute_display} either way."
                    )
                } else {
                    format!("{theirs} isn't used while PwrRoute's default takes everything else.")
                }),
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

/// A card with no rules yet: "4 profiles · no rules yet", "default
/// profile · no rules yet". The private-window node is no profile.
fn idle_meta(nodes: &[Node]) -> String {
    let profiles: Vec<&Node> = nodes.iter().filter(|node| node.private.is_none()).collect();
    match profiles[..] {
        [only] if only.profile.is_none() => "default profile \u{00b7} no rules yet".to_owned(),
        _ => format!(
            "{} \u{00b7} no rules yet",
            plural(profiles.len(), "profile")
        ),
    }
}

/// The node a target's route ends at.
fn node_ref(cards: &[Card], target: &Target) -> Option<NodeRef> {
    let card = target.card()?;
    let c = cards.get(card)?;
    let node = if c.single() {
        0
    } else if target.private_window() {
        c.private_node()?
    } else {
        c.nodes.iter().position(|node| {
            node.private.is_none() && same_profile(node.profile.as_deref(), target.profile())
        })?
    };
    Some((card, node))
}

/// The browser cards by today's links (by their first row while counts
/// are not shown), then Decided by JavaScript, Not opened, and the
/// browsers that are not installed, each by its first row, then the
/// browsers with no rules yet A-Z; and where each old index went. `first`
/// is each card's first feeding rank.
fn order_cards(cards: Vec<Card>, first: &[usize]) -> (Vec<Card>, Vec<usize>) {
    use std::cmp::Reverse;
    let rank = |card: &Card| match card.kind {
        CardKind::Browser if card.unused => 4,
        CardKind::Browser => 0,
        CardKind::Function => 1,
        CardKind::NotOpened => 2,
        CardKind::Ghost => 3,
    };
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by_key(|&i| {
        let card = &cards[i];
        if card.unused {
            // With no rules yet: A-Z.
            return (rank(card), Reverse(0), 0, card.key.clone(), i);
        }
        let links = if card.kind == CardKind::Browser {
            card.links.unwrap_or(0)
        } else {
            0
        };
        (rank(card), Reverse(links), first[i], String::new(), i)
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
            hostnames: None,
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

    fn private(rule: Rule) -> Rule {
        Rule {
            private: true,
            ..rule
        }
    }

    fn fixed(name: &str, profile: &str, args: &[&str]) -> BrowserSpec {
        BrowserSpec::Browser(FixedBrowser {
            name: name.into(),
            app_type: AppType::AppName,
            profile: profile.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            open_in_background: None,
        })
    }

    /// Private rules to one browser share its one private-window node,
    /// whatever profile they name: the profile is the row's detail and
    /// the node's tooltip's, not a node of its own.
    #[test]
    fn private_rules_route_to_the_browsers_private_window() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            rules: vec![
                private(rule("bank.example.com/*", "Google Chrome", "Work")),
                rule("docs.example.com/*", "Google Chrome", "Work"),
                private(rule("shop.example.com/*", "Google Chrome", "Personal")),
                private(rule("news.example.com/*", "Google Chrome", "")),
            ],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        let chrome = &map.cards[0];
        let labels: Vec<&str> = chrome.nodes.iter().map(Node::label).collect();
        // Work for rule 2 and Personal for Everything else: no profile node
        // is made for a private rule.
        assert_eq!(labels, ["Work", "Personal", "Design", "Incognito"]);
        let incognito = chrome.private_node().expect("Chrome's Incognito node");
        assert_eq!(incognito, 3);
        for row in [0, 2, 3] {
            assert_eq!(map.rows[row].node, Some((0, incognito)), "rule {}", row + 1);
        }
        assert_eq!(map.rows[1].node, Some((0, 0)));
        let node = &chrome.nodes[incognito];
        assert!(!node.unused);
        assert_eq!(node.rules, 3);
        assert_eq!(node.meta.as_deref(), Some("3 rules"));
        assert_eq!(
            node.tooltip,
            "Google Chrome \u{b7} Incognito: 3 rules \u{b7} as Work, Personal"
        );
        assert_eq!(chrome.meta, "4 rules", "the card counts private rules too");
        let label = |row: usize| match &map.rows[row].target {
            Target::Browser { label, .. } => label.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(label(0), "Incognito \u{b7} Work");
        assert_eq!(label(3), "Incognito");
        assert_eq!(
            map.rows[0].tooltip,
            "rule 1 \u{b7} opens an Incognito window as Work"
        );
        assert_eq!(
            map.rows[3].tooltip,
            "rule 4 \u{b7} opens an Incognito window"
        );

        // Hovering the node lights its three rows; its key survives a
        // re-sort, and a profile's key never lands on it.
        let lit = map.lit(Some(Focus::Node(0, incognito)));
        assert_eq!(lit.rows.iter().copied().collect::<Vec<_>>(), [0, 2, 3]);
        assert_eq!(lit.fan(0, incognito), 3);
        let key = map.key_of(Focus::Node(0, incognito)).expect("its key");
        assert_eq!(key, FocusKey::Private("google chrome".into()));
        assert_eq!(map.resolve(&key), Some(Focus::Node(0, incognito)));
        assert_eq!(
            map.resolve(&FocusKey::Node("google chrome".into(), None)),
            None,
            "Chrome has no default-profile node here"
        );
        assert_eq!(
            map.focus_label(Focus::Node(0, incognito)).as_deref(),
            Some("Google Chrome \u{b7} Incognito")
        );
    }

    /// Today's private links count on the private-window node, the rest
    /// on their profile.
    #[test]
    fn private_links_count_on_the_private_window() {
        let map = World::rules().build().map;
        let firefox = map
            .cards
            .iter()
            .find(|card| card.name == "Firefox")
            .expect("Firefox's card");
        assert_eq!(firefox.nodes[0].links, Some(57));
        assert_eq!(firefox.nodes[1].private.as_deref(), Some("Private window"));
        assert_eq!(firefox.nodes[1].links, Some(9));
        assert_eq!(firefox.links, Some(66));
    }

    /// A browser with no private mode opens a private rule's link in a
    /// normal window: the route ends at the profile, and the row says so.
    #[test]
    fn a_private_rule_to_a_browser_without_one_opens_a_normal_window() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            rules: vec![
                private(rule("bank.example.com/*", "Safari", "")),
                private(rule("dev.example.com/*", "Arc", "")),
            ],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        for (row, name) in [(0, "Safari"), (1, "Arc")] {
            let target = &map.rows[row].target;
            assert!(target.not_private() && !target.private_window(), "{name}");
            let card = &map.cards[target.card().expect("a card")];
            assert_eq!(card.name, name);
            assert_eq!(card.private_node(), None, "{name} has no private node");
            assert_eq!(map.rows[row].node, Some((target.card().unwrap(), 0)));
            assert!(matches!(target, Target::Browser { label, .. } if label == name));
        }
        assert_eq!(
            map.rows[0].tooltip,
            "rule 1 \u{b7} Safari can't open a private window from a link, so this \
             opens a normal one."
        );
    }

    /// A private window is named as the browser names it. The catalog is
    /// read by the browser's name, as the launcher reads it, so an app it
    /// knows only by the scan's bundle id gets no idle private node.
    #[test]
    fn private_windows_take_the_browsers_own_word() {
        let mut world = World::new(RulesFile::default(), Vec::new());
        world.installed.push(Installed {
            name: "Brave Browser".into(),
            bundle_id: Some("com.brave.Browser".into()),
            profiles: vec!["Personal".into()],
            directories: vec!["Default".into()],
            hidden: false,
        });
        world.installed.push(Installed {
            name: "Chromium Nightly".into(),
            bundle_id: Some("org.chromium.Chromium".into()),
            profiles: Vec::new(),
            directories: Vec::new(),
            hidden: false,
        });
        let map = world.build().map;
        let term = |name: &str| {
            let card = map
                .cards
                .iter()
                .find(|card| card.name == name)
                .unwrap_or_else(|| panic!("{name}'s card"));
            card.private_node()
                .map(|node| card.nodes[node].label().to_owned())
        };
        assert_eq!(term("Google Chrome").as_deref(), Some("Incognito"));
        assert_eq!(term("Microsoft Edge").as_deref(), Some("InPrivate"));
        assert_eq!(term("Firefox").as_deref(), Some("Private window"));
        assert_eq!(term("Brave Browser").as_deref(), Some("Private window"));
        assert_eq!(term("Chromium Nightly"), None);
        assert_eq!(term("Safari"), None);
        assert_eq!(term("Arc"), None);
        let brave = map
            .cards
            .iter()
            .find(|card| card.name == "Brave Browser")
            .expect("Brave's card");
        assert_eq!(brave.meta, "1 profile \u{b7} no rules yet");
        assert_eq!(brave.nodes[1].tooltip, "No rules open a private window yet");
    }

    /// A config handler, or its defaultBrowser, whose args carry a private
    /// flag opens a private window: the Finicky API has no field for it.
    #[test]
    fn a_configs_private_flag_routes_to_the_private_window() {
        let mut description = description();
        description.handlers[1].browser = fixed("Google Chrome", "Work", &["--incognito"]);
        description.default_browser = fixed("Firefox", "", &["--private-window"]);
        let world = World {
            config: Some(Some(description)),
            ..World::new(RulesFile::default(), Vec::new())
        };
        let map = world.build().map;
        let row = &map.rows[1];
        assert!(row.target.private_window());
        let chrome = row.target.card().expect("Chrome");
        assert_eq!(
            row.node,
            Some((chrome, map.cards[chrome].private_node().unwrap()))
        );
        assert_eq!(
            row.tooltip,
            "config handler 2 of ~/.finicky.ts \u{b7} opens an Incognito window as Work"
        );
        // Everything else's ∞ sits on Firefox's private window.
        let (card, node) = map.everything_else.node.expect("Everything else's node");
        assert_eq!(map.cards[card].name, "Firefox");
        assert_eq!(Some(node), map.cards[card].private_node());
        assert!(map.cards[card].nodes[node].everything_else);
    }

    /// The screens' private row targets: a profile named in a private
    /// window, and a browser without one.
    #[test]
    fn the_private_targets_fixture_draws_both_forms() {
        let map = World::new(fixtures::overview_private_rules(), Vec::new())
            .build()
            .map;
        let chrome = &map.rows[6].target;
        assert!(chrome.private_window());
        assert!(
            matches!(chrome, Target::Browser { label, .. } if label == "Incognito \u{b7} Work")
        );
        let safari = &map.rows[7].target;
        assert!(safari.not_private());
    }

    /// A private rule is looked up as the launcher looks it up: by the
    /// browser as written. Written by the app's name, which the catalog
    /// does not know, the launcher adds no flag; written by the bundle id
    /// the catalog knows, it does.
    #[test]
    fn a_private_rule_reads_the_catalog_as_the_launcher_does() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![
                private(rule("a.example.com/*", "Chromium Nightly", "")),
                private(rule("b.example.com/*", "org.chromium.Chromium", "")),
            ],
            ..RulesFile::default()
        };
        let mut world = World::new(rules, Vec::new());
        world.installed.push(Installed {
            name: "Chromium Nightly".into(),
            bundle_id: Some("org.chromium.Chromium".into()),
            profiles: Vec::new(),
            directories: Vec::new(),
            hidden: false,
        });
        world.day = fixtures::in_private(
            fixtures::in_private(
                fixtures::overview_day(
                    &[
                        (
                            Matched::Rule { index: 0 },
                            Some(("Chromium Nightly", "")),
                            5,
                        ),
                        (
                            Matched::Rule { index: 1 },
                            Some(("org.chromium.Chromium", "")),
                            3,
                        ),
                    ],
                    0,
                    0,
                ),
                0,
            ),
            1,
        );
        let map = world.build().map;
        let by_name = &map.rows[0].target;
        assert!(by_name.not_private(), "{by_name:?}");
        let by_id = &map.rows[1].target;
        assert!(by_id.private_window(), "{by_id:?}");
        // One card: the bundle id's route gives it its Incognito node.
        let card = by_id.card().expect("a card");
        assert_eq!(by_name.card(), Some(card));
        let node = map.cards[card].private_node().expect("an Incognito node");
        assert_eq!(map.cards[card].nodes[node].label(), "Incognito");
        assert_eq!(map.rows[1].node, Some((card, node)));
        assert_eq!(map.rows[0].node, Some((card, 0)));
        // Each rule's links count where they opened: the one by name in a
        // normal window, though it asked for a private one.
        assert_eq!(map.cards[card].nodes[0].links, Some(5));
        assert_eq!(map.cards[card].nodes[node].links, Some(3));
    }

    /// "+ Add a rule…" writes the card's name: an idle private-window node
    /// whose word came only from a route naming the browser another way
    /// does not offer it, as a rule by that name would open a normal
    /// window.
    #[test]
    fn add_a_rule_is_offered_where_it_would_land() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![Rule {
                enabled: false,
                ..private(rule("b.example.com/*", "org.chromium.Chromium", ""))
            }],
            ..RulesFile::default()
        };
        let mut world = World::new(rules, Vec::new());
        world.installed.push(Installed {
            name: "Chromium Nightly".into(),
            bundle_id: Some("org.chromium.Chromium".into()),
            profiles: Vec::new(),
            directories: Vec::new(),
            hidden: false,
        });
        let map = world.build().map;
        let card = map.rows[0].target.card().expect("a card");
        let nightly = &map.cards[card];
        let node = nightly.private_node().expect("an Incognito node");
        assert!(nightly.nodes[node].unused);
        assert!(!nightly.nodes[node].addable);
        assert!(nightly.nodes[0].addable, "its default profile");
        // A browser the catalog knows by its name offers it on both.
        let chrome = map
            .cards
            .iter()
            .find(|card| card.name == "Google Chrome")
            .expect("Chrome's card");
        assert!(chrome.nodes.iter().all(|node| node.addable));
    }

    /// A config's private flag goes to the browser as written, so a
    /// browser the catalog does not know still opens a private window: it
    /// gets a private node by the flag's word, not "not private".
    #[test]
    fn a_configs_private_flag_to_an_unknown_browser_is_private() {
        let mut description = description();
        description.handlers[0].browser = fixed("Nightly Build", "", &["--incognito"]);
        let mut world = World {
            config: Some(Some(description)),
            ..World::new(RulesFile::default(), Vec::new())
        };
        world.installed.push(Installed {
            name: "Nightly Build".into(),
            bundle_id: Some("com.example.nightly".into()),
            profiles: Vec::new(),
            directories: Vec::new(),
            hidden: false,
        });
        let map = world.build().map;
        let row = &map.rows[0];
        assert!(row.target.private_window(), "{:?}", row.target);
        assert!(!row.target.not_private());
        let card = row.target.card().expect("a card");
        assert_eq!(map.cards[card].name, "Nightly Build");
        let node = map.cards[card].private_node().expect("a private node");
        assert_eq!(map.cards[card].nodes[node].label(), "Incognito");
        assert_eq!(row.node, Some((card, node)));
        assert_eq!(
            row.tooltip,
            "config handler 1 of ~/.finicky.ts \u{b7} opens an Incognito window"
        );
    }

    /// A config names its browser with an explicit `appType`, which the
    /// launcher looks it up by: a `.app` path given as an app name is no
    /// path to it, so its private flag reaches a browser it does not know.
    #[test]
    fn a_configs_app_type_decides_the_lookup() {
        let mut description = description();
        description.handlers[0].browser = BrowserSpec::Browser(FixedBrowser {
            name: "/Applications/Safari.app".into(),
            app_type: AppType::Path,
            profile: String::new(),
            args: vec!["--private-window".into()],
            open_in_background: None,
        });
        description.handlers[1].browser = BrowserSpec::Browser(FixedBrowser {
            name: "/Applications/Safari.app".into(),
            app_type: AppType::AppName,
            profile: String::new(),
            args: vec!["--private-window".into()],
            open_in_background: None,
        });
        let world = World {
            config: Some(Some(description)),
            ..World::new(RulesFile::default(), Vec::new())
        };
        let map = world.build().map;
        // By path, the catalog's Safari on macOS, which has no private
        // window. Elsewhere the launcher reads it as this platform's path
        // and knows no browser there, so the flag goes as written.
        let by_path = &map.rows[0].target;
        if cfg!(target_os = "macos") {
            assert!(by_path.not_private(), "{by_path:?}");
        } else {
            assert!(by_path.private_window(), "{by_path:?}");
        }
        // By that name, nothing the catalog knows: the flag goes as written.
        assert!(
            map.rows[1].target.private_window(),
            "{:?}",
            map.rows[1].target
        );
    }

    /// The callout names both defaults as the map names a node: by browser
    /// when they differ, by profile when the browser is the same, and says
    /// plainly when nothing would change.
    #[test]
    fn the_callout_names_both_defaults_by_how_they_compare() {
        let bodies = |ours: (&str, &str), theirs: BrowserSpec| {
            let rules = |precedence| RulesFile {
                default_browser: ours.0.into(),
                default_profile: ours.1.into(),
                precedence,
                ..RulesFile::default()
            };
            let mut description = description();
            description.default_browser = theirs;
            let world = |precedence, just_overridden| World {
                config: Some(Some(description.clone())),
                just_overridden,
                ..World::new(rules(precedence), Vec::new())
            };
            let body = |callout: Option<Callout>| match callout.expect("a callout") {
                Callout::Offer { body, .. }
                | Callout::Done { body }
                | Callout::Settled { body } => body,
            };
            [
                body(world(Precedence::Config, false).build().callout),
                body(world(Precedence::PwrRoute, true).build().callout),
                body(world(Precedence::PwrRoute, false).build().callout),
            ]
        };
        let lead = "Your rules would run first, then ~/.finicky.ts's handlers, and";

        // 1. Different browsers.
        assert_eq!(
            bodies(("Google Chrome", "Personal"), fixed("Safari", "", &[])),
            [
                format!(
                    "{lead} Google Chrome \u{b7} Personal would take what is left instead of Safari."
                ),
                "~/.finicky.ts's defaultBrowser, Safari, isn't used while this is on.".into(),
                "~/.finicky.ts's defaultBrowser, Safari, isn't used while PwrRoute's default \
                 takes everything else."
                    .into(),
            ]
        );

        // 2. The same browser, other profiles: named by profile, and a
        // side with none is the default profile.
        assert_eq!(
            bodies(
                ("Microsoft Edge", "Testing"),
                fixed("Microsoft Edge", "Personal", &[])
            ),
            [
                format!(
                    "{lead} Microsoft Edge \u{b7} Testing would take what is left instead of \
                     Microsoft Edge \u{b7} Personal."
                ),
                "~/.finicky.ts's defaultBrowser, Microsoft Edge \u{b7} Personal, isn't used \
                 while this is on."
                    .into(),
                "~/.finicky.ts's defaultBrowser, Microsoft Edge \u{b7} Personal, isn't used \
                 while PwrRoute's default takes everything else."
                    .into(),
            ]
        );
        assert_eq!(
            bodies(
                ("Microsoft Edge", "Testing"),
                fixed("Microsoft Edge", "", &[])
            )[0],
            format!(
                "{lead} Microsoft Edge \u{b7} Testing would take what is left instead of \
                 Microsoft Edge \u{b7} default profile."
            )
        );

        // 3. The same browser and profile (a directory names the same
        // one): what is left opens where it did.
        assert_eq!(
            bodies(
                ("Microsoft Edge", "Personal"),
                fixed("Microsoft Edge", "Profile 1", &[])
            ),
            [
                format!("{lead} what is left would still open in Microsoft Edge \u{b7} Personal."),
                "Your rules now run first. What is left still opens in Microsoft Edge \u{b7} \
                 Personal."
                    .into(),
                "Your rules run first. What is left opens in Microsoft Edge \u{b7} Personal \
                 either way."
                    .into(),
            ]
        );
        assert_eq!(
            bodies(("Safari", ""), fixed("Safari", "", &[]))[0],
            format!("{lead} what is left would still open in Safari.")
        );

        // 4. The same browser and profile, but the config's opens an
        // InPrivate window: what is left would move, so it is named.
        assert_eq!(
            bodies(
                ("Microsoft Edge", ""),
                fixed("Microsoft Edge", "", &["--inprivate"])
            ),
            [
                format!(
                    "{lead} Microsoft Edge \u{b7} default profile would take what is left \
                     instead of Microsoft Edge \u{b7} InPrivate."
                ),
                "~/.finicky.ts's defaultBrowser, Microsoft Edge \u{b7} InPrivate, isn't used \
                 while this is on."
                    .into(),
                "~/.finicky.ts's defaultBrowser, Microsoft Edge \u{b7} InPrivate, isn't used \
                 while PwrRoute's default takes everything else."
                    .into(),
            ]
        );
        assert_eq!(
            bodies(
                ("Google Chrome", "Work"),
                fixed("Firefox", "", &["--private-window"])
            )[0],
            format!(
                "{lead} Google Chrome \u{b7} Work would take what is left instead of \
                 Firefox \u{b7} Private window."
            )
        );
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
            .find(|(name, profile, private, _)| {
                *name == "Google Chrome" && *profile == "Work" && !private
            })
            .map(|(_, _, _, n)| n);
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
            RowMatch::Patterns {
                patterns: vec![
                    Pattern {
                        text: "*.docs.example.com/*".into(),
                        regex: false,
                    },
                    Pattern {
                        text: "docs.example.org/*".into(),
                        regex: false,
                    },
                ],
                more: 0,
                opener: None,
            }
        );
        assert_eq!(map.rows[0].count, Some(214));
        assert_eq!(map.rows[0].tooltip, "rule 1");
        assert_eq!(
            map.rows[2].matcher,
            RowMatch::Opener(Opener {
                badge: badge_for("Slack"),
                name: "Slack".into(),
                more: 0,
            }),
            "\"*\" from an app reads as the app"
        );
        assert!(!map.rows[5].enabled);
        assert_eq!(map.rows[5].count, None, "a rule that is off counts nothing");

        // Routed browsers by today's share, then every other browser the
        // scan found, A-Z, with no rules yet.
        assert_eq!(
            names(map),
            [
                "Google Chrome",
                "Arc",
                "Firefox",
                "Microsoft Edge",
                "Safari"
            ]
        );
        assert_eq!(map.in_use, "3 in use \u{b7} 2 more installed");
        assert_eq!(map.also.as_deref(), Some("2 browsers \u{b7} no rules yet"));
        let chrome = &map.cards[0];
        assert_eq!(
            chrome.ribbon.as_deref(),
            Some("Everything else \u{b7} PwrRoute")
        );
        // Work (rules 1 and 2), Personal (Everything else, and rule 6 that
        // is off), then Design, which no rule names.
        let nodes: Vec<(&str, usize, Option<u64>, bool, bool)> = chrome
            .nodes
            .iter()
            .map(|node| {
                (
                    node.label(),
                    node.rules,
                    node.links,
                    node.everything_else,
                    node.unused,
                )
            })
            .collect();
        assert_eq!(
            nodes,
            [
                ("Work", 2, Some(302), false, false),
                ("Personal", 0, Some(642), true, false),
                ("Design", 0, None, false, true),
                ("Incognito", 0, None, false, true),
            ]
        );
        // Chrome's private window: last, never folded, no profile.
        assert_eq!(chrome.private_node(), Some(3));
        assert_eq!(chrome.nodes[3].profile, None);
        assert_eq!(
            chrome.nodes[3].tooltip,
            "No rules open an Incognito window yet"
        );
        assert_eq!(chrome.profiles(), 3);
        assert_eq!(chrome.visible(false), [0, 1, 2, 3]);
        assert_eq!(chrome.nodes[0].meta.as_deref(), Some("2 rules"));
        assert_eq!(chrome.nodes[1].meta, None);
        assert_eq!(chrome.nodes[2].meta.as_deref(), Some("no rules yet"));
        assert_eq!(
            chrome.nodes[0].tooltip,
            "Google Chrome \u{b7} Work: 2 rules, 302 links today"
        );
        assert_eq!(
            chrome.nodes[1].tooltip,
            "Google Chrome \u{b7} Personal: Everything else, 642 links today"
        );
        assert_eq!(chrome.nodes[2].tooltip, "No rules send links here yet");
        assert_eq!(chrome.shown, 3);
        assert_eq!(chrome.meta, "2 rules \u{b7} 944 links today");
        assert_eq!(chrome.share.as_ref().map(|s| s.label.as_str()), Some("67%"));
        assert_eq!(map.cards[1].meta, "1 rule \u{b7} 402 links today");
        assert_eq!(map.cards[1].nodes[0].label(), "default profile");
        // Each row's route ends at its node.
        assert_eq!(map.rows[0].node, Some((0, 0)));
        assert_eq!(map.rows[1].node, Some((0, 0)));
        assert_eq!(map.rows[2].node, Some((2, 0)), "Firefox");
        assert_eq!(map.rows[3].node, Some((1, 0)), "Arc");
        assert_eq!(map.rows[4].node, Some((2, 1)), "Firefox's private window");
        assert_eq!(map.rows[5].node, Some((0, 1)), "off, but Personal's");
        assert_eq!(map.everything_else.node, Some((0, 1)));

        // Rule 5 opens a private Firefox window: its route ends at the
        // private-window node, and the card counts it with the rest.
        let firefox = &map.cards[2];
        let nodes: Vec<(&str, Option<&str>, Option<u64>)> = firefox
            .nodes
            .iter()
            .map(|node| (node.label(), node.meta.as_deref(), node.links))
            .collect();
        assert_eq!(
            nodes,
            [
                ("default profile", Some("1 rule"), Some(57)),
                ("Private window", Some("1 rule"), Some(9)),
            ]
        );
        assert_eq!(firefox.meta, "2 rules \u{b7} 66 links today");
        assert_eq!(
            firefox.nodes[1].tooltip,
            "Firefox \u{b7} Private window: 1 rule, 9 links today"
        );
        assert!(map.rows[4].target.private_window());
        assert!(matches!(
            &map.rows[4].target,
            Target::Browser { label, .. } if label == "Private window"
        ));
        assert_eq!(map.rows[4].tooltip, "rule 5 \u{b7} opens a private window");
        // Arc has no private mode: its card ends with its profile.
        assert_eq!(map.cards[1].nodes.len(), 1);
        assert_eq!(map.cards[1].private_node(), None);

        // The browsers with no rules yet: quiet, Edge folding two of four
        // profiles, its InPrivate node after them and not counted.
        let edge = &map.cards[3];
        assert!(edge.unused && edge.share.is_none());
        assert_eq!(edge.meta, "4 profiles \u{b7} no rules yet");
        assert_eq!((edge.nodes.len(), edge.shown, edge.folded()), (5, 2, 2));
        assert_eq!(edge.visible(false), [0, 1, 4]);
        assert_eq!(edge.visible(true), [0, 1, 2, 3, 4]);
        assert_eq!(edge.nodes[4].label(), "InPrivate");
        assert_eq!(
            edge.nodes[4].tooltip,
            "No rules open an InPrivate window yet"
        );
        assert!(edge.nodes.iter().all(|node| node.unused));
        let safari = &map.cards[4];
        assert_eq!(safari.meta, "default profile \u{b7} no rules yet");
        assert_eq!(safari.nodes[0].label(), "default profile");
        assert_eq!(safari.nodes.len(), 1, "Safari has no private window");

        assert_eq!(map.everything_else.owner, "PwrRoute's default");
        assert_eq!(map.everything_else.count, Some(642));
        assert_eq!(map.everything_else.target.card(), Some(0));
    }

    fn lit_routes(lit: &Lit) -> Vec<End> {
        lit.routes
            .iter()
            .filter(|route| route.trace == Trace::Lit)
            .map(|route| route.from)
            .collect()
    }

    /// At rest every enabled rule and Everything else has a route, and the
    /// rule that is off has none.
    #[test]
    fn every_route_is_drawn_at_rest() {
        let map = World::rules().build().map;
        let lit = map.lit(None);
        assert!(!lit.traced);
        let from: Vec<End> = lit.routes.iter().map(|route| route.from).collect();
        assert_eq!(
            from,
            [
                End::Row(0),
                End::Row(1),
                End::Row(2),
                End::Row(3),
                End::Row(4),
                End::EverythingElse
            ]
        );
        assert!(
            lit.routes
                .iter()
                .all(|route| route.trace == Trace::Rest && route.running)
        );
        // Work's two routes fan into its port in their rows' order.
        let work: Vec<(usize, usize)> = lit
            .routes
            .iter()
            .filter(|route| route.to == (0, 0))
            .map(|route| (route.slot, route.fan))
            .collect();
        assert_eq!(work, [(0, 2), (1, 2)]);
        assert_eq!(lit.fan(0, 0), 2);
        assert!(!lit.any_not_running());
    }

    #[test]
    fn hovering_a_row_lights_its_route_port_and_node() {
        let map = World::rules().build().map;
        let lit = map.lit(Some(Focus::Row(1)));
        assert!(lit.traced);
        assert_eq!(lit.rows, BTreeSet::from([1]));
        assert!(lit.node(0, 0), "rule 2 sends to Work");
        assert!(lit.warm(0) && !lit.card(0), "Chrome warms, not fully lit");
        assert!(!lit.warm(1));
        assert!(!lit.everything_else);
        assert_eq!(lit_routes(&lit), [End::Row(1)]);
        // Every other route dims.
        assert!(
            lit.routes
                .iter()
                .filter(|route| route.from != End::Row(1))
                .all(|route| route.trace == Trace::Dim)
        );
    }

    #[test]
    fn hovering_a_card_lights_every_route_into_its_nodes() {
        let map = World::rules().build().map;
        let lit = map.lit(Some(Focus::Card(0)));
        assert!(lit.card(0) && lit.warm(0));
        assert_eq!(lit.rows, BTreeSet::from([0, 1]), "rule 6 is off");
        assert!(lit.everything_else);
        assert!(lit.node(0, 0) && lit.node(0, 1) && !lit.node(0, 2));
        assert_eq!(
            lit_routes(&lit),
            [End::Row(0), End::Row(1), End::EverythingElse]
        );

        let work = map.lit(Some(Focus::Node(0, 0)));
        assert_eq!(work.rows, BTreeSet::from([0, 1]));
        assert!(!work.everything_else);
        assert!(work.node(0, 0) && !work.node(0, 1) && !work.card(0));
        assert_eq!(lit_routes(&work), [End::Row(0), End::Row(1)]);

        let personal = map.lit(Some(Focus::Node(0, 1)));
        assert!(personal.rows.is_empty(), "rule 6 is off");
        assert!(personal.everything_else);
        assert_eq!(lit_routes(&personal), [End::EverythingElse]);

        let everything_else = map.lit(Some(Focus::EverythingElse));
        assert!(everything_else.node(0, 1));
        assert_eq!(lit_routes(&everything_else), [End::EverythingElse]);
    }

    /// The old map drew no curve at all past six rows into one place; the
    /// graph draws every one, fanned into the port.
    #[test]
    fn a_node_fed_by_seven_rows_lights_every_route() {
        let world = World::new(
            fixtures::overview_fanin_rules(),
            fixtures::overview_fanin_day(),
        );
        let map = world.build().map;
        let edge = names(&map)
            .iter()
            .position(|n| *n == "Microsoft Edge")
            .unwrap();
        assert_eq!(edge, 0, "it takes most of today's links");
        let rnd = &map.cards[edge].nodes[0];
        assert_eq!(rnd.label(), "Research and Development Team");
        assert_eq!(rnd.rules, 7);
        assert!(rnd.everything_else);
        let lit = map.lit(Some(Focus::Node(edge, 0)));
        assert_eq!(lit.rows, (0..7).collect::<BTreeSet<_>>());
        assert!(lit.everything_else);
        let routes: Vec<&Route> = lit.routes.iter().filter(|r| r.to == (edge, 0)).collect();
        assert_eq!(routes.len(), 8);
        assert!(routes.iter().all(|route| route.trace == Trace::Lit));
        // In row order, so they never cross at the port.
        let slots: Vec<usize> = routes.iter().map(|route| route.slot).collect();
        assert_eq!(slots, (0..8).collect::<Vec<_>>());
        assert!(routes.iter().all(|route| route.fan == 8));
        assert_eq!(lit_routes(&lit).len(), 8);
        // The other three rows' routes dim.
        assert_eq!(
            lit.routes
                .iter()
                .filter(|route| route.trace == Trace::Dim)
                .count(),
            3
        );
        // The whole card lights them too.
        assert_eq!(lit_routes(&map.lit(Some(Focus::Card(edge)))).len(), 8);
    }

    #[test]
    fn a_pin_traces_until_it_is_cleared() {
        // The pin is a focus like any other; the window keeps it over the
        // pointer and clears it on Esc. Here: what it lights, and that
        // nothing does once it is gone.
        let map = World::rules().build().map;
        let pinned = map.lit(Some(Focus::Node(0, 0)));
        assert_eq!(lit_routes(&pinned), [End::Row(0), End::Row(1)]);
        assert_eq!(
            map.focus_label(Focus::Node(0, 0)).as_deref(),
            Some("Google Chrome \u{b7} Work")
        );
        assert_eq!(map.focus_label(Focus::Row(2)).as_deref(), Some("rule 3"));
        let cleared = map.lit(None);
        assert!(!cleared.traced);
        assert!(
            cleared
                .routes
                .iter()
                .all(|route| route.trace == Trace::Rest)
        );
    }

    #[test]
    fn a_rule_that_is_off_draws_its_route_only_while_traced() {
        let map = World::rules().build().map;
        assert!(
            !map.lit(None)
                .routes
                .iter()
                .any(|route| route.from == End::Row(5))
        );
        let lit = map.lit(Some(Focus::Row(5)));
        let route = lit
            .routes
            .iter()
            .find(|route| route.from == End::Row(5))
            .expect("its would-be route");
        assert_eq!(route.trace, Trace::Lit);
        assert!(!route.running, "drawn as not running");
        assert!(lit.node(0, 1));
        // Its node's trace does not light it.
        assert!(!map.lit(Some(Focus::Node(0, 1))).row(5));
    }

    #[test]
    fn nothing_runs_while_paused_or_before_the_config_loads() {
        let mut world = World::rules();
        world.status.paused = Some(Pause {
            name: "Google Chrome".into(),
            display: "Google Chrome \u{b7} Personal".into(),
        });
        let lit = world.build().map.lit(None);
        assert!(lit.routes.iter().all(|route| !route.running));
        assert!(lit.any_not_running());

        // Nothing loaded, rules.json included: rules.json's rows do not
        // run, while Everything else (the fallback) does.
        let mut world = World::config();
        world.config = Some(None);
        world.status.config_error = Some(ConfigTrouble::NothingLoaded("x".into()));
        let lit = world.build().map.lit(None);
        for route in &lit.routes {
            assert_eq!(
                route.running,
                route.from == End::EverythingElse,
                "{route:?}"
            );
        }

        // The config never loaded but rules.json did: the router runs its
        // rows without the config, so every enabled, installed one runs.
        world.status.config_error = Some(ConfigTrouble::NotLoaded("x".into()));
        let map = world.build().map;
        let lit = map.lit(None);
        for route in &lit.routes {
            let expected = match route.from {
                End::Row(row) => map.rows[row].enabled && !map.rows[row].target.ghost(),
                _ => true,
            };
            assert_eq!(route.running, expected, "{route:?}");
        }
        assert!(
            lit.routes
                .iter()
                .any(|route| matches!(route.from, End::Row(_)) && route.running)
        );

        // A browser that is not installed: its route is not running.
        let map = World::config().build().map;
        let ghost = map
            .rows
            .iter()
            .position(|row| row.target.ghost())
            .expect("Orion's row");
        let lit = map.lit(None);
        let route = lit
            .routes
            .iter()
            .find(|route| route.from == End::Row(ghost))
            .unwrap();
        assert!(!route.running);
    }

    #[test]
    fn the_keyboard_steps_through_rows_then_nodes() {
        let map = World::rules().build().map;
        let order = map.keyboard_order(|_| false);
        assert_eq!(order[0], Focus::Row(0));
        assert_eq!(order[6], Focus::EverythingElse);
        assert_eq!(order[7], Focus::Node(0, 0));
        // A card's private window comes after its profiles.
        assert_eq!(order[10], Focus::Node(0, 3));
        // Edge shows two of its four profiles until it is opened, then its
        // InPrivate node.
        let edge = 3;
        let edge_nodes = |order: &[Focus]| -> Vec<Focus> {
            order
                .iter()
                .filter(|focus| matches!(focus, Focus::Node(card, _) if *card == edge))
                .copied()
                .collect()
        };
        assert_eq!(
            edge_nodes(&order),
            [
                Focus::Node(edge, 0),
                Focus::Node(edge, 1),
                Focus::Node(edge, 4)
            ]
        );
        let open = map.keyboard_order(|card| card.name == "Microsoft Edge");
        assert_eq!(open.len(), order.len() + 2);
        assert_eq!(edge_nodes(&open).last(), Some(&Focus::Node(edge, 4)));
    }

    #[test]
    fn map_widths_follow_the_window() {
        // The content width at 1000, 1240, 1440, 1600, 1880 and 2560:
        // both columns stop at their caps, and the routes take the rest.
        let at = |content: f32| {
            let w = MapWidths::of_content(content);
            (w.rule, w.gutter, w.dest)
        };
        assert_eq!(at(722.0), (360.0, 96.0, 266.0));
        assert_eq!(at(962.0), (520.0, 96.0, 346.0));
        assert_eq!(at(1162.0), (646.0, 96.0, 420.0));
        assert_eq!(at(1322.0), (720.0, 182.0, 420.0));
        assert_eq!(at(1602.0), (720.0, 462.0, 420.0));
        assert_eq!(at(2282.0), (720.0, 1142.0, 420.0));
        // The breakpoints: destinations reach 420 at a 1424 window, the
        // rules 720 at 1514.
        assert_eq!(at(1146.0).2, 420.0);
        assert!(at(1140.0).2 < 420.0);
        assert_eq!(at(1236.0), (720.0, 96.0, 420.0));
        assert!(at(1240.0).1 > 96.0);
        // Never wider than the content, and never under the gutter's 96.
        for content in [0.0, 300.0, 722.0, 1500.0, 4000.0] {
            let w = MapWidths::of_content(content);
            assert!(w.gutter >= GUTTER_MIN);
            if content >= 458.0 {
                assert_eq!(w.rule + w.gutter + w.dest, content, "{content}");
            }
        }
    }

    #[test]
    fn routes_fan_into_a_capsule_no_taller_than_25() {
        assert_eq!(fan_spread(1), (0.0, 0.0));
        assert_eq!(fan_spread(2), (3.0, 3.0));
        assert_eq!(fan_spread(4), (3.0, 9.0));
        let (step, spread) = fan_spread(9);
        assert!((step - 2.0).abs() < f32::EPSILON && (spread - 16.0).abs() < f32::EPSILON);
        let (_, spread) = fan_spread(40);
        assert!(spread <= 16.0 + f32::EPSILON);
    }

    #[test]
    fn tags_fit_whole_then_count_the_rest() {
        let plus = |_: usize| 14.0;
        // All three fit.
        assert_eq!(fit_tags(&[50.0, 60.0, 70.0], 0, 200.0, plus), 3);
        // Two and "+1".
        assert_eq!(fit_tags(&[50.0, 60.0, 70.0], 0, 140.0, plus), 2);
        // Not even one beside "+2": the first shows anyway, cut short.
        assert_eq!(fit_tags(&[150.0, 60.0, 70.0], 0, 100.0, plus), 1);
        // Matchers that are not tags count into "+N".
        assert_eq!(fit_tags(&[50.0], 2, 60.0, plus), 1);
        assert_eq!(fit_tags(&[], 0, 100.0, plus), 0);
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
        // A named function: its name, then what it tests.
        let RowMatch::Function {
            name,
            excerpt,
            source,
            more,
        } = &map.rows[0].matcher
        else {
            panic!("{:?}", map.rows[0].matcher);
        };
        assert_eq!(name.as_deref(), Some("isWorkplace"));
        assert_eq!(
            excerpt.as_deref(),
            Some("url.host.endsWith(\"workplace.example.com\")")
        );
        assert!(source.as_deref().is_some_and(|s| s.contains("isWorkplace")));
        assert_eq!(*more, 0);
        assert_eq!(map.rows[5].source, RowSource::Rule(0));
        assert_eq!(map.rows[5].rank, 6);

        assert_eq!(map.everything_else.owner, "~/.finicky.ts's default");
        // Browsers by today's share, the special cards, then the one with
        // no rules yet.
        assert_eq!(
            names(map),
            [
                "Safari",
                "Arc",
                "Google Chrome",
                "Firefox",
                "Decided by JavaScript",
                "Orion",
                "Microsoft Edge",
            ]
        );
        assert_eq!(map.in_use, "4 in use \u{b7} 1 more installed");
        assert_eq!(map.also.as_deref(), Some("1 browser \u{b7} no rules yet"));
        assert_eq!(map.cards[2].meta, "3 rules \u{b7} 314 links today");
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
        assert_eq!(function.nodes.len(), 1);
        assert_eq!(function.nodes[0].meta.as_deref(), Some("1 rule"));
        assert_eq!(function.nodes[0].links, Some(9));
        let ghost = &map.cards[5];
        assert_eq!(
            (ghost.kind, ghost.name.as_str()),
            (CardKind::Ghost, "Orion")
        );
        assert_eq!(
            ghost.meta,
            "1 rule \u{b7} its links fall through to Everything else"
        );
        assert_eq!(
            ghost.nodes[0].meta.as_deref(),
            Some("1 rule \u{b7} falls through")
        );
        assert!(matches!(map.rows[4].target, Target::Ghost { .. }));
        assert_eq!(map.rows[4].node, Some((5, 0)));

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

    fn sending(scope: pwrroute_router::SendScope) -> Sending {
        Sending {
            scope,
            name: "Google Chrome".into(),
            display: "Google Chrome \u{b7} Work".into(),
            app: "Google Chrome".into(),
            profile: Some("Work".into()),
            private: false,
        }
    }

    /// Send Links 2a: the hero says where the next link goes, in the
    /// accent tone, with Back to rules; the map stays lit and the node is
    /// found.
    #[test]
    fn next_link_2a_names_the_destination_and_offers_back_to_rules() {
        use pwrroute_router::SendScope;
        let mut world = World::rules();
        world.status.sending = Some(sending(SendScope::Next { left: 1, of: 1 }));
        let overview = world.build();
        assert_eq!(overview.hero.state, State::Sending);
        assert_eq!(overview.hero.tone, Tone::Accent);
        assert_eq!(
            overview.hero.sentence,
            "Next link \u{2192} Google Chrome \u{b7} Work."
        );
        assert_eq!(
            overview.hero.detail,
            ["Then your rules decide again. Rewrites still run."]
        );
        assert_eq!(overview.hero.action, Some(Action::BackToRules));
        assert!(overview.hero.action.unwrap().primary());
        assert!(!overview.map.dimmed);
        assert_eq!(overview.map.caption, Caption::InOrder);
        let sending = world.status.sending.as_ref().unwrap();
        let marked: Vec<(&str, &str)> = overview
            .map
            .cards
            .iter()
            .flat_map(|card| {
                card.nodes
                    .iter()
                    .filter(|node| sending.is_node(card, node))
                    .map(|node| (card.name.as_str(), node.label()))
            })
            .collect();
        assert_eq!(marked, [("Google Chrome", "Work")]);

        world.status.sending = Some(super::Sending {
            scope: SendScope::Next { left: 2, of: 3 },
            ..sending.clone()
        });
        let overview = world.build();
        assert_eq!(
            overview.hero.sentence,
            "Next 2 links \u{2192} Google Chrome \u{b7} Work."
        );
        assert_eq!(
            overview.hero.detail,
            ["2 of 3 left, then your rules decide again. Rewrites still run."]
        );
    }

    /// 2b: all links for the hour dims the rules under "When rules decide
    /// again"; a pause still comes first.
    #[test]
    fn all_links_2b_dims_the_rules_and_a_pause_comes_first() {
        use pwrroute_router::SendScope;
        let mut world = World::rules();
        world.status.sending = Some(sending(SendScope::All {
            until: Some(SystemTime::now() + std::time::Duration::from_secs(52 * 60 - 5)),
        }));
        let overview = world.build();
        assert_eq!(
            overview.hero.sentence,
            "All links \u{2192} Google Chrome \u{b7} Work."
        );
        assert_eq!(
            overview.hero.detail,
            ["For 52 more minutes, then your rules decide again. Rewrites still run."]
        );
        assert_eq!(overview.map.caption, Caption::WhenRulesDecide);
        assert!(overview.map.dimmed);
        world.status.sending = Some(sending(SendScope::All { until: None }));
        assert_eq!(
            world.build().hero.detail,
            ["Until you go back to rules. Rewrites still run."]
        );
        world.status.paused = Some(Pause {
            name: "Google Chrome".into(),
            display: "Google Chrome \u{b7} Personal".into(),
        });
        assert_eq!(world.status.state(), State::Paused);
        world.status.paused = None;
        world.status.default = DefaultState::Other("Safari".into());
        assert_eq!(
            world.status.state(),
            State::Sending,
            "sending before not default"
        );
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
                badge_for("Google Chrome"),
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
    fn config_error_3c_runs_the_rules_without_the_config_until_it_loads() {
        // The router leaves the config out and routes with rules.json
        // alone: its rules run, and PwrRoute's default takes the rest.
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
            [
                "Your rules still run. Everything else goes to Google Chrome \u{b7} Personal \
                 until it loads."
            ]
        );
        assert_eq!(overview.hero.action, Some(Action::ShowError));
        assert_eq!(overview.tiles[0].count, 212);
        assert!(overview.tiles[3].danger, "two failed");
        assert!(overview.callout.is_none());
        let map = overview.map;
        assert!(map.items.iter().any(|item| matches!(
            item,
            Item::Error { title, detail }
                if title == "~/.finicky.ts didn't load: links skip it"
                    && detail.starts_with("SyntaxError")
        )));
        assert_eq!(
            headers(&map),
            [
                ("~/.finicky.ts \u{b7} not running", ""),
                ("Your rules \u{b7} run without it", "3 rules"),
            ]
        );
        assert_eq!(map.rows.len(), 3, "only the visual rules");
        assert_eq!(
            map.rows.iter().map(|row| row.count).collect::<Vec<_>>(),
            [Some(66), Some(12), Some(3)]
        );
        assert!(map.rows.iter().all(|row| !row.idle));
        assert!(map.cards.iter().all(|card| !card.idle));
        assert_eq!(map.everything_else.owner, "PwrRoute's default");
        assert_eq!(map.everything_else.count, Some(131));
        assert_eq!(map.cards[0].name, "Google Chrome");

        // No rule enabled: every link opens in PwrRoute's default.
        for rule in &mut world.rules.rules {
            rule.enabled = false;
        }
        assert_eq!(
            world.build().hero.detail,
            ["Every link opens in Google Chrome \u{b7} Personal until it loads."]
        );
    }

    #[test]
    fn nothing_loaded_sends_every_link_to_the_fallback() {
        // Neither the config nor rules.json loaded: the router's stand-in
        // engine routes nothing, not even rules.json's rules or rewrites.
        let mut world = World::config();
        world.config = Some(None);
        world.day = fixtures::overview_config_error_day();
        world.status.config_error = Some(ConfigTrouble::NothingLoaded(
            "SyntaxError: Unexpected token \u{b7} line 42".into(),
        ));
        let overview = world.build();
        assert_eq!(overview.hero.state, State::ConfigError);
        assert_eq!(
            overview.hero.detail,
            ["Every link opens in Google Chrome \u{b7} Personal until it loads."]
        );
        let map = overview.map;
        assert!(map.items.iter().any(|item| matches!(
            item,
            Item::Error { title, .. }
                if title == "~/.finicky.ts didn't load: nothing runs until it does"
        )));
        assert_eq!(
            headers(&map),
            [
                ("~/.finicky.ts \u{b7} not running", ""),
                ("Your rules \u{b7} not running", "3 rules"),
            ]
        );
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
        // One browser is drawn like any other: its Personal node takes
        // Everything else, and its other profiles are there to route to.
        let chrome = &overview.map.cards[0];
        assert_eq!(chrome.name, "Google Chrome");
        assert_eq!(chrome.meta, "Everything else \u{b7} 23 links today");
        let nodes: Vec<(&str, bool, bool)> = chrome
            .nodes
            .iter()
            .map(|node| (node.label(), node.everything_else, node.unused))
            .collect();
        assert_eq!(
            nodes,
            [
                ("Personal", true, false),
                ("Work", false, true),
                ("Design", false, true),
                ("Incognito", false, true)
            ]
        );
        assert_eq!(chrome.nodes[0].links, Some(23));
        assert_eq!(overview.map.in_use, "1 in use \u{b7} 4 more installed");
        assert!(overview.map.cards[1..].iter().all(|card| card.unused));
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
        status.config_error = Some(ConfigTrouble::NothingLoaded("x".into()));
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
        assert_eq!(
            names(&map),
            [
                "Safari",
                "Arc",
                "Firefox",
                "Google Chrome",
                "Microsoft Edge"
            ]
        );
        let safari = &map.cards[0];
        assert_eq!(safari.ribbon.as_deref(), Some("Everything else"));
        assert_eq!(safari.meta, "Everything else \u{b7} 23 links today");
        assert!(safari.share.is_some(), "a lone browser has its share");
        assert!(safari.nodes[0].everything_else);
        assert_eq!(map.everything_else.node, Some((0, 0)));
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
        // After the routed browsers, before the ones with no rules yet.
        let kinds: Vec<(CardKind, bool)> = map
            .cards
            .iter()
            .map(|card| (card.kind, card.unused))
            .collect();
        assert_eq!(
            kinds[..2],
            [(CardKind::Browser, false), (CardKind::Ghost, false)]
        );
        assert!(kinds[2..].iter().all(|&(_, unused)| unused));

        // No scan yet: no ghost, and no browser to list as idle.
        world.installed.clear();
        let map = world.build().map;
        assert!(matches!(map.rows[0].target, Target::Browser { .. }));
        assert_eq!(map.in_use, "2 in use");
        assert_eq!(map.also, None);
    }

    /// Hidden in Settings › Browsers: off the map, unless a rule names it.
    #[test]
    fn hidden_browsers_stay_off_the_map() {
        let mut world = World::rules();
        for app in &mut world.installed {
            if app.name == "Safari" || app.name == "Arc" {
                app.hidden = true;
            }
        }
        let map = world.build().map;
        assert!(names(&map).contains(&"Arc"), "a rule names it");
        assert!(!names(&map).contains(&"Safari"));
        assert_eq!(map.in_use, "3 in use \u{b7} 1 more installed");
    }

    #[test]
    fn a_routed_card_shows_every_routed_profile_and_folds_the_idle_ones() {
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
        let labels: Vec<&str> = chrome.nodes.iter().map(Node::label).collect();
        // The routed ones by their first row, Everything else after every
        // rule, then the scan's others.
        assert_eq!(
            labels,
            [
                "A",
                "B",
                "C",
                "D",
                "E",
                "Personal",
                "Work",
                "Design",
                "Incognito"
            ]
        );
        // The private window does not count toward the folding.
        assert_eq!((chrome.shown, chrome.folded()), (6, 2));
        assert_eq!(chrome.visible(false), [0, 1, 2, 3, 4, 5, 8]);
        assert!(chrome.nodes[5].everything_else);
        assert!(
            map.lit(Some(Focus::EverythingElse)).node(0, 5),
            "the ∞ node is never folded"
        );
        // Never "+1 more": that node just shows.
        assert_eq!(folded_to(5, ROUTED_NODES), 5);
        assert_eq!(folded_to(6, ROUTED_NODES), ROUTED_NODES);
        assert_eq!(folded_to(3, IDLE_NODES), 3);
        assert_eq!(folded_to(4, IDLE_NODES), IDLE_NODES);
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
        assert_eq!(map.rows[0].node, None);
        assert!(
            map.lit(Some(Focus::Row(0)))
                .routes
                .iter()
                .all(|r| r.from != End::Row(0))
        );
        assert_eq!(map.cards[0].rules, 0);
    }

    #[test]
    fn handler_matchers_read_by_their_first_entry() {
        assert_eq!(
            handler_match(&[MatcherSpec::Function(function(""))]),
            RowMatch::Function {
                name: None,
                excerpt: None,
                source: None,
                more: 0
            },
            "a function with no source and no name: \"unnamed function\""
        );
        // An inline arrow's name is its property's: it says nothing, so
        // the row shows what it tests.
        let arrow = FunctionSpec {
            name: "match".into(),
            source: Some("({ url }) => url.host === \"tickets.example.com\"".into()),
            hostnames: None,
        };
        assert_eq!(
            handler_match(&[
                MatcherSpec::Function(arrow.clone()),
                MatcherSpec::Wildcard("x".into())
            ]),
            RowMatch::Function {
                name: None,
                excerpt: Some("url.host === \"tickets.example.com\"".into()),
                source: arrow.source.clone(),
                more: 1
            }
        );
        assert_eq!(
            handler_match(&[
                MatcherSpec::Regex("/^https:\\/\\/a\\.example\\.com/".into()),
                MatcherSpec::Wildcard("b.example.com/*".into()),
                MatcherSpec::Function(function("")),
            ]),
            RowMatch::Patterns {
                patterns: vec![
                    Pattern {
                        text: "/^https:\\/\\/a\\.example\\.com/".into(),
                        regex: true,
                    },
                    Pattern {
                        text: "b.example.com/*".into(),
                        regex: false,
                    },
                ],
                more: 1,
                opener: None,
            }
        );
        assert_eq!(handler_match(&[]), RowMatch::Any);
    }

    #[test]
    fn a_function_name_tells_only_when_it_is_not_generic() {
        for generic in [
            "",
            "  ",
            "match",
            "URL",
            "handler",
            "Matcher",
            "test",
            "fn",
            "anonymous",
            "default",
        ] {
            assert_eq!(telling_name(generic), None, "{generic:?}");
        }
        assert_eq!(telling_name("isWorkplace").as_deref(), Some("isWorkplace"));
    }

    #[test]
    fn excerpts_read_what_a_function_tests() {
        let excerpt = |source: &str| function_excerpt(source);
        // An arrow's expression.
        assert_eq!(
            excerpt("({ url }) => url.host === \"tickets.example.com\"").as_deref(),
            Some("url.host === \"tickets.example.com\"")
        );
        assert_eq!(excerpt("() => true").as_deref(), Some("true"));
        // Wrapped in parentheses.
        assert_eq!(
            excerpt("(url) => (url.pathname.startsWith(\"/j/\") ? \"Firefox\" : \"Safari\")")
                .as_deref(),
            Some("url.pathname.startsWith(\"/j/\") ? \"Firefox\" : \"Safari\"")
        );
        // A named function's first return.
        assert_eq!(
            excerpt(
                "function isWorkplace(url) {\n  return url.host.endsWith(\"workplace.example.com\");\n}"
            )
            .as_deref(),
            Some("url.host.endsWith(\"workplace.example.com\")")
        );
        // A multi-line async body: comments go, whitespace collapses, the
        // return's expression across lines.
        assert_eq!(
            excerpt(
                "async (url) => {\n  // the team's hosts\n  const host = url.host;\n  return host === \"a.example.com\" ||\n    /* or */ host === \"b.example.com\";\n}"
            )
            .as_deref(),
            Some("host === \"a.example.com\" || host === \"b.example.com\"")
        );
        // No return: the first statement, and more to it.
        assert_eq!(
            excerpt("(url) => { console.log(url.host); }").as_deref(),
            Some("console.log(url.host) \u{2026}")
        );
        // A `=>` inside a string or a nested call is not the arrow.
        assert_eq!(
            excerpt("function (url) { return [\"=>\"].includes(url.host); }").as_deref(),
            Some("[\"=>\"].includes(url.host)")
        );
        // "//" in a string is not a comment.
        assert_eq!(
            excerpt("(url) => url.href.startsWith(\"https://a.example.com\")").as_deref(),
            Some("url.href.startsWith(\"https://a.example.com\")")
        );
        // Long ones are cut before layout.
        let long = format!(
            "(url) => {}",
            "url.host === \"a.example.com\" || ".repeat(20)
        );
        let cut = excerpt(&long).expect("an excerpt");
        assert!(cut.chars().count() <= EXCERPT_MAX);
        assert!(cut.ends_with('\u{2026}'));
        assert_eq!(excerpt(""), None);
        assert_eq!(excerpt("() => {}"), None);
    }

    #[test]
    fn guards_read_by_their_condition_and_regex_literals_stay_whole() {
        let excerpt = |source: &str| function_excerpt(source);
        // A guard that returns true, then false: what the guard tests.
        assert_eq!(
            excerpt(
                "(url) => { if (url.host === \"a.example.com\") { return true; } return false; }"
            )
            .as_deref(),
            Some("url.host === \"a.example.com\"")
        );
        assert_eq!(
            excerpt(
                "(url) => {\n  if (url.host === \"a.example.com\") return true\n  return false\n}"
            )
            .as_deref(),
            Some("url.host === \"a.example.com\"")
        );
        // A guard that returns false tests the opposite.
        assert_eq!(
            excerpt("function (url) { if (url.port) return false; return true; }").as_deref(),
            Some("!(url.port)")
        );
        // A guard before a return that says something: the return.
        assert_eq!(
            excerpt(
                "(url) => { if (!url.host) return false; return url.host === \"b.example.com\"; }"
            )
            .as_deref(),
            Some("url.host === \"b.example.com\"")
        );
        // A method named return is not the keyword.
        assert_eq!(
            excerpt("() => { x.return(1) }").as_deref(),
            Some("x.return(1) \u{2026}")
        );
        // A regex literal holding "/*" or "//" is not a comment.
        assert_eq!(
            excerpt("u => /[/*]/.test(u.href) && u.host === \"c.example.com\"").as_deref(),
            Some("/[/*]/.test(u.href) && u.host === \"c.example.com\"")
        );
        assert_eq!(
            excerpt("(u) => /^https:\\/\\/d\\.example\\.com\\//.test(u.href) // its docs")
                .as_deref(),
            Some("/^https:\\/\\/d\\.example\\.com\\//.test(u.href)")
        );
        // A regex with a bracket in its class leaves the depth alone.
        assert_eq!(
            excerpt("function (u) { return /[(]/.test(u.pathname); }").as_deref(),
            Some("/[(]/.test(u.pathname)")
        );
        // Division is not a regex.
        assert_eq!(
            excerpt("(u) => u.port / 2 > 4000 // half").as_deref(),
            Some("u.port / 2 > 4000")
        );
    }

    /// The user's kind of config: inline arrows Finicky names "match", and
    /// a `finicky.matchHostnames` list, all into one profile.
    #[test]
    fn inline_handlers_read_their_source_and_hostname_lists_their_hosts() {
        let description = Engine::load(
            &fixtures::overview_handlers_config(),
            Rc::new(DefaultHost),
            Limits::default(),
        )
        .expect("the handlers config loads")
        .describe()
        .expect("it describes")
        .expect("it has a description");
        let map = World {
            config: Some(Some(description)),
            ..World::new(RulesFile::default(), Vec::new())
        }
        .build()
        .map;
        assert_eq!(map.rows.len(), 8);
        let read = |row: &Row| match &row.matcher {
            RowMatch::Function { name, excerpt, .. } => {
                assert_eq!(*name, None, "a generic name says nothing");
                excerpt.clone().expect("an excerpt")
            }
            other => panic!("a function row: {other:?}"),
        };
        let excerpts: Vec<String> = map.rows[..7].iter().map(read).collect();
        assert_eq!(
            excerpts,
            [
                "url.host === \"tickets.example.com\"",
                "url.host.endsWith(\".build.example.net\") && url.pathname.startsWith(\"/runs/\")",
                "url.host === \"wiki.example.org\"",
                "/^(docs|api)\\.example\\.com$/.test(url.host)",
                "url.searchParams.get(\"team\") === \"research\"",
                "url.host === \"review.example.com\" && url.pathname.includes(\"/pull/\")",
                "[\"chat.example.com\", \"meet.example.com\"].includes(url.host)",
            ]
        );
        let RowMatch::Patterns { patterns, more, .. } = &map.rows[7].matcher else {
            panic!("hostname tags: {:?}", map.rows[7].matcher);
        };
        assert_eq!(
            patterns
                .iter()
                .map(|p| (p.text.as_str(), p.regex))
                .collect::<Vec<_>>(),
            [
                ("status.example.com", false),
                ("metrics.example.net", false),
                ("/^dash\\d+\\.example\\.org$/", true),
            ]
        );
        assert_eq!(*more, 0);
        // All eight into one node, every route lit when it is traced.
        let node = map.rows[0].node.expect("a node");
        assert!(map.rows.iter().all(|row| row.node == Some(node)));
        let card = &map.cards[node.0];
        assert_eq!(
            (card.name.as_str(), card.nodes[node.1].label()),
            ("Microsoft Edge", "Research and Development Team")
        );
        let lit = map.lit(Some(Focus::Node(node.0, node.1)));
        assert_eq!(
            lit.routes.iter().filter(|r| r.trace == Trace::Lit).count(),
            8
        );
        assert_eq!(lit.fan(node.0, node.1), 8);
    }

    /// A pin is kept by what it is: when the cards re-sort, it finds its
    /// profile again.
    #[test]
    fn a_focus_key_finds_its_profile_after_the_cards_re_sort() {
        let before = World::rules().build().map;
        let after = World::new(fixtures::overview_rules(), Vec::new())
            .build()
            .map;
        let mut moved = false;
        for (c, card) in before.cards.iter().enumerate() {
            for (n, node) in card.nodes.iter().enumerate() {
                let key = before.key_of(Focus::Node(c, n)).expect("a key");
                let Some(Focus::Node(c2, n2)) = after.resolve(&key) else {
                    panic!("{key:?} is still on the map");
                };
                assert_eq!(after.cards[c2].name, card.name);
                assert_eq!(after.cards[c2].nodes[n2].profile, node.profile);
                moved |= (c2, n2) != (c, n);
            }
            let key = before.key_of(Focus::Card(c)).expect("a key");
            let Some(Focus::Card(c2)) = after.resolve(&key) else {
                panic!("{key:?} is still on the map");
            };
            assert_eq!(after.cards[c2].name, card.name);
        }
        assert!(moved, "the cards re-sort without today's links");
        let row = before.key_of(Focus::Row(2)).expect("a key");
        assert_eq!(row, FocusKey::Row(RowSource::Rule(2)));
        assert_eq!(after.resolve(&row), Some(Focus::Row(2)));
        assert_eq!(after.resolve(&FocusKey::Card("not a browser".into())), None);
    }

    /// A config names a Chromium profile by its directory ("Profile 1"):
    /// it is the profile the scan calls "Work", one node.
    #[test]
    fn a_profile_named_by_its_directory_is_the_scans_profile() {
        let rules = RulesFile {
            rules: vec![
                rule("a.example.com/*", "Google Chrome", "Profile 1"),
                rule("b.example.com/*", "Google Chrome", "Work"),
            ],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        let chrome = map
            .cards
            .iter()
            .find(|card| card.name == "Google Chrome")
            .expect("Chrome's card");
        let labels: Vec<&str> = chrome.nodes.iter().map(Node::label).collect();
        assert_eq!(labels, ["Work", "Personal", "Design", "Incognito"]);
        assert_eq!(chrome.nodes[0].rules, 2);
        assert_eq!(map.rows[0].node, map.rows[1].node);
        assert!(matches!(
            &map.rows[0].target,
            Target::Browser { label, .. } if label == "Work"
        ));
    }

    /// A browser only rules that are off send links to has no rules yet:
    /// not in use, with the idle browsers, its node idle but still the end
    /// of the off rule's would-be route.
    #[test]
    fn a_browser_only_rules_that_are_off_name_has_no_rules_yet() {
        let rules = RulesFile {
            default_browser: "Google Chrome".into(),
            rules: vec![
                rule("a.example.com/*", "Google Chrome", "Work"),
                Rule {
                    enabled: false,
                    ..rule("b.example.com/*", "Firefox", "")
                },
            ],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        assert_eq!(map.in_use, "1 in use \u{b7} 4 more installed");
        let firefox = map
            .cards
            .iter()
            .position(|card| card.name == "Firefox")
            .expect("Firefox's card");
        let card = &map.cards[firefox];
        assert!(card.unused);
        assert_eq!(card.meta, "default profile \u{b7} no rules yet");
        assert!(card.nodes[0].unused);
        assert_eq!(card.share, None);
        // With the browsers that have no rules yet, A-Z.
        let idle: Vec<&str> = map
            .cards
            .iter()
            .filter(|card| card.unused)
            .map(|card| card.name.as_str())
            .collect();
        assert_eq!(idle, ["Arc", "Firefox", "Microsoft Edge", "Safari"]);
        // Traced, the off rule draws its route there, not running.
        assert_eq!(map.rows[1].node, Some((firefox, 0)));
        let lit = map.lit(Some(Focus::Row(1)));
        let route = lit.route(End::Row(1)).expect("its would-be route");
        assert!(!route.running && !route.at_rest);
        // A node with no rules yet traces nothing.
        assert!(!map.lit(Some(Focus::Node(firefox, 0))).traced);
    }

    /// Tracing a rule that is off moves no other route at its node, and
    /// the legend's "not running" swatch is for what is drawn at rest.
    #[test]
    fn tracing_a_rule_that_is_off_moves_nothing_in_the_fan() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![
                rule("a.example.com/*", "Google Chrome", "Work"),
                Rule {
                    enabled: false,
                    ..rule("b.example.com/*", "Google Chrome", "Work")
                },
                rule("c.example.com/*", "Google Chrome", "Work"),
            ],
            ..RulesFile::default()
        };
        let map = World::new(rules, Vec::new()).build().map;
        let node = map.rows[0].node.expect("Chrome · Work");
        let rest = map.lit(None);
        let traced = map.lit(Some(Focus::Row(1)));
        assert_eq!(rest.fan(node.0, node.1), 2);
        assert_eq!(traced.fan(node.0, node.1), 2);
        let slots = |lit: &Lit| {
            [End::Row(0), End::Row(2)].map(|end| {
                let route = lit.route(end).expect("a route");
                (route.slot, route.fan)
            })
        };
        assert_eq!(slots(&rest), [(0, 2), (1, 2)]);
        assert_eq!(slots(&traced), slots(&rest));
        let off = traced.route(End::Row(1)).expect("the would-be route");
        assert_eq!((off.slot, off.fan), (1, 2));
        assert!(!rest.any_not_running());
        assert!(!traced.any_not_running());
    }
}
