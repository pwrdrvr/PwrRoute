//! The in-app approval step. The first time a client name connects, the app
//! shows PwrRoute's approval window and the person picks a role or denies.
//! The answer is remembered per name in [`Agents`]; nothing an agent sends
//! can approve. A window that closes unanswered ([`Verdict::Expired`]) is
//! not a denial: the next call asks again.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use rmcp::model::Implementation;
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    agents::{Agents, clean_name},
    roles::Role,
};

/// How long a prompt stays open, unless [`Config::prompt_ttl`] says
/// otherwise. An expired prompt is not a denial: the next call asks again.
///
/// [`Config::prompt_ttl`]: crate::Config::prompt_ttl
pub const DEFAULT_PROMPT_TTL: Duration = Duration::from_secs(5 * 60);
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
    /// How long the prompt waits for an answer from when it is raised;
    /// then it expires and the window should say so.
    pub expires_in: Duration,
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

    /// The prompt expired or the server stopped: close its window. A late
    /// answer through its reply is ignored.
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
        // The prompt may have expired; there is no one to tell then.
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
    /// Nobody answered before the prompt's TTL ran out, or the server
    /// stopped. Not a denial: a connection that sees this asks again.
    Expired,
}

/// The current answer for a client name: `None` while the window is up.
pub(crate) type Answer = watch::Receiver<Option<Verdict>>;

/// Raises at most one prompt per client name and shares its answer with
/// every connection waiting on it.
pub(crate) struct Approvals {
    approver: Arc<dyn Approver>,
    agents: Agents,
    pending: Mutex<HashMap<String, Answer>>,
    next_id: AtomicU64,
    closed: CancellationToken,
    /// How long a prompt stays open before it expires.
    ttl: Duration,
}

impl Approvals {
    pub(crate) fn new(approver: Arc<dyn Approver>, agents: Agents, ttl: Duration) -> Self {
        Self {
            approver,
            agents,
            pending: Mutex::default(),
            next_id: AtomicU64::new(1),
            closed: CancellationToken::new(),
            ttl,
        }
    }

    /// The answer for `client`, asking the person if no prompt for its name
    /// is up yet. `None` when no prompt can be raised: too many are open,
    /// or the server is stopping. Nobody was asked then, so it is not a
    /// denial.
    pub(crate) fn ask(self: &Arc<Self>, client: &Implementation) -> Option<Answer> {
        let name = clean_name(&client.name);
        let (tx, rx) = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // A prompt answered a moment ago is not this connection's.
            if let Some(answer) = pending.get(&name)
                && answer.borrow().is_none()
            {
                return Some(answer.clone());
            }
            pending.remove(&name);
            if pending.len() >= MAX_PROMPTS || self.closed.is_cancelled() {
                return None;
            }
            let (tx, rx) = watch::channel(None);
            pending.insert(name.clone(), rx.clone());
            (tx, rx)
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
                expires_in: self.ttl,
            },
            ApprovalReply(reply),
        );

        let this = Arc::clone(self);
        let ttl = self.ttl;
        tokio::spawn(async move {
            let decision = tokio::select! {
                answer = tokio::time::timeout(ttl, answer) => match answer {
                    Ok(Ok(decision)) => Some(decision),
                    // The app dropped the reply: a denial.
                    Ok(Err(_)) => Some(ApprovalDecision::Deny),
                    Err(_) => None,
                },
                () = this.closed.cancelled() => None,
            };
            let verdict = match decision {
                Some(ApprovalDecision::Approve { role }) => match this.agents.allow(&name, role) {
                    Ok(()) => Verdict::Answered(ApprovalDecision::Approve { role }),
                    // Not recorded means not approved.
                    Err(_) => Verdict::Answered(ApprovalDecision::Deny),
                },
                Some(ApprovalDecision::Deny) => Verdict::Answered(ApprovalDecision::Deny),
                // Nobody answered: close the window, and let the next call
                // ask again.
                None => {
                    this.approver.cancel(&id);
                    Verdict::Expired
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
                .is_some_and(|a| a.same_channel(&tx.subscribe()))
            {
                pending.remove(&name);
            }
        });
        Some(rx)
    }

    /// Closes every open prompt. The waiters see it expire, not a denial.
    pub(crate) fn close(&self) {
        self.closed.cancel();
    }
}
