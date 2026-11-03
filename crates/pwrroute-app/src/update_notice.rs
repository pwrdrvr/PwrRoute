//! What PwrRoute says about updates outside Settings › Updates: the main
//! window's toast (`toast.rs` draws it), the tray's update row and the
//! strip's chip.
//!
//! Two kinds of news, kept apart as PwrSnap does:
//!
//! - **The answer to Check for Updates….** Only that menu item asks; the
//!   startup, timer and Settings checks never raise a toast (Settings
//!   answers its own button inline). While the check runs the toast is a
//!   live card ([`Toast::Checking`], then [`Toast::Downloading`]) with no
//!   countdown; once it settles an answer with nothing to press
//!   ([`Toast::Answer`]) counts down [`ANSWER_FOR`] and goes.
//! - **A release ready to install** ([`Toast::Ready`]), from any check:
//!   held for the slot in force (`updater::ready`). It stays until it is
//!   installed or Later dismisses that version; the tray row and the strip
//!   chip keep offering it quietly (design: Updates and Default Browser
//!   1c, 1d).
//!
//! Plain data over `updater::state`; [`Notices`] is the one entity the
//! surfaces observe, nudged by `updater::on_change`.

use std::time::Duration;

use gpui_kit::{App, AppContext as _, Context, Entity, Global};

use crate::updater::{self, Outcome, Ready, State};

/// How long a settled answer stays: PwrSnap's 8 s.
pub const ANSWER_FOR: Duration = Duration::from_secs(8);

/// A settled answer to Check for Updates… that leaves nothing to press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub title: String,
    pub message: String,
    /// The danger tone: the check failed.
    pub error: bool,
}

/// What the main window's toast shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Toast {
    /// Check for Updates… is asking GitHub.
    Checking,
    /// Check for Updates… found a release and is fetching it.
    Downloading {
        version: String,
        switch_back: bool,
        percent: Option<u8>,
    },
    /// Check for Updates… settled with nothing to press. `generation`
    /// names this answer, so its countdown starts once and a later answer
    /// starts its own.
    Answer { answer: Answer, generation: u64 },
    /// A release is downloaded for the slot in force.
    Ready { ready: Ready, restarting: bool },
}

/// What the surfaces remember between the updater's changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notices {
    /// Check for Updates… is waiting for the check after this many have
    /// finished (`State::finished`): the one it started, or the one that
    /// was already running.
    asked: Option<u64>,
    /// The last answer, until its countdown ends or another replaces it.
    answer: Option<(Answer, u64)>,
    generation: u64,
    /// Later on this release: its toast stays down. The tray row and the
    /// strip chip still offer it.
    dismissed: Option<Ready>,
    /// The updater as of its last change, and the release ready then:
    /// what the surfaces draw from, so a frame never reads the disk
    /// (`updater::ready` reads the install's manifest).
    seen: State,
    ready: Option<Ready>,
}

impl Notices {
    /// Check for Updates… was chosen while `finished` checks had ended.
    pub fn ask(&mut self, finished: u64) {
        self.asked = Some(finished);
        self.answer = None;
        // Asking again brings a dismissed release back.
        self.dismissed = None;
    }

    /// Takes in the updater's state after a change. Returns the new
    /// answer's generation when one settled, for its countdown.
    pub fn follow(&mut self, state: &State, ready: Option<&Ready>) -> Option<u64> {
        let after = self.asked?;
        if state.checking || state.finished <= after {
            return None;
        }
        self.asked = None;
        let outcome = state.last.as_ref().map(|(_, outcome)| outcome)?;
        let answer = answer_of(outcome, ready)?;
        self.generation += 1;
        self.answer = Some((answer, self.generation));
        Some(self.generation)
    }

    /// The countdown of answer `generation` ended.
    pub fn expire(&mut self, generation: u64) {
        if self.answer.as_ref().is_some_and(|(_, g)| *g == generation) {
            self.answer = None;
        }
    }

    /// Close: the answer, or Later on the ready release.
    pub fn dismiss(&mut self, ready: Option<&Ready>) {
        if self.answer.take().is_none() {
            self.dismissed = ready.cloned();
        }
    }

    /// What the toast shows for `state`, with `ready` the release Restart
    /// to update would install.
    pub fn toast(&self, state: &State, ready: Option<&Ready>) -> Option<Toast> {
        if self.asked.is_some() {
            return Some(match &state.download {
                Some(download) => Toast::Downloading {
                    version: download.version.clone(),
                    switch_back: download.switch_back,
                    percent: download.percent,
                },
                None => Toast::Checking,
            });
        }
        if let Some((answer, generation)) = &self.answer {
            return Some(Toast::Answer {
                answer: answer.clone(),
                generation: *generation,
            });
        }
        let ready = ready.filter(|ready| self.dismissed.as_ref() != Some(*ready))?;
        Some(Toast::Ready {
            ready: ready.clone(),
            restarting: state.restarting,
        })
    }
}

/// The answer a settled check gives, or `None` when the ready release is
/// the answer (its own card says it).
pub fn answer_of(outcome: &Outcome, ready: Option<&Ready>) -> Option<Answer> {
    let answer = |title: &str, message: String| Answer {
        title: title.to_owned(),
        message,
        error: false,
    };
    match outcome {
        Outcome::Downloaded(_) | Outcome::Pending(_) | Outcome::SwitchReady(_)
            if ready.is_some() =>
        {
            None
        }
        Outcome::Downloaded(version) | Outcome::Pending(version) => Some(answer(
            "Update downloaded",
            format!("{version} installs when PwrRoute quits."),
        )),
        Outcome::SwitchReady(version) => Some(answer(
            "Switch downloaded",
            format!("{version} installs from Settings \u{203a} Updates."),
        )),
        Outcome::UpToDate(version) => Some(answer(
            "PwrRoute is up to date",
            format!("{version} is the newest release on your channel."),
        )),
        Outcome::NothingPublished(slot) => Some(answer(
            "Nothing to install",
            format!("Nothing is published on {slot} yet."),
        )),
        Outcome::NotInstalled(_) => Some(answer("Updates unavailable", not_updatable())),
        Outcome::Failed(why) => Some(Answer {
            title: "Update check failed".to_owned(),
            message: format!("{}. Routing is unaffected.", why.trim_end_matches('.')),
            error: true,
        }),
        // Never settles a check: the running one does.
        Outcome::AlreadyChecking => None,
    }
}

/// Why this copy cannot update itself, as Settings › Updates says it.
fn not_updatable() -> String {
    if cfg!(debug_assertions) {
        "A development build: updates are off here.".to_owned()
    } else {
        "This copy was not installed by Velopack, so whatever installed it updates it.".to_owned()
    }
}

/// The ready release's words, shared by the toast, the tray and the strip.
pub fn ready_title(ready: &Ready) -> &'static str {
    if ready.switch_back {
        "Switch ready"
    } else {
        "Update ready"
    }
}

/// "Restart to update to 0.15.0" (or "Restart to switch to 0.13.0").
pub fn restart_label(ready: &Ready) -> String {
    let verb = if ready.switch_back {
        "switch"
    } else {
        "update"
    };
    format!("Restart to {verb} to {}", ready.version)
}

/// The release's page on GitHub, for Release notes. A version that is
/// not plain SemVer (a dev stamp) gets none.
pub fn release_notes_url(version: &str) -> Option<String> {
    crate::update_channel::parse(version)?;
    Some(format!(
        "https://github.com/pwrdrvr/PwrRoute/releases/tag/v{version}"
    ))
}

struct NoticesGlobal(Entity<Notices>);

impl Global for NoticesGlobal {}

/// The entity the toast, the tray and the strip observe.
pub fn notices(cx: &App) -> Option<Entity<Notices>> {
    cx.try_global::<NoticesGlobal>()
        .map(|global| global.0.clone())
}

/// Makes [`notices`] and follows the updater. Call once, after
/// `updater::install`.
pub fn install(cx: &mut App) {
    if cx.has_global::<NoticesGlobal>() {
        return;
    }
    let fixture = fixture();
    let live = fixture.is_none();
    let entity = cx.new(|_| {
        fixture.unwrap_or_else(|| Notices {
            seen: updater::state(),
            ready: updater::ready(),
            ..Notices::default()
        })
    });
    cx.set_global(NoticesGlobal(entity));
    if live {
        updater::on_change(follow);
    }
    crate::tray::follow_updates(cx);
}

/// `PWRROUTE_UPDATE_FIXTURE=checking`, `downloading`, `up-to-date`,
/// `failed` or `ready` draws that toast (and, for `ready`, the tray banner
/// and the strip chip) for screenshots and manual checks; the surfaces
/// then ignore the updater. The answers stay up: nothing counts them down.
fn fixture() -> Option<Notices> {
    let kind = std::env::var("PWRROUTE_UPDATE_FIXTURE").ok()?;
    let mut notices = Notices::default();
    match kind.as_str() {
        "checking" => {
            notices.asked = Some(0);
            notices.seen.checking = true;
        }
        "downloading" => {
            notices.asked = Some(0);
            notices.seen.checking = true;
            notices.seen.download = Some(updater::Download {
                version: "0.15.0".into(),
                switch_back: false,
                percent: Some(42),
            });
        }
        "up-to-date" => {
            notices.answer = answer_of(&Outcome::UpToDate("0.14.0".into()), None).map(|a| (a, 1));
        }
        "failed" => {
            notices.answer = answer_of(
                &Outcome::Failed("Could not reach GitHub Releases (timed out)".into()),
                None,
            )
            .map(|a| (a, 1));
        }
        "ready" => {
            notices.ready = Some(Ready {
                version: "0.15.0".into(),
                switch_back: false,
            });
        }
        _ => return None,
    }
    Some(notices)
}

/// The updater changed: settle a waiting check and redraw the surfaces.
fn follow(cx: &mut App) {
    let Some(entity) = notices(cx) else {
        return;
    };
    let state = updater::state();
    // A download reports every percent: while one runs and nothing new is
    // held, the ready release is the one already read (`updater::ready`
    // reads the install's manifest).
    let unchanged = {
        let notices = entity.read(cx);
        state.download.is_some() && state.held == notices.seen.held
    };
    let ready = if unchanged {
        entity.read(cx).ready.clone()
    } else {
        updater::ready()
    };
    entity.update(cx, |notices, cx| {
        if let Some(generation) = notices.follow(&state, ready.as_ref()) {
            count_down(generation, cx);
        }
        notices.seen = state;
        notices.ready = ready;
        cx.notify();
    });
}

fn count_down(generation: u64, cx: &mut Context<'_, Notices>) {
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(ANSWER_FOR).await;
        let _ = this.update(cx, |notices, cx| {
            notices.expire(generation);
            cx.notify();
        });
    })
    .detach();
}

/// Check for Updates…: the main window's toast answers it. With no main
/// window open, Settings › Updates opens and answers it inline instead.
pub fn check_for_updates(cx: &mut App) {
    let main_open = cx
        .windows()
        .into_iter()
        .any(|window| window.downcast::<crate::app::PwrRoute>().is_some());
    if !main_open {
        crate::settings::open(cx, Some(crate::settings::Pane::Updates));
        updater::check_in_background(updater::Trigger::Settings);
        return;
    }
    if let Some(entity) = notices(cx) {
        let state = updater::state();
        entity.update(cx, |notices, cx| {
            notices.ask(state.finished);
            notices.seen = state;
            cx.notify();
        });
    }
    updater::check_in_background(updater::Trigger::Menu);
}

/// Restart to update, from the toast, the tray or the strip. The toast
/// says "Restarting…" from the updater's state (`State::restarting`).
pub fn restart(_: &mut App) {
    updater::restart_in_background();
}

/// Close or Later on the toast.
pub fn dismiss(cx: &mut App) {
    let Some(entity) = notices(cx) else {
        return;
    };
    entity.update(cx, |notices, cx| {
        let ready = notices.ready.clone();
        notices.dismiss(ready.as_ref());
        cx.notify();
    });
}

/// The toast as things stand.
pub fn toast(cx: &App) -> Option<Toast> {
    let notices = notices(cx)?.read(cx);
    notices.toast(&notices.seen, notices.ready.as_ref())
}

/// The release Restart to update installs, as of the updater's last
/// change: the strip's chip and the tray's row.
pub fn ready(cx: &App) -> Option<Ready> {
    notices(cx)?.read(cx).ready.clone()
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::updater::Download;

    fn ready(version: &str) -> Ready {
        Ready {
            version: version.to_owned(),
            switch_back: false,
        }
    }

    fn finished(state: &mut State, outcome: Outcome) {
        state.checking = false;
        state.download = None;
        state.finished += 1;
        state.last = Some((SystemTime::now(), outcome));
    }

    #[test]
    fn only_a_menu_check_raises_a_toast_until_a_release_is_ready() {
        let mut notices = Notices::default();
        // The startup check runs and finds nothing: no toast.
        let mut state = State {
            checking: true,
            ..State::default()
        };
        assert_eq!(notices.toast(&state, None), None);
        finished(&mut state, Outcome::UpToDate("0.14.0".into()));
        assert_eq!(notices.follow(&state, None), None);
        assert_eq!(notices.toast(&state, None), None);
        // A background download that lands is offered.
        assert!(matches!(
            notices.toast(&state, Some(&ready("0.15.0"))),
            Some(Toast::Ready { .. })
        ));
    }

    #[test]
    fn a_menu_check_shows_its_progress_then_counts_down_its_answer() {
        let mut notices = Notices::default();
        let mut state = State::default();
        notices.ask(state.finished);
        state.checking = true;
        assert_eq!(notices.toast(&state, None), Some(Toast::Checking));
        state.download = Some(Download {
            version: "0.15.0".into(),
            switch_back: false,
            percent: Some(42),
        });
        assert_eq!(
            notices.toast(&state, None),
            Some(Toast::Downloading {
                version: "0.15.0".into(),
                switch_back: false,
                percent: Some(42),
            })
        );
        finished(&mut state, Outcome::Failed("timed out".into()));
        let generation = notices.follow(&state, None).expect("an answer settles");
        let Some(Toast::Answer { answer, .. }) = notices.toast(&state, None) else {
            panic!("the answer shows");
        };
        assert!(answer.error);
        assert_eq!(answer.message, "timed out. Routing is unaffected.");
        // Another answer's countdown does not take this one down.
        notices.expire(generation + 1);
        assert!(notices.toast(&state, None).is_some());
        notices.expire(generation);
        assert_eq!(notices.toast(&state, None), None);
    }

    #[test]
    fn a_menu_check_waits_for_the_check_already_running() {
        let mut notices = Notices::default();
        let mut state = State {
            checking: true,
            finished: 3,
            ..State::default()
        };
        notices.ask(state.finished);
        // The running check has not ended: still waiting.
        assert_eq!(notices.follow(&state, None), None);
        finished(&mut state, Outcome::UpToDate("0.14.0".into()));
        assert!(notices.follow(&state, None).is_some());
    }

    #[test]
    fn a_download_is_answered_by_the_ready_card_which_later_puts_away() {
        let mut notices = Notices::default();
        let mut state = State::default();
        notices.ask(state.finished);
        finished(&mut state, Outcome::Downloaded("0.15.0".into()));
        let ready = ready("0.15.0");
        assert_eq!(notices.follow(&state, Some(&ready)), None);
        assert_eq!(
            notices.toast(&state, Some(&ready)),
            Some(Toast::Ready {
                ready: ready.clone(),
                restarting: false
            })
        );
        // Restart pressed: the card says so until the restart fails.
        state.restarting = true;
        assert!(matches!(
            notices.toast(&state, Some(&ready)),
            Some(Toast::Ready {
                restarting: true,
                ..
            })
        ));
        state.restarting = false;
        notices.dismiss(Some(&ready));
        assert_eq!(notices.toast(&state, Some(&ready)), None);
        // A newer release comes back up.
        assert!(
            notices
                .toast(&state, Some(&self::ready("0.15.1")))
                .is_some()
        );
        // Asking again brings the dismissed one back.
        notices.ask(state.finished);
        finished(&mut state, Outcome::Pending("0.15.0".into()));
        notices.follow(&state, Some(&ready));
        assert!(matches!(
            notices.toast(&state, Some(&ready)),
            Some(Toast::Ready { .. })
        ));
    }

    #[test]
    fn answers_say_what_a_check_found() {
        let up = answer_of(&Outcome::UpToDate("0.14.0".into()), None).unwrap();
        assert_eq!(up.title, "PwrRoute is up to date");
        assert!(!up.error);
        assert!(answer_of(&Outcome::NotInstalled("no manifest".into()), None).is_some());
        assert!(answer_of(&Outcome::AlreadyChecking, None).is_none());
        // Downloaded with no ready release (held for another slot) still
        // says something.
        assert!(answer_of(&Outcome::Downloaded("0.15.0".into()), None).is_some());
    }

    #[test]
    fn ready_words_and_notes() {
        let switch = Ready {
            version: "0.13.0".into(),
            switch_back: true,
        };
        assert_eq!(
            restart_label(&ready("0.15.0")),
            "Restart to update to 0.15.0"
        );
        assert_eq!(restart_label(&switch), "Restart to switch to 0.13.0");
        assert_eq!(ready_title(&switch), "Switch ready");
        assert_eq!(
            release_notes_url("0.15.0").as_deref(),
            Some("https://github.com/pwrdrvr/PwrRoute/releases/tag/v0.15.0")
        );
        assert_eq!(release_notes_url("dev"), None);
    }
}
