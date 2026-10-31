//! Settings › Short Links (Settings Sections 1c, 1d): the hosts whose links
//! are followed to where they land before the rules run, whose list it
//! is, and how a link is followed. It reads the config; it never edits it.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div};

use super::SettingsWindow;
use super::controls::{Tone, chip, field, field_help, field_sub_code, pane_head, tag};
use super::model::ShortLinksSource;
use super::style::para;
use crate::theme::{self, px};

pub const HOSTS_ID: &str = "short-links-hosts";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "Routing",
            "Short Links",
            "Links from a shortener are followed to where they land, so your rules match the \
             real site and Activity shows both.",
            None,
        ))
        .child(hosts_card(this, cx))
        .child(limits_card(this, cx))
}

fn hosts_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let short = &this.model.configuration.short_links;
    let hosts = short
        .hosts
        .iter()
        .fold(div().flex().flex_wrap().gap(px(6.0)), |hosts, host| {
            hosts.child(tag(host.clone()))
        });
    let help = "A Finicky config can set its own list with urlShorteners; an empty list turns \
                expansion off.";
    let row = match &short.source {
        ShortLinksSource::BuiltIn { config } => {
            let sub = match config {
                Some(config) => field_sub_code(
                    &format!("Finicky\u{2019}s list. {config} does not set "),
                    "urlShorteners",
                    ", so this one applies.",
                ),
                None => para(
                    "Finicky\u{2019}s list. With rules.json alone, this one applies.",
                    11.0,
                    400.0,
                    1.5,
                    theme::text_muted(),
                ),
            };
            field(
                "Built-in list",
                Some(sub),
                [
                    hosts.into_any_element(),
                    field_help(help).into_any_element(),
                ],
            )
        }
        ShortLinksSource::Config(config) => field(
            format!("From {config}"),
            Some(field_sub_code(
                "",
                "urlShorteners",
                " replaces the built-in list.",
            )),
            [
                hosts.into_any_element(),
                field_help(help).into_any_element(),
            ],
        ),
        ShortLinksSource::Off(config) => field(
            "Off",
            Some(field_sub_code(
                &format!("{config} sets "),
                "urlShorteners: []",
                ".",
            )),
            [field_help(
                "Short links are routed as they are: rules see the shortener\u{2019}s host, \
                 not where it leads, and nothing leaves this Mac before a rule sees it.",
            )
            .into_any_element()],
        ),
    };
    let on = !matches!(short.source, ShortLinksSource::Off(_));
    let state = if on {
        chip("On", Tone::Ok)
    } else {
        chip("Off", Tone::Neutral)
    };
    this.disclosure(
        HOSTS_ID,
        super::controls::heading("Expanded before matching", "Shortener hosts"),
        Some(state.into_any_element()),
        vec![row],
        cx,
    )
}

fn limits_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let words = |text: &'static str| -> SharedString { text.into() };
    let rows = vec![
        field(
            "Each link",
            None,
            [field_help(words(
                "HEAD first, then GET if that does not land; up to 3 redirects, 750 ms each. \
                 If a shortener does not answer in time, the short link itself is routed.",
            ))
            .into_any_element()],
        ),
        field(
            "What leaves this Mac",
            None,
            [field_help(words(
                "Only the short link, to the shortener\u{2019}s own host, before a rule sees \
                 it. Nothing about it is logged.",
            ))
            .into_any_element()],
        ),
    ];
    this.disclosure(
        "short-links-limits",
        super::controls::heading("How it follows a link", "Limits"),
        None,
        rows,
        cx,
    )
}
