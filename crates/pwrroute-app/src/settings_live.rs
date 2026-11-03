//! The Settings window as the shipped app opens it: every pane from its
//! producer, and every request to the producer that answers it. `main.rs`
//! installs this as the [`SettingsHost`](crate::settings::SettingsHost);
//! the fixtures are left to the design canvas and the tests. Outside
//! `settings/` because it reaches the MCP door (`mcp.rs`), which the
//! `screens` example does not compile.

use gpui_kit::{App, Window};

use crate::settings::model::{SettingsModel, SettingsRequest};
use crate::settings::{
    browsers_host, configuration_host, privacy_host, sync_host, system_host, updates_host,
};
use crate::{appearance, default_browser, mcp};

/// Every pane, read fresh. The default-browser chip follows the window from
/// here on, as the main window's does, and Configuration reads its files
/// again when the window comes to the front after another writer changed
/// them.
pub fn model(window: &mut Window, cx: &mut App) -> SettingsModel {
    default_browser::watch(window, cx);
    appearance::watch(window);
    configuration_host::follow(window, cx);
    let local_agents = mcp::local_agents(cx);
    SettingsModel {
        pending: [crate::settings::Pane::Browsers, crate::settings::Pane::Configuration,
            crate::settings::Pane::Privacy, crate::settings::Pane::Sync,
            crate::settings::Pane::Diagnostics, crate::settings::Pane::Updates].into(),
        problems: Default::default(),
        general: appearance::general(cx),
        browsers: browsers_host::initial(cx),
        configuration: configuration_host::initial(),
        sync: Default::default(),
        privacy: privacy_host::build(None, None, None, None),
        updates: updates_host::initial(),
        experimental: system_host::experimental(cx),
        diagnostics: system_host::build_diagnostics(&Default::default()),
        about: system_host::about(),
        local_agents,
    }
}

/// Hands `request` to the first producer that answers it. The one request
/// no producer takes is logged: nothing acts on the OS by default.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) {
    if let SettingsRequest::LoadPane(pane) = request {
        use crate::settings::{self, Pane};
        let pane = pane.load_key();
        cx.defer(move |cx| match pane {
            Pane::Browsers => browsers_host::load(cx),
            Pane::Configuration => configuration_host::load(cx),
            Pane::Privacy => privacy_host::load(cx),
            Pane::Sync => sync_host::load(cx),
            Pane::Diagnostics => system_host::load(cx),
            Pane::Updates => settings::load(cx, Pane::Updates, updates_host::model,
                |model, updates| model.updates = updates),
            _ => {}
        });
        return;
    }
    let handled = appearance::handle_settings_request(request, cx)
        || default_browser::handle_settings_request(request, cx)
        || privacy_host::handle_request(request, cx)
        || mcp::handle_settings_request(request, cx)
        || browsers_host::handle_request(request, cx)
        || configuration_host::handle_request(request, cx)
        || sync_host::handle_request(request, cx)
        || updates_host::handle_request(request, cx)
        || system_host::handle_request(request, cx);
    if !handled {
        log::warn!("settings: {request:?} is not wired up yet");
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use gpui_kit::TestAppContext;
    use crate::{browser_scan, settings};

    #[gpui_kit::test]
    fn opening_about_does_not_scan_browsers(cx: &mut TestAppContext) {
        cx.update(|cx| {
            browser_scan::install(pwrroute_browsers::Env::fixture(
                pwrroute_browsers::Platform::current(), std::env::temp_dir()), cx);
            settings::install(cx);
            cx.set_global(settings::SettingsHost {
                model: Rc::new(super::model),
                on_request: Rc::new(|request, _, cx| super::handle_request(request, cx)),
            });
            settings::open(cx, Some(settings::Pane::About));
            assert!(!browser_scan::has_run(cx), "About must not run another pane's browser scan");
        });
    }
}
