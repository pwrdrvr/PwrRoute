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
use super::model::{SendMode, TrayModel};

/// What the tray reported, for the main thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrayEvent {
    /// A left click on the icon, with the icon's position and size in
    /// physical pixels (macOS: from the primary screen's top-left).
    Click {
        position: (f64, f64),
        size: (f64, f64),
    },
    /// A native menu item.
    Command(TrayCommand),
}

/// One line of the native menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item(TrayCommand, String),
    /// A checkable item and whether it is checked.
    Check(TrayCommand, String, bool),
    Separator,
}

/// The design's 1c with Send links' rows (Send Links 1g), in order.
/// macOS's right-click menu is the same list. The send rows name the
/// destination, so their words change with it.
pub fn menu_entries(model: &TrayModel) -> Vec<MenuEntry> {
    use MenuEntry::{Check, Item, Separator};
    let quit = if cfg!(target_os = "macos") {
        "Quit PwrRoute"
    } else {
        "Quit"
    };
    let mode = model.send_mode();
    let to = model.destination.display();
    vec![
        Item(TrayCommand::OpenMain, "Open PwrRoute".into()),
        Item(TrayCommand::TestLink, "Test a link\u{2026}".into()),
        Separator,
        Check(
            TrayCommand::SendRules,
            "Route by rules".into(),
            mode == Some(SendMode::Rules),
        ),
        Check(
            TrayCommand::SendNext,
            format!("Next link \u{2192} {to}"),
            mode == Some(SendMode::Next),
        ),
        Check(
            TrayCommand::SendAll,
            format!("All links \u{2192} {to}"),
            mode == Some(SendMode::All),
        ),
        Item(TrayCommand::ChooseWhere, "Choose where\u{2026}".into()),
        Separator,
        Check(
            TrayCommand::TogglePause,
            "Pause routing".into(),
            model.is_paused(),
        ),
        Separator,
        Item(TrayCommand::Settings, "Settings\u{2026}".into()),
        Item(TrayCommand::About, "About PwrRoute".into()),
        Separator,
        Item(TrayCommand::Quit, quit.into()),
    ]
}

/// The live tray icon. Dropping it removes the icon.
pub struct NativeTray {
    icon: TrayIcon,
    /// Every checkable row, by its command: their checks and words follow
    /// the model.
    checks: HashMap<TrayCommand, CheckMenuItem>,
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
        for entry in menu_entries(model) {
            let item: Box<dyn IsMenuItem> = match entry {
                MenuEntry::Item(command, label) => {
                    Box::new(MenuItem::with_id(command.menu_id(), label, true, None))
                }
                MenuEntry::Check(command, label, checked) => {
                    let item =
                        CheckMenuItem::with_id(command.menu_id(), label, true, checked, None);
                    checks.insert(command, item.clone());
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
            .with_menu(Box::new(menu))
            // The left click is ours: the dropdown on macOS, the window
            // elsewhere. The menu is the right click.
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self { icon, checks })
    }

    /// A left click on the icon where it is now, as tray-icon would report
    /// it. `None` where the platform does not say (Linux).
    pub fn click_here(&self) -> Option<TrayEvent> {
        let rect = self.icon.rect()?;
        Some(TrayEvent::Click {
            position: (rect.position.x, rect.position.y),
            size: (f64::from(rect.size.width), f64::from(rect.size.height)),
        })
    }

    /// Bring the menu's check marks, the send rows' words and the tooltip
    /// up to date.
    pub fn sync(&self, model: &TrayModel) {
        for entry in menu_entries(model) {
            if let MenuEntry::Check(command, label, checked) = entry
                && let Some(item) = self.checks.get(&command)
            {
                item.set_checked(checked);
                item.set_text(label);
            }
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
                MenuEntry::Item(_, label) | MenuEntry::Check(_, label, _) => label.as_str(),
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
                "Next link \u{2192} Microsoft Edge \u{00b7} Work",
                "All links \u{2192} Microsoft Edge \u{00b7} Work",
                "Choose where\u{2026}",
                "-",
                "Pause routing",
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
            "no send row while paused"
        );
        model.routing = super::super::model::Routing::Sending(pwrroute_router::SendScope::Next {
            left: 1,
            of: 1,
        });
        assert_eq!(checked(&model), [TrayCommand::SendNext]);
    }

    #[test]
    fn every_menu_command_round_trips_through_its_id() {
        let model = TrayModel::fixture(SystemTime::now());
        for entry in menu_entries(&model) {
            if let MenuEntry::Item(command, _) | MenuEntry::Check(command, _, _) = entry {
                assert_eq!(TrayCommand::from_menu_id(command.menu_id()), Some(command));
            }
        }
        assert_eq!(TrayCommand::from_menu_id("something-else"), None);
    }
}
