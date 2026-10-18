//! Short-link expansion before resolving, as the Go host's `shorturl`
//! package did it.
//!
//! A link on a shortener's domain is followed (HEAD first, GET if that does
//! not end in a 200) through a bounded number of redirects under a deadline,
//! and the URL it lands on is what the rules see; `originalUrl` keeps the
//! short link. When expansion fails part way, the last redirect target seen
//! is used, and with none, the short link itself.
//!
//! The domains are the Go host's built-in list plus any a config adds in
//! `options.urlShorteners`. Finicky v4's schema accepts that option and the Go
//! host ignored it, so adding to the defaults (rather than replacing them)
//! keeps every v4 config, including the example's `urlShorteners: []`,
//! expanding what it expanded under Go.
//!
//! The one network request sits behind [`Network`], so tests use a fake and
//! the router can run expansion away from the engine's thread. Nothing here
//! logs or stores a URL.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use url::Url;

/// The Go host's built-in shortener domains (`shortener_domains.json`).
pub const DEFAULT_SHORTENERS: &[&str] = &[
    "aka.ms",
    "bit.ly",
    "buff.ly",
    "d.to",
    "dub.sh",
    "goo.gl",
    "is.gd",
    "msteams.link",
    "ow.ly",
    "safelinks.protection.outlook.com",
    "shorturl.at",
    "spoti.fi",
    "t.co",
    "tiny.cc",
    "tinyurl.com",
    "urlshortener.teams.microsoft.com",
    "wu8.in",
    "linkprotect.cudasvc.com",
];

/// The HTTP method of one expansion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Head,
    Get,
}

/// What one request answered. Only the status and `Location` matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
}

/// Makes one HTTP request without following redirects. An implementation
/// must not log or store the URL, and must give up by `timeout`.
pub trait Network {
    fn request(&self, method: Method, url: &str, timeout: Duration) -> Result<Reply>;
}

/// A link on its way to the rules: `url` is what they match, `original_url`
/// what the user clicked (`options.originalUrl` in a config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub original_url: String,
}

impl Link {
    /// A link that was not expanded.
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            original_url: url.clone(),
            url,
        }
    }

    /// Whether expansion changed the URL.
    pub fn was_expanded(&self) -> bool {
        self.url != self.original_url
    }
}

/// Which links to expand and how far to follow them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortLinks {
    /// A link's host must equal one of these or end in `.` plus one.
    pub domains: Vec<String>,
    /// Redirects seen per attempt. Reaching the limit stops the attempt,
    /// and the last redirect's target is used without being requested: the
    /// Go host's `CheckRedirect` refused the third redirect the same way.
    pub max_redirects: usize,
    /// The budget for each of the HEAD and GET attempts, all redirects
    /// included. The Go host's was 750 ms.
    pub timeout: Duration,
}

impl Default for ShortLinks {
    fn default() -> Self {
        Self::with_domains(std::iter::empty::<String>())
    }
}

impl ShortLinks {
    /// The built-in domains plus `extra`.
    pub fn with_domains<S: Into<String>>(extra: impl IntoIterator<Item = S>) -> Self {
        let mut domains: Vec<String> = DEFAULT_SHORTENERS.iter().map(|d| (*d).to_owned()).collect();
        for d in extra {
            let d = d.into().trim().trim_start_matches('.').to_ascii_lowercase();
            if !d.is_empty() && !domains.contains(&d) {
                domains.push(d);
            }
        }
        Self {
            domains,
            max_redirects: 3,
            timeout: Duration::from_millis(750),
        }
    }

    /// Whether `url` is on a shortener's domain.
    ///
    /// The Go host compared with a plain suffix test on the host and port,
    /// so `notbit.ly` counted as `bit.ly` and `bit.ly:443` did not. This
    /// compares whole labels of the host name.
    pub fn is_short(&self, url: &str) -> bool {
        let Ok(parsed) = Url::parse(url) else {
            return false;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        let Some(host) = parsed.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();
        self.domains.iter().any(|d| {
            host == *d
                || host
                    .strip_suffix(d.as_str())
                    .is_some_and(|rest| rest.ends_with('.'))
        })
    }

    /// Expands `url` if it is a short link. Never fails: a link that cannot
    /// be expanded goes to the rules as it is, as it did under Go.
    pub fn expand(&self, url: &str, network: &dyn Network) -> Link {
        if !self.is_short(url) {
            return Link::new(url);
        }
        let mut last_seen = None;
        for method in [Method::Head, Method::Get] {
            if let Ok(Some(landed)) = self.follow(method, url, network, &mut last_seen) {
                return Link {
                    url: landed,
                    original_url: url.to_owned(),
                };
            }
        }
        Link {
            url: last_seen.unwrap_or_else(|| url.to_owned()),
            original_url: url.to_owned(),
        }
    }

    /// One attempt: `Some` with the final URL when it ends in a 200.
    fn follow(
        &self,
        method: Method,
        start: &str,
        network: &dyn Network,
        last_seen: &mut Option<String>,
    ) -> Result<Option<String>> {
        let deadline = Instant::now() + self.timeout;
        let mut current = Url::parse(start)?;
        let mut redirects = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timed out");
            }
            let reply = network.request(method, current.as_str(), remaining)?;
            match (reply.status, reply.location) {
                (200, _) => return Ok(Some(current.into())),
                (300..=399, Some(location)) => {
                    let next = current.join(&location)?;
                    if !matches!(next.scheme(), "http" | "https") {
                        bail!("redirect leaves the web");
                    }
                    *last_seen = Some(next.to_string());
                    redirects += 1;
                    if redirects >= self.max_redirects {
                        bail!("too many redirects");
                    }
                    current = next;
                }
                _ => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    /// Answers from a table: `(method, url) -> reply`; anything else fails.
    #[derive(Default)]
    struct Table {
        replies: HashMap<(Method, String), Reply>,
        asked: RefCell<Vec<(Method, String)>>,
    }

    impl Table {
        fn redirect(mut self, method: Method, from: &str, to: &str) -> Self {
            self.replies.insert(
                (method, from.to_owned()),
                Reply {
                    status: 301,
                    location: Some(to.to_owned()),
                },
            );
            self
        }

        fn status(mut self, method: Method, url: &str, status: u16) -> Self {
            self.replies.insert(
                (method, url.to_owned()),
                Reply {
                    status,
                    location: None,
                },
            );
            self
        }
    }

    impl Network for Table {
        fn request(&self, method: Method, url: &str, _timeout: Duration) -> Result<Reply> {
            self.asked.borrow_mut().push((method, url.to_owned()));
            match self.replies.get(&(method, url.to_owned())) {
                Some(reply) => Ok(reply.clone()),
                None => bail!("offline"),
            }
        }
    }

    #[test]
    fn matches_whole_host_labels() {
        let links = ShortLinks::default();
        assert!(links.is_short("https://bit.ly/x"));
        assert!(links.is_short("https://eur01.safelinks.protection.outlook.com/?url=x"));
        assert!(links.is_short("http://BIT.LY:8080/x"));
        assert!(!links.is_short("https://notbit.ly/x"));
        assert!(!links.is_short("https://example.com/bit.ly"));
        assert!(!links.is_short("mailto:someone@bit.ly"));
    }

    #[test]
    fn a_config_adds_domains() {
        let links = ShortLinks::with_domains(["go.corp.example", ".bit.ly"]);
        assert!(links.is_short("https://go.corp.example/x"));
        assert_eq!(
            links.domains.iter().filter(|d| *d == "bit.ly").count(),
            1,
            "duplicates are dropped"
        );
    }

    #[test]
    fn other_links_are_not_requested() {
        let net = Table::default();
        let link = ShortLinks::default().expand("https://example.com/", &net);
        assert!(!link.was_expanded());
        assert!(net.asked.borrow().is_empty());
    }

    #[test]
    fn follows_redirects_to_a_200() {
        let net = Table::default()
            .redirect(Method::Head, "https://bit.ly/x", "https://example.com/a")
            .redirect(Method::Head, "https://example.com/a", "/b?c=1")
            .status(Method::Head, "https://example.com/b?c=1", 200);
        let link = ShortLinks::default().expand("https://bit.ly/x", &net);
        assert_eq!(link.url, "https://example.com/b?c=1");
        assert_eq!(link.original_url, "https://bit.ly/x");
    }

    #[test]
    fn falls_back_to_get_when_head_is_refused() {
        let net = Table::default()
            .status(Method::Head, "https://t.co/x", 405)
            .redirect(Method::Get, "https://t.co/x", "https://example.com/")
            .status(Method::Get, "https://example.com/", 200);
        let link = ShortLinks::default().expand("https://t.co/x", &net);
        assert_eq!(link.url, "https://example.com/");
    }

    #[test]
    fn stops_after_the_redirect_limit_and_uses_the_last_target() {
        // Go's client made three requests and refused the third redirect;
        // its target is the URL used, unrequested. GET then retries the same.
        let net = Table::default()
            .redirect(Method::Head, "https://bit.ly/1", "https://bit.ly/2")
            .redirect(Method::Head, "https://bit.ly/2", "https://bit.ly/3")
            .redirect(Method::Head, "https://bit.ly/3", "https://bit.ly/4")
            .status(Method::Head, "https://bit.ly/4", 200);
        let link = ShortLinks::default().expand("https://bit.ly/1", &net);
        assert_eq!(link.url, "https://bit.ly/4");
        let asked = net.asked.borrow();
        assert_eq!(asked.iter().filter(|(m, _)| *m == Method::Head).count(), 3);
        assert!(!asked.iter().any(|(_, u)| u == "https://bit.ly/4"));
    }

    #[test]
    fn an_unreachable_shortener_leaves_the_link_alone() {
        let link = ShortLinks::default().expand("https://bit.ly/x", &Table::default());
        assert_eq!(link, Link::new("https://bit.ly/x"));
    }

    #[test]
    fn a_failure_after_a_redirect_uses_the_redirect() {
        let net =
            Table::default().redirect(Method::Head, "https://bit.ly/x", "https://example.com/");
        let link = ShortLinks::default().expand("https://bit.ly/x", &net);
        assert_eq!(link.url, "https://example.com/");
    }

    #[test]
    fn a_redirect_off_the_web_is_not_followed() {
        let net =
            Table::default().redirect(Method::Head, "https://bit.ly/x", "javascript:alert(1)");
        let link = ShortLinks::default().expand("https://bit.ly/x", &net);
        assert_eq!(link.url, "https://bit.ly/x");
    }

    struct Slow;

    impl Network for Slow {
        fn request(&self, _method: Method, url: &str, timeout: Duration) -> Result<Reply> {
            std::thread::sleep(timeout);
            Ok(Reply {
                status: 302,
                location: Some(format!("{url}x")),
            })
        }
    }

    #[test]
    fn the_deadline_bounds_each_attempt() {
        let links = ShortLinks {
            timeout: Duration::from_millis(30),
            ..ShortLinks::default()
        };
        let started = Instant::now();
        let link = links.expand("https://bit.ly/x", &Slow);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(link.url.starts_with("https://bit.ly/x"));
    }
}
