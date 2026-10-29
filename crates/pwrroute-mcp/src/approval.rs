//! The in-app approval step. The first time a client name connects, the app
//! shows PwrRoute's approval window and the person picks a role or denies.
//! The answer is remembered per name in [`Agents`]; nothing an agent sends
//! can approve. The window has no time limit: it stays up while any
//! connection under the name is waiting on it, and closes when the last one
//! goes away or the server stops ([`Verdict::Closed`]). That is not a
//! denial: the next connection asks again.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use rmcp::model::Implementation;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    agents::{Agents, clean_name},
    roles::Role,
};

/// Approval windows open at once, one per client name. More are denied.
const MAX_PROMPTS: usize = 8;

/// What the approval window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// Identifies this prompt; [`Approver::cancel`] names it.
    pub id: String,
    /// The `clientInfo.name` the agent sent, which approvals are filed
    /// under. Self-reported: the window should say so.
    pub client_name: String,
    /// The client's display title, when it sent one.
    pub client_title: Option<String>,
    pub client_version: String,
    /// The roles to choose from, least first.
    pub roles: Vec<Role>,
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "decision")]
pub enum ApprovalDecision {
    Approve { role: Role },
    Deny,
}

/// The app's side of approval. The app implements it by showing its
/// window; tests use [`crate::fake::FakeApprover`].
pub trait Approver: Send + Sync + 'static {
    /// Shows the prompt and returns at once. The app answers later, from
    /// any thread, through `reply`. Dropping `reply` unanswered denies.
    fn request(&self, request: ApprovalRequest, reply: ApprovalReply);

    /// Nobody is left waiting on the prompt, or the server stopped: close
    /// its window. A late answer through its reply is ignored.
    fn cancel(&self, id: &str) {
        let _ = id;
    }
}

/// Answers one approval prompt.
#[derive(Debug)]
pub struct ApprovalReply(oneshot::Sender<ApprovalDecision>);

impl ApprovalReply {
    pub fn approve(self, role: Role) {
        self.send(ApprovalDecision::Approve { role });
    }

    pub fn deny(self) {
        self.send(ApprovalDecision::Deny);
    }

    pub fn send(self, decision: ApprovalDecision) {
        // The prompt may have closed; there is no one to tell then.
        let _ = self.0.send(decision);
    }

    /// Whether the prompt still waits for an answer.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        !self.0.is_closed()
    }

    /// A reply and the answer it will carry, for a host's own tests of its
    /// approval window.
    #[must_use]
    pub fn pair() -> (Self, ApprovalAnswer) {
        let (tx, rx) = oneshot::channel();
        (Self(tx), ApprovalAnswer(rx))
    }
}

/// The other end of [`ApprovalReply::pair`].
#[derive(Debug)]
pub struct ApprovalAnswer(oneshot::Receiver<ApprovalDecision>);

impl ApprovalAnswer {
    /// The decision, once the reply was answered.
    pub fn get(&mut self) -> Option<ApprovalDecision> {
        self.0.try_recv().ok()
    }
}

/// How a prompt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The person answered, or the app dropped the reply (its window
    /// closed): a denial.
    Answered(ApprovalDecision),
    /// The window closed unanswered: every connection waiting on it went
    /// away, or the server stopped. Not a denial: a connection that sees
    /// this asks again.
    Closed,
}

/// The current answer for a client name: `None` while the window is up.
pub(crate) type Answer = watch::Receiver<Option<Verdict>>;

/// A connection's hold on the prompt for its name: the answer it waits on,
/// and the claim that keeps the window up. When the last connection
/// holding one goes away, nobody is left to answer for, and the window
/// closes.
#[derive(Clone)]
pub(crate) struct Waiting {
    pub(crate) answer: Answer,
    _held: mpsc::Sender<()>,
}

/// Raises at most one prompt per client name and shares its answer with
/// every connection waiting on it.
pub(crate) struct Approvals {
    approver: Arc<dyn Approver>,
    agents: Agents,
    /// The prompt up for each name: its answer, and a weak handle on the
    /// claims its connections hold.
    pending: Mutex<HashMap<String, (Answer, mpsc::WeakSender<()>)>>,
    next_id: AtomicU64,
    closed: CancellationToken,
}

impl Approvals {
    pub(crate) fn new(approver: Arc<dyn Approver>, agents: Agents) -> Self {
        Self {
            approver,
            agents,
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            closed: CancellationToken::new(),
        }
    }

    /// The answer for `client`, asking the person if no prompt for its name
    /// is up yet. `None` when no prompt can be raised: too many are open,
    /// or the server is stopping. Nobody was asked then, so it is not a
    /// denial.
    pub(crate) fn ask(self: &Arc<Self>, client: &Implementation) -> Option<Waiting> {
        let name = clean_name(&client.name);
        let (tx, rx, held, mut released) = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // A prompt answered a moment ago is not this connection's, nor
            // one whose callers all just left: it is closing.
            if let Some((answer, claims)) = pending.get(&name)
                && answer.borrow().is_none()
                && let Some(held) = claims.upgrade()
            {
                return Some(Waiting {
                    answer: answer.clone(),
                    _held: held,
                });
            }
            pending.remove(&name);
            if pending.len() >= MAX_PROMPTS || self.closed.is_cancelled() {
                return None;
            }
            let (tx, rx) = watch::channel(None);
            // Nothing is ever sent: `recv` returns when every claim is gone.
            let (held, released) = mpsc::channel(1);
            pending.insert(name.clone(), (rx.clone(), held.downgrade()));
            (tx, rx, held, released)
        };

        let id = format!("approval-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let (reply, answer) = oneshot::channel();
        self.approver.request(
            ApprovalRequest {
                id: id.clone(),
                client_name: name.clone(),
                client_title: client.title.clone(),
                client_version: client.version.clone(),
                roles: Role::ALL.to_vec(),
            },
            ApprovalReply(reply),
        );

        let this = Arc::clone(self);
        tokio::spawn(async move {
            let decision = tokio::select! {
                answer = answer => match answer {
                    Ok(decision) => Some(decision),
                    // The app dropped the reply: a denial.
                    Err(_) => Some(ApprovalDecision::Deny),
                },
                // Every connection waiting on it went away.
                _ = released.recv() => None,
                () = this.closed.cancelled() => None,
            };
            let verdict = match decision {
                Some(ApprovalDecision::Approve { role }) => match this.agents.allow(&name, role) {
                    Ok(()) => Verdict::Answered(ApprovalDecision::Approve { role }),
                    // Not recorded means not approved.
                    Err(_) => Verdict::Answered(ApprovalDecision::Deny),
                },
                Some(ApprovalDecision::Deny) => Verdict::Answered(ApprovalDecision::Deny),
                // Nobody is left to answer for: close the window, and let
                // the next connection ask again.
                None => {
                    this.approver.cancel(&id);
                    Verdict::Closed
                }
            };
            // Answer the waiters, then drop the prompt unless a newer one
            // for the name has taken its place.
            let _ = tx.send(Some(verdict));
            let mut pending = this
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if pending
                .get(&name)
                .is_some_and(|(a, _)| a.same_channel(&tx.subscribe()))
            {
                pending.remove(&name);
            }
        });
        Some(Waiting {
            answer: rx,
            _held: held,
        })
    }

    /// Closes every open prompt. The waiters see it close, not a denial.
    pub(crate) fn close(&self) {
        self.closed.cancel();
    }
}
