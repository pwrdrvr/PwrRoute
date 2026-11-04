use super::*;
use crate::approval::{ApprovalReply, ApprovalRequest, Approver};
use proptest::prelude::*;

#[derive(Default)]
struct Prompts(Mutex<Vec<(String, ApprovalReply)>>);
impl Approver for Prompts {
    fn request(&self, request: ApprovalRequest, reply: ApprovalReply) {
        self.0.lock().unwrap().push((request.id, reply));
    }
    fn cancel(&self, id: &str) {
        self.0.lock().unwrap().retain(|(key, _)| key != id);
    }
}

#[derive(Clone, Debug)]
enum Cmd {
    Call(usize),
    Approve(Role),
    Deny,
    DropReply,
    CloseUnanswered,
    Revoke,
    Forget,
    AllowAgain(Role),
    Reconnect(usize),
}
fn command() -> impl Strategy<Value = Cmd> {
    let role = prop::sample::select(Role::ALL.to_vec());
    prop_oneof![
        5 => (0usize..2).prop_map(Cmd::Call),
        2 => role.clone().prop_map(Cmd::Approve),
        1 => Just(Cmd::Deny), 1 => Just(Cmd::DropReply),
        2 => Just(Cmd::CloseUnanswered),
        1 => Just(Cmd::Revoke), 1 => Just(Cmd::Forget),
        1 => role.prop_map(Cmd::AllowAgain),
        2 => (0usize..2).prop_map(Cmd::Reconnect),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Window {
    Open,
    Approved,
    Denied,
    Closed,
}
#[derive(Default)]
struct Model {
    standing: Option<(Role, bool)>,
    windows: Vec<Window>,
    open: Option<usize>,
    slots: [Option<usize>; 2],
    denied: [bool; 2],
}
impl Model {
    fn call(&mut self, i: usize) -> String {
        match self.standing {
            Some((_, true)) => return "revoked".into(),
            Some((role, false)) => return format!("ok:{role:?}"),
            None => {}
        }
        if self.denied[i] {
            return "access_denied".into();
        }
        if self.slots[i].is_some_and(|w| self.windows[w] == Window::Denied) {
            self.denied[i] = true;
            return "access_denied".into();
        }
        let w = *self.open.get_or_insert_with(|| {
            self.windows.push(Window::Open);
            self.windows.len() - 1
        });
        self.slots[i] = Some(w);
        "approval_pending".into()
    }
    fn answer(&mut self, state: Window, role: Option<Role>) {
        if let Some(w) = self.open.take() {
            self.windows[w] = state;
            if let Some(role) = role {
                self.standing = Some((role, false));
            }
        }
    }
}

async fn settle() {
    // All spawned tasks are immediate channel work; yielding queues this
    // driver behind them. No wall-clock sleeps or runtime auto-advance.
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
}

fn run(cmds: &[Cmd]) -> Result<(), TestCaseError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    rt.block_on(async {
        let prompts = Arc::new(Prompts::default());
        let agents = Agents::open(None).unwrap();
        let mut tools = Tools {
            backends: crate::fake::backends().0,
            agents: agents.clone(),
            audit: Arc::new(crate::audit::NoAudit),
            approvals: Arc::new(Approvals::new(prompts.clone(), agents.clone())),
            approval_wait: Duration::from_secs(1),
        };
        let client = Implementation::new("model-agent", "1");
        let mut conns = [
            tools.for_connection(Connection::new(1, agents.clone())),
            tools.for_connection(Connection::new(2, agents.clone())),
        ];
        let mut next = 3;
        let mut model = Model::default();
        for (step, cmd) in cmds.iter().enumerate() {
            match *cmd {
                Cmd::Call(i) => {
                    let want = model.call(i);
                    let got = match conns[i].authorize(&client, &CancellationToken::new()).await {
                        Ok(role) => format!("ok:{role:?}"),
                        Err(error) => error.split(':').next().unwrap().to_owned(),
                    };
                    prop_assert_eq!(got, want, "step {} {:?}", step, cmd);
                }
                Cmd::Approve(role) => {
                    if let Some((_, reply)) = prompts.0.lock().unwrap().pop() {
                        reply.approve(role);
                    }
                    model.answer(Window::Approved, Some(role));
                }
                Cmd::Deny | Cmd::DropReply => {
                    if let Some((_, reply)) = prompts.0.lock().unwrap().pop() {
                        if matches!(cmd, Cmd::Deny) {
                            reply.deny();
                        } else {
                            drop(reply);
                        }
                    }
                    model.answer(Window::Denied, None);
                }
                Cmd::Revoke => {
                    agents.revoke("model-agent").unwrap();
                    if let Some(s) = &mut model.standing {
                        s.1 = true;
                    }
                }
                Cmd::Forget => {
                    agents.forget("model-agent").unwrap();
                    model.standing = None;
                }
                Cmd::AllowAgain(role) => {
                    if model.standing.is_some() {
                        agents.allow("model-agent", role).unwrap();
                        model.standing = Some((role, false));
                    }
                }
                Cmd::CloseUnanswered => {
                    tools.approvals.close();
                    settle().await;
                    tools.approvals = Arc::new(Approvals::new(prompts.clone(), agents.clone()));
                    for conn in &mut conns {
                        conn.tools.approvals = tools.approvals.clone();
                    }
                    model.answer(Window::Closed, None);
                }
                Cmd::Reconnect(i) => {
                    conns[i] = tools.for_connection(Connection::new(next, agents.clone()));
                    next += 1;
                    model.slots[i] = None;
                    model.denied[i] = false;
                    if let Some(w) = model.open
                        && !model.slots.contains(&Some(w))
                    {
                        model.answer(Window::Closed, None);
                    }
                }
            }
            settle().await;
            prop_assert_eq!(
                prompts.0.lock().unwrap().len(),
                usize::from(model.open.is_some()),
                "step {} {:?}",
                step,
                cmd
            );
        }
        tools.approvals.close();
        settle().await;
        Ok(())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn approval_door_matches_connection_and_standing_model(cmds in prop::collection::vec(command(), 1..32)) {
        run(&cmds)?;
    }
}

#[test]
fn closing_an_unanswered_window_allows_a_new_connection_to_ask() {
    run(&[
        Cmd::Call(0),
        Cmd::Reconnect(0),
        Cmd::Call(0),
        Cmd::Approve(Role::Reader),
        Cmd::Call(0),
    ])
    .unwrap();
}

#[test]
fn denial_belongs_only_to_connections_that_waited_on_its_window() {
    run(&[
        Cmd::Call(0),
        Cmd::Call(1),
        Cmd::Deny,
        Cmd::Call(0),
        Cmd::Call(1),
        Cmd::Reconnect(0),
        Cmd::Call(0),
    ])
    .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_closed_unanswered_prompt_does_not_poison_the_waiting_connection() {
    let prompts = Arc::new(Prompts::default());
    let agents = Agents::open(None).unwrap();
    let approvals = Arc::new(Approvals::new(prompts.clone(), agents.clone()));
    let tools = Tools {
        backends: crate::fake::backends().0,
        agents: agents.clone(),
        audit: Arc::new(crate::audit::NoAudit),
        approvals: approvals.clone(),
        approval_wait: Duration::from_secs(1),
    };
    let mut handler = tools.for_connection(Connection::new(1, agents.clone()));
    let waiting = handler.clone();
    let call = tokio::spawn(async move {
        waiting
            .authorize(
                &Implementation::new("model-agent", "1"),
                &CancellationToken::new(),
            )
            .await
    });
    settle().await;
    assert_eq!(prompts.0.lock().unwrap().len(), 1);
    approvals.close();
    let error = call.await.unwrap().unwrap_err();
    assert!(error.starts_with("approval_closed:"), "{error}");
    assert!(!handler.connection.denied.load(Ordering::Relaxed));
    // Resume serving approvals with the same connection slot: a Closed
    // verdict must be discarded, while a person's Deny must be retained.
    handler.tools.approvals = Arc::new(Approvals::new(prompts.clone(), agents));
    let error = handler
        .authorize(
            &Implementation::new("model-agent", "1"),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.starts_with("approval_pending:"), "{error}");
    assert_eq!(prompts.0.lock().unwrap().len(), 1);
    handler.tools.approvals.close();
    settle().await;
}

#[test]
fn an_unanswered_close_reopens_on_the_next_call() {
    run(&[Cmd::Call(0), Cmd::CloseUnanswered, Cmd::Call(0)]).unwrap();
}
