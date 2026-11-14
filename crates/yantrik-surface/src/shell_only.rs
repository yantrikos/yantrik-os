//! Actions that are the shell's alone ([`Action::shell_only`]).
//!
//! Some things an app can do are the person's, by pointer, on the desktop's own screen: the
//! meeting brief's rows open the exact message or note a row names. A Mind that could raise them
//! could walk message ids in a mailbox, each opening in front of the person and marked read on
//! the server. So the action is not published, and the dispatch refuses it to everyone but the
//! shell: not a Mind, not `yos` (the person's own command line is `agent: false` and still is not
//! the shell), not a program that merely connects.
//!
//! The shell is judged as notifications-service judges the desktop for its own calls: the
//! program on the socket, as the kernel's peer credentials and `/proc` say, under this
//! process's own account. A program running as the same user can imitate that, the limit `#154`
//! records for the whole control surface; what this ends is a Mind or `yos` using the act.

use yantrik_ipc_transport::owner;

use crate::context::{agent_token, caller, Caller};

/// Whether `who` is the desktop's own shell, read from the kernel now. `false` when nothing could
/// be established: that is the direction to fail in.
pub fn is_the_shell(who: Option<Caller>) -> bool {
    owner::must_be_program(who.map(Into::into), owner::SHELL_BINARY).is_ok()
}

/// `Ok` when the call being dispatched on this thread is the shell's, judged by `judge`; else the
/// refusal. A call that carries an agent token is never the shell's.
pub(crate) fn require(action: &str, judge: fn(Option<Caller>) -> bool) -> Result<(), String> {
    if agent_token().is_none() && judge(caller()) {
        return Ok(());
    }
    Err(format!("`{action}` is the desktop's own: the person opens it by pointer on their screen, and it is not offered to anything else"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yantrik_ipc_contracts::control_surface::{Action, Param};
    use yantrik_ipc_transport::gate::Mode;

    use super::*;
    use crate::context::{AgentTokenScope, CallerScope};
    use crate::{Authority, LocalRegistry};

    const SHELL_PID: i32 = 4242;

    fn the_shell(who: Option<Caller>) -> bool {
        who.is_some_and(|c| c.pid == SHELL_PID)
    }

    fn from(pid: i32) -> Option<Caller> {
        Some(Caller { pid, uid: owner::own_euid(), gid: 0, the_mind: false })
    }

    /// An app with one ordinary action and one that is the shell's alone, which runs `ran`.
    fn app(ran: std::rc::Rc<std::cell::Cell<u32>>) -> LocalRegistry {
        let mut reg = LocalRegistry::new("mail");
        reg.set_shell_judge(the_shell);
        reg.add(Action::new("open_message", "Open a message").arg(Param::text("which")), Box::new(|_| Ok(json!("ok"))));
        reg.add(
            Action::new("open_message_id", "Open exactly one message").shell_only().arg(Param::text("id")),
            Box::new(move |_| {
                ran.set(ran.get() + 1);
                Ok(json!("opened"))
            }),
        );
        reg
    }

    fn act(reg: &LocalRegistry) -> Result<serde_json::Value, String> {
        let open = Authority { ceiling: "dangerous".into(), mode: Mode::named("bypass_all"), granted: false, asks_above: None };
        reg.act("open_message_id", &json!({ "id": "9" }), None, "t", &open)
    }

    #[test]
    fn the_action_is_not_in_describe() {
        let reg = app(Default::default());
        let names: Vec<String> =
            reg.describe()["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["open_message"]);
        let e = reg.act("nope", &json!({}), None, "t", &Authority { ceiling: "dangerous".into(), mode: Mode::named("bypass_all"), granted: false, asks_above: None }).unwrap_err();
        assert!(!e.contains("open_message_id"), "{e}");
    }

    #[test]
    fn a_call_from_the_shell_runs_it() {
        let ran = std::rc::Rc::new(std::cell::Cell::new(0));
        let reg = app(ran.clone());
        let _who = CallerScope::enter(from(SHELL_PID));
        assert!(act(&reg).is_ok());
        assert_eq!(ran.get(), 1);
    }

    #[test]
    fn an_agent_a_yos_call_and_an_unknown_caller_are_refused_and_nothing_runs() {
        let ran = std::rc::Rc::new(std::cell::Cell::new(0));
        let reg = app(ran.clone());
        {
            let _who = CallerScope::enter(from(SHELL_PID));
            let _agent = AgentTokenScope::enter(Some("mind-1".to_string()));
            assert!(act(&reg).unwrap_err().contains("the desktop's own"), "an agent's call");
            assert!(reg.check_call("open_message_id", &json!({ "id": "9" })).is_err(), "refused before a grant is spent");
        }
        {
            let _who = CallerScope::enter(from(77));
            assert!(act(&reg).unwrap_err().contains("the desktop's own"), "`yos act` is the person's and is not the shell");
        }
        assert!(act(&reg).is_err(), "nobody established");
        assert_eq!(ran.get(), 0);
    }

    #[test]
    fn the_real_judge_does_not_take_this_test_program_for_the_shell() {
        let me = i32::try_from(std::process::id()).unwrap_or(0);
        assert!(!is_the_shell(from(me)), "its program is not yantrik-ui");
        assert!(!is_the_shell(None));
    }
}
