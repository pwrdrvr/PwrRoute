//! The Finicky importer's reading of a config: for each of its rewrites,
//! its handlers and its default browser, whether a visual rule in
//! `rules.json` holds it exactly, and where it is in the file.
//!
//! The review starts from what the config produced ([`Engine::describe`]),
//! not from its text. A wildcard matcher, a browser name with a profile, a
//! string rewrite and a fixed default browser are what `rules.json`
//! stores, and the router hands them to the same Finicky API, so they
//! import exactly. A function matcher, a `url` function, a browser
//! function and a regex (which `rules.json` cannot hold) stay JS.
//!
//! A function that looks like a visual rule gets a suggestion, found by
//! probing rather than by reading the JS: string literals in its source
//! become candidate patterns (a host is `host/*`, then `host/*` with
//! `*.host/*`; a GitHub org is `github.com/org/*`), and the function itself
//! runs on a set of probe links beside each candidate. A candidate the
//! function agrees with on every probe is the suggestion. That is evidence,
//! not a proof: a link no probe resembles may still be routed differently.
//! The probes are links built from the config's own literals plus the
//! recent links the caller passes; they stay in memory.

use std::{collections::HashMap, fs, ops::RangeInclusive};

use anyhow::Result;
use rquickjs::CatchResultExt;

use crate::{
    AppType, BrowserSpec, Description, Engine, FixedBrowser, MatcherSpec, Rewrite, RewriteAction,
    Rule, UrlSpec, js_error, rules,
};

/// A config as the importer reads it, in the order `openUrl` runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub rewrites: Vec<Reviewed>,
    pub handlers: Vec<Reviewed>,
    pub default_browser: Reviewed,
    /// How many links each suggestion was probed on: those built from the
    /// config's literals and [`Review::recent`] of the caller's.
    pub probes: usize,
    /// How many of the probes were the caller's recent links.
    pub recent: usize,
}

/// One rewrite, handler or default browser of the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewed {
    pub verdict: Verdict,
    /// Where it is in the config file, when it could be found there. A rule
    /// built in a file the config imports is not found.
    pub source: Option<SourceLines>,
}

/// What a visual rule can hold of a config's rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// This visual rule is the same rule, by construction.
    Imports(Visual),
    /// Only JS can say it. `why` is a sentence for the reader.
    StaysJs {
        why: &'static str,
        suggestion: Option<Suggested>,
    },
}

/// The `rules.json` entry a config's rule becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visual {
    Rewrite(Rewrite),
    Handler(Rule),
    /// `defaultBrowser` and `defaultProfile`.
    Default {
        browser: String,
        profile: String,
    },
}

/// A visual handler a function matcher agreed with on every probe link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggested {
    pub rule: Rule,
    /// The probe links both agreed on: every one there was.
    pub agreed: usize,
    /// Of those, how many both matched; the rest neither matched.
    pub matched: usize,
}

/// A rule's lines in the config file, with a line of context each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLines {
    /// The line number of `lines[0]`, from 1.
    pub first: usize,
    pub lines: Vec<String>,
    /// The line numbers of the rule itself.
    pub hit: RangeInclusive<usize>,
}

pub(crate) const WHY_FUNCTION: &str =
    "A function matcher can look at anything, so no visual rule is provably the same.";
pub(crate) const WHY_REGEX: &str = "rules.json holds wildcard patterns, not regular expressions, \
     so a regex matcher stays in the config.";
const WHY_OTHER: &str = "Its matcher is not one Finicky matches with.";
const WHY_URL_FUNCTION: &str =
    "A url function computes the new URL as it runs; a visual rewrite cannot.";
const WHY_URL_TEXT: &str = "Its url is not a full URL a visual rewrite can store.";
const WHY_BROWSER_FUNCTION: &str =
    "A browser function picks the browser per link; a visual rule names one.";
const WHY_ARGS: &str =
    "It passes launch arguments or a background flag, which visual rules do not store.";
const WHY_APP_TYPE: &str =
    "rules.json would read its app name as a different kind (name, bundle id or path).";

/// The most lines a rule's source spans before the rest is left out.
const MAX_HIT_LINES: usize = 8;

/// How many probe links one JS call takes, each call under the resolve
/// deadline.
const PROBE_CHUNK: usize = 25;

/// The combinations of candidate patterns tried per handler, at most.
const MAX_OPTIONS: usize = 16;

impl Engine {
    /// The importer's review of the JS or TS config: a verdict for each
    /// rewrite, handler and the default browser, with its source lines.
    /// `recent` are links to probe suggestions on besides those built from
    /// the config's literals (the router's recent links, in memory). `None`
    /// when `rules.json` is the whole config. Nothing is written.
    pub fn review_import(&self, recent: &[&str]) -> Result<Option<Review>> {
        let Some(description) = self.describe()? else {
            return Ok(None);
        };
        let text = self
            .config
            .as_deref()
            .and_then(|path| fs::read_to_string(path).ok())
            .unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();

        let mut cursor = 0;
        let rewrites = description
            .rewrites
            .iter()
            .map(|rewrite| Reviewed {
                verdict: rewrite_verdict(&rewrite.matchers, &rewrite.url),
                source: locate(&lines, &mut cursor, &anchors(&rewrite.matchers), &["url"]),
            })
            .collect();
        let mut cursor = 0;
        let mut handlers: Vec<Reviewed> = description
            .handlers
            .iter()
            .map(|handler| Reviewed {
                verdict: handler_verdict(&handler.matchers, &handler.browser),
                source: locate(
                    &lines,
                    &mut cursor,
                    &anchors(&handler.matchers),
                    &["browser"],
                ),
            })
            .collect();
        let default_browser = Reviewed {
            verdict: match visual_browser(&description.default_browser) {
                Ok((browser, profile)) => Verdict::Imports(Visual::Default { browser, profile }),
                Err(why) => Verdict::StaysJs {
                    why,
                    suggestion: None,
                },
            },
            source: locate(&lines, &mut 0, &["defaultBrowser".to_owned()], &[]),
        };

        let (probes, recent) = self.suggest_for(&description, &mut handlers, recent)?;
        Ok(Some(Review {
            rewrites,
            handlers,
            default_browser,
            probes,
            recent,
        }))
    }

    /// Fills in the suggestion of each handler that stays JS for a function
    /// or regex matcher and names a browser a visual rule can. Returns how
    /// many links were probed, and how many of them were `recent`.
    fn suggest_for(
        &self,
        description: &Description,
        handlers: &mut [Reviewed],
        recent: &[&str],
    ) -> Result<(usize, usize)> {
        let wanted: Vec<(usize, Vec<Vec<String>>)> = description
            .handlers
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                matches!(
                    handlers[*i].verdict,
                    Verdict::StaysJs {
                        why: WHY_FUNCTION | WHY_REGEX,
                        ..
                    }
                )
            })
            .filter(|(_, h)| visual_browser(&h.browser).is_ok())
            .map(|(i, h)| (i, options(&h.matchers)))
            .filter(|(_, options)| !options.is_empty())
            .collect();
        if wanted.is_empty() {
            return Ok((0, 0));
        }

        let mut urls = probe_links(&description.handlers);
        let derived = urls.len();
        for link in recent {
            if url::Url::parse(link).is_ok() && !urls.iter().any(|u| u == link) {
                urls.push((*link).to_owned());
            }
        }
        let recent_count = urls.len() - derived;
        let refs: Vec<&str> = urls.iter().map(String::as_str).collect();
        let functions = self.probe(&refs)?;

        let mut pattern_hits: HashMap<String, Vec<bool>> = HashMap::new();
        for (index, options) in wanted {
            let Ok((browser, profile)) = visual_browser(&description.handlers[index].browser)
            else {
                continue;
            };
            let truth: Vec<bool> = functions.iter().map(|row| row[index]).collect();
            if !truth.iter().any(|&hit| hit) {
                continue;
            }
            for patterns in options {
                let mut agrees = true;
                for pattern in &patterns {
                    if !pattern_hits.contains_key(pattern) {
                        let hits = self.pattern_hits(pattern, &refs)?;
                        pattern_hits.insert(pattern.clone(), hits);
                    }
                }
                for (u, &want) in truth.iter().enumerate() {
                    let got = patterns.iter().any(|p| pattern_hits[p][u]);
                    if got != want {
                        agrees = false;
                        break;
                    }
                }
                if agrees {
                    if let Verdict::StaysJs { suggestion, .. } = &mut handlers[index].verdict {
                        *suggestion = Some(Suggested {
                            rule: Rule {
                                patterns,
                                browser,
                                profile,
                                ..Rule::default()
                            },
                            agreed: truth.len(),
                            matched: truth.iter().filter(|&&hit| hit).count(),
                        });
                    }
                    break;
                }
            }
        }
        self.console.clear();
        Ok((urls.len(), recent_count))
    }

    /// For each of `urls`, whether each of the config's own handlers
    /// matches it, as `openUrl` calls the matchers (no opener). The
    /// config's `console` output from these runs is dropped.
    fn probe(&self, urls: &[&str]) -> Result<Vec<Vec<bool>>> {
        let mut rows = Vec::with_capacity(urls.len());
        for chunk in urls.chunks(PROBE_CHUNK) {
            let resolve = self.limits.resolve;
            let part: Vec<Vec<bool>> = self.with_deadline(resolve, |ctx| {
                let probe: rquickjs::Function<'_> = ctx.eval("__pwrroute.probe")?;
                let list = rquickjs::Array::new(ctx.clone())?;
                for (i, url) in chunk.iter().enumerate() {
                    list.set(i, *url)?;
                }
                probe
                    .call((list,))
                    .catch(&ctx)
                    .map_err(|e| js_error("probing the config's handlers", &e))
            })?;
            rows.extend(part);
        }
        self.console.clear();
        Ok(rows)
    }

    /// Whether `pattern` matches each of `urls`, by Finicky's `isMatch`.
    fn pattern_hits(&self, pattern: &str, urls: &[&str]) -> Result<Vec<bool>> {
        let hits = self.collisions(pattern, urls)?;
        let mut hit = hits.iter().peekable();
        Ok(urls
            .iter()
            .map(|url| {
                if hit.peek().is_some_and(|h| h.as_str() == *url) {
                    hit.next();
                    true
                } else {
                    false
                }
            })
            .collect())
    }
}

fn rewrite_verdict(matchers: &[MatcherSpec], url: &UrlSpec) -> Verdict {
    let stays = |why| Verdict::StaysJs {
        why,
        suggestion: None,
    };
    let patterns = match wildcards(matchers) {
        Ok(patterns) => patterns,
        Err(why) => return stays(why),
    };
    match url {
        UrlSpec::Function(_) => stays(WHY_URL_FUNCTION),
        UrlSpec::Url(target) if url::Url::parse(target).is_err() => stays(WHY_URL_TEXT),
        UrlSpec::Url(target) => Verdict::Imports(Visual::Rewrite(Rewrite {
            patterns,
            actions: vec![RewriteAction::Url(target.clone())],
            ..Rewrite::default()
        })),
    }
}

fn handler_verdict(matchers: &[MatcherSpec], browser: &BrowserSpec) -> Verdict {
    let stays = |why| Verdict::StaysJs {
        why,
        suggestion: None,
    };
    // The matcher decides first: a function matcher stays JS whatever the
    // browser, and says so in its own words.
    let patterns = match wildcards(matchers) {
        Ok(patterns) => patterns,
        Err(why) => {
            return match visual_browser(browser) {
                Ok(_) => stays(why),
                Err(browser_why) if why == WHY_OTHER => stays(browser_why),
                Err(_) => stays(why),
            };
        }
    };
    match visual_browser(browser) {
        Ok((browser, profile)) => Verdict::Imports(Visual::Handler(Rule {
            patterns,
            browser,
            profile,
            ..Rule::default()
        })),
        Err(why) => stays(why),
    }
}

/// The matchers as wildcard patterns, or why they are not.
fn wildcards(matchers: &[MatcherSpec]) -> Result<Vec<String>, &'static str> {
    let mut patterns = Vec::new();
    let mut why = None;
    for matcher in matchers {
        match matcher {
            MatcherSpec::Wildcard(pattern) => patterns.push(pattern.clone()),
            MatcherSpec::Function(_) => why = Some(WHY_FUNCTION),
            MatcherSpec::Regex(_) => why = why.or(Some(WHY_REGEX)),
            MatcherSpec::Other(_) => why = why.or(Some(WHY_OTHER)),
        }
    }
    match why {
        Some(why) => Err(why),
        None if patterns.iter().all(String::is_empty) => Err(WHY_OTHER),
        None => Ok(patterns.into_iter().filter(|p| !p.is_empty()).collect()),
    }
}

/// A fixed browser as `rules.json` names it, `(browser, profile)`, or why it
/// cannot. `rules.json` tells a name, a bundle id and a path apart by their
/// shape, as Finicky does a string, so a browser whose `appType` is not the
/// one its name's shape says would open differently.
fn visual_browser(spec: &BrowserSpec) -> Result<(String, String), &'static str> {
    let fixed: &FixedBrowser = match spec {
        BrowserSpec::Function(_) => return Err(WHY_BROWSER_FUNCTION),
        BrowserSpec::Browser(fixed) => fixed,
    };
    // `browser: null` is Don't open, which `rules.json` writes as a word.
    if fixed.app_type == AppType::None {
        return Ok((rules::DONT_OPEN.to_owned(), String::new()));
    }
    if !fixed.args.is_empty() || fixed.open_in_background.is_some() {
        return Err(WHY_ARGS);
    }
    let shape = rules::app_type(&fixed.name);
    let kind = match fixed.app_type {
        AppType::AppName => "appName",
        AppType::BundleId => "bundleId",
        AppType::Path => "path",
        AppType::None => "none",
    };
    if shape != kind || fixed.name.trim().is_empty() {
        return Err(WHY_APP_TYPE);
    }
    Ok((fixed.name.clone(), fixed.profile.clone()))
}

// --- suggestions -----------------------------------------------------------

/// What a handler's literals suggest, as alternatives to try in order:
/// each is the whole handler's patterns, its wildcard matchers as they are
/// and each function or regex matcher as candidates from its literals.
fn options(matchers: &[MatcherSpec]) -> Vec<Vec<String>> {
    let mut combos: Vec<Vec<String>> = vec![Vec::new()];
    for matcher in matchers {
        let choices: Vec<Vec<String>> = match matcher {
            MatcherSpec::Wildcard(pattern) if pattern.is_empty() => continue,
            MatcherSpec::Wildcard(pattern) => vec![vec![pattern.clone()]],
            MatcherSpec::Function(function) => candidates(
                &string_literals(function.source.as_deref().unwrap_or("")),
                false,
            ),
            MatcherSpec::Regex(regex) => candidates(&regex_literals(regex), true),
            MatcherSpec::Other(_) => return Vec::new(),
        };
        if choices.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        for combo in &combos {
            for choice in &choices {
                let mut joined = combo.clone();
                for pattern in choice {
                    if !joined.contains(pattern) {
                        joined.push(pattern.clone());
                    }
                }
                next.push(joined);
            }
        }
        next.truncate(MAX_OPTIONS);
        combos = next;
    }
    combos.retain(|combo| !combo.is_empty());
    combos
}

/// Candidate pattern sets for one matcher's literals, narrowest first.
fn candidates(literals: &[String], regex: bool) -> Vec<Vec<String>> {
    let hosts = hosts(literals);
    let orgs = orgs(literals);
    let mut choices = Vec::new();
    let only_github = hosts.iter().all(|h| h == "github.com");
    if !orgs.is_empty() && only_github {
        choices.push(
            orgs.iter()
                .map(|org| format!("github.com/{}/*", escape(org)))
                .collect(),
        );
    }
    if !hosts.is_empty() {
        choices.push(hosts.iter().map(|h| format!("{}/*", escape(h))).collect());
        choices.push(
            hosts
                .iter()
                .flat_map(|h| [format!("{}/*", escape(h)), format!("*.{}/*", escape(h))])
                .collect(),
        );
        if regex {
            choices.push(hosts.iter().map(|h| format!("{}*", escape(h))).collect());
        }
    }
    choices
}

/// The literals that name a host: `host.example`, also written `.host.example`
/// or `*.host.example`, lowercased.
fn hosts(literals: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for literal in literals {
        if let Some(host) = host_literal(literal)
            && !found.contains(&host)
        {
            found.push(host);
        }
    }
    found
}

fn host_literal(text: &str) -> Option<String> {
    let host = text
        .trim()
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let label_ok = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let tld = labels.last()?;
    (labels.len() >= 2
        && labels.iter().all(label_ok)
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic()))
    .then_some(host)
}

/// The literals that could be a GitHub org or user: letters, digits and
/// inner hyphens, at most 39, optionally between slashes (`"/org/"`).
fn orgs(literals: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for literal in literals {
        let org = literal.trim().trim_matches('/');
        let ok = !org.is_empty()
            && org.len() <= 39
            && !org.starts_with('-')
            && !org.ends_with('-')
            && org.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if ok && !found.iter().any(|f: &String| f == org) {
            found.push(org.to_owned());
        }
    }
    found
}

/// The string literals in a function's source: `"…"`, `'…'`, and template
/// literals without a `${…}`. A heuristic over the text, not a parse: the
/// probes decide whether what it finds means anything.
fn string_literals(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, '"' | '\'' | '`') {
            continue;
        }
        let mut literal = String::new();
        let mut plain = true;
        while let Some(next) = chars.next() {
            match next {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        literal.push(escaped);
                    }
                }
                q if q == c => break,
                '$' if c == '`' && chars.peek() == Some(&'{') => {
                    plain = false;
                    literal.push(next);
                }
                _ => literal.push(next),
            }
        }
        if plain && !literal.is_empty() && !found.contains(&literal) {
            found.push(literal);
        }
    }
    found
}

/// The host-like runs in a regex written `/source/flags`, with its escapes
/// undone: `/^https:\/\/news\.example\.com/` has `news.example.com`.
fn regex_literals(regex: &str) -> Vec<String> {
    let body = regex
        .strip_prefix('/')
        .and_then(|rest| rest.rsplit_once('/'))
        .map_or(regex, |(body, _flags)| body);
    let mut plain = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                // `\w`, `\d` and the like are classes, not text.
                Some(e) if e.is_ascii_alphanumeric() => plain.push(' '),
                Some(e) => plain.push(e),
                None => {}
            }
        } else {
            plain.push(c);
        }
    }
    plain
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .filter(|run| !run.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Links to probe on, from every function and regex handler's literals:
/// each host bare, deep, as a subdomain, over http, with a suffix and with
/// a prefix; each org's repository and a different org's; and one host no
/// literal names.
fn probe_links(handlers: &[crate::HandlerRule]) -> Vec<String> {
    let mut hosts_seen = Vec::new();
    let mut orgs_seen = Vec::new();
    for handler in handlers {
        for matcher in &handler.matchers {
            let literals = match matcher {
                MatcherSpec::Function(function) => {
                    string_literals(function.source.as_deref().unwrap_or(""))
                }
                MatcherSpec::Regex(regex) => regex_literals(regex),
                _ => continue,
            };
            for host in hosts(&literals) {
                if !hosts_seen.contains(&host) {
                    hosts_seen.push(host);
                }
            }
            for org in orgs(&literals) {
                if !orgs_seen.contains(&org) {
                    orgs_seen.push(org);
                }
            }
        }
    }
    let mut links = Vec::new();
    let mut add = |link: String| {
        if !links.contains(&link) {
            links.push(link);
        }
    };
    for host in &hosts_seen {
        add(format!("https://{host}/"));
        add(format!("https://{host}/a/b?x=1"));
        add(format!("https://sub.{host}/"));
        add(format!("http://{host}/"));
        add(format!("https://{host}.evil/"));
        add(format!("https://not{host}/"));
    }
    for org in &orgs_seen {
        add(format!("https://github.com/{org}/repo"));
    }
    if !orgs_seen.is_empty() {
        let other = if orgs_seen.iter().any(|o| o == "other") {
            "other-org"
        } else {
            "other"
        };
        add(format!("https://github.com/{other}/repo"));
    }
    add("https://nothost.example/".to_owned());
    links
}

/// A literal `*` must be `\*` in a pattern.
fn escape(text: &str) -> String {
    text.replace('*', "\\*")
}

// --- source lines ----------------------------------------------------------

/// Text that finds a rule in the file: each pattern quoted, a regex as
/// written, the first line of a function's source, then the literals a
/// function holds, quoted.
fn anchors(matchers: &[MatcherSpec]) -> Vec<String> {
    let mut found = Vec::new();
    for matcher in matchers {
        match matcher {
            MatcherSpec::Wildcard(pattern) => {
                found.push(format!("\"{pattern}\""));
                found.push(format!("'{pattern}'"));
                found.push(format!("`{pattern}`"));
            }
            MatcherSpec::Regex(regex) => {
                if let Some((body, _)) = regex.rsplit_once('/') {
                    found.push(format!("{body}/"));
                }
            }
            MatcherSpec::Function(function) => {
                let source = function.source.as_deref().unwrap_or("");
                if let Some(first) = source.lines().next().map(str::trim)
                    && first.len() > 3
                {
                    found.push(first.to_owned());
                }
                for literal in string_literals(source) {
                    found.push(format!("\"{literal}\""));
                    found.push(format!("'{literal}'"));
                }
            }
            MatcherSpec::Other(_) => {}
        }
    }
    found
}

/// The rule's lines: the first line at or after `cursor` that holds one of
/// `anchors` (else the first anywhere), through the nearest line naming one
/// of `ends` within [`MAX_HIT_LINES`], with a line of context each side.
/// `cursor` moves past it, so equal rules find their own lines in order.
fn locate(
    lines: &[&str],
    cursor: &mut usize,
    anchors: &[String],
    ends: &[&str],
) -> Option<SourceLines> {
    if anchors.is_empty() || lines.is_empty() {
        return None;
    }
    let holds = |line: &str| anchors.iter().any(|a| line.contains(a.as_str()));
    let at = (*cursor..lines.len())
        .find(|&i| holds(lines[i]))
        .or_else(|| (0..lines.len()).find(|&i| holds(lines[i])))?;
    let names_end = |i: usize| ends.iter().any(|end| lines[i].contains(end));
    let (mut start, mut end) = (at, at);
    if !ends.is_empty() && !names_end(at) {
        let after = (at + 1..lines.len().min(at + MAX_HIT_LINES)).find(|&i| names_end(i));
        let before = (at.saturating_sub(MAX_HIT_LINES - 1)..at)
            .rev()
            .find(|&i| names_end(i));
        match (after, before) {
            (Some(after), _) => end = after,
            (None, Some(before)) => start = before,
            (None, None) => {}
        }
    }
    *cursor = end + 1;
    let first = start.saturating_sub(1);
    let last = (end + 1).min(lines.len() - 1);
    Some(SourceLines {
        first: first + 1,
        lines: lines[first..=last]
            .iter()
            .map(|l| (*l).to_owned())
            .collect(),
        hit: start + 1..=end + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_come_from_quotes_of_every_kind() {
        assert_eq!(
            string_literals(r#"(url) => hostnameIs(url, "host.example") || f('a\'b', `c`)"#),
            ["host.example", "a'b", "c"]
        );
        assert!(string_literals("(url) => `x${url.host}`").is_empty());
    }

    #[test]
    fn a_regex_gives_its_host_runs() {
        assert_eq!(
            regex_literals(r"/^https:\/\/news\.example\.com\/\d+/i"),
            ["https", "news.example.com"]
        );
    }

    #[test]
    fn hosts_and_orgs_are_told_apart() {
        let literals: Vec<String> = [
            "Host.Example",
            ".sub.example",
            "org",
            "/team/",
            "x y",
            "1.2",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(hosts(&literals), ["host.example", "sub.example"]);
        assert_eq!(orgs(&literals), ["org", "team"]);
    }

    #[test]
    fn candidates_go_from_the_host_to_its_subdomains() {
        assert_eq!(
            candidates(&["host.example".to_owned()], false),
            [
                vec!["host.example/*".to_owned()],
                vec!["host.example/*".to_owned(), "*.host.example/*".to_owned()],
            ]
        );
        assert_eq!(
            candidates(&["org".to_owned()], false),
            [vec!["github.com/org/*".to_owned()]]
        );
    }

    #[test]
    fn a_rule_is_found_with_its_browser_line() {
        let text = "export default {\n  handlers: [\n    {\n      match: \"a.example/*\",\n      browser: \"Firefox\",\n    },\n  ],\n};";
        let lines: Vec<&str> = text.lines().collect();
        let found = locate(
            &lines,
            &mut 0,
            &anchors(&[MatcherSpec::Wildcard("a.example/*".into())]),
            &["browser"],
        )
        .unwrap();
        assert_eq!(found.first, 3);
        assert_eq!(found.hit, 4..=5);
        assert_eq!(found.lines.len(), 4);
    }
}
