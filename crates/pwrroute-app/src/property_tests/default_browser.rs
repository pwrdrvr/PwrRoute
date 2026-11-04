use super::*;
use gpui_kit::TestAppContext;
use proptest::{prelude::*, test_runner::TestRunner};
use std::cell::Cell;

fn status(which: u8) -> Option<DefaultStatus> {
    let id = match which {
        0 => return None,
        1 => pwrroute_os::BUNDLE_ID,
        2 => "com.google.Chrome",
        _ => "org.mozilla.firefox",
    };
    Some(DefaultStatus {
        is_default: which == 1,
        http: Some(id.into()),
        https: Some(id.into()),
        other_copy: None,
    })
}
fn tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(POLL_EVERY);
    cx.run_until_parked();
}

fn approval(
    cx: &mut TestAppContext,
    approve: u64,
    lag: u64,
    failures: u64,
) -> Result<(), TestCaseError> {
    let os = Rc::new(RefCell::new(status(2)));
    let registrar = cx.update(|cx| install(Os::fake(os.clone(), Rc::new(Cell::new(0))), cx));
    cx.update(|cx| request(cx).unwrap());
    for second in 1..=approve + lag + failures + 1 {
        // Failed reads happen after OS convergence. No observable answer
        // should become a refusal, even if grace expires during failures.
        *os.borrow_mut() = if second > approve + lag && second <= approve + lag + failures {
            None
        } else {
            status(if second > approve + lag { 1 } else { 2 })
        };
        if second == approve {
            cx.update(activated);
        }
        tick(cx);
        let (state, waiting, kept, note) = registrar.read_with(cx, |r, _| {
            (r.state.clone(), r.waiting(), r.kept, r.card().note)
        });
        prop_assert!(!kept, "second {} note {:?}", second, note);
        if second > approve + lag + failures {
            prop_assert_eq!(state, DefaultState::PwrRoute);
            prop_assert!(!waiting, "first good read must end the wait");
        } else {
            prop_assert!(waiting, "second {} ended wait early", second);
        }
    }
    Ok(())
}

#[gpui_kit::test]
fn approved_requests_survive_generated_os_lag_and_failed_reads(cx: &mut TestAppContext) {
    let cx = RefCell::new(cx);
    TestRunner::new(ProptestConfig::with_cases(64))
        .run(
            &(1u64..30, 0u64..=3, 0u64..=3),
            |(second, lag, failures)| approval(&mut cx.borrow_mut(), second, lag, failures),
        )
        .unwrap();
}

#[derive(Clone, Debug)]
enum Cmd {
    SetOs(u8),
    Request,
    Second,
    Activate,
    Recheck,
}
fn command() -> impl Strategy<Value = Cmd> {
    prop_oneof![3 => (0u8..4).prop_map(Cmd::SetOs), 2 => Just(Cmd::Request),
                5 => Just(Cmd::Second), 2 => Just(Cmd::Activate), 2 => Just(Cmd::Recheck)]
}

fn sequence(cx: &mut TestAppContext, cmds: &[Cmd]) -> Result<(), TestCaseError> {
    let os = Rc::new(RefCell::new(status(2)));
    let registrar = cx.update(|cx| install(Os::fake(os.clone(), Rc::new(Cell::new(0))), cx));
    let mut before = DefaultState::Other("Google Chrome".into());
    let mut elapsed = 0;
    let mut requested = false;
    for cmd in cmds {
        let was_waiting = registrar.read_with(cx, |r, _| r.waiting());
        let mut read = false;
        match *cmd {
            Cmd::SetOs(which) => *os.borrow_mut() = status(which),
            Cmd::Request => {
                before = registrar.read_with(cx, |r, _| r.state.clone());
                elapsed = 0;
                requested = true;
                cx.update(|cx| request(cx).unwrap());
            }
            Cmd::Second => {
                elapsed += 1;
                tick(cx);
                read = was_waiting;
            }
            Cmd::Activate => {
                cx.update(activated);
                read = true;
            }
            Cmd::Recheck => {
                cx.update(|cx| refresh(false, cx));
                read = true;
            }
        }
        registrar.read_with(cx, |r, _| {
            prop_assert!(!(r.kept && r.waiting()));
            if r.kept {
                prop_assert_eq!(&r.state, &before, "after {:?}", cmd);
            }
            if requested && was_waiting && read && os.borrow().is_none() && elapsed < 60 {
                prop_assert!(r.waiting(), "unknown is not an answer: {:?}", cmd);
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[gpui_kit::test]
fn registrar_command_sequences_preserve_request_invariants(cx: &mut TestAppContext) {
    let cx = RefCell::new(cx);
    TestRunner::new(ProptestConfig::with_cases(64))
        .run(&prop::collection::vec(command(), 1..48), |cmds| {
            sequence(&mut cx.borrow_mut(), &cmds)
        })
        .unwrap();
}

#[gpui_kit::test]
fn activation_immediately_after_request_is_not_refusal(cx: &mut TestAppContext) {
    sequence(cx, &[Cmd::Request, Cmd::Activate]).unwrap();
    assert!(cx.update(|cx| registrar(cx).unwrap().read(cx).waiting()));
}

#[gpui_kit::test]
fn one_failed_poll_does_not_end_the_request(cx: &mut TestAppContext) {
    sequence(cx, &[Cmd::Request, Cmd::SetOs(0), Cmd::Second]).unwrap();
}
