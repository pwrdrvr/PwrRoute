//! The OS tray icon and its native menu, through the `tray-icon` crate.
//!
//! macOS: an `NSStatusItem` with the template mark. A left click opens the
//! GPUI dropdown under it (see `dropdown`); a right click opens the native
//! menu. Windows and Linux: the notification-area icon, whose native context
//! menu is the design's 1c, without recents; a left click opens the window.
//! Linux speaks StatusNotifierItem over D-Bus (`ksni`), which GNOME shows
//! only with the AppIndicator extension, as for every SNI app.
//!
//! tray-icon reports clicks and menu picks on its own threads (the main
//! thread on macOS and Windows, a D-Bus thread on Linux). They are forwarded
//! as [`TrayEvent`]s over a channel for GPUI's main thread to act on.

use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use std::collections::HashMap;

use super::TrayCommand;
use super::icon;
use super::model::{Mode, Routing, TrayModel};
use pwrroute_router::SendScope;

/// What the tray reported, for the main thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayEvent {
    /// A left click on the icon, with the icon's position and size in
    /// physical pixels (macOS: from the primary screen's top-left).
    /// `pointer` is where the pointer pressed it (macOS: AppKit's global
    /// space, in points); `None` for [`NativeTray::click_here`], whose
    /// pointer can be anywhere, and off macOS.
    Click {
        position: (f64, f64),
        size: (f64, f64),
        pointer: Option<(f64, f64)>,
    },
    /// A native menu item.
    Command(TrayCommand),
}

/// Where the pointer is as the item is pressed, read on the main thread
/// before the click is queued: the dropdown opens after refreshes that can
/// take long enough for the pointer to leave the item.
#[cfg(target_os = "macos")]
fn pressed_at() -> Option<(f64, f64)> {
    Some(crate::macos::pointer_location())
}

#[cfg(not(target_os = "macos"))]
fn pressed_at() -> Option<(f64, f64)> {
    None
}

/// One line of the native menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item(TrayCommand, String),
    /// A checkable item and whether it is checked.
    Check(TrayCommand, String, bool),
    /// "Held links (N)…", enabled while links wait.
    Held(String, bool),
    /// Sync's row (design: Synced Rules 3c): its words, and whether it
    /// acts (only a conflict's opens Settings › Cloud Sync).
    Sync(String, bool),
    Separator,
}

/// The design's 1c with the Incoming links rows (Incoming Links 2g), in
/// order. macOS's right-click menu is the same list. The send row names
/// the destination, so its words change with it; Choose… opens the
/// Overview for anything else.
pub fn menu_entries(model: &TrayModel) -> Vec<MenuEntry> {
    use MenuEntry::{Check, Held, Item, Separator};
    let quit = if cfg!(target_os = "macos") {
        "Quit PwrRoute"
    } else {
        "Quit"
    };
    let mode = model.mode();
    let to = model.destination.display();
    let held = model.held.len();
    let sync = model.sync.iter().flat_map(|row| {
        [
            MenuEntry::Sync(row.menu_label(), row.actionable()),
            Separator,
        ]
    });
    sync.chain([
        Item(TrayCommand::OpenMain, "Open PwrRoute".into()),
        Item(TrayCommand::TestLink, "Test a link\u{2026}".into()),
        Separator,
        Check(
            TrayCommand::SendRules,
            "Route by rules".into(),
            mode == Mode::Rules,
        ),
        Check(
            TrayCommand::SendNext,
            format!("Send next link \u{2192} {to}"),
            mode == Mode::Send,
        ),
        Check(
            TrayCommand::HoldNext,
            "Hold next link".into(),
            matches!(model.routing, Routing::Holding(SendScope::Next { .. })),
        ),
        Check(
            TrayCommand::HoldHour,
            "Hold for 1 hour".into(),
            matches!(model.routing, Routing::Holding(SendScope::All { .. })),
        ),
        Check(
            TrayCommand::TogglePause,
            "Pause routing".into(),
            mode == Mode::Pause,
        ),
        Item(TrayCommand::ChooseWhere, "Choose\u{2026}".into()),
        Separator,
        Held(
            if held == 0 {
                "Held links\u{2026}".to_owned()
            } else {
                format!("Held links ({held})\u{2026}")
            },
            held > 0,
        ),
        Separator,
        Item(TrayCommand::Settings, "Settings\u{2026}".into()),
        Item(TrayCommand::About, "About PwrRoute".into()),
        Separator,
        Item(TrayCommand::Quit, quit.into()),
    ])
    .collect()
}

/// The live tray icon. Dropping it removes the icon.
pub struct NativeTray {
    icon: TrayIcon,
    /// Every checkable row, by its command: their checks and words follow
    /// the model.
    checks: HashMap<TrayCommand, CheckMenuItem>,
    /// "Held links (N)…": its count and whether it is enabled follow the
    /// model.
    held: Option<MenuItem>,
    /// Sync's first row and the separator under it, put in and taken out
    /// as rules start and stop syncing.
    menu: Menu,
    sync_row: MenuItem,
    sync_separator: PredefinedMenuItem,
    sync_shown: std::cell::Cell<bool>,
}

impl NativeTray {
    /// Put the icon up and forward its events to `events`. Call on the main
    /// thread once the app has finished launching.
    pub fn create(
        model: &TrayModel,
        events: async_channel::Sender<TrayEvent>,
    ) -> anyhow::Result<Self> {
        let menu = Menu::new();
        let mut checks = HashMap::new();
        let mut held = None;
        let sync_row = MenuItem::with_id(TrayCommand::Sync.menu_id(), "", false, None);
        let sync_separator = PredefinedMenuItem::separator();
        let mut sync_shown = false;
        let mut after_sync = false;
        for entry in menu_entries(model) {
            let item: Box<dyn IsMenuItem> = match entry {
                MenuEntry::Sync(label, enabled) => {
                    sync_row.set_text(label);
                    sync_row.set_enabled(enabled);
                    sync_shown = true;
                    after_sync = true;
                    Box::new(sync_row.clone())
                }
                MenuEntry::Separator if after_sync => {
                    after_sync = false;
                    Box::new(sync_separator.clone())
                }
                MenuEntry::Item(command, label) => {
                    Box::new(MenuItem::with_id(command.menu_id(), label, true, None))
                }
                MenuEntry::Check(command, label, checked) => {
                    let item =
                        CheckMenuItem::with_id(command.menu_id(), label, true, checked, None);
                    checks.insert(command, item.clone());
                    Box::new(item)
                }
                MenuEntry::Held(label, enabled) => {
                    let item =
                        MenuItem::with_id(TrayCommand::HeldLinks.menu_id(), label, enabled, None);
                    held = Some(item.clone());
                    Box::new(item)
                }
                MenuEntry::Separator => Box::new(PredefinedMenuItem::separator()),
            };
            menu.append(item.as_ref())?;
        }

        let menu_events = events.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = TrayCommand::from_menu_id(event.id().as_ref()) {
                let _ = menu_events.try_send(TrayEvent::Command(command));
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state,
                rect,
                ..
            } = event
            {
                // macOS reports the press and the release; open on the press,
                // as a menu does. Elsewhere only the release comes.
                let wanted = if cfg!(target_os = "macos") {
                    MouseButtonState::Down
                } else {
                    MouseButtonState::Up
                };
                if button_state == wanted {
                    let _ = events.try_send(TrayEvent::Click {
                        position: (rect.position.x, rect.position.y),
                        size: (f64::from(rect.size.width), f64::from(rect.size.height)),
                        pointer: pressed_at(),
                    });
                }
            }
        }));

        let bitmap = if cfg!(target_os = "macos") {
            icon::template()
        } else {
            icon::colored()
        };
        let image = Icon::from_rgba(bitmap.rgba, bitmap.width, bitmap.height)?;
        let builder = TrayIconBuilder::new().with_id("pwrroute");
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(image);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(image);
        let icon = builder
            .with_tooltip(model.tooltip())
            .with_menu(Box::new(menu.clone()))
            // The left click is ours: the dropdown on macOS, the window
            // elsewhere. The menu is the right click.
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self {
            icon,
            checks,
            held,
            menu,
            sync_row,
            sync_separator,
            sync_shown: std::cell::Cell::new(sync_shown),
        })
    }

    /// A left click on the icon where it is now, as tray-icon would report
    /// it. `None` where the platform does not say (Linux).
    pub fn click_here(&self) -> Option<TrayEvent> {
        let rect = self.icon.rect()?;
        Some(TrayEvent::Click {
            position: (rect.position.x, rect.position.y),
            size: (f64::from(rect.size.width), f64::from(rect.size.height)),
            pointer: None,
        })
    }

    /// Bring the menu's check marks, the send row's words, the held count
    /// and the tooltip up to date.
    pub fn sync(&self, model: &TrayModel) {
        for entry in menu_entries(model) {
            match entry {
                MenuEntry::Check(command, label, checked) => {
                    if let Some(item) = self.checks.get(&command) {
                        item.set_checked(checked);
                        item.set_text(label);
                    }
                }
                MenuEntry::Held(label, enabled) => {
                    if let Some(item) = &self.held {
                        item.set_text(label);
                        item.set_enabled(enabled);
                    }
                }
                MenuEntry::Item(..) | MenuEntry::Sync(..) | MenuEntry::Separator => {}
            }
        }
        match &model.sync {
            Some(row) => {
                self.sync_row.set_text(row.menu_label());
                self.sync_row.set_enabled(row.actionable());
                if !self.sync_shown.get() {
                    let _ = self.menu.insert(&self.sync_row, 0);
                    let _ = self.menu.insert(&self.sync_separator, 1);
                    self.sync_shown.set(true);
                }
            }
            None if self.sync_shown.get() => {
                let _ = self.menu.remove(&self.sync_row);
                let _ = self.menu.remove(&self.sync_separator);
                self.sync_shown.set(false);
            }
            None => {}
        }
        let _ = self.icon.set_tooltip(Some(model.tooltip()));
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    #[test]
    fn the_native_menu_is_the_design_list() {
        let mut model = TrayModel::fixture(SystemTime::now());
        let entries = menu_entries(&model);
        let labels: Vec<&str> = entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(_, label)
                | MenuEntry::Check(_, label, _)
                | MenuEntry::Held(label, _)
                | MenuEntry::Sync(label, _) => label.as_str(),
                MenuEntry::Separator => "-",
            })
            .collect();
        let quit = if cfg!(target_os = "macos") {
            "Quit PwrRoute"
        } else {
            "Quit"
        };
        assert_eq!(
            labels,
            [
                "Open PwrRoute",
                "Test a link\u{2026}",
                "-",
                "Route by rules",
                "Send next link \u{2192} Microsoft Edge \u{00b7} Work",
                "Hold next link",
                "Hold for 1 hour",
                "Pause routing",
                "Choose\u{2026}",
                "-",
                "Held links\u{2026}",
                "-",
                "Settings\u{2026}",
                "About PwrRoute",
                "-",
                quit,
            ]
        );
        let checked = |model: &TrayModel| -> Vec<TrayCommand> {
            menu_entries(model)
                .into_iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Check(command, _, true) => Some(command),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(checked(&model), [TrayCommand::SendRules]);
        model.set_paused(true);
        assert_eq!(
            checked(&model),
            [TrayCommand::TogglePause],
            "one row checked: the slot holds one"
        );
        model.routing = Routing::Sending(SendScope::Next { left: 1, of: 1 });
        assert_eq!(checked(&model), [TrayCommand::SendNext]);
        model.routing = Routing::Holding(SendScope::Next { left: 2, of: 3 });
        assert_eq!(checked(&model), [TrayCommand::HoldNext]);
        let held = |model: &TrayModel| {
            menu_entries(model)
                .into_iter()
                .find_map(|entry| match entry {
                    MenuEntry::Held(label, enabled) => Some((label, enabled)),
                    _ => None,
                })
        };
        assert_eq!(held(&model), Some(("Held links\u{2026}".to_owned(), false)));
        let holding = TrayModel::holding_fixture(SystemTime::now());
        assert_eq!(checked(&holding), [TrayCommand::HoldHour]);
        assert_eq!(
            held(&holding),
            Some(("Held links (2)\u{2026}".to_owned(), true))
        );
    }

    #[test]
    fn syncing_puts_its_row_first_and_only_a_conflict_acts() {
        use super::super::model::{SyncRow, SyncTone};
        let mut model = TrayModel::fixture(SystemTime::now());
        model.sync = Some(SyncRow {
            title: "Rules synced 2 min ago".into(),
            sub: Some("Dropbox, iCloud".into()),
            tone: SyncTone::Ok,
        });
        let entries = menu_entries(&model);
        assert_eq!(
            entries[..2],
            [
                MenuEntry::Sync(
                    "Rules synced 2 min ago \u{00b7} Dropbox, iCloud".into(),
                    false
                ),
                MenuEntry::Separator,
            ]
        );
        assert_eq!(
            entries[2],
            MenuEntry::Item(TrayCommand::OpenMain, "Open PwrRoute".into())
        );
        model.sync = Some(SyncRow {
            title: "Sync conflict".into(),
            sub: Some("Resolve\u{2026}".into()),
            tone: SyncTone::Bad,
        });
        assert!(matches!(menu_entries(&model)[0], MenuEntry::Sync(_, true)));
    }

    #[test]
    fn every_menu_command_round_trips_through_its_id() {
        let model = TrayModel::fixture(SystemTime::now());
        for entry in menu_entries(&model) {
            if let MenuEntry::Item(command, _) | MenuEntry::Check(command, _, _) = entry {
                assert_eq!(TrayCommand::from_menu_id(command.menu_id()), Some(command));
            }
        }
        assert_eq!(
            TrayCommand::from_menu_id(TrayCommand::HeldLinks.menu_id()),
            Some(TrayCommand::HeldLinks)
        );
        assert_eq!(TrayCommand::from_menu_id("something-else"), None);
    }
}
