//! Links on the command line.

use std::ffi::OsString;

use pwrroute_router::{Link, OpenerInfo, Source};

/// `--source=apple-event`: the links on this command line arrived as a macOS
/// `kAEGetURL` Apple Event, which the link helper passes on when it has to
/// launch the app ([`apple_event_launch_args`]).
const SOURCE_FLAG: &str = "--source=";
const APPLE_EVENT: &str = "apple-event";
/// `--opener=<JSON>`: the app that sent them, as the router's
/// [`OpenerInfo`] serializes it.
const OPENER_FLAG: &str = "--opener=";

/// The arguments that launch the app for links that arrived as one Apple
/// Event: the links, then where they came from. Each link is a plain
/// argument, as [`links_from_args`] reads them, so an app that does not
/// know the flags still routes them. The opener is the first link's: every
/// link of one event shares it.
pub fn apple_event_launch_args(links: &[Link]) -> Vec<String> {
    let mut args: Vec<String> = links.iter().map(|link| link.url.clone()).collect();
    args.push(format!("{SOURCE_FLAG}{APPLE_EVENT}"));
    // Never the window title (an email subject, a document name): the
    // command line is visible to every process on the machine.
    if let Some(json) = links
        .first()
        .and_then(|link| link.opener.as_ref())
        .map(|opener| OpenerInfo {
            window_title: None,
            ..opener.clone()
        })
        .and_then(|opener| serde_json::to_string(&opener).ok())
    {
        args.push(format!("{OPENER_FLAG}{json}"));
    }
    args
}

/// How the links on a command line arrived, `argv[0]` excluded: a command
/// line, unless the link helper said an Apple Event, and the opener it
/// passed, if any ([`apple_event_launch_args`]). A malformed opener is
/// dropped; the links still route.
pub fn link_origin_from_args<I>(args: I) -> (Source, Option<OpenerInfo>)
where
    I: IntoIterator<Item = OsString>,
{
    let mut source = Source::CommandLine;
    let mut opener = None;
    for arg in args
        .into_iter()
        .skip(1)
        .filter_map(|a| a.into_string().ok())
    {
        if arg.strip_prefix(SOURCE_FLAG) == Some(APPLE_EVENT) {
            source = Source::AppleEvent;
        } else if let Some(json) = arg.strip_prefix(OPENER_FLAG) {
            opener = serde_json::from_str(json).ok();
        }
    }
    (source, opener)
}

/// The links in a command line, `argv[0]` excluded.
///
/// Windows starts PwrRoute as `"pwrroute.exe" "%1"` and Linux as
/// `pwrroute %u`, so a link is a plain argument. Flags (`-x`, `--x`) are the
/// caller's, and anything that is not an absolute URL is skipped: a
/// single-letter scheme is a Windows drive (`C:\…`), not a link.
pub fn links_from_args<I>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = OsString>,
{
    args.into_iter()
        .skip(1)
        .filter_map(|arg| arg.into_string().ok())
        .filter(|arg| !arg.starts_with('-') && is_link(arg))
        .collect()
}

fn is_link(arg: &str) -> bool {
    let Some((scheme, rest)) = arg.split_once(':') else {
        return false;
    };
    scheme.len() > 1
        && !rest.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[String]) -> Vec<OsString> {
        std::iter::once("pwrroute".into())
            .chain(args.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn an_apple_event_launch_round_trips() {
        let opener = OpenerInfo {
            name: "Mail".into(),
            bundle_id: "com.apple.mail".into(),
            path: "/System/Applications/Mail.app".into(),
            window_title: None,
        };
        let links = vec![
            Link::new("https://example.com/a?b=c", Source::AppleEvent)
                .with_opener(Some(opener.clone())),
            Link::new("https://example.org/--opener=x", Source::AppleEvent)
                .with_opener(Some(opener.clone())),
        ];
        let args = apple_event_launch_args(&links);
        assert_eq!(
            args[..2],
            [
                "https://example.com/a?b=c",
                "https://example.org/--opener=x"
            ]
        );
        let argv = os(&args);
        assert_eq!(
            links_from_args(argv.clone()),
            [
                "https://example.com/a?b=c",
                "https://example.org/--opener=x"
            ],
            "the flags are not links"
        );
        assert_eq!(
            link_origin_from_args(argv),
            (Source::AppleEvent, Some(opener))
        );
    }

    #[test]
    fn the_window_title_stays_off_the_command_line() {
        let opener = OpenerInfo {
            name: "Mail".into(),
            bundle_id: "com.apple.mail".into(),
            path: "/System/Applications/Mail.app".into(),
            window_title: Some("A private subject".into()),
        };
        let links =
            [Link::new("https://example.com/", Source::AppleEvent).with_opener(Some(opener))];
        let args = apple_event_launch_args(&links);
        assert!(
            args.iter().all(|a| !a.contains("private subject")),
            "{args:?}"
        );
        let (_, read) = link_origin_from_args(os(&args));
        assert_eq!(
            read.map(|o| (o.bundle_id, o.window_title)),
            Some(("com.apple.mail".into(), None))
        );
    }

    #[test]
    fn an_event_without_a_sender_has_no_opener() {
        let args =
            apple_event_launch_args(&[Link::new("https://example.com/", Source::AppleEvent)]);
        assert_eq!(args, ["https://example.com/", "--source=apple-event"]);
        assert_eq!(link_origin_from_args(os(&args)), (Source::AppleEvent, None));
    }

    #[test]
    fn a_plain_command_line_is_a_command_line() {
        let plain = os(&["https://example.com/".to_owned(), "--dry-run".to_owned()]);
        assert_eq!(link_origin_from_args(plain), (Source::CommandLine, None));
        let bad = os(&[
            "--opener={not json".to_owned(),
            "--source=elsewhere".to_owned(),
        ]);
        assert_eq!(link_origin_from_args(bad), (Source::CommandLine, None));
    }

    fn links(args: &[&str]) -> Vec<String> {
        links_from_args(args.iter().map(OsString::from))
    }

    #[test]
    fn links_are_picked_out_of_argv() {
        assert_eq!(
            links(&[
                "pwrroute",
                "--dry-run",
                "https://example.com/a",
                "http://example.org",
                "mailto:someone@example.com",
            ]),
            [
                "https://example.com/a",
                "http://example.org",
                "mailto:someone@example.com"
            ]
        );
    }

    #[test]
    fn the_program_flags_paths_and_drives_are_not_links() {
        assert!(
            links(&[
                "https://argv0.example.com",
                "-psn_0_12345",
                "--config=https://x.example.com",
                r"C:\Users\me\file.html",
                "/home/me/file.html",
                "relative/path",
                "1http://bad-scheme",
                "https:",
            ])
            .is_empty()
        );
    }
}
