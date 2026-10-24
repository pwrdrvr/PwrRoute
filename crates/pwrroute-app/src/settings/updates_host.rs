//! The producer behind Settings › Updates: this copy as Velopack sees it
//! (`updater::installation`), the release channel it follows
//! (`updater::selection`, over the releases the last check read) and what
//! the last check said (`updater::state`). Check now starts the same check
//! the app menu does; a slot picked here is saved to `settings.json` and
//! checked at once. `updater::install` redraws the pane when a check ends.

use std::{thread, time::SystemTime};

use chrono::{DateTime, Local};
use gpui_kit::{App, SharedString};

use super::model::{SettingsRequest, SlotTile, UpdateState, Updates};
use crate::settings;
use crate::settings_store;
use crate::update_channel::{self, Slot, Track, Train, UpdateSettings};
use crate::updater::{self, Installation, Outcome, State};

/// The pane's model as things stand.
pub fn model() -> Updates {
    build(
        &updater::installation(),
        &updater::selection(),
        &updater::state(),
        Local::now(),
    )
}

/// The model from its parts, as of `now`.
pub fn build(
    installation: &Installation,
    selection: &UpdateSettings,
    state: &State,
    now: DateTime<Local>,
) -> Updates {
    let (version, install, not_updatable) = match installation {
        Installation::Velopack { version } => (
            version.clone(),
            "Installed by Velopack; a downloaded release installs when you quit, or from \
             Restart to update."
                .to_owned(),
            None,
        ),
        Installation::NotInstalled { .. } => {
            let why = if cfg!(debug_assertions) {
                "A development build: updates are off here."
            } else {
                "Not installed by Velopack (a package, an archive or a build from source): \
                 whatever installed this copy updates it."
            };
            (
                env!("CARGO_PKG_VERSION").to_owned(),
                why.to_owned(),
                Some(why.to_owned()),
            )
        }
    };
    let slot = selection.resolve(&version);
    // A download for another slot is neither shown nor installed.
    let held = state
        .held
        .as_ref()
        .filter(|held| held.is_for(slot) && held.version != version);
    let state_shown = match (not_updatable, state.checking, held, &state.last) {
        (Some(why), _, _, _) => UpdateState::NotUpdatable(why.into()),
        (None, true, _, _) => UpdateState::Checking,
        (None, false, Some(held), _) if held.switch_back => {
            UpdateState::SwitchReady(held.version.clone().into())
        }
        (None, false, Some(held), _) => UpdateState::Ready(held.version.clone().into()),
        (None, false, None, None) => UpdateState::NeverChecked,
        (None, false, None, Some((_, outcome))) => match outcome {
            Outcome::AlreadyChecking => UpdateState::Checking,
            Outcome::NotInstalled(why) => {
                UpdateState::NotUpdatable(format!("This copy cannot update itself: {why}.").into())
            }
            Outcome::NothingPublished(_) => UpdateState::NothingPublished,
            // Ready for the slot it was held for: up to date on this one
            // until the check the new pick started says otherwise.
            Outcome::UpToDate(_)
            | Outcome::Downloaded(_)
            | Outcome::Pending(_)
            | Outcome::SwitchReady(_) => UpdateState::UpToDate,
            Outcome::Failed(why) => {
                UpdateState::Failed(format!("Could not check for updates: {why}.").into())
            }
        },
    };
    Updates {
        channel: slot.to_string().into(),
        slots: tiles(&version, slot, state, installation),
        inferred: selection.is_inferred(),
        version: version.into(),
        install: install.into(),
        state: state_shown,
        last_checked: state.last.as_ref().map(|(at, _)| clock(*at, now)),
    }
}

/// The four tiles: each slot's release from the list the last check read.
fn tiles(
    installed: &str,
    selected: Slot,
    state: &State,
    installation: &Installation,
) -> Vec<SlotTile> {
    Slot::ALL
        .into_iter()
        .map(|slot| {
            let release = match &state.releases {
                Some(Ok(releases)) => update_channel::select(releases, slot),
                _ => None,
            };
            let note = match (&release, &state.releases) {
                (Some(_), _) => about(slot),
                (None, Some(Ok(_))) => "Nothing published here yet.",
                (None, Some(Err(_))) => "Could not read the release feed.",
                (None, None) if matches!(installation, Installation::NotInstalled { .. }) => {
                    "Read by an installed copy's check."
                }
                (None, None) => "Reading published releases.",
            };
            let installed_here = release.as_ref().is_some_and(|release| {
                release.version().is_some() && release.version() == update_channel::parse(installed)
            });
            SlotTile {
                slot,
                release: release.map(|release| release.tag.into()),
                note: note.into(),
                selected: slot == selected,
                installed: installed_here,
            }
        })
        .collect()
}

/// What a slot is, under its release.
fn about(slot: Slot) -> &'static str {
    match (slot.train, slot.track) {
        (Train::Stable, Track::Latest) => "Smoke-checked. The default for everyone.",
        (Train::Stable, Track::Prerelease) => "Release candidates for the stable line.",
        (Train::Beta, Track::Latest) => "Beta builds off main.",
        (Train::Beta, Track::Prerelease) => "Newest alpha off main. May not install.",
    }
}

/// "Today 08:13", or the date for another day.
fn clock(at: SystemTime, now: DateTime<Local>) -> SharedString {
    let at: DateTime<Local> = at.into();
    if at.date_naive() == now.date_naive() {
        format!("Today {}", at.format("%H:%M")).into()
    } else {
        at.format("%Y-%m-%d %H:%M").to_string().into()
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_request(request: &SettingsRequest, cx: &mut App) -> bool {
    match request {
        SettingsRequest::CheckForUpdates => {
            updater::check_in_background(updater::Trigger::Settings);
            refresh(cx);
        }
        SettingsRequest::SetUpdateSlot(slot) => {
            let settings = UpdateSettings::picked(*slot);
            match settings_store::store(cx) {
                Some(store) => {
                    let saved =
                        store.update(cx, |store, _| store.update(|all| all.updates = settings));
                    if let Err(error) = saved {
                        log::warn!("cannot save settings: {error:#}");
                    }
                }
                None => log::warn!("settings: no store; the release channel is not saved"),
            }
            updater::set_selection(settings);
            refresh(cx);
        }
        SettingsRequest::RestartToUpdate => {
            // Velopack exits this process once the package is applied, so
            // nothing here waits on it.
            let spawned = thread::Builder::new()
                .name("pwrroute-restart".to_owned())
                .spawn(|| {
                    if let Err(error) = updater::restart_to_update() {
                        log::warn!("updates: could not restart to update: {error}");
                    }
                });
            if let Err(error) = spawned {
                log::warn!("updates: could not restart to update: {error}");
            }
        }
        _ => return false,
    }
    true
}

/// Reads the model again into the open window.
fn refresh(cx: &mut App) {
    if !settings::is_open(cx) {
        return;
    }
    let updates = model();
    settings::update(cx, move |model| model.updates = updates);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::update_channel::Release;
    use crate::updater::Held;

    fn velopack(version: &str) -> Installation {
        Installation::Velopack {
            version: version.to_owned(),
        }
    }

    fn inferred() -> UpdateSettings {
        UpdateSettings::default()
    }

    fn state(checking: bool, last: Option<(SystemTime, Outcome)>) -> State {
        State {
            checking,
            last,
            ..State::default()
        }
    }

    fn held(version: &str, slot: Slot, switch_back: bool) -> Held {
        Held {
            version: version.to_owned(),
            file_name: format!("PwrRoute-{version}-osx-full.nupkg"),
            slot: slot.key(),
            switch_back,
        }
    }

    /// A fixed local time, so "Today" never depends on when the test runs.
    fn local(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        use chrono::TimeZone;
        Local
            .with_ymd_and_hms(2026, 10, 23, hour, minute, second)
            .single()
            .expect("an unambiguous local time")
    }

    #[test]
    fn without_a_check_the_pane_reads_never_checked() {
        let model = build(
            &velopack("0.2.0"),
            &inferred(),
            &State::default(),
            Local::now(),
        );
        assert_eq!(model.version, "0.2.0");
        assert_eq!(model.channel, "Stable \u{00b7} Latest");
        assert!(model.inferred);
        assert_eq!(model.state, UpdateState::NeverChecked);
        assert_eq!(model.last_checked, None);
        assert!(model.install.starts_with("Installed by Velopack"));
        // No list read yet: every tile says it is reading.
        assert_eq!(model.slots.len(), 4);
        assert!(
            model
                .slots
                .iter()
                .all(|tile| tile.release.is_none() && tile.note == "Reading published releases.")
        );
        assert_eq!(
            model.slots.iter().filter(|tile| tile.selected).count(),
            1,
            "one slot is in force"
        );
        assert!(model.slots[0].selected);
    }

    #[test]
    fn the_build_implies_the_slot_until_one_is_picked() {
        let rc = velopack("0.1.0-rc.3");
        let model = build(&rc, &inferred(), &State::default(), Local::now());
        assert_eq!(model.channel, "Stable \u{00b7} Prerelease");
        assert!(model.slots[1].selected);
        let picked = UpdateSettings::picked(Slot::BETA_LATEST);
        let model = build(&rc, &picked, &State::default(), Local::now());
        assert_eq!(model.channel, "Beta \u{00b7} Latest");
        assert!(!model.inferred);
        assert!(model.slots[2].selected);
    }

    #[test]
    fn a_development_build_is_not_updatable_and_shows_the_crate_version() {
        let installation = Installation::NotInstalled {
            why: "no manifest".to_owned(),
        };
        let model = build(&installation, &inferred(), &State::default(), Local::now());
        assert_eq!(model.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(
            model.channel,
            crate::update_channel::infer(env!("CARGO_PKG_VERSION")).to_string()
        );
        assert!(matches!(model.state, UpdateState::NotUpdatable(_)));
        assert_eq!(model.last_checked, None);
        assert!(
            model
                .slots
                .iter()
                .all(|tile| tile.note == "Read by an installed copy's check.")
        );
        // The real producer on this test binary says the same.
        assert!(matches!(super::model().state, UpdateState::NotUpdatable(_)));
    }

    #[test]
    fn the_tiles_show_each_slots_release_and_mark_this_copy() {
        let releases = vec![
            Release::new("v0.2.0-alpha.3", true),
            Release::new("v0.1.1", true),
            Release::new("v0.1.0", false),
        ];
        let read = State {
            releases: Some(Ok(releases)),
            ..State::default()
        };
        let model = build(&velopack("0.1.0"), &inferred(), &read, Local::now());
        let shown: Vec<_> = model
            .slots
            .iter()
            .map(|tile| (tile.release.as_ref().map(|r| r.to_string()), tile.installed))
            .collect();
        assert_eq!(
            shown,
            [
                (Some("v0.1.0".to_owned()), true),
                (Some("v0.1.1".to_owned()), false),
                (Some("v0.1.0".to_owned()), true),
                (Some("v0.2.0-alpha.3".to_owned()), false),
            ]
        );
        assert_eq!(
            model.slots[0].note,
            "Smoke-checked. The default for everyone."
        );
        // Nothing promoted, or the list could not be read.
        let empty = State {
            releases: Some(Ok(vec![Release::new("v0.1.0-rc.1", true)])),
            ..State::default()
        };
        let model = build(&velopack("0.1.0"), &inferred(), &empty, Local::now());
        assert_eq!(model.slots[0].note, "Nothing published here yet.");
        let failed = State {
            releases: Some(Err("GitHub answered 500".into())),
            ..State::default()
        };
        let model = build(&velopack("0.1.0"), &inferred(), &failed, Local::now());
        assert_eq!(model.slots[0].note, "Could not read the release feed.");
    }

    #[test]
    fn a_download_shows_only_for_the_slot_it_was_held_for() {
        let now = local(12, 0, 0);
        let with = |held: Held| State {
            last: Some((
                SystemTime::from(now),
                Outcome::Downloaded(held.version.clone()),
            )),
            held: Some(held),
            ..State::default()
        };
        let copy = velopack("0.1.0");
        let model = build(
            &copy,
            &inferred(),
            &with(held("0.1.1", Slot::STABLE_LATEST, false)),
            now,
        );
        assert_eq!(model.state, UpdateState::Ready("0.1.1".into()));
        // Picked another slot: the download is not offered.
        let beta = UpdateSettings::picked(Slot::BETA_LATEST);
        let model = build(
            &copy,
            &beta,
            &with(held("0.1.1", Slot::STABLE_LATEST, false)),
            now,
        );
        assert_eq!(model.state, UpdateState::UpToDate);
        // An older release for the slot picked: Restart to switch.
        let model = build(
            &copy,
            &beta,
            &with(held("0.0.9", Slot::BETA_LATEST, true)),
            now,
        );
        assert_eq!(model.state, UpdateState::SwitchReady("0.0.9".into()));
    }

    #[test]
    fn the_last_outcome_becomes_the_state_and_the_time() {
        let now = local(12, 0, 0);
        let at = |ago: Duration| SystemTime::from(now) - ago;
        let copy = velopack("0.2.0");

        let model = build(&copy, &inferred(), &state(true, None), now);
        assert_eq!(model.state, UpdateState::Checking);

        let model = build(
            &copy,
            &inferred(),
            &state(
                false,
                Some((
                    at(Duration::from_secs(60)),
                    Outcome::UpToDate("0.2.0".into()),
                )),
            ),
            now,
        );
        assert_eq!(model.state, UpdateState::UpToDate);
        assert_eq!(model.last_checked.unwrap(), "Today 11:59");

        let model = build(
            &copy,
            &inferred(),
            &state(
                false,
                Some((
                    at(Duration::from_secs(3 * 86_400)),
                    Outcome::NothingPublished("Stable \u{00b7} Latest".into()),
                )),
            ),
            now,
        );
        assert_eq!(model.state, UpdateState::NothingPublished);
        let checked = model.last_checked.unwrap();
        assert!(!checked.starts_with("Today "), "{checked}");
        assert_eq!(checked.len(), "2026-10-18 08:13".len());

        let model = build(
            &copy,
            &inferred(),
            &state(
                false,
                Some((at(Duration::ZERO), Outcome::Failed("timed out".into()))),
            ),
            now,
        );
        assert_eq!(
            model.state,
            UpdateState::Failed("Could not check for updates: timed out.".into())
        );
    }

    #[gpui_kit::test]
    fn a_picked_slot_is_saved_to_settings_json_and_followed(cx: &mut gpui_kit::TestAppContext) {
        use crate::settings_store::{Settings, SettingsStore};
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            settings_store::install(
                SettingsStore::load(Some(dir.path().to_path_buf())),
                None,
                cx,
            );
        });
        let pick = SettingsRequest::SetUpdateSlot(Slot::BETA_LATEST);
        assert!(cx.update(|cx| handle_request(&pick, cx)));
        let text = std::fs::read_to_string(Settings::path(dir.path())).unwrap();
        assert!(text.contains("\"selectionSource\": \"user\""), "{text}");
        assert!(text.contains("\"train\": \"beta\""), "{text}");
        assert_eq!(
            Settings::load(dir.path()).unwrap().updates,
            UpdateSettings::picked(Slot::BETA_LATEST)
        );
        assert_eq!(
            updater::selection(),
            UpdateSettings::picked(Slot::BETA_LATEST)
        );
        // Back to the build's own slot, for the other tests in this binary.
        updater::set_selection(UpdateSettings::default());
    }

    /// A check a minute before midnight is yesterday's, not "Today", once
    /// the day has turned.
    #[test]
    fn a_check_before_midnight_is_dated_after_it() {
        let now = local(0, 0, 30);
        let checked = SystemTime::from(now) - Duration::from_secs(60);
        let state = state(false, Some((checked, Outcome::UpToDate("0.2.0".into()))));
        let model = build(&velopack("0.2.0"), &inferred(), &state, now);
        assert_eq!(model.last_checked.unwrap(), "2026-10-22 23:59");
    }
}
