//! Bounded, latest-request UI work. QuickJS values are created, used and
//! dropped on this thread. A batch yields between engine calls, so menu
//! impacts and saved examples cannot monopolize interactive previews.
use std::{
    array,
    sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Lane { Preview, Tests, Link, Impact, Config, Import, Onboarding, Activity }
const LANES: usize = 8;
type Step = Box<dyn FnMut() -> bool>;
type Factory = Box<dyn FnOnce(Cancel) -> Step + Send>;
struct Pending { at: Instant, cancel: Cancel, factory: Factory }
struct Queue { pending: [Option<Pending>; LANES], tokens: [Option<Cancel>; LANES], closed: bool }
struct Shared { queue: Mutex<Queue>, wake: Condvar }

#[derive(Clone)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
    fn cancel(&self) { self.0.store(true, Ordering::Release); }
}

pub struct EngineWorker { client: Client }
#[derive(Clone)]
pub struct Client { shared: Arc<Shared> }
impl std::ops::Deref for EngineWorker { type Target = Client; fn deref(&self) -> &Client { &self.client } }
impl EngineWorker {
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue { pending: array::from_fn(|_| None), tokens: array::from_fn(|_| None), closed: false }),
            wake: Condvar::new(),
        });
        let worker = shared.clone();
        thread::Builder::new().name("pwrroute-ui-engine".into()).spawn(move || {
            let mut running: [Option<(Cancel, Step)>; LANES] = array::from_fn(|_| None);
            let mut next = 0;
            loop {
                let mut queue = worker.queue.lock().unwrap();
                if queue.closed { break; }
                let now = Instant::now();
                for (lane, task) in running.iter_mut().enumerate() {
                    if task.as_ref().is_some_and(|(cancel, _)| cancel.cancelled()) { *task = None; }
                    if queue.pending[lane].as_ref().is_some_and(|job| job.at <= now) {
                        let job = queue.pending[lane].take().unwrap();
                        // Construct engines outside the queue lock: submitting or
                        // closing must never wait for config evaluation.
                        drop(queue);
                        let cancel = job.cancel;
                        let step = (job.factory)(cancel.clone());
                        *task = Some((cancel, step));
                        queue = worker.queue.lock().unwrap();
                    }
                }
                let lane = (0..LANES).map(|i| (next + i) % LANES).find(|i| running[*i].is_some());
                if let Some(lane) = lane {
                    drop(queue);
                    next = (lane + 1) % LANES;
                    let (cancel, step) = running[lane].as_mut().unwrap();
                    if cancel.cancelled() || step() { running[lane] = None; }
                } else {
                    let delay = queue.pending.iter().flatten().map(|job| job.at.saturating_duration_since(Instant::now())).min();
                    drop(match delay {
                        Some(delay) => worker.wake.wait_timeout(queue, delay).unwrap().0,
                        None => worker.wake.wait(queue).unwrap(),
                    });
                }
            }
            // No join on GPUI: any current call finishes under its existing
            // engine/network deadline, then all engines are dropped here.
        }).expect("start UI engine worker");
        Self { client: Client { shared } }
    }
    pub fn client(&self) -> Client { self.client.clone() }
}
impl Client {
    pub fn cancel(&self, lane: Lane) {
        let mut queue = self.shared.queue.lock().unwrap();
        if let Some(token) = queue.tokens[lane as usize].take() { token.cancel(); }
        queue.pending[lane as usize] = None;
        self.shared.wake.notify_one();
    }

    /// At most one pending and one running job per lane. Replacements
    /// cancel both the old result and the remainder of its batch.
    pub fn submit<T: Send + 'static, F, S>(&self, lane: Lane, debounce: Duration, build: F) -> async_channel::Receiver<T>
    where F: FnOnce(Cancel) -> S + Send + 'static, S: FnMut() -> Option<T> + 'static {
        let (tx, rx) = async_channel::bounded(1);
        let cancel = Cancel(Arc::new(AtomicBool::new(false)));
        let factory: Factory = Box::new(move |cancel| {
            let mut step = build(cancel.clone());
            Box::new(move || {
                if cancel.cancelled() || tx.is_closed() { return true; }
                match step() {
                    Some(result) => { if !cancel.cancelled() { let _ = tx.try_send(result); } true }
                    None => false,
                }
            })
        });
        let mut queue = self.shared.queue.lock().unwrap();
        let i = lane as usize;
        if let Some(old) = queue.tokens[i].replace(cancel.clone()) { old.cancel(); }
        if !queue.closed { queue.pending[i] = Some(Pending { at: Instant::now() + debounce, cancel, factory }); }
        self.shared.wake.notify_one();
        rx
    }

    pub fn once<T: Send + 'static>(&self, lane: Lane, work: impl FnOnce() -> T + Send + 'static) -> async_channel::Receiver<T> {
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
        for cancel in queue.tokens.iter().flatten() { cancel.cancel(); }
        queue.pending = array::from_fn(|_| None);
        self.shared.wake.notify_one();
    }
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
        let first = worker.once(Lane::Preview, move || { entered.send(()).unwrap(); blocked.recv().unwrap(); 0 });
        started.recv().unwrap();
        let mut last = None;
        for n in 1..1000 { last = Some(worker.once(Lane::Preview, move || n)); }
        assert_eq!(worker.shared.queue.lock().unwrap().pending.iter().flatten().count(), 1);
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
            move || { if first { first = false; entered.send(()).unwrap(); blocked.recv().unwrap(); } None::<()> }
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
        let active = worker.once(Lane::Tests, move || { entered.send(()).unwrap(); blocked.recv().unwrap(); 1 });
        started.recv().unwrap();
        let pending = worker.once(Lane::Config, || panic!("closed worker ran pending job"));
        drop(worker);
        assert!(pending.recv_blocking().is_err());
        release.send(()).unwrap();
        assert!(active.recv_blocking().is_err());
    }
}

pub type Stamps = Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>;
pub fn stamps(engine: &pwrroute_engine::Engine) -> Stamps {
    engine.dependencies().into_iter().map(|path| { let at = modified(&path); (path, at) }).collect()
}
fn modified(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
pub fn unchanged(stamps: &Stamps) -> bool {
    stamps.iter().all(|(path, at)| modified(path) == *at)
}

struct Service(EngineWorker);
impl gpui_kit::Global for Service {}
/// Standalone importer/onboarding operations use one process-owned worker.
pub fn service(cx: &mut gpui_kit::App) -> Client {
    if cx.try_global::<Service>().is_none() { cx.set_global(Service(EngineWorker::new())); }
    cx.global::<Service>().0.client()
}
