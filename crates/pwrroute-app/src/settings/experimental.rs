//! Optional features, reached directly from the Settings sidebar.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, div};

use super::SettingsWindow;
use super::controls::{field, field_help, field_sub, heading, pane_head, switch};
use super::model::SettingsRequest;

pub const YIELD_FOCUS_ID: &str = "experimental-yield-browser-focus";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let pane = div().child(pane_head(
        "Experimental",
        "Experimental features",
        "Features that may change shape or be removed without notice.",
        None,
    ));
    let Some(enabled) = this.model.experimental.yield_browser_focus else {
        return pane.child(field_help(
            "No experimental features are available on this platform.",
        ));
    };
    pane.child(this.disclosure(
        "experimental-browser-focus",
        heading("Experimental", "Browser focus"),
        None,
        vec![field(
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
        )],
        cx,
    ))
}
