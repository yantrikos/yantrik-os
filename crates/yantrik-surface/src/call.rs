//! One `app.act` call as it arrived on the socket, before any of it reaches a handler.

use serde_json::Value;
use yantrik_ipc_contracts::control_surface::View;
use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::gate::{agent_token_of, grant_of, Authority, CallingAgent};
use yantrik_ipc_transport::reach::{self, Reach};

use crate::context::{off_the_reactor, Caller, Later};

/// The code of every refusal a surface gives: an unknown action, a wrong argument, a stale
/// revision, `CEILING:`, `GRANT:`, a handler's own `Err`. An application answer, not a transport
/// failure, so it stays out of a client's circuit breaker.
pub const REFUSED: i32 = -32602;
/// The code for a surface that could not answer at all: a window whose UI thread did not answer
/// in time.
pub const UNANSWERED: i32 = -32000;
/// The code for a method this surface does not serve.
pub const NO_SUCH_METHOD: i32 = -32601;

/// A refusal, as the socket carries it.
pub fn refusal(message: String) -> ServiceError {
    ServiceError { code: REFUSED, message }
}

/// Apps bind `app-<id>.sock`, so an app cannot collide with the service of the same name.
///
/// `notes` is already taken by notes-service, which stores notes; `app-notes` is the window a
/// person is looking at. They are different things and must not share a socket.
pub fn service_id_for(app_id: &str) -> String {
    format!("app-{app_id}")
}

/// A name for one dispatch, so anything waiting on its effects can say which one it is waiting on.
///
/// Scoped to the socket and monotonic within a run. Not a UUID: it is read by people in logs and
/// compared by machines within a single session, and `app-notes#7` does both better than
/// thirty-two hex digits would.
pub fn next_action_id(service_id: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("{service_id}#{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The acts a mind-account caller makes without an agent's standing, as (surface, action). Anything
/// not named here needs the token of a live agent, so an action added tomorrow needs it by
/// default, and naming one here is a decision a reader sees.
///
/// One entry. The shell's `memory_validate` is asked by the person's memory server (#447), which
/// runs as the mind account and is not an agent acting on the desktop: it has no conversation, no
/// harness and so no token, and would be refused every time. It changes nothing, and its handler
/// answers the mind account and nobody else, which is the check that stands in for standing here.
/// The pair names the surface too, so another app's action of the same name gains nothing.
pub const STANDING_NOT_NEEDED: &[(&str, &str)] = &[("shell", "memory_validate")];

/// Whether an act on `app_id`'s surface needs a mind-account caller's standing.
pub fn needs_standing(app_id: &str, action: &str) -> bool {
    !STANDING_NOT_NEEDED.contains(&(app_id, action))
}

/// The parts of an `app.act` request, lifted off it in the one order that is safe.
///
/// ```text
/// app.act { action, args?, expect_revision?, grant?, agent_token? }
/// ```
#[derive(Clone, Debug)]
pub struct ActCall {
    pub action: String,
    /// The arguments, with any agent token a caller put among them already taken out.
    pub args: Value,
    /// The token that travelled beside `args`. See [`crate::agent_token`].
    pub agent_token: Option<String>,
    /// The revision the caller decided on, if it said. See the guard in
    /// [`crate::Registry::act`].
    pub expect_revision: Option<String>,
    /// A person's Allow, as the `request_id` the shell answered `request_approval` with.
    pub grant: Option<String>,
}

impl ActCall {
    /// Read an `app.act` request's params. The one refusal here is an empty `action`.
    pub fn parse(params: &Value) -> Result<ActCall, ServiceError> {
        let action = params.get("action").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if action.is_empty() {
            return Err(refusal("act needs a non-empty `action`".into()));
        }
        let mut args = params.get("args").cloned().unwrap_or_else(|| serde_json::json!({}));
        // Lifted off before anything reads `args` — a grant is bound to them — and out of `args`
        // if a caller put it there. See `agent_token`.
        let agent_token = agent_token_of(params, &mut args);
        // Optional, and deliberately so: a caller acting on its own initiative has nothing to
        // compare against, and demanding a revision it never read would only teach it to send
        // back whatever it last saw.
        let expect_revision = params.get("expect_revision").and_then(Value::as_str).map(str::to_string);
        // Optional for the same reason — most calls need none.
        let grant = grant_of(params);
        Ok(ActCall { action, args, agent_token, expect_revision, grant })
    }

    /// The reach the call's agent token carries (`yantrik_ipc_transport::reach`): what the role
    /// the agent was started from may touch. `None` for a call with no token, or a token with no
    /// reach — this rule only ever narrows. IO, read beside the ceiling and the mode; a reach file
    /// that is there and cannot be read refuses the call rather than let it through unheld.
    pub fn reach(&self) -> Result<Option<Reach>, ServiceError> {
        match self.agent_token.as_deref() {
            Some(token) => reach::reach_of(token).map_err(|why| {
                refusal(format!(
                    "REACH: {why}, so no act carrying an agent token runs until it can be. Nothing was run."
                ))
            }),
            None => Ok(None),
        }
    }

    /// While the person is in Private mode (`yantrik_ipc_transport::privacy`), no agent acts: a
    /// call that carries an agent token, or comes from the mind account, is refused before
    /// anything else is looked at. The person's own calls, which carry neither, are unaffected.
    pub fn require_not_private(&self, who: Option<Caller>) -> Result<(), ServiceError> {
        let token = !self.agent_token.as_deref().map(str::trim).unwrap_or_default().is_empty();
        let mind = who.is_some_and(|c| yantrik_ipc_transport::mind_door::is_mind(c.uid));
        if (token || mind) && yantrik_ipc_transport::privacy::is_private() {
            return Err(refusal(yantrik_ipc_transport::privacy::REFUSAL.into()));
        }
        Ok(())
    }

    /// A caller the kernel says is the mind account (#411) acts only as an agent the shell has
    /// attached: with the token its harness was given, and a token the shell knows. Without one it
    /// would act unheld by any reach, as nobody in particular. Everyone else is unaffected.
    ///
    /// `app_id` is the surface's own: the few actions [`STANDING_NOT_NEEDED`] names are let
    /// through on that surface alone.
    pub fn require_standing(&self, app_id: &str, who: Option<Caller>) -> Result<(), ServiceError> {
        if !needs_standing(app_id, &self.action) {
            return Ok(());
        }
        let Some(caller) = who else { return Ok(()) };
        if !yantrik_ipc_transport::mind_door::is_mind(caller.uid) {
            return Ok(());
        }
        let token = self.agent_token.as_deref().map(str::trim).unwrap_or_default();
        if token.is_empty() {
            return Err(refusal(
                "MIND: a mind acts on the desktop only as an attached agent, with the token its harness \
                 was given; this call carried none. Nothing was run."
                    .into(),
            ));
        }
        let pid = u32::try_from(caller.pid).ok().filter(|p| *p > 0);
        match reach::standing_of(token, pid) {
            Ok(true) => Ok(()),
            Ok(false) => Err(refusal(
                "MIND: this agent token is not one the shell has given a live agent. Nothing was run.".into(),
            )),
            Err(why) => Err(refusal(format!("MIND: {why}, so no act from a mind runs until it can be. Nothing was run."))),
        }
    }

    /// Spend this call's grant, if it carries one, for exactly this action and these arguments
    /// as they were sent.
    ///
    /// A grant is a person's Allow for one call. It is spent only once everything that could
    /// still refuse the call without asking anybody has passed, or the Allow is used up on an act
    /// that never runs and the person is asked again for something they already said yes to:
    ///
    /// 1. `checked` — the action exists, the calling agent's reach covers it, and its arguments
    ///    are right (present, known, of the declared type or losslessly converted to it). It
    ///    answers with the grade the surface publishes for the action now.
    /// 2. the ceiling, on that grade (#154) — inside `Authority::spend`.
    /// 3. the spend, against the arguments as sent: what the person saw on the card is what the
    ///    grant is bound to, never the converted form the handler will read — and for the agent
    ///    this call arrived as (`who` is the kernel's account of it), because a grant asked for
    ///    by one agent is not another's to spend (#182).
    ///
    /// `checked` is called only when there is a grant: for a window it is a round trip to the UI
    /// thread, paid only by a call a person has just answered a card for.
    pub fn spend_grant(
        &self,
        authority: &mut Authority,
        app_id: &str,
        who: Option<Caller>,
        checked: impl FnOnce() -> Result<&'static str, ServiceError>,
    ) -> Result<(), ServiceError> {
        let Some(id) = self.grant.as_deref() else { return Ok(()) };
        let grade = checked()?;
        // The token beside `args` and the pid the kernel stamped on the call it arrived in:
        // together they are who the shell resolves the spend against. No token, no claim — the
        // person's own `yos act` spends as no agent, as it always has.
        let caller = self.agent_token.as_ref().map(|token| CallingAgent {
            token: token.clone(),
            pid: who.and_then(|c| u32::try_from(c.pid).ok()).filter(|pid| *pid > 0),
        });
        authority.spend(id, app_id, &self.action, grade, &self.args, caller.as_ref()).map_err(refusal)
    }

    /// The dispatch's own record of the call. The audit log is the shell's job.
    pub fn log(&self, action_id: &str, authority: &Authority, who: Option<Caller>) {
        tracing::info!(
            action = %self.action,
            id = %action_id,
            ceiling = %authority.ceiling,
            mode = %authority.mode.name,
            granted = authority.granted,
            // Logged as a pair so a line in the journal says who as well as what.
            caller_pid = who.map(|c| c.pid).unwrap_or(0),
            caller_uid = who.map(|c| c.uid).unwrap_or(0),
            // Whether one came, never the token itself.
            agent_token = self.agent_token.is_some(),
            "app.act"
        );
    }
}

/// Run the rest of an answer a handler left with [`crate::answer_later`], and put its result in
/// the envelope — with the view read again afterwards (`reread`), so the state beside the result
/// is the state the result came from rather than the state before the wait. A `reread` that
/// cannot answer (a window too busy) leaves the view from when the handler ran: the result stands.
pub fn finish_later(
    mut envelope: Value,
    later: Later,
    reread: impl FnOnce() -> Option<View>,
) -> Result<Value, ServiceError> {
    let result = off_the_reactor(later).map_err(refusal)?;
    envelope["result"] = result;
    if let Some(view) = reread() {
        envelope["revision"] = view.revision().into();
        envelope["summary"] = view.summary.into();
        envelope["state"] = view.state;
    }
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_shells_memory_validate_is_asked_without_standing() {
        assert_eq!(STANDING_NOT_NEEDED, &[("shell", "memory_validate")], "an addition is a decision for a reader");
        assert!(!needs_standing("shell", "memory_validate"));
        // The same name on another surface, and any other shell action, still need it.
        assert!(needs_standing("weather", "memory_validate"));
        assert!(needs_standing("shell", "open_app"));
        assert!(needs_standing("shell", "memory_validate "), "names are exact");
        // Let through before anything asks who is calling: nobody here is the mind account, so
        // this is the exemption itself answering, for any caller.
        let call = ActCall::parse(&json!({"action": "memory_validate"})).unwrap();
        let someone = Some(Caller { pid: 4242, uid: 1000, gid: 1000 });
        assert!(call.require_standing("shell", someone).is_ok());
        assert!(call.require_standing("shell", None).is_ok());
    }

    #[test]
    fn every_dispatch_gets_its_own_name() {
        let first = next_action_id("app-notes");
        let second = next_action_id("app-notes");
        assert_ne!(first, second, "two waits must not key on the same id");
        assert!(first.starts_with("app-notes#"), "{first}");
    }

    #[test]
    fn service_ids_do_not_collide_with_services() {
        // notes-service owns `notes`; the Notes window must not bind the same socket.
        assert_eq!(service_id_for("notes"), "app-notes");
        assert_ne!(service_id_for("notes"), "notes");
    }

    #[test]
    fn a_call_is_read_in_the_one_safe_order() {
        let call = ActCall::parse(&json!({
            "action": "  kill_process ",
            "args": { "pid": 42, "agent_token": "smuggled" },
            "agent_token": " tok-1 ",
            "expect_revision": "0123456789abcdef",
            "grant": " appr-7 ",
        }))
        .unwrap();
        assert_eq!(call.action, "kill_process");
        assert_eq!(call.args, json!({"pid": 42}), "the token is never among the arguments");
        assert_eq!(call.agent_token.as_deref(), Some("tok-1"));
        assert_eq!(call.expect_revision.as_deref(), Some("0123456789abcdef"));
        assert_eq!(call.grant.as_deref(), Some("appr-7"));

        let bare = ActCall::parse(&json!({"action": "go"})).unwrap();
        assert_eq!(bare.args, json!({}), "no args is no arguments");
        assert!(bare.agent_token.is_none() && bare.expect_revision.is_none() && bare.grant.is_none());

        for params in [json!({}), json!({"action": ""}), json!({"action": "  "}), json!({"action": 7})] {
            let err = ActCall::parse(&params).unwrap_err();
            assert_eq!((err.code, err.message.as_str()), (REFUSED, "act needs a non-empty `action`"), "{params}");
        }
    }

    #[test]
    fn a_call_without_a_grant_never_asks_for_the_grade() {
        let call = ActCall::parse(&json!({"action": "go"})).unwrap();
        let mut authority = Authority {
            ceiling: "sensitive".into(),
            mode: yantrik_ipc_transport::gate::Mode::named("ask"),
            granted: false,
            asks_above: None,
        };
        call.spend_grant(&mut authority, "notes", None, || panic!("asked for a grade with no grant to spend"))
            .unwrap();
        assert!(call.reach().unwrap().is_none(), "no token, no reach, no file read");
        assert!(!authority.granted);
    }

    #[test]
    fn work_finished_later_is_the_answer_and_the_view_is_read_after_it() {
        let envelope = json!({"accepted": true, "result": "placeholder", "summary": "before", "state": {}, "revision": "x"});
        let done = finish_later(envelope.clone(), Box::new(|| Ok(json!({"exit": 0}))), || {
            Some(View::new("after").with("n", 1))
        })
        .unwrap();
        assert_eq!(done["result"], json!({"exit": 0}));
        assert_eq!(done["summary"], "after");
        assert_eq!(done["revision"], View::new("after").with("n", 1).revision());

        // A reread that cannot answer leaves the view the handler saw; the result stands.
        let kept = finish_later(envelope.clone(), Box::new(|| Ok(json!(1))), || None).unwrap();
        assert_eq!((kept["result"].clone(), kept["summary"].clone()), (json!(1), json!("before")));

        let refused = finish_later(envelope, Box::new(|| Err("refused after 10 ms".into())), || None).unwrap_err();
        assert_eq!((refused.code, refused.message.as_str()), (REFUSED, "refused after 10 ms"));
    }
}
