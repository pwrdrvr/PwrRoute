//! Settings › General (design: Appearance and Themes 1a-1e): the
//! Appearance card, with Theme (System, Dark or Light), the Dark theme and
//! the Light theme (PWR-23), and the OS's Increase Contrast as PwrRoute
//! follows it. General came back for this; it sits after About, outside
//! the groups, as PwrAgent's and PwrGit's do.

use gpui_kit::base::actions::{SelectDown, SelectUp};
use gpui_kit::base::{Radio, Select, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, ElementId, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    anchored, deferred, div, point,
};

use super::SettingsWindow;
use super::configuration::trigger;
use super::controls::{
    Kind, Tone, button, chip, field, field_help, field_sub, field_with, heading, pane_head,
};
use super::model::SettingsRequest;
use super::style::para;
use crate::theme::{
    self, Appearance, DarkTheme, LightTheme, ThemeChoice, ThemeInfo, mono, px, text,
};
use crate::widgets::{field_label, menu_card, menu_heading, menu_row_with, segmented};

/// The Appearance card.
pub const APPEARANCE_ID: &str = "general-appearance";

/// The Theme control.
pub const THEME_ID: &str = "general-theme";

/// The line under Theme: what System resolves to, or what is locked.
pub const THEME_HELP_ID: &str = "general-theme-help";

/// The Dark theme and Light theme selects; their rows are
/// `(<id>-option, index)` in [`DarkTheme::ALL`] / [`LightTheme::ALL`]'s
/// order.
pub const DARK_THEME_ID: &str = "general-dark-theme";
pub const LIGHT_THEME_ID: &str = "general-light-theme";
pub const DARK_THEME_OPTION_ID: &str = "general-dark-theme-option";
pub const LIGHT_THEME_OPTION_ID: &str = "general-light-theme-option";

/// The offer of the other scheme's pair after a pick (design 1e), and its
/// two buttons.
pub const OFFER_ID: &str = "general-theme-offer";
pub const OFFER_USE_ID: &str = "general-theme-offer-use";
pub const OFFER_DISMISS_ID: &str = "general-theme-offer-dismiss";

/// The selects' width (design 1c).
const SELECT_WIDTH: f32 = 300.0;

/// One answer of the Theme control.
pub fn theme_id(choice: ThemeChoice) -> &'static str {
    match choice {
        ThemeChoice::System => "general-theme-system",
        ThemeChoice::Dark => "general-theme-dark",
        ThemeChoice::Light => "general-theme-light",
    }
}

/// The open theme list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Menu {
    scheme: Appearance,
    highlighted: usize,
}

/// Right after a pick, the other scheme's theme from the same palette,
/// offered once (design 1e). Nothing changes the other scheme unasked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Under the Dark theme: use this as the Light theme.
    Light(LightTheme),
    /// Under the Light theme: use this as the Dark theme.
    Dark(DarkTheme),
}

impl Offer {
    /// What a pick in `scheme`'s list offers, with `dark` and `light` the
    /// themes after it: the other scheme's theme from the same palette,
    /// unless it is already set.
    pub fn after_pick(scheme: Appearance, dark: DarkTheme, light: LightTheme) -> Option<Self> {
        match scheme {
            Appearance::Dark => dark.pair().filter(|pair| *pair != light).map(Self::Light),
            Appearance::Light => Some(light.pair())
                .filter(|pair| *pair != dark)
                .map(Self::Dark),
        }
    }
}

/// The pane's own view state: the open list and the offer. Neither is a
/// preference; both go when the pane does.
pub struct State {
    menu: Option<Menu>,
    dark_trigger: FocusHandle,
    light_trigger: FocusHandle,
    menu_focus: FocusHandle,
    offer: Option<Offer>,
}

impl State {
    pub fn new(cx: &mut Context<'_, SettingsWindow>) -> Self {
        Self {
            menu: None,
            dark_trigger: cx.focus_handle(),
            light_trigger: cx.focus_handle(),
            menu_focus: cx.focus_handle(),
            offer: None,
        }
    }

    /// Drops the open list and the offer.
    pub fn close(&mut self) {
        self.menu = None;
        self.offer = None;
    }

    /// The `screens` example's open list and offer.
    #[allow(dead_code, reason = "the screens example draws the list and the offer")]
    pub fn preview(&mut self, open: Option<Appearance>, offer: Option<Offer>) {
        self.menu = open.map(|scheme| Menu {
            scheme,
            highlighted: 0,
        });
        self.offer = offer;
    }

    /// The open list's scheme, for tests.
    #[cfg(test)]
    pub fn open_menu(&self) -> Option<Appearance> {
        self.menu.map(|menu| menu.scheme)
    }

    /// The offer on show, for tests.
    #[cfg(test)]
    pub fn offer(&self) -> Option<Offer> {
        self.offer
    }
}

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let general = &this.model.general;
    let theme_control = ThemeChoice::ALL
        .into_iter()
        .fold(segmented().id(THEME_ID), |control, choice| {
            control.child(theme_segment(choice, general.theme, cx))
        })
        .test_support();
    let theme_help = div()
        .id(THEME_HELP_ID)
        .child(field_help(general.theme_help()))
        .test_support();
    let contrast = if general.increase_contrast {
        "On"
    } else {
        "Off"
    };
    let rows = vec![
        field(
            "Theme",
            Some(field_sub(if cfg!(target_os = "macos") {
                "System follows macOS: PwrRoute turns light or dark when the Mac does, while it runs."
            } else {
                "System follows your desktop: PwrRoute turns light or dark when it does, while it runs."
            })),
            [
                theme_control.into_any_element(),
                theme_help.into_any_element(),
            ],
        ),
        scheme_row(this, Appearance::Dark, "Dark theme", cx),
        scheme_row(this, Appearance::Light, "Light theme", cx),
        field(
            "Increase Contrast",
            Some(field_sub(if cfg!(target_os = "macos") {
                "macOS\u{2019}s setting, in Accessibility \u{203a} Display."
            } else if cfg!(target_os = "windows") {
                "Windows\u{2019} contrast themes, in Accessibility."
            } else {
                "Your desktop\u{2019}s high contrast setting."
            })),
            [
                chip(
                    format!("{contrast} \u{00b7} follows {}", general.os),
                    Tone::Neutral,
                )
                .into_any_element(),
                field_help(
                    "Each theme has its own Increase Contrast variant: edges and quiet text get \
                     stronger, the theme\u{2019}s colors stay.",
                )
                .into_any_element(),
            ],
        ),
    ];
    div()
        .child(pane_head(
            "General",
            "General",
            "How PwrRoute looks, in every window at once.",
            None,
        ))
        .child(this.disclosure(
            APPEARANCE_ID,
            heading("Appearance", "Theme"),
            Some(chip(general.chip(), Tone::Neutral).into_any_element()),
            rows,
            cx,
        ))
}

/// The Dark theme or Light theme row: "Showing" beside the one drawn, the
/// select, and the offer of the other scheme's pair under it.
fn scheme_row(
    this: &SettingsWindow,
    scheme: Appearance,
    label: &'static str,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let general = &this.model.general;
    let select = theme_select(this, scheme, cx);
    let offer = this
        .general
        .offer
        .filter(|offer| {
            matches!(
                (scheme, offer),
                (Appearance::Dark, Offer::Light(_)) | (Appearance::Light, Offer::Dark(_))
            )
        })
        .map(|offer| offer_row(offer, cx).into_any_element());
    field_with(
        showing_label(label, general.showing == scheme),
        Some(para(
            general.theme_sub(scheme),
            11.0,
            400.0,
            1.5,
            theme::text_muted(),
        )),
        std::iter::once(select).chain(offer),
    )
}

/// `.seg span`, on gpui-kit's `Radio`: one answer of Theme. The window
/// flips at once; the producer saves, applies and reports back.
fn theme_segment(
    choice: ThemeChoice,
    current: ThemeChoice,
    cx: &mut Context<'_, SettingsWindow>,
) -> Radio {
    let selected = choice == current;
    let set = cx.listener(move |this, _: &bool, window, cx| this.set_theme(choice, window, cx));
    Radio::new(theme_id(choice))
        .checked(selected)
        .accessibility_label(choice.label())
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
            choice.label(),
            11.0,
            550.0,
            if selected {
                theme::accent_bright()
            } else {
                theme::text_secondary()
            },
        ))
}

/// A field's label, with `.showing` beside the scheme being drawn.
fn showing_label(label: &'static str, showing: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(para(label, 12.0, 600.0, 1.3, theme::text_primary()))
        .when(showing, |row| {
            row.child(
                div()
                    .flex_none()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded_full()
                    .bg(theme::accent_soft())
                    .child(text("Showing", 9.0, 600.0, theme::accent_bright())),
            )
        })
}

/// A theme's swatch: canvas, sidebar, accent, text, as PwrAgent's
/// `.theme-swatch` draws them.
fn swatch(colors: [Hsla; 4]) -> Div {
    colors.into_iter().fold(
        div()
            .flex()
            .flex_none()
            .overflow_hidden()
            .border_1()
            .border_color(theme::border_strong())
            .rounded(px(3.0)),
        |row, color| row.child(div().w(px(7.0)).h(px(12.0)).bg(color)),
    )
}

/// The themes of `scheme`'s list, in order, with their swatches.
fn themes(scheme: Appearance) -> Vec<(&'static ThemeInfo, [Hsla; 4])> {
    match scheme {
        Appearance::Dark => DarkTheme::ALL
            .into_iter()
            .map(|theme| (theme.info(), theme.palette(false).swatch()))
            .collect(),
        Appearance::Light => LightTheme::ALL
            .into_iter()
            .map(|theme| (theme.info(), theme.palette(false).swatch()))
            .collect(),
    }
}

/// Which row of `scheme`'s list is chosen.
fn chosen(this: &SettingsWindow, scheme: Appearance) -> usize {
    let general = &this.model.general;
    match scheme {
        Appearance::Dark => DarkTheme::ALL
            .iter()
            .position(|theme| *theme == general.dark_theme),
        Appearance::Light => LightTheme::ALL
            .iter()
            .position(|theme| *theme == general.light_theme),
    }
    .unwrap_or(0)
}

/// The closed select's hint: who the theme is from.
fn origin(index: usize, info: &ThemeInfo) -> &'static str {
    match (index, info.credit) {
        (0, _) => "Pwr family default",
        (_, None) => "Pwr family",
        (_, Some(_)) => "community",
    }
}

/// The Dark theme or Light theme select: gpui-kit's `Select` around an
/// `.inp` trigger with the swatch, the name and where it is from, and the
/// list under it, the Pwr family's first, then the community palettes.
fn theme_select(
    this: &SettingsWindow,
    scheme: Appearance,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let (id, option_id, label, focus) = match scheme {
        Appearance::Dark => (
            DARK_THEME_ID,
            DARK_THEME_OPTION_ID,
            "Dark theme",
            &this.general.dark_trigger,
        ),
        Appearance::Light => (
            LIGHT_THEME_ID,
            LIGHT_THEME_OPTION_ID,
            "Light theme",
            &this.general.light_trigger,
        ),
    };
    let themes = themes(scheme);
    let current = chosen(this, scheme);
    let (info, colors) = themes[current];
    let open = this.general.menu.is_some_and(|menu| menu.scheme == scheme);
    let lit = this
        .general
        .menu
        .filter(|menu| menu.scheme == scheme)
        .map(|menu| menu.highlighted);
    let content = div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(8.0))
        .child(swatch(colors))
        .child(field_label(info.name, theme::text_primary()))
        .child(div().ml_auto().child(mono(
            origin(current, info),
            10.5,
            500.0,
            theme::text_muted(),
        )));
    let trigger = trigger(
        ElementId::Name(format!("{id}-trigger").into()),
        open,
        SELECT_WIDTH,
        content,
    )
    .map(crate::accessibility::clickable)
    .on_click(cx.listener(move |this, _, _, cx| {
        this.open_theme_menu(scheme, !open, cx);
    }));
    let menu = open.then(|| {
        let mut card = menu_card(ElementId::Name(format!("{id}-menu").into()))
            .existing_navigation()
            .track_focus(&this.general.menu_focus)
            .w(px(SELECT_WIDTH))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.general.menu = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| {
                this.move_theme_highlight(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| {
                this.move_theme_highlight(1, cx);
            }))
            .child(menu_heading("Pwr family"));
        let mut community = false;
        for (index, (info, colors)) in themes.iter().enumerate() {
            if info.credit.is_some() && !community {
                community = true;
                card = card
                    .child(
                        div()
                            .h(px(1.0))
                            .mx(px(6.0))
                            .my(px(4.0))
                            .bg(theme::border_subtle()),
                    )
                    .child(menu_heading("Community palettes"));
            }
            let hint = (!info.hint.is_empty()).then(|| SharedString::from(info.hint));
            let row = menu_row_with(
                (option_id, index),
                Some(swatch(*colors).into_any_element()),
                info.name,
                hint,
            )
            .child(
                div()
                    .w(px(12.0))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .when(index == current, |cell| {
                        cell.child(text("\u{2713}", 11.0, 600.0, theme::accent_bright()))
                    }),
            );
            card = card.child(option_row(row, scheme, index, lit == Some(index), cx));
        }
        card
    });
    let open_change = cx.listener(move |this, open: &bool, _, cx| {
        this.open_theme_menu(scheme, *open, cx);
    });
    let confirm = cx.listener(move |this, _: &(), window, cx| {
        if let Some(menu) = this.general.menu {
            this.choose_theme(menu.scheme, menu.highlighted, window, cx);
        }
    });
    Select::new(id)
        .open(open)
        .accessibility_label(label)
        .accessibility_value(info.name)
        .on_open_change(move |open, window, cx| open_change(&open, window, cx))
        .on_confirm(move |window, cx| confirm(&(), window, cx))
        .focus_handle(focus)
        .content_focus_handle(&this.general.menu_focus)
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
    scheme: Appearance,
    index: usize,
    highlighted: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    row.when(highlighted, |row| row.bg(theme::bg_hover_strong()))
        .hover(|style| style.bg(theme::bg_hover()))
        .on_mouse_move(cx.listener(move |this, _, _, cx| {
            if let Some(menu) = this.general.menu.as_mut()
                && menu.highlighted != index
            {
                menu.highlighted = index;
                cx.notify();
            }
        }))
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.choose_theme(scheme, index, window, cx);
        }))
        .test_support()
        .into_any_element()
}

/// `.offer`: "Use Catppuccin Latte when PwrRoute is light?", Use it, Not
/// now.
fn offer_row(offer: Offer, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let question = match offer {
        Offer::Light(theme) => format!("Use {} when PwrRoute is light?", theme.info().name),
        Offer::Dark(theme) => format!("Use {} when PwrRoute is dark?", theme.info().name),
    };
    div()
        .id(OFFER_ID)
        .flex()
        .items_center()
        .gap(px(8.0))
        .py(px(7.0))
        .pl(px(10.0))
        .pr(px(8.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .bg(theme::bg_elevated())
        .child(
            div()
                .mr(px(4.0))
                .child(para(question, 11.0, 400.0, 1.4, theme::text_secondary())),
        )
        .child(
            button(OFFER_USE_ID, "Use it", Kind::Default, true).on_click(cx.listener(
                |this, _, window, cx| {
                    this.accept_offer(window, cx);
                },
            )),
        )
        .child(
            button(OFFER_DISMISS_ID, "Not now", Kind::Quiet, true).on_click(cx.listener(
                |this, _, _, cx| {
                    this.general.offer = None;
                    cx.notify();
                },
            )),
        )
        .test_support()
}

impl SettingsWindow {
    fn set_theme(&mut self, choice: ThemeChoice, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.general.offer = None;
        let general = &mut self.model.general;
        if general.theme == choice {
            return;
        }
        general.theme = choice;
        if choice != ThemeChoice::System {
            general.showing = choice.resolve(general.showing);
        }
        self.request(SettingsRequest::SetTheme(choice), window, cx);
        cx.notify();
    }

    fn open_theme_menu(&mut self, scheme: Appearance, open: bool, cx: &mut Context<'_, Self>) {
        self.general.offer = None;
        self.general.menu = open.then(|| Menu {
            scheme,
            highlighted: chosen(self, scheme),
        });
        cx.notify();
    }

    fn move_theme_highlight(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let Some(menu) = self.general.menu.as_mut() else {
            return;
        };
        let last = match menu.scheme {
            Appearance::Dark => DarkTheme::ALL.len(),
            Appearance::Light => LightTheme::ALL.len(),
        } - 1;
        menu.highlighted = menu.highlighted.saturating_add_signed(step).min(last);
        cx.notify();
    }

    /// Picks row `index` of `scheme`'s list. The window flips at once and
    /// offers the other scheme's pair when it differs; the producer saves
    /// and applies.
    pub(crate) fn choose_theme(
        &mut self,
        scheme: Appearance,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.general.menu = None;
        let trigger = match scheme {
            Appearance::Dark => self.general.dark_trigger.clone(),
            Appearance::Light => self.general.light_trigger.clone(),
        };
        window.focus(&trigger, cx);
        let general = &mut self.model.general;
        let request = match scheme {
            Appearance::Dark => DarkTheme::ALL
                .get(index)
                .filter(|picked| **picked != general.dark_theme)
                .map(|&picked| {
                    general.dark_theme = picked;
                    SettingsRequest::SetDarkTheme(picked)
                }),
            Appearance::Light => LightTheme::ALL
                .get(index)
                .filter(|picked| **picked != general.light_theme)
                .map(|&picked| {
                    general.light_theme = picked;
                    SettingsRequest::SetLightTheme(picked)
                }),
        };
        // The list closed either way; a pick of the theme in force (or a
        // row the list does not have) changes nothing else.
        let Some(request) = request else {
            cx.notify();
            return;
        };
        self.general.offer = Offer::after_pick(scheme, general.dark_theme, general.light_theme);
        self.request(request, window, cx);
        cx.notify();
    }

    /// Use it: the other scheme takes the offered theme.
    fn accept_offer(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(offer) = self.general.offer.take() else {
            return;
        };
        let general = &mut self.model.general;
        let request = match offer {
            Offer::Light(theme) => {
                general.light_theme = theme;
                SettingsRequest::SetLightTheme(theme)
            }
            Offer::Dark(theme) => {
                general.dark_theme = theme;
                SettingsRequest::SetDarkTheme(theme)
            }
        };
        self.request(request, window, cx);
        cx.notify();
    }
}
