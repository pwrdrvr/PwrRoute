//! Help's document windows: Changelog, View License and Third-Party
//! Notices, as PwrAgent, PwrSnap and PwrGit show them. The documents are
//! compiled in, so each window shows exactly what this build ships and
//! works offline on every platform, bundle or not.
//!
//! The changelog is drawn from its Markdown (headings, bullets, `code`,
//! **bold**); the license and the notices are monospace text, wrapped at
//! [`WRAP_COLUMNS`] once when the window opens so the notices' 1.3 MB draw
//! as a list of fixed-height rows.

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, HighlightStyle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, UniformListScrollHandle, Window, WindowHandle, div, px, uniform_list,
};

use crate::aux_window::{self, Frame};
use crate::menu_bar::MenuBar;
use crate::theme::{self, mono, text};
use crate::widgets::{ButtonKind, button, chip};

pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");
pub const LICENSE: &str = include_str!("../../../LICENSE");
pub const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD_PARTY_LICENSES");

/// Where the license and the notices wrap.
pub const WRAP_COLUMNS: usize = 100;

/// The window's Copy button.
pub const COPY_ID: &str = "doc-copy";

/// Which document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doc {
    Changelog,
    License,
    ThirdPartyNotices,
}

impl Doc {
    /// The crumb after "Help ›".
    pub fn title(self) -> &'static str {
        match self {
            Self::Changelog => "Changelog",
            Self::License => "License",
            Self::ThirdPartyNotices => "Third-Party Notices",
        }
    }

    fn frame(self) -> Frame {
        Frame {
            title: match self {
                Self::Changelog => "PwrRoute Changelog",
                Self::License => "PwrRoute License",
                Self::ThirdPartyNotices => "PwrRoute Third-Party Notices",
            },
            size: (920.0, 760.0),
            min: (640.0, 480.0),
        }
    }

    pub fn source(self) -> &'static str {
        match self {
            Self::Changelog => CHANGELOG,
            Self::License => LICENSE,
            Self::ThirdPartyNotices => THIRD_PARTY_NOTICES,
        }
    }
}

/// Opens `doc`'s window, or brings it forward.
pub fn open(doc: Doc, cx: &mut App) {
    let existing = cx.windows().into_iter().find_map(|window| {
        let handle = window.downcast::<DocWindow>()?;
        (handle.read(cx).ok()?.doc == doc).then_some(handle)
    });
    if aux_window::activate(existing, cx) {
        return;
    }
    open_window(doc, cx);
}

/// A new window for `doc`, for `open` and the tests.
pub fn open_window(doc: Doc, cx: &mut App) -> Option<WindowHandle<DocWindow>> {
    aux_window::open(&doc.frame(), cx, |_, cx| DocWindow::new(doc, cx))
}

/// The window's view.
pub struct DocWindow {
    pub doc: Doc,
    body: Body,
    focus: FocusHandle,
    menu_bar: Entity<MenuBar>,
    copied: bool,
    scroll: gpui_kit::ScrollHandle,
}

enum Body {
    Changelog(Vec<Block>),
    /// Wrapped lines, shared with the list's row builder.
    Lines(Rc<Vec<SharedString>>, UniformListScrollHandle),
}

impl DocWindow {
    pub fn new(doc: Doc, cx: &mut Context<'_, Self>) -> Self {
        let body = match doc {
            Doc::Changelog => Body::Changelog(changelog(CHANGELOG, env!("CARGO_PKG_VERSION"))),
            Doc::License | Doc::ThirdPartyNotices => Body::Lines(
                Rc::new(
                    wrap(doc.source(), WRAP_COLUMNS)
                        .into_iter()
                        .map(SharedString::from)
                        .collect(),
                ),
                UniformListScrollHandle::new(),
            ),
        };
        Self {
            doc,
            body,
            focus: cx.focus_handle(),
            menu_bar: MenuBar::entity(doc.title(), cx),
            copied: false,
            scroll: gpui_kit::ScrollHandle::new(),
        }
    }

    fn copy(&mut self, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.doc.source().to_owned()));
        self.copied = true;
        cx.notify();
    }
}

impl Render for DocWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let copy_label = if self.copied { "Copied" } else { "Copy" };
        let strip = aux_window::strip(self.doc.title(), &self.menu_bar, window, cx).child(
            button(copy_label, ButtonKind::Default, true)
                .id(COPY_ID)
                .debug_selector(|| COPY_ID.into())
                // On Windows the strip is the drag area: a press here must
                // be the button's.
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
        );
        let body = match &self.body {
            Body::Changelog(blocks) => div()
                .id("doc-scroll")
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .max_w(px(760.0))
                        .px(px(32.0))
                        .py(px(24.0))
                        .flex()
                        .flex_col()
                        .children(
                            blocks
                                .iter()
                                .enumerate()
                                .map(|(index, block)| draw_block(index, block)),
                        ),
                )
                .into_any_element(),
            Body::Lines(lines, scroll) => {
                let rows = lines.clone();
                div()
                    .flex_1()
                    .min_h_0()
                    .p(px(14.0))
                    .child(
                        div()
                            .size_full()
                            .border_1()
                            .border_color(theme::border_subtle())
                            .rounded(px(8.0))
                            .bg(theme::bg_sidebar())
                            .py(px(10.0))
                            .child(
                                uniform_list("doc-lines", rows.len(), move |range, _, _| {
                                    range
                                        .map(|index| draw_line(index, rows[index].clone()))
                                        .collect()
                                })
                                .track_scroll(scroll)
                                .size_full(),
                            ),
                    )
                    .into_any_element()
            }
        };
        div()
            .id("document-window")
            .role(gpui_kit::Role::Document)
            .aria_label(self.doc.title())
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    let key = event.keystroke.key.as_str();
                    if key == "escape" {
                        window.remove_window();
                        cx.stop_propagation();
                        return;
                    }
                    if matches!(key, "pageup" | "pagedown" | "home" | "end") {
                        let scroll = match &this.body {
                            Body::Changelog(_) => this.scroll.clone(),
                            Body::Lines(_, handle) => handle.0.borrow().base_handle.clone(),
                        };
                        let old = scroll.offset();
                        let page = window.viewport_size().height * 0.8;
                        let y = match key {
                            "pageup" => (old.y + page).min(gpui_kit::px(0.0)),
                            "pagedown" => old.y - page,
                            "home" => gpui_kit::px(0.0),
                            _ => gpui_kit::Pixels::MIN,
                        };
                        scroll.set_offset(gpui_kit::point(old.x, y));
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(strip)
            .child(body)
    }
}

/// A license line's height: 12px at the design's 1.55.
const LINE_HEIGHT: f32 = 18.0;

// ---- The changelog -------------------------------------------------------

/// A piece of the changelog, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `## v0.2.2 - 2026-10-24`: the version, its date, and whether it is
    /// this copy's.
    Version {
        name: String,
        date: Option<String>,
        current: bool,
    },
    /// `- Area - what changed`.
    Bullet(Vec<Span>),
    /// Any other text.
    Paragraph(Vec<Span>),
}

/// A run of inline text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Inline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline {
    Plain,
    Bold,
    Code,
}

/// The changelog's blocks: the `# Changelog` title is the window's crumb,
/// and a version with nothing under it (an empty Unreleased) is left out.
pub fn changelog(markdown: &str, version: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(spans(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            flush(&mut paragraph, &mut blocks);
            let (name, date) = match heading.split_once(" - ") {
                Some((name, date)) => (name.trim(), Some(date.trim().to_owned())),
                None => (heading.trim(), None),
            };
            let current = name.strip_prefix('v') == Some(version);
            blocks.push(Block::Version {
                name: name.to_owned(),
                date,
                current,
            });
        } else if trimmed.starts_with("# ") {
            flush(&mut paragraph, &mut blocks);
        } else if trimmed.starts_with("###") {
            // A subsection ("### Fixed"): its words in bold, as a line.
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Paragraph(vec![Span {
                text: trimmed.trim_start_matches('#').trim().to_owned(),
                style: Inline::Bold,
            }]));
        } else if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet(bullet_spans(item)));
        } else if let Some(Block::Bullet(last)) = blocks.last_mut()
            && paragraph.is_empty()
            && line.starts_with("  ")
        {
            // A bullet's continuation line.
            for span in std::iter::once(Span {
                text: " ".to_owned(),
                style: Inline::Plain,
            })
            .chain(spans(trimmed))
            {
                match last.last_mut() {
                    Some(end) if end.style == span.style => end.text.push_str(&span.text),
                    _ => last.push(span),
                }
            }
        } else {
            paragraph.push(trimmed);
        }
    }
    flush(&mut paragraph, &mut blocks);
    // Drop versions with nothing under them.
    let mut kept = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let empty_version = matches!(block, Block::Version { .. })
            && !matches!(
                blocks.get(index + 1),
                Some(Block::Bullet(_) | Block::Paragraph(_))
            );
        if !empty_version {
            kept.push(block.clone());
        }
    }
    kept
}

fn draw_line(index: usize, content: SharedString) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(("doc-line", index))
        .h(px(LINE_HEIGHT))
        .px(px(16.0))
        .child(mono(content, 12.0, 400.0, theme::text_secondary()))
}

/// "Area - what changed": the area in bold, as the changelog's convention.
fn bullet_spans(item: &str) -> Vec<Span> {
    match item.split_once(" - ") {
        Some((area, rest)) if !area.contains('`') && area.split_whitespace().count() <= 5 => {
            let mut out = vec![Span {
                text: format!("{area} \u{2014} "),
                style: Inline::Bold,
            }];
            out.extend(spans(rest));
            out
        }
        _ => spans(item),
    }
}

/// Inline Markdown: `code` and **bold**; everything else as written.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut push = |text: &str, style: Inline| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => out.push(Span {
                text: text.to_owned(),
                style,
            }),
        }
    };
    let mut rest = text;
    while !rest.is_empty() {
        let code = rest.find('`');
        let bold = rest.find("**");
        match (code, bold) {
            (Some(start), bold) if bold.is_none_or(|bold| start < bold) => {
                match rest[start + 1..].find('`') {
                    Some(len) => {
                        push(&rest[..start], Inline::Plain);
                        push(&rest[start + 1..start + 1 + len], Inline::Code);
                        rest = &rest[start + 2 + len..];
                    }
                    None => {
                        push(rest, Inline::Plain);
                        rest = "";
                    }
                }
            }
            (_, Some(start)) => match rest[start + 2..].find("**") {
                Some(len) => {
                    push(&rest[..start], Inline::Plain);
                    push(&rest[start + 2..start + 2 + len], Inline::Bold);
                    rest = &rest[start + 4 + len..];
                }
                None => {
                    push(rest, Inline::Plain);
                    rest = "";
                }
            },
            _ => {
                push(rest, Inline::Plain);
                rest = "";
            }
        }
    }
    out
}

fn draw_block(index: usize, block: &Block) -> gpui_kit::Stateful<gpui_kit::Div> {
    let block = match block {
        Block::Version {
            name,
            date,
            current,
        } => div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .pt(px(22.0))
            .pb(px(10.0))
            .mb(px(6.0))
            .border_b_1()
            .border_color(theme::border_faint())
            .child(text(name.clone(), 17.0, 650.0, theme::text_primary()))
            .when_some(date.clone(), |row, date| {
                row.child(mono(date, 11.0, 500.0, theme::text_muted()))
            })
            .when(*current, |row| row.child(chip("This copy"))),
        Block::Bullet(spans) => div()
            .flex()
            .gap(px(10.0))
            .py(px(5.0))
            .child(
                div()
                    .flex_none()
                    .pt(px(7.0))
                    .child(div().size(px(4.0)).rounded_full().bg(theme::accent())),
            )
            .child(styled(spans).flex_1().min_w_0()),
        Block::Paragraph(spans) => styled(spans).py(px(6.0)),
    };
    block.id(("doc-block", index))
}

/// Spans as one wrapping run: sans 12.5/1.6, bold in the primary color,
/// code in the accent with a faint fill (a highlight cannot change font).
fn styled(spans: &[Span]) -> gpui_kit::Div {
    let mut content = String::new();
    let mut highlights = Vec::new();
    for span in spans {
        let start = content.len();
        content.push_str(&span.text);
        let style = match span.style {
            Inline::Plain => continue,
            Inline::Bold => HighlightStyle {
                color: Some(theme::text_primary()),
                font_weight: Some(theme::weight(650.0)),
                ..Default::default()
            },
            Inline::Code => HighlightStyle {
                color: Some(theme::accent_bright()),
                background_color: Some(theme::accent_tint()),
                ..Default::default()
            },
        };
        highlights.push((start..content.len(), style));
    }
    div()
        .font_family(theme::SANS)
        .font_weight(theme::weight(400.0))
        .text_size(px(12.5))
        .line_height(px(20.0))
        .text_color(theme::text_secondary())
        .child(StyledText::new(content).with_highlights(highlights))
}

// ---- Plain text ----------------------------------------------------------

/// `text`'s lines, each broken at spaces to at most `columns` characters
/// (a word longer than that is cut). Tabs become four spaces.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        if line.chars().count() <= columns {
            out.push(line);
            continue;
        }
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        // A deep indent would leave no room: drop it.
        let indent = if indent.len() * 2 < columns {
            indent
        } else {
            String::new()
        };
        let mut current = indent.clone();
        let mut has_word = false;
        for word in line.split(' ').filter(|word| !word.is_empty()) {
            if has_word && current.chars().count() + 1 + word.chars().count() > columns {
                out.push(std::mem::replace(&mut current, indent.clone()));
                has_word = false;
            }
            if has_word {
                current.push(' ');
            }
            current.push_str(word);
            has_word = true;
            while current.chars().count() > columns {
                out.push(current.chars().take(columns).collect());
                current = current.chars().skip(columns).collect();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Element;

    #[test]
    fn repeated_document_text_has_distinct_stable_instance_ids() {
        let lines: Vec<_> = ["", "", "Repeated", "Repeated"]
            .into_iter()
            .enumerate()
            .map(|(index, content)| Element::id(&draw_line(index, content.into())).unwrap())
            .collect();
        for (index, id) in lines.iter().enumerate() {
            assert!(!lines[..index].contains(id));
        }
        assert_eq!(
            Some(lines[0].clone()),
            Element::id(&draw_line(0, "".into()))
        );
        let block = Block::Version {
            name: "v0.1.0".into(),
            date: Some("2026-10-29".into()),
            current: true,
        };
        assert_ne!(
            Element::id(&draw_block(0, &block)),
            Element::id(&draw_block(1, &block))
        );
    }

    #[test]
    fn the_changelog_reads_as_versions_and_bullets() {
        let blocks = changelog(
            "# Changelog\n\n## Unreleased\n\n## v0.2.2 - 2026-10-24\n\n- Browser hand-off - \
             Gives the `front` away.\n  Continued here.\n- Plain bullet with **bold**\n\n\
             Important: a note.\n\n### Fixed\n\n## v0.2.1 - 2026-10-23\n\n- Rules - One.\n",
            "0.2.2",
        );
        assert_eq!(
            blocks,
            vec![
                Block::Version {
                    name: "v0.2.2".into(),
                    date: Some("2026-10-24".into()),
                    current: true,
                },
                Block::Bullet(vec![
                    Span {
                        text: "Browser hand-off \u{2014} ".into(),
                        style: Inline::Bold
                    },
                    Span {
                        text: "Gives the ".into(),
                        style: Inline::Plain
                    },
                    Span {
                        text: "front".into(),
                        style: Inline::Code
                    },
                    Span {
                        text: " away. Continued here.".into(),
                        style: Inline::Plain
                    },
                ]),
                Block::Bullet(vec![
                    Span {
                        text: "Plain bullet with ".into(),
                        style: Inline::Plain
                    },
                    Span {
                        text: "bold".into(),
                        style: Inline::Bold
                    },
                ]),
                Block::Paragraph(vec![Span {
                    text: "Important: a note.".into(),
                    style: Inline::Plain
                }]),
                Block::Paragraph(vec![Span {
                    text: "Fixed".into(),
                    style: Inline::Bold
                }]),
                Block::Version {
                    name: "v0.2.1".into(),
                    date: Some("2026-10-23".into()),
                    current: false,
                },
                Block::Bullet(vec![
                    Span {
                        text: "Rules \u{2014} ".into(),
                        style: Inline::Bold
                    },
                    Span {
                        text: "One.".into(),
                        style: Inline::Plain
                    },
                ]),
            ]
        );
    }

    #[test]
    fn the_shipped_changelog_has_this_version() {
        let blocks = changelog(CHANGELOG, env!("CARGO_PKG_VERSION"));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, Block::Version { current: true, .. })),
            "CHANGELOG.md has no section for {}",
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn unclosed_marks_stay_as_written() {
        assert_eq!(
            spans("a `b and **c"),
            vec![Span {
                text: "a `b and **c".into(),
                style: Inline::Plain
            }]
        );
    }

    #[test]
    fn long_lines_wrap_at_spaces_and_keep_their_indent() {
        assert_eq!(
            wrap("short\n  one two three four", 10),
            ["short", "  one two", "  three", "  four"]
        );
        assert_eq!(wrap("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        assert_eq!(wrap("", 5), Vec::<String>::new());
        assert_eq!(wrap("a\n\nb", 5), ["a", "", "b"]);
    }

    #[test]
    fn the_documents_are_compiled_in() {
        assert!(LICENSE.contains("John Sterling"));
        assert!(THIRD_PARTY_NOTICES.len() > 10_000);
        assert!(
            wrap(THIRD_PARTY_NOTICES, WRAP_COLUMNS)
                .iter()
                .all(|line| line.chars().count() <= WRAP_COLUMNS)
        );
    }
}
