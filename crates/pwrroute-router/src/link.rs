//! A link as the OS hands it over, and the checks it passes before routing.

use pwrroute_engine::Opener;
use serde::{Deserialize, Serialize};

/// The longest URL the router accepts, as the Go host did.
pub const MAX_URL_LEN: usize = 64 * 1024;

/// One link to route: the URL and, when the OS says, the app it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<OpenerInfo>,
    #[serde(default)]
    pub source: Source,
}

impl Link {
    pub fn new(url: impl Into<String>, source: Source) -> Self {
        Self {
            url: url.into(),
            opener: None,
            source,
        }
    }

    #[must_use]
    pub fn with_opener(mut self, opener: Option<OpenerInfo>) -> Self {
        self.opener = opener;
        self
    }
}

/// How a link reached PwrRoute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// A macOS `kAEGetURL` Apple Event.
    AppleEvent,
    /// The command line: a Windows ProgID or Linux `.desktop` launch, or a
    /// user running `pwrroute URL`.
    #[default]
    CommandLine,
    /// The app itself or an agent, through the router's API.
    Api,
}

impl Source {
    /// A fixed label for logs. Never carries link data.
    pub fn label(self) -> &'static str {
        match self {
            Self::AppleEvent => "apple_event",
            Self::CommandLine => "command_line",
            Self::Api => "api",
        }
    }
}

/// The sending app, as the OS reports it. The engine's [`Opener`] in a form
/// that also crosses the IPC socket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenerInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub bundle_id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
}

impl From<&OpenerInfo> for Opener {
    fn from(o: &OpenerInfo) -> Self {
        Self {
            name: o.name.clone(),
            bundle_id: o.bundle_id.clone(),
            path: o.path.clone(),
            window_title: o.window_title.clone(),
        }
    }
}

/// Why a link was not routed. The messages never quote the URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    Empty,
    TooLong,
    NotAbsolute,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "the link is empty",
            Self::TooLong => "the link is longer than 64 KiB",
            Self::NotAbsolute => "the link is not an absolute URL with a scheme",
        })
    }
}

impl std::error::Error for LinkError {}

/// Checks a URL before it reaches the engine and returns it trimmed.
///
/// The URL itself is passed on as received: Finicky's matchers see the
/// string the OS sent, not a re-serialized one.
pub fn normalize_url(raw: &str) -> Result<String, LinkError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(LinkError::Empty);
    }
    if trimmed.len() > MAX_URL_LEN {
        return Err(LinkError::TooLong);
    }
    // A web link without a host fails to parse, so no separate check.
    url::Url::parse(trimmed).map_err(|_| LinkError::NotAbsolute)?;
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_links_pass_through_as_received() {
        assert_eq!(
            normalize_url("  https://example.com/a?b=c#d \n").unwrap(),
            "https://example.com/a?b=c#d"
        );
        // Not re-serialized: url would add the trailing slash.
        assert_eq!(
            normalize_url("https://example.com").unwrap(),
            "https://example.com"
        );
        assert!(normalize_url("mailto:someone@example.com").is_ok());
    }

    #[test]
    fn bad_links_are_refused() {
        assert_eq!(normalize_url(""), Err(LinkError::Empty));
        assert_eq!(
            normalize_url("example.com/path"),
            Err(LinkError::NotAbsolute)
        );
        assert_eq!(normalize_url("https://"), Err(LinkError::NotAbsolute));
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert_eq!(normalize_url(&long), Err(LinkError::TooLong));
    }

    #[test]
    fn errors_never_quote_the_url() {
        let secret = "https://example.com/?token=hunter2";
        for err in [LinkError::Empty, LinkError::TooLong, LinkError::NotAbsolute] {
            assert!(!err.to_string().contains(secret));
        }
    }
}
