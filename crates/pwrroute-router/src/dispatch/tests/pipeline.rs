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
        Ok(Reply { status: 200, location: None })
    }
}

fn background<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> mpsc::Receiver<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(f()); });
    rx
}

const DEADLINE: Duration = Duration::from_secs(2);

#[test]
fn blocked_expansion_does_not_starve_control_test_or_an_ordinary_dispatch() {
    let (entered, started) = mpsc::channel();
    let gate = Gate::default();
    let router = Router::spawn(
        Options { network: Some(Arc::new(BlockNetwork { entered, gate: gate.clone() })), ..Options::default() },
        Some(rules_factory(github_rules())), DryRunLauncher::new(),
    ).unwrap();
    let _release = Release(gate);
    let handle = router.handle();
    handle.dispatch(link("https://bit.ly/slow")).unwrap();
    started.recv_timeout(DEADLINE).unwrap();
    let h = handle.clone();
    let control = background(move || {
        h.set_retention(Retention { count: 12, hours: 0 }).unwrap();
        h.status().unwrap()
    });
    assert!(control.recv_timeout(DEADLINE).is_ok(), "control queued behind expansion");
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
    let router = Router::spawn(Options::default(), None,
        BlockLaunch { entered, gate: gate.clone() }).unwrap();
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
    assert!(response.recv_timeout(DEADLINE).is_ok(), "status queued behind launch");
}
