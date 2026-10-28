//! The design's small controls: chips, tags, badges, buttons, inputs and
//! switches. Each mirrors one class in `design/pwrroute.css`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img, px,
};

use crate::theme::{self, icon, mono, text};

/// `.tag`: a mono chip for a pattern or identifier. Never wider than its
/// container: a long pattern or profile name ends in an ellipsis.
pub fn tag(label: impl Into<SharedString>) -> Div {
    tag_frame().max_w_full().overflow_hidden().child(one_line(
        label,
        theme::mono_family(500.0),
        500.0,
        10.5,
        10.5,
        theme::text_secondary(),
    ))
}

/// `.tag.re`: a regex pattern.
pub fn regex_tag(label: impl Into<SharedString>) -> Div {
    tag_frame()
        .bg(theme::info_soft())
        .border_color(theme::transparent())
        .child(mono(label, 10.5, 500.0, theme::info_text()))
}

/// `.tag.fn`: a JS function matcher or transformer.
pub fn fn_tag() -> Div {
    tag_frame()
        .bg(theme::accent_soft())
        .border_color(theme::transparent())
        .child(mono("fn", 10.5, 500.0, theme::accent_bright()))
}

/// `.tag.app`: an app, with its badge.
pub fn app_tag(badge_label: impl Into<AppBadge>, name: impl Into<SharedString>) -> Div {
    tag_frame()
        .child(badge(badge_label))
        .child(mono(name, 10.5, 500.0, theme::text_primary()))
}

/// `.tag.act`: a rewrite action, `kind` dimmed before the value.
pub fn action_tag(kind: impl Into<SharedString>, value: impl Into<SharedString>) -> Div {
    tag_frame()
        .child(mono(kind, 10.5, 500.0, theme::text_subtle()))
        .child(mono(value, 10.5, 500.0, theme::accent_bright()))
}

/// The box every `.tag` shares.
pub fn tag_frame() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .h(px(20.0))
        .px(px(7.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(6.0))
        .bg(theme::bg_elevated())
}

/// A browser or app's `.bi` badge: the app's own icon, as Finder shows it,
/// once it has loaded, else its two letters (`app_icons.rs`). [`badge_for`]
/// makes one for an app; a plain string is a fixed label that never becomes
/// an icon ([`NOT_OPENED_BADGE`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AppBadge {
    letters: SharedString,
    /// The app, as `badge_for` was given it: a name, a bundle id or a path.
    app: Option<SharedString>,
}

impl AppBadge {
    /// `app`'s badge with the `letters` given, not the ones [`badge_for`]
    /// would pick: Test a link's example senders.
    pub fn new(letters: impl Into<SharedString>, app: impl Into<SharedString>) -> Self {
        Self {
            letters: letters.into(),
            app: Some(app.into()),
        }
    }

    /// The two letters, which the badge draws when there is no icon.
    pub fn letters(&self) -> &SharedString {
        &self.letters
    }

    /// These letters for `app` when they name no app yet: a label built
    /// from fixed letters and a name.
    pub fn or_app(self, app: &str) -> Self {
        let app = app.trim();
        if self.app.is_some() || app.is_empty() {
            return self;
        }
        Self {
            app: Some(app.to_owned().into()),
            ..self
        }
    }

    /// The same letters that never become an icon: a target drawn as not
    /// installed.
    pub fn without_icon(&self) -> Self {
        Self::from(self.letters.clone())
    }
}

impl From<SharedString> for AppBadge {
    fn from(letters: SharedString) -> Self {
        Self { letters, app: None }
    }
}

impl From<&str> for AppBadge {
    fn from(letters: &str) -> Self {
        Self::from(SharedString::from(letters.to_owned()))
    }
}

impl From<String> for AppBadge {
    fn from(letters: String) -> Self {
        Self::from(SharedString::from(letters))
    }
}

impl From<&AppBadge> for AppBadge {
    fn from(badge: &AppBadge) -> Self {
        badge.clone()
    }
}

impl PartialEq<str> for AppBadge {
    fn eq(&self, letters: &str) -> bool {
        self.letters == letters
    }
}

impl PartialEq<&str> for AppBadge {
    fn eq(&self, letters: &&str) -> bool {
        self.letters == *letters
    }
}

/// How far an icon reaches past its badge's box on each side. Finder's icons
/// keep a transparent margin, so a 16px icon's art covers about a 12px
/// badge, and the layout around it is the same as with the letters.
const ICON_BLEED: f32 = 2.0;

/// `.bi`: a browser or app's 12px badge, its icon or its letters.
pub fn badge(label: impl Into<AppBadge>) -> Div {
    icon_or(&label.into(), 12.0, letter_badge)
}

/// `badge`'s app icon, centered over a `size` box, or `letters` drawn from
/// its letters when there is no icon (or while it loads). The box keeps its
/// size either way.
pub fn icon_or(
    badge: &AppBadge,
    size: f32,
    letters: impl Fn(SharedString) -> Div + Clone + 'static,
) -> Div {
    let Some(app) = badge.app.clone() else {
        return letters(badge.letters.clone());
    };
    let icon = size + 2.0 * ICON_BLEED;
    let fallback = {
        let label = badge.letters.clone();
        let app = app.clone();
        move || {
            let app = app.clone();
            letters(label.clone())
                .debug_selector(move || format!("app-letters-{app}"))
                .into_any_element()
        }
    };
    let selector = app.clone();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .child(
            img(crate::app_icons::source(app, icon))
                .debug_selector(move || format!("app-icon-{selector}"))
                .flex_none()
                .size(px(icon))
                // Centers the letters when they stand in.
                .flex()
                .items_center()
                .justify_center()
                .with_loading(fallback.clone())
                .with_fallback(fallback),
        )
}

/// `.bi`'s two letters in their 12px box.
fn letter_badge(label: SharedString) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(12.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(theme::border_strong())
        .font_family(theme::mono_family(700.0))
        .font_weight(theme::weight(700.0))
        .text_size(px(7.0))
        .line_height(px(7.0))
        .text_color(theme::text_muted())
        .child(label)
}

/// A `.chip`'s colors.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Plain,
    /// `.chip.warn`
    Warn,
    /// `.chip.acc`: a state the user chose (a send).
    Accent,
}

/// `.chip`: a count or state in a card header.
pub fn chip(label: impl Into<SharedString>) -> Div {
    toned_chip(label, Tone::Plain)
}

/// `.chip` in one of its tones.
pub fn toned_chip(label: impl Into<SharedString>, tone: Tone) -> Div {
    let (background, border, foreground) = match tone {
        Tone::Plain => (
            theme::bg_elevated(),
            theme::border_subtle(),
            theme::text_muted(),
        ),
        Tone::Warn => (theme::warn_soft(), theme::transparent(), theme::warn_text()),
        Tone::Accent => (
            theme::accent_soft(),
            theme::transparent(),
            theme::accent_bright(),
        ),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(9.0))
        .py(px(4.0))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(background)
        .child(text(label, 10.0, 600.0, foreground))
}

/// `.btn` with a plus icon; `.btn.primary` when `primary`.
pub fn add_button(label: &'static str, primary: bool) -> Div {
    let kind = if primary {
        ButtonKind::Primary
    } else {
        ButtonKind::Default
    };
    let foreground = kind.foreground();
    button_frame(kind, false)
        .child(icon("pwrroute/plus.svg", 13.0, foreground))
        .child(text(label, 11.5, 550.0, foreground))
}

/// Which `.btn` variant.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Default,
    /// `.btn.primary`
    Primary,
    /// `.btn.quiet`
    Quiet,
    /// `.btn.quiet.danger`
    Danger,
}

impl ButtonKind {
    fn foreground(self) -> Hsla {
        match self {
            Self::Default => theme::text_primary(),
            Self::Primary => theme::accent_on(),
            Self::Quiet => theme::text_secondary(),
            Self::Danger => theme::danger_text(),
        }
    }
}

fn button_frame(kind: ButtonKind, small: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .map(|button| {
            if small {
                button.py(px(5.0)).px(px(9.0))
            } else {
                button.py(px(7.0)).px(px(12.0))
            }
        })
        .border_1()
        .rounded(px(7.0))
        .cursor_pointer()
        .map(|button| match kind {
            ButtonKind::Primary => button
                .bg(theme::accent())
                .border_color(theme::transparent()),
            ButtonKind::Default => button
                .bg(theme::bg_elevated())
                .border_color(theme::border_default()),
            ButtonKind::Quiet | ButtonKind::Danger => button
                .bg(theme::transparent())
                .border_color(theme::transparent()),
        })
}

/// `.btn`, `.btn.sm` when `small`.
pub fn button(label: impl Into<SharedString>, kind: ButtonKind, small: bool) -> Div {
    let size = if small { 11.0 } else { 11.5 };
    button_frame(kind, small).child(text(label, size, 550.0, kind.foreground()))
}

/// `.inp`: a field's box. Content goes in as children.
pub fn input_box() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(30.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .overflow_hidden()
}

/// `.inp.focus`. GPUI has no `outline`, so the 2px ring with its 1px offset
/// is a border around the box, pulled out by a negative margin so it takes
/// no layout space.
pub fn focus_ring(input: Div) -> Div {
    div()
        .m(px(-3.0))
        .p(px(1.0))
        .border_2()
        .border_color(theme::accent())
        .rounded(px(10.0))
        .child(input)
}

/// `.caret`: a select's ▾.
pub fn caret() -> Div {
    div()
        .ml_auto()
        .child(text("\u{25be}", 10.0, 400.0, theme::text_subtle()))
}

/// `.sw`: a switch.
pub fn switch(on: bool) -> Div {
    let (track, border, knob): (Hsla, Hsla, Hsla) = if on {
        (theme::accent(), theme::transparent(), theme::accent_on())
    } else {
        (
            theme::bg_hover_strong(),
            theme::border_default(),
            theme::text_muted(),
        )
    };
    div()
        .relative()
        .flex_none()
        .w(px(30.0))
        .h(px(18.0))
        .rounded_full()
        .border_1()
        .bg(track)
        .border_color(border)
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(if on { 14.0 } else { 2.0 }))
                .size(px(12.0))
                .rounded_full()
                .bg(knob),
        )
}

/// `.seg`: a segmented control's frame. Segments go in as children.
pub fn segmented() -> Div {
    div()
        .flex()
        .flex_none()
        .gap(px(1.0))
        .p(px(2.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
}

/// `.seg span`; `.on` when `selected`.
pub fn segment(label: impl Into<SharedString>, selected: bool) -> Div {
    let foreground = if selected {
        theme::accent_bright()
    } else {
        theme::text_secondary()
    };
    div()
        .py(px(5.0))
        .px(px(10.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .when(selected, |segment| segment.bg(theme::bg_row_active()))
        .child(text(label, 11.0, 550.0, foreground))
}

/// `.strip-chip`: a status chip in the title strip, with its dot.
pub fn status_chip(label: impl Into<SharedString>, dot: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(24.0))
        .px(px(9.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded_full()
        .bg(theme::bg_elevated())
        .child(theme::dot(dot))
        .child(mono(label, 10.5, 600.0, theme::text_muted()))
}

/// `.st`: a status dot and its word.
pub fn status(label: impl Into<SharedString>, dot: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .child(theme::dot(dot))
        .child(text(label, 11.0, 500.0, theme::text_secondary()))
}

/// `.kbd`: a key cap.
pub fn key_cap(label: &'static str) -> Div {
    key_cap_in(
        label,
        18.0,
        10.0,
        theme::border_default(),
        theme::text_muted(),
    )
}

/// `.kbd` at another height and in other colours: on a primary button.
pub fn key_cap_in(label: &'static str, height: f32, size: f32, border: Hsla, color: Hsla) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(height))
        .px(px(5.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .child(mono(label, size, 600.0, color))
}

/// A wrapping run of sans text: CSS `font: <weight> <size>/<line-height>`.
pub fn paragraph(
    content: impl Into<SharedString>,
    size: f32,
    line_height: f32,
    color: Hsla,
) -> Div {
    div()
        .font_family(theme::SANS)
        .font_weight(theme::weight(400.0))
        .text_size(px(size))
        .line_height(px(line_height))
        .text_color(color)
        .child(content.into())
}

/// `.note`: small subtle help text.
pub fn note(content: impl Into<SharedString>) -> Div {
    paragraph(content, 10.5, 15.75, theme::text_subtle())
}

/// How a `browser: null` destination reads: the link is not opened.
pub const NOT_OPENED: &str = "Not opened";
pub const NOT_OPENED_BADGE: &str = "\u{2014}";

/// One line of text that ends in an ellipsis when its box is too narrow.
pub fn one_line(
    content: impl Into<SharedString>,
    family: &'static str,
    css_weight: f32,
    size: f32,
    line_height: f32,
    color: Hsla,
) -> Div {
    clip_x(content.into(), size, line_height)
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family(family)
        .font_weight(theme::weight(css_weight))
        .text_size(px(size))
        .line_height(px(line_height))
        .text_color(color)
}

/// Clips `content` at its sides but not at its top and bottom, so the
/// ellipsized line keeps its descenders. Lay it out as `content` would be:
/// it takes the same space, and text styles set on it reach the text.
///
/// GPUI centers a line's ascent plus descent in its line height, and that
/// is 1.3em in Geist Mono (1.14em in Geist), so text whose line height is
/// its size draws its g, p and y below its box. `overflow_hidden` would cut
/// them off there, and so would `overflow_x_hidden`, which also clips at
/// the box's top and bottom. Here the clip is a box taller by the overhang
/// on each side, pulled back by a negative margin.
pub fn clip_x(content: impl IntoElement, size: f32, line_height: f32) -> Div {
    let slack = px(glyph_overhang(size, line_height));
    div().min_w_0().flex().flex_col().child(
        div()
            .min_w_0()
            .my(-slack)
            .py(slack)
            .overflow_hidden()
            .child(content),
    )
}

/// How far glyphs can reach past a line `line_height` tall, plus a pixel
/// for antialiasing.
fn glyph_overhang(size: f32, line_height: f32) -> f32 {
    ((size * 1.3 - line_height) / 2.0).max(0.0) + 1.0
}

/// A select field's value: one line of 12.5px text that ends in an
/// ellipsis rather than pushing the caret out of the `.inp`.
pub fn field_label(content: impl Into<SharedString>, color: Hsla) -> Div {
    one_line(content, theme::SANS, 400.0, 12.5, 12.5, color)
}

/// A group's heading inside a `.menu`: "On this Mac", "In your rules".
pub fn menu_heading(title: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.0))
        .pt(px(8.0))
        .pb(px(3.0))
        .child(text(title, 10.0, 600.0, theme::text_subtle()))
}

/// The rule between a `.menu`'s choices and its way out ("Other app…").
pub fn menu_divider() -> Div {
    div()
        .h(px(1.0))
        .my(px(4.0))
        .mx(px(4.0))
        .bg(theme::border_subtle())
}

/// The tallest a `.menu` gets before it scrolls: 8½ rows of `.mi`, so the
/// half row says there is more (design: Popovers and Overflow 1c).
pub const MENU_MAX_HEIGHT: f32 = 280.0;

/// `.menu`: the popover card a select or a "⋯" button opens. Rows go in as
/// children; past [`MENU_MAX_HEIGHT`] the card scrolls. The caller places
/// it and gives it a width: a select's list is as wide as its field
/// (`.absolute().left_0().right_0()`), never narrower.
pub fn menu_card(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .flex()
        .flex_col()
        .max_h(px(MENU_MAX_HEIGHT))
        .overflow_y_scroll()
        .p(px(4.0))
        .bg(theme::bg_elevated())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .shadow_lg()
}

/// `.mi`: one menu row. The badge and the label come first; the dimmed mono
/// `.ex` hint on the right takes what is left and gives way first, ending
/// in an ellipsis at the card's edge, and the label ends in one only when
/// it alone does not fit (design: Popovers and Overflow 1a, 1d).
pub fn menu_row(
    id: impl Into<ElementId>,
    badge_label: Option<AppBadge>,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
) -> Stateful<Div> {
    let id = id.into();
    let hint_id = id.clone();
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .min_w_0()
        .overflow_hidden()
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .when_some(badge_label, |row, label| row.child(badge(label)))
        .child(one_line(
            label,
            theme::SANS,
            500.0,
            12.0,
            14.4,
            theme::text_primary(),
        ))
        .when_some(hint, |row, hint| {
            row.child(
                div()
                    .debug_selector(move || format!("{hint_id}-hint"))
                    .ml_auto()
                    .pl(px(16.0))
                    // Its padding, so the row counts it when the label
                    // takes the rest.
                    .min_w(px(16.0))
                    // The hint shrinks long before the label does.
                    .flex_shrink(10_000.0)
                    .child(one_line(
                        hint,
                        theme::mono_family(500.0),
                        500.0,
                        10.5,
                        10.5,
                        theme::text_muted(),
                    )),
            )
        })
}

/// A browser row's hint in a select: one or two profiles by name, three or
/// more as a count; the profile select lists every one (design: Popovers
/// and Overflow 1a).
pub fn profiles_hint(profiles: &[String]) -> Option<String> {
    match profiles {
        [] => None,
        [one] => Some(one.clone()),
        [one, two] => Some(format!("{one} \u{00b7} {two}")),
        many => Some(format!("{} profiles", many.len())),
    }
}

/// The badge for a browser or app: its icon, with the letters the design
/// draws for it when there is none.
pub fn badge_for(name: &str) -> AppBadge {
    let app = name.trim();
    AppBadge {
        letters: badge_letters(name),
        app: (!app.is_empty()).then(|| SharedString::from(app.to_owned())),
    }
}

/// The badge letters for a browser or app, as the design draws them.
fn badge_letters(name: &str) -> SharedString {
    const KNOWN: &[(&str, &str)] = &[
        ("chrome", "Ch"),
        ("firefox", "Ff"),
        ("arc", "Ar"),
        ("safari", "Sf"),
        ("zoom", "Zm"),
        ("figma", "Fg"),
        ("slack", "Sl"),
        ("edge", "Ed"),
        ("edgemac", "Ed"),
        ("brave", "Br"),
        ("mail", "Ml"),
        ("ical", "Ca"),
        ("calendar", "Ca"),
        ("ghostty", "Gh"),
    ];
    // Whole words (`Google Chrome`, `com.apple.Safari`), so a short key
    // like "arc" does not match inside "Search" or "archiver". A long key
    // also matches a word it starts (`firefoxdeveloperedition`).
    let lower = name.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let known = KNOWN.iter().find(|(key, _)| {
        words
            .iter()
            .any(|word| word == key || (key.len() >= 5 && word.starts_with(key)))
    });
    if let Some((_, label)) = known {
        return (*label).into();
    }
    let last = name.rsplit('.').next().unwrap_or(name);
    let mut letters = last.chars().filter(|c| c.is_alphanumeric());
    let first = letters.next().map(|c| c.to_ascii_uppercase());
    let second = letters.next().map(|c| c.to_ascii_lowercase());
    first.into_iter().chain(second).collect::<String>().into()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{
        Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, div,
    };

    use super::{AppBadge, NOT_OPENED_BADGE, badge, badge_for, one_line, profiles_hint};

    #[test]
    fn a_badge_names_its_app_and_a_fixed_label_names_none() {
        let chrome = badge_for(" Google Chrome ");
        assert_eq!(chrome, "Ch");
        assert_eq!(chrome.app.as_deref(), Some("Google Chrome"));
        assert_eq!(badge_for("  ").app, None);
        assert_eq!(AppBadge::from(NOT_OPENED_BADGE).app, None);
        assert_eq!(chrome.without_icon().app, None);
        assert_eq!(chrome.without_icon(), "Ch");
        // Bare letters take the name they are shown with; a badge that
        // names its app keeps it.
        assert_eq!(
            AppBadge::from("Ch").or_app("Google Chrome").app.as_deref(),
            Some("Google Chrome")
        );
        assert_eq!(
            badge_for("com.google.Chrome")
                .or_app("Google Chrome")
                .app
                .as_deref(),
            Some("com.google.Chrome")
        );
        let slack = AppBadge::new("Sl", "com.tinyspeck.slackmacgap");
        assert_eq!(
            (slack.letters().as_ref(), slack.app.as_deref()),
            ("Sl", Some("com.tinyspeck.slackmacgap"))
        );
    }

    /// A row of badges.
    struct Badges(Vec<AppBadge>);

    impl Render for Badges {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .flex()
                .children(self.0.iter().map(|b| badge(b.clone())))
        }
    }

    fn open(cx: &mut TestAppContext) -> &mut VisualTestContext {
        let (_, cx) = cx.add_window_view(|_, _| {
            Badges(vec![
                badge_for("com.apple.finder"),
                badge_for("com.example.no-such-app"),
            ])
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx
    }

    #[gpui_kit::test]
    fn without_icons_every_badge_is_letters(cx: &mut TestAppContext) {
        let cx = open(cx);
        assert!(cx.debug_bounds("app-letters-com.apple.finder").is_some());
        assert!(
            cx.debug_bounds("app-letters-com.example.no-such-app")
                .is_some()
        );
    }

    #[gpui_kit::test]
    fn an_installed_app_draws_its_icon_and_a_missing_one_its_letters(cx: &mut TestAppContext) {
        cx.update(crate::app_icons::install);
        let cx = open(cx);
        // The icon loads off the UI thread, then the view draws again.
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let finder = cx
            .debug_bounds("app-icon-com.apple.finder")
            .expect("Finder's badge");
        // The badge keeps its 12px box; the icon is 16px around it.
        assert_eq!(finder.size.width, gpui_kit::px(16.0));
        assert_eq!(
            cx.debug_bounds("app-letters-com.apple.finder").is_none(),
            cfg!(target_os = "macos"),
            "Finder's icon on macOS, its letters elsewhere"
        );
        assert!(
            cx.debug_bounds("app-letters-com.example.no-such-app")
                .is_some()
        );

        // A rescan drops what was read and reads it again.
        cx.update(|_, cx| crate::app_icons::forget(cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("app-letters-com.apple.finder").is_none(),
            cfg!(target_os = "macos")
        );
    }

    #[test]
    fn a_browser_row_names_two_profiles_and_counts_more() {
        let names = |n: usize| -> Vec<String> {
            ["Personal", "Work", "Research", "Design", "Spanish"][..n]
                .iter()
                .map(|name| (*name).to_owned())
                .collect()
        };
        assert_eq!(profiles_hint(&names(0)), None);
        assert_eq!(profiles_hint(&names(1)).as_deref(), Some("Personal"));
        assert_eq!(
            profiles_hint(&names(2)).as_deref(),
            Some("Personal \u{00b7} Work")
        );
        assert_eq!(profiles_hint(&names(3)).as_deref(), Some("3 profiles"));
        assert_eq!(profiles_hint(&names(5)).as_deref(), Some("5 profiles"));
    }

    #[test]
    fn badges_match_whole_words() {
        assert_eq!(badge_for("Google Chrome"), "Ch");
        assert_eq!(badge_for("com.apple.Safari"), "Sf");
        assert_eq!(badge_for("Arc"), "Ar");
        assert_eq!(badge_for("us.zoom.xos"), "Zm");
        assert_eq!(badge_for("com.microsoft.edgemac"), "Ed");
        assert_eq!(badge_for("Firefox Developer Edition"), "Ff");
        // "arc" inside another word is not Arc.
        assert_eq!(badge_for("Search Browser"), "Se");
        assert_eq!(badge_for("com.example.archiver"), "Ar");
        assert_eq!(badge_for("Searchlight"), "Se");
    }

    /// `one_line` above a fixed box, in a column.
    struct Lines;

    impl Render for Lines {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .w(gpui_kit::px(120.0))
                .child(
                    one_line(
                        "https://example.com/gpq/a-long-path-that-does-not-fit",
                        crate::theme::mono_family(500.0),
                        500.0,
                        11.5,
                        11.5,
                        crate::theme::text_primary(),
                    )
                    .debug_selector(|| "line".into()),
                )
                .child(
                    super::clip_x(div().h(gpui_kit::px(12.0)), 12.0, 12.0)
                        .debug_selector(|| "clipped".into()),
                )
                .child(
                    div()
                        .h(gpui_kit::px(10.0))
                        .debug_selector(|| "after".into()),
                )
        }
    }

    /// The clip leaves room for descenders without moving anything: a
    /// clipped line still takes exactly its line height and its width.
    #[gpui_kit::test]
    fn a_clipped_line_takes_its_line_height(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| Lines);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let line = cx.debug_bounds("line").expect("the line");
        let clipped = cx.debug_bounds("clipped").expect("the clipped box");
        let after = cx.debug_bounds("after").expect("the box after it");
        assert_eq!(line.size.height, gpui_kit::px(11.5));
        assert_eq!(line.size.width, gpui_kit::px(120.0));
        assert_eq!(clipped.origin.y, line.origin.y + gpui_kit::px(11.5));
        assert_eq!(clipped.size.height, gpui_kit::px(12.0));
        assert_eq!(after.origin.y, clipped.origin.y + gpui_kit::px(12.0));
    }

    #[test]
    fn the_clip_clears_geist_monos_glyphs() {
        // Geist Mono 500's hhea ascent and descent (1.005em, 0.295em) and
        // the deepest ink of g, p and y (0.162em below the baseline), laid
        // out as GPUI does: ascent plus descent centered in the line.
        for (size, line_height) in [(10.0, 10.0), (10.5, 10.5), (11.5, 11.5), (12.0, 12.0)] {
            let baseline: f32 = (line_height - 1.3 * size) / 2.0 + 1.005 * size;
            let below = baseline + 0.162 * size - line_height;
            assert!(super::glyph_overhang(size, line_height) > below);
        }
        assert_eq!(super::glyph_overhang(10.0, 20.0), 1.0);
    }
}
