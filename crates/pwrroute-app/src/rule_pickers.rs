//! What the rule editor's pickers offer (design: Rule Editor Pickers 3a to
//! 4c): the browsers the scan found with their profiles, the apps the
//! rules name that the scan did not find, and the apps rules require links
//! to come from. Plain data over the scan (`PwrRoute::browsers` and
//! `PwrRoute::installed`) and the edited handlers; `editor.rs` draws it.
//! A value the scan cannot match is never rewritten here: the editor shows
//! it as written, with a warning, until the person picks something else.

use crate::activity_actions::BrowserChoice;
use crate::overview_model::Installed;
use crate::rules_model::Handler;

/// The app the scan found that `value` names: its name or bundle id,
/// ignoring case, as the launcher matches them.
pub fn installed_named<'a>(value: &str, installed: &'a [Installed]) -> Option<&'a Installed> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    installed.iter().find(|app| {
        app.name.eq_ignore_ascii_case(value)
            || app
                .bundle_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(value))
    })
}

/// Whether the scan has nothing for `value` once it ran: the rule names an
/// app that is not on this machine.
pub fn is_missing(
    value: &str,
    browsers: &[BrowserChoice],
    installed: &[Installed],
    scanned: bool,
) -> bool {
    let value = value.trim();
    scanned
        && !value.is_empty()
        && !browsers.iter().any(|b| b.name.eq_ignore_ascii_case(value))
        && installed_named(value, installed).is_none()
}

/// The apps the rules require links to come from, each once, less the
/// ones handler `except` already names: what "+ app" offers.
pub fn named_openers(handlers: &[Handler], except: usize) -> Vec<String> {
    let own = handlers
        .get(except)
        .map(|h| h.openers.as_slice())
        .unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for opener in handlers.iter().flat_map(|h| h.openers.iter()) {
        let opener = opener.trim();
        if opener.is_empty()
            || own.iter().any(|o| o.trim().eq_ignore_ascii_case(opener))
            || names.iter().any(|n| n.eq_ignore_ascii_case(opener))
        {
            continue;
        }
        names.push(opener.to_owned());
    }
    names
}

/// How the private-window switch reads for a browser value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivateMode {
    /// "an Incognito window", "an InPrivate window", "a private window".
    Window(String),
    /// The scan's browser can't open one from a link (Safari, Arc).
    Unsupported,
    /// No browser yet, or one the catalog does not know: the plain words.
    Unknown,
}

/// What the switch says for `value`: the browser's own word when the
/// catalog has one; "can't" only for a browser the scan found.
pub fn private_mode(value: &str, detected: bool) -> PrivateMode {
    match crate::overview_model::private_term(value) {
        Some(term) => PrivateMode::Window(crate::overview_model::window_phrase(&term)),
        None if detected => PrivateMode::Unsupported,
        None => PrivateMode::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(browser: &str, openers: &[&str]) -> Handler {
        let mut rule = crate::fixtures::rules_file().rules.remove(0);
        rule.browser = browser.into();
        rule.opener = openers.iter().map(|o| (*o).to_owned()).collect();
        crate::rules_model::handler(&rule, crate::rules_model::RowBadge::None, Vec::new())
    }

    #[test]
    fn plus_app_offers_the_apps_other_rules_name() {
        let handlers = [
            handler("Firefox", &["Example Chat"]),
            handler("Firefox", &["example chat", "com.example.mail"]),
            handler("Firefox", &["com.example.mail"]),
        ];
        assert_eq!(named_openers(&handlers, 2), ["Example Chat"]);
        assert_eq!(
            named_openers(&handlers, 9),
            ["Example Chat", "com.example.mail"]
        );
    }

    #[test]
    fn the_private_switch_uses_the_browsers_word() {
        assert_eq!(
            private_mode("Microsoft Edge", true),
            PrivateMode::Window("an InPrivate window".into())
        );
        assert_eq!(
            private_mode("Google Chrome", true),
            PrivateMode::Window("an Incognito window".into())
        );
        assert_eq!(private_mode("Safari", true), PrivateMode::Unsupported);
        assert_eq!(private_mode("Example Viewer", false), PrivateMode::Unknown);
    }
}
