//! A new rule starts from a link the person opened (design: Rule From a
//! Link). The rail's Start from picker lists the recent links Activity
//! holds in memory; choosing one puts its narrowest suggested pattern in
//! the rule, selected so typing replaces it, and the link in Try it. Clear
//! goes back to a blank rule. The Overview's first-rule prompt names the
//! last of them.
//!
//! The links are the feed's `Full` records and stay in memory: the chosen
//! one is only ever in Try it, which is never written. What reaches
//! `rules.json` is the pattern, once the rule has a browser.

use std::time::SystemTime;

use crate::theme::px;
use chrono::{DateTime, Local};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, ParentElement as _, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
};
use pwrroute_engine::{Privacy, suggest_rule};
use pwrroute_router::{DispatchRecord, Outcome};

use crate::activity::{self, matched_url};
use crate::app::{PwrRoute, Selection};
use crate::editor::RailMenu;
use crate::rules_model::RowBadge;
use crate::theme::{self, text, tracked};
use crate::widgets::{ButtonKind, button, caret, input_box, one_line};

/// How many links the picker offers.
pub const RECENT_LIMIT: usize = 20;

/// The link a draft started from: the URL as opened, which Try it runs
/// through the rewrites, and the URL the handlers saw after them, which
/// the patterns are suggested from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartLink {
    pub url: String,
    pub seen: String,
}

/// A link the person opened, as the picker lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentLink {
    /// As opened, before the rewrites (`activity::matched_url`).
    pub url: String,
    /// After the rewrites: what the handlers matched.
    pub seen: String,
    /// "08:40".
    pub time: String,
    /// Where it went: "Microsoft Edge", "Google Chrome · Work".
    pub opened: String,
}

impl RecentLink {
    pub fn start(&self) -> StartLink {
        StartLink {
            url: self.url.clone(),
            seen: self.seen.clone(),
        }
    }
}

/// The links to start from: the records kept with their full URL, newest
/// first, one per URL, at most `limit`.
pub fn recent(records: &[DispatchRecord], limit: usize) -> Vec<RecentLink> {
    let mut links: Vec<RecentLink> = Vec::new();
    for record in records {
        if links.len() == limit {
            break;
        }
        if record.privacy != Privacy::Full || record.outcome == Outcome::Tested {
            continue;
        }
        let url = matched_url(record);
        if url.trim().is_empty() || links.iter().any(|link| link.url == url) {
            continue;
        }
        links.push(RecentLink {
            url: url.to_owned(),
            seen: record.final_url.as_deref().unwrap_or(url).to_owned(),
            time: clock(record.received_at),
            opened: crate::activity_actions::opened_label(record),
        });
    }
    links
}

/// The feed's links to start from; none without a feed.
pub fn recent_links(cx: &App) -> Vec<RecentLink> {
    activity::feed(cx)
        .map(|feed| recent(feed.read(cx).records(), RECENT_LIMIT))
        .unwrap_or_default()
}

/// An app that sent links lately (design: Destination and App Pickers
/// 2b): what a rule writes for it, its name, and how many of the links in
/// memory it sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    /// Its bundle id, else its name: what `rules[].opener` keeps.
    pub value: String,
    pub name: String,
    pub links: usize,
}

/// The apps that sent the links in memory, the latest sender first, each
/// once, at most `limit`. Tests run from no app, so they count for none.
pub fn senders(records: &[DispatchRecord], limit: usize) -> Vec<Sender> {
    let mut out: Vec<Sender> = Vec::new();
    for record in records {
        if record.outcome == Outcome::Tested {
            continue;
        }
        let Some(app) = record
            .link
            .opener
            .as_ref()
            .filter(|a| !a.name.trim().is_empty())
        else {
            continue;
        };
        let value = if app.bundle_id.trim().is_empty() {
            app.name.trim()
        } else {
            app.bundle_id.trim()
        };
        // One app, whether a record named it by bundle id or only by name.
        let name = app.name.trim();
        if let Some(sender) = out
            .iter_mut()
            .find(|s| s.value.eq_ignore_ascii_case(value) || s.name.eq_ignore_ascii_case(name))
        {
            sender.links += 1;
            // Keep the bundle id once any record has it.
            if sender.value == sender.name && value != name {
                sender.value = value.to_owned();
            }
        } else if out.len() < limit {
            out.push(Sender {
                value: value.to_owned(),
                name: app.name.trim().to_owned(),
                links: 1,
            });
        }
    }
    out
}

/// The feed's senders; none without a feed.
pub fn recent_senders(cx: &App) -> Vec<Sender> {
    activity::feed(cx)
        .map(|feed| senders(feed.read(cx).records(), 8))
        .unwrap_or_default()
}

/// The last link the person opened, if the feed kept it whole.
pub fn last_link(cx: &App) -> Option<RecentLink> {
    activity::feed(cx).and_then(|feed| recent(feed.read(cx).records(), 1).into_iter().next())
}

/// Why the picker has nothing to offer.
fn empty_reason(cx: &App) -> &'static str {
    let any = activity::feed(cx).is_some_and(|feed| !feed.read(cx).records().is_empty());
    if any {
        "Activity kept no full links to start from (Settings \u{203a} Privacy)."
    } else {
        "Click a link in any app and it shows up here."
    }
}

fn clock(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M").to_string()
}

/// The patterns a link suggests, narrowest first.
pub fn candidates(url: &str) -> Vec<String> {
    suggest_rule(url).candidates
}

/// The link the draft at `index` started from, if it still has one.
pub fn link_of(app: &PwrRoute, index: usize) -> Option<&StartLink> {
    match &app.draft_link {
        Some((at, Some(link))) if *at == index => Some(link),
        _ => None,
    }
}

/// Start from: the link the draft began with, a picker over the recent
/// links, and Clear (design 1a to 1d). Only for a draft.
pub fn section(app: &PwrRoute, index: usize, cx: &mut Context<'_, PwrRoute>) -> Div {
    let links = recent_links(cx);
    let chosen = link_of(app, index).map(|link| link.url.clone());
    let open = app.rail.as_ref().and_then(|rail| rail.menu) == Some(RailMenu::StartFrom);
    let head = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .min_h(px(18.0))
        .child(tracked("Start from", 9.5, 600.0, 0.14, theme::text_muted()))
        .when(chosen.is_some(), |head| {
            head.child(
                button("Clear", ButtonKind::Quiet, true)
                    .id("start-clear")
                    .debug_selector(|| "start-clear".into())
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.clear_start(window, cx))),
            )
        });
    let section = div()
        .debug_selector(|| "start-from".into())
        .flex()
        .flex_col()
        .gap(px(8.0))
        .py(px(14.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(head);
    if links.is_empty() && chosen.is_none() {
        return section
            .child(
                // Nothing to pick: the field's edge goes quiet, and its
                // words stay readable (muted, not faded).
                input_box()
                    .debug_selector(|| "start-box".into())
                    .w_full()
                    .border_color(theme::border_default())
                    .child(crate::widgets::field_label(
                        "No links yet",
                        theme::text_muted(),
                    ))
                    .child(theme::dimmed(caret)),
            )
            .child(help(empty_reason(cx)));
    }
    let time = chosen
        .as_ref()
        .and_then(|url| links.iter().find(|link| &link.url == url))
        .map(|link| link.time.clone());
    let content = match &chosen {
        Some(url) => div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1().min_w_0().child(one_line(
                url.clone(),
                theme::mono_family(500.0),
                500.0,
                11.5,
                14.0,
                theme::text_primary(),
            )))
            .when_some(time, |row, time| {
                row.child(div().flex_none().child(crate::theme::mono(
                    time,
                    10.5,
                    500.0,
                    theme::text_subtle(),
                )))
            }),
        None => div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1().min_w_0().child(crate::widgets::field_label(
                "Pick a link you opened\u{2026}",
                theme::text_subtle(),
            )))
            .child(div().flex_none().child(crate::theme::mono(
                format!("{} recent", links.len()),
                10.5,
                500.0,
                theme::text_subtle(),
            ))),
    };
    let picker = input_box()
        .id("start-box")
        .debug_selector(|| "start-box".into())
        .w_full()
        .cursor_pointer()
        .child(content)
        .child(caret())
        .capture_any_mouse_down(cx.listener(|this, _, _, _| this.note_rail_menu()))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_rail_menu(RailMenu::StartFrom, cx)));
    let note = match chosen.as_ref().and_then(|url| {
        links
            .iter()
            .position(|link| &link.url == url)
            .map(|at| (at, &links[at]))
    }) {
        Some((0, link)) => format!(
            "The last link you opened. It went to {}. Only Try it holds the link; it is never \
             saved.",
            link.opened
        ),
        Some((_, link)) => format!(
            "A link you opened at {}. It went to {}. Only Try it holds the link; it is never \
             saved.",
            link.time, link.opened
        ),
        None if chosen.is_some() => "Only Try it holds the link; it is never saved.".to_owned(),
        None => "Pick a link to fill the pattern and Try it.".to_owned(),
    };
    section
        .child(
            div()
                .relative()
                .w_full()
                .child(picker)
                .when(open, |picker| {
                    picker.child(deferred(menu(&links, chosen.as_deref(), cx)))
                }),
        )
        .child(help(note))
}

fn help(content: impl Into<SharedString>) -> Div {
    crate::widgets::paragraph(content, 10.5, 15.75, theme::text_subtle())
}

/// The picker's list (design 1b): each link on one line, its time and where
/// it went after it.
fn menu(
    links: &[RecentLink],
    chosen: Option<&str>,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let mut menu = crate::widgets::menu_card("start-menu")
        .debug_selector(|| "start-menu".into())
        .absolute()
        .top(px(34.0))
        .left_0()
        .right_0()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if let Some(rail) = &mut this.rail {
                rail.menu = None;
            }
            cx.notify();
        }))
        .child(crate::widgets::menu_heading(
            "Recent links \u{00b7} newest first",
        ));
    for (index, link) in links.iter().enumerate() {
        let start = link.start();
        let selected = chosen == Some(link.url.as_str());
        menu = menu.child(
            div()
                .id(("start-link", index))
                .debug_selector(move || format!("start-link-{index}"))
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .min_w_0()
                .px(px(8.0))
                .py(px(6.0))
                .rounded(px(5.0))
                .cursor_pointer()
                .when(selected, |row| row.bg(theme::bg_row_active()))
                .hover(|row| row.bg(theme::bg_hover()))
                .child(div().flex_1().min_w_0().child(one_line(
                    link.url.clone(),
                    theme::mono_family(500.0),
                    500.0,
                    11.0,
                    13.2,
                    theme::text_primary(),
                )))
                // The URL keeps the room; where it went gives way first.
                .child(div().flex_shrink_0().max_w(px(150.0)).min_w_0().child(one_line(
                    format!("{} \u{00b7} {}", link.time, link.opened),
                    theme::mono_family(500.0),
                    500.0,
                    10.5,
                    13.2,
                    theme::text_muted(),
                )))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_from(start.clone(), window, cx);
                })),
        );
    }
    menu.child(crate::widgets::menu_divider()).child(
        div().px(px(8.0)).pt(px(2.0)).pb(px(4.0)).child(help(
            "Links Activity keeps as host only, or not at all, are not listed. Picking one \
             replaces the pattern and Try it.",
        )),
    )
}

/// The patterns the start link suggests, under the pattern list; the one
/// in the first field is lit (design 1a). Only while the rule is a draft:
/// a saved rule, or another one at the index, is not the link's.
pub fn suggestions(
    app: &PwrRoute,
    index: usize,
    first: &str,
    cx: &mut Context<'_, PwrRoute>,
) -> Option<Div> {
    app.handlers
        .get(index)
        .filter(|handler| handler.rule.badge == RowBadge::Draft)?;
    let candidates = candidates(&link_of(app, index)?.seen);
    if candidates.is_empty() {
        return None;
    }
    let first = first.trim();
    let mut row = div()
        .debug_selector(|| "start-suggestions".into())
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.0))
        .child(text("Suggested", 10.5, 500.0, theme::text_subtle()));
    for (at, pattern) in candidates.into_iter().enumerate() {
        let on = pattern == first;
        let chosen = pattern.clone();
        row = row.child(
            div()
                .id(("start-suggestion", at))
                .debug_selector(move || format!("start-suggestion-{at}"))
                .py(px(4.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .border_1()
                .cursor_pointer()
                .border_color(if on {
                    theme::accent_border()
                } else {
                    theme::border_default()
                })
                .when(on, |chip| chip.bg(theme::bg_row_active()))
                .hover(|chip| chip.bg(theme::bg_hover()))
                .child(crate::theme::mono(
                    pattern,
                    10.5,
                    500.0,
                    if on {
                        theme::accent_bright()
                    } else {
                        theme::text_secondary()
                    },
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.use_pattern(chosen.clone(), window, cx);
                })),
        );
    }
    Some(row)
}

impl PwrRoute {
    /// The selected draft starts from the last link the person opened, when
    /// there is one; otherwise it stays blank with Start from offering
    /// nothing.
    pub fn start_from_last(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        match last_link(cx) {
            Some(link) => self.start_from(link.start(), window, cx),
            None => self.draft_link = Some((h, None)),
        }
    }

    /// The selected draft starts from `link`: its first pattern becomes the
    /// narrowest the link suggests (from the URL the handlers saw), selected
    /// so typing replaces it, and Try it holds the link as opened.
    pub fn start_from(&mut self, link: StartLink, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        let pattern = candidates(&link.seen)
            .into_iter()
            .next()
            .unwrap_or_default();
        // The sample first: the rebuild below keeps an edited sample, and
        // its Try it run is then the one that counts.
        if let Some(rail) = &mut self.rail {
            rail.menu = None;
            rail.sample.set(link.url.clone());
            rail.sample_edited = true;
        }
        self.draft_link = Some((h, Some(link)));
        self.set_first_pattern(&pattern, cx);
        self.focus_field(window, cx, |rail| rail.patterns.first());
        cx.notify();
    }

    /// A suggested pattern's chip: the first pattern becomes it.
    pub fn use_pattern(
        &mut self,
        pattern: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_first_pattern(&pattern, cx);
        self.focus_field(window, cx, |rail| rail.patterns.first());
        cx.notify();
    }

    /// Clear: the draft goes back to a blank rule. Its patterns, the link
    /// and Try it go; the browser and the rest stay.
    pub fn clear_start(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(selection @ Selection::Handler(h)) = self.selection else {
            return;
        };
        let count = self
            .handlers
            .get(h)
            .map_or(0, |handler| handler.patterns.len());
        for at in (1..count).rev() {
            self.store.remove_pattern(selection, at);
        }
        if count == 0 {
            self.store.add_pattern(selection);
        } else {
            self.store.set_pattern(selection, 0, "");
        }
        self.draft_link = Some((h, None));
        if let Some(rail) = &mut self.rail {
            rail.sample_edited = false;
        }
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        self.focus_field(window, cx, |rail| rail.patterns.first());
        cx.notify();
    }

    /// Writes the selected handler's first pattern, adding it when there is
    /// none, and selects the field's text.
    fn set_first_pattern(&mut self, pattern: &str, cx: &mut Context<'_, Self>) {
        let Some(selection @ Selection::Handler(h)) = self.selection else {
            return;
        };
        if self
            .handlers
            .get(h)
            .is_some_and(|handler| handler.patterns.is_empty())
        {
            self.store.add_pattern(selection);
        }
        self.store.set_pattern(selection, 0, pattern);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        if let Some(field) = self
            .rail
            .as_mut()
            .and_then(|rail| rail.patterns.first_mut())
        {
            field.select_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwrroute_router::{Link, Source, Step};

    fn record(id: u64, url: &str, privacy: Privacy) -> DispatchRecord {
        let mut record = crate::fixtures::activity()
            .into_iter()
            .find(|record| record.browser.is_some())
            .expect("a fixture record that opened");
        record.id = pwrroute_router::DispatchId(id);
        record.privacy = privacy;
        record.link = Link::new(url, Source::AppleEvent);
        record.steps = vec![Step::Received {
            url: url.to_owned(),
        }];
        record.outcome = Outcome::Opened;
        record.final_url = Some(url.to_owned());
        if let Some(browser) = &mut record.browser {
            "Microsoft Edge".clone_into(&mut browser.name);
            browser.profile.clear();
        }
        record
    }

    #[test]
    fn senders_count_one_app_once_by_bundle_id_or_name() {
        let from = |id: u64, name: &str, bundle_id: &str| {
            let mut record = record(id, "https://docs.example.com/a", Privacy::Full);
            record.link = record.link.with_opener(Some(pwrroute_router::OpenerInfo {
                name: name.into(),
                bundle_id: bundle_id.into(),
                ..pwrroute_router::OpenerInfo::default()
            }));
            record
        };
        let records = vec![
            from(4, "Slack", ""),
            from(3, "Mail", "com.apple.mail"),
            from(2, "Slack", "com.tinyspeck.slackmacgap"),
            from(1, "slack", "com.tinyspeck.slackmacgap"),
        ];
        let found = senders(&records, 8);
        assert_eq!(
            found
                .iter()
                .map(|s| (s.value.as_str(), s.name.as_str(), s.links))
                .collect::<Vec<_>>(),
            [
                ("com.tinyspeck.slackmacgap", "Slack", 3),
                ("com.apple.mail", "Mail", 1)
            ]
        );
    }

    #[test]
    fn recent_lists_full_links_newest_first_once_each() {
        let records = vec![
            record(
                5,
                "https://git.example.com/acme/widgets/pull/128",
                Privacy::Full,
            ),
            record(4, "https://docs.example.com/guides/setup", Privacy::Host),
            record(
                3,
                "https://git.example.com/acme/widgets/pull/128",
                Privacy::Full,
            ),
            record(2, "https://meet.example.com/abc", Privacy::Full),
            record(1, "https://hidden.example.com/", Privacy::None),
        ];
        let links = recent(&records, RECENT_LIMIT);
        let urls: Vec<&str> = links.iter().map(|link| link.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://git.example.com/acme/widgets/pull/128",
                "https://meet.example.com/abc",
            ]
        );
        assert_eq!(links[0].opened, "Microsoft Edge");
    }

    #[test]
    fn recent_skips_test_runs_and_stops_at_the_limit() {
        let mut tested = record(99, "https://tested.example.com/", Privacy::Full);
        tested.outcome = Outcome::Tested;
        let mut records = vec![tested];
        for id in 0..(RECENT_LIMIT as u64 + 5) {
            records.push(record(
                id,
                &format!("https://example.com/{id}"),
                Privacy::Full,
            ));
        }
        let links = recent(&records, RECENT_LIMIT);
        assert_eq!(links.len(), RECENT_LIMIT);
        assert_eq!(links[0].url, "https://example.com/0");
    }

    #[test]
    fn a_rewritten_link_is_suggested_as_the_handlers_saw_it() {
        let mut rewritten = record(1, "https://x.com/pwrdrvr/status/1", Privacy::Full);
        rewritten.final_url = Some("https://xcancel.com/pwrdrvr/status/1".into());
        let links = recent(&[rewritten], RECENT_LIMIT);
        assert_eq!(links[0].url, "https://x.com/pwrdrvr/status/1");
        assert_eq!(links[0].seen, "https://xcancel.com/pwrdrvr/status/1");
        assert_eq!(candidates(&links[0].seen)[0], "xcancel.com/pwrdrvr/*");
    }

    #[test]
    fn candidates_are_narrowest_first() {
        assert_eq!(
            candidates("https://git.example.com/acme/widgets/pull/128"),
            ["git.example.com/acme/*", "git.example.com/*"]
        );
    }
}
