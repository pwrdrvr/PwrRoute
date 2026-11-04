//! The published release list, from GitHub's REST API, for the release
//! channel ([`crate::update_channel`]): each release's tag, its Pre-release
//! flag, and whether it carries this platform's Velopack feed.
//!
//! One page of 30 is usually all of it. GitHub's "latest" release (the
//! newest promoted one) anchors the read: while it is not on the pages read
//! so far, the next page is read too, up to [`MAX_PAGES`], so a long run of
//! alphas cannot push Stable · Latest out of view. Unauthenticated, as
//! Velopack's own GitHub source is: 60 requests an hour per address, and a
//! check takes two. Those first two requests overlap on one scoped helper
//! thread; subsequent pages are read only when the anchor requires them.
//! All requests share a 20-second budget, including reading their bodies.
//! Cancellation stops new requests and discards results; blocking I/O
//! already under way must finish or time out before the helper is joined.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use ureq::{
    Agent,
    tls::{RootCerts, TlsConfig, TlsProvider},
};

use crate::update_channel::Release;

/// The repository's API root.
const API: &str = "https://api.github.com/repos/pwrdrvr/PwrRoute";

/// Releases per page: GitHub's default is 30.
const PER_PAGE: usize = 30;

/// How far a read goes looking for the latest release.
pub const MAX_PAGES: usize = 10;

/// The total GitHub discovery budget, rather than a fresh timeout per page.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A release as the API lists it, with only what choosing one needs.
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
}

/// The releases a copy on Velopack channel `channel` ("osx", "win") can
/// update to: published, with `releases.<channel>.json` attached.
pub fn fetch(channel: &str, cancelled: &AtomicBool) -> Result<Vec<Release>, String> {
    let started = Instant::now();
    let agent = agent();
    fetch_with(
        channel,
        cancelled,
        &|request, timeout| get(&agent, &request.url(), timeout),
        &|| started.elapsed(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Request {
    Latest,
    Page(usize),
}

impl Request {
    fn url(self) -> String {
        match self {
            Self::Latest => format!("{API}/releases/latest"),
            Self::Page(page) => format!("{API}/releases?per_page={PER_PAGE}&page={page}"),
        }
    }
}

fn fetch_with(
    channel: &str,
    cancelled: &AtomicBool,
    transport: &(impl Fn(Request, Duration) -> Result<Option<String>, String> + Sync),
    elapsed: &(impl Fn() -> Duration + Sync),
) -> Result<Vec<Release>, String> {
    let remaining = || {
        if cancelled.load(Ordering::Acquire) {
            return Err("GitHub release discovery was cancelled".to_owned());
        }
        TIMEOUT
            .checked_sub(elapsed())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| "GitHub release discovery timed out (20 second budget)".to_owned())
    };
    let request = |kind| {
        let result = transport(kind, remaining()?);
        // Do not publish a response (even a successful final page) after
        // shutdown or expiry. The same gate prevents starting later pages.
        remaining()?;
        result
    };
    remaining()?;
    let (latest, first) = thread::scope(|scope| {
        let latest = thread::Builder::new()
            .name("pwrroute-release-anchor".into())
            .spawn_scoped(scope, || request(Request::Latest))
            .map_err(|error| format!("could not start GitHub release discovery ({error})"))?;
        let first = request(Request::Page(1));
        // Always join, including when page one failed. Dropping a join
        // handle does not cancel ureq; its remaining global timeout bounds
        // the wait. No application HTTP worker outlives this fetch.
        let latest = latest
            .join()
            .map_err(|_| "GitHub release discovery worker panicked".to_owned())?;
        Ok::<_, String>((latest, first))
    })?;
    remaining()?;
    let latest = latest_tag(latest?)?;
    let mut first = Some(first);
    let feed = feed_name(channel);
    let mut releases = Vec::new();
    let mut seen_latest = latest.is_none();
    for page in 1..=MAX_PAGES {
        let body = first
            .take()
            .unwrap_or_else(|| request(Request::Page(page)))?
            .ok_or_else(|| "GitHub has no release list for PwrRoute".to_owned())?;
        let (usable, listed, tags) = parse_page(&body, &feed)?;
        releases.extend(usable);
        seen_latest |= latest.as_ref().is_some_and(|latest| tags.contains(latest));
        if listed < PER_PAGE || seen_latest {
            break;
        }
    }
    remaining()?;
    Ok(releases)
}

/// `releases.<channel>.json`: the Velopack feed a release must carry.
pub fn feed_name(channel: &str) -> String {
    format!("releases.{channel}.json")
}

/// Where a release's files download from: Velopack's HTTP source reads
/// `releases.<channel>.json` and the packages under it.
pub fn download_base(tag: &str) -> String {
    format!("https://github.com/pwrdrvr/PwrRoute/releases/download/{tag}/")
}

/// One page of the list: the usable releases, how many were listed, and
/// every tag listed (for the latest-release anchor).
fn parse_page(body: &str, feed: &str) -> Result<(Vec<Release>, usize, Vec<String>), String> {
    let listed: Vec<ApiRelease> = serde_json::from_str(body)
        .map_err(|error| format!("GitHub's release list did not parse ({error})"))?;
    let count = listed.len();
    let tags = listed.iter().map(|r| r.tag_name.clone()).collect();
    let usable = listed
        .into_iter()
        .filter(|r| !r.draft && r.assets.iter().any(|a| a.name.eq_ignore_ascii_case(feed)))
        .map(|r| Release::new(r.tag_name, r.prerelease))
        .collect();
    Ok((usable, count, tags))
}

/// The newest promoted release's tag; `None` while nothing is promoted.
fn latest_tag(body: Option<String>) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Latest {
        tag_name: String,
    }
    let Some(body) = body else {
        return Ok(None);
    };
    serde_json::from_str::<Latest>(&body)
        .map(|latest| Some(latest.tag_name))
        .map_err(|error| format!("GitHub's latest release did not parse ({error})"))
}

/// The body of a GET; `None` for a 404.
fn get(agent: &Agent, url: &str, timeout: Duration) -> Result<Option<String>, String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .config()
        .timeout_global(Some(timeout))
        .build()
        .call()
        .map_err(|error| format!("could not reach GitHub ({error})"))?;
    match response.status().as_u16() {
        200 => response
            .body_mut()
            .read_to_string()
            .map(Some)
            .map_err(|error| format!("could not read GitHub's answer ({error})")),
        404 => Ok(None),
        403 | 429 => {
            Err("GitHub's hourly limit for this network is used up; try again later".into())
        }
        status => Err(format!("GitHub answered {status}")),
    }
}

fn agent() -> Agent {
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .root_certs(RootCerts::PlatformVerifier)
        .build();
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("PwrRoute/", env!("CARGO_PKG_VERSION")))
        .tls_config(tls)
        .build()
        .new_agent()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    };

    // Timeouts are deadlock guards only; overlap is proved by both
    // transports entering before either is allowed to return.
    const GUARD: Duration = Duration::from_secs(5);

    #[derive(Default)]
    struct Gate(Mutex<bool>, Condvar);

    impl Gate {
        fn wait(&self) {
            let open = self.0.lock().unwrap();
            drop(self.1.wait_while(open, |open| !*open).unwrap());
        }

        fn open(&self) {
            *self.0.lock().unwrap() = true;
            self.1.notify_all();
        }
    }

    fn page(count: usize, tag: &str) -> String {
        serde_json::to_string(&vec![
            serde_json::json!({
                "tag_name": tag,
                "prerelease": true,
                "assets": [{"name": "releases.osx.json"}]
            });
            count
        ])
        .unwrap()
    }

    #[test]
    fn initial_requests_overlap_and_page_one_is_reused() {
        let gate = Gate::default();
        let (entered, entries) = mpsc::channel();
        let cancelled = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                fetch_with(
                    "osx",
                    &cancelled,
                    &|request, _| {
                        entered.send(request).unwrap();
                        gate.wait();
                        match request {
                            Request::Latest => Ok(Some(r#"{"tag_name":"v0.1.0"}"#.into())),
                            Request::Page(1) => Ok(Some(page(PER_PAGE, "v0.1.0"))),
                            other => panic!("unnecessary request: {other:?}"),
                        }
                    },
                    &|| Duration::ZERO,
                )
            });
            let first = entries.recv_timeout(GUARD).unwrap();
            let second = entries.recv_timeout(GUARD);
            gate.open();
            let releases = worker.join().unwrap().unwrap();
            assert_ne!(
                first,
                second.expect("both requests must enter before either returns")
            );
            assert_eq!(releases.len(), PER_PAGE);
            assert!(
                entries.try_recv().is_err(),
                "page one must not be fetched again"
            );
        });
    }

    #[test]
    fn pagination_stops_at_the_anchor_even_without_its_platform_feed() {
        let requests = Mutex::new(Vec::new());
        let releases = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|request, _| {
                requests.lock().unwrap().push(request);
                Ok(Some(match request {
                    Request::Latest => r#"{"tag_name":"v0.1.0"}"#.into(),
                    Request::Page(1) => page(PER_PAGE, "v0.2.0-alpha.1"),
                    Request::Page(2) => {
                        page(PER_PAGE, "v0.1.0").replace("releases.osx.json", "releases.win.json")
                    }
                    other => panic!("unnecessary request: {other:?}"),
                }))
            },
            &|| Duration::ZERO,
        )
        .unwrap();
        assert_eq!(releases.len(), PER_PAGE);
        assert_eq!(requests.into_inner().unwrap().len(), 3);
    }

    #[test]
    fn missing_anchor_stops_at_a_short_page_or_the_page_limit() {
        for short_page in [Some(2), None] {
            let pages = Mutex::new(Vec::new());
            let releases = fetch_with(
                "osx",
                &AtomicBool::new(false),
                &|request, _| {
                    Ok(Some(match request {
                        Request::Latest => r#"{"tag_name":"missing"}"#.into(),
                        Request::Page(number) => {
                            pages.lock().unwrap().push(number);
                            page(
                                if short_page == Some(number) {
                                    0
                                } else {
                                    PER_PAGE
                                },
                                "v0.2.0-alpha.1",
                            )
                        }
                    }))
                },
                &|| Duration::ZERO,
            )
            .unwrap();
            let expected = short_page.unwrap_or(MAX_PAGES);
            assert_eq!(
                pages.into_inner().unwrap(),
                (1..=expected).collect::<Vec<_>>()
            );
            assert_eq!(
                releases.len(),
                PER_PAGE * if short_page.is_some() { 1 } else { MAX_PAGES }
            );
        }
    }

    #[test]
    fn no_promoted_release_needs_only_page_one_and_no_releases_is_empty() {
        for count in [0, PER_PAGE] {
            let releases = fetch_with(
                "osx",
                &AtomicBool::new(false),
                &|request, _| match request {
                    Request::Latest => Ok(None),
                    Request::Page(1) => Ok(Some(page(count, "v0.2.0-alpha.1"))),
                    other => panic!("unnecessary request: {other:?}"),
                },
                &|| Duration::ZERO,
            )
            .unwrap();
            assert_eq!(releases.len(), count);
        }
        let error = fetch_with("osx", &AtomicBool::new(false), &|_, _| Ok(None), &|| {
            Duration::ZERO
        })
        .unwrap_err();
        assert_eq!(error, "GitHub has no release list for PwrRoute");
    }

    #[test]
    fn errors_do_not_turn_partial_pages_into_channel_fallbacks() {
        for failure in [Request::Latest, Request::Page(1), Request::Page(2)] {
            let error = fetch_with(
                "osx",
                &AtomicBool::new(false),
                &|request, _| {
                    if request == failure {
                        return Err("rate limited or unavailable".into());
                    }
                    Ok(Some(match request {
                        Request::Latest => r#"{"tag_name":"missing"}"#.into(),
                        Request::Page(_) => page(PER_PAGE, "v0.2.0-alpha.1"),
                    }))
                },
                &|| Duration::ZERO,
            )
            .unwrap_err();
            assert_eq!(error, "rate limited or unavailable");
        }
        let error = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|request, _| {
                Ok(Some(match request {
                    Request::Latest => "{".into(),
                    Request::Page(_) => "[]".into(),
                }))
            },
            &|| Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.contains("latest release did not parse"));
    }

    #[test]
    fn pagination_uses_the_remaining_total_budget_and_discards_partial_results() {
        let elapsed = AtomicU64::new(0);
        let page_timeouts = Mutex::new(Vec::new());
        let result = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|request, timeout| {
                Ok(Some(match request {
                    Request::Latest => r#"{"tag_name":"missing"}"#.into(),
                    Request::Page(number) => {
                        page_timeouts.lock().unwrap().push((number, timeout));
                        elapsed.store(if number == 1 { 7 } else { 20 }, Ordering::SeqCst);
                        page(PER_PAGE, "v0.2.0-alpha.1")
                    }
                }))
            },
            &|| Duration::from_secs(elapsed.load(Ordering::SeqCst)),
        );
        assert!(result.unwrap_err().contains("timed out"));
        assert_eq!(
            page_timeouts.into_inner().unwrap(),
            [(1, Duration::from_secs(20)), (2, Duration::from_secs(13))]
        );
    }

    #[test]
    fn cancellation_prevents_requests_and_discards_in_flight_results() {
        let cancelled = AtomicBool::new(true);
        let result = fetch_with(
            "osx",
            &cancelled,
            &|_, _| panic!("cancelled before starting"),
            &|| Duration::ZERO,
        );
        assert!(result.unwrap_err().contains("cancelled"));

        cancelled.store(false, Ordering::SeqCst);
        let result = fetch_with(
            "osx",
            &cancelled,
            &|request, _| {
                Ok(Some(match request {
                    Request::Latest => r#"{"tag_name":"missing"}"#.into(),
                    Request::Page(1) => {
                        cancelled.store(true, Ordering::SeqCst);
                        page(PER_PAGE, "v0.2.0-alpha.1")
                    }
                    other => panic!("request after cancellation: {other:?}"),
                }))
            },
            &|| Duration::ZERO,
        );
        assert!(result.unwrap_err().contains("cancelled"));
    }

    #[test]
    fn failure_or_shutdown_joins_the_other_initial_request_before_returning() {
        for shutdown in [false, true] {
            let cancelled = AtomicBool::new(false);
            let gate = Gate::default();
            let (entered, entries) = mpsc::channel();
            let (page_failed, failure) = mpsc::channel();
            let helper_finished = AtomicBool::new(false);
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| {
                    let result = fetch_with(
                        "osx",
                        &cancelled,
                        &|request, _| {
                            entered.send(request).unwrap();
                            match request {
                                Request::Latest => {
                                    gate.wait();
                                    helper_finished.store(true, Ordering::SeqCst);
                                    Ok(None)
                                }
                                Request::Page(1) => {
                                    page_failed.send(()).unwrap();
                                    Err("page one failed".into())
                                }
                                other => panic!("request after failure: {other:?}"),
                            }
                        },
                        &|| Duration::ZERO,
                    );
                    assert!(
                        helper_finished.load(Ordering::SeqCst),
                        "fetch left a live HTTP worker"
                    );
                    result
                });
                // Both have entered, and the caller's request has failed,
                // but the helper is still blocked until explicitly released.
                let first = entries.recv_timeout(GUARD);
                let second = entries.recv_timeout(GUARD);
                let failed = failure.recv_timeout(GUARD);
                if shutdown {
                    cancelled.store(true, Ordering::SeqCst);
                }
                gate.open();
                let error = worker.join().unwrap().unwrap_err();
                assert!(first.is_ok() && second.is_ok() && failed.is_ok());
                assert_eq!(
                    error,
                    if shutdown {
                        "GitHub release discovery was cancelled"
                    } else {
                        "page one failed"
                    }
                );
            });
        }
    }

    #[test]
    fn latest_errors_keep_precedence_when_both_initial_requests_fail() {
        let error = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|request, _| {
                Err(if request == Request::Latest {
                    "latest failed"
                } else {
                    "page failed"
                }
                .into())
            },
            &|| Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(error, "latest failed");
    }

    #[test]
    fn an_expired_budget_starts_no_requests_even_for_an_empty_list() {
        let error = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|_, _| panic!("request after deadline"),
            &|| TIMEOUT,
        )
        .unwrap_err();
        assert!(error.contains("timed out"));

        let elapsed = AtomicU64::new(0);
        let error = fetch_with(
            "osx",
            &AtomicBool::new(false),
            &|request, _| match request {
                Request::Latest => Ok(None),
                Request::Page(1) => {
                    elapsed.store(20, Ordering::SeqCst);
                    Ok(Some("[]".into()))
                }
                other => panic!("request after deadline: {other:?}"),
            },
            &|| Duration::from_secs(elapsed.load(Ordering::SeqCst)),
        )
        .unwrap_err();
        assert!(error.contains("timed out"));
    }

    #[test]
    fn http_statuses_keep_404_and_rate_limit_semantics() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };

        for status in [200, 404, 403, 429, 500] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket.set_read_timeout(Some(GUARD)).unwrap();
                let mut headers = Vec::new();
                let mut byte = [0];
                while !headers.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).unwrap();
                    headers.push(byte[0]);
                }
                write!(
                    socket,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]"
                )
                .unwrap();
            });
            let result = get(&agent(), &format!("http://{address}/releases"), GUARD);
            server.join().unwrap();
            match status {
                200 => assert_eq!(result.unwrap(), Some("[]".into())),
                404 => assert_eq!(result.unwrap(), None),
                403 | 429 => assert_eq!(
                    result.unwrap_err(),
                    "GitHub's hourly limit for this network is used up; try again later"
                ),
                _ => assert_eq!(result.unwrap_err(), "GitHub answered 500"),
            }
        }
    }

    #[test]
    fn a_page_keeps_published_releases_with_this_platforms_feed() {
        let body = r#"[
            {"tag_name": "v0.2.0-alpha.1", "draft": true, "prerelease": true,
             "assets": [{"name": "releases.osx.json"}]},
            {"tag_name": "v0.1.0", "prerelease": true,
             "assets": [{"name": "releases.osx.json"}, {"name": "releases.win.json"}]},
            {"tag_name": "v0.1.0-rc.3", "prerelease": false,
             "assets": [{"name": "RELEASES-OSX.json"}, {"name": "Releases.OSX.json"}]},
            {"tag_name": "v0.0.9", "prerelease": false,
             "assets": [{"name": "releases.win.json"}]}
        ]"#;
        let (usable, listed, tags) = parse_page(body, &feed_name("osx")).unwrap();
        assert_eq!(listed, 4);
        assert_eq!(tags.len(), 4);
        assert_eq!(
            usable,
            [
                Release::new("v0.1.0", true),
                Release::new("v0.1.0-rc.3", false)
            ]
        );
        assert!(parse_page("{", "releases.osx.json").is_err());
    }

    #[test]
    fn a_release_downloads_from_its_own_tag() {
        assert_eq!(
            download_base("v0.1.0"),
            "https://github.com/pwrdrvr/PwrRoute/releases/download/v0.1.0/"
        );
    }
}

#[cfg(test)]
#[path = "property_tests/github_releases.rs"]
mod properties;
