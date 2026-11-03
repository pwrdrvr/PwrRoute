//! Bounded, latest-request UI work. QuickJS values are created, used and
//! dropped on this thread. A batch yields between engine calls, so menu
//! impacts and saved examples cannot monopolize interactive previews.
use std::{
    array,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Lane {
    Preview,
    Tests,
    Link,
    Impact,
    Config,
    Import,
    Onboarding,
    Activity,
    Collisions,
    CheckRule,
}
pub const LANES: usize = 10;
enum Progress { Yield, Waiting, Done }
type Step = Box<dyn FnMut() -> Progress>;
type Factory = Box<dyn FnOnce(Cancel) -> Step + Send>;
struct Pending {
    at: Instant,
    cancel: Cancel,
    factory: Factory,
    wake: Wake,
}
struct Queue {
    pending: [Option<Pending>; LANES],
    tokens: [Option<Cancel>; LANES],
    closed: bool,
}
struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
    active: AtomicUsize,
}
struct Running {
    cancel: Cancel,
    step: Step,
    wake: Wake,
    shared: Arc<Shared>,
}
impl Drop for Running {
    fn drop(&mut self) {
        self.shared.active.fetch_sub(1, Ordering::Release);
        self.shared.wake.notify_all();
    }
}

#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

/// A parked engine task resumes only after its I/O result is available.
#[derive(Clone)]
pub struct Wake {
    ready: Arc<AtomicBool>,
    shared: Weak<Shared>,
}
impl Wake {
    fn wake(&self) {
        if let Some(shared) = self.shared.upgrade() {
            // Serialize with the scheduler's check-and-wait to avoid lost wakes.
            let _queue = shared.queue.lock().unwrap();
            self.ready.store(true, Ordering::Release);
            shared.wake.notify_one();
        }
    }
}

pub struct EngineWorker {
    client: Client,
}
#[derive(Clone)]
pub struct Client {
    shared: Arc<Shared>,
}
impl std::ops::Deref for EngineWorker {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}
impl EngineWorker {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                pending: array::from_fn(|_| None),
                tokens: array::from_fn(|_| None),
                closed: false,
            }),
            wake: Condvar::new(),
            active: AtomicUsize::new(0),
        });
        let worker = shared.clone();
        thread::Builder::new()
            .name("pwrroute-ui-engine".into())
            .spawn(move || {
                let mut running: [Option<Running>; LANES] = array::from_fn(|_| None);
                let mut next = 0;
                loop {
                    let mut queue = worker.queue.lock().unwrap();
                    if queue.closed {
                        break;
                    }
                    let now = Instant::now();
                    for (lane, task) in running.iter_mut().enumerate() {
                        if task.as_ref().is_some_and(|task| task.cancel.cancelled()) {
                            *task = None;
                        }
                        if queue.pending[lane]
                            .as_ref()
                            .is_some_and(|job| job.at <= now)
                        {
                            let job = queue.pending[lane].take().unwrap();
                            // Construct engines outside the queue lock: submitting or
                            // closing must never wait for config evaluation.
                            worker.active.fetch_add(1, Ordering::Release);
                            drop(queue);
                            let cancel = job.cancel;
                            let step = (job.factory)(cancel.clone());
                            *task = Some(Running {
                                cancel,
                                step,
                                wake: job.wake,
                                shared: worker.clone(),
                            });
                            queue = worker.queue.lock().unwrap();
                        }
                    }
                    let lane = (0..LANES)
                        .map(|i| (next + i) % LANES)
                        .find(|i| running[*i].as_ref().is_some_and(|task| task.wake.ready.load(Ordering::Acquire)));
                    if let Some(lane) = lane {
                        drop(queue);
                        next = (lane + 1) % LANES;
                        let task = running[lane].as_mut().unwrap();
                        task.wake.ready.store(false, Ordering::Release);
                        match if task.cancel.cancelled() { Progress::Done } else { (task.step)() } {
                            Progress::Done => running[lane] = None,
                            Progress::Yield => { task.wake.ready.store(true, Ordering::Release); }
                            Progress::Waiting => {}
                        }
                    } else {
                        let delay = queue
                            .pending
                            .iter()
                            .flatten()
                            .map(|job| job.at.saturating_duration_since(Instant::now()))
                            .min();
                        drop(match delay {
                            Some(delay) => worker.wake.wait_timeout(queue, delay).unwrap().0,
                            None => worker.wake.wait(queue).unwrap(),
                        });
                    }
                }
                // No join on GPUI: any current call finishes under its existing
                // engine/network deadline, then all engines are dropped here.
            })
            .expect("start UI engine worker");
        Self {
            client: Client { shared },
        }
    }
    pub fn client(&self) -> Client {
        self.client.clone()
    }
}
impl Client {
    #[cfg(any(test, target_os = "macos"))]
    #[allow(
        dead_code,
        reason = "the macOS screens example compiles this module separately"
    )]
    pub fn is_idle(&self) -> bool {
        let queue = self.shared.queue.lock().unwrap();
        queue.pending.iter().all(Option::is_none) && self.shared.active.load(Ordering::Acquire) == 0
    }

    pub fn cancel(&self, lane: Lane) {
        let mut queue = self.shared.queue.lock().unwrap();
        if let Some(token) = queue.tokens[lane as usize].take() {
            token.cancel();
        }
        queue.pending[lane as usize] = None;
        self.shared.wake.notify_one();
    }

    /// At most one pending and one running job per lane. Replacements
    /// cancel both the old result and the remainder of its batch.
    pub fn submit<T: Send + 'static, F, S>(
        &self,
        lane: Lane,
        debounce: Duration,
        build: F,
    ) -> async_channel::Receiver<T>
    where
        F: FnOnce(Cancel) -> S + Send + 'static,
        S: FnMut() -> Option<T> + 'static,
    {
        self.submit_poll(lane, debounce, move |cancel, _| {
            let mut step = build(cancel);
            move || std::task::Poll::Ready(step())
        })
    }

    /// Pending parks this lane without busy polling; `wake` resumes it.
    pub fn submit_poll<T: Send + 'static, F, S>(
        &self, lane: Lane, debounce: Duration, build: F,
    ) -> async_channel::Receiver<T>
    where
        F: FnOnce(Cancel, Wake) -> S + Send + 'static,
        S: FnMut() -> std::task::Poll<Option<T>> + 'static,
    {
        #[cfg(test)]
        let debounce = { let _ = debounce; Duration::ZERO };
        let (tx, rx) = async_channel::bounded(1);
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let wake = Wake { ready: Arc::new(AtomicBool::new(true)), shared: Arc::downgrade(&self.shared) };
        let ready = wake.clone();
        let factory: Factory = Box::new(move |cancel| {
            let mut step = build(cancel.clone(), ready);
            Box::new(move || {
                if cancel.cancelled() || tx.is_closed() { return Progress::Done; }
                match step() {
                    std::task::Poll::Ready(Some(result)) => {
                        if !cancel.cancelled() { let _ = tx.try_send(result); }
                        Progress::Done
                    }
                    std::task::Poll::Ready(None) => Progress::Yield,
                    std::task::Poll::Pending => Progress::Waiting,
                }
            })
        });
        let mut queue = self.shared.queue.lock().unwrap();
        let i = lane as usize;
        if let Some(old) = queue.tokens[i].replace(cancel.clone()) { old.cancel(); }
        if !queue.closed {
            queue.pending[i] = Some(Pending { at: Instant::now() + debounce, cancel, factory, wake });
        }
        self.shared.wake.notify_one();
        rx
    }

    pub fn once<T: Send + 'static>(
        &self,
        lane: Lane,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> async_channel::Receiver<T> {
        self.submit(lane, Duration::ZERO, move |_| {
            let mut work = Some(work);
            move || Some(work.take().unwrap()())
        })
    }
}
impl Drop for EngineWorker {
    fn drop(&mut self) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.closed = true;
        for cancel in queue.tokens.iter().flatten() {
            cancel.cancel();
        }
        queue.pending = array::from_fn(|_| None);
        self.shared.wake.notify_one();
    }
}

// One fixed I/O thread per Test a link owner, with one running and one
// latest pending request. In-flight HTTP observes the existing network
// timeout; cancellation suppresses its result and all subsequent engine work.
type IoJob = Box<dyn FnOnce() + Send>;
struct IoQueue { pending: Option<IoJob>, latest: Option<Cancel>, closed: bool }
struct IoShared { queue: Mutex<IoQueue>, wake: Condvar }
pub struct IoWorker { client: IoClient }
#[derive(Clone)]
pub struct IoClient { shared: Arc<IoShared> }
impl IoWorker {
    pub fn new() -> Self {
        let shared = Arc::new(IoShared { queue: Mutex::new(IoQueue { pending: None, latest: None, closed: false }), wake: Condvar::new() });
        let worker = shared.clone();
        thread::Builder::new().name("pwrroute-link-io".into()).spawn(move || loop {
            let mut queue = worker.queue.lock().unwrap();
            while queue.pending.is_none() && !queue.closed { queue = worker.wake.wait(queue).unwrap(); }
            if queue.closed { break; }
            let job = queue.pending.take().unwrap();
            drop(queue);
            job();
        }).expect("start link I/O worker");
        Self { client: IoClient { shared } }
    }
    pub fn client(&self) -> IoClient { self.client.clone() }
    pub fn cancel(&self) { self.client.cancel(); }
}
impl IoClient {
    fn cancel(&self) {
        let mut queue = self.shared.queue.lock().unwrap();
        if let Some(token) = queue.latest.take() { token.cancel(); }
        queue.pending = None;
    }
    pub fn submit<T: Send + 'static>(&self, wake: Wake, work: impl FnOnce() -> T + Send + 'static) -> async_channel::Receiver<T> {
        let (tx, rx) = async_channel::bounded(1);
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let token = cancel.clone();
        let job = Box::new(move || {
            if !token.cancelled() && !tx.is_closed() {
                let result = work();
                if !token.cancelled() { let _ = tx.try_send(result); }
            }
            drop(tx);
            wake.wake();
        });
        let mut queue = self.shared.queue.lock().unwrap();
        if let Some(old) = queue.latest.replace(cancel) { old.cancel(); }
        if !queue.closed { queue.pending = Some(job); }
        self.shared.wake.notify_one();
        rx
    }
}
impl Drop for IoWorker {
    fn drop(&mut self) {
        let mut queue = self.client.shared.queue.lock().unwrap();
        queue.closed = true;
        if let Some(token) = queue.latest.take() { token.cancel(); }
        queue.pending = None;
        self.client.shared.wake.notify_one();
    }
}

pub type Stamps = Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>;
pub fn stamps(engine: &pwrroute_engine::Engine) -> Stamps {
    engine
        .dependencies()
        .into_iter()
        .map(|path| {
            let at = modified(&path);
            (path, at)
        })
        .collect()
}
/// Preserve every version evaluated by a batch, without repeated entries.
pub fn retain_stamps(all: &mut Stamps, observed: Stamps) {
    for entry in observed { if !all.contains(&entry) { all.push(entry); } }
}

fn modified(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
pub fn unchanged(stamps: &Stamps) -> bool {
    stamps.iter().all(|(path, at)| modified(path) == *at)
}

struct Service {
    tasks: [Option<gpui_kit::Task<()>>; LANES],
    worker: EngineWorker,
}
impl gpui_kit::Global for Service {}
/// Standalone importer/onboarding operations use one process-owned worker.
pub fn service(cx: &mut gpui_kit::App) -> Client {
    if cx.try_global::<Service>().is_none() {
        cx.set_global(Service {
            tasks: array::from_fn(|_| None),
            worker: EngineWorker::new(),
        });
    }
    cx.global::<Service>().worker.client()
}

#[cfg(test)]
impl Client {
    pub(crate) fn wait_idle(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.is_idle() {
                return;
            }
            let queue = self.shared.queue.lock().unwrap();
            assert!(Instant::now() < deadline, "UI engine worker did not settle");
            drop(
                self.shared
                    .wake
                    .wait_timeout(queue, Duration::from_millis(10))
                    .unwrap(),
            );
        }
    }
}
#[cfg(test)]
pub fn settle(cx: &mut gpui_kit::App) {
    for window in cx.windows() {
        if let Some(window) = window.downcast::<crate::app::PwrRoute>()
            && let Ok(client) = window.read(cx).map(|app| app.engine_worker.client())
        {
            client.wait_idle();
        }
    }
    if let Some(service) = cx.try_global::<Service>() {
        service.worker.wait_idle();
    }
}

pub fn hold(cx: &mut gpui_kit::App, lane: Lane, task: gpui_kit::Task<()>) {
    let _ = service(cx);
    cx.global_mut::<Service>().tasks[lane as usize] = Some(task);
}

pub fn cancel_service(cx: &mut gpui_kit::App, lane: Lane) {
    if cx.try_global::<Service>().is_some() {
        let service = cx.global_mut::<Service>();
        service.tasks[lane as usize] = None;
        service.worker.cancel(lane);
    }
}

/// GPUI's deterministic test scheduler forbids wakes from OS threads.
/// Tests wait on the same result channel when the completion task is
/// polled; production suspends without blocking the UI thread.
pub async fn receive<T>(
    receiver: async_channel::Receiver<T>,
) -> Result<T, async_channel::RecvError> {
    #[cfg(test)]
    {
        receiver.recv_blocking()
    }
    #[cfg(not(test))]
    {
        receiver.recv().await
    }
}

pub fn primary(path: Option<&std::path::Path>) -> Stamps {
    path.map(|path| vec![(path.to_path_buf(), modified(path))])
        .unwrap_or_default()
}
pub fn dependencies(mut before: Stamps, engine: &pwrroute_engine::Engine) -> Stamps {
    for entry in stamps(engine) {
        if !before.iter().any(|(path, _)| path == &entry.0) {
            before.push(entry);
        }
    }
    before
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn slow_work_does_not_block_submission_and_only_latest_pending_runs() {
        let worker = EngineWorker::new();
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let first = worker.once(Lane::Preview, move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            0
        });
        started.recv().unwrap();
        let mut last = None;
        for n in 1..1000 {
            last = Some(worker.once(Lane::Preview, move || n));
        }
        assert_eq!(
            worker
                .shared
                .queue
                .lock()
                .unwrap()
                .pending
                .iter()
                .flatten()
                .count(),
            1
        );
        release.send(()).unwrap();
        assert_eq!(last.unwrap().recv_blocking().unwrap(), 999);
        assert!(first.recv_blocking().is_err());
    }
    #[test]
    fn batches_yield_to_interactive_work_and_close_cancels_remaining_steps() {
        let worker = EngineWorker::new();
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let batch = worker.submit(Lane::Impact, Duration::ZERO, move |_| {
            let mut first = true;
            move || {
                if first {
                    first = false;
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                }
                None::<()>
            }
        });
        started.recv().unwrap();
        let interactive = worker.once(Lane::Preview, || 42);
        release.send(()).unwrap();
        assert_eq!(interactive.recv_blocking().unwrap(), 42);
        drop(worker);
        assert!(batch.recv_blocking().is_err());
    }
    #[test]
    fn closure_drops_pending_and_cancels_running_without_waiting() {
        let worker = EngineWorker::new();
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let active = worker.once(Lane::Tests, move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            1
        });
        started.recv().unwrap();
        let pending = worker.once(Lane::Config, || panic!("closed worker ran pending job"));
        drop(worker);
        assert!(pending.recv_blocking().is_err());
        release.send(()).unwrap();
        assert!(active.recv_blocking().is_err());
    }
    #[test]
    fn shutdown_drops_non_send_engine_state_on_its_owner_thread() {
        struct Owned(std::sync::mpsc::Sender<std::thread::ThreadId>);
        impl Drop for Owned {
            fn drop(&mut self) {
                let _ = self.0.send(std::thread::current().id());
            }
        }
        let worker = EngineWorker::new();
        let (created, ready) = mpsc::channel();
        let (dropped, finished) = mpsc::channel();
        let result = worker.submit(Lane::Preview, Duration::ZERO, move |_| {
            let owned = std::rc::Rc::new(Owned(dropped));
            created.send(()).unwrap();
            move || {
                let _ = &owned;
                None::<()>
            }
        });
        ready.recv().unwrap();
        drop(worker);
        assert_ne!(finished.recv().unwrap(), thread::current().id());
        assert!(result.recv_blocking().is_err());
    }
    fn io_wake() -> Wake { Wake { ready: Arc::new(AtomicBool::new(false)), shared: Weak::new() } }

    #[test]
    fn link_io_is_bounded_coalesced_and_shutdown_suppresses_results() {
        let io = IoWorker::new();
        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let first = io.client().submit(io_wake(), move || { entered.send(()).unwrap(); blocked.recv().unwrap(); 0 });
        started.recv().unwrap();
        let ran = Arc::new(AtomicUsize::new(0));
        let mut latest = None;
        for n in 1..1000 {
            let ran = ran.clone();
            latest = Some(io.client().submit(io_wake(), move || { ran.fetch_add(1, Ordering::Relaxed); n }));
        }
        assert!(io.client.shared.queue.lock().unwrap().pending.is_some());
        release.send(()).unwrap();
        assert_eq!(latest.unwrap().recv_blocking().unwrap(), 999);
        assert_eq!(ran.load(Ordering::Relaxed), 1);
        assert!(first.recv_blocking().is_err());

        let (entered, started) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let active = io.client().submit(io_wake(), move || { entered.send(()).unwrap(); blocked.recv().unwrap(); 1 });
        started.recv().unwrap();
        let pending = io.client().submit(io_wake(), || panic!("closed I/O owner ran pending request"));
        drop(io);
        assert!(pending.recv_blocking().is_err());
        release.send(()).unwrap();
        assert!(active.recv_blocking().is_err());
    }

    #[test]
    fn parked_engine_does_not_spin_and_retains_owner_state_until_woken() {
        let worker = EngineWorker::new();
        let (ready, started) = mpsc::channel();
        let steps = Arc::new(AtomicUsize::new(0));
        let ran = steps.clone();
        let result = worker.submit_poll(Lane::Link, Duration::ZERO, move |_, wake| {
            let owned = std::rc::Rc::new(42);
            let mut ready = Some(ready);
            move || {
                ran.fetch_add(1, Ordering::Relaxed);
                if let Some(ready) = ready.take() {
                    ready.send(wake.clone()).unwrap();
                    std::task::Poll::Pending
                } else { std::task::Poll::Ready(Some(*owned)) }
            }
        });
        let wake = started.recv().unwrap();
        let preview = worker.once(Lane::Preview, || 7);
        assert_eq!(preview.recv_blocking().unwrap(), 7);
        assert_eq!(steps.load(Ordering::Relaxed), 1);
        wake.wake();
        assert_eq!(result.recv_blocking().unwrap(), 42);
        assert_eq!(steps.load(Ordering::Relaxed), 2);
    }

}
