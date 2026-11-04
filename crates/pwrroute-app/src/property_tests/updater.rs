use super::*;
use proptest::prelude::*;
use std::{cell::Cell, sync::mpsc};

// Only the worker thread uses the seam, so existing direct tests of the
// Velopack path remain independent. Each fake fetch parks until the driver
// releases its channel. Finished is sent after actual coordinator publication.
#[derive(Default)]
pub(super) struct Environment {
    pub(super) coordinator: Mutex<Coordinator>,
    pub(super) stopping: AtomicBool,
    events: Mutex<Option<mpsc::Sender<Event>>>,
}
thread_local! {
    static ENVIRONMENT: Cell<Option<&'static Environment>> = const { Cell::new(None) };
    // One fixture per test thread, reused across generated cases. Worker
    // threads receive this explicit environment when spawned; unrelated
    // tests retain the ordinary app globals.
    static FIXTURE: &'static Environment = Box::leak(Box::new(Environment::default()));
}
pub(super) fn environment() -> Option<&'static Environment> {
    ENVIRONMENT.with(Cell::get)
}
pub(super) fn install_environment(environment: Option<&'static Environment>) {
    ENVIRONMENT.with(|slot| slot.set(environment));
}
const GUARD: Duration = Duration::from_secs(5);

enum Event {
    Parked(Check, mpsc::Sender<()>),
    Finished(Option<Check>),
}
pub(super) fn fetch(check: Check) -> Option<Outcome> {
    let events = environment()?.events.lock().unwrap().clone()?;
    let (tx, rx) = mpsc::channel();
    events.send(Event::Parked(check, tx)).unwrap();
    rx.recv_timeout(GUARD)
        .expect("model must release every parked fetch");
    Some(Outcome::UpToDate("model".into()))
}
pub(super) fn finished(next: Option<Check>) {
    if let Some(environment) = environment()
        && let Some(events) = environment.events.lock().unwrap().as_ref()
    {
        events.send(Event::Finished(next)).unwrap();
    }
}

#[derive(Clone, Debug)]
enum Cmd {
    Request(Trigger),
    Select(usize),
    Release(usize),
    Stop,
}
fn command() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        5 => prop::sample::select(vec![Trigger::Startup, Trigger::Timer, Trigger::Menu, Trigger::Settings, Trigger::Selection]).prop_map(Cmd::Request),
        3 => (0usize..4).prop_map(Cmd::Select),
        3 => (0usize..4).prop_map(Cmd::Release),
        1 => Just(Cmd::Stop),
    ]
}

struct Driver {
    events: mpsc::Receiver<Event>,
    parked: Vec<(Check, mpsc::Sender<()>)>,
    completed: Vec<(Check, usize)>,
}
impl Driver {
    fn park(&mut self) {
        match self.events.recv_timeout(GUARD).unwrap() {
            Event::Parked(check, release) => self.parked.push((check, release)),
            Event::Finished(_) => panic!("worker finished before its fetch was released"),
        }
    }
    fn release(&mut self, n: usize, at: usize) {
        if self.parked.is_empty() {
            return;
        }
        let (check, release) = self.parked.remove(n % self.parked.len());
        release.send(()).unwrap();
        match self.events.recv_timeout(GUARD).unwrap() {
            Event::Finished(next) => {
                self.completed.push((check, at));
                if next.is_some() {
                    self.park();
                }
            }
            Event::Parked(_, _) => panic!("publication must precede next fetch"),
        }
    }
}
impl Drop for Driver {
    fn drop(&mut self) {
        // Drain even after a property assertion, so shrinking never leaves
        // a worker or test hook in the next case.
        while !self.parked.is_empty() {
            self.release(0, usize::MAX);
        }
        *environment().unwrap().events.lock().unwrap() = None;
        stopping().store(false, Ordering::Release);
        *coordinator().lock().unwrap() = Coordinator::default();
        install_environment(None);
    }
}

fn run(cmds: &[Cmd]) -> Result<(), TestCaseError> {
    FIXTURE.with(|fixture| install_environment(Some(fixture)));
    // A persisted Beta selection from an earlier launch. Selecting Stable
    // Latest now starts the exact Selection/Menu schedule from #166.
    *coordinator().lock().unwrap() = Coordinator {
        selection: UpdateSettings::picked(Slot::BETA_PRERELEASE),
        ..Coordinator::default()
    };
    stopping().store(false, Ordering::Release);
    let (events, rx) = mpsc::channel();
    *environment().unwrap().events.lock().unwrap() = Some(events);
    let mut driver = Driver {
        events: rx,
        parked: Vec::new(),
        completed: Vec::new(),
    };
    let installed = installed_version();
    let mut slot = selection().resolve(&installed);
    let mut generation = 0;
    let mut manuals = Vec::new();
    let mut asked = false;
    let mut stopped = false;
    for (at, cmd) in cmds.iter().enumerate() {
        let idle = driver.parked.is_empty();
        match *cmd {
            Cmd::Request(trigger) => {
                if !stopped {
                    asked = true;
                    if trigger.is_manual() {
                        manuals.push((at, slot, generation));
                    }
                }
                check_in_background(trigger);
                if idle && !stopped {
                    driver.park();
                }
            }
            Cmd::Select(n) => {
                // Quit ends user interaction. Stop cases still exercise
                // ignored background requests and draining accepted work.
                if !stopped {
                    let pick = Slot::ALL[n];
                    let changed = slot != pick;
                    if changed {
                        generation += 1;
                        asked = true;
                    }
                    slot = pick;
                    set_selection(UpdateSettings::picked(pick));
                    if idle && changed {
                        driver.park();
                    }
                }
            }
            Cmd::Release(n) => driver.release(n, at),
            Cmd::Stop => {
                apply_pending_on_quit();
                stopped = true;
            }
        }
        prop_assert_eq!(
            state().checking,
            !driver.parked.is_empty(),
            "after {:?}",
            cmd
        );
    }
    while !driver.parked.is_empty() {
        driver.release(0, cmds.len());
    }
    if asked {
        prop_assert_eq!(state().last.map(|r| r.slot), Some(slot));
    }
    for (at, slot, request_generation) in manuals {
        if request_generation == generation {
            prop_assert!(
                driver
                    .completed
                    .iter()
                    .any(|(c, finished)| c.slot == slot && c.trigger.is_manual() && *finished > at),
                "manual at {} on {:?} had no later completion; commands {:?}",
                at,
                slot,
                cmds
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]
    #[test]
    fn updater_requests_and_worker_schedule_obey_the_selected_slot(cmds in prop::collection::vec(command(), 1..32)) {
        run(&cmds)?;
    }
}

#[test]
fn manual_check_during_selection_check_finishes_after_the_request() {
    run(&[Cmd::Select(0), Cmd::Request(Trigger::Menu)]).unwrap();
    run(&[
        Cmd::Request(Trigger::Startup),
        Cmd::Select(0),
        Cmd::Request(Trigger::Menu),
    ])
    .unwrap();
}
