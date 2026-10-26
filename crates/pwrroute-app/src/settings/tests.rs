//! The Settings window through gpui-kit's harness: open it the ways a
//! reader does, switch panes, and check what reaches the request handler.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle,
};

use super::local_agents::{ACCESS_SWITCH_ID, COLLAPSE_ALL_ID};
use super::model::{DefaultState, RoleId, SessionId, SettingsRequest, UpdateState};
use super::{
    Pane, SettingsHost, SettingsWindow, about, browsers, configuration, diagnostics, fixtures,
    updates,
};
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
            model: Rc::new(|_, _| fixtures::model()),
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
        Pane::Updates,
        Pane::Diagnostics,
        Pane::About,
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
        model.browsers.default.state = DefaultState::Other("Google Chrome".into());
        model.browsers.default.note = None;
    });
    click(cx, browsers::REGISTER_ID);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::RegisterDefaultBrowser]
    );
    // The window waits for the OS to say so; it does not claim success.
    assert!(!view.read_with(cx, |view, _| view.model.browsers.default.state.is_default()));
}

#[gpui_kit::test]
fn a_producer_reports_the_os_answer_into_the_open_window(cx: &mut TestAppContext) {
    let requests = install(cx);
    cx.update(|cx| super::open(cx, Some(Pane::Browsers)));
    let handle = settings_windows(cx).remove(0);
    let card = |cx: &mut TestAppContext| {
        handle
            .read_with(cx, |view, _| view.model.browsers.default.clone())
            .unwrap()
    };
    assert_eq!(card(cx).state, DefaultState::PwrRoute);

    let answer = super::model::DefaultBrowser {
        state: DefaultState::Other("Firefox".into()),
        note: Some("Firefox is still the default.".into()),
    };
    let reported = answer.clone();
    cx.update(|cx| super::update(cx, move |model| model.browsers.default = reported));
    assert_eq!(card(cx), answer);
    // The card says so, and the button is back and live.
    let cx = &mut VisualTestContext::from_window(handle.into(), cx);
    draw(cx);
    assert!(visible(cx, browsers::REGISTER_ID));
    click(cx, browsers::REGISTER_ID);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::RegisterDefaultBrowser]
    );
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
fn the_importer_card_is_a_summary_that_opens_the_importer(cx: &mut TestAppContext) {
    let (_view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Configuration.nav_id());
    click(cx, configuration::REVIEW_IMPORT_ID);
    assert_eq!(*requests.borrow(), [SettingsRequest::ReviewImport]);
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

#[gpui_kit::test]
fn the_source_tiles_only_show_the_choice_while_switching_is_not_built(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |model| model.configuration.can_switch = false);
    click(cx, Pane::Configuration.nav_id());
    click(cx, configuration::FINICKY_SLOT_ID);
    assert_eq!(
        view.read_with(cx, |view, _| view.model.configuration.active),
        super::model::RulesSource::Visual
    );
    assert!(requests.borrow().is_empty());
    // The files card, below the fold, still acts.
    view.update(cx, |view, cx| {
        view.scroll
            .set_offset(gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(-2000.0)));
        cx.notify();
    });
    draw(cx);
    click(cx, "config-reload");
    click(cx, "config-reveal");
    click(cx, "config-data-folder");
    assert_eq!(
        *requests.borrow(),
        [
            SettingsRequest::ReloadRules,
            SettingsRequest::RevealRules,
            SettingsRequest::OpenDataFolder,
        ]
    );
}

#[gpui_kit::test]
fn the_browsers_pane_hides_an_app_and_rescans(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, ("browsers-visible", 0_usize));
    assert!(!view.read_with(cx, |view, _| view.model.browsers.detected[0].visible));
    click(cx, ("browsers-other-visible", 1_usize));
    click(cx, "browsers-rescan");
    assert_eq!(
        *requests.borrow(),
        [
            SettingsRequest::SetAppVisible {
                app: "com.google.Chrome".into(),
                visible: false,
            },
            SettingsRequest::SetAppVisible {
                app: "com.figma.Desktop".into(),
                visible: false,
            },
            SettingsRequest::RescanBrowsers,
        ]
    );
}

#[gpui_kit::test]
fn the_release_channel_tiles_pick_a_slot_and_a_switch_back_restarts(cx: &mut TestAppContext) {
    use super::model::Slot;
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Updates.nav_id());
    for slot in Slot::ALL {
        assert!(visible(cx, updates::slot_id(slot)), "{slot}");
    }
    click(cx, updates::slot_id(Slot::BETA_PRERELEASE));
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetUpdateSlot(Slot::BETA_PRERELEASE)]
    );
    // An older release downloaded for the slot picked: Restart to switch.
    view.update(cx, |view, cx| {
        view.model.updates.state = UpdateState::SwitchReady("0.1.9".into());
        cx.notify();
    });
    draw(cx);
    assert!(visible(cx, updates::RESTART_ID));
    click(cx, updates::RESTART_ID);
    assert_eq!(requests.borrow()[1], SettingsRequest::RestartToUpdate);
}

#[gpui_kit::test]
fn the_updates_pane_checks_now_and_restarts_into_a_download(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Updates.nav_id());
    // The fixture has a release downloaded: Restart, not Check now.
    assert!(visible(cx, updates::RESTART_ID));
    assert!(!visible(cx, updates::CHECK_ID));
    click(cx, updates::RESTART_ID);
    assert_eq!(*requests.borrow(), [SettingsRequest::RestartToUpdate]);

    view.update(cx, |view, cx| {
        view.model.updates.state = UpdateState::NeverChecked;
        view.model.updates.last_checked = None;
        cx.notify();
    });
    draw(cx);
    assert!(visible(cx, updates::CHECK_ID));
    click(cx, updates::CHECK_ID);
    assert_eq!(requests.borrow()[1], SettingsRequest::CheckForUpdates);

    // While a check runs the button is inert; a failure offers Try again.
    view.update(cx, |view, cx| {
        view.model.updates.state = UpdateState::Checking;
        cx.notify();
    });
    draw(cx);
    click(cx, updates::CHECK_ID);
    assert_eq!(requests.borrow().len(), 2);
    view.update(cx, |view, cx| {
        view.model.updates.state = UpdateState::Failed("Could not check: timed out.".into());
        cx.notify();
    });
    draw(cx);
    click(cx, updates::CHECK_ID);
    assert_eq!(requests.borrow()[2], SettingsRequest::CheckForUpdates);
    // A copy that cannot update offers nothing.
    view.update(cx, |view, cx| {
        view.model.updates.state = UpdateState::NotUpdatable("A development build.".into());
        cx.notify();
    });
    draw(cx);
    assert!(!visible(cx, updates::CHECK_ID));
    assert!(!visible(cx, updates::RESTART_ID));
}

#[gpui_kit::test]
fn the_diagnostics_pane_copies_the_report_and_opens_the_data_folder(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Diagnostics.nav_id());
    click(cx, diagnostics::COPY_ID);
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    let report = view.read_with(cx, |view, _| view.model.diagnostics.report());
    assert_eq!(copied.as_deref(), Some(report.as_str()));
    assert!(!report.contains("://"), "{report}");
    assert!(requests.borrow().is_empty());

    click(cx, diagnostics::OPEN_DATA_ID);
    click(cx, "diagnostics-reload");
    assert_eq!(
        *requests.borrow(),
        [
            SettingsRequest::OpenDataFolder,
            SettingsRequest::ReloadRules
        ]
    );
    // No data folder: the button is inert.
    view.update(cx, |view, cx| {
        view.model.diagnostics.data_dir_exists = false;
        cx.notify();
    });
    draw(cx);
    click(cx, diagnostics::OPEN_DATA_ID);
    assert_eq!(requests.borrow().len(), 2);
}

#[gpui_kit::test]
fn the_about_pane_opens_helps_documents(cx: &mut TestAppContext) {
    let (_, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::About.nav_id());
    click(cx, about::LICENSE_ID);
    click(cx, about::NOTICES_ID);
    click(cx, about::CHANGELOG_ID);
    assert_eq!(
        *requests.borrow(),
        [
            SettingsRequest::OpenLicense,
            SettingsRequest::OpenThirdPartyNotices,
            SettingsRequest::OpenChangelog,
        ]
    );
}

#[gpui_kit::test]
fn diagnostics_opens_the_logs_while_there_is_a_log(cx: &mut TestAppContext) {
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Diagnostics.nav_id());
    click(cx, diagnostics::OPEN_LOGS_ID);
    assert_eq!(*requests.borrow(), [SettingsRequest::OpenLogs]);
    view.update(cx, |view, cx| {
        view.model.diagnostics.log_exists = false;
        cx.notify();
    });
    draw(cx);
    click(cx, diagnostics::OPEN_LOGS_ID);
    assert_eq!(requests.borrow().len(), 1);
}

#[gpui_kit::test]
fn the_privacy_pane_switches_the_journal_and_confirms_full_urls(cx: &mut TestAppContext) {
    use super::model::JournalLevel;
    use super::privacy::{self, Sheet};
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Privacy.nav_id());
    assert!(visible(cx, Pane::Privacy.pane_id()));
    for id in [
        privacy::KEEP_COUNT_ID,
        privacy::KEEP_HOURS_ID,
        privacy::DESCRIPTION_ID,
        privacy::JOURNAL_HOSTS_ID,
        privacy::REVEAL_ID,
        privacy::CLEAR_JOURNAL_ID,
        privacy::EXPORT_ID,
    ] {
        assert!(visible(cx, id), "{id} is drawn");
    }
    assert!(!visible(cx, privacy::SHEET_ID));
    let journal =
        |cx: &mut VisualTestContext| view.read_with(cx, |view, _| view.model.privacy.journal);
    assert_eq!(journal(cx), JournalLevel::Hosts);

    // The level in force is inert; Off applies at once.
    click(cx, privacy::JOURNAL_HOSTS_ID);
    assert!(requests.borrow().is_empty());
    click(cx, privacy::JOURNAL_OFF_ID);
    assert_eq!(journal(cx), JournalLevel::Off);
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetJournal(JournalLevel::Off)]
    );

    // Full URLs opens the sheet and moves nothing until it is confirmed.
    click(cx, privacy::JOURNAL_FULL_ID);
    assert_eq!(
        view.read_with(cx, |view, _| view.privacy.sheet()),
        Some(Sheet::WriteFull)
    );
    assert!(visible(cx, privacy::SHEET_ID));
    assert_eq!(journal(cx), JournalLevel::Off);
    assert_eq!(requests.borrow().len(), 1);
    click(cx, privacy::SHEET_CANCEL_ID);
    assert_eq!(view.read_with(cx, |view, _| view.privacy.sheet()), None);
    assert_eq!(journal(cx), JournalLevel::Off);
    assert_eq!(requests.borrow().len(), 1);

    // Escape is Cancel.
    click(cx, privacy::JOURNAL_FULL_ID);
    assert!(visible(cx, privacy::SHEET_ID));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(view.read_with(cx, |view, _| view.privacy.sheet()), None);
    assert!(!visible(cx, privacy::SHEET_ID));

    click(cx, privacy::JOURNAL_FULL_ID);
    click(cx, privacy::SHEET_CONFIRM_ID);
    assert_eq!(view.read_with(cx, |view, _| view.privacy.sheet()), None);
    assert_eq!(journal(cx), JournalLevel::Full);
    assert_eq!(
        requests.borrow()[1],
        SettingsRequest::SetJournal(JournalLevel::Full)
    );

    // Clear now on the journal confirms too, and takes the files with it.
    click(cx, privacy::CLEAR_JOURNAL_ID);
    assert_eq!(
        view.read_with(cx, |view, _| view.privacy.sheet()),
        Some(Sheet::ClearJournal)
    );
    click(cx, privacy::SHEET_CONFIRM_ID);
    assert_eq!(requests.borrow()[2], SettingsRequest::ClearJournal);
    assert!(view.read_with(cx, |view, _| view.model.privacy.journal_files.is_none()));

    // The other buttons each send their request.
    click(cx, privacy::CLEAR_RECENT_ID);
    click(cx, privacy::REVEAL_ID);
    click(cx, privacy::EXPORT_ID);
    assert_eq!(
        requests.borrow()[3..],
        [
            SettingsRequest::ClearRecent,
            SettingsRequest::RevealJournal,
            SettingsRequest::ExportActivity,
        ]
    );
}

#[gpui_kit::test]
fn the_privacy_pane_applies_a_retention_on_return_and_keeps_it_in_range(cx: &mut TestAppContext) {
    use super::privacy;
    let (view, cx, requests) = settings(cx, |_| {});
    click(cx, Pane::Privacy.nav_id());
    let keep = |cx: &mut VisualTestContext| {
        view.read_with(cx, |view, _| {
            (view.model.privacy.keep_count, view.model.privacy.keep_hours)
        })
    };
    assert_eq!(keep(cx), (500, 72));

    click(cx, privacy::KEEP_COUNT_ID);
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("200");
    draw(cx);
    // Typing alone changes nothing: Return applies.
    assert_eq!(keep(cx), (500, 72));
    assert!(requests.borrow().is_empty());
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(keep(cx), (200, 72));
    assert_eq!(
        *requests.borrow(),
        [SettingsRequest::SetRetention {
            count: 200,
            hours: 72
        }]
    );

    // Out of range reverts to the value in force and sends nothing.
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("5");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(keep(cx), (200, 72));
    assert_eq!(requests.borrow().len(), 1);

    click(cx, privacy::KEEP_HOURS_ID);
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("24");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(keep(cx), (200, 24));
    assert_eq!(
        requests.borrow()[1],
        SettingsRequest::SetRetention {
            count: 200,
            hours: 24
        }
    );
}

#[gpui_kit::test]
fn the_privacy_pane_draws_the_journal_off_state(cx: &mut TestAppContext) {
    use super::model::JournalLevel;
    use super::privacy;
    let (view, cx, requests) = settings(cx, |model| {
        model.privacy.journal = JournalLevel::Off;
        model.privacy.journal_files = None;
        model.privacy.rules_keep_less = None;
    });
    click(cx, Pane::Privacy.nav_id());
    assert!(visible(cx, privacy::DESCRIPTION_ID));
    // Nothing on disk: Reveal and Clear now are inert, Export is not.
    click(cx, privacy::CLEAR_JOURNAL_ID);
    assert_eq!(view.read_with(cx, |view, _| view.privacy.sheet()), None);
    click(cx, privacy::REVEAL_ID);
    assert!(requests.borrow().is_empty());
    click(cx, privacy::EXPORT_ID);
    assert_eq!(*requests.borrow(), [SettingsRequest::ExportActivity]);

    // Switching panes closes an open sheet.
    click(cx, privacy::JOURNAL_FULL_ID);
    assert!(visible(cx, privacy::SHEET_ID));
    click(cx, Pane::Browsers.nav_id());
    click(cx, Pane::Privacy.nav_id());
    assert!(!visible(cx, privacy::SHEET_ID));
    assert_eq!(
        view.read_with(cx, |view, _| view.model.privacy.journal),
        JournalLevel::Off
    );
}

// Settings Everything else (WP-F)

mod everything_else {
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::SystemTime;

    use gpui_kit::{TestAppContext, VisualTestContext, WindowHandle};
    use pwrroute_engine::{AppType, BrowserSpec, FixedBrowser, Precedence, RulesFile};

    use super::super::configuration::{
        BROWSER_OPTION_ID, CONFIG_DEFAULT_ID, DEFAULT_BROWSER_ID, DEFAULT_PROFILE_ID,
        EVERYTHING_ELSE_ID, PROBLEM_ID, PROFILE_OPTION_ID, PWRROUTE_DEFAULT_ID, Picking,
        WARNINGS_ID,
    };
    use super::super::configuration_host::{
        ConfigFacts, Facts, build, edit_rules, set_everything_else,
    };
    use super::super::model::{Configuration, DefaultOwner, RulesSource, SettingsRequest};
    use super::super::{Pane, SettingsHost, SettingsWindow, fixtures};
    use super::{click, draw, settings, visible};
    use crate::app::PwrRoute;
    use crate::browser_scan::DetectedApp;
    use crate::router::RoutingSource;
    use crate::rules_store::{CHANGED_ON_DISK, RulesStore};

    fn owner(
        view: &gpui_kit::Entity<super::SettingsWindow>,
        cx: &mut VisualTestContext,
    ) -> DefaultOwner {
        view.read_with(cx, |view, _| {
            view.model
                .configuration
                .everything_else
                .as_ref()
                .expect("the Everything else field")
                .owner
        })
    }

    #[gpui_kit::test]
    fn the_row_is_hidden_while_visual_rules_are_in_charge(cx: &mut TestAppContext) {
        // The fixture: visual rules in charge, a config found beside them.
        let (_view, cx, _) = settings(cx, |_| {});
        click(cx, Pane::Configuration.nav_id());
        assert!(!visible(cx, EVERYTHING_ELSE_ID));
    }

    #[gpui_kit::test]
    fn the_row_shows_only_while_a_config_is_in_charge(cx: &mut TestAppContext) {
        let (view, cx, _) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else();
        });
        click(cx, Pane::Configuration.nav_id());
        assert!(visible(cx, EVERYTHING_ELSE_ID));
        assert!(visible(cx, CONFIG_DEFAULT_ID));
        assert!(visible(cx, PWRROUTE_DEFAULT_ID));
        assert!(visible(cx, DEFAULT_BROWSER_ID));
        // Google Chrome has profiles: the profile select sits beside it.
        assert!(visible(cx, DEFAULT_PROFILE_ID));

        // The same field with visual rules in charge is not drawn: that is
        // a rule for the window, whatever the producer sends.
        view.update(cx, |view, cx| {
            view.model.configuration.active = RulesSource::Visual;
            cx.notify();
        });
        draw(cx);
        assert!(!visible(cx, EVERYTHING_ELSE_ID));
    }

    #[gpui_kit::test]
    fn choosing_an_answer_asks_to_write_precedence(cx: &mut TestAppContext) {
        let (view, cx, requests) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else();
        });
        click(cx, Pane::Configuration.nav_id());
        assert_eq!(owner(&view, cx), DefaultOwner::PwrRoute);

        // The answer already chosen asks for nothing.
        click(cx, PWRROUTE_DEFAULT_ID);
        assert!(requests.borrow().is_empty());

        click(cx, CONFIG_DEFAULT_ID);
        assert_eq!(owner(&view, cx), DefaultOwner::Config);
        draw(cx);
        click(cx, PWRROUTE_DEFAULT_ID);
        assert_eq!(owner(&view, cx), DefaultOwner::PwrRoute);
        assert_eq!(
            *requests.borrow(),
            [
                SettingsRequest::SetDefaultOwner(DefaultOwner::Config),
                SettingsRequest::SetDefaultOwner(DefaultOwner::PwrRoute),
            ]
        );
    }

    #[gpui_kit::test]
    fn warnings_show_only_while_pwrroutes_default_takes_everything_else(cx: &mut TestAppContext) {
        let (_view, cx, _) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else_warnings();
        });
        click(cx, Pane::Configuration.nav_id());
        assert!(visible(cx, WARNINGS_ID));

        click(cx, CONFIG_DEFAULT_ID);
        draw(cx);
        assert!(!visible(cx, WARNINGS_ID));

        click(cx, PWRROUTE_DEFAULT_ID);
        draw(cx);
        assert!(visible(cx, WARNINGS_ID));
    }

    #[gpui_kit::test]
    fn no_warning_line_when_nothing_changes_hands(cx: &mut TestAppContext) {
        let (_view, cx, _) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else();
        });
        click(cx, Pane::Configuration.nav_id());
        assert!(!visible(cx, WARNINGS_ID));
    }

    #[gpui_kit::test]
    fn the_browser_and_profile_are_chosen_from_the_selects(cx: &mut TestAppContext) {
        let (view, cx, requests) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else();
        });
        click(cx, Pane::Configuration.nav_id());

        click(cx, DEFAULT_BROWSER_ID);
        assert_eq!(
            view.read_with(cx, |view, _| view.configuration.open_menu()),
            Some(Picking::Browser)
        );
        draw(cx);
        click(cx, (BROWSER_OPTION_ID, 1_usize));
        assert_eq!(
            view.read_with(cx, |view, _| view.configuration.open_menu()),
            None
        );
        // Safari has no profiles: its own default, and no profile select.
        draw(cx);
        assert!(!visible(cx, DEFAULT_PROFILE_ID));

        click(cx, DEFAULT_BROWSER_ID);
        draw(cx);
        click(cx, (BROWSER_OPTION_ID, 0_usize));
        draw(cx);
        click(cx, DEFAULT_PROFILE_ID);
        assert_eq!(
            view.read_with(cx, |view, _| view.configuration.open_menu()),
            Some(Picking::Profile)
        );
        draw(cx);
        // "Default profile", then Personal and Work.
        click(cx, (PROFILE_OPTION_ID, 2_usize));
        assert_eq!(
            *requests.borrow(),
            [
                SettingsRequest::SetDefaultBrowser {
                    browser: "com.apple.Safari".into(),
                    profile: "".into(),
                },
                SettingsRequest::SetDefaultBrowser {
                    browser: "com.google.Chrome".into(),
                    profile: "".into(),
                },
                SettingsRequest::SetDefaultBrowser {
                    browser: "com.google.Chrome".into(),
                    profile: "Work".into(),
                },
            ]
        );
    }

    #[gpui_kit::test]
    fn a_pane_switch_closes_an_open_select(cx: &mut TestAppContext) {
        let (view, cx, _) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else();
        });
        click(cx, Pane::Configuration.nav_id());
        click(cx, DEFAULT_BROWSER_ID);
        click(cx, Pane::Browsers.nav_id());
        assert_eq!(
            view.read_with(cx, |view, _| view.configuration.open_menu()),
            None
        );
    }

    fn rules_at(dir: &Path, text: &str) -> PathBuf {
        let path = dir.join("rules.json");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn on_disk(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[gpui_kit::test]
    fn the_main_windows_store_writes_precedence(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = rules_at(
            dir.path(),
            r#"{ "defaultBrowser": "com.google.Chrome",
                 "rules": [{ "match": "a.example.com/*", "browser": "Safari" }] }"#,
        );
        cx.update(|cx| crate::assets::load_fonts(cx));
        let store = RulesStore::load(Some(path.clone()));
        let (app, cx) = cx.add_window_view(|_, cx| PwrRoute::with_store(None, None, store, cx));
        draw(cx);

        // The path is not read while the main window is open: its store is.
        // As the Settings window's deferred request runs: outside the main
        // window's own update.
        let saved = cx
            .cx
            .update(|cx| edit_rules(None, cx, |store| store.set_precedence(Precedence::PwrRoute)));
        assert_eq!(saved, Ok(()));
        assert_eq!(on_disk(&path)["precedence"], "pwrroute");
        assert_eq!(on_disk(&path)["rules"][0]["match"], "a.example.com/*");
        app.read_with(cx, |app, _| {
            assert_eq!(app.store.precedence(), Precedence::PwrRoute);
            assert!(!app.store.needs_save());
        });

        // Back to the config's default: the field is left out again.
        let saved = cx
            .cx
            .update(|cx| edit_rules(None, cx, |store| store.set_precedence(Precedence::Config)));
        assert_eq!(saved, Ok(()));
        assert!(on_disk(&path).get("precedence").is_none());

        // Another writer changed the file under an unsaved edit: refused,
        // and nothing is written over it.
        app.update(cx, |app, _| app.store.set_browser(0, "Firefox"));
        std::fs::write(&path, r#"{ "defaultBrowser": "com.apple.Safari" }"#).unwrap();
        let saved = cx
            .cx
            .update(|cx| edit_rules(None, cx, |store| store.set_precedence(Precedence::PwrRoute)));
        assert_eq!(saved, Err(CHANGED_ON_DISK.to_owned()));
        assert_eq!(
            on_disk(&path),
            serde_json::json!({ "defaultBrowser": "com.apple.Safari" })
        );
    }

    #[gpui_kit::test]
    fn without_a_main_window_the_file_is_written_through_a_store(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = rules_at(dir.path(), r#"{ "defaultBrowser": "com.apple.Safari" }"#);
        let saved = cx.update(|cx| {
            edit_rules(Some(path.clone()), cx, |store| {
                store.set_precedence(Precedence::PwrRoute);
                store.set_default_browser("com.google.Chrome", "Personal");
            })
        });
        assert_eq!(saved, Ok(()));
        let file = RulesFile::load(&path).unwrap();
        assert_eq!(file.precedence, Precedence::PwrRoute);
        assert_eq!(file.default_browser, "com.google.Chrome");
        assert_eq!(file.default_profile, "Personal");

        // A file that does not load is never written over.
        std::fs::write(&path, "{ not json").unwrap();
        let saved = cx.update(|cx| {
            edit_rules(Some(path.clone()), cx, |store| {
                store.set_precedence(Precedence::Config);
            })
        });
        assert_eq!(
            saved,
            Err("rules.json did not load, so it is not overwritten.".to_owned())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    // ---- Through the window: click, request, write, read back ---------------

    fn scanned(name: &str, bundle_id: &str, profiles: &[&str]) -> DetectedApp {
        DetectedApp {
            name: name.into(),
            bundle_id: Some(bundle_id.into()),
            location: PathBuf::from(format!("/Applications/{name}.app")),
            profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
            directories: Vec::new(),
            profiles_blocked: false,
        }
    }

    /// The pane as the producer builds it from `rules.json` at `path`, with
    /// `~/.finicky.ts` (default Safari) in charge beside it.
    fn read_back(home: &Path, path: &Path) -> Configuration {
        let config = home.join(".finicky.ts");
        let facts = Facts {
            source: RoutingSource {
                config: Some(config.clone()),
                ..RoutingSource::default()
            },
            rules_path: Some(path.to_path_buf()),
            rules: path
                .exists()
                .then(|| RulesFile::load(path).map_err(|error| format!("{error:#}"))),
            rules_modified: None,
            config: Some(ConfigFacts {
                path: config,
                counts: Some((0, 5)),
                shorteners: None,
                modified: None,
                import: None,
                default: Some(BrowserSpec::Browser(FixedBrowser {
                    name: "Safari".into(),
                    app_type: AppType::AppName,
                    profile: String::new(),
                    args: Vec::new(),
                    open_in_background: None,
                })),
                catch_all: None,
            }),
            home: Some(home.to_path_buf()),
            browsers: vec![
                scanned("Google Chrome", "com.google.Chrome", &["Personal", "Work"]),
                scanned("Safari", "com.apple.Safari", &[]),
            ],
        };
        build(&facts, SystemTime::now())
    }

    /// Settings opened as a reader opens it, on Configuration, with the
    /// real Everything else handler writing `rules.json` at `path` and
    /// reading it back.
    fn open_over(
        cx: &mut TestAppContext,
        home: &Path,
        path: &Path,
    ) -> WindowHandle<SettingsWindow> {
        let (home, path) = (home.to_path_buf(), path.to_path_buf());
        cx.update(|cx| {
            crate::assets::load_fonts(cx);
            super::super::install(cx);
            let (model_home, model_path) = (home.clone(), path.clone());
            cx.set_global(SettingsHost {
                model: Rc::new(move |_, _| {
                    let mut model = fixtures::model();
                    model.configuration = read_back(&model_home, &model_path);
                    model
                }),
                on_request: Rc::new(move |request, _, cx| {
                    let (home, read) = (home.clone(), path.clone());
                    set_everything_else(
                        request,
                        Some(path.clone()),
                        move |_| read_back(&home, &read),
                        cx,
                    );
                }),
            });
        });
        cx.update(|cx| super::super::open(cx, Some(Pane::Configuration)));
        cx.run_until_parked();
        super::settings_windows(cx)[0]
    }

    fn field_in(
        handle: WindowHandle<SettingsWindow>,
        cx: &mut VisualTestContext,
    ) -> super::super::model::EverythingElse {
        handle
            .read_with(cx, |view, _| {
                view.model.configuration.everything_else.clone()
            })
            .unwrap()
            .expect("the Everything else field")
    }

    #[gpui_kit::test]
    fn a_change_is_written_and_read_back_into_the_window(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = rules_at(dir.path(), r#"{ "precedence": "pwrroute" }"#);
        let handle = open_over(cx, dir.path(), &path);
        let mut cx = VisualTestContext::from_window(handle.into(), cx);
        draw(&mut cx);
        let field = field_in(handle, &mut cx);
        assert_eq!(field.owner, DefaultOwner::PwrRoute);
        assert_eq!(field.browser, None);

        // No default browser yet: pick Safari.
        click(&mut cx, DEFAULT_BROWSER_ID);
        draw(&mut cx);
        click(&mut cx, (BROWSER_OPTION_ID, 1_usize));
        cx.run_until_parked();
        assert_eq!(on_disk(&path)["defaultBrowser"], "com.apple.Safari");
        let field = field_in(handle, &mut cx);
        assert_eq!(
            field.chosen().map(|chosen| chosen.value.clone()),
            Some("com.apple.Safari".into())
        );
        assert_eq!(field.problem, None);

        // Back to the config's default: written, and what only the
        // producer says (the source card's order) is read back.
        draw(&mut cx);
        click(&mut cx, CONFIG_DEFAULT_ID);
        cx.run_until_parked();
        assert!(on_disk(&path).get("precedence").is_none());
        let found = handle
            .read_with(&cx, |view, _| {
                view.model
                    .configuration
                    .finicky
                    .as_ref()
                    .unwrap()
                    .found
                    .clone()
            })
            .unwrap();
        assert!(
            found.contains("runs first and takes everything else"),
            "{found}"
        );
        assert_eq!(field_in(handle, &mut cx).owner, DefaultOwner::Config);
    }

    #[gpui_kit::test]
    fn a_change_that_is_not_saved_says_why_and_the_control_flips_back(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = rules_at(dir.path(), r#"{ "defaultBrowser": "com.google.Chrome" }"#);
        let handle = open_over(cx, dir.path(), &path);
        let mut cx = VisualTestContext::from_window(handle.into(), cx);
        draw(&mut cx);
        assert_eq!(field_in(handle, &mut cx).owner, DefaultOwner::Config);
        assert!(!visible(&mut cx, PROBLEM_ID));

        // Another writer breaks the file before the click.
        std::fs::write(&path, "{ not json").unwrap();
        click(&mut cx, PWRROUTE_DEFAULT_ID);
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
        let field = field_in(handle, &mut cx);
        assert_eq!(field.owner, DefaultOwner::Config);
        assert_eq!(
            field.problem.as_deref(),
            Some("Not saved: rules.json did not load, so it is not overwritten.")
        );
        draw(&mut cx);
        assert!(visible(&mut cx, PROBLEM_ID));
    }

    #[gpui_kit::test]
    fn a_change_the_user_makes_clears_the_not_saved_line(cx: &mut TestAppContext) {
        let (_view, cx, _) = settings(cx, |model| {
            model.configuration = fixtures::configuration_everything_else_not_saved();
        });
        click(cx, Pane::Configuration.nav_id());
        assert!(visible(cx, PROBLEM_ID));
        click(cx, CONFIG_DEFAULT_ID);
        draw(cx);
        assert!(!visible(cx, PROBLEM_ID));
    }
}
