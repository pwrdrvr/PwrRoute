//! 1b Configuration: where rules come from, the Finicky importer, short
//! links and the files on disk.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div, px,
};

use super::controls::{
    Kind, Tone, action_tag, button_when, card_description, checkbox, chip, command_box, field,
    field_help, field_sub, field_sub_code, fn_tag, heading, note, pane_head, radio_dot,
    row_actions, slot, tag,
};
use super::model::{ImportCandidate, Piece, RulesSource, SettingsRequest};
use super::style::para;
use super::{SettingsWindow, request_button};
use crate::theme::{self, mono, text};

pub const VISUAL_SLOT_ID: &str = "config-source-visual";
pub const FINICKY_SLOT_ID: &str = "config-source-finicky";
pub const IMPORT_ID: &str = "config-import";
pub const KEEP_CONFIG_ID: &str = "config-import-keep";

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

    fn toggle_candidate(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if let Some(candidate) = self
            .model
            .configuration
            .import
            .as_mut()
            .and_then(|plan| plan.candidates.get_mut(index))
            .filter(|candidate| candidate.importable)
        {
            candidate.checked = !candidate.checked;
            cx.notify();
        }
    }

    fn import(&mut self, keep_config: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(plan) = self.model.configuration.import.as_ref() else {
            return;
        };
        let candidates = plan
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.importable && candidate.checked)
            .map(|(index, _)| index)
            .collect();
        self.request(
            SettingsRequest::Import {
                candidates,
                keep_config,
            },
            window,
            cx,
        );
    }
}

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "Rules source",
            "Configuration",
            "Where the router reads its rules from. Visual rules are the default. A Finicky \
             config, when selected, runs first and owns the default browser.",
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
        "The router follows the files on disk: a Finicky config at one of its usual paths runs \
         first and owns the default browser; without one, rules.json is the whole config. \
         Choosing here is not built yet."
    };
    let rows = vec![field(
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
    this.disclosure(
        "config-source",
        heading("Where rules come from", "Source"),
        Some(chip(chosen, Tone::Ok).into_any_element()),
        rows,
        cx,
    )
}

/// A small count as a word, the way the design's sentences spell it.
fn count_word(count: usize) -> String {
    const WORDS: [&str; 10] = [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    WORDS
        .get(count)
        .map_or_else(|| count.to_string(), |word| (*word).to_owned())
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
                checked: false,
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
    let lines = plan.candidates.iter().enumerate().map(|(index, candidate)| {
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
            .child(
                div().w(px(24.0)).flex_none().child(
                    checkbox(("config-import-check", index), candidate.checked, candidate.importable)
                        .on_change({
                            let toggle = cx.listener(move |this, _: &(), _, cx| {
                                this.toggle_candidate(index, cx);
                            });
                            move |_, _, window, cx| toggle(&(), window, cx)
                        }),
                ),
            )
            .child(summary(candidate))
            // A grid item stretches: the chip fills its 100px column.
            .child(div().w(px(100.0)).flex_none().flex().child(verdict.flex_1()))
            .into_any_element()
    });
    let checked = plan.checked();
    let stays = plan.stays_js();
    let import_label = SharedString::from(match checked {
        1 => "Import 1 rule".to_owned(),
        count => format!("Import {count} rules"),
    });
    let enabled = checked > 0;
    // Keeping the config only means something while some rules stay JS.
    let actions = row_actions()
        .mt(px(6.0))
        .child(
            button_when(IMPORT_ID, import_label, Kind::Primary, false, enabled)
                .on_click(cx.listener(|this, _, window, cx| this.import(false, window, cx))),
        )
        .when(stays > 0, |actions| {
            actions.child(
                button_when(
                    KEEP_CONFIG_ID,
                    "Import and keep the config for the rest",
                    Kind::Default,
                    false,
                    enabled,
                )
                .on_click(cx.listener(|this, _, window, cx| this.import(true, window, cx))),
            )
        })
        .into_any_element();
    let matchers = if stays == 1 {
        "function matcher keeps"
    } else {
        "function matchers keep"
    };
    let help = (stays > 0).then(|| {
        field_help(format!(
            "The second button leaves {path} selected so the {} {matchers} working, with the \
             imported rules running after it.",
            count_word(stays),
        ))
        .into_any_element()
    });
    let rows = vec![field(
        "Rules found",
        Some(field_sub(
            "Checked rows become visual rules. Unchecked ones keep running from the config if it \
             stays selected.",
        )),
        lines.chain([actions]).chain(help),
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
    let rows = vec![field(
        "Expanded before matching",
        Some(field_sub_code(
            "Links from these hosts are followed one redirect so rules see the real \
             destination. A Finicky config adds its ",
            "urlShorteners",
            ".",
        )),
        [
            hosts.into_any_element(),
            note("The list is the engine\u{2019}s; rules.json cannot change it yet.")
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
