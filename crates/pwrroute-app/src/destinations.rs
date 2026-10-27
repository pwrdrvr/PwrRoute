//! Open in as one control (design: Destination and App Pickers 1a to 1d):
//! a lane per browser the scan found, its profiles as pills, the
//! browser's own default first and its private window last in its own
//! word. One pill is on across every lane, and a click is the whole
//! choice: browser, profile and private window together. The Add a rule
//! sheet and the rule editor draw the same lanes.
//!
//! [`lanes`] is plain data over the scan and the rule's current values;
//! [`draw`] draws it in any view, at a [`Density`] (the tray's card is
//! `Compact`). A value the scan cannot match is never rewritten:
//! the rule's browser gets a lane of its own, first, drawn as missing once
//! the scan ran.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, px, svg,
};

use crate::activity_actions::BrowserChoice;
use crate::overview_model::{Installed, private_term};
use crate::theme::{self, text};
use crate::widgets::{AppBadge, badge, badge_for};

/// What a pill writes to the rule.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    /// `rules[].browser`: an app name, bundle id or path.
    pub browser: String,
    /// `rules[].profile`; empty is the browser's own default.
    pub profile: String,
    /// `rules[].private`.
    pub private: bool,
}

/// What kind of pill it is, which sets how it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillKind {
    /// The browser's own default profile.
    Default,
    Profile,
    /// The browser's private window, by its word ("InPrivate").
    Private,
}

/// One pill of a lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub label: String,
    pub kind: PillKind,
    pub choice: Choice,
    pub on: bool,
}

/// One browser and its pills.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane {
    /// The browser as the reader knows it.
    pub name: String,
    /// What a pill writes as the rule's browser.
    pub value: String,
    pub badge: AppBadge,
    pub pills: Vec<Pill>,
    /// "where it opened", "4 profiles", "not installed".
    pub tag: Option<String>,
    /// The rule names this browser and the scan did not find it.
    pub missing: bool,
    /// One of its pills is on.
    pub on: bool,
}

/// The lanes for `current`: `first` (the browser a link opened in, or
/// the rule's own) leads, with `first_tag` beside it; then the scan's
/// browsers in its order. `browsers` is the scan less the hidden apps,
/// `installed` the whole scan, `scanned` whether it ran.
pub fn lanes(
    browsers: &[BrowserChoice],
    installed: &[Installed],
    current: &Choice,
    first: Option<(&str, &str)>,
    scanned: bool,
) -> Vec<Lane> {
    let value = current.browser.trim();
    // The scan's name for the rule's browser, which may be written as a
    // bundle id or in another case.
    let current_name = (!value.is_empty()).then(|| {
        browsers
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(value))
            .map(|b| b.name.clone())
            .or_else(|| {
                crate::rule_pickers::installed_named(value, installed).map(|app| app.name.clone())
            })
    });
    let mut out: Vec<Lane> = Vec::new();
    // The rule's browser, when the scan does not list it: first, as
    // written, never rewritten.
    if let Some(None) = current_name {
        out.push(lane_for(
            value,
            value,
            &profile_list(current.profile.trim()),
            current,
            Some(
                if scanned {
                    "not installed"
                } else {
                    "named in this rule"
                }
                .to_owned(),
            ),
            scanned,
        ));
    }
    let mut order: Vec<&BrowserChoice> = browsers.iter().collect();
    if let Some((name, _)) = first
        && let Some(at) = order.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    {
        let lead = order.remove(at);
        order.insert(0, lead);
    }
    for browser in order {
        let tag = match first {
            Some((name, tag)) if browser.name.eq_ignore_ascii_case(name) && !tag.is_empty() => {
                Some(tag.to_owned())
            }
            _ if private_term(&browser.name).is_none() => {
                Some("no private window from a link".to_owned())
            }
            _ => None,
        };
        let directories = crate::rule_pickers::installed_named(&browser.name, installed)
            .map(|app| app.directories.clone())
            .unwrap_or_default();
        let profiles: Vec<(String, Option<String>)> = browser
            .profiles
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), directories.get(i).cloned()))
            .collect();
        let selected = current_name
            .as_ref()
            .and_then(|n| n.as_ref())
            .is_some_and(|n| n.eq_ignore_ascii_case(&browser.name));
        let mut lane = lane_for(
            &browser.name,
            &browser.name,
            &profiles,
            &Choice {
                // Only the rule's own browser lights a pill.
                browser: if selected {
                    browser.name.clone()
                } else {
                    String::new()
                },
                ..current.clone()
            },
            tag,
            false,
        );
        // A rule that names the browser another way keeps its value.
        if selected {
            for pill in &mut lane.pills {
                if pill.on {
                    pill.choice.browser = browser.name.clone();
                }
            }
        }
        out.push(lane);
    }
    out
}

/// The rule's profile is one the scan read for its browser by neither
/// name nor folder. A browser whose profiles were not read is no answer.
pub fn unknown_profile(
    browsers: &[BrowserChoice],
    installed: &[Installed],
    current: &Choice,
) -> bool {
    let value = current.browser.trim();
    let wanted = current.profile.trim();
    if value.is_empty() || wanted.is_empty() {
        return false;
    }
    let name = browsers
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(value))
        .map(|b| b.name.as_str())
        .or_else(|| {
            crate::rule_pickers::installed_named(value, installed).map(|a| a.name.as_str())
        });
    let Some(browser) = name.and_then(|n| browsers.iter().find(|b| b.name == n)) else {
        return false;
    };
    if browser.profiles.is_empty() {
        return false;
    }
    let directories = crate::rule_pickers::installed_named(&browser.name, installed)
        .map(|app| app.directories.as_slice())
        .unwrap_or_default();
    !browser
        .profiles
        .iter()
        .any(|p| p.eq_ignore_ascii_case(wanted))
        && !directories.iter().any(|d| d.eq_ignore_ascii_case(wanted))
}

/// A rule's profile, as the only one a lane knows of.
fn profile_list(profile: &str) -> Vec<(String, Option<String>)> {
    if profile.is_empty() {
        Vec::new()
    } else {
        vec![(profile.to_owned(), None)]
    }
}

/// One lane: the default, each profile (with its folder, which a rule
/// may name instead), and the private window when the catalog has one.
fn lane_for(
    name: &str,
    value: &str,
    profiles: &[(String, Option<String>)],
    current: &Choice,
    tag: Option<String>,
    missing: bool,
) -> Lane {
    let selected = !current.browser.trim().is_empty();
    let wanted = current.profile.trim();
    let is = |profile: &str, directory: Option<&String>| {
        profile.eq_ignore_ascii_case(wanted)
            || directory.is_some_and(|d| !d.is_empty() && d.eq_ignore_ascii_case(wanted))
    };
    let term = private_term(name).or_else(|| private_term(value));
    let private_on = selected && current.private && term.is_some();
    let choice = |profile: &str, private: bool| Choice {
        browser: value.to_owned(),
        profile: profile.to_owned(),
        private,
    };
    let mut pills = vec![Pill {
        label: "Default".to_owned(),
        kind: PillKind::Default,
        choice: choice("", false),
        on: selected && !private_on && wanted.is_empty(),
    }];
    pills.extend(profiles.iter().map(|(profile, directory)| Pill {
        label: profile.clone(),
        kind: PillKind::Profile,
        choice: choice(profile, false),
        on: selected && !private_on && is(profile, directory.as_ref()),
    }));
    // A profile the scan does not list (kept for another machine, or a
    // browser whose profiles can't be read) is still the rule's: it gets
    // a pill of its own, on, so the choice is never hidden.
    if selected
        && !private_on
        && !wanted.is_empty()
        && !profiles.iter().any(|(p, d)| is(p, d.as_ref()))
    {
        pills.push(Pill {
            label: wanted.to_owned(),
            kind: PillKind::Profile,
            choice: choice(wanted, false),
            on: true,
        });
    }
    if let Some(term) = term {
        // A rule that pairs a profile with private keeps both.
        let (label, profile) = if private_on && !wanted.is_empty() {
            (format!("{term} \u{00b7} {wanted}"), wanted.to_owned())
        } else {
            (term, String::new())
        };
        pills.push(Pill {
            label,
            kind: PillKind::Private,
            choice: choice(&profile, true),
            on: private_on,
        });
    }
    let on = pills.iter().any(|p| p.on);
    let shown = crate::default_browser::browser_name(name);
    Lane {
        badge: AppBadge::new(badge_for(&shown).letters().clone(), value.to_owned()),
        name: shown,
        value: value.to_owned(),
        pills,
        tag,
        missing,
        on,
    }
}

/// The private window's mask, as the Overview draws a private node.
const MASK: &str = "pwrroute/mask.svg";

/// How tightly the lanes are drawn: `Regular` in the window, `Compact`
/// in the tray's card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    Regular,
    #[allow(
        dead_code,
        reason = "for the tray's Send links card, which draws it next"
    )]
    Compact,
}

impl Density {
    /// Gap between lanes and between pills.
    fn gap(self) -> f32 {
        match self {
            Self::Regular => 6.0,
            Self::Compact => 4.0,
        }
    }

    /// A lane's padding: top, sides, bottom.
    fn padding(self) -> (f32, f32, f32) {
        match self {
            Self::Regular => (9.0, 10.0, 10.0),
            Self::Compact => (6.0, 8.0, 7.0),
        }
    }

    /// Gap between a lane's head and its pills.
    fn head_gap(self) -> f32 {
        match self {
            Self::Regular => 8.0,
            Self::Compact => 5.0,
        }
    }

    /// A pill's height and side padding.
    fn pill(self) -> (f32, f32) {
        match self {
            Self::Regular => (24.0, 10.0),
            Self::Compact => (22.0, 8.0),
        }
    }
}

/// The lanes, each pill a click that hands its [`Choice`] to `pick`, in
/// any view. Selectors are `{id}-lane-{lane}` and `{id}-{lane}-{pill}`.
pub fn draw<V: 'static>(
    id: &'static str,
    lanes: &[Lane],
    density: Density,
    pick: impl Fn(&mut V, Choice, &mut Window, &mut Context<'_, V>) + Clone + 'static,
    cx: &mut Context<'_, V>,
) -> Div {
    let (top, side, bottom) = density.padding();
    lanes.iter().enumerate().fold(
        div().w_full().flex().flex_col().gap(px(density.gap())),
        |list, (l, lane)| {
            let pills = lane.pills.iter().enumerate().fold(
                div().flex().flex_wrap().gap(px(density.gap().min(5.0))),
                |row, (p, pill)| {
                    let pick = pick.clone();
                    let choice = pill.choice.clone();
                    row.child(
                        pill_view(pill, density)
                            .id(ElementId::Name(format!("{id}-{l}-{p}").into()))
                            .debug_selector(move || format!("{id}-{l}-{p}"))
                            .cursor_pointer()
                            .when(!pill.on, |el| {
                                el.hover(|style| {
                                    style
                                        .bg(theme::bg_hover_strong())
                                        .text_color(theme::text_primary())
                                })
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                pick(this, choice.clone(), window, cx);
                            })),
                    )
                },
            );
            let head = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(badge(lane.badge.clone()))
                .child(
                    crate::widgets::one_line(
                        lane.name.clone(),
                        theme::SANS,
                        600.0,
                        12.0,
                        12.0,
                        theme::text_primary(),
                    )
                    .flex_1(),
                )
                .when_some(lane.tag.clone(), |head, tag| {
                    head.child(div().flex_none().child(text(
                        tag,
                        10.0,
                        500.0,
                        if lane.missing {
                            theme::warn_text()
                        } else {
                            theme::text_subtle()
                        },
                    )))
                });
            list.child(
                div()
                    .id(ElementId::Name(format!("{id}-lane-{l}").into()))
                    .debug_selector(move || format!("{id}-lane-{l}"))
                    .flex()
                    .flex_col()
                    .gap(px(density.head_gap()))
                    .pt(px(top))
                    .px(px(side))
                    .pb(px(bottom))
                    .border_1()
                    .rounded(px(9.0))
                    .bg(theme::bg_input())
                    .map(|el| {
                        if lane.on {
                            el.border_color(theme::accent_border())
                        } else if lane.missing {
                            el.border_dashed().border_color(theme::border_default())
                        } else {
                            el.border_color(theme::border_subtle())
                        }
                    })
                    .child(head)
                    .child(pills),
            )
        },
    )
}

/// `.pl`: a pill, on in the accent, the private one dashed with its mask.
fn pill_view(pill: &Pill, density: Density) -> Div {
    let (height, side) = density.pill();
    let color = if pill.on {
        theme::accent_on()
    } else if pill.kind == PillKind::Profile {
        theme::text_secondary()
    } else {
        theme::text_muted()
    };
    let label: SharedString = pill.label.clone().into();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .h(px(height))
        .px(px(side))
        .rounded_full()
        .border_1()
        .text_color(color)
        .map(|el| {
            if pill.on {
                el.bg(theme::accent()).border_color(theme::accent())
            } else if pill.kind == PillKind::Private {
                el.bg(theme::bg_panel())
                    .border_dashed()
                    .border_color(theme::border_default())
            } else {
                el.bg(theme::bg_panel())
                    .border_color(theme::border_subtle())
            }
        })
        .when(pill.kind == PillKind::Private, |el| {
            el.child(svg().path(MASK).size(px(12.0)).text_color(color))
        })
        .child(
            div()
                .font_family(theme::SANS)
                .font_weight(theme::weight(if pill.on { 600.0 } else { 500.0 }))
                .text_size(px(11.0))
                .line_height(px(11.0))
                .whitespace_nowrap()
                .child(label),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan() -> Vec<BrowserChoice> {
        vec![
            BrowserChoice {
                name: "Microsoft Edge".into(),
                profiles: vec!["Work".into(), "Personal".into()],
            },
            BrowserChoice {
                name: "Safari".into(),
                profiles: Vec::new(),
            },
        ]
    }

    fn on(lanes: &[Lane]) -> Vec<(String, String)> {
        lanes
            .iter()
            .flat_map(|lane| {
                lane.pills
                    .iter()
                    .filter(|p| p.on)
                    .map(|p| (lane.name.clone(), p.label.clone()))
            })
            .collect()
    }

    #[test]
    fn one_pill_is_the_whole_choice() {
        let none = lanes(&scan(), &[], &Choice::default(), None, true);
        assert!(on(&none).is_empty(), "nothing is chosen for the person");
        let edge = &none[0];
        assert_eq!(
            edge.pills
                .iter()
                .map(|p| p.label.as_str())
                .collect::<Vec<_>>(),
            ["Default", "Work", "Personal", "InPrivate"]
        );
        assert_eq!(
            edge.pills[3].choice,
            Choice {
                browser: "Microsoft Edge".into(),
                profile: String::new(),
                private: true,
            }
        );
        // Safari opens no private window from a link: no such pill.
        assert_eq!(none[1].pills.len(), 1);
        assert_eq!(
            none[1].tag.as_deref(),
            Some("no private window from a link")
        );

        let work = Choice {
            browser: "microsoft edge".into(),
            profile: "work".into(),
            private: false,
        };
        assert_eq!(
            on(&lanes(&scan(), &[], &work, None, true)),
            [("Microsoft Edge".into(), "Work".into())]
        );
        let private = Choice {
            private: true,
            profile: String::new(),
            ..work.clone()
        };
        assert_eq!(
            on(&lanes(&scan(), &[], &private, None, true)),
            [("Microsoft Edge".into(), "InPrivate".into())]
        );
        // A profile with private keeps both, on the private pill.
        let both = Choice {
            private: true,
            ..work
        };
        let drawn = lanes(&scan(), &[], &both, None, true);
        assert_eq!(
            on(&drawn),
            [("Microsoft Edge".into(), "InPrivate \u{00b7} work".into())]
        );
        assert_eq!(drawn[0].pills[3].choice.profile, "work");
    }

    #[test]
    fn a_profile_named_by_its_folder_or_a_bundle_id_finds_its_pill() {
        let installed = vec![Installed {
            name: "Microsoft Edge".into(),
            bundle_id: Some("com.microsoft.edgemac".into()),
            profiles: vec!["Work".into(), "Personal".into()],
            directories: vec!["Default".into(), "Profile 1".into()],
            hidden: false,
        }];
        let rule = Choice {
            browser: "com.microsoft.edgemac".into(),
            profile: "Profile 1".into(),
            private: false,
        };
        let drawn = lanes(&scan(), &installed, &rule, None, true);
        assert_eq!(
            drawn.len(),
            2,
            "the bundle id is the scan's Edge, not a lane of its own"
        );
        assert_eq!(on(&drawn), [("Microsoft Edge".into(), "Personal".into())]);
    }

    #[test]
    fn the_link_s_browser_leads_and_an_unknown_one_is_kept() {
        let first = lanes(
            &scan(),
            &[],
            &Choice::default(),
            Some(("Safari", "where it opened")),
            true,
        );
        assert_eq!(first[0].name, "Safari");
        assert_eq!(first[0].tag.as_deref(), Some("where it opened"));

        let gone = Choice {
            browser: "Vivaldi".into(),
            profile: "Work".into(),
            private: false,
        };
        let drawn = lanes(&scan(), &[], &gone, None, true);
        assert_eq!(drawn[0].name, "Vivaldi");
        assert!(drawn[0].missing);
        assert_eq!(drawn[0].tag.as_deref(), Some("not installed"));
        assert_eq!(on(&drawn), [("Vivaldi".into(), "Work".into())]);
        // Before the scan ran nothing is drawn as missing.
        assert!(!lanes(&scan(), &[], &gone, None, false)[0].missing);
    }

    #[test]
    fn a_profile_the_scan_lacks_keeps_a_pill_of_its_own() {
        let other = Choice {
            browser: "Microsoft Edge".into(),
            profile: "Lab".into(),
            private: false,
        };
        let drawn = lanes(&scan(), &[], &other, None, true);
        assert_eq!(on(&drawn), [("Microsoft Edge".into(), "Lab".into())]);
        assert_eq!(
            drawn[0].pills.len(),
            5,
            "Default, Work, Personal, Lab, InPrivate"
        );
        assert!(unknown_profile(&scan(), &[], &other));
        // A browser whose profiles were not read is no answer.
        let safari = Choice {
            browser: "Safari".into(),
            profile: "Lab".into(),
            private: false,
        };
        assert!(!unknown_profile(&scan(), &[], &safari));
        assert_eq!(
            on(&lanes(&scan(), &[], &safari, None, true)),
            [("Safari".into(), "Lab".into())]
        );
        let work = Choice {
            profile: "work".into(),
            ..other
        };
        assert!(!unknown_profile(&scan(), &[], &work));
    }
}
