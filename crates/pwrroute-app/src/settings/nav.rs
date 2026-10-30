//! The Settings nav (Settings Sections 1a, after PwrAgent's Settings): group
//! labels, and under Config Files and Cloud Sync a caret and a child row per
//! config file or cloud folder, each with a status dot and an OFF or NOT
//! USED chip. A child opens its pane and brings its row into view
//! ([`SettingsWindow::target`], marked by [`SettingsWindow::mark_target`]).

use gpui_kit::base::{Button, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
};

use super::model::{ConfigFileState, SettingsModel, SyncTone};
use super::{Pane, SettingsWindow};
use crate::theme::{self, mono, px, text, tracked};

/// The red dot on Cloud Sync's nav item while a folder is in conflict.
pub const SYNC_DOT_ID: &str = "settings-nav-sync-dot";

/// The caret's gutter, so every label lines up whether or not it has one.
const GUTTER: f32 = 18.0;

/// What a child row's dot says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Ok,
    Warn,
    Bad,
    Off,
}

/// One row under Config Files or Cloud Sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    /// The row it brings into view: [`config_key`] or [`folder_key`].
    pub key: SharedString,
    pub label: SharedString,
    pub dot: Dot,
    /// "OFF", "NOT USED".
    pub chip: Option<&'static str>,
}

/// The target key of a config file's row in Config Files.
pub fn config_key(path: &std::path::Path) -> SharedString {
    format!("config:{}", path.display()).into()
}

/// The target key of `rules.json`'s row in Config Files.
pub const RULES_KEY: &str = "config:rules.json";

/// The target key of a cloud folder's row in Cloud Sync.
pub fn folder_key(root: &std::path::Path) -> SharedString {
    format!("folder:{}", root.display()).into()
}

/// The `index`th row under `pane`'s nav item.
pub fn child_id(pane: Pane, index: usize) -> ElementId {
    ElementId::Name(format!("{}-child-{index}", pane.nav_id()).into())
}

/// The caret that folds `pane`'s rows.
pub fn caret_id(pane: Pane) -> ElementId {
    ElementId::Name(format!("{}-caret", pane.nav_id()).into())
}

/// The rows under `pane`: none for a pane without children.
pub fn children(model: &SettingsModel, pane: Pane) -> Vec<Child> {
    match pane {
        Pane::Configuration => {
            let configuration = &model.configuration;
            let mut rows: Vec<Child> = configuration
                .config_files
                .iter()
                .map(|file| Child {
                    key: config_key(&file.path),
                    label: file.shown.clone(),
                    dot: match file.state {
                        ConfigFileState::InCharge => Dot::Ok,
                        ConfigFileState::Failed => Dot::Warn,
                        ConfigFileState::NotUsed => Dot::Off,
                    },
                    chip: (file.state == ConfigFileState::NotUsed).then_some("NOT USED"),
                })
                .collect();
            rows.push(Child {
                key: RULES_KEY.into(),
                label: "rules.json".into(),
                dot: match configuration.rules_loaded {
                    Some(true) => Dot::Ok,
                    Some(false) => Dot::Warn,
                    None => Dot::Off,
                },
                chip: None,
            });
            rows
        }
        Pane::Sync => {
            let sync = &model.sync;
            let folders = sync.folders.iter().map(|folder| Child {
                key: folder_key(&folder.root),
                label: folder.label.clone(),
                dot: if folder.conflict.is_some() {
                    Dot::Bad
                } else {
                    match folder.tone {
                        SyncTone::Ok => Dot::Ok,
                        SyncTone::Warn => Dot::Warn,
                        SyncTone::Bad => Dot::Bad,
                        SyncTone::Neutral => Dot::Off,
                    }
                },
                chip: None,
            });
            let offered = sync.offered.iter().map(|place| Child {
                key: folder_key(&place.root),
                label: place.label.clone(),
                dot: Dot::Off,
                chip: Some("OFF"),
            });
            folders.chain(offered).collect()
        }
        _ => Vec::new(),
    }
}

/// `.snav`: every pane, its group label and its children.
pub fn draw(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let agents = this.model.local_agents.active_sessions();
    let conflict = this.model.sync.has_conflict();
    Pane::ALL.into_iter().fold(
        div()
            .id("settings-nav")
            .w(px(super::NAV_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .pt(px(10.0))
            .px(px(10.0))
            .pb(px(12.0))
            .overflow_y_scroll()
            .bg(theme::bg_sidebar())
            .border_r_1()
            .border_color(theme::border_subtle()),
        |nav, pane| {
            let count = (pane == Pane::LocalAgents && agents > 0).then(|| agents.to_string());
            let children = children(&this.model, pane);
            let caret = (!children.is_empty()).then(|| !this.folded.contains(&pane));
            nav.when_some(pane.group(), |nav, group| {
                nav.child(
                    div()
                        .mt(px(12.0))
                        .mb(px(4.0))
                        .px(px(10.0))
                        .child(tracked(group, 9.5, 700.0, 0.08, theme::text_muted())),
                )
            })
            // Synced Rules 1c: a conflict marks Cloud Sync wherever the
            // reader is in Settings, folded or not.
            .child(item(pane, this.pane == pane, count, caret, pane == Pane::Sync && conflict, cx))
            .when(caret == Some(true), |nav| {
                nav.child(
                    children
                        .into_iter()
                        .enumerate()
                        .fold(div().flex().flex_col().gap(px(2.0)).mt(px(2.0)).mb(px(4.0)), {
                            |list, (index, row)| {
                                let on = this.pane == pane
                                    && this.target.as_ref() == Some(&row.key);
                                list.child(child(pane, index, row, on, cx))
                            }
                        }),
                )
            })
        },
    )
}

/// A pane's row: `.nrow`; `.on` when `selected`. `caret` is whether its
/// group is open, for a pane with children.
fn item(
    pane: Pane,
    selected: bool,
    count: Option<String>,
    caret: Option<bool>,
    dot: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> Button {
    let foreground = if selected {
        theme::accent_bright()
    } else {
        theme::text_secondary()
    };
    // The 1px edge is the focus ring's, out of the design's 8/10 padding.
    crate::widgets::focus_visible(Button::new(pane.nav_id()), false)
        .selected(selected)
        .accessibility_label(pane.label())
        .relative()
        .w_full()
        .justify_start()
        .gap(px(4.0))
        .pr(px(9.0))
        .py(px(7.0))
        .border_1()
        .border_color(theme::transparent())
        .rounded(px(7.0))
        .cursor_pointer()
        .map(|item| {
            if selected {
                // `.rail-item.on::before`: the 3px bar on the rail's edge.
                item.bg(theme::bg_row_active()).child(
                    div()
                        .absolute()
                        .left(px(-10.0))
                        .top(px(7.0))
                        .bottom(px(7.0))
                        .w(px(3.0))
                        .rounded_r(px(2.0))
                        .bg(theme::accent()),
                )
            } else {
                item.hover(|style| style.bg(theme::bg_hover()))
            }
        })
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, _, cx| this.show(pane, cx)))
        .child(gutter(pane, caret, cx))
        .child(text(pane.label(), 13.0, 500.0, foreground))
        .when_some(count, |item, count| {
            item.child(
                div()
                    .ml_auto()
                    .child(mono(count, 10.5, 600.0, theme::text_subtle())),
            )
        })
        .when(dot, |item| {
            // `.rail-item .xdot`
            item.child(
                div()
                    .id(SYNC_DOT_ID)
                    .debug_selector(|| SYNC_DOT_ID.into())
                    .ml_auto()
                    .size(px(6.0))
                    .rounded_full()
                    .bg(theme::status_error())
                    .test_support(),
            )
        })
}

/// The caret's gutter: the directory caret for a pane with children, a
/// click on it folding the group without leaving the pane in view.
fn gutter(
    pane: Pane,
    caret: Option<bool>,
    cx: &mut Context<'_, SettingsWindow>,
) -> gpui_kit::AnyElement {
    let gutter = div().w(px(GUTTER)).flex_none().flex().justify_center();
    let Some(open) = caret else {
        return gutter.into_any_element();
    };
    gutter
        .id(caret_id(pane))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            if !this.folded.remove(&pane) {
                this.folded.insert(pane);
            }
            cx.notify();
        }))
        .child(text(
            if open { "\u{25be}" } else { "\u{25b8}" },
            10.0,
            500.0,
            theme::text_muted(),
        ))
        .test_support()
        .into_any_element()
}

/// `.nsub`: indented on the guide rail, with its dot, label and chip.
fn child(
    pane: Pane,
    index: usize,
    row: Child,
    on: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let id = child_id(pane, index);
    let foreground = if on {
        theme::accent_bright()
    } else {
        theme::text_muted()
    };
    let key = row.key.clone();
    crate::widgets::focus_visible(Button::new(id), false)
        .selected(on)
        .accessibility_label(match row.chip {
            Some(chip) => SharedString::from(format!("{}, {}", row.label, chip.to_lowercase())),
            None => row.label.clone(),
        })
        .ml(px(26.0))
        .flex()
        .items_center()
        .justify_start()
        .gap(px(8.0))
        .py(px(6.0))
        .pl(px(10.0))
        .pr(px(8.0))
        .border_l_1()
        .border_color(if on {
            theme::accent()
        } else {
            theme::border_subtle()
        })
        .rounded_r(px(6.0))
        .cursor_pointer()
        .when(!on, |row| row.hover(|style| style.bg(theme::bg_hover())))
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.bring_into_view(pane, key.clone(), window, cx);
        }))
        .child(dot(row.dot))
        .child(div().flex_1().min_w_0().child(crate::widgets::one_line(
            row.label,
            theme::SANS,
            500.0,
            12.0,
            16.0,
            foreground,
        )))
        .when_some(row.chip, |row, chip| {
            row.child(
                div()
                    .flex_none()
                    .px(px(5.0))
                    .py(px(2.0))
                    .border_1()
                    .border_color(theme::border_subtle())
                    .rounded_full()
                    .child(mono(chip, 9.0, 500.0, theme::text_subtle())),
            )
        })
}

fn dot(dot: Dot) -> Div {
    theme::dot(match dot {
        Dot::Ok => theme::status_ok(),
        Dot::Warn => theme::status_warning(),
        Dot::Bad => theme::status_error(),
        Dot::Off => theme::border_strong(),
    })
}

impl SettingsWindow {
    /// Open `pane` and bring the row `key` names into view, marked.
    pub(crate) fn bring_into_view(
        &mut self,
        pane: Pane,
        key: SharedString,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.show(pane, cx);
        self.target = Some(key);
        cx.notify();
        // The row takes the anchor when this frame draws; the anchor scrolls
        // on the frame after (it waits a frame itself), then a redraw shows
        // the pane there.
        let anchor = self.anchor.clone();
        window.on_next_frame(move |window, cx| {
            anchor.scroll_to(window, cx);
            window.on_next_frame(|window, _| window.refresh());
        });
    }

    /// `row` as the target row when `key` is the one a nav child chose:
    /// the anchor the pane scrolls to, and the accent rail (`.dst.hl`).
    /// `row` takes `key` as its id, which the anchor needs.
    pub(crate) fn mark_target(&self, key: &SharedString, row: Div) -> Stateful<Div> {
        let row = row.id(ElementId::Name(key.clone()));
        if self.target.as_ref() != Some(key) {
            return row;
        }
        row.relative()
            .anchor_scroll(Some(self.anchor.clone()))
            .bg(theme::bg_row_active())
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(px(3.0))
                    .bg(theme::accent()),
            )
    }
}
