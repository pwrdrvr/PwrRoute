//! Privacy (`design/PwrRoute Activity Privacy and Route Tests.dc.html`,
//! turn 1): what the router keeps of each link, where, and for how long.
//! The one place a URL can be allowed onto disk.
//!
//! The numbers and the switch are the settings; the sentence under the
//! switch is the router's (`RouterHandle::retention().description`),
//! rendered verbatim so the pane, the Activity footer and an agent say the
//! same thing. Full URLs goes through a sheet first (1c), as does Clear now
//! on the journal: both are a step the reader confirms, and Escape or
//! Cancel leaves things where they were.

use gpui_kit::base::{Button, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    anchored, deferred, div, point, px,
};

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, button, button_when, card_description, chip, field, field_help, field_sub, heading,
    pane_head, row_actions,
};
use super::model::{JournalLevel, Privacy, SettingsRequest};
use super::style::{self, para};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, mono, text, tracked};

/// Element ids the tests reach for.
pub const KEEP_COUNT_ID: &str = "privacy-keep-count";
pub const KEEP_HOURS_ID: &str = "privacy-keep-hours";
pub const CLEAR_RECENT_ID: &str = "privacy-clear-recent";
pub const JOURNAL_OFF_ID: &str = "privacy-journal-off";
pub const JOURNAL_HOSTS_ID: &str = "privacy-journal-hosts";
pub const JOURNAL_FULL_ID: &str = "privacy-journal-full";
pub const REVEAL_ID: &str = "privacy-reveal";
pub const CLEAR_JOURNAL_ID: &str = "privacy-clear-journal";
pub const EXPORT_ID: &str = "privacy-export";
pub const DESCRIPTION_ID: &str = "privacy-description";
pub const SHEET_ID: &str = "privacy-sheet";
pub const SHEET_CANCEL_ID: &str = "privacy-sheet-cancel";
pub const SHEET_CONFIRM_ID: &str = "privacy-sheet-confirm";

/// "Keep the last" accepts this many links (the design's "10 to 2,000").
pub const COUNT_RANGE: std::ops::RangeInclusive<usize> = 10..=2_000;
/// "For at most" accepts this many hours: an hour to a year.
pub const HOURS_RANGE: std::ops::RangeInclusive<u64> = 1..=8_760;

/// The pane's own view state: the two number fields and the open sheet.
pub struct State {
    count: TextField,
    hours: TextField,
    sheet: Option<Sheet>,
    /// The open sheet's, so Escape reaches it.
    sheet_focus: FocusHandle,
}

/// A step the reader confirms before it happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    /// 1c: "Write full URLs to disk?"
    WriteFull,
    /// "Delete the journal?"
    ClearJournal,
}

impl State {
    pub fn new(privacy: &Privacy, cx: &mut Context<'_, SettingsWindow>) -> Self {
        Self {
            count: TextField::new(privacy.keep_count.to_string(), cx),
            hours: TextField::new(privacy.keep_hours.to_string(), cx),
            sheet: None,
            sheet_focus: cx.focus_handle(),
        }
    }

    /// The sheet that is open, for tests.
    #[cfg(test)]
    pub fn sheet(&self) -> Option<Sheet> {
        self.sheet
    }

    pub fn close_sheet(&mut self) {
        self.sheet = None;
    }
}

impl SettingsWindow {
    /// Return in a number field: a value in range is applied and sent; the
    /// field shows the number as applied either way.
    fn commit_retention(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let privacy = &self.model.privacy;
        let count = self
            .privacy
            .count
            .text
            .trim()
            .replace(',', "")
            .parse::<usize>()
            .ok()
            .filter(|n| COUNT_RANGE.contains(n))
            .unwrap_or(privacy.keep_count);
        let hours = self
            .privacy
            .hours
            .text
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|n| HOURS_RANGE.contains(n))
            .unwrap_or(privacy.keep_hours);
        self.privacy.count.set(count.to_string());
        self.privacy.hours.set(hours.to_string());
        let changed = count != privacy.keep_count || hours != privacy.keep_hours;
        self.model.privacy.keep_count = count;
        self.model.privacy.keep_hours = hours;
        if changed {
            self.request(SettingsRequest::SetRetention { count, hours }, window, cx);
        }
        cx.notify();
    }

    /// The journal switch. Off and Hosts only apply at once; Full URLs opens
    /// the sheet and moves nothing until it is confirmed.
    fn choose_journal(
        &mut self,
        level: JournalLevel,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if level == self.model.privacy.journal {
            return;
        }
        if level == JournalLevel::Full {
            self.open_sheet(Sheet::WriteFull, window, cx);
            return;
        }
        self.set_journal(level, window, cx);
    }

    fn set_journal(
        &mut self,
        level: JournalLevel,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.model.privacy.journal = level;
        self.request(SettingsRequest::SetJournal(level), window, cx);
        cx.notify();
    }

    fn open_sheet(&mut self, sheet: Sheet, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.privacy.sheet = Some(sheet);
        window.focus(&self.privacy.sheet_focus, cx);
        cx.notify();
    }

    fn cancel_sheet(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.privacy.sheet.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn confirm_sheet(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(sheet) = self.privacy.sheet.take() else {
            return;
        };
        window.focus(&self.focus, cx);
        match sheet {
            Sheet::WriteFull => self.set_journal(JournalLevel::Full, window, cx),
            Sheet::ClearJournal => {
                self.model.privacy.journal_files = None;
                self.request(SettingsRequest::ClearJournal, window, cx);
            }
        }
        cx.notify();
    }
}

pub fn pane(
    this: &mut SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let sheet = this
        .privacy
        .sheet
        .map(|sheet| sheet_overlay(this, sheet, window, cx));
    div()
        .child(pane_head(
            "Links and what is kept",
            "Privacy",
            "What PwrRoute remembers about the links it routes, where, and for how long. The \
             defaults keep everything in memory and nothing on disk.",
            None,
        ))
        .child(recent_card(this, window, cx))
        .child(journal_card(this, cx))
        .child(keep_card(this, cx))
        .children(sheet)
}

/// `.inp.num` with its `.unit`: a number the reader types, applied on
/// Return.
fn number_field(
    id: &'static str,
    field: &TextField,
    unit: &'static str,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let input = text_field::field(
        id,
        field,
        Look {
            mono: true,
            placeholder: "",
        },
        None,
        window,
    )
    .test_support()
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
        let edit = if this.privacy.count.focus.is_focused(window) {
            this.privacy.count.key(&event.keystroke, cx)
        } else {
            this.privacy.hours.key(&event.keystroke, cx)
        };
        match edit {
            Edit::Ignored => return,
            Edit::Submit => this.commit_retention(window, cx),
            Edit::Changed | Edit::Handled => {}
        }
        cx.stop_propagation();
        cx.notify();
    }))
    .on_click(cx.listener(move |this, _, window, cx| {
        let focus = if id == KEEP_COUNT_ID {
            this.privacy.count.focus.clone()
        } else {
            this.privacy.hours.focus.clone()
        };
        window.focus(&focus, cx);
        cx.notify();
    }));
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(div().w(px(72.0)).flex_none().flex().child(input))
        .child(text(unit, 11.5, 500.0, theme::text_muted()))
}

fn recent_card(
    this: &SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let privacy = &this.model.privacy;
    let rows = vec![
        field(
            "Keep the last",
            Some(field_sub(
                "The router\u{2019}s buffer. Activity, the tray\u{2019}s recents and agents read \
                 from it.",
            )),
            [
                number_field(KEEP_COUNT_ID, &this.privacy.count, "links", window, cx)
                    .into_any_element(),
                field_help("Oldest first out. 10 to 2,000. Return applies it.").into_any_element(),
            ],
        ),
        field(
            "For at most",
            Some(field_sub(
                "A record older than this is dropped at the next link, even when there is room.",
            )),
            [
                number_field(KEEP_HOURS_ID, &this.privacy.hours, "hours", window, cx)
                    .into_any_element(),
                field_help("Everything in the buffer goes when the router quits.")
                    .into_any_element(),
            ],
        ),
        field(
            "Forget now",
            Some(field_sub(
                "Empties the buffer here, in Activity and in the tray.",
            )),
            [row_actions()
                .child(
                    button(CLEAR_RECENT_ID, "Clear now", Kind::Default, true).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.request(SettingsRequest::ClearRecent, window, cx);
                        }),
                    ),
                )
                .child(field_help("Today\u{2019}s counts stay."))
                .into_any_element()],
        ),
    ];
    let label = format!(
        "{} \u{00b7} {} h \u{00b7} in memory",
        crate::activity::thousands(privacy.keep_count as u64),
        privacy.keep_hours
    );
    this.disclosure(
        "privacy-recent",
        heading("In memory", "Recent links"),
        Some(chip(label, Tone::Neutral).into_any_element()),
        rows,
        cx,
    )
}

/// `.seg span` as a button: one level of the journal switch.
fn level_button(
    id: &'static str,
    label: &'static str,
    level: JournalLevel,
    current: JournalLevel,
    cx: &mut Context<'_, SettingsWindow>,
) -> Button {
    let selected = level == current;
    let foreground = if selected {
        theme::accent_bright()
    } else {
        theme::text_secondary()
    };
    Button::new(id)
        .selected(selected)
        .accessibility_label(label)
        .py(px(5.0))
        .px(px(10.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .when(selected, |segment| segment.bg(theme::bg_row_active()))
        .when(!selected, |segment| {
            segment.hover(|style| style.bg(theme::bg_hover()))
        })
        .on_click(cx.listener(move |this, _, window, cx| {
            this.choose_journal(level, window, cx);
        }))
        .child(text(label, 11.0, 550.0, foreground))
}

/// `.disc`: the sentence the router says about what is kept, under its
/// "What is kept" lead. `warn` is the sheet's variant.
fn disclosure_box(
    id: &'static str,
    sentence: SharedString,
    warn: bool,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    let (border, fill) = if warn {
        (style::accent_border(), theme::bg_row_active())
    } else {
        (theme::border_faint(), theme::bg_app())
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .w_full()
        .py(px(10.0))
        .px(px(12.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(border)
        .bg(fill)
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(tracked(
            "What is kept",
            9.5,
            600.0,
            0.14,
            theme::text_muted(),
        ))
        .child(para(sentence, 11.5, 400.0, 1.6, theme::text_secondary()))
        .test_support()
}

fn journal_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let privacy = &this.model.privacy;
    let on = privacy.journal != JournalLevel::Off;
    let switch = div()
        .flex()
        .flex_none()
        .gap(px(1.0))
        .p(px(2.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .child(level_button(
            JOURNAL_OFF_ID,
            "Off",
            JournalLevel::Off,
            privacy.journal,
            cx,
        ))
        .child(level_button(
            JOURNAL_HOSTS_ID,
            "Hosts only",
            JournalLevel::Hosts,
            privacy.journal,
            cx,
        ))
        .child(level_button(
            JOURNAL_FULL_ID,
            "Full URLs",
            JournalLevel::Full,
            privacy.journal,
            cx,
        ));
    let files_note: SharedString = match (&privacy.journal_files, on) {
        (Some(files), _) => files.summary.clone(),
        (None, true) => "no file yet".into(),
        (None, false) => "not created while off".into(),
    };
    let path_box = div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(8.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .when(!on && privacy.journal_files.is_none(), |cmd| {
            cmd.opacity(0.45)
        })
        .child(
            div().flex_1().min_w_0().child(
                para(
                    privacy.journal_path.clone(),
                    11.0,
                    500.0,
                    1.5,
                    theme::text_secondary(),
                )
                .font_family(theme::mono_family(500.0)),
            ),
        )
        .child(para(files_note, 10.5, 400.0, 1.5, theme::text_subtle()).flex_none());
    let has_files = privacy.journal_files.is_some();
    let actions = row_actions()
        .child(
            button_when(REVEAL_ID, "Reveal", Kind::Default, true, on || has_files).on_click(
                cx.listener(|this, _, window, cx| {
                    this.request(SettingsRequest::RevealJournal, window, cx);
                }),
            ),
        )
        .child(
            button_when(
                CLEAR_JOURNAL_ID,
                "Clear now",
                Kind::Default,
                true,
                has_files,
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_sheet(Sheet::ClearJournal, window, cx);
            })),
        )
        .child(
            button(EXPORT_ID, "Export\u{2026}", Kind::Default, true).on_click(cx.listener(
                |this, _, window, cx| {
                    this.request(SettingsRequest::ExportActivity, window, cx);
                },
            )),
        );
    let help = if has_files {
        "Clear now deletes the files. Export writes the links in memory to a file you choose, \
         each at the level its rule allows."
    } else {
        "Export writes the links in memory to a file you choose, each at the level its rule \
         allows. It never holds more than Activity shows."
    };
    let mut rows = vec![
        field(
            "Write links to disk",
            Some(field_sub(
                "Takes effect on the next link. Turning it off does not delete the file.",
            )),
            [
                switch.into_any_element(),
                disclosure_box(DESCRIPTION_ID, privacy.description.clone(), false)
                    .into_any_element(),
            ]
            .into_iter()
            .chain(privacy.journal_error.clone().map(|error| {
                para(error, 10.5, 400.0, 1.5, theme::danger_text()).into_any_element()
            })),
        ),
        field(
            "File",
            Some(field_sub("One JSON object per line, newest last.")),
            [
                path_box.into_any_element(),
                actions.into_any_element(),
                field_help(help).into_any_element(),
            ],
        ),
    ];
    if !privacy.can_save {
        rows.push(field(
            "No data folder",
            None,
            [para(
                "The OS gave PwrRoute no data folder, so settings are not saved and the journal \
                 cannot be turned on.",
                11.0,
                400.0,
                1.5,
                theme::danger_text(),
            )
            .into_any_element()],
        ));
    }
    let label: SharedString = match (privacy.journal, &privacy.journal_files) {
        (JournalLevel::Off, _) => "Off".into(),
        (JournalLevel::Hosts, Some(files)) => {
            format!("Hosts only \u{00b7} since {}", files.since).into()
        }
        (JournalLevel::Hosts, None) => "Hosts only".into(),
        (JournalLevel::Full, Some(files)) => {
            format!("Full URLs \u{00b7} since {}", files.since).into()
        }
        (JournalLevel::Full, None) => "Full URLs".into(),
    };
    let tone = if on { Tone::Warn } else { Tone::Neutral };
    this.disclosure(
        "privacy-journal",
        heading("On disk", "Journal").description(card_description(
            "Off, PwrRoute never writes a URL to disk. On, it appends one line per link to a \
             file you can read, reveal and clear.",
        )),
        Some(chip(label, tone).into_any_element()),
        rows,
        cx,
    )
}

/// One row of the `.tbl.keep` table.
fn keep_row(cells: [AnyElement; 4], head: bool) -> Div {
    let [keep, memory, journal, json] = cells;
    div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .py(px(if head { 7.0 } else { 10.0 }))
        .px(px(16.0))
        .child(div().w(px(110.0)).flex_none().child(keep))
        .child(div().flex_1().min_w_0().child(memory))
        .child(div().flex_1().min_w_0().child(journal))
        .child(div().w(px(150.0)).flex_none().child(json))
}

fn keep_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let head =
        |label: &str| tracked(label, 9.5, 600.0, 0.14, theme::text_muted()).into_any_element();
    let cell = |words: &'static str| {
        para(words, 11.0, 400.0, 1.5, theme::text_secondary()).into_any_element()
    };
    let level = |words: &'static str| {
        para(words, 11.0, 600.0, 1.5, theme::text_primary()).into_any_element()
    };
    let code = |field: &'static str, note: Option<&'static str>| {
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(mono(field, 10.5, 500.0, theme::text_secondary()))
            .when_some(note, |cell, note| {
                cell.child(para(note, 10.5, 400.0, 1.5, theme::text_subtle()))
            })
            .into_any_element()
    };
    let table = div()
        .flex()
        .flex_col()
        .child(
            keep_row(
                [
                    head("Keep"),
                    head("In Activity and the tray"),
                    head("In the journal, when on"),
                    head("rules.json"),
                ],
                true,
            )
            .bg(theme::bg_elevated())
            .border_b_1()
            .border_color(theme::border_faint()),
        )
        .child(
            keep_row(
                [
                    level("Full"),
                    cell(
                        "The link as received and as opened, the app that sent it, every step \
                         that fired.",
                    ),
                    cell("The same, at the journal\u{2019}s level."),
                    code("\"privacy\": \"full\"", Some("the default")),
                ],
                false,
            )
            .border_b_1()
            .border_color(theme::border_faint()),
        )
        .child(
            keep_row(
                [
                    level("Host only"),
                    cell(
                        "Scheme and host, as in https://mybank.example. Path, query and fragment \
                         are dropped before the record exists; so are the opened URL and each \
                         step\u{2019}s URL.",
                    ),
                    cell("Scheme and host only, even when the journal keeps full URLs."),
                    code("\"privacy\": \"host\"", None),
                ],
                false,
            )
            .border_b_1()
            .border_color(theme::border_faint()),
        )
        .child(keep_row(
            [
                level("Nothing"),
                cell("A count. No URL, no host, no sending app. The row is not listed."),
                cell("Not written. The day\u{2019}s count stays in memory."),
                code("\"privacy\": \"none\"", None),
            ],
            false,
        ));
    let private_sub = div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(mono(
            "\"private\": true",
            10.5,
            500.0,
            theme::accent_bright(),
        ))
        .child(para("on a rule.", 11.0, 400.0, 1.5, theme::text_muted()));
    let rows = vec![
        table,
        field(
            "Private windows",
            Some(private_sub),
            [para(
                "A rule that opens a private window keeps host only unless its privacy says \
                 otherwise. A link no rule matched keeps the full URL, so you can see what to \
                 add a rule for. A Finicky config\u{2019}s handlers keep full URLs, and are \
                 private only when their arguments say so (--incognito, --inprivate, \
                 --private-window).",
                10.5,
                400.0,
                1.5,
                theme::text_subtle(),
            )
            .max_w(px(style::ch(60.0, 10.5)))
            .into_any_element()],
        ),
    ];
    this.disclosure(
        "privacy-keep",
        heading("Per rule", "What each rule keeps").description(card_description(
            "A rule decides how much of the links it takes is remembered, in memory and in the \
             journal. Set it in the rule editor under Keep.",
        )),
        None,
        rows,
        cx,
    )
}

/// `.scrim` and `.sheet.narrow`, over the whole window: the step the
/// reader confirms. Escape and Cancel leave the switch where it was.
fn sheet_overlay(
    this: &SettingsWindow,
    sheet: Sheet,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let privacy = &this.model.privacy;
    let viewport = window.viewport_size();
    let (eyebrow, title, body, note, confirm): (
        &str,
        &str,
        gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>>,
        SharedString,
        &str,
    ) = match sheet {
        Sheet::WriteFull => (
            "Journal",
            "Write full URLs to disk?",
            disclosure_box(
                "privacy-sheet-description",
                privacy.full_description.clone(),
                true,
            ),
            match &privacy.rules_keep_less {
                Some(rules) => format!(
                    "That includes every process running as you, backups, and sync tools that \
                     watch the data folder. {rules}; this does not change that."
                )
                .into(),
                None => "That includes every process running as you, backups, and sync tools \
                         that watch the data folder. A rule that keeps host only or nothing \
                         still holds back what it says."
                    .into(),
            },
            "Write full URLs",
        ),
        Sheet::ClearJournal => (
            "Journal",
            "Delete the journal?",
            disclosure_box(
                "privacy-sheet-description",
                format!(
                    "Every journal file is deleted ({}), and the links in memory are forgotten \
                     with them. Journaling goes on from the next link if it is still on.",
                    privacy.journal_path
                )
                .into(),
                true,
            ),
            "Today\u{2019}s counts stay. Anything another program copied from the file is not \
             touched."
                .into(),
            "Delete",
        ),
    };
    let footer_note: &str = match sheet {
        Sheet::WriteFull => "Until you turn it off or clear it.",
        Sheet::ClearJournal => "This cannot be undone.",
    };
    let panel = div()
        .id(SHEET_ID)
        .debug_selector(|| SHEET_ID.into())
        .test_support()
        .track_focus(&this.privacy.sheet_focus)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                this.cancel_sheet(window, cx);
                cx.stop_propagation();
            }
        }))
        .w(px(460.0))
        .flex()
        .flex_col()
        .bg(theme::bg_popover())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(12.0))
        .shadow_lg()
        .child(
            div()
                .pt(px(16.0))
                .px(px(18.0))
                .pb(px(12.0))
                .border_b_1()
                .border_color(theme::border_faint())
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::accent()))
                .child(text(title, 16.0, 700.0, theme::text_primary()).line_height(px(19.2))),
        )
        .child(
            div()
                .py(px(14.0))
                .px(px(18.0))
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(body)
                .child(para(note, 10.5, 400.0, 1.5, theme::text_subtle())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .py(px(12.0))
                .px(px(18.0))
                .border_t_1()
                .border_color(theme::border_faint())
                .child(div().flex_1().child(para(
                    footer_note,
                    10.5,
                    400.0,
                    1.5,
                    theme::text_subtle(),
                )))
                .child(
                    button(SHEET_CANCEL_ID, "Cancel", Kind::Quiet, false)
                        .on_click(cx.listener(|this, _, window, cx| this.cancel_sheet(window, cx))),
                )
                .child(
                    button(SHEET_CONFIRM_ID, confirm, Kind::Primary, false).on_click(
                        cx.listener(|this, _, window, cx| this.confirm_sheet(window, cx)),
                    ),
                ),
        );
    let mut scrim = theme::bg_app();
    scrim.a = 0.62;
    deferred(
        anchored().position(point(px(0.0), px(0.0))).child(
            div()
                .id("privacy-scrim")
                .w(viewport.width)
                .h(viewport.height)
                .bg(scrim)
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.0))
                .on_click(cx.listener(|this, _, window, cx| this.cancel_sheet(window, cx)))
                .child(panel.on_click(|_, _, cx| cx.stop_propagation())),
        ),
    )
    .into_any_element()
}
