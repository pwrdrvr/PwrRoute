//! The app's end of the link: accepts connections from `pwrroute mcp` and
//! serves MCP on each, one rmcp session per connection.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result};
use rmcp::ServiceExt as _;
use tokio::{sync::Semaphore, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    agents::Agents,
    approval::{Approvals, Approver},
    audit::AuditSink,
    backend::Backends,
    link::{Endpoint, Listener},
    tools::{Connection, Tools},
};

/// Connections served at once; more are hung up on.
const MAX_CONNECTIONS: usize = 16;

/// What to listen on, and how long a new agent's first call waits for the
/// person.
#[derive(Debug, Clone)]
pub struct Config {
    pub endpoint: Endpoint,
    /// How long a tool call from a client not yet approved waits for the
    /// approval window before answering `approval_pending`.
    pub approval_wait: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: Endpoint::for_user(),
            approval_wait: Duration::from_secs(120),
        }
    }
}

/// A running server. Dropping it does not stop it; call [`Self::stop`].
pub struct McpServer {
    endpoint: Endpoint,
    approvals: Arc<Approvals>,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl McpServer {
    /// Listens on `config.endpoint` and serves until [`Self::stop`]. If
    /// another PwrRoute already listens there, this is an error rather than
    /// a takeover. Must be called inside a Tokio runtime.
    pub async fn start(
        config: Config,
        agents: Agents,
        backends: Backends,
        approver: Arc<dyn Approver>,
        audit: Arc<dyn AuditSink>,
    ) -> Result<Self> {
        let mut listener = Listener::bind(config.endpoint.path())
            .with_context(|| format!("listening for local agents at {}", config.endpoint))?;
        let approvals = Arc::new(Approvals::new(approver, agents.clone()));
        let shutdown = CancellationToken::new();
        let tools = Tools {
            backends,
            agents,
            audit,
            approvals: Arc::clone(&approvals),
            approval_wait: config.approval_wait,
        };

        let stop = shutdown.clone();
        let task = tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
            let next_id = AtomicU64::new(1);
            loop {
                let stream = tokio::select! {
                    accepted = listener.accept() => match accepted {
                        Ok(stream) => stream,
                        // A transient accept error is not worth stopping
                        // for; back off so a persistent one cannot spin.
                        Err(_) => {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            continue;
                        }
                    },
                    () = stop.cancelled() => break,
                };
                let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                    continue;
                };
                let handler = tools.for_connection(Connection::new(
                    next_id.fetch_add(1, Ordering::Relaxed),
                    tools.agents.clone(),
                ));
                let ct = stop.child_token();
                tokio::spawn(async move {
                    if let Ok(running) = handler.serve_with_ct(stream, ct).await {
                        let _ = running.waiting().await;
                    }
                    drop(slot);
                });
            }
            // Dropping the listener removes the socket file.
            drop(listener);
        });

        Ok(Self {
            endpoint: config.endpoint,
            approvals,
            shutdown,
            task,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Stops listening, closes every connection, and closes open approval
    /// windows unanswered. Approvals already given are kept.
    pub async fn stop(self) {
        self.approvals.close();
        self.shutdown.cancel();
        let _ = self.task.await;
    }
}

/// A [`McpServer`] on a thread of its own, with its own Tokio runtime, for
/// a host that has no runtime (the app, whose main thread is GPUI's).
/// [`Self::stop`] stops the server and joins the thread.
pub struct McpHost {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    endpoint: Endpoint,
}

impl std::fmt::Debug for McpHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHost")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl McpHost {
    /// Starts the server on a new thread and returns once it listens, or
    /// with the error that stopped it.
    pub fn start(
        config: Config,
        agents: Agents,
        backends: Backends,
        approver: Arc<dyn Approver>,
        audit: Arc<dyn AuditSink>,
    ) -> Result<Self> {
        let endpoint = config.endpoint.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("pwrroute-mcp".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.into()));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let server =
                        match McpServer::start(config, agents, backends, approver, audit).await {
                            Ok(server) => server,
                            Err(e) => {
                                let _ = ready_tx.send(Err(e));
                                return;
                            }
                        };
                    let _ = ready_tx.send(Ok(()));
                    // Blocks this thread's only task until told to stop,
                    // without holding the runtime: the server runs on it.
                    let stopped = tokio::task::spawn_blocking(move || stop_rx.recv());
                    let _ = stopped.await;
                    server.stop().await;
                });
            })
            .context("starting the local-agent thread")?;
        ready_rx
            .recv()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the local-agent thread exited")))?;
        Ok(Self {
            stop: Some(stop_tx),
            thread: Some(thread),
            endpoint,
        })
    }

    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Stops the server and waits for the thread.
    pub fn stop(mut self) {
        self.shut_down();
    }

    fn shut_down(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for McpHost {
    fn drop(&mut self) {
        self.shut_down();
    }
}
