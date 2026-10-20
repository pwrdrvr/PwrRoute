//! Help for writing a rule for a link the rules did not route: wildcard
//! patterns that would catch it, narrowest first, and which other links a
//! pattern would also catch.
//!
//! The candidates are built from the URL's parts in Rust, but whether a
//! pattern matches is only ever Finicky's `isMatch`, run through the engine
//! ([`Engine::collisions`]), so the answer is exactly what routing would do.

use anyhow::Result;
use rquickjs::CatchResultExt;

use crate::{Engine, js_error};

/// Patterns that would route one link.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suggestion {
    /// Wildcard patterns that match the link, narrowest first: the host with
    /// the first path segment (`github.com/pwrdrvr/*`), then the host alone
    /// (`github.com/*`). A link whose path stops at its first segment gets
    /// itself, exactly, in place of the first. A scheme outside the ones a
    /// pattern matches implicitly (`http`, `https`, `ftp`, `mailto`,
    /// `file`, `tel`, `sms`, `data`) is written out, and so is the scheme
    /// of a link with a port, since `host:port` would read as a scheme.
    /// Empty for text that is not a URL.
    pub candidates: Vec<String>,
    /// A wildcard cannot say what distinguishes this link, so a precise rule
    /// needs a JS config: a regex matcher, or a `url` function with a
    /// capture group for a rewrite. `why` says which.
    pub needs_js: bool,
    /// One line per candidate, in order, then one per reason `needs_js` is
    /// set.
    pub why: Vec<&'static str>,
}

/// Schemes a pattern without one matches anyway, from Finicky's
/// `matchWildcard`.
const IMPLICIT_SCHEMES: [&str; 8] = [
    "http", "https", "ftp", "mailto", "file", "tel", "sms", "data",
];

const WHY_PAGE: &str = "Matches this page only.";
const WHY_SECTION: &str = "Matches everything under the first part of the path.";
const WHY_HOST: &str = "Matches everything on this host.";
const WHY_SCHEME: &str = "Matches every link with this scheme.";
const WHY_NOT_A_URL: &str = "This is not a URL PwrRoute can match.";
const WHY_ID: &str = "The first part of the path looks like an id. A wildcard cannot limit a part to ids; a regex matcher in a JS config can.";
const WHY_INNER_URL: &str = "A query parameter carries another URL. Unwrapping it is a rewrite with a capture group, which needs a url function in a JS config.";
const WHY_USERINFO: &str = "The link carries a user name before the host, which a pattern that starts with the host cannot see; a JS matcher can read it.";

/// Wildcard patterns for a rule that would route `url`, and whether a rule
/// that fits it precisely needs JS instead.
pub fn suggest_rule(url: &str) -> Suggestion {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return Suggestion {
            candidates: Vec::new(),
            needs_js: false,
            why: vec![WHY_NOT_A_URL],
        };
    };
    let mut suggestion = Suggestion::default();
    let scheme = parsed.scheme();
    // A pattern that starts with `host:port` reads as one with a scheme, so
    // a port needs the scheme written out too.
    let prefix = if IMPLICIT_SCHEMES.contains(&scheme) && parsed.port().is_none() {
        String::new()
    } else if parsed.host_str().is_some() {
        format!("{scheme}://")
    } else {
        format!("{scheme}:")
    };

    let Some(host) = parsed.host_str() else {
        // mailto:, tel:, data: and the like: the whole link, or every link
        // of its kind.
        suggestion.candidates.push(escape(parsed.as_str()));
        suggestion.why.push(WHY_PAGE);
        suggestion.candidates.push(format!("{scheme}:*"));
        suggestion.why.push(WHY_SCHEME);
        return suggestion;
    };
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let path = parsed.path();
    if path.is_empty() {
        // A scheme of its own (`slack://channel?id=1`) may have no path at
        // all, so `host/*` would not match it.
        suggestion.candidates.push(escape(parsed.as_str()));
        suggestion.why.push(WHY_PAGE);
        suggestion
            .candidates
            .push(format!("{prefix}{}*", escape(&authority)));
        suggestion.why.push(WHY_HOST);
        return suggestion;
    }
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    if !first.is_empty() {
        if path.starts_with(&format!("/{first}/")) {
            suggestion
                .candidates
                .push(format!("{prefix}{authority}/{}/*", escape(first)));
            suggestion.why.push(WHY_SECTION);
        } else {
            // `host/first/*` needs a slash after the segment, which this
            // link does not have, so the narrowest pattern is the link.
            suggestion.candidates.push(escape(parsed.as_str()));
            suggestion.why.push(WHY_PAGE);
        }
    }
    suggestion.candidates.push(format!("{prefix}{authority}/*"));
    suggestion.why.push(WHY_HOST);

    if looks_like_id(first) {
        suggestion.needs_js = true;
        suggestion.why.push(WHY_ID);
    }
    if parsed
        .query_pairs()
        .any(|(_, value)| value.starts_with("http://") || value.starts_with("https://"))
    {
        suggestion.needs_js = true;
        suggestion.why.push(WHY_INNER_URL);
    }
    if !parsed.username().is_empty() {
        suggestion.needs_js = true;
        suggestion.why.push(WHY_USERINFO);
    }
    suggestion
}

/// A path segment that is an identifier rather than a section name: all
/// digits, a run of hex, or a UUID.
fn looks_like_id(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    let digits = segment.chars().all(|c| c.is_ascii_digit());
    let hex = segment.len() >= 8 && segment.chars().all(|c| c.is_ascii_hexdigit());
    let uuid = segment.len() == 36
        && segment.split('-').map(str::len).eq([8, 4, 4, 4, 12])
        && segment.chars().all(|c| c == '-' || c.is_ascii_hexdigit());
    digits || hex || uuid
}

/// A literal `*` in a URL must be `\*` in a pattern, or it is a wildcard.
fn escape(text: &str) -> String {
    text.replace('*', "\\*")
}

impl Engine {
    /// Which of `urls` `pattern` matches, using Finicky's own `isMatch` as
    /// routing would. Lets a user see what else a candidate rule would catch
    /// before saving it (the router's recent links are the usual input). A
    /// URL that does not parse is never matched. Nothing is written.
    pub fn collisions(&self, pattern: &str, urls: &[&str]) -> Result<Vec<String>> {
        if urls.is_empty() {
            return Ok(Vec::new());
        }
        let resolve = self.limits.resolve;
        self.with_deadline(resolve, |ctx| {
            let matches: rquickjs::Function<'_> = ctx.eval("__pwrroute.matches")?;
            let list = rquickjs::Array::new(ctx.clone())?;
            for (i, url) in urls.iter().enumerate() {
                list.set(i, *url)?;
            }
            let hits: Vec<bool> = matches
                .call((pattern, list))
                .catch(&ctx)
                .map_err(|e| js_error("matching the pattern", &e))?;
            Ok(urls
                .iter()
                .zip(hits)
                .filter(|(_, hit)| *hit)
                .map(|(url, _)| (*url).to_owned())
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates(url: &str) -> Vec<String> {
        suggest_rule(url).candidates
    }

    #[test]
    fn candidates_go_from_the_section_to_the_host() {
        assert_eq!(
            candidates("https://github.com/pwrdrvr/PwrRoute/pull/1"),
            ["github.com/pwrdrvr/*", "github.com/*"]
        );
        assert_eq!(candidates("https://example.com/"), ["example.com/*"]);
        assert_eq!(candidates("https://example.com"), ["example.com/*"]);
        assert_eq!(
            candidates("http://localhost:3000/app/x"),
            ["http://localhost:3000/app/*", "http://localhost:3000/*"]
        );
    }

    #[test]
    fn a_path_that_stops_at_its_first_part_is_matched_exactly() {
        let s = suggest_rule("https://github.com/pwrdrvr?tab=repositories");
        assert_eq!(
            s.candidates,
            [
                "https://github.com/pwrdrvr?tab=repositories",
                "github.com/*"
            ]
        );
        assert_eq!(s.why, [WHY_PAGE, WHY_HOST]);
        assert!(!s.needs_js);
    }

    #[test]
    fn an_unusual_scheme_is_written_out() {
        assert_eq!(
            candidates("vscode://file/Users/x/y.rs"),
            ["vscode://file/Users/*", "vscode://file/*"]
        );
        assert_eq!(
            candidates("mailto:alice@example.com"),
            ["mailto:alice@example.com", "mailto:*"]
        );
        assert_eq!(
            candidates("slack://channel?id=1"),
            ["slack://channel?id=1", "slack://channel*"]
        );
    }

    #[test]
    fn ids_and_wrapped_urls_need_js() {
        let s = suggest_rule("https://issues.example/12345/comments");
        assert!(s.needs_js);
        assert_eq!(s.candidates, ["issues.example/12345/*", "issues.example/*"]);
        assert_eq!(s.why.last(), Some(&WHY_ID));
        let s = suggest_rule("https://www.google.com/url?q=https://example.com/x");
        assert!(s.needs_js);
        assert!(s.why.contains(&WHY_INNER_URL));
        let s = suggest_rule("https://user@host.example/a/b");
        assert!(s.needs_js);
        assert!(s.why.contains(&WHY_USERINFO));
        assert!(!suggest_rule("https://docs.example/guide/intro").needs_js);
    }

    #[test]
    fn text_that_is_not_a_url_gets_no_candidates() {
        let s = suggest_rule("github.com/x");
        assert!(s.candidates.is_empty());
        assert_eq!(s.why, [WHY_NOT_A_URL]);
    }

    #[test]
    fn a_literal_asterisk_is_escaped() {
        assert_eq!(
            candidates("https://a.example/q*"),
            ["https://a.example/q\\*", "a.example/*"]
        );
    }

    #[test]
    fn ids_are_digits_hex_or_uuids() {
        assert!(looks_like_id("12345"));
        assert!(looks_like_id("deadbeef01"));
        assert!(looks_like_id("123e4567-e89b-12d3-a456-426614174000"));
        assert!(!looks_like_id("pwrdrvr"));
        assert!(!looks_like_id("cafe"));
        assert!(!looks_like_id(""));
    }
}
