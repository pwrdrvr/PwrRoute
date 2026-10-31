//! Settings › General (design: Appearance and Themes 1a, 1b): the
//! Appearance card, with Theme (System, Dark or Light) and the OS's
//! Increase Contrast as PwrRoute follows it. General came back for this;
//! it sits after About, outside the groups, as PwrAgent's and PwrGit's do.

use gpui_kit::base::{Radio, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window,
    div,
};

use super::SettingsWindow;
use super::controls::{Tone, chip, field, field_help, field_sub, heading, pane_head};
use super::model::SettingsRequest;
use crate::theme::{self, ThemeChoice, px, text};
use crate::widgets::segmented;

/// The Appearance card.
pub const APPEARANCE_ID: &str = "general-appearance";

/// The Theme control.
pub const THEME_ID: &str = "general-theme";

/// The line under Theme: what System resolves to, or what is locked.
pub const THEME_HELP_ID: &str = "general-theme-help";

/// One answer of the Theme control.
pub fn theme_id(choice: ThemeChoice) -> &'static str {
    match choice {
        ThemeChoice::System => "general-theme-system",
        ThemeChoice::Dark => "general-theme-dark",
        ThemeChoice::Light => "general-theme-light",
    }
}

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let general = &this.model.general;
    let theme_control = ThemeChoice::ALL
        .into_iter()
        .fold(segmented().id(THEME_ID), |control, choice| {
            control.child(theme_segment(choice, general.theme, cx))
        })
        .test_support();
    let theme_help = div()
        .id(THEME_HELP_ID)
        .child(field_help(general.theme_help()))
        .test_support();
    let contrast = if general.increase_contrast {
        "On"
    } else {
        "Off"
    };
    let rows = vec![
        field(
            "Theme",
            Some(field_sub(if cfg!(target_os = "macos") {
                "System follows macOS: PwrRoute turns light or dark when the Mac does, while it runs."
            } else {
                "System follows your desktop: PwrRoute turns light or dark when it does, while it runs."
            })),
            [
                theme_control.into_any_element(),
                theme_help.into_any_element(),
            ],
        ),
        field(
            "Increase Contrast",
            Some(field_sub(if cfg!(target_os = "macos") {
                "macOS\u{2019}s setting, in Accessibility \u{203a} Display."
            } else if cfg!(target_os = "windows") {
                "Windows\u{2019} contrast themes, in Accessibility."
            } else {
                "Your desktop\u{2019}s high contrast setting."
            })),
            [
                chip(
                    format!("{contrast} \u{00b7} follows {}", general.os),
                    Tone::Neutral,
                )
                .into_any_element(),
                field_help("When it is on, edges and quiet text get stronger, in both schemes.")
                    .into_any_element(),
            ],
        ),
    ];
    div()
        .child(pane_head(
            "General",
            "General",
            "How PwrRoute looks, in every window at once.",
            None,
        ))
        .child(this.disclosure(
            APPEARANCE_ID,
            heading("Appearance", "Theme"),
            Some(chip(general.chip(), Tone::Neutral).into_any_element()),
            rows,
            cx,
        ))
}

/// `.seg span`, on gpui-kit's `Radio`: one answer of Theme. The window
/// flips at once; the producer saves, applies and reports back.
fn theme_segment(
    choice: ThemeChoice,
    current: ThemeChoice,
    cx: &mut Context<'_, SettingsWindow>,
) -> Radio {
    let selected = choice == current;
    let set = cx.listener(move |this, _: &bool, window, cx| this.set_theme(choice, window, cx));
    Radio::new(theme_id(choice))
        .checked(selected)
        .accessibility_label(choice.label())
        .on_change(move |on, _, window, cx| set(&on, window, cx))
        .flex_none()
        .py(px(5.0))
        .px(px(10.0))
        .rounded(px(5.0))
        .map(|segment| {
            if selected {
                segment.bg(theme::bg_row_active())
            } else {
                segment
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::bg_hover()))
            }
        })
        .child(text(
            choice.label(),
            11.0,
            550.0,
            if selected {
                theme::accent_bright()
            } else {
                theme::text_secondary()
            },
        ))
}

impl SettingsWindow {
    fn set_theme(&mut self, choice: ThemeChoice, window: &mut Window, cx: &mut Context<'_, Self>) {
        let general = &mut self.model.general;
        if general.theme == choice {
            return;
        }
        general.theme = choice;
        if choice != ThemeChoice::System {
            general.showing = choice.resolve(general.showing);
        }
        self.request(SettingsRequest::SetTheme(choice), window, cx);
        cx.notify();
    }
}
