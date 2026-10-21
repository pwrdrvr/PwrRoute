//! The middle pane of the Rules screen: the heading, the Run tests strip,
//! the rewrites and handlers cards with each rule's example chips, the
//! catch-all row, the saved tests and the file footer. The read-only rows a
//! Finicky config shows (`config_view`) are drawn here too.
//!
//! The chips and the strip read `PwrRoute::tests` (`try_it::TestRuns`),
//! which runs the file's examples and tests on every save and on Run tests.
//! A failure row names the input, what the file expected, what happened and
//! which rule won instead (Activity Privacy 3a).

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div, px, svg,
};
use pwrroute_engine::{Source, Stage, Step, TestFailure, TestSource, discovery};

use crate::app::{PwrRoute, Selection};
use crate::config_view::display_path;
use crate::rules_model::{Destination, Matcher, RowBadge, Rule};
use crate::rules_store::clock;
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{
    ButtonKind, Tone, action_tag, add_button, app_tag, badge, badge_for, button, chip, fn_tag,
    one_line, paragraph, regex_tag, tag, toned_chip,
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
        Matcher::Privacy(label) => chip(label.clone()),
    }
}

/// `.ex i`: the mark before an example: ✓ when it passed, × when it did
/// not, a dot while the tests have not run since the rules changed.
pub(crate) fn example_mark(passed: Option<bool>) -> Div {
    let (label, fill, color) = match passed {
        Some(true) => ("\u{2713}", theme::success_soft(), theme::success_text()),
        Some(false) => ("\u{00d7}", theme::danger_soft(), theme::danger_text()),
        None => ("\u{00b7}", theme::bg_elevated(), theme::text_subtle()),
    };
    div()
        .flex_none()
        .size(px(12.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(fill)
        .child(mono(label, 8.0, 700.0, color))
}

/// A winning step as the strip and the chips name it: "handler 2",
/// "config handler 1", "the default".
pub(crate) fn winner_label(step: &Step) -> String {
    match (step.stage, step.source) {
        (Stage::Default, _) => "the default".into(),
        (Stage::Handler, Source::Rules) => format!("handler {}", step.index + 1),
        (Stage::Handler, Source::Config) => format!("config handler {}", step.index + 1),
        (Stage::Rewrite, Source::Rules) => format!("rewrite {}", step.index + 1),
        (Stage::Rewrite, Source::Config) => format!("config rewrite {}", step.index + 1),
    }
}

/// `.ex`: one example chip under a rule's row.
fn example_chip(url: &str, passed: Option<bool>, went_to: Option<String>) -> Div {
    let failed = passed == Some(false);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .h(px(20.0))
        .pl(px(5.0))
        .pr(px(7.0))
        .max_w(px(420.0))
        .border_1()
        .border_color(if failed {
            theme::status_error()
        } else {
            theme::border_subtle()
        })
        .rounded(px(6.0))
        .bg(theme::bg_elevated())
        .child(example_mark(passed))
        .child(one_line(
            url.to_owned(),
            theme::mono_family(500.0),
            500.0,
            10.5,
            10.5,
            if failed {
                theme::danger_text()
            } else {
                theme::text_secondary()
            },
        ))
        .when_some(went_to, |chip, went_to| {
            chip.child(
                text(
                    format!("\u{2192} {went_to}"),
                    10.0,
                    500.0,
                    theme::danger_text(),
                )
                .whitespace_nowrap(),
            )
        })
}

/// `.exrow`: a rule's examples, under its row.
fn examples_row(app: &PwrRoute, selection: Selection, last: bool) -> Option<Stateful<Div>> {
    let chips: Vec<Div> = match selection {
        Selection::Handler(h) => app
            .handlers
            .get(h)?
            .examples
            .iter()
            .enumerate()
            .map(|(i, url)| {
                let source = TestSource::RuleExample {
                    rule: h,
                    example: i,
                };
                let went_to = app.tests.failure(source).map(|f| {
                    f.winner
                        .as_ref()
                        .map_or_else(|| f.actual.clone(), winner_label)
                });
                example_chip(url, app.tests.passed(source), went_to)
            })
            .collect(),
        Selection::Rewrite(r) => app
            .rewrites
            .get(r)?
            .examples
            .iter()
            .enumerate()
            .map(|(i, example)| {
                let source = TestSource::RewriteExample {
                    rewrite: r,
                    example: i,
                };
                let went_to = app.tests.failure(source).map(|f| f.actual.clone());
                example_chip(&example.url, app.tests.passed(source), went_to)
            })
            .collect(),
    };
    if chips.is_empty() {
        return None;
    }
    let (kind, index) = match selection {
        Selection::Handler(h) => ("handler", h),
        Selection::Rewrite(r) => ("rewrite", r),
    };
    Some(
        div()
            .id(ElementId::named_usize(kind, index + 10_000))
            .debug_selector(move || format!("{kind}-{index}-examples"))
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(5.0))
            .pl(px(54.0))
            .pr(px(12.0))
            .pb(px(8.0))
            .when(!last, |row| {
                row.border_b_1().border_color(theme::border_faint())
            })
            .child(tracked("Examples", 9.5, 600.0, 0.12, theme::text_subtle()).mr(px(3.0)))
            .children(chips),
    )
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
                let last = index + 1 == app.rewrites.len();
                let examples = examples_row(app, Selection::Rewrite(index), last);
                let state = RowState {
                    selected: app.selection == Some(Selection::Rewrite(index)),
                    // The examples row carries the border when there is one.
                    last: last || examples.is_some(),
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
                .children(examples)
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
                let last = index + 1 == app.handlers.len();
                let examples = examples_row(app, Selection::Handler(index), last);
                let state = RowState {
                    selected: app.selection == Some(Selection::Handler(index)),
                    last: last || examples.is_some(),
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
                .children(examples)
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

/// What a failure's input is an example or test of: "example of handler 3",
/// "test 2", "example of rewrite 1".
fn source_label(source: TestSource) -> String {
    match source {
        TestSource::RuleExample { rule, .. } => format!("example of handler {}", rule + 1),
        TestSource::RewriteExample { rewrite, .. } => format!("example of rewrite {}", rewrite + 1),
        TestSource::Test { test } => format!("test {}", test + 1),
    }
}

/// `.fail-row`: one failure of the last run: input, expected, actual and
/// the winner, with a way to the handler that took the link.
fn failure_row(index: usize, failure: &TestFailure, cx: &mut Context<'_, PwrRoute>) -> Div {
    let column = |title: &'static str, value: Div| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(tracked(title, 9.5, 600.0, 0.14, theme::text_muted()))
            .child(value)
    };
    let value = |content: String| {
        div().flex().flex_col().gap(px(4.0)).child(paragraph(
            content,
            11.5,
            16.1,
            theme::text_primary(),
        ))
    };
    let from = match &failure.opener {
        Some(app) => format!("from {app}"),
        None => "from any app".into(),
    };
    let input = value(failure.input.clone()).child(paragraph(
        format!("{} \u{00b7} {from}", source_label(failure.source)),
        10.5,
        13.65,
        theme::text_muted(),
    ));
    let winner = match &failure.winner {
        Some(step) => {
            let patterns: Vec<String> = step
                .matchers
                .iter()
                .map(|m| match m {
                    pwrroute_engine::Matcher::Wildcard(s)
                    | pwrroute_engine::Matcher::Regex(s)
                    | pwrroute_engine::Matcher::Other(s) => s.clone(),
                    pwrroute_engine::Matcher::Function(name) => format!("fn {name}"),
                })
                .collect();
            value(winner_label(step)).when(!patterns.is_empty(), |cell| {
                cell.child(paragraph(
                    patterns.join(" \u{00b7} "),
                    10.5,
                    13.65,
                    theme::text_muted(),
                ))
            })
        }
        None => value("no handler".into()),
    };
    let edit = failure
        .winner
        .as_ref()
        .filter(|step| step.source == Source::Rules && step.stage == Stage::Handler)
        .map(|step| {
            let handler = step.index;
            button(
                format!("Edit handler {}", handler + 1),
                ButtonKind::Default,
                true,
            )
            .id(("failure-edit", index))
            .debug_selector(move || format!("failure-edit-{index}"))
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus, cx);
                this.select(Selection::Handler(handler), cx);
            }))
        });
    div()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .pt(px(10.0))
        .px(px(14.0))
        .pb(px(12.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .bg(theme::danger_soft())
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(12.0))
                .child(
                    div()
                        .flex_none()
                        .mt(px(16.0))
                        .size(px(16.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme::status_error())
                        .child(mono("\u{00d7}", 9.0, 700.0, theme::accent_on())),
                )
                .child(column("Input", input).flex_grow(1.4))
                .child(column("Expected", value(failure.expected.clone())))
                .child(column("Actual", value(failure.actual.clone())))
                .child(column("Winner", winner)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(28.0))
                .children(edit)
                .child(paragraph(
                    "Nothing is undone for you; the rules are saved as they are.",
                    10.5,
                    15.75,
                    theme::text_muted(),
                )),
        )
}

/// `.tstrip`: how the last test run went, with a row per failure. Nothing
/// until the file has an example or a test.
fn tests_strip(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Option<Stateful<Div>> {
    let (examples, tests) = app.tests.counts();
    if examples + tests == 0 {
        return None;
    }
    let mut what = Vec::new();
    if examples > 0 {
        what.push(format!(
            "{examples} {}",
            if examples == 1 { "example" } else { "examples" }
        ));
    }
    if tests > 0 {
        what.push(format!(
            "{tests} saved {}",
            if tests == 1 { "test" } else { "tests" }
        ));
    }
    let when = match app.tests.ran_at() {
        Some(at) if app.tests.report().is_some() => format!("ran on save, {}", clock(at)),
        Some(_) => "the rules changed since the last run".into(),
        None => "not run yet".into(),
    };
    let note = format!("{} \u{00b7} {when}", what.join(" \u{00b7} "));
    let (summary, failures, bad): (Vec<Div>, Vec<&TestFailure>, bool) = match app.tests.report() {
        Some(Ok(report)) => {
            let failed = report.failures.len();
            let mut summary = Vec::new();
            if failed > 0 {
                summary.push(text(
                    format!("{failed} failed"),
                    12.0,
                    600.0,
                    theme::danger_text(),
                ));
            }
            summary.push(text(
                format!("{} passed", report.passed),
                12.0,
                500.0,
                theme::text_primary(),
            ));
            (summary, report.failures.iter().collect(), failed > 0)
        }
        Some(Err(error)) => (
            vec![text(error.clone(), 12.0, 500.0, theme::danger_text())],
            Vec::new(),
            true,
        ),
        None => (
            vec![text("Tests", 12.0, 500.0, theme::text_primary())],
            Vec::new(),
            false,
        ),
    };
    let row = div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(10.0))
        .px(px(14.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(10.0))
                .children(summary)
                .child(text(note, 12.0, 400.0, theme::text_muted())),
        )
        .child(
            button("Run tests", ButtonKind::Default, true)
                .id("run-tests")
                .debug_selector(|| "run-tests".into())
                .on_click(cx.listener(|this, _, _, cx| this.run_tests(cx))),
        );
    let strip: Stateful<Div> = div()
        .id("tests-strip")
        .debug_selector(|| "tests-strip".into())
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(if bad {
            theme::status_error()
        } else {
            theme::border_subtle()
        })
        .rounded(px(10.0))
        .bg(theme::bg_panel())
        .overflow_hidden()
        .child(row);
    Some(
        failures
            .into_iter()
            .enumerate()
            .fold(strip, |strip: Stateful<Div>, (index, failure)| {
                strip.child(failure_row(index, failure, cx))
            }),
    )
}

/// The Saved tests card: the file's top-level `tests`, each with its
/// result. Nothing until there is one.
fn saved_tests(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Option<Div> {
    let tests = app.store.tests();
    if tests.is_empty() {
        return None;
    }
    let failed = tests
        .iter()
        .enumerate()
        .filter(|(i, _)| app.tests.passed(TestSource::Test { test: *i }) == Some(false))
        .count();
    let count = if failed > 0 {
        chip_in(format!("{failed} failed"), ChipTone::Bad)
    } else if app.tests.report().is_some() {
        toned_chip(format!("{} passed", tests.len()), Tone::Plain)
            .bg(theme::success_soft())
            .border_color(theme::transparent())
            .text_color(theme::success_text())
    } else {
        toned_chip(format!("{} saved", tests.len()), Tone::Plain)
    };
    let header = div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .py(px(13.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            card_title(
                "Whole resolutions \u{00b7} rules.json \u{203a} tests",
                "Saved tests",
            )
            .child(
                paragraph(
                    "A link, where it came from, and what must come out: browser, profile and \
                     the URL after rewrites. Saved from Test a link or from an Activity row.",
                    11.0,
                    16.5,
                    theme::text_muted(),
                )
                .mt(px(4.0))
                .max_w(px(520.0)),
            ),
        )
        .child(count);
    let card = tests
        .iter()
        .enumerate()
        .fold(card().child(header), |card, (index, test)| {
            let source = TestSource::Test { test: index };
            let passed = app.tests.passed(source);
            let failed = passed == Some(false);
            let from = test.opener.as_ref().map(|opener| {
                let name = opener
                    .split_once(':')
                    .map_or(opener.as_str(), |(name, _)| name);
                format!("from {name}")
            });
            let (name, profile) = match (&test.expect.profile, test.expect.browser.split_once(':'))
            {
                (Some(profile), _) => (test.expect.browser.clone(), Some(profile.clone())),
                (None, Some((name, profile))) => (name.to_owned(), Some(profile.to_owned())),
                (None, None) => (test.expect.browser.clone(), None),
            };
            let last = index + 1 == tests.len();
            card.child(
                div()
                    .id(("saved-test", index))
                    .debug_selector(move || format!("saved-test-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(9.0))
                    .px(px(12.0))
                    .when(!last, |row| {
                        row.border_b_1().border_color(theme::border_faint())
                    })
                    .child(
                        div()
                            .w(px(22.0))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .child(example_mark(passed)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.0))
                            .child(one_line(
                                test.url.clone(),
                                theme::mono_family(500.0),
                                500.0,
                                11.0,
                                11.0,
                                if failed {
                                    theme::danger_text()
                                } else {
                                    theme::text_primary()
                                },
                            ))
                            .when_some(from, |cell, from| {
                                cell.child(text(from, 11.0, 500.0, theme::text_muted()))
                            }),
                    )
                    .child(arrow())
                    .child(
                        div()
                            .w(px(column::DESTINATION))
                            .flex_none()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(7.0))
                            .child(badge(badge_for(&name)))
                            .child(text(name, 12.0, 500.0, theme::text_primary()))
                            .when_some(profile, |cell, profile| {
                                cell.child(mono(profile, 10.5, 500.0, theme::text_muted()))
                            })
                            .when_some(test.expect.url.clone(), |cell, url| {
                                cell.child(one_line(
                                    url,
                                    theme::mono_family(500.0),
                                    500.0,
                                    10.5,
                                    10.5,
                                    theme::text_muted(),
                                ))
                            }),
                    )
                    .child(
                        div()
                            .id(("test-remove", index))
                            .debug_selector(move || format!("test-remove-{index}"))
                            .w(px(column::MORE))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .cursor_pointer()
                            .child(mono("\u{00d7}", 11.0, 500.0, theme::text_subtle()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_test(index, cx);
                            })),
                    ),
            )
        });
    Some(card)
}

impl PwrRoute {
    /// Runs every example and test now: Run tests, and each save.
    pub fn run_tests(&mut self, cx: &mut Context<'_, Self>) {
        self.tests.set_rules(self.store.edited());
        self.tests.run();
        cx.notify();
    }

    /// Removes a saved test from the file.
    pub fn remove_test(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.store.remove_test(index);
        self.refresh(cx);
    }
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
        crate::rules_store::Status::Failed(_) | crate::rules_store::Status::Changed => {
            (theme::status_error(), app.store.status_label())
        }
        _ => (theme::status_ok(), "saved on change".to_owned()),
    };
    let strip = tests_strip(app, cx);
    let saved = saved_tests(app, cx);
    let footer = if app.tests.counts() == (0, 0) {
        status
    } else {
        format!("{status} \u{00b7} tests run on every save")
    };
    let banners = crate::importer_view::rules_banners(app, cx);
    pane_frame("rules-pane")
        .child(heading(app, cx))
        .children(banners)
        .children(strip)
        .child(visual_cards(app, cx))
        .child(everything_else)
        .children(saved)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(theme::dot(dot))
                .child(one_line(
                    format!(
                        "rules.json \u{00b7} {} \u{00b7} {footer}",
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
