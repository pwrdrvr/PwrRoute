//! The published release list, from GitHub's REST API, for the release
//! channel ([`crate::update_channel`]): each release's tag, its Pre-release
//! flag, and whether it carries this platform's Velopack feed.
//!
//! One page of 30 is usually all of it. GitHub's "latest" release (the
//! newest promoted one) anchors the read: while it is not on the pages read
//! so far, the next page is read too, up to [`MAX_PAGES`], so a long run of
//! alphas cannot push Stable · Latest out of view. Unauthenticated, as
//! Velopack's own GitHub source is: 60 requests an hour per address, and a
//! check takes two.

use std::{sync::Arc, time::Duration};

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
pub fn fetch(channel: &str) -> Result<Vec<Release>, String> {
    let agent = agent();
    let latest = latest_tag(&agent)?;
    let feed = feed_name(channel);
    let mut releases = Vec::new();
    let mut seen_latest = latest.is_none();
    for page in 1..=MAX_PAGES {
        let body = get(
            &agent,
            &format!("{API}/releases?per_page={PER_PAGE}&page={page}"),
        )?
        .ok_or_else(|| "GitHub has no release list for PwrRoute".to_owned())?;
        let (usable, listed, tags) = parse_page(&body, &feed)?;
        releases.extend(usable);
        seen_latest |= latest.as_ref().is_some_and(|latest| tags.contains(latest));
        if listed < PER_PAGE || seen_latest {
            break;
        }
    }
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
fn latest_tag(agent: &Agent) -> Result<Option<String>, String> {
    #[derive(Deserialize)]
    struct Latest {
        tag_name: String,
    }
    let Some(body) = get(agent, &format!("{API}/releases/latest"))? else {
        return Ok(None);
    };
    serde_json::from_str::<Latest>(&body)
        .map(|latest| Some(latest.tag_name))
        .map_err(|error| format!("GitHub's latest release did not parse ({error})"))
}

/// The body of a GET; `None` for a 404.
fn get(agent: &Agent, url: &str) -> Result<Option<String>, String> {
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
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
