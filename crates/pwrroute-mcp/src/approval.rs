//! The in-app approval step.
//!
//! `/authorize` decides nothing itself. It asks the app, which shows
//! PwrRoute's own approval window, and the browser page only waits: it has
//! no form, script or parameter that can approve anything. This is the
//! PwrSnap and PwrGit door.

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::roles::{Permission, Role};

/// What the approval window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// Identifies this prompt; [`Approver::cancel`] names it.
    pub id: String,
    /// The name the client registered with, or "Local MCP client". Display
    /// only: any local process can claim any name, which is why a person
    /// approves.
    pub client_name: String,
    /// A Session Name not already used by an active session.
    pub suggested_session_name: String,
    /// Where the authorization code will go.
    pub redirect_uri: String,
    /// The scopes the client asked for.
    pub requested: Vec<Permission>,
    /// The roles that fit within `requested`. The person picks one of these.
    pub roles: Vec<Role>,
}

/// The person's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "decision")]
pub enum ApprovalDecision {
    Approve {
        /// 1 to 200 characters once trimmed, unique among active sessions.
        session_name: String,
        /// One of [`ApprovalRequest::roles`].
        role: Role,
    },
    Deny,
}

/// The app's side of approval. The app implements it by showing its
/// window; tests use [`crate::fake::FakeApprover`].
pub trait Approver: Send + Sync + 'static {
    /// Shows the prompt and returns at once. The app answers later, from
    /// any thread, through `reply`. Dropping `reply` unanswered denies.
    fn request(&self, request: ApprovalRequest, reply: ApprovalReply);

    /// The request expired or the listener stopped: close its window. A
    /// late answer through its reply is ignored.
    fn cancel(&self, id: &str) {
        let _ = id;
    }
}

/// Answers one approval prompt.
#[derive(Debug)]
pub struct ApprovalReply(oneshot::Sender<ApprovalDecision>);

impl ApprovalReply {
    pub(crate) fn channel() -> (Self, oneshot::Receiver<ApprovalDecision>) {
        let (tx, rx) = oneshot::channel();
        (Self(tx), rx)
    }

    pub fn approve(self, session_name: impl Into<String>, role: Role) {
        self.send(ApprovalDecision::Approve {
            session_name: session_name.into(),
            role,
        });
    }

    pub fn deny(self) {
        self.send(ApprovalDecision::Deny);
    }

    pub fn send(self, decision: ApprovalDecision) {
        // The request may have expired; there is no one to tell then.
        let _ = self.0.send(decision);
    }

    /// Whether the request still waits for an answer.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        !self.0.is_closed()
    }
}
