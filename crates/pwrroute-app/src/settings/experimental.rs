//! Optional features, reached directly from the Settings sidebar.

use gpui_kit::{AnyElement, Context, Div, IntoElement, ParentElement as _, Styled as _, div};

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, button_when, chip, field, field_help, field_sub, heading, pane_head, switch,
};
use super::model::{LinkHelperCard, LinkHelperState, SettingsRequest};

pub const YIELD_FOCUS_ID: &str = "experimental-yield-browser-focus";

/// The link helper row's one button: Use the link helper, or Stop using.
pub const LINK_HELPER_ID: &str = "experimental-link-helper";

/// What the link helper is for. New copy: not in the design yet.
const LINK_HELPER_SUB: &str = "macOS brings the default browser\u{2019}s app forward for \
     every link. Hand links to a windowless helper so PwrRoute\u{2019}s window stays where it \
     is. macOS asks you to confirm the change.";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let pane = div().child(pane_head(
        "Experimental",
        "Experimental features",
        "Features that may change shape or be removed without notice.",
        None,
    ));
    let experimental = &this.model.experimental;
    if experimental.yield_browser_focus.is_none() && experimental.link_helper.is_none() {
        return pane.child(field_help(
            "No experimental features are available on this platform.",
        ));
    }
    let mut fields = Vec::new();
    if let Some(enabled) = experimental.yield_browser_focus {
        fields.push(field(
            "Yield focus to browsers",
            Some(field_sub(
                "Let a running browser bring its destination window forward. Off by default; applies to the next link.",
            )),
            [switch(YIELD_FOCUS_ID, enabled)
                .accessibility_label("Yield focus to browsers")
                .on_change({
                    let set = cx.listener(|this, enabled: &bool, window, cx| {
                        this.model.experimental.yield_browser_focus = Some(*enabled);
                        this.request(SettingsRequest::SetYieldBrowserFocus(*enabled), window, cx);
                        cx.notify();
                    });
                    move |enabled, _, window, cx| set(&enabled, window, cx)
                })
                .into_any_element()],
        ));
    }
    if let Some(helper) = &experimental.link_helper {
        fields.push(field(
            "Windowless link helper",
            Some(field_sub(LINK_HELPER_SUB)),
            link_helper_controls(helper, cx),
        ));
    }
    pane.child(this.disclosure(
        "experimental-browser-focus",
        heading("Experimental", "Browser focus"),
        None,
        fields,
        cx,
    ))
}

/// The row's chip, its button and the line under them. The button asks
/// for the helper when it is off and for the app when it is on; it waits
/// while macOS has the question, and is inert in a build without a helper.
fn link_helper_controls(
    helper: &LinkHelperCard,
    cx: &mut Context<'_, SettingsWindow>,
) -> Vec<AnyElement> {
    let (status, label, use_it, help) = match &helper.state {
        LinkHelperState::On => (
            chip("On", Tone::Ok),
            "Stop using",
            Some(false),
            "Links go to the helper, which hands each one to PwrRoute without bringing it \
             forward.",
        ),
        LinkHelperState::Off => (
            chip("Off", Tone::Neutral),
            "Use the link helper",
            Some(true),
            "Links go to PwrRoute itself.",
        ),
        LinkHelperState::Unavailable(why) => (
            chip("Unavailable", Tone::Neutral),
            "Use the link helper",
            None,
            why.as_ref(),
        ),
    };
    let enabled = use_it.is_some() && !helper.waiting;
    let mut button = button_when(LINK_HELPER_ID, label, Kind::Default, false, enabled);
    if let Some(on) = use_it.filter(|_| enabled) {
        button = button.on_click(cx.listener(move |this, _, window, cx| {
            this.request(SettingsRequest::UseLinkHelper(on), window, cx);
        }));
    }
    let help = helper
        .note
        .as_ref()
        .map_or_else(|| help.to_owned(), ToString::to_string);
    vec![
        div()
            .flex()
            .items_center()
            .gap(crate::theme::px(8.0))
            .child(status)
            .child(button)
            .into_any_element(),
        field_help(help).into_any_element(),
    ]
}
