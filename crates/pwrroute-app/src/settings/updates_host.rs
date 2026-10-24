//! The producer behind Settings › Updates: this copy as Velopack sees it
//! (`updater::installation`) and what the last check said
//! (`updater::state`). Check now starts the same check the app menu does;
//! `updater::install` redraws the pane when it finishes.

use std::{thread, time::SystemTime};

use chrono::{DateTime, Local};
use gpui_kit::{App, SharedString};

use super::model::{SettingsRequest, UpdateState, Updates};
use crate::settings;
use crate::updater::{self, Installation, Outcome, State};

/// The pane's model as things stand.
pub fn model() -> Updates {
    build(&updater::installation(), &updater::state(), Local::now())
}

/// The model from its parts, as of `now`.
pub fn build(installation: &Installation, state: &State, now: DateTime<Local>) -> Updates {
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
    let state_shown = match (not_updatable, state.checking, &state.last) {
        (Some(why), _, _) => UpdateState::NotUpdatable(why.into()),
        (None, true, _) => UpdateState::Checking,
        (None, false, None) => UpdateState::NeverChecked,
        (None, false, Some((_, outcome))) => match outcome {
            Outcome::AlreadyChecking => UpdateState::Checking,
            Outcome::NotInstalled(why) => {
                UpdateState::NotUpdatable(format!("This copy cannot update itself: {why}.").into())
            }
            Outcome::UpToDate(_) => UpdateState::UpToDate,
            Outcome::Downloaded(version) | Outcome::Pending(version) => {
                UpdateState::Ready(version.clone().into())
            }
            Outcome::Failed(why) => {
                UpdateState::Failed(format!("Could not check for updates: {why}.").into())
            }
        },
    };
    Updates {
        channel: updater::channel(&version).into(),
        version: version.into(),
        install: install.into(),
        state: state_shown,
        last_checked: state.last.as_ref().map(|(at, _)| clock(*at, now)),
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
        SettingsRequest::RestartToUpdate => {
            // Velopack exits this process once the package is applied, so
            // nothing here waits on it.
            let spawned = thread::Builder::new()
                .name("pwrroute-restart".to_owned())
                .spawn(|| {
                    if let Err(error) = updater::restart_to_update() {
                        eprintln!("pwrroute: updates: could not restart to update: {error}");
                    }
                });
            if let Err(error) = spawned {
                eprintln!("pwrroute: updates: could not restart to update: {error}");
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

    fn velopack() -> Installation {
        Installation::Velopack {
            version: "0.2.0".to_owned(),
        }
    }

    #[test]
    fn without_a_check_the_pane_reads_never_checked() {
        let model = build(&velopack(), &State::default(), Local::now());
        assert_eq!(model.version, "0.2.0");
        assert_eq!(model.channel, "stable");
        assert_eq!(model.state, UpdateState::NeverChecked);
        assert_eq!(model.last_checked, None);
        assert!(model.install.starts_with("Installed by Velopack"));
    }

    #[test]
    fn a_development_build_is_not_updatable_and_shows_the_crate_version() {
        let installation = Installation::NotInstalled {
            why: "no manifest".to_owned(),
        };
        let model = build(&installation, &State::default(), Local::now());
        assert_eq!(model.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(model.channel, "prerelease");
        assert!(matches!(model.state, UpdateState::NotUpdatable(_)));
        assert_eq!(model.last_checked, None);
        // The real producer on this test binary says the same.
        assert!(matches!(super::model().state, UpdateState::NotUpdatable(_)));
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
    fn the_last_outcome_becomes_the_state_and_the_time() {
        let now = local(12, 0, 0);
        let at = |ago: Duration| SystemTime::from(now) - ago;
        let state = |checking, last| State { checking, last };

        let model = build(&velopack(), &state(true, None), now);
        assert_eq!(model.state, UpdateState::Checking);

        let model = build(
            &velopack(),
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
            &velopack(),
            &state(
                false,
                Some((
                    at(Duration::from_secs(3 * 86_400)),
                    Outcome::Downloaded("0.2.1".into()),
                )),
            ),
            now,
        );
        assert_eq!(model.state, UpdateState::Ready("0.2.1".into()));
        let checked = model.last_checked.unwrap();
        assert!(!checked.starts_with("Today "), "{checked}");
        assert_eq!(checked.len(), "2026-10-18 08:13".len());

        let model = build(
            &velopack(),
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
        // A prerelease install follows prereleases.
        let model = build(
            &Installation::Velopack {
                version: "0.3.0-beta.1".to_owned(),
            },
            &State::default(),
            now,
        );
        assert_eq!(model.channel, "prerelease");
    }

    /// A check a minute before midnight is yesterday's, not "Today", once
    /// the day has turned.
    #[test]
    fn a_check_before_midnight_is_dated_after_it() {
        let now = local(0, 0, 30);
        let checked = SystemTime::from(now) - Duration::from_secs(60);
        let state = State {
            checking: false,
            last: Some((checked, Outcome::UpToDate("0.2.0".into()))),
        };
        let model = build(&velopack(), &state, now);
        assert_eq!(model.last_checked.unwrap(), "2026-10-22 23:59");
    }
}
