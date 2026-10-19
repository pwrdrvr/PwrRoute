//! 1b Configuration: where rules come from, the Finicky importer, short
//! links and the files on disk.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div, px,
};

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, action_tag, add_pill, button, card_description, card_heading, checkbox, chip,
    command_box, field, field_help, field_sub, field_sub_code, fn_tag, pane_head, radio_dot,
    row_actions, slot, switch, tag,
};
use super::model::{ImportCandidate, Piece, RulesSource, SettingsRequest};
use super::style::para;
use crate::theme::{self, mono, text};

pub const VISUAL_SLOT_ID: &str = "config-source-visual";
pub const FINICKY_SLOT_ID: &str = "config-source-finicky";
pub const IMPORT_ID: &str = "config-import";

impl SettingsWindow {
    fn set_source(&mut self, source: RulesSource, window: &mut Window, cx: &mut Context<'_, Self>) {
        let configuration = &mut self.model.configuration;
        if configuration.active == source
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

    fn set_expand_short_links(
        &mut self,
        on: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.model.configuration.expand_short_links = on;
        self.request(SettingsRequest::SetExpandShortLinks(on), window, cx);
        cx.notify();
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

/// A button whose only job is to send `request`.
fn request_button(
    id: &'static str,
    label: &'static str,
    kind: Kind,
    small: bool,
    request: SettingsRequest,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    button(id, label, kind, small).on_click(cx.listener(move |this, _, window, cx| {
        this.request(request.clone(), window, cx);
    }))
}

/// `.slot`: one source tile.
fn source_slot(
    source: RulesSource,
    path: SharedString,
    about: SharedString,
    active: RulesSource,
    available: bool,
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
    slot(id, selected)
        .disabled(!available)
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
            cx,
        ))
        .child(source_slot(
            RulesSource::Finicky,
            finicky_path,
            finicky_about,
            active,
            configuration.finicky.is_some(),
            cx,
        ));
    let chosen = match active {
        RulesSource::Visual => "Visual rules",
        RulesSource::Finicky => "Finicky config",
    };
    let rows = vec![field(
        "Active source",
        Some(field_sub(
            "Switching takes effect on the next link. Nothing is deleted either way.",
        )),
        [
            tiles.into_any_element(),
            row_actions()
                .child(request_button(
                    "config-choose-file",
                    "Choose another file\u{2026}",
                    Kind::Default,
                    true,
                    SettingsRequest::ChooseConfigFile,
                    cx,
                ))
                .child(field_help(".js or .ts, Finicky v4 shape."))
                .into_any_element(),
        ],
    )];
    this.disclosure(
        "config-source",
        card_heading("Where rules come from", "Source", None),
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
        let (source, asides): (Vec<_>, Vec<_>) = candidate
            .summary
            .iter()
            .cloned()
            .partition(|piece| matches!(piece, Piece::Source(_)));
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
    let actions = row_actions()
        .mt(px(6.0))
        .child(
            button(IMPORT_ID, import_label, Kind::Primary, false)
                .disabled(checked == 0)
                .when(checked == 0, |button| button.opacity(0.6))
                .on_click(cx.listener(|this, _, window, cx| this.import(false, window, cx))),
        )
        .child(
            button(
                "config-import-keep",
                "Import and keep the config for the rest",
                Kind::Default,
                false,
            )
            .disabled(checked == 0)
            .on_click(cx.listener(|this, _, window, cx| this.import(true, window, cx))),
        )
        .into_any_element();
    let matchers = if stays == 1 {
        "function matcher keeps".to_owned()
    } else {
        "function matchers keep".to_owned()
    };
    let help = field_help(format!(
        "The second button leaves {path} selected so the {} {matchers} working, with the \
         imported rules running after it.",
        count_word(stays),
    ))
    .into_any_element();
    let rows = vec![field(
        "Rules found",
        Some(field_sub(
            "Checked rows become visual rules. Unchecked ones keep running from the config if it \
             stays selected.",
        )),
        lines.chain([actions, help]),
    )];
    let stays_chip =
        (stays > 0).then(|| chip(format!("{stays} stay JS"), Tone::Warn).into_any_element());
    Some(this.disclosure(
        "config-import",
        card_heading(
            "Migration",
            "Import from Finicky",
            Some(card_description(format!(
                "Reads {path} and offers what the visual rules can hold. The file itself is not \
                 changed."
            ))),
        ),
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
        })
        .child(
            add_pill("config-add-shortener", "+ host").on_click(cx.listener(
                |this, _, window, cx| this.request(SettingsRequest::AddShortener, window, cx),
            )),
        );
    let rows = vec![field(
        "Expand short links before matching",
        Some(field_sub_code(
            "Follows one redirect for these hosts so rules see the real destination. Finicky\u{2019}s ",
            "urlShorteners",
            ".",
        )),
        [
            switch(
                "config-expand-short-links",
                configuration.expand_short_links,
            )
            .accessibility_label("Expand short links before matching")
            .on_change({
                let set = cx.listener(|this, on: &bool, window, cx| {
                    this.set_expand_short_links(*on, window, cx);
                });
                move |on, _, window, cx| set(&on, window, cx)
            })
            .into_any_element(),
            hosts.into_any_element(),
        ],
    )];
    this.disclosure(
        "config-short-links",
        card_heading("Options", "Short links", None),
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
    this.disclosure(
        "config-files",
        card_heading("On disk", "Files", None),
        None,
        rows,
        cx,
    )
}
