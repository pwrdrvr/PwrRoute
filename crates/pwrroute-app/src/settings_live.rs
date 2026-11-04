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

/// A cheap initial model. Expensive panes stay pending until selected;
/// producers capture GPUI data and build their snapshots in the background.
pub fn model(window: &mut Window, cx: &mut App) -> SettingsModel {
    default_browser::watch(window, cx);
    appearance::watch(window);
    configuration_host::follow(window, cx);
    let local_agents = mcp::local_agents(cx);
    SettingsModel {
        pending: [
            crate::settings::Pane::Browsers,
            crate::settings::Pane::Configuration,
            crate::settings::Pane::Privacy,
            crate::settings::Pane::ShortLinks,
            crate::settings::Pane::Sync,
            crate::settings::Pane::Diagnostics,
            crate::settings::Pane::Updates,
        ]
        .into(),
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
    if let SettingsRequest::RetryPane(pane) = request {
        if *pane == crate::settings::Pane::Browsers {
            crate::browser_scan::rescan(cx);
        }
        crate::settings::reload(cx, *pane);
        return;
    }
    if let SettingsRequest::LoadPane(pane) = request {
        use crate::settings::{self, Pane};
        let pane = *pane;
        cx.defer(move |cx| match pane {
            Pane::Browsers => browsers_host::load(cx),
            Pane::Configuration => configuration_host::load(cx),
            Pane::ShortLinks => configuration_host::load_short_links(cx),
            Pane::Privacy => privacy_host::load(cx),
            Pane::Sync => sync_host::load(cx),
            Pane::Diagnostics => system_host::load(cx),
            Pane::Updates => {
                settings::load(cx, Pane::Updates, updates_host::model, |model, updates| {
                    model.updates = updates
                })
            }
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
    use crate::{browser_scan, settings};
    use gpui_kit::TestAppContext;
    use std::rc::Rc;

    #[gpui_kit::test]
    fn opening_about_does_not_scan_browsers(cx: &mut TestAppContext) {
        cx.update(|cx| {
            browser_scan::install(
                pwrroute_browsers::Env::fixture(
                    pwrroute_browsers::Platform::current(),
                    std::env::temp_dir(),
                ),
                cx,
            );
            settings::install(cx);
            cx.set_global(settings::SettingsHost {
                model: Rc::new(super::model),
                on_request: Rc::new(|request, _, cx| super::handle_request(request, cx)),
            });
            settings::open(cx, Some(settings::Pane::About));
            assert!(
                !browser_scan::has_run(cx),
                "About must not run another pane's browser scan"
            );
        });
    }

    #[gpui_kit::test]
    fn short_links_loads_without_starting_the_browser_or_configuration_panes(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            browser_scan::install(
                pwrroute_browsers::Env::fixture(
                    pwrroute_browsers::Platform::current(),
                    std::env::temp_dir(),
                ),
                cx,
            );
            settings::install(cx);
            cx.set_global(settings::SettingsHost {
                model: Rc::new(super::model),
                on_request: Rc::new(|request, _, cx| super::handle_request(request, cx)),
            });
            settings::open(cx, Some(settings::Pane::ShortLinks));
            super::handle_request(
                &settings::model::SettingsRequest::LoadPane(settings::Pane::ShortLinks),
                cx,
            );
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!browser_scan::is_loading(cx));
            assert!(!browser_scan::has_run(cx));
            let handle = cx
                .windows()
                .into_iter()
                .find_map(|window| window.downcast::<settings::SettingsWindow>())
                .unwrap();
            handle
                .update(cx, |view, _, _| {
                    assert!(!view.model.pending.contains(&settings::Pane::ShortLinks));
                    assert!(view.model.pending.contains(&settings::Pane::Configuration));
                })
                .unwrap();
        });
    }
}
