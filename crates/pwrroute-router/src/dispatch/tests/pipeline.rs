use super::*;
use std::sync::Condvar;

// Always release blocked I/O before Router::drop joins its workers, even
// when an assertion fails. Timeouts diagnose a deadlock; gates order events.
#[derive(Clone, Default)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn wait(&self) {
        let (lock, changed) = &*self.0;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = changed.wait(open).unwrap();
        }
    }

    fn open(&self) {
        let (lock, changed) = &*self.0;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

struct Release(Gate);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.open();
    }
}

struct BlockNetwork {
    entered: mpsc::Sender<()>,
    gate: Gate,
}

impl Network for BlockNetwork {
    fn request(&self, _: Method, _: &str, _: Duration) -> Result<Reply> {
        let _ = self.entered.send(());
        self.gate.wait();
        Ok(Reply {
            status: 200,
            location: None,
        })
    }
}

fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx
}

const DEADLINE: Duration = Duration::from_secs(2);

#[test]
fn blocked_expansion_does_not_starve_control_test_or_an_ordinary_dispatch() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(rules_factory(github_rules())),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate);
    let handle = router.handle();
    handle.dispatch(link("https://bit.ly/slow")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let h = handle.clone();
    let control = background(move || {
        h.set_retention(Retention {
            count: 12,
            hours: 0,
        })
        .unwrap();
        h.status().unwrap()
    });
    assert!(
        control.recv_timeout(DEADLINE).is_ok(),
        "control queued behind expansion"
    );
    let h = handle.clone();
    let tested = background(move || h.test(link("https://example.com/test")));
    assert!(tested.recv_timeout(DEADLINE).unwrap().is_ok());
    let opened = background(move || handle.dispatch_and_wait(link("https://example.com/open")));
    assert!(opened.recv_timeout(DEADLINE).unwrap().is_ok());
}

struct BlockLaunch {
    entered: mpsc::Sender<()>,
    gate: Gate,
}

impl Launcher for BlockLaunch {
    fn launch(&mut self, _: &Browser) -> Result<()> {
        let _ = self.entered.send(());
        self.gate.wait();
        Ok(())
    }
}

#[test]
fn blocked_launch_does_not_starve_status_refresh_or_test() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options::default(),
        None,
        BlockLaunch {
            entered,
            gate: gate.clone(),
        },
    )
    .unwrap();
    let _release = Release(gate);
    let handle = router.handle();
    handle.dispatch(link("https://example.com/slow")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let response = background(move || {
        handle.refresh().unwrap();
        let status = handle.status().unwrap();
        handle.test(link("https://example.com/test")).unwrap();
        status
    });
    assert!(
        response.recv_timeout(DEADLINE).is_ok(),
        "status queued behind launch"
    );
}

#[derive(Clone)]
struct ParallelLaunch {
    entered: mpsc::Sender<Browser>,
    gate: Gate,
    fail_slow: bool,
}

impl Launcher for ParallelLaunch {
    fn fork(&self) -> Option<Box<dyn Launcher>> {
        Some(Box::new(self.clone()))
    }
    fn ordering_key(&self, browser: &Browser) -> String {
        browser.name.trim_end_matches(" Alias").to_owned()
    }
    fn launch(&mut self, browser: &Browser) -> Result<()> {
        self.entered.send(browser.clone()).unwrap();
        if browser.url.ends_with("/slow") {
            self.gate.wait();
        }
        if self.fail_slow && browser.name == "Slow" {
            anyhow::bail!("unavailable");
        }
        Ok(())
    }
}

struct PolicyResolver {
    policy: Arc<Mutex<ShortLinks>>,
    refreshed: Arc<AtomicU64>,
}

impl Resolver for PolicyResolver {
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        let private = link.url.contains("private");
        Ok(Resolved {
            browser: Browser {
                name: if link.url.contains("slow.example") {
                    "Slow"
                } else {
                    "Fast"
                }
                .into(),
                app_type: AppType::AppName,
                url: link.url.clone(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
                private,
                privacy: if private {
                    Privacy::Host
                } else {
                    Privacy::Full
                },
            },
            privacy: if private {
                Privacy::Host
            } else {
                Privacy::Full
            },
            error: None,
            rule_steps: Vec::new(),
        })
    }
    fn short_links(&self) -> Option<ShortLinks> {
        Some(self.policy.lock().unwrap().clone())
    }
    fn refresh(&self) -> std::result::Result<(), String> {
        self.refreshed.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn policy_factory(policy: Arc<Mutex<ShortLinks>>) -> ResolverFactory {
    Box::new(move || {
        Ok(Box::new(PolicyResolver {
            policy,
            refreshed: Arc::default(),
        }))
    })
}

#[test]
fn different_browsers_progress_but_same_browser_profiles_and_fallback_wait() {
    let (entered, events) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("Fast", ""),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::off())))),
        ParallelLaunch {
            entered,
            gate: gate.clone(),
            fail_slow: true,
        },
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    let slow = handle.dispatch(link("https://slow.example/slow")).unwrap();
    assert_eq!(events.recv_timeout(DEADLINE).unwrap().name, "Slow");
    let fast = handle
        .dispatch_and_wait(link("https://fast.example/first"))
        .unwrap();
    assert_eq!(events.recv_timeout(DEADLINE).unwrap().name, "Fast");
    assert!(handle.get(slow).is_none());
    assert_eq!(fast.outcome, Outcome::Opened);
    let same = handle
        .dispatch(link("https://slow.example/second"))
        .unwrap();
    handle.status().unwrap();
    // A second profile/alias must share the first browser's lane too.
    handle.set_send_to(Some(SendTo::next(Destination {
        browser: "Slow Alias".into(),
        profile: "Work".into(),
        private: false,
    })));
    let alias = handle.dispatch(link("https://slow.example/third")).unwrap();
    handle.status().unwrap();
    assert!(events.try_recv().is_err());
    gate.open();
    drop(router); // drains accepted attempts and their fallbacks
    let rest: Vec<_> = events.try_iter().collect();
    let slow_urls: Vec<_> = rest
        .iter()
        .filter(|b| b.name.starts_with("Slow"))
        .map(|b| b.url.as_str())
        .collect();
    assert_eq!(
        slow_urls,
        ["https://slow.example/second", "https://slow.example/third"]
    );
    assert_eq!(handle.get(slow).unwrap().browser.unwrap().name, "Fast");
    assert_eq!(handle.get(same).unwrap().browser.unwrap().name, "Fast");
    assert_eq!(
        handle.get(alias).unwrap().browser.unwrap().name,
        "Slow Alias"
    );
}

#[test]
fn admission_backpressures_every_public_ingress_without_losing_links() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options::default(),
        None,
        BlockLaunch {
            entered,
            gate: gate.clone(),
        },
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    handle.set_hold(Some(SendScope::next(1)));
    let held = handle
        .dispatch_and_wait(link("https://example.com/held"))
        .unwrap();
    for i in 0..super::super::pipeline::CAPACITY {
        handle
            .dispatch(link(&format!("https://example.com/{i}")))
            .unwrap();
    }
    started.recv_timeout(DEADLINE).unwrap();
    // All following public paths must wait for capacity rather than return
    // transient RouterGone or consume a held link that cannot be admitted.
    let h = handle.clone();
    let os = background(move || h.dispatch(link("https://example.com/os")));
    let h = handle.clone();
    let ipc = background(move || {
        h.answer(
            Request::Open {
                links: vec![link("https://example.com/ipc")],
            },
            || {},
        )
    });
    let h = handle.clone();
    let api = background(move || h.dispatch_and_wait(link("https://example.com/api")));
    let h = handle.clone();
    let test = background(move || h.test(link("https://example.com/test")));
    let h = handle.clone();
    let released = background(move || h.open_not_opened(held.id, OpenAs::Rules));
    handle.status().unwrap();
    assert!(os.try_recv().is_err());
    assert!(ipc.try_recv().is_err());
    assert!(api.try_recv().is_err());
    assert!(test.try_recv().is_err());
    assert!(released.try_recv().is_err());
    assert_eq!(handle.pending(), [held.id]);
    gate.open();
    let os_id = os.recv_timeout(DEADLINE).unwrap().unwrap();
    let ipc = ipc.recv_timeout(DEADLINE).unwrap();
    assert!(ipc.ok);
    assert_eq!(
        api.recv_timeout(DEADLINE).unwrap().unwrap().outcome,
        Outcome::Opened
    );
    assert_eq!(
        test.recv_timeout(DEADLINE).unwrap().unwrap().outcome,
        Outcome::Tested
    );
    assert_eq!(
        released.recv_timeout(DEADLINE).unwrap().unwrap().outcome,
        Outcome::Opened
    );
    drop(router);
    assert!(handle.get(os_id).is_some());
    assert!(handle.get(ipc.dispatched[0]).unwrap().forwarded);
    assert_eq!(handle.recent().len(), super::super::pipeline::CAPACITY + 5); // held + four real opens
    assert_eq!(
        started.try_iter().count() + 1,
        super::super::pipeline::CAPACITY + 4
    );
}

#[test]
fn shutdown_drains_accepted_records_and_wakes_capacity_waiters() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let (recorded, records) = mpsc::channel();
    let router = Router::spawn(
        Options {
            on_dispatch: Some(Arc::new(move |r| {
                recorded.send(r.id).unwrap();
            })),
            ..Options::default()
        },
        None,
        BlockLaunch {
            entered,
            gate: gate.clone(),
        },
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    let accepted: Vec<_> = (0..super::super::pipeline::CAPACITY)
        .map(|i| {
            handle
                .dispatch(link(&format!("https://example.com/{i}")))
                .unwrap()
        })
        .collect();
    started.recv_timeout(DEADLINE).unwrap();
    let h = handle.clone();
    let waiting = background(move || h.dispatch(link("https://example.com/unaccepted")));
    let shutdown = background(move || drop(router));
    assert_eq!(waiting.recv_timeout(DEADLINE).unwrap(), Err(RouterGone));
    assert!(shutdown.try_recv().is_err(), "active launch must be joined");
    gate.open();
    shutdown.recv_timeout(DEADLINE).unwrap();
    let recorded: Vec<_> = records.try_iter().collect();
    assert_eq!(recorded, accepted);
    assert_eq!(handle.status(), Err(RouterGone));
}

#[test]
fn short_link_can_finish_after_a_later_link_but_next_reservations_keep_admission_order() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        launcher.clone(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    handle.set_send_to(Some(next(2)));
    let first = handle.dispatch(link("https://bit.ly/slow")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let second = handle
        .dispatch_and_wait(link("https://fast.example/second"))
        .unwrap();
    assert_eq!(sent_step(&second).unwrap().0, Sent::Next { nth: 2, of: 2 });
    assert!(handle.send_to().is_none());
    handle.set_hold(Some(SendScope::next(1)));
    let third = handle
        .dispatch_and_wait(link("https://fast.example/third"))
        .unwrap();
    assert_eq!(third.outcome, Outcome::NotOpened);
    handle.set_paused(Some(PauseTarget {
        browser: "Paused".into(),
        profile: String::new(),
    }));
    gate.open();
    drop(router);
    let first = handle.get(first).unwrap();
    assert_eq!(sent_step(&first).unwrap().0, Sent::Next { nth: 1, of: 2 });
    assert_eq!(first.browser.unwrap().name, "Microsoft Edge");
    let launched = launcher.launched();
    assert_eq!(launched[0].url, second.link.url);
    assert_eq!(launched[1].url, "https://bit.ly/slow");
}

#[test]
fn catch_waits_for_earlier_private_resolution_then_preserves_next_counter() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    handle.set_send_to(Some(next(2)));
    handle.catch_next(true);
    let first = handle.dispatch(link("https://bit.ly/private")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let second = handle
        .dispatch(link("https://fast.example/second"))
        .unwrap();
    let third = handle.dispatch(link("https://fast.example/third")).unwrap();
    handle.status().unwrap();
    assert!(handle.get(second).is_none());
    gate.open();
    drop(router);
    let first = handle.get(first).unwrap();
    assert_eq!(first.privacy, Privacy::Host);
    assert!(!first.held_for_rule());
    assert_eq!(sent_step(&first).unwrap().0, Sent::Next { nth: 1, of: 2 });
    assert!(handle.get(second).unwrap().held_for_rule());
    assert_eq!(
        sent_step(&handle.get(third).unwrap()).unwrap().0,
        Sent::Next { nth: 2, of: 2 }
    );
}

#[test]
fn policy_changes_discard_inflight_expansion_and_status_is_a_refresh_barrier() {
    struct Redirect {
        gate: Gate,
        entered: mpsc::Sender<()>,
        following: Arc<AtomicU64>,
    }
    impl Network for Redirect {
        fn request(&self, _: Method, url: &str, _: Duration) -> Result<Reply> {
            if url.contains("bit.ly") {
                self.entered.send(()).unwrap();
                self.gate.wait();
                Ok(Reply {
                    status: 302,
                    location: Some("https://slow.example/landed".into()),
                })
            } else {
                self.following.fetch_add(1, Ordering::SeqCst);
                Ok(Reply {
                    status: 200,
                    location: None,
                })
            }
        }
    }
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let policy = Arc::new(Mutex::new(ShortLinks::default()));
    let following = Arc::new(AtomicU64::new(0));
    let refreshed = Arc::new(AtomicU64::new(0));
    let factory = {
        let policy = policy.clone();
        let refreshed = refreshed.clone();
        Box::new(move || Ok(Box::new(PolicyResolver { policy, refreshed }) as Box<dyn Resolver>))
    };
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(Redirect {
                gate: gate.clone(),
                entered,
                following: following.clone(),
            })),
            ..Options::default()
        },
        Some(factory),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    let id = handle.dispatch(link("https://bit.ly/slow")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    *policy.lock().unwrap() = ShortLinks::off();
    let before = refreshed.load(Ordering::SeqCst);
    handle.status().unwrap();
    assert!(refreshed.load(Ordering::SeqCst) > before);
    gate.open();
    drop(router);
    let record = handle.get(id).unwrap();
    assert_eq!(
        following.load(Ordering::SeqCst),
        0,
        "policy invalidation must cancel further requests"
    );
    assert_eq!(record.final_url.as_deref(), Some("https://bit.ly/slow"));
    assert_eq!(record.browser.unwrap().name, "Fast");
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Expanded { .. }))
    );
}

#[test]
fn expansion_concurrency_is_fixed_and_completed_work_cannot_grow_the_queue() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    for i in 0..12 {
        handle
            .dispatch(link(&format!("https://bit.ly/{i}")))
            .unwrap();
    }
    for _ in 0..4 {
        started.recv_timeout(DEADLINE).unwrap();
    }
    handle.status().unwrap();
    assert!(
        started.try_recv().is_err(),
        "more than four network requests started"
    );
    // A normal link needs neither an expansion slot nor a busy launch lane.
    assert_eq!(
        handle
            .dispatch_and_wait(link("https://fast.example/pass"))
            .unwrap()
            .outcome,
        Outcome::DryRun
    );
    gate.open();
    drop(router);
    assert_eq!(started.try_iter().count(), 8);
    assert_eq!(handle.recent().len(), 13);
    // Completion order must not turn the old short links into newer clicks.
    assert_eq!(handle.recent()[0].link.url, "https://fast.example/pass");
}

#[test]
fn admitted_scope_survives_expiry_and_replacement_but_hold_needs_retention_at_commit() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let (recorded, records) = mpsc::channel();
    let router = Router::spawn(
        Options {
            on_dispatch: Some(Arc::new(move |r| {
                recorded.send(r.clone()).unwrap();
            })),
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    let mut send = next(1);
    send.scope = SendScope::All { until: None };
    handle.set_send_to(Some(send));
    let sent = handle.dispatch(link("https://bit.ly/sent")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    handle.rescope(SendScope::All {
        until: Some(SystemTime::UNIX_EPOCH),
    });
    let ordinary = handle
        .dispatch_and_wait(link("https://fast.example/ordinary"))
        .unwrap();
    assert!(sent_step(&ordinary).is_none());
    handle.set_hold(Some(SendScope::next(1)));
    let held = handle.dispatch(link("https://bit.ly/held")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    handle
        .set_retention(Retention { count: 0, hours: 0 })
        .unwrap();
    // Callbacks still deliver completed records while memory retention is off.
    gate.open();
    drop(router);
    let records: Vec<_> = records.try_iter().collect();
    let sent_record = records.iter().find(|r| r.id == sent).unwrap();
    assert_eq!(sent_record.browser.as_ref().unwrap().name, "Microsoft Edge");
    let held_record = records.iter().find(|r| r.id == held).unwrap();
    assert_eq!(held_record.outcome, Outcome::DryRun);
    assert!(
        !held_record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Held { .. }))
    );
    assert!(!handle.pending().contains(&held));
    assert!(handle.get(sent).is_none());
}

#[test]
fn cancelling_catch_and_changing_mode_while_privacy_is_pending_commits_once() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options {
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    handle.catch_next(true);
    handle.set_hold(Some(SendScope::next(2)));
    let first = handle.dispatch(link("https://bit.ly/private")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let second = handle
        .dispatch(link("https://fast.example/second"))
        .unwrap();
    handle.status().unwrap();
    // Catch deferred these state decisions until privacy became known. A
    // control change before that commit governs them, in admission order.
    handle.catch_next(false);
    handle.set_send_to(Some(next(2)));
    gate.open();
    drop(router);
    assert_eq!(
        sent_step(&handle.get(first).unwrap()).unwrap().0,
        Sent::Next { nth: 1, of: 2 }
    );
    assert_eq!(
        sent_step(&handle.get(second).unwrap()).unwrap().0,
        Sent::Next { nth: 2, of: 2 }
    );
    assert!(handle.pending().is_empty());
}

#[test]
fn expiry_boundary_and_rejected_normalization_do_not_consume_next() {
    let deadline = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
    let mut send = next(2);
    send.scope = SendScope::All {
        until: Some(deadline),
    };
    let mut slot = Some(Override::Sending(send));
    assert!(matches!(
        take_override_from(&mut slot, false, deadline - Duration::from_nanos(1)),
        Some(Took::Sent(_, Sent::All))
    ));
    assert!(take_override_from(&mut slot, false, deadline).is_none());
    assert!(slot.is_none());

    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    handle.set_send_to(Some(next(2)));
    for invalid in [" ", "https://", "relative/path"] {
        let record = handle.dispatch_and_wait(link(invalid)).unwrap();
        assert!(
            record
                .steps
                .iter()
                .any(|s| matches!(s, Step::Rejected { .. }))
        );
    }
    assert_eq!(handle.send_to().unwrap().scope, SendScope::next(2));
    let valid = handle
        .dispatch_and_wait(link("  https://example.com/trimmed \n"))
        .unwrap();
    assert_eq!(sent_step(&valid).unwrap().0, Sent::Next { nth: 1, of: 2 });
    assert_eq!(
        valid.final_url.as_deref(),
        Some("https://example.com/trimmed")
    );
}

#[test]
fn count_retention_keeps_newest_receipts_when_old_expansion_completes_last() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let (recorded, delivered) = mpsc::channel();
    let router = Router::spawn(
        Options {
            retention: Retention { count: 2, hours: 0 },
            network: Some(Arc::new(BlockNetwork {
                entered,
                gate: gate.clone(),
            })),
            on_dispatch: Some(Arc::new(move |r| {
                recorded.send(r.id).unwrap();
            })),
            ..Options::default()
        },
        Some(policy_factory(Arc::new(Mutex::new(ShortLinks::default())))),
        DryRunLauncher::new(),
    )
    .unwrap();
    let _release = Release(gate.clone());
    let handle = router.handle();
    let first = handle.dispatch(link("https://bit.ly/oldest")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let second = handle
        .dispatch_and_wait(link("https://fast.example/second"))
        .unwrap();
    let third = handle
        .dispatch_and_wait(link("https://fast.example/third"))
        .unwrap();
    gate.open();
    drop(router);
    assert_eq!(
        delivered.try_iter().collect::<Vec<_>>(),
        [second.id, third.id, first]
    );
    assert_eq!(
        handle.recent().iter().map(|r| r.id).collect::<Vec<_>>(),
        [third.id, second.id]
    );
    assert!(handle.get(first).is_none());
}
