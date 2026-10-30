//! What a Rules row's "⋯" does (design: Rule Editing 3a, 3b and 4b).
//!
//! A rule's menu moves it to the top, up, down or to the bottom of its own
//! list, duplicates it, turns it off or on, tests a link with it and
//! deletes it. Dragging a row moves it too, and so do ⌥⌘↑ and ⌥⌘↓ on the
//! selected row; ⌘D duplicates and ⌫ deletes. A move says how many recent
//! links it would send somewhere else: the in-memory dispatches resolved
//! against the rules as they are and as they would be. With fewer than
//! [`IMPACT_MIN`] of them the count is left out rather than claim a
//! confidence it does not have.
//!
//! Delete is immediate. The row stays where it was as a tombstone with
//! Undo (and ⌘Z) until the next edit, and Undo puts the same rule back at
//! the same place, saved or not as it was.
//!
//! Everything else's menu chooses its browser and profile; a Finicky
//! config's read-only row offers its file and the importer.

use std::collections::HashMap;

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, AnyElement, App, AppContext as _, Context, Div, DragMoveEvent, ElementId,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div, point,
};
use pwrroute_engine::Opener;
use pwrroute_router::Privacy;

use crate::app::{PwrRoute, Screen, Selection};
use crate::rules_model::{Destination, Matcher, Rule};
use crate::rules_store::{Removed, RulesStore};
use crate::theme::{self, mono, text};
use crate::try_it::Probe;
use crate::widgets::{ButtonKind, badge, badge_for, button, menu_card, menu_row, paragraph};

/// Fewer recent links than this and a move says nothing about them.
pub const IMPACT_MIN: usize = 20;

/// How many of the newest links a move is weighed against.
pub const IMPACT_OF: usize = 100;

/// The menu's width: the design's 270, plus room for a count beside a key.
const MENU_WIDTH: f32 = 310.0;

/// Which row a menu belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuRow {
    /// A rewrite or rule of `rules.json`.
    Rule(Selection),
    /// The Everything else row.
    EverythingElse,
    /// A Finicky config's read-only row: a rewrite or a handler, by its
    /// place in the config.
    Config { rewrite: bool, index: usize },
}

/// The rows' menus, the tombstone and the drag.
#[derive(Default)]
pub struct RowActions {
    /// The open menu.
    pub menu: Option<MenuRow>,
    /// The menu open when the mouse went down on a "⋯": the menu closes on
    /// that mouse down, so the click that follows reads this.
    pub menu_was: Option<MenuRow>,
    /// Why Everything else's last choice was not saved.
    pub refused: Option<String>,
    /// The last rule deleted, for Undo, until the next edit.
    pub deleted: Option<Deleted>,
    /// The row being dragged, set when the drag starts.
    pub dragging: Option<Selection>,
    /// Where the dragged row would land.
    pub drop: Option<DropAt>,
    /// The recent links and where they open now, for the moves' counts:
    /// read when a menu opens or a drag starts.
    baseline: Option<Baseline>,
    /// Each move's count, by the row and where it would land.
    impacts: HashMap<(Selection, usize), Option<usize>>,
}

impl RowActions {
    /// An edit was made: the counts are stale and Undo is gone.
    pub fn edited(&mut self) {
        self.deleted = None;
        self.forget_counts();
    }

    /// How many recent links the counts weigh, once a menu or drag read
    /// them.
    pub fn weighed(&self) -> Option<usize> {
        self.baseline.as_ref().map(|baseline| baseline.links.len())
    }

    fn forget_counts(&mut self) {
        self.baseline = None;
        self.impacts.clear();
    }
}

/// A deleted rule: what it was, for the tombstone, and how to put it back.
pub struct Deleted {
    /// Where it was.
    pub at: Selection,
    /// Its row as it was drawn.
    pub row: Rule,
    removed: Removed,
}

/// Where a dragged row would land: `to` in its list, drawn as a line above
/// or below row `over`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropAt {
    pub from: Selection,
    pub to: usize,
    pub over: usize,
    pub above: bool,
}

/// Where a link opens, as far as a move can change it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Opens {
    browser: String,
    profile: String,
    private: bool,
    url: String,
}

/// The links a move is weighed against, and where each opens now.
struct Baseline {
    links: Vec<(String, Option<Opener>)>,
    now: Vec<Option<Opens>>,
}

fn opens(probe: &mut Probe, url: &str, opener: Option<&Opener>) -> Option<Opens> {
    probe.opens(url, opener).map(|browser| Opens {
        browser: browser.name,
        profile: browser.profile,
        private: browser.private,
        url: browser.url,
    })
}

/// The newest [`IMPACT_OF`] links the router keeps in memory with their
/// full URL, as the rules saw them; `None` with fewer than [`IMPACT_MIN`].
fn recent_links(cx: &App) -> Option<Vec<(String, Option<Opener>)>> {
    let feed = crate::activity::feed(cx)?;
    let links: Vec<(String, Option<Opener>)> = feed
        .read(cx)
        .records()
        .iter()
        .filter(|record| record.privacy == Privacy::Full)
        .take(IMPACT_OF)
        .map(|record| {
            (
                crate::activity::matched_url(record).to_owned(),
                crate::activity_actions::engine_opener(record),
            )
        })
        .collect();
    (links.len() >= IMPACT_MIN).then_some(links)
}

/// How many of `links` would open somewhere else, or as another URL, once
/// `selection` moves to `to`; `None` when it would not move. Both sides
/// are the file as written, which is what the router routes by.
fn changes_from(
    probe: &Probe,
    store: &RulesStore,
    selection: Selection,
    to: usize,
    links: &[(String, Option<Opener>)],
    now: &[Option<Opens>],
) -> Option<usize> {
    let mut moved = store.clone();
    moved.move_to(selection, to)?;
    let mut after = probe.with_rules(moved.written());
    Some(
        links
            .iter()
            .zip(now)
            .filter(|((url, opener), now)| opens(&mut after, url, opener.as_ref()) != **now)
            .count(),
    )
}

/// "23 links change", "1 link changes", "no recent links change".
pub fn impact_label(count: usize) -> String {
    match count {
        0 => "no recent links change".into(),
        1 => "1 link changes".into(),
        n => format!("{n} links change"),
    }
}

/// Where `index` ends up when the row at `from` moves to `to`.
pub fn shifted(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < to && index > from && index <= to {
        index - 1
    } else if to < from && index >= to && index < from {
        index + 1
    } else {
        index
    }
}

fn position(selection: Selection) -> usize {
    match selection {
        Selection::Rewrite(i) | Selection::Handler(i) => i,
    }
}

fn same_list(a: Selection, b: Selection) -> bool {
    matches!(
        (a, b),
        (Selection::Rewrite(_), Selection::Rewrite(_))
            | (Selection::Handler(_), Selection::Handler(_))
    )
}

fn at(selection: Selection, index: usize) -> Selection {
    match selection {
        Selection::Rewrite(_) => Selection::Rewrite(index),
        Selection::Handler(_) => Selection::Handler(index),
    }
}

/// A rule as its list names it: "rule 3", "rewrite 1".
pub fn rule_name(selection: Selection) -> String {
    match selection {
        Selection::Rewrite(i) => format!("rewrite {}", i + 1),
        Selection::Handler(i) => format!("rule {}", i + 1),
    }
}

/// The keys, as this platform writes them.
pub struct Keys {
    pub up: &'static str,
    pub down: &'static str,
    pub duplicate: &'static str,
    pub delete: &'static str,
    pub undo: &'static str,
}

pub fn keys(cx: &App) -> Keys {
    match crate::chrome::current(cx) {
        crate::chrome::Chrome::Mac => Keys {
            up: "\u{2325}\u{2318}\u{2191}",
            down: "\u{2325}\u{2318}\u{2193}",
            duplicate: "\u{2318}D",
            delete: "\u{232b}",
            undo: "\u{2318}Z",
        },
        crate::chrome::Chrome::Windows | crate::chrome::Chrome::Linux => Keys {
            up: "Ctrl+Alt+\u{2191}",
            down: "Ctrl+Alt+\u{2193}",
            duplicate: "Ctrl+D",
            delete: "Del",
            undo: "Ctrl+Z",
        },
    }
}

// --- actions ----------------------------------------------------------------

impl PwrRoute {
    /// "⋯": opens the row's menu, or closes it when it was open. A rule's
    /// menu weighs its moves as it opens.
    pub fn toggle_row_actions(&mut self, row: MenuRow, cx: &mut Context<'_, Self>) {
        if self.row_actions.menu_was == Some(row) {
            self.row_actions.menu = None;
            cx.notify();
            return;
        }
        self.row_actions.refused = None;
        self.row_actions.forget_counts();
        if let MenuRow::Rule(selection) = row {
            let last = self.store.len_of(selection).saturating_sub(1);
            let here = position(selection);
            for to in [0, here.saturating_sub(1), here + 1, last] {
                if to != here && to <= last {
                    self.impact(selection, to, cx);
                }
            }
        }
        self.row_actions.menu = Some(row);
        cx.notify();
    }

    pub fn close_row_actions(&mut self, cx: &mut Context<'_, Self>) {
        if self.row_actions.menu.take().is_some() {
            cx.notify();
        }
    }

    /// How many recent links moving `selection` to `to` would send
    /// elsewhere, worked out once per menu or drag.
    pub fn impact(&mut self, selection: Selection, to: usize, cx: &App) -> Option<usize> {
        if let Some(count) = self.row_actions.impacts.get(&(selection, to)) {
            return *count;
        }
        if self.row_actions.baseline.is_none() {
            self.row_actions.baseline = recent_links(cx).map(|links| {
                let mut probe = self.probe.with_rules(self.store.written());
                let now = links
                    .iter()
                    .map(|(url, opener)| opens(&mut probe, url, opener.as_ref()))
                    .collect();
                Baseline { links, now }
            });
        }
        let count = self.row_actions.baseline.as_ref().and_then(|baseline| {
            changes_from(
                &self.probe,
                &self.store,
                selection,
                to,
                &baseline.links,
                &baseline.now,
            )
        });
        self.row_actions.impacts.insert((selection, to), count);
        count
    }

    /// Moves a rule within its list. The selection, the editor and a
    /// draft's note follow the rows they were on.
    pub fn move_rule(
        &mut self,
        selection: Selection,
        to: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.menu = None;
        self.row_actions.drop = None;
        let Some(landed) = self.store.move_to(selection, to) else {
            cx.notify();
            return;
        };
        let (from, to) = (position(selection), position(landed));
        let follow = |s: Selection| {
            if same_list(s, selection) {
                at(s, shifted(position(s), from, to))
            } else {
                s
            }
        };
        self.selection = self.selection.map(follow);
        if let Some(rail) = &mut self.rail {
            rail.of = rail.of.map(follow);
        }
        if let Selection::Handler(_) = selection {
            self.drafts_after_move(from, to);
        }
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        window.focus(&self.focus, cx);
    }

    /// One row up or down: ⌥⌘↑ and ⌥⌘↓.
    fn step_rule(
        &mut self,
        target: Option<Selection>,
        by: isize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(selection) = target else {
            return false;
        };
        let to = position(selection).saturating_add_signed(by);
        if to >= self.store.len_of(selection) || to == position(selection) {
            // At the end already: the key is still this list's.
            return true;
        }
        self.move_rule(selection, to, window, cx);
        true
    }

    /// A copy of the rule just below it, selected.
    pub fn duplicate_rule(
        &mut self,
        selection: Selection,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.menu = None;
        let Some(copy) = self.store.duplicate(selection) else {
            return;
        };
        if let Selection::Handler(index) = copy {
            self.drafts_after_insert(index);
        }
        self.selection = Some(copy);
        self.refresh(cx);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
    }

    /// Turn off, or Turn on.
    pub fn set_rule_enabled(
        &mut self,
        selection: Selection,
        enabled: bool,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.menu = None;
        self.store.set_enabled(selection, enabled);
        self.refresh(cx);
    }

    /// Test a link with this rule: its first example, else a link its
    /// first pattern matches, from the app it requires, on Test a link.
    pub fn test_rule(
        &mut self,
        selection: Selection,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.menu = None;
        let (url, opener) = match selection {
            Selection::Handler(i) => {
                let Some(handler) = self.handlers.get(i) else {
                    return;
                };
                (
                    handler
                        .examples
                        .first()
                        .cloned()
                        .unwrap_or_else(|| crate::try_it::sample_url(&handler.patterns)),
                    handler
                        .openers
                        .iter()
                        .find(|o| !o.trim().is_empty())
                        .cloned(),
                )
            }
            Selection::Rewrite(i) => {
                let Some(rewrite) = self.rewrites.get(i) else {
                    return;
                };
                (
                    rewrite
                        .examples
                        .first()
                        .map(|example| example.url.clone())
                        .unwrap_or_else(|| crate::try_it::sample_url(&rewrite.patterns)),
                    None,
                )
            }
        };
        // Test a link tests the file as written: write what can be.
        self.save_now(cx);
        self.test.url.set(&url);
        self.test.opener = opener
            .and_then(|opener| {
                self.test
                    .openers()
                    .iter()
                    .position(|choice| choice.as_ref().is_some_and(|c| c.is_named(&opener)))
            })
            .unwrap_or(0);
        self.test.run();
        self.navigate(Screen::TestLink, window, cx);
    }

    /// Deletes a rule now, leaving its tombstone with Undo. The selection
    /// stays on its row, or moves to the row that took the deleted one's
    /// place.
    pub fn delete_rule(
        &mut self,
        selection: Selection,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.menu = None;
        let row = match selection {
            Selection::Handler(i) => self.handlers.get(i).map(|h| h.rule.clone()),
            Selection::Rewrite(i) => self.rewrites.get(i).map(|r| r.rule.clone()),
        };
        let Some(removed) = self.store.remove(selection) else {
            return;
        };
        let deleted = position(selection);
        if let Selection::Handler(_) = selection {
            self.drafts_after_delete(deleted);
        }
        let len = self.store.len_of(selection);
        let selected_was = self.selection;
        self.selection = match self.selection {
            Some(s) if s == selection => (len > 0).then(|| at(s, deleted.min(len - 1))),
            Some(s) if same_list(s, selection) && position(s) > deleted => {
                Some(at(s, position(s) - 1))
            }
            other => other,
        };
        self.refresh(cx);
        // A draft was never in the file: discarding it leaves nothing to
        // put back.
        let row = row.filter(|row| row.badge != crate::rules_model::RowBadge::Draft);
        self.row_actions.deleted = row.map(|row| Deleted {
            at: selection,
            row,
            removed,
        });
        if selected_was == Some(selection) {
            self.rebuild_rail(cx);
        } else {
            if let Some(rail) = &mut self.rail {
                rail.of = self.selection;
            }
            self.rebuild_rail_kept(cx);
        }
        window.focus(&self.focus, cx);
    }

    /// Undo delete: the rule back at its place, selected. `false` when
    /// there is nothing to undo.
    pub fn undo_delete(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some(deleted) = self.row_actions.deleted.take() else {
            return false;
        };
        let restored = self.store.restore(deleted.removed);
        if let Selection::Handler(index) = restored {
            self.drafts_after_insert(index);
        }
        self.selection = Some(restored);
        self.refresh(cx);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
        true
    }

    /// Everything else's menu: `rules.json`'s default browser and profile,
    /// written now. A choice the store would not save keeps the menu open
    /// with the reason.
    pub fn set_everything_else(
        &mut self,
        browser: &str,
        profile: &str,
        cx: &mut Context<'_, Self>,
    ) {
        match self.write_default_browser(browser, profile, cx) {
            Ok(()) => self.row_actions.menu = None,
            Err(reason) => self.row_actions.refused = Some(reason),
        }
        cx.notify();
    }

    /// The Rules list's keys, on the selected row: ⌥⌘↑ / ⌥⌘↓ move it, ⌘D
    /// duplicates it, ⌫ deletes it, ⌘Z undoes the delete, Esc closes a menu
    /// or puts a dragged row back. Returns whether the key was taken.
    pub fn rules_list_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        let plain = !m.secondary() && !m.alt && !m.shift && !m.control && !m.platform;
        // The keys the menu shows act on its row; with none open, on the
        // selected one.
        let target = match self.row_actions.menu {
            Some(MenuRow::Rule(selection)) => Some(selection),
            _ => self.selection,
        };
        match keystroke.key.as_str() {
            "escape" if cx.has_active_drag() => {
                cx.stop_active_drag(window);
                self.row_actions.dragging = None;
                self.row_actions.drop = None;
                cx.notify();
                true
            }
            "escape" if self.row_actions.menu.is_some() => {
                self.close_row_actions(cx);
                true
            }
            "up" if m.secondary() && m.alt && !m.shift => self.step_rule(target, -1, window, cx),
            "down" if m.secondary() && m.alt && !m.shift => self.step_rule(target, 1, window, cx),
            "d" if m.secondary() && !m.alt && !m.shift => match target {
                Some(selection) => {
                    self.duplicate_rule(selection, window, cx);
                    true
                }
                None => false,
            },
            "z" if m.secondary() && !m.alt && !m.shift => self.undo_delete(window, cx),
            "backspace" | "delete" if plain => match target {
                Some(selection) => {
                    self.delete_rule(selection, window, cx);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    /// A dragged row over row `over` of its list: where it would land.
    fn drag_over_row(
        &mut self,
        dragged: Selection,
        over: Selection,
        above: bool,
        cx: &mut Context<'_, Self>,
    ) {
        if !same_list(dragged, over) {
            // Nowhere to land in the other list: forget the last target, so
            // a release here moves nothing.
            if self.row_actions.drop.take().is_some() {
                cx.notify();
            }
            return;
        }
        let (from, row) = (position(dragged), position(over));
        let to = match (row.cmp(&from), above) {
            (std::cmp::Ordering::Equal, _) => from,
            (std::cmp::Ordering::Greater, true) => row - 1,
            (std::cmp::Ordering::Greater, false) => row,
            (std::cmp::Ordering::Less, true) => row,
            (std::cmp::Ordering::Less, false) => row + 1,
        };
        let drop = DropAt {
            from: dragged,
            to,
            over: row,
            above,
        };
        if self.row_actions.drop != Some(drop) {
            self.row_actions.drop = Some(drop);
            if to != from {
                self.impact(dragged, to, cx);
            }
            cx.notify();
        }
    }

    /// A dragged row released over row `on`: it moves only to the place
    /// drawn on that row.
    fn drop_row(
        &mut self,
        dragged: Selection,
        on: Selection,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.row_actions.dragging = None;
        match self.row_actions.drop.take() {
            Some(drop)
                if drop.from == dragged
                    && same_list(dragged, on)
                    && drop.over == position(on)
                    && drop.to != position(dragged) =>
            {
                self.move_rule(dragged, drop.to, window, cx);
            }
            _ => cx.notify(),
        }
    }
}

// --- drawing ----------------------------------------------------------------

/// `.rr .more`: the "⋯" button, in the accent while its menu is open, and
/// the menu under it.
pub fn more_cell(
    row: MenuRow,
    id: impl Into<ElementId>,
    app: &PwrRoute,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let open = app.row_actions.menu == Some(row);
    let selector = match row {
        MenuRow::Rule(Selection::Rewrite(i)) => format!("rewrite-{i}-more"),
        MenuRow::Rule(Selection::Handler(i)) => format!("handler-{i}-more"),
        MenuRow::EverythingElse => "everything-else-more".to_owned(),
        MenuRow::Config {
            rewrite: true,
            index,
        } => format!("config-rewrite-{index}-more"),
        MenuRow::Config {
            rewrite: false,
            index,
        } => format!("config-handler-{index}-more"),
    };
    let button = div()
        .id(id)
        .aria_label("More rule actions")
        .aria_expanded(open)
        .debug_selector(move || selector.clone())
        .w(px(20.0))
        .py(px(3.0))
        .flex()
        .justify_center()
        .rounded(px(4.0))
        .cursor_pointer()
        .map(|more| {
            if open {
                more.bg(theme::bg_hover_strong()).child(mono(
                    "\u{22ef}",
                    12.0,
                    600.0,
                    theme::accent_bright(),
                ))
            } else {
                more.hover(|style| style.bg(theme::bg_hover())).child(mono(
                    "\u{22ef}",
                    12.0,
                    600.0,
                    theme::text_subtle(),
                ))
            }
        })
        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
            this.row_actions.menu_was = this.row_actions.menu;
        }))
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            // The menu's keys (Esc, ⌘D, ⌫) reach the list.
            window.focus(&this.focus, cx);
            this.toggle_row_actions(row, cx);
        }));
    let menu = open.then(|| popover(menu_body(row, app, cx), cx));
    div()
        .relative()
        .child(button)
        .children(menu)
        .into_any_element()
}

/// The menu card under the "⋯", kept in the window.
fn popover(body: Vec<AnyElement>, cx: &mut Context<'_, PwrRoute>) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .child(
            deferred(
                anchored()
                    .anchor(Anchor::TopRight)
                    .offset(point(px(0.0), px(24.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(
                        menu_card("rule-menu")
                            .debug_selector(|| "rule-menu".into())
                            .w(px(MENU_WIDTH))
                            .rounded(px(8.0))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.close_row_actions(cx);
                            }))
                            .children(body),
                    ),
            )
            .with_priority(1),
        )
        .into_any_element()
}

/// How a menu item reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Plain,
    /// `.mi.dim`: not possible here.
    Dim,
    /// `.mi.danger`
    Danger,
}

/// `.mi`: label, then the move's count (`.ex`) and the key (`.k`). Not
/// `widgets::menu_row`, which has one hint and one color: a move shows a
/// count and a key, and an item can be dim or danger. The label still ends
/// in an ellipsis at the card's edge.
fn item(
    id: &'static str,
    label: impl Into<SharedString>,
    count: Option<String>,
    key: Option<&'static str>,
    tone: Tone,
) -> Stateful<Div> {
    let color = match tone {
        Tone::Plain => theme::text_primary(),
        Tone::Dim => theme::text_subtle(),
        Tone::Danger => theme::danger_text(),
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(8.0))
        .py(px(6.0))
        .min_w_0()
        .rounded(px(5.0))
        .child(div().flex_1().min_w_0().child(crate::widgets::one_line(
            label,
            theme::SANS,
            500.0,
            12.0,
            14.4,
            color,
        )))
        .when_some(count, |row, count| {
            row.child(
                mono(count, 10.5, 500.0, theme::text_muted())
                    .debug_selector(move || format!("{id}-count")),
            )
        })
        .when_some(key, |row, key| {
            row.child(mono(key, 10.5, 500.0, theme::text_subtle()))
        })
        .when(tone != Tone::Dim, |row| {
            row.cursor_pointer()
                .hover(|style| style.bg(theme::bg_row_active()))
        })
}

/// `.msep`
fn separator() -> AnyElement {
    div()
        .h(px(1.0))
        .mx(px(6.0))
        .my(px(4.0))
        .bg(theme::border_subtle())
        .into_any_element()
}

fn foot(content: String) -> AnyElement {
    div()
        .px(px(8.0))
        .pt(px(6.0))
        .pb(px(4.0))
        .child(paragraph(content, 10.5, 15.0, theme::text_subtle()))
        .into_any_element()
}

fn menu_body(row: MenuRow, app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Vec<AnyElement> {
    match row {
        MenuRow::Rule(selection) => rule_menu(selection, app, cx),
        MenuRow::EverythingElse => everything_else_menu(app, cx),
        MenuRow::Config { .. } => config_menu(app, cx),
    }
}

/// Design 3b: the moves with their counts, Duplicate, Turn off, Test a
/// link with this rule, Delete.
fn rule_menu(
    selection: Selection,
    app: &PwrRoute,
    cx: &mut Context<'_, PwrRoute>,
) -> Vec<AnyElement> {
    let keys = keys(cx);
    let here = position(selection);
    let last = app.store.len_of(selection).saturating_sub(1);
    let count = |to: usize| {
        app.row_actions
            .impacts
            .get(&(selection, to))
            .copied()
            .flatten()
            .map(impact_label)
    };
    let mut items: Vec<AnyElement> = Vec::new();
    let mut step = |id: &'static str, label: &'static str, to: Option<usize>, key| {
        let tone = if to.is_some() { Tone::Plain } else { Tone::Dim };
        let row = item(id, label, to.and_then(count), key, tone);
        items.push(match to {
            Some(to) => row
                .map(crate::accessibility::clickable)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.move_rule(selection, to, window, cx);
                }))
                .into_any_element(),
            None => row.into_any_element(),
        });
    };
    step(
        "rule-menu-top",
        "Move to top",
        (here > 0).then_some(0),
        None,
    );
    step(
        "rule-menu-up",
        "Move up",
        (here > 0).then(|| here - 1),
        Some(keys.up),
    );
    step(
        "rule-menu-down",
        "Move down",
        (here < last).then_some(here + 1),
        Some(keys.down),
    );
    step(
        "rule-menu-bottom",
        "Move to bottom",
        (here < last).then_some(last),
        None,
    );
    items.push(separator());
    items.push(
        item(
            "rule-menu-duplicate",
            "Duplicate",
            None,
            Some(keys.duplicate),
            Tone::Plain,
        )
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.duplicate_rule(selection, window, cx);
        }))
        .into_any_element(),
    );
    let enabled = app.store.enabled(selection).unwrap_or(true);
    items.push(
        item(
            "rule-menu-enabled",
            if enabled { "Turn off" } else { "Turn on" },
            None,
            None,
            Tone::Plain,
        )
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.set_rule_enabled(selection, !enabled, cx);
        }))
        .into_any_element(),
    );
    let (test, delete) = match selection {
        Selection::Rewrite(_) => ("Test a link with this rewrite", "Delete rewrite"),
        Selection::Handler(_) => ("Test a link with this rule", "Delete rule"),
    };
    items.push(
        item("rule-menu-test", test, None, None, Tone::Plain)
            .map(crate::accessibility::clickable)
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.test_rule(selection, window, cx);
            }))
            .into_any_element(),
    );
    items.push(separator());
    items.push(
        item(
            "rule-menu-delete",
            delete,
            None,
            Some(keys.delete),
            Tone::Danger,
        )
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.delete_rule(selection, window, cx);
        }))
        .into_any_element(),
    );
    items
}

/// Everything else's browser and profile, as a select: each browser's
/// default, then its profiles, the current one checked.
fn everything_else_menu(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Vec<AnyElement> {
    let file = app.store.edited();
    let current = crate::default_browser::browser_name(&file.default_browser);
    let current_profile = file.default_profile.clone();
    let mut items: Vec<AnyElement> = vec![
        div()
            .px(px(8.0))
            .pt(px(4.0))
            .pb(px(4.0))
            .child(crate::theme::tracked(
                "Everything else opens in",
                9.5,
                600.0,
                0.12,
                theme::text_subtle(),
            ))
            .into_any_element(),
    ];
    let mut index = 0;
    for browser in &app.browsers {
        let profiles = std::iter::once(None).chain(browser.profiles.iter().map(Some));
        for profile in profiles {
            let chosen = *current == *browser.name
                && profile.map_or(current_profile.is_empty(), |p| *p == current_profile);
            let label = match profile {
                Some(profile) => format!("{} \u{00b7} {profile}", browser.name),
                None => browser.name.clone(),
            };
            let hint = match (chosen, profile) {
                (true, _) => Some("\u{2713}".into()),
                (false, None) if !browser.profiles.is_empty() => Some("its default".into()),
                _ => None,
            };
            let (name, profile) = (browser.name.clone(), profile.cloned().unwrap_or_default());
            items.push(
                menu_row(
                    ElementId::NamedInteger("everything-else-choice".into(), index as u64),
                    Some(badge_for(&name)),
                    label,
                    hint,
                )
                .debug_selector(move || format!("everything-else-choice-{index}"))
                .hover(|row| row.bg(theme::bg_row_active()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.set_everything_else(&name, &profile, cx);
                }))
                .into_any_element(),
            );
            index += 1;
        }
    }
    if app.browsers.is_empty() {
        items.push(foot(
            "No browsers found. Rescan in Settings \u{203a} Browsers.".into(),
        ));
    }
    // Don't open, last and set apart (design: Not Opened 1b): only links a
    // rule chooses reach a browser.
    let dont_open = pwrroute_engine::is_dont_open(&file.default_browser);
    items.push(
        div()
            .mx(px(2.0))
            .my(px(4.0))
            .h(px(1.0))
            .bg(theme::border_faint())
            .into_any_element(),
    );
    items.push(
        menu_row(
            "everything-else-dont-open",
            Some(badge_for(pwrroute_engine::DONT_OPEN)),
            crate::widgets::DONT_OPEN.to_owned(),
            Some(if dont_open {
                "\u{2713}".into()
            } else {
                "only rules open links".into()
            }),
        )
        .debug_selector(|| "everything-else-dont-open".into())
        .hover(|row| row.bg(theme::bg_row_active()))
        .on_click(cx.listener(|this, _, _, cx| {
            cx.stop_propagation();
            this.set_everything_else(pwrroute_engine::DONT_OPEN, "", cx);
        }))
        .into_any_element(),
    );
    items.push(foot(
        "Links no rule chooses wait in Activity, where you can open them or add a rule.".into(),
    ));
    if let Some(reason) = &app.row_actions.refused {
        items.push(foot(format!("Not saved: {reason}")));
    }
    items
}

/// A config's read-only row: its file, and the importer.
fn config_menu(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Vec<AnyElement> {
    let Some(config) = &app.config else {
        return Vec::new();
    };
    let file = config.file.clone();
    let import = config.file.clone();
    vec![
        item(
            "config-menu-open",
            format!("Open {} in editor", config.path),
            None,
            None,
            Tone::Plain,
        )
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.close_row_actions(cx);
            cx.open_with_system(&file);
        }))
        .into_any_element(),
        item(
            "config-menu-import",
            "Import to visual rules\u{2026}",
            None,
            None,
            Tone::Plain,
        )
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.close_row_actions(cx);
            let file = import.clone();
            cx.defer(move |cx| crate::importer_view::open(file, false, cx));
        }))
        .into_any_element(),
        foot(format!(
            "Read-only here: {} is edited in its own file.",
            config.path
        )),
    ]
}

/// What a rule matches, in words: its patterns, "from Slack", "function".
fn matcher_words(rule: &Rule) -> impl Iterator<Item = String> + '_ {
    rule.matchers.iter().filter_map(|matcher| match matcher {
        Matcher::Pattern(p) | Matcher::Regex(p) => Some(p.to_string()),
        Matcher::App { name, .. } => Some(format!("from {name}")),
        Matcher::Function(_) => Some("function".into()),
        Matcher::Label(label) => Some((*label).to_owned()),
        Matcher::Off | Matcher::OffHere | Matcher::Privacy(_) => None,
    })
}

/// `.rr.tomb` (design 4b): where the deleted rule was, with Undo, until
/// the next edit.
///
/// `after_last`: it follows the list's last row, which draws no bottom
/// border, so the tombstone draws the line above itself and none below.
pub fn tombstone(
    deleted: &Deleted,
    after_last: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let keys = keys(cx);
    let what = matcher_words(&deleted.row).next();
    let to = match &deleted.row.destination {
        Destination::Browser { name, profile, .. } => Some(match profile {
            Some(profile) => format!("{name} \u{00b7} {profile}"),
            None => name.to_string(),
        }),
        Destination::Rewrite(changes) => changes.first().map(|c| c.value.to_string()),
        Destination::Function(_) => Some("function".into()),
        Destination::Unset(_) => None,
    };
    div()
        .id("rule-tombstone")
        .debug_selector(|| "rule-tombstone".into())
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(9.0))
        .pl(px(10.0))
        .pr(px(12.0))
        .bg(theme::bg_elevated())
        .border_color(theme::border_faint())
        .map(|row| {
            if after_last {
                row.border_t_1()
            } else {
                row.border_b_1()
            }
        })
        .child(div().w(px(12.0)).flex_none())
        .child(div().w(px(22.0)).flex_none().child(mono(
            "\u{2013}",
            10.5,
            600.0,
            theme::text_subtle(),
        )))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(text("Deleted", 11.5, 500.0, theme::text_muted()))
                .when_some(what, |row, what| row.child(crate::widgets::tag(what)))
                .when_some(to, |row, to| {
                    row.child(mono("\u{2192}", 12.0, 500.0, theme::text_subtle()))
                        .child(
                            crate::widgets::one_line(
                                to,
                                theme::SANS,
                                500.0,
                                11.5,
                                14.0,
                                theme::text_muted(),
                            )
                            .min_w_0(),
                        )
                }),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(text(
                    format!("restores as {}", rule_name(deleted.at)),
                    10.5,
                    500.0,
                    theme::text_subtle(),
                ))
                .child(
                    button("Undo", ButtonKind::Default, true)
                        .id("rule-undo-delete")
                        .debug_selector(|| "rule-undo-delete".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.undo_delete(window, cx);
                        })),
                )
                .child(mono(keys.undo, 10.5, 500.0, theme::text_subtle())),
        )
}

/// What follows the pointer while a row is dragged (`.rr.lift`): its
/// number and what it matches.
pub struct DraggedRule {
    pub selection: Selection,
    label: SharedString,
    destination: Option<(crate::widgets::AppBadge, SharedString)>,
}

impl Render for DraggedRule {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .w(px(380.0))
            .py(px(9.0))
            .px(px(12.0))
            .bg(theme::bg_elevated())
            .border_1()
            .border_color(theme::accent_border())
            .rounded(px(8.0))
            .shadow_lg()
            .child(mono(
                format!("{:02}", position(self.selection) + 1),
                10.5,
                600.0,
                theme::accent_bright(),
            ))
            .child(div().flex_1().min_w_0().child(crate::widgets::one_line(
                self.label.clone(),
                theme::mono_family(500.0),
                500.0,
                11.0,
                14.0,
                theme::text_primary(),
            )))
            .when_some(self.destination.clone(), |row, (icon, name)| {
                row.child(mono("\u{2192}", 12.0, 500.0, theme::text_subtle()))
                    .child(badge(icon))
                    .child(text(name, 12.0, 500.0, theme::text_primary()).whitespace_nowrap())
            })
    }
}

/// A rule row that can be dragged within its list and dropped on.
pub fn draggable(
    row: Stateful<Div>,
    selection: Selection,
    rule: &Rule,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let label: SharedString = matcher_words(rule).collect::<Vec<_>>().join("  ").into();
    let destination = match &rule.destination {
        Destination::Browser { badge, name, .. } => Some((badge.clone(), name.clone())),
        _ => None,
    };
    let app = cx.entity().downgrade();
    row.on_drag(
        DraggedRule {
            selection,
            label,
            destination,
        },
        move |dragged, _, window, cx| {
            let _ = app.update(cx, |this, cx| {
                window.focus(&this.focus, cx);
                this.row_actions.menu = None;
                this.row_actions.drop = None;
                this.row_actions.forget_counts();
                this.row_actions.dragging = Some(dragged.selection);
                cx.notify();
            });
            cx.new(|_| DraggedRule {
                selection: dragged.selection,
                label: dragged.label.clone(),
                destination: dragged.destination.clone(),
            })
        },
    )
    .on_drag_move(
        cx.listener(move |this, event: &DragMoveEvent<DraggedRule>, _, cx| {
            let pointer = event.event.position;
            if !event.bounds.contains(&pointer) {
                return;
            }
            let dragged = event.drag(cx).selection;
            let above = pointer.y < event.bounds.center().y;
            this.drag_over_row(dragged, selection, above, cx);
        }),
    )
    .on_drop(cx.listener(move |this, dragged: &DraggedRule, window, cx| {
        this.drop_row(dragged.selection, selection, window, cx);
    }))
}

/// The line where a dragged row would land, on the row it is over, and the
/// tip that says what the drop does (design 3a).
pub fn drop_marker(app: &PwrRoute, selection: Selection, cx: &App) -> Option<AnyElement> {
    let drop = app.row_actions.drop.filter(|drop| {
        cx.has_active_drag()
            && same_list(drop.from, selection)
            && drop.over == position(selection)
            && drop.to != position(drop.from)
    })?;
    let count = app
        .row_actions
        .impacts
        .get(&(drop.from, drop.to))
        .copied()
        .flatten();
    let line = div()
        .absolute()
        .left(px(8.0))
        .right(px(8.0))
        .h(px(2.0))
        .rounded(px(1.0))
        .bg(theme::accent())
        .map(|line| {
            if drop.above {
                line.top(px(-1.0))
            } else {
                line.bottom(px(-1.0))
            }
        });
    let tip = div()
        .debug_selector(|| "rule-drop-tip".into())
        .flex()
        .flex_col()
        .gap(px(3.0))
        .py(px(8.0))
        .px(px(10.0))
        .bg(theme::bg_elevated())
        .border_1()
        .border_color(theme::accent_border())
        .rounded(px(8.0))
        .shadow_lg()
        .child(text(
            format!("Drop as {}", rule_name(at(drop.from, drop.to))),
            11.5,
            600.0,
            theme::text_primary(),
        ))
        .when_some(count.zip(app.row_actions.weighed()), |tip, (count, of)| {
            tip.child(text(
                format!("Changes {count} of the last {of} links."),
                10.5,
                500.0,
                theme::text_muted(),
            ))
        })
        .child(text("Esc puts it back.", 10.5, 500.0, theme::text_subtle()));
    let tip = div()
        .absolute()
        .right(px(36.0))
        .map(|corner| {
            if drop.above {
                corner.top(px(4.0))
            } else {
                corner.bottom(px(4.0))
            }
        })
        .child(
            deferred(
                anchored()
                    .anchor(if drop.above {
                        Anchor::TopRight
                    } else {
                        Anchor::BottomRight
                    })
                    .snap_to_window_with_margin(px(8.0))
                    .child(tip),
            )
            .with_priority(2),
        );
    Some(
        div()
            .absolute()
            .inset_0()
            .child(line)
            .child(tip)
            .into_any_element(),
    )
}

/// The other list dims while a row is lifted: rows move only within
/// their own list.
pub fn dimmed_while_dragging(app: &PwrRoute, rewrites: bool, cx: &App) -> bool {
    cx.has_active_drag()
        && app
            .row_actions
            .dragging
            .is_some_and(|dragged| matches!(dragged, Selection::Rewrite(_)) != rewrites)
}

/// The row being dragged, faded where it was.
pub fn lifted(app: &PwrRoute, selection: Selection, cx: &App) -> bool {
    cx.has_active_drag() && app.row_actions.dragging == Some(selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_follows_a_move() {
        // 0 1 2 3 4, row 1 to 3: 0 2 3 1 4.
        assert_eq!(
            (0..5).map(|i| shifted(i, 1, 3)).collect::<Vec<_>>(),
            [0, 3, 1, 2, 4]
        );
        // Row 3 to 0: 3 0 1 2 4.
        assert_eq!(
            (0..5).map(|i| shifted(i, 3, 0)).collect::<Vec<_>>(),
            [1, 2, 3, 0, 4]
        );
    }

    #[test]
    fn impact_labels() {
        assert_eq!(impact_label(0), "no recent links change");
        assert_eq!(impact_label(1), "1 link changes");
        assert_eq!(impact_label(23), "23 links change");
    }
}
