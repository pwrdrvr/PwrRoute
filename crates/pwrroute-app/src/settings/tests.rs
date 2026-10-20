//! The Settings window through gpui-kit's harness: open it the ways a
//! reader does, switch panes, and check what reaches the request handler.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle,
};

use super::local_agents::{ACCESS_SWITCH_ID, COLLAPSE_ALL_ID};
use super::model::{RoleId, SessionId, SettingsRequest};
use super::{Pane, SettingsHost, SettingsWindow, browsers, configuration, fixtures};
use crate::app::PwrRoute;
use crate::chrome::OPEN_SETTINGS_ID;

type Requests = Rc<RefCell<Vec<SettingsRequest>>>;

/// Install the shortcut and a handler that records every request.
fn install(cx: &mut TestAppContext) -> Requests {
    let requests = Requests::default();
    let recorded = requests.clone();
    cx.update(|cx| {
        crate::assets::load_fonts(cx);
        super::install(cx);
        cx.set_global(SettingsHost {
            on_request: Rc::new(move |request, _, _| recorded.borrow_mut().push(request.clone())),
        });
    });
    requests
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Every open Settings window.
fn settings_windows(cx: &mut TestAppContext) -> Vec<WindowHandle<SettingsWindow>> {
    cx.update(|cx| {
        cx.windows()
            .into_iter()
            .filter_map(|window| window.downcast::<SettingsWindow>())
            .collect()
    })
}

/// A Settings window over `model`, opened directly, ready to click.
fn settings(
    cx: &mut TestAppContext,
    edit: impl FnOnce(&mut super::model::SettingsModel),
) -> (Entity<SettingsWindow>, &mut VisualTestContext, Requests) {
    let requests = install(cx);
    let recorded = requests.clone();
    let mut model = fixtures::model();
    edit(&mut model);
    let (view, cx) = cx.add_window_view(move |_, cx| {
        SettingsWindow::new(
            model,
            Rc::new(move |request, _, _| recorded.borrow_mut().push(request.clone())),
            cx,
        )
    });
    draw(cx);
    (view, cx, requests)
}

fn click(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) {
    let id = id.into();
    cx.update(|window, cx| window.click(id, cx));
}

fn visible(cx: &mut VisualTestContext, id: &'static str) -> bool {
    cx.update(|window, _| window.try_find(id).is_some_and(|found| found.visible()))
}

#[gpui_kit::test]
fn the_rail_item_opens_one_settings_window(cx: &mut TestAppContext) {
    install(cx);
    let (_main, main) = cx.add_window_view(|_, cx| PwrRoute::new(cx));
    draw(main);

    let item = main
        .debug_bounds(OPEN_SETTINGS_ID)
        .expect("Open Settings item");
    main.simulate_click(item.center(), Modifiers::none());
    assert_eq!(settings_windows(main).len(), 1);

    // A second click brings the same window forward rather than a twin.
    draw(main);
    main.simulate_click(item.center(), Modifiers::none());
    assert_eq!(settings_windows(main).len(), 1);
}

#[gpui_kit::test]
fn the_shortcut_opens_settings_once(cx: &mut TestAppContext) {
    install(cx);
    let (_main, main) = cx.add_window_view(|_, cx| PwrRoute::new(cx));
    draw(main);

    main.simulate_keystrokes(super::SHORTCUT);
    assert_eq!(settings_windows(main).len(), 1);
    main.simulate_keystrokes(super::SHORTCUT);
    assert_eq!(settings_windows(main).len(), 1);
}

#[gpui_kit::test]
fn the_nav_switches_panes(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| super::open(cx, None));
    let handle = settings_windows(cx)[0];
    let any: AnyWindowHandle = handle.into();
    let cx = &mut VisualTestContext::from_window(any, cx);
    draw(cx);

    let pane = |cx: &mut VisualTestContext| {
        handle
            .read_with(cx, |view, _| view.pane)
            .expect("the Settings window is open")
    };
    assert_eq!(pane(cx), Pane::Browsers);
    assert!(visible(cx, Pane::Browsers.pane_id()));

    for target in [
        Pane::Configuration,
        Pane::LocalAgents,
        Pane::General,
        Pane::Browsers,
    ] {
        click(cx, target.nav_id());
        assert_eq!(pane(cx), target);
        assert!(visible(cx, target.pane_id()), "{target:?} is drawn");
        for other in Pane::ALL.into_iter().filter(|pane| *pane != target) {
            assert!(!visible(cx, other.pane_id()), "{other:?} is not drawn");
        }
    }
}

#[gpui_kit::test]
fn opening_settings_at_a_pane_switches_the_open_window(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| super::open(cx, None));
    cx.update(|cx| super::open(cx, Some(Pane::LocalAgents)));
    let windows = settings_windows(cx);
    assert_eq!(windows.len(), 1);
    let pane = windows[0]
        .read_with(cx, |view, _| view.pane)
        .expect("the Settings window is open");
    assert_eq!(pane, Pane::LocalAgents);
}

#[gpui_kit::test]
fn register_calls_the_callback_and_nothing_else(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |model| {
        model.browsers.default.is_default = false;
        model.browsers.default.confirmed = None;
    });
    click(cx, browsers::REGISTER_ID);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::RegisterDefaultBrowser]
    );
    // The window waits for the OS to say so; it does not claim success.
    assert!(!view.read_with(cx, |view, _| view.model.browsers.default.is_default));
}

#[gpui_kit::test]
fn register_is_inert_while_pwrroute_is_the_default(cx: &mut TestAppContext) {
    let (_view, cx, requests) = settings(cx, |_| {});
    click(cx, browsers::REGISTER_ID);
    assert!(requests.borrow().is_empty());
}

#[gpui_kit::test]
fn a_role_is_chosen_from_the_select(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::LocalAgents.nav_id());

    click(cx, ("agents-role", 0_usize));
    assert_eq!(
        view.read_with(cx, |view, _| view.agents.open_menu()),
        Some(SessionId("s1".into()))
    );

    click(cx, ("agents-role-option", 2_usize));
    assert_eq!(view.read_with(cx, |view, _| view.agents.open_menu()), None);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetSessionRole {
            session: SessionId("s1".into()),
            role: RoleId("editor".into()),
        }]
    );
}

#[gpui_kit::test]
fn a_role_follows_its_session_when_the_list_changes(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::LocalAgents.nav_id());
    click(cx, ("agents-role", 1_usize));

    // The producer drops the first session while PwrAgnt's menu is open.
    view.update(cx, |view, cx| {
        view.model.local_agents.sessions.remove(0);
        cx.notify();
    });
    draw(cx);
    click(cx, ("agents-role-option", 0_usize));
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetSessionRole {
            session: SessionId("s2".into()),
            role: RoleId("reader".into()),
        }]
    );
}

#[gpui_kit::test]
fn keep_the_config_is_offered_only_while_rules_stay_js(cx: &mut TestAppContext) {
    let (_view, cx, _) = settings(cx, |model| {
        if let Some(plan) = model.configuration.import.as_mut() {
            plan.candidates.retain(|candidate| candidate.importable);
        }
    });
    click(cx, Pane::Configuration.nav_id());
    assert!(visible(cx, configuration::IMPORT_ID));
    assert!(!visible(cx, configuration::KEEP_CONFIG_ID));
}

#[gpui_kit::test]
fn collapse_all_folds_and_unfolds_the_cards(cx: &mut TestAppContext) {
    let (_view, cx, _) = settings(cx, |_| {});
    click(cx, Pane::LocalAgents.nav_id());
    assert!(visible(cx, ACCESS_SWITCH_ID));

    click(cx, COLLAPSE_ALL_ID);
    assert!(!visible(cx, ACCESS_SWITCH_ID));

    click(cx, COLLAPSE_ALL_ID);
    assert!(visible(cx, ACCESS_SWITCH_ID));
}

#[gpui_kit::test]
fn the_switch_turns_agent_access_off(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::LocalAgents.nav_id());
    click(cx, ACCESS_SWITCH_ID);
    assert!(!view.read_with(cx, |view, _| view.model.local_agents.enabled));
    assert_eq!(*requests.borrow(), [SettingsRequest::SetAgentAccess(false)]);
}

#[gpui_kit::test]
fn the_importer_sends_the_checked_rules(cx: &mut TestAppContext) {
    let (_view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Configuration.nav_id());

    // Uncheck the first rewrite; the function matchers cannot be checked.
    click(cx, ("config-import-check", 0_usize));
    click(cx, ("config-import-check", 4_usize));
    click(cx, configuration::IMPORT_ID);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::Import {
            candidates: vec![1, 2, 3, 6],
            keep_config: false,
        }]
    );
}

#[gpui_kit::test]
fn the_source_tiles_pick_the_rules_source(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Configuration.nav_id());
    click(cx, configuration::FINICKY_SLOT_ID);
    assert_eq!(
        view.read_with(cx, |view, _| view.model.configuration.active),
        super::model::RulesSource::Finicky
    );
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetRulesSource(
            super::model::RulesSource::Finicky
        )]
    );
}
