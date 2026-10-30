//! 1b Configuration: where rules come from, the Finicky importer, short
//! links and the files on disk. While a Finicky config is in charge, the
//! Source card also says whose default takes everything else (Overview and
//! Setup 5d): the config's or PwrRoute's, and which browser PwrRoute's is.

use std::time::Duration;

use gpui_kit::base::actions::{SelectDown, SelectUp};
use gpui_kit::base::{Radio, Select, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, Div, ElementId, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, ease_out_quint,
    point, px,
};

use super::controls::{
    Kind, Tone, action_tag, badge, card_description, chip, command_box, field, field_help,
    field_sub, field_sub_code, fn_tag, heading, note, pane_head, radio_dot, row_actions, slot, tag,
};
use super::model::{
    DefaultOwner, EverythingElse, ImportCandidate, Piece, RulesSource, SettingsRequest,
};
use super::style::{self, para};
use super::{SettingsWindow, request_button};
use crate::theme::{self, mono, text};
use crate::widgets::{field_label, menu_card, menu_row, profiles_hint, segmented};

pub const VISUAL_SLOT_ID: &str = "config-source-visual";
pub const FINICKY_SLOT_ID: &str = "config-source-finicky";
/// The importer card's "Review import…".
pub const REVIEW_IMPORT_ID: &str = "config-review-import";
/// The Everything else field's segmented control, and its two answers.
pub const EVERYTHING_ELSE_ID: &str = "config-everything-else";
pub const CONFIG_DEFAULT_ID: &str = "config-everything-else-config";
pub const PWRROUTE_DEFAULT_ID: &str = "config-everything-else-pwrroute";
/// What PwrRoute's default changes, under the control.
pub const WARNINGS_ID: &str = "config-everything-else-warnings";
/// Why the last change was not saved, under the selects.
pub const PROBLEM_ID: &str = "config-everything-else-problem";
/// The browser and profile selects, and their options (by index).
pub const DEFAULT_BROWSER_ID: &str = "config-default-browser";
pub const DEFAULT_PROFILE_ID: &str = "config-default-profile";
pub const BROWSER_OPTION_ID: &str = "config-default-browser-option";
pub const PROFILE_OPTION_ID: &str = "config-default-profile-option";

/// The profile select's first row: `defaultProfile` left empty.
const DEFAULT_PROFILE: &str = "Default profile";

/// The selects' widths (design 5d).
const BROWSER_WIDTH: f32 = 260.0;
const PROFILE_WIDTH: f32 = 150.0;

/// How long the meaning under the control takes to fade in after a flip.
const FADE: Duration = Duration::from_millis(150);

/// Which of the Everything else selects is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picking {
    Browser,
    Profile,
}

#[derive(Clone, Copy, Debug)]
struct Menu {
    picking: Picking,
    highlighted: usize,
}

/// The pane's own view state: the open select, and how many times the
/// reader flipped the control (a flip replays the fade; the first draw is
/// still).
pub struct State {
    menu: Option<Menu>,
    browser_trigger: FocusHandle,
    profile_trigger: FocusHandle,
    menu_focus: FocusHandle,
    flips: usize,
}

impl State {
    pub fn new(cx: &mut Context<'_, SettingsWindow>) -> Self {
        Self {
            menu: None,
            browser_trigger: cx.focus_handle(),
            profile_trigger: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            flips: 0,
        }
    }

    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    /// The open select, for tests.
    #[cfg(test)]
    pub fn open_menu(&self) -> Option<Picking> {
        self.menu.map(|menu| menu.picking)
    }
}

impl SettingsWindow {
    fn set_source(&mut self, source: RulesSource, window: &mut Window, cx: &mut Context<'_, Self>) {
        let configuration = &mut self.model.configuration;
        if !configuration.can_switch
            || configuration.active == source
            || (source == RulesSource::Finicky && configuration.finicky.is_none())
        {
            return;
        }
        configuration.active = source;
        self.request(SettingsRequest::SetRulesSource(source), window, cx);
        cx.notify();
    }

    /// The segmented control: the window flips at once and the producer
    /// writes `rules.json`, then reads it back.
    fn set_default_owner(
        &mut self,
        owner: DefaultOwner,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(field) = self.model.configuration.everything_else.as_mut() else {
            return;
        };
        if field.owner == owner {
            return;
        }
        field.owner = owner;
        field.problem = None;
        self.configuration.flips += 1;
        self.configuration.menu = None;
        self.request(SettingsRequest::SetDefaultOwner(owner), window, cx);
        cx.notify();
    }

    /// How many rows `picking`'s list has.
    fn default_options(&self, picking: Picking) -> usize {
        let Some(field) = &self.model.configuration.everything_else else {
            return 0;
        };
        match picking {
            Picking::Browser => field.browsers.len(),
            Picking::Profile => field.chosen().map_or(0, |chosen| chosen.profiles.len() + 1),
        }
    }

    fn open_default_menu(&mut self, picking: Picking, open: bool, cx: &mut Context<'_, Self>) {
        let options = self.default_options(picking);
        self.configuration.menu = (open && options > 0).then(|| {
            let field = self.model.configuration.everything_else.as_ref();
            let current = match picking {
                Picking::Browser => field.and_then(|field| field.browser),
                Picking::Profile => field.and_then(|field| {
                    let chosen = field.chosen()?;
                    match &field.profile {
                        Some(profile) => chosen
                            .profiles
                            .iter()
                            .position(|name| name == profile)
                            .map(|index| index + 1),
                        None => Some(0),
                    }
                }),
            };
            Menu {
                picking,
                highlighted: current.unwrap_or(0),
            }
        });
        cx.notify();
    }

    fn move_default_highlight(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let Some(menu) = self.configuration.menu else {
            return;
        };
        let last = self.default_options(menu.picking).saturating_sub(1);
        if let Some(menu) = self.configuration.menu.as_mut() {
            menu.highlighted = menu.highlighted.saturating_add_signed(step).min(last);
            cx.notify();
        }
    }

    /// Picks row `index` of `picking`'s list: a browser (with its own
    /// default profile) or one of the chosen browser's profiles.
    pub(crate) fn choose_default(
        &mut self,
        picking: Picking,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.configuration.menu = None;
        let trigger = match picking {
            Picking::Browser => self.configuration.browser_trigger.clone(),
            Picking::Profile => self.configuration.profile_trigger.clone(),
        };
        window.focus(&trigger, cx);
        let Some(field) = self.model.configuration.everything_else.as_mut() else {
            return;
        };
        let (browser, profile) = match picking {
            Picking::Browser => {
                if field.browsers.get(index).is_none() || field.browser == Some(index) {
                    cx.notify();
                    return;
                }
                (Some(index), None)
            }
            Picking::Profile => {
                let Some(chosen) = field.chosen() else {
                    return;
                };
                let profile = match index {
                    0 => None,
                    _ => match chosen.profiles.get(index - 1) {
                        Some(profile) => Some(profile.clone()),
                        None => return,
                    },
                };
                if profile == field.profile {
                    cx.notify();
                    return;
                }
                (field.browser, profile)
            }
        };
        field.browser = browser;
        field.profile = profile.clone();
        field.problem = None;
        let Some(chosen) = field.chosen() else {
            return;
        };
        let request = SettingsRequest::SetDefaultBrowser {
            browser: chosen.value.clone(),
            profile: profile.unwrap_or_default(),
        };
        self.request(request, window, cx);
        cx.notify();
    }
}

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "Rules source",
            "Configuration",
            "Where the router reads its rules from. Visual rules are the default. A Finicky \
             config, when selected, runs its rewrites first.",
            None,
        ))
        .child(source_card(this, cx))
        .when_some(import_card(this, cx), Div::child)
        .child(short_links_card(this, cx))
        .child(files_card(this, cx))
}

/// `.slot`: one source tile.
fn source_slot(
    source: RulesSource,
    path: SharedString,
    about: SharedString,
    active: RulesSource,
    available: bool,
    enabled: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let (id, title) = match source {
        RulesSource::Visual => (VISUAL_SLOT_ID, "Visual rules"),
        RulesSource::Finicky => (FINICKY_SLOT_ID, "Finicky config"),
    };
    let selected = source == active;
    let set = cx.listener(move |this, _: &bool, window, cx| this.set_source(source, window, cx));
    let title_color = if selected {
        theme::accent_bright()
    } else {
        theme::text_primary()
    };
    // Dimmed only when there is no such file; a tile that merely cannot be
    // chosen still shows the source in charge at full strength.
    slot(id, selected)
        .disabled(!available || !enabled)
        .accessibility_label(title)
        .on_change(move |on, _, window, cx| set(&on, window, cx))
        .when(!available, |slot| slot.opacity(0.5))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(text(title, 12.0, 600.0, title_color).line_height(px(14.4)))
                .child(radio_dot(selected)),
        )
        .child(
            para(path, 10.5, 500.0, 1.3, theme::text_muted())
                .font_family(theme::mono_family(500.0)),
        )
        .child(para(about, 10.5, 400.0, 1.45, theme::text_subtle()))
}

fn source_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let configuration = &this.model.configuration;
    let active = configuration.active;
    let (finicky_path, finicky_about) = configuration.finicky.as_ref().map_or_else(
        || {
            (
                SharedString::from("no config found"),
                SharedString::from("Looked for ~/.finicky.js and ~/.finicky.ts."),
            )
        },
        |config| (config.summary.clone(), config.found.clone()),
    );
    let can_switch = configuration.can_switch;
    let tiles = div()
        .w_full()
        .flex()
        .gap(px(8.0))
        .child(source_slot(
            RulesSource::Visual,
            configuration.visual_summary.clone(),
            "Edited in the Rules view and by Editor agents. Hot-reloaded by the router.".into(),
            active,
            true,
            can_switch,
            cx,
        ))
        .child(source_slot(
            RulesSource::Finicky,
            finicky_path,
            finicky_about,
            active,
            configuration.finicky.is_some(),
            can_switch,
            cx,
        ));
    let chosen = match active {
        RulesSource::Visual => "Visual rules",
        RulesSource::Finicky => "Finicky config",
    };
    let sub = if can_switch {
        "Switching takes effect on the next link. Nothing is deleted either way."
    } else {
        "The router follows the files on disk: a Finicky config at one of its usual paths is \
         in charge; without one, rules.json is the whole config. Choosing here is not built \
         yet."
    };
    let mut rows = vec![field(
        "Active source",
        Some(field_sub(sub)),
        [
            tiles.into_any_element(),
            field_help(
                "Looked for ~/.finicky.js, ~/.finicky.ts and ~/.config/finicky/finicky.js or .ts.",
            )
            .into_any_element(),
        ],
    )];
    // Only while a config is in charge: with visual rules alone, rules.json
    // is the whole config and its default takes everything else.
    if active == RulesSource::Finicky
        && let Some(everything_else) = &configuration.everything_else
    {
        rows.push(everything_else_field(this, everything_else, cx));
    }
    this.disclosure(
        "config-source",
        heading("Where rules come from", "Source"),
        Some(chip(chosen, Tone::Ok).into_any_element()),
        rows,
        cx,
    )
}

/// The summary column of an `.imp` row. A function matcher's source is one
/// truncated line, with any aside below it.
fn summary(candidate: &ImportCandidate) -> Div {
    if !candidate.importable {
        let (asides, source): (Vec<_>, Vec<_>) = candidate
            .summary
            .iter()
            .cloned()
            .partition(|piece| matches!(piece, Piece::Aside(_) | Piece::Code(_)));
        let line = |pieces: Vec<Piece>| {
            pieces_row(&ImportCandidate {
                summary: pieces,
                importable: false,
            })
        };
        return div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(line(source).flex_none().w_full())
            .when(!asides.is_empty(), |column| {
                column.child(line(asides).flex_none().w_full())
            });
    }
    pieces_row(candidate)
}

fn pieces_row(candidate: &ImportCandidate) -> Div {
    candidate.summary.iter().fold(
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.0))
            .overflow_hidden()
            .when(candidate.importable, Div::flex_wrap),
        |row, piece| match piece {
            Piece::Text(words) => row
                .child(para(words.clone(), 11.5, 400.0, 1.4, theme::text_secondary()).flex_none()),
            Piece::Tag(pattern) => row.child(tag(pattern.clone())),
            Piece::Action(action) => row.child(action_tag(action.clone())),
            Piece::Source(source) => row.child(fn_tag()).child(
                mono(source.clone(), 10.5, 400.0, theme::text_muted())
                    .flex_shrink_1()
                    .min_w_0()
                    .text_ellipsis(),
            ),
            Piece::Aside(words) => row.child(
                para(words.clone(), 10.5, 400.0, 1.5, theme::text_subtle())
                    .flex_none()
                    .whitespace_nowrap(),
            ),
            Piece::Code(code) => row.child(
                mono(code.clone(), 10.5, 400.0, theme::text_muted())
                    .flex_shrink_1()
                    .min_w_0()
                    .text_ellipsis(),
            ),
        },
    )
}

fn import_card(
    this: &SettingsWindow,
    cx: &mut Context<'_, SettingsWindow>,
) -> Option<impl IntoElement> {
    let configuration = &this.model.configuration;
    let plan = configuration.import.as_ref()?;
    let path = configuration.finicky.as_ref().map_or_else(
        || SharedString::from("the config"),
        |config| config.path.clone(),
    );
    // A summary: the verdicts, read-only. The importer itself (Review,
    // Place, Choose source) is where rules are picked.
    let lines = plan.candidates.iter().map(|candidate| {
        let verdict = if candidate.importable {
            chip("Imports", Tone::Ok)
        } else {
            chip("Stays JS", Tone::Warn)
        };
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .py(px(7.0))
            .border_b_1()
            .border_color(theme::border_faint())
            .child(summary(candidate))
            // A grid item stretches: the chip fills its 100px column.
            .child(div().w(px(100.0)).flex_none().flex().child(verdict.flex_1()))
            .into_any_element()
    });
    let stays = plan.stays_js();
    let actions = row_actions()
        .mt(px(6.0))
        .child(request_button(
            REVIEW_IMPORT_ID,
            "Review import\u{2026}",
            Kind::Primary,
            false,
            SettingsRequest::ReviewImport,
            cx,
        ))
        .into_any_element();
    let help = field_help(format!(
        "{} of {} import exactly. Review shows each rule's source, its verdict and the visual \
         rule it becomes; nothing is written until you import, and {path} is never changed.",
        plan.importable(),
        plan.candidates.len()
    ))
    .into_any_element();
    let rows = vec![field(
        "Rules found",
        Some(field_sub(
            "What the visual rules can hold. Rules that stay JS keep running from the config \
             while it is selected.",
        )),
        lines.chain([actions, help]),
    )];
    let stays_chip =
        (stays > 0).then(|| chip(format!("{stays} stay JS"), Tone::Warn).into_any_element());
    Some(this.disclosure(
        "config-import",
        heading("Migration", "Import from Finicky").description(card_description(format!(
            "Reads {path} and offers what the visual rules can hold. The file itself is not \
             changed."
        ))),
        stays_chip,
        rows,
        cx,
    ))
}

fn short_links_card(
    this: &SettingsWindow,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let configuration = &this.model.configuration;
    let hosts = configuration
        .shorteners
        .iter()
        .fold(div().flex().flex_wrap().gap(px(5.0)), |hosts, host| {
            hosts.child(tag(host.clone()))
        });
    let off = configuration.shorteners.is_empty();
    let rows = vec![field(
        if off {
            "Expansion is off"
        } else {
            "Expanded before matching"
        },
        Some(field_sub_code(
            "A link from one of these hosts is followed to where it lands (HEAD, then GET, up \
             to 3 redirects, 750 ms each) before any rule runs, so rules match the \
             destination and Activity shows both. A Finicky config sets its own list with ",
            "urlShorteners",
            "; an empty list turns expansion off.",
        )),
        [
            hosts.into_any_element(),
            note(if off {
                "The config in charge sets urlShorteners to an empty list, so no link is \
                 followed and nothing leaves this Mac before a rule sees it."
            } else {
                "The list is the engine\u{2019}s; rules.json cannot change it yet."
            })
            .into_any_element(),
        ],
    )];
    this.disclosure(
        "config-short-links",
        heading("Options", "Short links"),
        None,
        rows,
        cx,
    )
}

fn files_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let rows = vec![field(
        "rules.json",
        Some(field_sub(
            "Validated before publication; a bad edit keeps the last good snapshot.",
        )),
        [
            command_box(
                std::slice::from_ref(&this.model.configuration.rules_path),
                None,
            )
            .into_any_element(),
            field_help(this.model.configuration.rules_status.clone()).into_any_element(),
            row_actions()
                .child(request_button(
                    "config-reveal",
                    "Reveal",
                    Kind::Default,
                    true,
                    SettingsRequest::RevealRules,
                    cx,
                ))
                .child(request_button(
                    "config-data-folder",
                    "Open data folder",
                    Kind::Default,
                    true,
                    SettingsRequest::OpenDataFolder,
                    cx,
                ))
                .child(request_button(
                    "config-reload",
                    "Reload now",
                    Kind::Default,
                    true,
                    SettingsRequest::ReloadRules,
                    cx,
                ))
                .into_any_element(),
        ],
    )];
    this.disclosure("config-files", heading("On disk", "Files"), None, rows, cx)
}

// ---- Everything else (Overview and Setup 5d) -------------------------------

/// The Everything else field: who takes the links nothing claims, what
/// that means, what PwrRoute's default would change, and its browser.
fn everything_else_field(
    this: &SettingsWindow,
    field_model: &EverythingElse,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let owner = field_model.owner;
    let control = segmented()
        .id(EVERYTHING_ELSE_ID)
        .child(owner_segment(
            DefaultOwner::Config,
            field_model.config_label.clone(),
            owner,
            cx,
        ))
        .child(owner_segment(
            DefaultOwner::PwrRoute,
            "PwrRoute's default".into(),
            owner,
            cx,
        ))
        .test_support();
    let detail = match owner {
        DefaultOwner::Config => field_model.config_detail.clone(),
        DefaultOwner::PwrRoute => field_model.pwrroute_detail.clone(),
    };
    let warnings =
        (owner == DefaultOwner::PwrRoute && !field_model.warnings.is_empty()).then(|| {
            field_model
                .warnings
                .iter()
                .fold(
                    div().id(WARNINGS_ID).flex().flex_col().gap(px(2.0)),
                    |list, warning| list.child(warning_line(warning.clone(), style::warn_text())),
                )
                .test_support()
        });
    let meaning = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(field_help(detail))
        .children(warnings);
    // A flip fades the new meaning in; the first draw is still, so a
    // screenshot or a reopened window shows it at once.
    let flips = this.configuration.flips;
    let meaning = if flips == 0 {
        meaning.into_any_element()
    } else {
        meaning
            .with_animation(
                ("config-everything-else-meaning", flips),
                Animation::new(FADE).with_easing(ease_out_quint()),
                |meaning, delta| meaning.opacity(delta),
            )
            .into_any_element()
    };
    let selects = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(browser_select(this, field_model, cx))
        .when_some(
            field_model
                .chosen()
                .filter(|chosen| !chosen.profiles.is_empty()),
            |row, _| row.child(profile_select(this, field_model, cx)),
        );
    let problem = field_model
        .problem
        .clone()
        .map(|problem| error_line(problem).test_support().into_any_element());
    field(
        "Everything else",
        Some(field_sub("Links no handler or rule claims.")),
        [
            control.into_any_element(),
            meaning,
            selects.into_any_element(),
        ]
        .into_iter()
        .chain(problem),
    )
}

/// `.seg span`, on gpui-kit's `Radio`: one answer of the control.
fn owner_segment(
    owner: DefaultOwner,
    label: SharedString,
    current: DefaultOwner,
    cx: &mut Context<'_, SettingsWindow>,
) -> Radio {
    let id = match owner {
        DefaultOwner::Config => CONFIG_DEFAULT_ID,
        DefaultOwner::PwrRoute => PWRROUTE_DEFAULT_ID,
    };
    let selected = owner == current;
    let set = cx.listener(move |this, _: &bool, window, cx| {
        this.set_default_owner(owner, window, cx);
    });
    Radio::new(id)
        .checked(selected)
        .accessibility_label(label.clone())
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
            label,
            11.0,
            550.0,
            if selected {
                theme::accent_bright()
            } else {
                theme::text_secondary()
            },
        ))
}

/// `.warnl`: a `.wi` "!" and one line, in `color`.
fn warning_line(words: SharedString, color: gpui_kit::Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(14.0))
                .rounded_full()
                .bg(style::warn_soft())
                .child(text("!", 9.0, 700.0, color)),
        )
        .child(para(words, 10.5, 500.0, 1.5, color))
}

/// `.errl`: why a change was not saved, in mono on danger-soft.
fn error_line(words: SharedString) -> gpui_kit::Stateful<Div> {
    div()
        .id(PROBLEM_ID)
        .self_start()
        .py(px(6.0))
        .px(px(8.0))
        .rounded(px(5.0))
        .bg(theme::danger_soft())
        .child(
            para(words, 10.5, 500.0, 1.4, theme::danger_text())
                .font_family(theme::mono_family(500.0)),
        )
}

/// An `.inp` select trigger with its value and caret.
fn trigger(id: ElementId, open: bool, width: f32, content: Div) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .w(px(width))
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(30.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(if open {
            theme::border_strong()
        } else {
            theme::border_default()
        })
        .rounded(px(7.0))
        .overflow_hidden()
        .cursor_pointer()
        .hover(|style| style.border_color(theme::border_strong()))
        .child(content)
        .child(
            div()
                .ml_auto()
                .child(text("\u{25be}", 10.0, 400.0, theme::text_subtle())),
        )
}

/// One of the two selects: gpui-kit's `Select` around `trigger`, with the
/// list open under it.
fn select(
    this: &SettingsWindow,
    picking: Picking,
    value: SharedString,
    trigger_content: Div,
    rows: Vec<AnyElement>,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let (id, label, width, focus) = match picking {
        Picking::Browser => (
            DEFAULT_BROWSER_ID,
            "Everything else browser",
            BROWSER_WIDTH,
            &this.configuration.browser_trigger,
        ),
        Picking::Profile => (
            DEFAULT_PROFILE_ID,
            "Everything else profile",
            PROFILE_WIDTH,
            &this.configuration.profile_trigger,
        ),
    };
    let open = this
        .configuration
        .menu
        .is_some_and(|menu| menu.picking == picking);
    let open_change = cx.listener(move |this, open: &bool, _, cx| {
        this.open_default_menu(picking, *open, cx);
    });
    let confirm = cx.listener(move |this, _: &(), window, cx| {
        if let Some(menu) = this.configuration.menu {
            this.choose_default(menu.picking, menu.highlighted, window, cx);
        }
    });
    let trigger = trigger(
        ElementId::Name(format!("{id}-trigger").into()),
        open,
        width,
        trigger_content,
    )
    .map(crate::accessibility::clickable)
    .on_click(cx.listener(move |this, _, _, cx| {
        this.open_default_menu(picking, !open, cx);
    }));
    let menu = open.then(|| {
        rows.into_iter().fold(
            menu_card(ElementId::Name(format!("{id}-menu").into()))
                .existing_navigation()
                .track_focus(&this.configuration.menu_focus)
                .w(px(width))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.configuration.menu = None;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &SelectUp, _, cx| {
                    this.move_default_highlight(-1, cx);
                }))
                .on_action(cx.listener(|this, _: &SelectDown, _, cx| {
                    this.move_default_highlight(1, cx);
                })),
            |menu, row| menu.child(row),
        )
    });
    Select::new(id)
        .open(open)
        .accessibility_label(label)
        .accessibility_value(value)
        .on_open_change(move |open, window, cx| open_change(&open, window, cx))
        .on_confirm(move |window, cx| confirm(&(), window, cx))
        .focus_handle(focus)
        .content_focus_handle(&this.configuration.menu_focus)
        .child(trigger)
        .when_some(menu, |select, menu| {
            select.child(deferred(
                anchored()
                    .offset(point(px(0.0), px(4.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(menu),
            ))
        })
        .into_any_element()
}

/// A list row: highlighted under the pointer or the arrow keys; a click
/// picks it.
fn option_row(
    row: gpui_kit::Stateful<Div>,
    picking: Picking,
    index: usize,
    highlighted: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    row.when(highlighted, |row| row.bg(theme::bg_hover_strong()))
        .hover(|style| style.bg(theme::bg_hover()))
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            if let Some(menu) = this.configuration.menu.as_mut()
                && menu.highlighted != index
            {
                menu.highlighted = index;
                cx.notify();
            }
        }))
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.choose_default(picking, index, window, cx);
        }))
        .test_support()
        .into_any_element()
}

fn highlighted(this: &SettingsWindow, picking: Picking) -> Option<usize> {
    this.configuration
        .menu
        .filter(|menu| menu.picking == picking)
        .map(|menu| menu.highlighted)
}

/// The browser select: the chosen browser with its badge, or "Choose a
/// browser" while `rules.json` names none.
fn browser_select(
    this: &SettingsWindow,
    field_model: &EverythingElse,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let (value, content) = match field_model.chosen() {
        Some(chosen) => (
            chosen.name.clone(),
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(8.0))
                .child(badge(chosen.badge.clone()))
                .child(field_label(chosen.name.clone(), theme::text_primary())),
        ),
        None => (
            SharedString::from("none"),
            div().child(text(
                if field_model.browsers.is_empty() {
                    "No browsers found"
                } else {
                    "Choose a browser"
                },
                12.5,
                400.0,
                theme::text_subtle(),
            )),
        ),
    };
    let lit = highlighted(this, Picking::Browser);
    let rows = field_model
        .browsers
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            let profiles: Vec<String> = choice.profiles.iter().map(ToString::to_string).collect();
            option_row(
                menu_row(
                    (BROWSER_OPTION_ID, index),
                    Some(choice.badge.clone()),
                    choice.name.clone(),
                    profiles_hint(&profiles).map(Into::into),
                ),
                Picking::Browser,
                index,
                lit == Some(index),
                cx,
            )
        })
        .collect();
    select(this, Picking::Browser, value, content, rows, cx)
}

/// The profile select, beside the browser's when it has profiles: "Default
/// profile" (the browser's own), then each by name.
fn profile_select(
    this: &SettingsWindow,
    field_model: &EverythingElse,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let profiles = field_model
        .chosen()
        .map(|chosen| chosen.profiles.clone())
        .unwrap_or_default();
    let (value, content) = match &field_model.profile {
        Some(profile) => (
            profile.clone(),
            div()
                .flex_1()
                .min_w_0()
                .child(field_label(profile.clone(), theme::text_primary())),
        ),
        None => (
            SharedString::from(DEFAULT_PROFILE),
            div().child(text(DEFAULT_PROFILE, 12.5, 400.0, theme::text_subtle())),
        ),
    };
    let lit = highlighted(this, Picking::Profile);
    let rows = std::iter::once(SharedString::from(DEFAULT_PROFILE))
        .chain(profiles)
        .enumerate()
        .map(|(index, name)| {
            option_row(
                menu_row((PROFILE_OPTION_ID, index), None, name, None),
                Picking::Profile,
                index,
                lit == Some(index),
                cx,
            )
        })
        .collect();
    select(this, Picking::Profile, value, content, rows, cx)
}
