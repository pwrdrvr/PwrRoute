//! The main window's toast: a card in the lower right, over the pane,
//! after PwrSnap's update banner. An eyebrow, a sentence, and then one of
//! a progress track (work still running: no countdown), a countdown strip
//! along the top edge (an answer that leaves on its own) or nothing (an
//! offer that stays until it is taken or put away), and the actions.
//!
//! Updates are its only producer today (`update_notice`). Under Reduce
//! Motion the sweep stands still; the countdown is a functional delay and
//! still ends the answer, its strip just does not drain.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, BoxShadow, ClickEvent, Div, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, hsla, point, relative,
};

use crate::theme::{self, px, tracked};
use crate::update_notice::{self, ANSWER_FOR, Toast};
use crate::widgets::{self, ButtonKind};

/// The card's id, for tests and E2E.
pub const TOAST_ID: &str = "toast";
/// Its buttons' ids.
pub const RESTART_ID: &str = "toast-restart";
pub const LATER_ID: &str = "toast-later";
pub const NOTES_ID: &str = "toast-notes";
pub const CLOSE_ID: &str = "toast-close";

/// The card's width in design px; its sentence wraps inside.
const WIDTH: f32 = 340.0;

/// How the card reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Error,
}

/// What runs under the sentence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    /// Work of unknown length: a sweep.
    Indeterminate,
    /// 0-100.
    Percent(u8),
}

/// A toast's content, apart from where it comes from.
pub struct Card {
    pub eyebrow: SharedString,
    pub message: SharedString,
    pub tone: Tone,
    pub track: Option<Track>,
    /// The countdown strip: a name for this answer, so it drains once, and
    /// how long it runs.
    pub countdown: Option<(SharedString, Duration)>,
    pub actions: Vec<AnyElement>,
}

/// The card, anchored in the window's lower right by the caller.
pub fn card(card: Card, reduce_motion: bool) -> gpui_kit::Stateful<Div> {
    let (eyebrow, edge) = match card.tone {
        Tone::Accent => (theme::accent_bright(), theme::accent_border()),
        Tone::Error => (theme::danger_text(), theme::status_error()),
    };
    let label = format!("{}: {}", card.eyebrow, card.message);
    crate::accessibility::exposed(div(), gpui_kit::Role::Status, &label)
        .id(TOAST_ID)
        .debug_selector(|| TOAST_ID.into())
        .occlude()
        .relative()
        .overflow_hidden()
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .bg(theme::bg_elevated())
        .border_1()
        .border_color(edge)
        .rounded(px(10.0))
        .shadow(vec![BoxShadow {
            color: hsla(0.0, 0.0, 0.0, 0.35),
            offset: point(px(0.0), px(8.0)),
            blur_radius: px(24.0),
            spread_radius: px(0.0),
            inset: false,
        }])
        .when_some(card.countdown, |frame, (name, length)| {
            frame.child(countdown(name, length, edge, reduce_motion))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(tracked(&card.eyebrow, 10.0, 700.0, 0.06, eyebrow))
                .child(sentence(card.message, theme::text_primary()))
                .when_some(card.track, |content, track| {
                    content.child(progress(track, reduce_motion))
                }),
        )
        .when(!card.actions.is_empty(), |frame| {
            frame.child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .children(card.actions),
            )
        })
}

/// Running text that wraps: `theme::text` is one line.
fn sentence(content: SharedString, color: Hsla) -> Div {
    div()
        .font_family(theme::SANS)
        .font_weight(gpui_kit::FontWeight(500.0))
        .text_size(px(12.0))
        .line_height(px(17.0))
        .text_color(color)
        .child(content)
}

/// `.app-update-banner__track`: 4px, the accent over a muted groove.
fn progress(track: Track, reduce_motion: bool) -> AnyElement {
    let groove = div()
        .relative()
        .overflow_hidden()
        .h(px(4.0))
        .mt(px(5.0))
        .rounded_full()
        .bg(theme::bg_hover_strong());
    let bar = || div().absolute().top_0().bottom_0().rounded_full().bg(theme::accent());
    match track {
        Track::Percent(percent) => groove
            .child(bar().left_0().w(relative(f32::from(percent.min(100)) / 100.0)))
            .into_any_element(),
        Track::Indeterminate if reduce_motion => groove
            .child(bar().left_0().right_0().opacity(0.5))
            .into_any_element(),
        Track::Indeterminate => groove
            .child(bar().w(relative(0.35)).with_animation(
                "toast-sweep",
                Animation::new(Duration::from_millis(1100)).repeat(),
                // From just off the left edge to just off the right.
                |bar, t| bar.left(relative(-0.35 + 1.35 * t)),
            ))
            .into_any_element(),
    }
}

/// `.app-update-banner__timer`: 2px along the top edge, draining to the
/// left over `length`.
fn countdown(name: SharedString, length: Duration, color: Hsla, reduce_motion: bool) -> AnyElement {
    let strip = div()
        .absolute()
        .top_0()
        .left_0()
        .h(px(2.0))
        .bg(color);
    if reduce_motion {
        return strip.w_full().into_any_element();
    }
    strip
        .with_animation(name, Animation::new(length), |strip, t| {
            strip.w(relative(1.0 - t))
        })
        .into_any_element()
}

/// A small button that runs `action` on click.
fn action(
    id: &'static str,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    action: impl Fn(&mut App) + 'static,
) -> AnyElement {
    widgets::button(label, kind, true)
        .id(id)
        .debug_selector(|| id.into())
        .on_click(move |_: &ClickEvent, _, cx| action(cx))
        .into_any_element()
}

/// The update toast for the main window, when there is one.
pub fn update_toast(cx: &App) -> Option<AnyElement> {
    let toast = update_notice::toast(cx)?;
    let reduce_motion = cx.reduce_motion();
    let content = match toast {
        Toast::Checking => Card {
            eyebrow: "Checking for updates".into(),
            message: "Asking GitHub for the latest release\u{2026}".into(),
            tone: Tone::Accent,
            track: Some(Track::Indeterminate),
            countdown: None,
            actions: Vec::new(),
        },
        Toast::Downloading {
            version,
            switch_back,
            percent,
        } => Card {
            eyebrow: if switch_back {
                "Downloading switch"
            } else {
                "Downloading update"
            }
            .into(),
            message: match percent {
                Some(percent) => format!("PwrRoute {version} \u{00b7} {percent}%"),
                None => format!("PwrRoute {version}"),
            }
            .into(),
            tone: Tone::Accent,
            track: Some(percent.map_or(Track::Indeterminate, Track::Percent)),
            countdown: None,
            actions: Vec::new(),
        },
        Toast::Answer { answer, generation } => Card {
            eyebrow: answer.title.into(),
            message: answer.message.into(),
            tone: if answer.error {
                Tone::Error
            } else {
                Tone::Accent
            },
            track: None,
            countdown: Some((format!("toast-countdown-{generation}").into(), ANSWER_FOR)),
            actions: vec![action(
                CLOSE_ID,
                "Close",
                ButtonKind::Quiet,
                update_notice::dismiss,
            )],
        },
        Toast::Ready { ready, restarting } => {
            let mut actions = vec![
                if restarting {
                    widgets::button("Restarting\u{2026}", ButtonKind::Primary, true)
                        .id(RESTART_ID)
                        .debug_selector(|| RESTART_ID.into())
                        .opacity(theme::DISABLED)
                        .into_any_element()
                } else {
                    action(
                        RESTART_ID,
                        if ready.switch_back {
                            "Restart to switch"
                        } else {
                            "Restart to update"
                        },
                        ButtonKind::Primary,
                        update_notice::restart,
                    )
                },
                action(LATER_ID, "Later", ButtonKind::Quiet, update_notice::dismiss),
            ];
            if let Some(url) = update_notice::release_notes_url(&ready.version) {
                actions.push(action(
                    NOTES_ID,
                    "Release notes",
                    ButtonKind::Quiet,
                    move |cx| crate::help::open_link(&url, cx),
                ));
            }
            let verb = if ready.switch_back { "switch" } else { "update" };
            Card {
                eyebrow: update_notice::ready_title(&ready).into(),
                message: format!(
                    "{} is downloaded. Restart to {verb}; routing pauses for about two seconds.",
                    ready.version
                )
                .into(),
                tone: Tone::Accent,
                track: None,
                countdown: None,
                actions,
            }
        }
    };
    Some(card(content, reduce_motion).into_any_element())
}
