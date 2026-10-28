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
    /// itself, exactly, in place of the first; with a query or fragment it
    /// gets its page and a `*` instead (`auth.example.com/authorize*`), since
    /// the query and fragment never enter a pattern and a pattern without a
    /// `*` must equal the whole link. A scheme outside the ones a
    /// pattern matches implicitly (`http`, `https`, `ftp`, `mailto`,
    /// `file`, `tel`, `sms`, `data`) is written out, and so is the scheme
    /// of a link with a port, since `host:port` would read as a scheme, or
    /// with a user name, which is written out too (`https://user@host/*`).
    /// A password is never written: `user:*@` stands for it. Empty for text
    /// that is not a URL.
    pub candidates: Vec<String>,
    /// A wildcard cannot say what distinguishes this link, so a precise rule
    /// needs a JS config: a regex matcher, or a `url` function with a
    /// capture group for a rewrite. `why` says which.
    pub needs_js: bool,
    /// One line per candidate, in order, then one per reason `needs_js` is
    /// set.
    pub why: Vec<&'static str>,
}

/// Query parameters a redirect wrapper carries its destination in.
const WRAPPER_PARAMS: [&str; 12] = [
    "q",
    "u",
    "url",
    "link",
    "target",
    "dest",
    "destination",
    "to",
    "href",
    "goto",
    "out",
    "r",
];

/// Schemes a pattern without one matches anyway, from Finicky's
/// `matchWildcard`.
const IMPLICIT_SCHEMES: [&str; 8] = [
    "http", "https", "ftp", "mailto", "file", "tel", "sms", "data",
];

const WHY_PAGE: &str = "Matches this page only.";
const WHY_PAGE_ANY_QUERY: &str = "Matches paths that start here, any query.";
const WHY_SECTION: &str = "Matches everything under the first part of the path.";
const WHY_HOST: &str = "Matches everything on this host.";
const WHY_SCHEME: &str = "Matches every link with this scheme.";
const WHY_NOT_A_URL: &str = "This is not a URL PwrRoute can match.";
const WHY_ID: &str = "The first part of the path looks like an id. A wildcard cannot limit a part to ids; a regex matcher in a JS config can.";
const WHY_INNER_URL: &str = "A query parameter carries another URL. Unwrapping it is a rewrite with a capture group, which needs a url function in a JS config.";
const WHY_USER_SECTION: &str =
    "Matches everything under the first part of the path, for this user name.";
const WHY_USER_HOST: &str = "Matches everything on this host, for this user name.";

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
    // A user name sits between `//` and the host, where the prefix a
    // scheme-less pattern gets has no room for it, so it is written out with
    // the scheme. A password never goes in a pattern: `user:*@` stands for
    // it.
    let user = parsed.username();
    let userinfo = match (user.is_empty(), parsed.password()) {
        (true, _) => String::new(),
        (false, None) => format!("{}@", escape(user)),
        (false, Some(_)) => format!("{}:*@", escape(user)),
    };
    // A pattern that starts with `host:port` reads as one with a scheme, so
    // a port needs the scheme written out too.
    let prefix =
        if IMPLICIT_SCHEMES.contains(&scheme) && parsed.port().is_none() && userinfo.is_empty() {
            String::new()
        } else if parsed.host_str().is_some() {
            format!("{scheme}://")
        } else {
            format!("{scheme}:")
        };
    let link = link_pattern(&parsed, &prefix, &userinfo);
    let (why_section, why_host) = if userinfo.is_empty() {
        (WHY_SECTION, WHY_HOST)
    } else {
        (WHY_USER_SECTION, WHY_USER_HOST)
    };

    // A query or fragment is never written into a pattern: an OAuth link's
    // `state` or a tracking id changes every time, and a pattern without a
    // `*` must equal the whole link, so it would never match again.
    let tail = parsed.query().is_some() || parsed.fragment().is_some();
    let Some(host) = parsed.host_str() else {
        // mailto:, tel:, data: and the like: the whole link, or every link
        // of its kind.
        if tail {
            suggestion
                .candidates
                .push(format!("{scheme}:{}*", escape(parsed.path())));
            suggestion.why.push(WHY_PAGE_ANY_QUERY);
        } else {
            suggestion.candidates.push(link);
            suggestion.why.push(WHY_PAGE);
        }
        suggestion.candidates.push(format!("{scheme}:*"));
        suggestion.why.push(WHY_SCHEME);
        return suggestion;
    };
    let origin = match parsed.port() {
        Some(port) => format!("{prefix}{userinfo}{}:{port}", escape(host)),
        None => format!("{prefix}{userinfo}{}", escape(host)),
    };
    let path = parsed.path();
    if path.is_empty() {
        // A scheme of its own (`slack://channel?id=1`) may have no path at
        // all, so `host/*` would not match it. With a query, `host*` is
        // already the page with any query.
        if !tail {
            suggestion.candidates.push(link);
            suggestion.why.push(WHY_PAGE);
        }
        suggestion.candidates.push(format!("{origin}*"));
        suggestion.why.push(why_host);
        return suggestion;
    }
    let first = path.trim_start_matches('/').split('/').next().unwrap_or("");
    if !first.is_empty() {
        if path.starts_with(&format!("/{first}/")) {
            suggestion
                .candidates
                .push(format!("{origin}/{}/*", escape(first)));
            suggestion.why.push(why_section);
        } else if tail {
            // `host/first/*` needs a slash after the segment, which this
            // link does not have: the page, with any query.
            suggestion
                .candidates
                .push(format!("{origin}{}*", escape(path)));
            suggestion.why.push(WHY_PAGE_ANY_QUERY);
        } else {
            // No slash after the segment and nothing after the path: the
            // link itself.
            suggestion.candidates.push(link);
            suggestion.why.push(WHY_PAGE);
        }
    }
    suggestion.candidates.push(format!("{origin}/*"));
    suggestion.why.push(why_host);

    if looks_like_id(first) {
        suggestion.needs_js = true;
        suggestion.why.push(WHY_ID);
    }
    // A redirect wrapper (`google.com/url?q=…`, `l.example/l.php?u=…`)
    // carries its destination in a parameter named for it. Other URLs in a
    // query are not the link's destination: an OAuth sign-in's
    // `redirect_uri` (RFC 6749) or `resource` (RFC 8707, which MCP clients
    // send), a login's `return_to`. Those need nothing unwrapped.
    if parsed.query_pairs().any(|(name, value)| {
        WRAPPER_PARAMS.contains(&name.to_ascii_lowercase().as_str())
            && (value.starts_with("http://") || value.starts_with("https://"))
    }) {
        suggestion.needs_js = true;
        suggestion.why.push(WHY_INNER_URL);
    }
    suggestion
}

/// The link itself as a pattern: exact when it has no password, else the
/// same link with `user:*@` in place of the credentials, since a password
/// must not be written into a rule.
fn link_pattern(parsed: &url::Url, prefix: &str, userinfo: &str) -> String {
    if parsed.password().is_none() {
        return escape(parsed.as_str());
    }
    let mut rest = String::new();
    if let Some(host) = parsed.host_str() {
        rest.push_str(&escape(host));
    }
    if let Some(port) = parsed.port() {
        rest.push_str(&format!(":{port}"));
    }
    rest.push_str(&escape(parsed.path()));
    if let Some(query) = parsed.query() {
        rest.push_str(&format!("?{}", escape(query)));
    }
    if let Some(fragment) = parsed.fragment() {
        rest.push_str(&format!("#{}", escape(fragment)));
    }
    format!("{prefix}{userinfo}{rest}")
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
        let s = suggest_rule("https://github.com/pwrdrvr");
        assert_eq!(s.candidates, ["https://github.com/pwrdrvr", "github.com/*"]);
        assert_eq!(s.why, [WHY_PAGE, WHY_HOST]);
        assert!(!s.needs_js);
    }

    #[test]
    fn the_query_and_fragment_never_enter_a_pattern() {
        let s = suggest_rule("https://github.com/pwrdrvr?tab=repositories");
        assert_eq!(s.candidates, ["github.com/pwrdrvr*", "github.com/*"]);
        assert_eq!(s.why, [WHY_PAGE_ANY_QUERY, WHY_HOST]);
        assert_eq!(
            candidates("https://example.com/page#top"),
            ["example.com/page*", "example.com/*"]
        );
        // An OAuth sign-in: one segment, a long query, a redirect_uri that
        // is no link to unwrap.
        let s = suggest_rule(
            "https://auth.example.com/authorize?response_type=code&client_id=demo\
             &redirect_uri=https%3A%2F%2Fapp.example%2Fcallback&state=demo-state-0001",
        );
        assert_eq!(
            s.candidates,
            ["auth.example.com/authorize*", "auth.example.com/*"]
        );
        assert!(!s.needs_js, "{:?}", s.why);
        // An MCP client's sign-in also names the server as `resource`.
        assert!(
            !suggest_rule(
                "https://mcp.example.com/authorize?response_type=code&client_id=demo\
                 &redirect_uri=http%3A%2F%2F127.0.0.1%3A3118%2Fcallback\
                 &resource=https%3A%2F%2Fmcp.example.com%2Fmcp&state=one"
            )
            .needs_js
        );
        assert!(
            !suggest_rule("https://id.example/login?return_to=https%3A%2F%2Fapp.example%2Fhome")
                .needs_js
        );
        assert!(
            s.candidates.iter().all(|c| !c.contains('?')),
            "{:?}",
            s.candidates
        );
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
        assert_eq!(candidates("slack://channel?id=1"), ["slack://channel*"]);
        assert_eq!(
            candidates("mailto:alice@example.com?subject=hi"),
            ["mailto:alice@example.com*", "mailto:*"]
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
        assert!(
            suggest_rule("https://l.example.com/l.php?u=https%3A%2F%2Fexample.com%2Fx").needs_js
        );
        assert!(!suggest_rule("https://docs.example/guide/intro").needs_js);
    }

    #[test]
    fn a_user_name_is_written_out_and_a_password_never_is() {
        let s = suggest_rule("https://user@host.example/a/b");
        assert_eq!(
            s.candidates,
            [
                "https://user@host.example/a/*",
                "https://user@host.example/*"
            ]
        );
        assert_eq!(s.why, [WHY_USER_SECTION, WHY_USER_HOST]);
        assert!(!s.needs_js);
        assert_eq!(
            candidates("https://user@host.example/"),
            ["https://user@host.example/*"]
        );
        let s = suggest_rule("https://user:s3cret@host.example/a?q=1#f");
        assert_eq!(
            s.candidates,
            [
                "https://user:*@host.example/a*",
                "https://user:*@host.example/*"
            ]
        );
        assert!(s.candidates.iter().all(|c| !c.contains("s3cret")));
        assert_eq!(
            candidates("http://user:s3cret@localhost:3000/x/y"),
            [
                "http://user:*@localhost:3000/x/*",
                "http://user:*@localhost:3000/*"
            ]
        );
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
