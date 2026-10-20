//! The middle pane of the Rules screen: the heading, the rewrites and
//! handlers cards, the catch-all row and the file footer. The read-only rows
//! a Finicky config shows (`config_view`) are drawn here too.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div, px, svg,
};
use pwrroute_engine::discovery;

use crate::app::{PwrRoute, Selection};
use crate::config_view::display_path;
use crate::rules_model::{Destination, Matcher, RowBadge, Rule};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{
    ButtonKind, Tone, action_tag, add_button, app_tag, badge, button, chip, fn_tag, one_line,
    paragraph, regex_tag, tag, toned_chip,
};

/// Widths of a row's fixed columns. The design lays rows out as the grid
/// `12px 22px minmax(0,1fr) 14px minmax(0,230px) 64px 20px`; at this pane
/// width the second minmax track resolves to its 230px maximum, so fixed
/// flex columns give the same tracks. A read-only row (`.rr.ro`) drops the
/// grip and hits and caps the destination at 200px.
mod column {
    pub const GRIP: f32 = 12.0;
    pub const INDEX: f32 = 22.0;
    pub const ARROW: f32 = 14.0;
    pub const DESTINATION: f32 = 230.0;
    pub const READ_ONLY_DESTINATION: f32 = 200.0;
    pub const HITS: f32 = 64.0;
    pub const MORE: f32 = 20.0;
}

fn matcher(matcher: &Matcher) -> Div {
    match matcher {
        Matcher::Pattern(pattern) => tag(pattern.clone()),
        Matcher::Regex(pattern) => regex_tag(pattern.clone()),
        Matcher::Label(label) => text(*label, 11.0, 500.0, theme::text_muted()),
        Matcher::App { badge, name } => app_tag(badge.clone(), name.clone()),
        Matcher::Function(source) => function(source),
        Matcher::Off => chip("off"),
    }
}

/// `.tag.fn` and its `.srcline`, which takes the rest of the cell: a JS
/// function by its source, on one line.
fn function(source: &SharedString) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(5.0))
        .child(fn_tag())
        .child(
            one_line(
                source.clone(),
                theme::mono_family(400.0),
                400.0,
                10.5,
                13.65,
                theme::text_muted(),
            )
            .flex_1(),
        )
}

pub(crate) fn destination(destination: &Destination) -> Div {
    match destination {
        Destination::Rewrite(changes) => changes.iter().fold(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.0))
                .min_w_0(),
            |row, change| row.child(action_tag(change.kind, change.value.clone())),
        ),
        Destination::Browser {
            badge: badge_label,
            name,
            profile,
        } => div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .min_w_0()
            .child(badge(badge_label.clone()))
            .child(text(name.clone(), 12.0, 500.0, theme::text_primary()))
            .when_some(profile.clone(), |row, profile| {
                row.child(mono(profile, 10.5, 500.0, theme::text_muted()))
            }),
        Destination::Function(source) => div()
            .flex()
            .items_center()
            .min_w_0()
            .child(function(source)),
        // `.dest .ph`
        Destination::Unset(placeholder) => div().flex().items_center().min_w_0().child(text(
            *placeholder,
            12.0,
            500.0,
            theme::text_subtle(),
        )),
    }
}

/// How a row reads besides its content.
#[derive(Clone, Copy, Default)]
pub(crate) struct RowState {
    pub selected: bool,
    pub last: bool,
    /// `.rr.ro`: a Finicky config's row, which cannot be moved or edited.
    pub read_only: bool,
}

/// `.rr`: grip, index, matchers, arrow, destination, hits, menu.
///
/// `matchers` replaces the rule's own match column when given, for the
/// catch-all row whose match is a label rather than patterns.
pub(crate) fn rule_row(
    id: impl Into<ElementId>,
    index: &str,
    rule: &Rule,
    state: RowState,
    matchers: Option<Div>,
) -> Stateful<Div> {
    // `.rr.off > * { opacity: .5 }`
    let dim = |cell: Div| {
        if rule.enabled {
            cell
        } else {
            cell.opacity(0.5)
        }
    };
    let not_saved = rule.badge == RowBadge::NotSaved;
    let grip = !rule.matchers.is_empty();
    let matchers = matchers.unwrap_or_else(|| {
        rule.matchers.iter().fold(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.0)),
            |cell, item| cell.child(matcher(item)),
        )
    });
    let destination_width = if state.read_only {
        column::READ_ONLY_DESTINATION
    } else {
        column::DESTINATION
    };
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(9.0))
        .pl(px(10.0))
        .pr(px(12.0))
        .when(!state.last, |row| {
            row.border_b_1().border_color(theme::border_faint())
        })
        .map(|row| {
            if state.selected {
                // `.rr.sel` and its `::before` bar.
                row.bg(theme::bg_row_active()).child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.0))
                        .bottom(px(6.0))
                        .w(px(3.0))
                        .rounded_r(px(2.0))
                        .bg(theme::accent()),
                )
            } else if state.read_only {
                row
            } else {
                row.hover(|style| style.bg(theme::bg_hover()))
            }
        })
        .when(!state.read_only, |row| {
            row.child(dim(div()
                .w(px(column::GRIP))
                .flex_none()
                .flex()
                .justify_center()
                .when(grip, |cell| {
                    cell.child(
                        svg()
                            .path("pwrroute/grip.svg")
                            .w(px(7.0))
                            .h(px(12.0))
                            .text_color(theme::text_subtle()),
                    )
                })))
        })
        .child(dim(div().w(px(column::INDEX)).flex_none().child(mono(
            index.to_owned(),
            10.5,
            600.0,
            // `.rr.bad .idx`
            if not_saved {
                theme::danger_text()
            } else {
                theme::text_subtle()
            },
        ))))
        .child(dim(matchers))
        .child(dim(div().w(px(column::ARROW)).flex_none().child(mono(
            "\u{2192}",
            12.0,
            500.0,
            theme::text_subtle(),
        ))))
        .child(dim(div()
            .w(px(destination_width))
            .flex_none()
            .min_w_0()
            .child(destination(&rule.destination))))
        .when(!state.read_only, |row| {
            // `.hits`, or `.badge` for a rule whose edit is not saved.
            let cell = div().w(px(column::HITS)).flex_none().flex().justify_end();
            row.child(dim(if not_saved {
                cell.items_center()
                    .gap(px(5.0))
                    .child(theme::dot(theme::status_error()))
                    .child(text("not saved", 10.0, 600.0, theme::danger_text()))
            } else {
                cell.child(mono(rule.hits.clone(), 10.5, 500.0, theme::text_subtle()))
            }))
        })
        .child(dim(div()
            .w(px(column::MORE))
            .flex_none()
            .flex()
            .justify_center()
            // The catch-all of a read-only list has no menu.
            .when(!(state.read_only && rule.matchers.is_empty()), |cell| {
                cell.child(mono("\u{22ef}", 12.0, 600.0, theme::text_subtle()))
            })))
}

/// `.card`.
pub(crate) fn card() -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(10.0))
        .bg(theme::bg_panel())
        .overflow_hidden()
}

/// `.card-hd-main`: eyebrow and title.
pub(crate) fn card_title(eyebrow: &str, title: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::text_muted()))
        .child(
            text(title, 13.5, 600.0, theme::text_primary())
                .mt(px(6.0))
                .line_height(px(16.2)),
        )
}

/// `.card-hd`: eyebrow, title and whatever sits on the right.
pub(crate) fn card_header_with(eyebrow: &str, title: &'static str, trailing: Div) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .py(px(13.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(card_title(eyebrow, title))
        .child(trailing)
}

/// A `.chip`'s tone beyond the ones `widgets` draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipTone {
    /// `.chip.acc`: a draft.
    Accent,
    /// `.chip.bad`: not saved.
    Bad,
}

/// `.chip` in one of [`ChipTone`]'s colors.
pub(crate) fn chip_in(label: impl Into<SharedString>, tone: ChipTone) -> Div {
    let (background, foreground): (Hsla, Hsla) = match tone {
        ChipTone::Accent => (theme::accent_soft(), theme::accent_bright()),
        ChipTone::Bad => (theme::danger_soft(), theme::danger_text()),
    };
    chip(label)
        .bg(background)
        .border_color(theme::transparent())
        .text_color(foreground)
}

/// The count chip of a card: "7 handlers · 1 off · 1 draft", or in red
/// when an edit is not saved.
fn count_chip(rules: &[&Rule], noun: &str) -> Div {
    let off = rules.iter().filter(|r| !r.enabled).count();
    let drafts = rules.iter().filter(|r| r.badge == RowBadge::Draft).count();
    let not_saved = rules
        .iter()
        .filter(|r| r.badge == RowBadge::NotSaved)
        .count();
    if not_saved > 0 {
        return chip_in(format!("{not_saved} not saved"), ChipTone::Bad);
    }
    let mut parts = vec![format!("{} {noun}", rules.len())];
    if off > 0 {
        parts.push(format!("{off} off"));
    }
    if drafts > 0 {
        parts.push(format!(
            "{drafts} {}",
            if drafts == 1 { "draft" } else { "drafts" }
        ));
    }
    toned_chip(parts.join(" \u{00b7} "), Tone::Plain)
}

/// `.pane-head-main`: eyebrow, title and optional help.
pub(crate) fn pane_heading(eyebrow: &str, title: &'static str, help: Option<&'static str>) -> Div {
    pane_heading_with(eyebrow, title, help.map(String::from))
}

fn pane_heading_with(eyebrow: &str, title: &'static str, help: Option<String>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::accent()))
        .child(
            text(title, 19.0, 700.0, theme::text_primary())
                .mt(px(6.0))
                .line_height(px(22.8)),
        )
        .when_some(help, |heading, help| {
            heading.child(
                div()
                    .mt(px(6.0))
                    // 56ch: Geist's "0" is about 0.6em, 6.9px at 11.5px.
                    .max_w(px(386.0))
                    .font_family(theme::SANS)
                    .font_weight(theme::weight(400.0))
                    .text_size(px(11.5))
                    .line_height(px(18.4))
                    .text_color(theme::text_muted())
                    .child(help),
            )
        })
}

/// `.pane-head`: what this screen is, and the add buttons. With no rules
/// the help says where every link goes instead.
fn heading(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Div {
    let help = if app.store.is_empty() {
        let (_, name) = app.store.default_browser();
        let profile = app.store.everything_else();
        let target = match profile.destination {
            Destination::Browser {
                profile: Some(profile),
                ..
            } => format!("{name} \u{00b7} {profile}"),
            _ => name.to_string(),
        };
        format!("Every link opens in {target} until a rule says otherwise.")
    } else {
        "Rewrites run in order and every match applies. Then the first matching handler \
         opens the link. Anything left goes to the default."
            .into()
    };
    div()
        .flex()
        .items_start()
        .gap(px(16.0))
        .child(pane_heading_with("Routing", "Rules", Some(help)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    add_button("Add rewrite", false)
                        .id("add-rewrite")
                        .debug_selector(|| "add-rewrite".into())
                        .on_click(cx.listener(|this, _, window, cx| this.add_rewrite(window, cx))),
                )
                .child(
                    add_button("Add handler", true)
                        .id("add-handler")
                        .debug_selector(|| "add-handler".into())
                        .on_click(cx.listener(|this, _, window, cx| this.add_handler(window, cx))),
                ),
        )
}

/// `.empty`: what a card is for, with an example in the row's own
/// vocabulary, and the way to add the first one.
fn empty_card(title: &'static str, what: &'static str, example: Div, add: Stateful<Div>) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(14.0))
        .pt(px(16.0))
        .px(px(16.0))
        .pb(px(18.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(text(title, 12.5, 600.0, theme::text_primary()).line_height(px(16.25)))
                .child(paragraph(what, 11.5, 17.8, theme::text_muted())
                        // 56ch at 11.5px.
                        .max_w(px(386.0)))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.0))
                        .child(text("for example", 10.5, 500.0, theme::text_subtle()))
                        .child(example),
                ),
        )
        .child(add)
}

fn arrow() -> Div {
    mono("\u{2192}", 12.0, 500.0, theme::text_subtle())
}

/// `.pane`: the screen's scrolling column.
pub(crate) fn pane_frame(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(18.0))
        .py(px(22.0))
        .px(px(24.0))
        .overflow_y_scroll()
}

/// The rewrites and handlers cards, which select a row for the editor.
pub(crate) fn visual_cards(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Div {
    let rewrite_rules: Vec<&Rule> = app.rewrites.iter().map(|r| &r.rule).collect();
    let rewrites = card().child(card_header_with(
        "Step 1 \u{00b7} run in order \u{00b7} every match applies",
        "Rewrites",
        count_chip(&rewrite_rules, "rewrites"),
    ));
    let rewrites = if app.rewrites.is_empty() {
        rewrites.child(empty_card(
            "No rewrites",
            "A rewrite changes a link before any handler sees it: a privacy front end, https \
             for an internal host, tracking parameters gone.",
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(tag("x.com/*"))
                .child(arrow())
                .child(action_tag("host", "xcancel.com")),
            button("Add rewrite", ButtonKind::Default, true)
                .id("empty-add-rewrite")
                .debug_selector(|| "empty-add-rewrite".into())
                .on_click(cx.listener(|this, _, window, cx| this.add_rewrite(window, cx))),
        ))
    } else {
        app.rewrites
            .iter()
            .enumerate()
            .fold(rewrites, |card, (index, rewrite)| {
                let state = RowState {
                    selected: app.selection == Some(Selection::Rewrite(index)),
                    last: index + 1 == app.rewrites.len(),
                    read_only: false,
                };
                card.child(
                    rule_row(
                        ElementId::named_usize("rewrite", index),
                        &format!("{:02}", index + 1),
                        &rewrite.rule,
                        state,
                        None,
                    )
                    .debug_selector(move || format!("rewrite-{index}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.select(Selection::Rewrite(index), cx);
                    })),
                )
            })
    };

    let handler_rules: Vec<&Rule> = app.handlers.iter().map(|h| &h.rule).collect();
    let handlers = card().child(card_header_with(
        "Step 2 \u{00b7} first match wins",
        "Handlers",
        count_chip(&handler_rules, "handlers"),
    ));
    let handlers = if app.handlers.is_empty() {
        handlers.child(empty_card(
            "No handlers",
            "A handler sends matching links to a browser, a profile or an app. The first one \
             that matches wins, so specific rules go above general ones.",
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(tag("*.atlassian.net/*"))
                .child(arrow())
                .child(badge("Ch"))
                .child(text(
                    "Google Chrome \u{00b7} Work",
                    10.5,
                    500.0,
                    theme::text_secondary(),
                )),
            button("Add handler", ButtonKind::Primary, true)
                .id("empty-add-handler")
                .debug_selector(|| "empty-add-handler".into())
                .on_click(cx.listener(|this, _, window, cx| this.add_handler(window, cx))),
        ))
    } else {
        app.handlers
            .iter()
            .enumerate()
            .fold(handlers, |card, (index, handler)| {
                let state = RowState {
                    selected: app.selection == Some(Selection::Handler(index)),
                    last: index + 1 == app.handlers.len(),
                    read_only: false,
                };
                card.child(
                    rule_row(
                        ElementId::named_usize("handler", index),
                        &format!("{:02}", index + 1),
                        &handler.rule,
                        state,
                        None,
                    )
                    .debug_selector(move || format!("handler-{index}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus, cx);
                        this.select(Selection::Handler(index), cx);
                    })),
                )
            })
    };
    // `.card-foot.bad`: which rule the file keeps an older version of.
    let not_saved: Vec<String> = app
        .rewrites
        .iter()
        .enumerate()
        .filter(|(_, r)| r.rule.badge == RowBadge::NotSaved)
        .map(|(i, _)| format!("Rewrite {}", i + 1))
        .chain(
            app.handlers
                .iter()
                .enumerate()
                .filter(|(_, h)| h.rule.badge == RowBadge::NotSaved)
                .map(|(i, _)| format!("Handler {}", i + 1)),
        )
        .collect();
    let handlers = handlers.when(!not_saved.is_empty(), |card| {
        let who = not_saved.join(", ");
        let verb = if not_saved.len() == 1 { "is" } else { "are" };
        card.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(9.0))
                .px(px(12.0))
                .bg(theme::danger_soft())
                .child(paragraph(
                    format!(
                        "{who} {verb} not saved. The router uses the saved version until it is \
                         fixed."
                    ),
                    10.5,
                    14.7,
                    theme::danger_text(),
                )),
        )
    });

    div()
        .flex()
        .flex_col()
        .gap(px(18.0))
        .child(rewrites)
        .child(handlers)
}

/// `.blank`: nothing to route by. No `rules.json` and no Finicky config,
/// the state after Later on first run: three ways out, and where a config
/// was looked for.
fn nothing_to_route_by(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Div {
    let (_, fallback) = app.store.default_browser();
    let choice = |primary: bool, title: &'static str, what: &'static str, action: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .py(px(11.0))
            .px(px(14.0))
            .border_1()
            .rounded(px(8.0))
            .map(|choice| {
                if primary {
                    choice
                        .border_color(theme::accent_border())
                        .bg(theme::bg_row_active())
                } else {
                    choice
                        .border_color(theme::border_subtle())
                        .bg(theme::bg_panel())
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(text(title, 12.5, 600.0, theme::text_primary()).line_height(px(16.25)))
                    .child(paragraph(what, 11.0, 16.0, theme::text_muted())),
            )
            .child(action)
    };
    let env = discovery::Environment {
        home: app.home.clone(),
        ..discovery::Environment::current()
    };
    let looked = discovery::config_candidates(&env).into_iter().fold(
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .py(px(10.0))
            .px(px(12.0))
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(8.0))
            .bg(theme::bg_panel())
            .child(text(
                "Looked for a Finicky config in",
                10.5,
                500.0,
                theme::text_muted(),
            )),
        |list, path| {
            list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(mono(
                        display_path(&path, app.home.as_deref()),
                        10.5,
                        500.0,
                        theme::text_muted(),
                    ))
                    .child(div().ml_auto().child(mono(
                        "not found",
                        10.5,
                        500.0,
                        theme::text_subtle(),
                    ))),
            )
        },
    );
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .py(px(40.0))
        .px(px(24.0))
        .child(
            div()
                .w(px(520.0))
                .flex()
                .flex_col()
                .gap(px(14.0))
                .child(tracked("Rules", 9.5, 600.0, 0.14, theme::accent()))
                .child(
                    text(
                        "Nothing to route by yet",
                        21.0,
                        700.0,
                        theme::text_primary(),
                    )
                    .line_height(px(25.2)),
                )
                .child(paragraph(
                    format!(
                        "There is no rules.json and no Finicky config, so any link that reaches \
                         PwrRoute opens in {fallback}."
                    ),
                    11.5,
                    18.4,
                    theme::text_muted(),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        // Choosing a browser and a config file wait for the
                        // browser picker and the file dialog.
                        .child(choice(
                            true,
                            "Choose where links go",
                            "Creates rules.json with a default browser. Add rules afterwards, \
                             or never.",
                            button("Choose browser\u{2026}", ButtonKind::Primary, true),
                        ))
                        .child(choice(
                            false,
                            "Use a Finicky config",
                            "Any .js or .ts in Finicky v4 shape. It runs as written.",
                            button("Choose file\u{2026}", ButtonKind::Default, true),
                        ))
                        .child(choice(
                            false,
                            "Start with a handler",
                            "The default stays Safari until you change it.",
                            div().child(
                                button("Add handler", ButtonKind::Default, true)
                                    .id("blank-add-handler")
                                    .debug_selector(|| "blank-add-handler".into())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_handler(window, cx);
                                    })),
                            ),
                        )),
                )
                .child(looked),
        )
}

/// The Rules screen's pane. Scrolls: at the default window height the
/// handlers card runs past the bottom, which the design clips.
pub fn pane(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    if app.nothing_to_route_by() {
        return pane_frame("rules-pane")
            .debug_selector(|| "rules-blank".into())
            .child(nothing_to_route_by(app, cx));
    }
    let everything_else = card().child(rule_row(
        "everything-else",
        "\u{221e}",
        &app.store.everything_else(),
        RowState {
            last: true,
            ..RowState::default()
        },
        Some(div().flex_1().min_w_0().child(text(
            "Everything else",
            11.0,
            600.0,
            theme::text_primary(),
        ))),
    ));

    let (dot, status) = match app.store.status() {
        crate::rules_store::Status::Failed(_) => (theme::status_error(), app.store.status_label()),
        _ => (theme::status_ok(), "saved on change".to_owned()),
    };
    pane_frame("rules-pane")
        .child(heading(app, cx))
        .child(visual_cards(app, cx))
        .child(everything_else)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(theme::dot(dot))
                .child(one_line(
                    format!(
                        "rules.json \u{00b7} {} \u{00b7} {status}",
                        app.store.location(app.home.as_deref())
                    ),
                    theme::mono_family(500.0),
                    500.0,
                    10.5,
                    10.5,
                    theme::text_subtle(),
                )),
        )
}
