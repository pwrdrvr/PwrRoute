//! Bounded I/O stages. Only plain links and resolved browsers cross these
//! threads; the resolver and all state transitions stay on the owner.
use super::*;
use std::{collections::VecDeque, sync::Condvar};

pub(super) const CAPACITY: usize = 64;
const LANES: usize = 4;

struct Budget {
    used: usize,
    closed: bool,
}

pub(super) struct Admission {
    budget: Mutex<Budget>,
    send_lock: Mutex<()>,
    changed: Condvar,
}

impl Default for Admission {
    fn default() -> Self {
        Self {
            budget: Mutex::new(Budget {
                used: 0,
                closed: false,
            }),
            send_lock: Mutex::new(()),
            changed: Condvar::new(),
        }
    }
}

impl Admission {
    pub(super) fn acquire(self: &Arc<Self>) -> std::result::Result<Permit, RouterGone> {
        let mut budget = self.budget.lock().unwrap_or_else(PoisonError::into_inner);
        while budget.used == CAPACITY && !budget.closed {
            budget = self
                .changed
                .wait(budget)
                .unwrap_or_else(PoisonError::into_inner);
        }
        if budget.closed {
            return Err(RouterGone);
        }
        budget.used += 1;
        Ok(Permit(self.clone()))
    }

    pub(super) fn send(
        &self,
        tx: &mpsc::SyncSender<Command>,
        command: Command,
    ) -> std::result::Result<(), RouterGone> {
        let _send = self
            .send_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let budget = self.budget.lock().unwrap_or_else(PoisonError::into_inner);
        if budget.closed {
            drop(budget);
            return Err(RouterGone);
        }
        drop(budget);
        // Never hold the budget while waiting for the bounded mailbox:
        // the owner must be able to release completed work's permits.
        tx.send(command).map_err(|_| RouterGone)
    }

    pub(super) fn close(&self, tx: &mpsc::SyncSender<Command>) {
        let _send = self
            .send_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut budget = self.budget.lock().unwrap_or_else(PoisonError::into_inner);
        budget.closed = true;
        drop(budget);
        self.changed.notify_all();
        let _ = tx.send(Command::Shutdown);
    }
}

pub(super) struct Permit(Arc<Admission>);
impl Drop for Permit {
    fn drop(&mut self) {
        let mut budget = self.0.budget.lock().unwrap_or_else(PoisonError::into_inner);
        budget.used -= 1;
        self.0.changed.notify_one();
    }
}

enum Reply {
    Dispatch(Option<mpsc::Sender<DispatchRecord>>),
    Test(mpsc::Sender<DispatchRecord>),
    Release(mpsc::Sender<std::result::Result<DispatchRecord, OpenLaterError>>),
}

pub(super) enum Reservation {
    Ready(Option<Took>),
    /// Catch needs the resolved privacy of earlier links before a later
    /// link can take either Catch or a Next counter.
    Ordered,
}

impl Reservation {
    pub(super) fn take(
        self,
        worker: &Worker,
        source: crate::link::Source,
        privacy: Privacy,
    ) -> Option<Took> {
        match self {
            Self::Ready(took) => took,
            Self::Ordered => worker
                .take_catch(source, privacy)
                .or_else(|| worker.take_override()),
        }
    }
}

struct Job {
    request: Queued,
    mode: Mode,
    reply: Reply,
    reservation: Reservation,
    policy: Option<ShortLinks>,
    expanded: Option<EngineLink>,
    expand_us: u64,
    started: Instant,
    expanding: bool,
    cancelled: Arc<AtomicBool>,
}

struct Expand {
    id: DispatchId,
    url: String,
    policy: ShortLinks,
    cancelled: Arc<AtomicBool>,
}

/// Policy invalidation cancels subsequent HTTP requests, including GET
/// after HEAD. The in-flight request retains Network's bounded deadline;
/// no timed-out worker is detached or replaced by another request thread.
struct CheckedNetwork<'a> {
    network: &'a (dyn Network + Send + Sync),
    cancelled: &'a AtomicBool,
}

impl Network for CheckedNetwork<'_> {
    fn request(
        &self,
        method: pwrroute_engine::Method,
        url: &str,
        timeout: Duration,
    ) -> Result<pwrroute_engine::Reply> {
        if self.cancelled.load(Ordering::SeqCst) {
            anyhow::bail!("shortener policy changed");
        }
        let reply = self.network.request(method, url, timeout)?;
        if self.cancelled.load(Ordering::SeqCst) {
            anyhow::bail!("shortener policy changed");
        }
        Ok(reply)
    }
}

pub(super) struct Expanded {
    id: DispatchId,
    link: EngineLink,
    micros: u64,
}

struct Launch {
    job: Job,
    record: DispatchRecord,
    browser: Browser,
    attempt: u8,
    diagnostic: bool,
    started: Instant,
}

struct LaunchWork {
    id: DispatchId,
    browser: Browser,
    attempt: u8,
    diagnostic: bool,
}

pub(super) struct Attempt {
    result: std::result::Result<(), String>,
    dry_run: bool,
}

/// A lane has at most one assigned item. Its thread is joined, never
/// detached: a deadline cannot leave an old launch running behind a retry.
struct Lane<T> {
    tx: Option<mpsc::Sender<T>>,
    thread: Option<thread::JoinHandle<()>>,
    busy: Option<String>,
}

impl<T: Send + 'static> Lane<T> {
    fn spawn(name: &str, mut run: impl FnMut(T) + Send + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(item) = rx.recv() {
                run(item);
            }
        })?;
        Ok(Self {
            tx: Some(tx),
            thread: Some(thread),
            busy: None,
        })
    }

    fn send(&mut self, item: T, key: String) {
        self.busy = Some(key);
        // The owning lane cannot stop while the actor is alive.
        if self.tx.as_ref().unwrap().send(item).is_err() {
            panic!("router I/O lane stopped");
        }
    }
}

impl<T> Drop for Lane<T> {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) struct Pipeline {
    expand: Vec<Lane<Expand>>,
    launch: Vec<Lane<LaunchWork>>,
    active: Vec<Option<Launch>>,
    template: Option<Box<dyn Launcher>>,
    dry_run: bool,
    log_enabled: Arc<AtomicBool>,
    waiting: VecDeque<Job>,
    launching: VecDeque<Launch>,
    outstanding: usize,
}

impl Pipeline {
    pub(super) fn new(
        launcher: Box<dyn Launcher>,
        tx: mpsc::SyncSender<Command>,
        network: Option<Arc<dyn Network + Send + Sync>>,
    ) -> std::io::Result<Self> {
        let dry_run = launcher.is_dry_run();
        let mut launchers = Vec::new();
        let template = if let Some(fork) = launcher.fork() {
            launchers.push(fork);
            for _ in 1..LANES {
                // A forkable launcher supplies the same capability each time.
                let Some(fork) = launcher.fork() else {
                    break;
                };
                launchers.push(fork);
            }
            Some(launcher)
        } else {
            launchers.push(launcher);
            None
        };
        let log_enabled = Arc::new(AtomicBool::new(false));
        let mut launch = Vec::new();
        for (lane, mut launcher) in launchers.into_iter().enumerate() {
            let tx = tx.clone();
            let log_enabled = log_enabled.clone();
            launch.push(Lane::spawn("pwrroute-launch", move |item: LaunchWork| {
                let dry_run = launcher.is_dry_run();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| if item.diagnostic {
                    launcher.launch_observed(&item.browser, &mut |detail| {
                        if log_enabled.load(Ordering::Relaxed) { log::info!(target: "pwrroute::link", "link {} launch attempt={}: {detail}", item.id.0, item.attempt); }
                    })
                } else { launcher.launch(&item.browser) })).map_err(|_| "the launcher panicked".to_owned()).and_then(|result| result.map_err(|e| format!("{e:#}")));
                let _ = tx.send(Command::Launched { lane, result: Attempt { result, dry_run } });
            })?);
        }
        // One client shared by a fixed number of expansion workers. No
        // request or client construction runs on the resolver thread.
        let client = Arc::new(std::sync::OnceLock::<Arc<dyn Network + Send + Sync>>::new());
        if let Some(network) = network {
            let _ = client.set(network);
        }
        let mut expand = Vec::new();
        for lane in 0..LANES {
            let tx = tx.clone();
            let client = client.clone();
            expand.push(Lane::spawn("pwrroute-expand", move |item: Expand| {
                let started = Instant::now();
                let link = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let network =
                        client.get_or_init(|| Arc::new(pwrroute_engine::HttpNetwork::new()));
                    let checked = CheckedNetwork {
                        network: network.as_ref(),
                        cancelled: &item.cancelled,
                    };
                    let link = item.policy.expand(&item.url, &checked);
                    if item.cancelled.load(Ordering::SeqCst) {
                        EngineLink::new(&item.url)
                    } else {
                        link
                    }
                }))
                .unwrap_or_else(|_| EngineLink::new(&item.url));
                let _ = tx.send(Command::Expanded {
                    lane,
                    job: Expanded {
                        id: item.id,
                        link,
                        micros: Timings::micros(started.elapsed()).max(1),
                    },
                });
            })?);
        }
        let active = (0..launch.len()).map(|_| None).collect();
        Ok(Self {
            expand,
            launch,
            active,
            template,
            dry_run,
            log_enabled,
            waiting: VecDeque::new(),
            launching: VecDeque::new(),
            outstanding: 0,
        })
    }

    fn key(&self, browser: &Browser) -> String {
        self.template.as_ref().map_or_else(
            || "serial".into(),
            |launcher| launcher.ordering_key(browser),
        )
    }
}

impl Worker {
    pub(super) fn run(mut self, rx: &mpsc::Receiver<Command>) {
        self.update_launch_logging();
        let mut stopping = false;
        while !stopping || self.io.outstanding > 0 {
            let Ok(command) = rx.recv() else {
                break;
            };
            match command {
                Command::Dispatch { request, reply } => {
                    self.accept(request, Mode::Dispatch, Reply::Dispatch(reply))
                }
                Command::Test { request, reply } => {
                    self.accept(request, Mode::Test, Reply::Test(reply))
                }
                Command::Release {
                    from,
                    how,
                    received,
                    received_at,
                    reply,
                    permit,
                } => {
                    if let Some(link) = self.recent.take_pending(from) {
                        let request = Queued {
                            _permit: permit,
                            id: DispatchId(self.next_id.fetch_add(1, Ordering::Relaxed)),
                            link,
                            forwarded: false,
                            received,
                            received_at,
                        };
                        self.accept(request, Mode::Release { from, how }, Reply::Release(reply));
                    } else {
                        let _ = reply.send(Err(OpenLaterError::NotPending));
                    }
                }
                Command::Replace { factory, reply } => {
                    let result = match build(Some(factory)) {
                        Ok(resolver) => {
                            self.resolver = Ok(resolver);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    };
                    self.refresh_config();
                    let _ = reply.send(result);
                }
                Command::Status { reply } => {
                    let (_, config_error) = self.refresh_config();
                    let _ = reply.send(RouterStatus {
                        config_error,
                        dry_run: self.io.dry_run,
                        journal_error: self
                            .journal
                            .as_ref()
                            .and_then(|j| j.error().map(str::to_owned)),
                    });
                }
                Command::Refresh => {
                    self.refresh_config();
                }
                Command::SetRetention { retention, reply } => {
                    self.recent.set_retention(retention);
                    self.options.retention = retention;
                    self.update_launch_logging();
                    let _ = reply.send(());
                }
                Command::SetJournal { config, reply } => {
                    self.journal = None;
                    self.journal = config.clone().map(Journal::new);
                    *self
                        .journal_config
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = config.clone();
                    self.options.journal = config;
                    self.update_launch_logging();
                    let _ = reply.send(());
                }
                Command::Expanded {
                    lane,
                    job: expanded,
                } => {
                    self.io.expand[lane].busy = None;
                    if let Some(job) = self
                        .io
                        .waiting
                        .iter_mut()
                        .find(|j| j.request.id == expanded.id)
                    {
                        job.expanded = Some(expanded.link);
                        job.expand_us = expanded.micros;
                        job.expanding = false;
                    }
                }
                Command::Launched { lane, result } => {
                    self.io.launch[lane].busy = None;
                    self.launched(lane, result);
                }
                Command::Shutdown => stopping = true,
            }
            self.pump();
        }
    }

    fn refresh_config(&mut self) -> (Option<ShortLinks>, Option<String>) {
        let refreshed = self.in_charge();
        let policy = refreshed
            .as_ref()
            .ok()
            .and_then(|resolver| resolver.short_links());
        let error = refreshed.err();
        for job in &self.io.waiting {
            if job.expanding && job.policy != policy {
                job.cancelled.store(true, Ordering::SeqCst);
            }
        }
        (policy, error)
    }

    fn update_launch_logging(&self) {
        self.io.log_enabled.store(
            self.options.diagnostic_log.is_some()
                && keeps_links(self.recent.retention(), self.options.journal.as_ref()),
            Ordering::Relaxed,
        );
    }

    fn accept(&mut self, request: Queued, mode: Mode, reply: Reply) {
        let started = Instant::now();
        let url = normalize_url(&request.link.url).ok();
        let policy = if url.is_some() {
            self.refresh_config().0
        } else {
            None
        };
        let state_waits = self
            .io
            .waiting
            .iter()
            .any(|job| matches!(job.reservation, Reservation::Ordered));
        let reservation = if mode != Mode::Dispatch || url.is_none() {
            Reservation::Ready(None)
        } else if state_waits
            || (self.catch.load(Ordering::SeqCst)
                && request.link.source != crate::link::Source::Api)
        {
            Reservation::Ordered
        } else {
            Reservation::Ready(self.take_override())
        };
        // None means expansion still needs scheduling. Invalid URLs never
        // make a network request or consume a counter.
        let expanded = match (&url, &policy) {
            (Some(url), Some(policy)) if policy.is_short(url) => None,
            _ => Some(EngineLink::new(url.as_deref().unwrap_or(&request.link.url))),
        };
        self.io.outstanding += 1;
        self.io.waiting.push_back(Job {
            request,
            mode,
            reply,
            reservation,
            policy,
            expanded,
            expand_us: 0,
            started,
            expanding: false,
            cancelled: Arc::new(AtomicBool::new(false)),
        });
    }

    fn pump(&mut self) {
        // Schedule expansion only on idle lanes; the owner never waits for
        // a worker's input queue. The admission permit bounds all stages,
        // including completed expansions waiting behind Catch.
        loop {
            let Some(lane) = self.io.expand.iter().position(|lane| lane.busy.is_none()) else {
                break;
            };
            let Some(index) = self
                .io
                .waiting
                .iter()
                .position(|job| job.expanded.is_none() && !job.expanding)
            else {
                break;
            };
            let policy = self.refresh_config().0;
            let job = &mut self.io.waiting[index];
            let url = normalize_url(&job.request.link.url).expect("expansion has a valid URL");
            job.policy = policy;
            if let Some(policy) = job.policy.as_ref().filter(|policy| policy.is_short(&url)) {
                let item = Expand {
                    id: job.request.id,
                    url,
                    policy: policy.clone(),
                    cancelled: job.cancelled.clone(),
                };
                job.expanding = true;
                self.io.expand[lane].send(item, String::new());
            } else {
                job.expanded = Some(EngineLink::new(url));
            }
        }
        loop {
            let mut ordered_seen = false;
            let ready = self.io.waiting.iter().position(|job| {
                let ordered = matches!(job.reservation, Reservation::Ordered);
                let blocked = ordered && ordered_seen;
                ordered_seen |= ordered;
                job.expanded.is_some() && !blocked
            });
            let Some(index) = ready else {
                break;
            };
            let mut job = self.io.waiting.remove(index).unwrap();
            let (policy, refresh_error) = self.refresh_config();
            let expanded = if policy == job.policy && !job.cancelled.load(Ordering::SeqCst) {
                job.expanded.take().unwrap()
            } else {
                // Routing uses the current, refreshed config. Never combine
                // a stale shortener policy's redirect with its replacement.
                EngineLink::new(
                    normalize_url(&job.request.link.url)
                        .unwrap_or_else(|_| job.request.link.url.clone()),
                )
            };
            let reservation = std::mem::replace(&mut job.reservation, Reservation::Ready(None));
            let mut record = self.process(
                &job.request,
                &job.mode,
                &expanded,
                reservation,
                refresh_error,
            );
            record.timings.queue_us = Timings::micros(job.started - job.request.received);
            record.timings.expand_us = job.expand_us;
            if job.mode == Mode::Test || record.browser.is_none() {
                self.finish(job, record);
            } else {
                let browser = record.browser.clone().unwrap();
                self.enqueue_launch(job, record, browser, 1, Instant::now());
            }
        }
        // FIFO among ready attempts with equal keys. A busy browser cannot
        // occupy a worker needed by a different browser. Fallback attempts
        // reenter this same scheduler rather than bypassing browser order.
        for lane in 0..self.io.launch.len() {
            if self.io.launch[lane].busy.is_some() {
                continue;
            }
            let ready = self.io.launching.iter().position(|item| {
                let key = self.io.key(&item.browser);
                !self
                    .io
                    .launch
                    .iter()
                    .any(|lane| lane.busy.as_ref() == Some(&key))
            });
            if let Some(index) = ready {
                let item = self.io.launching.remove(index).unwrap();
                let key = self.io.key(&item.browser);
                let work = LaunchWork {
                    id: item.record.id,
                    browser: item.browser.clone(),
                    attempt: item.attempt,
                    diagnostic: item.diagnostic,
                };
                self.io.active[lane] = Some(item);
                self.io.launch[lane].send(work, key);
            }
        }
    }

    fn enqueue_launch(
        &mut self,
        job: Job,
        record: DispatchRecord,
        browser: Browser,
        attempt: u8,
        started: Instant,
    ) {
        let diagnostic = self.options.diagnostic_log.is_some()
            && keeps_links(self.recent.retention(), self.options.journal.as_ref());
        self.io.launching.push_back(Launch {
            job,
            record,
            browser,
            attempt,
            diagnostic,
            started,
        });
    }

    fn launched(&mut self, lane: usize, result: Attempt) {
        let Attempt { result, dry_run } = result;
        let launch = self.io.active[lane]
            .take()
            .expect("launch completion owns a lane");
        let Launch {
            job,
            mut record,
            browser,
            attempt,
            started,
            ..
        } = launch;
        match result {
            Ok(()) => {
                record.steps.push(Step::Launched { dry_run });
                record.outcome = if dry_run {
                    Outcome::DryRun
                } else {
                    Outcome::Opened
                };
                record.browser = Some(browser);
            }
            Err(message) => {
                record.steps.push(Step::LaunchFailed { message });
                if attempt == 1 {
                    let mut fallback = self.fallback(&browser.url);
                    if !same_app(&browser, &fallback) && !is_self(&fallback) {
                        fallback.private = browser.private;
                        record.steps.push(Step::Fallback {
                            reason: format!("{} could not be opened", browser.name),
                        });
                        self.enqueue_launch(job, record, fallback, 2, started);
                        return;
                    }
                }
                record.outcome = Outcome::Failed;
            }
        }
        record.timings.launch_us = Timings::micros(started.elapsed());
        self.finish(job, record);
    }

    fn finish(&mut self, job: Job, mut record: DispatchRecord) {
        record.timings.total_us = Timings::micros(job.request.received.elapsed());
        if job.mode != Mode::Test {
            record = self.record(record);
        }
        match job.reply {
            Reply::Dispatch(Some(reply)) => {
                let _ = reply.send(record);
            }
            Reply::Dispatch(None) => {}
            Reply::Test(reply) => {
                let _ = reply.send(record);
            }
            Reply::Release(reply) => {
                let _ = reply.send(Ok(record));
            }
        }
        self.io.outstanding -= 1;
        // Dropping the request releases capacity only after record delivery.
    }
}
