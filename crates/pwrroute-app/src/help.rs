//! Help: the menu's rows and what each does. Links to the project go
//! through PwrRoute's own router, as a click anywhere would, so they open
//! in the browser the rules pick; Changelog, the license, the notices and
//! the log open in PwrRoute's own windows (`doc_window`, `logs_window`) and
//! work offline.

use gpui_kit::{App, ClipboardItem};
use pwrroute_router::{Link, Source};

use crate::{doc_window, logs_window, mcp, settings, settings_store};

pub use help_actions::*;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod help_actions {
    use gpui_kit::actions;

    actions!(
        pwrroute,
        [
            /// The README on GitHub: PwrRoute's documentation for now.
            OpenDocumentation,
            /// CHANGELOG.md in its own window.
            OpenChangelog,
            /// The first-run window again.
            ReplayOnboarding,
            /// GitHub's new-issue chooser.
            ReportIssue,
            /// GitHub's private vulnerability report.
            ReportVulnerability,
            /// Diagnostics' lines on the clipboard.
            CopyDiagnostics,
            /// The diagnostic log in its own window.
            OpenLogs,
            /// The repository on GitHub.
            ViewSource,
            /// LICENSE in its own window.
            ViewLicense,
            /// THIRD_PARTY_LICENSES in its own window.
            ThirdPartyNotices,
        ]
    );
}

/// The repository, from the workspace's `repository`.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Where each link row goes.
pub fn documentation_url() -> String {
    format!("{REPOSITORY}#readme")
}

pub fn issue_url() -> String {
    format!("{REPOSITORY}/issues/new/choose")
}

/// SECURITY.md asks for GitHub's private reporting first.
pub fn vulnerability_url() -> String {
    format!("{REPOSITORY}/security/advisories/new")
}

pub fn install(cx: &mut App) {
    cx.on_action(|_: &OpenDocumentation, cx| open_link(&documentation_url(), cx));
    cx.on_action(|_: &ReportIssue, cx| open_link(&issue_url(), cx));
    cx.on_action(|_: &ReportVulnerability, cx| open_link(&vulnerability_url(), cx));
    cx.on_action(|_: &ViewSource, cx| open_link(REPOSITORY, cx));
    // The windows wait for the dispatch to finish: chosen from inside the
    // window it would bring forward (the strip off macOS), GPUI still holds
    // it, so it could be neither read nor raised and a second would open.
    let doc = |doc: doc_window::Doc, cx: &mut App| cx.defer(move |cx| doc_window::open(doc, cx));
    cx.on_action(move |_: &OpenChangelog, cx| doc(doc_window::Doc::Changelog, cx));
    cx.on_action(move |_: &ViewLicense, cx| doc(doc_window::Doc::License, cx));
    cx.on_action(move |_: &ThirdPartyNotices, cx| {
        doc(doc_window::Doc::ThirdPartyNotices, cx);
    });
    cx.on_action(|_: &OpenLogs, cx| cx.defer(logs_window::open));
    cx.on_action(|_: &CopyDiagnostics, cx| copy_diagnostics(cx));
    cx.on_action(|_: &ReplayOnboarding, cx| cx.defer(crate::onboarding::open));
}

/// Opens one of PwrRoute's own pages: through the router, so the rules
/// pick the browser as for any click, else (no router in this process)
/// through the OS.
pub fn open_link(url: &str, cx: &mut App) {
    if let Some(router) = settings_store::router(cx)
        && router.dispatch(Link::new(url, Source::Api)).is_ok()
    {
        return;
    }
    cx.open_url(url);
}

/// Settings › Diagnostics' lines: version, OS, folders, the router and
/// the log. Paths and states only, never a link.
pub fn diagnostics_report(cx: &mut App) -> String {
    let agents = mcp::local_agents(cx);
    settings::system_host::diagnostics(cx, &agents).report()
}

/// [`diagnostics_report`] on the clipboard, as Diagnostics' Copy does.
pub fn copy_diagnostics(cx: &mut App) {
    let report = diagnostics_report(cx);
    cx.write_to_clipboard(ClipboardItem::new_string(report));
    log::info!(target: "pwrroute", "help: copied diagnostics info");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_point_at_the_repository() {
        assert_eq!(REPOSITORY, "https://github.com/pwrdrvr/PwrRoute");
        assert_eq!(
            documentation_url(),
            "https://github.com/pwrdrvr/PwrRoute#readme"
        );
        assert_eq!(
            issue_url(),
            "https://github.com/pwrdrvr/PwrRoute/issues/new/choose"
        );
        assert_eq!(
            vulnerability_url(),
            "https://github.com/pwrdrvr/PwrRoute/security/advisories/new"
        );
    }
}
