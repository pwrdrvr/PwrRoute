//! Links on the command line.

use std::ffi::OsString;

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
