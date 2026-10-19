//! What a loaded JS or TS config says, without running it on a URL: its
//! rewrites and handlers in the order Finicky's `openUrl` runs them, and its
//! default browser. The Rules screen shows this read-only while a config is
//! in charge.
//!
//! The description comes from the validated `finalConfig` (see
//! `js/host.js`). A fixed browser is spelled out by the API's own
//! `createBrowserConfig`, so `"Firefox:work"` reads as Firefox with the work
//! profile, as it would open. A function is described by its name and source
//! text; what it returns is known only when it runs.

use serde::{Deserialize, Serialize};

use crate::AppType;

/// A config's routing, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Description {
    pub rewrites: Vec<RewriteRule>,
    pub handlers: Vec<HandlerRule>,
    pub default_browser: BrowserSpec,
}

/// One entry of the config's `rewrite` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriteRule {
    /// The `match`, one entry per pattern when it is a list.
    pub matchers: Vec<MatcherSpec>,
    pub url: UrlSpec,
}

/// One entry of the config's `handlers` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlerRule {
    /// The `match`, one entry per pattern when it is a list.
    pub matchers: Vec<MatcherSpec>,
    pub browser: BrowserSpec,
}

/// A `match` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MatcherSpec {
    /// A Finicky wildcard pattern.
    Wildcard(String),
    /// A regular expression, as `/source/flags`.
    Regex(String),
    Function(FunctionSpec),
    /// Something Finicky would not match with.
    Other(String),
}

/// Where a rewrite sends the URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UrlSpec {
    /// A fixed URL.
    Url(String),
    Function(FunctionSpec),
}

/// A handler's or the default's browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserSpec {
    /// A fixed browser. `browser: null` is one whose `app_type` is
    /// [`AppType::None`]: the link is not opened.
    Browser(FixedBrowser),
    /// A function that picks the browser per URL.
    Function(FunctionSpec),
}

/// A browser named in the config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixedBrowser {
    /// An app name, bundle id or path, as `app_type` says.
    pub name: String,
    pub app_type: AppType,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub open_in_background: Option<bool>,
}

/// A JS function in the config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionSpec {
    /// Its name, or empty for an anonymous function.
    pub name: String,
    /// Its source text as QuickJS kept it: the config after the TypeScript
    /// and syntax transform, so types are gone and spacing may differ from
    /// the file. `None` for a native or bound function.
    #[serde(default)]
    pub source: Option<String>,
}
