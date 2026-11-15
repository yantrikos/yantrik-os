//! Actions that are the shell's alone ([`Action::shell_only`]).
//!
//! Some things an app can do are the person's, by pointer, on the desktop's own screen: the
//! meeting brief's rows open the exact message or note a row names. A Mind that could raise them
//! could walk message ids in a mailbox, each opening in front of the person and marked read on
//! the server. So the action is not published, and:
//!
//! * `app.act` refuses it to **every** caller, the shell included, in the words it uses for an
//!   action that does not exist, so a refusal confirms nothing;
//! * `app.press` is the one method that runs it ([`crate::Registry::press_in`]), and it runs
//!   nothing else. Only the shell's own press (`wire/app_act.rs`) sends it. Not `forward_to_app`,
//!   not the companion's `app_action`, not a recipe, not a notification button: judging the
//!   caller by its process was not enough, because the built-in companion is a Mind that runs
//!   *inside* the shell's process and sends no token, and everything that relays an `app.act` for
//!   someone else does so as `yantrik-ui`.
//!
//! The press is judged on the process on the socket, as the kernel's peer credentials and `/proc`
//! say, by the rule notifications-service uses for the desktop: the installed `yantrik-ui`, or the
//! one beside this program in a directory it may trust ([`owner::is_the_installed_program`]), under
//! this process's own account. A program running as the same user can still imitate that (the
//! limit `#154` records for the whole control surface); what this ends is a Mind, `yos`, a copy or
//! a renamed interpreter using the act.

use serde_json::Value;
use yantrik_ipc_contracts::control_surface::View;
use yantrik_ipc_transport::gate::Authority;
use yantrik_ipc_transport::owner;

use crate::context::{agent_token, caller, Caller};
use crate::private::Hold;
use crate::registry::Registry;

/// What every refusal of `app.press` says: the same words for a caller that is not the shell, a
/// press that carries an agent token, an action that is published and an action that is not there.
pub(crate) const PRESS_REFUSED: &str = "there is nothing to press: that is not something this app takes";

/// Whether `who` is the desktop's own shell, read from the kernel now. `false` when nothing could
/// be established: that is the direction to fail in.
pub fn is_the_shell(who: Option<Caller>) -> bool {
    owner::is_the_installed_program(who.map(Into::into), owner::SHELL_BINARY).is_ok()
}

/// Whether the call being dispatched on this thread is the shell's own press, judged by `judge`.
/// A call that carries an agent token is never the shell's.
pub(crate) fn is_a_press_by_the_shell(judge: fn(Option<Caller>) -> bool) -> bool {
    agent_token().is_none() && judge(caller())
}

impl<D, H> Registry<D, H>
where
    D: ?Sized + Fn() -> View,
    H: ?Sized + Fn(&Value) -> Result<Value, String>,
{
    /// `app.press`: run an action declared [`Action::shell_only`], and nothing else, for the
    /// shell's own press. Everything else — a caller that is not the shell as the kernel says
    /// (`set_shell_judge`), a call that carries an agent token, an action that is published, an
    /// action that is not there — is refused in the one sentence [`PRESS_REFUSED`], so
    /// a probe learns nothing. Past that gate it meets what an `act` meets: the state rule, the
    /// arguments, the ceiling and the mode, the guard, the handler.
    ///
    /// No grant is read: a press is the person's pointer on their own screen, and a grant is bound
    /// to a call that asks to be allowed.
    pub fn press_in(
        &self,
        name: &str,
        args: &Value,
        expect_revision: Option<&str>,
        action_id: &str,
        authority: &Authority,
        hold: Hold,
    ) -> Result<Value, String> {
        if !self.is_hidden(name) || !is_a_press_by_the_shell(self.shell_judge) {
            return Err(PRESS_REFUSED.to_string());
        }
        self.dispatch(name, args, expect_revision, action_id, authority, hold)
    }

    /// [`Registry::press_in`] in process, as [`Registry::act`] is `act_in`: nobody is vouched for.
    pub fn press(
        &self,
        name: &str,
        args: &Value,
        expect_revision: Option<&str>,
        action_id: &str,
        authority: &Authority,
    ) -> Result<Value, String> {
        self.press_in(name, args, expect_revision, action_id, authority, Hold::of(self.private_now(), false))
    }
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

    type Ran = std::rc::Rc<std::cell::Cell<u32>>;

    /// An app with one ordinary action and one that is the shell's alone, each counting its runs.
    fn app(judge: fn(Option<Caller>) -> bool) -> (LocalRegistry, Ran, Ran) {
        let (open_ran, exact_ran): (Ran, Ran) = Default::default();
        let (a, b) = (open_ran.clone(), exact_ran.clone());
        let mut reg = LocalRegistry::new("mail");
        reg.set_shell_judge(judge);
        reg.add(
            Action::new("open_message", "Open a message").arg(Param::text("which")),
            Box::new(move |_| {
                a.set(a.get() + 1);
                Ok(json!("ok"))
            }),
        );
        reg.add(
            Action::new("open_message_id", "Open exactly one message").shell_only().arg(Param::text("id")),
            Box::new(move |_| {
                b.set(b.get() + 1);
                Ok(json!("opened"))
            }),
        );
        (reg, open_ran, exact_ran)
    }

    fn open() -> Authority {
        Authority { ceiling: "dangerous".into(), mode: Mode::named("bypass_all"), granted: false, asks_above: None }
    }

    /// Each action's own argument, so a refusal is never for the arguments.
    fn args(name: &str) -> serde_json::Value {
        if name == "open_message" { json!({ "which": "x" }) } else { json!({ "id": "9" }) }
    }

    fn act(reg: &LocalRegistry, name: &str) -> Result<serde_json::Value, String> {
        reg.act(name, &args(name), None, "t", &open())
    }

    fn press(reg: &LocalRegistry, name: &str) -> Result<serde_json::Value, String> {
        reg.press(name, &args(name), None, "t", &open())
    }

    #[test]
    fn the_action_is_not_in_describe() {
        let (reg, ..) = app(the_shell);
        let names: Vec<String> =
            reg.describe()["actions"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["open_message"]);
        let e = act(&reg, "nope").unwrap_err();
        assert!(!e.contains("open_message_id"), "{e}");
    }

    /// The hole round 3 left: the companion runs inside the shell and sends no token, so a judge
    /// that says "shell" for it must still not reach the action through `app.act`.
    #[test]
    fn act_refuses_it_to_everyone_even_when_the_judge_says_shell() {
        let (reg, open_ran, exact_ran) = app(the_shell);
        let _who = CallerScope::enter(from(SHELL_PID));
        let refused = act(&reg, "open_message_id").unwrap_err();
        assert_eq!(exact_ran.get(), 0, "the handler never ran");
        assert!(reg.check_call("open_message_id", &json!({ "id": "9" })).is_err(), "refused before a grant is spent");
        // The words of an action that is not there, so a refusal confirms nothing and the name
        // is only the one the caller typed.
        let absent = act(&reg, "open_message_idx").unwrap_err();
        assert_eq!(refused.replace("open_message_id", "NAME"), absent.replace("open_message_idx", "NAME"));
        assert!(!refused.contains("shell") && !refused.contains("desktop") && !refused.contains("press"), "{refused}");
        assert_eq!(reg.explain("open_message_id", &json!({})).unwrap_err().replace("open_message_id", "NAME"),
                   reg.explain("open_message_idx", &json!({})).unwrap_err().replace("open_message_idx", "NAME"));
        assert!(reg.published_grade("open_message_id").is_none(), "not even a grade is said for it");
        // And an ordinary action is untouched.
        assert!(act(&reg, "open_message").is_ok());
        assert_eq!(open_ran.get(), 1);
    }

    #[test]
    fn a_press_from_the_shell_runs_it() {
        let (reg, _, exact_ran) = app(the_shell);
        let _who = CallerScope::enter(from(SHELL_PID));
        assert!(press(&reg, "open_message_id").is_ok());
        assert_eq!(exact_ran.get(), 1);
    }

    #[test]
    fn a_press_is_refused_to_an_agent_yos_and_an_unknown_caller_and_nothing_runs() {
        let (reg, _, exact_ran) = app(the_shell);
        let mut refusals = Vec::new();
        {
            let _who = CallerScope::enter(from(SHELL_PID));
            let _agent = AgentTokenScope::enter(Some("mind-1".to_string()));
            refusals.push(press(&reg, "open_message_id").unwrap_err());
        }
        {
            let _who = CallerScope::enter(from(77));
            refusals.push(press(&reg, "open_message_id").unwrap_err());
        }
        refusals.push(press(&reg, "open_message_id").unwrap_err());
        assert_eq!(exact_ran.get(), 0);
        assert!(refusals.iter().all(|r| *r == PRESS_REFUSED), "{refusals:?}");
    }

    #[test]
    fn a_press_runs_only_a_shell_only_action_and_says_the_same_of_one_that_is_not_there() {
        let (reg, open_ran, _) = app(the_shell);
        let _who = CallerScope::enter(from(SHELL_PID));
        let published = press(&reg, "open_message").unwrap_err();
        let absent = press(&reg, "no_such_action").unwrap_err();
        assert_eq!(open_ran.get(), 0, "a published action is not pressed");
        assert_eq!(published, PRESS_REFUSED);
        assert_eq!(absent, PRESS_REFUSED);
        assert!(!PRESS_REFUSED.contains("open_message"), "it names nothing");
    }

    #[test]
    fn the_real_judge_does_not_take_this_test_program_for_the_shell() {
        let me = i32::try_from(std::process::id()).unwrap_or(0);
        assert!(!is_the_shell(from(me)), "its program is not yantrik-ui");
        assert!(!is_the_shell(None));
    }

    /// Start `program` with `arg`, trying again while the system says it is busy: the copy was
    /// just written, and a descriptor that is still open for writing in a process another test
    /// thread forked makes the exec fail with ETXTBSY until that process execs (rust-lang/rust#114554).
    fn spawn_when_not_busy(program: &std::path::Path, arg: &str) -> std::process::Child {
        let mut tries = 0;
        loop {
            match std::process::Command::new(program).arg(arg).spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 10 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other.expect("the copy runs"),
            }
        }
    }

    /// The name alone is not the shell: a copy of another program called `yantrik-ui`, in a
    /// directory the desktop does not keep its programs in, is refused.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_renamed_copy_called_yantrik_ui_is_not_the_shell() {
        let dir = std::env::temp_dir().join(format!("yantrik-surface-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sleep = ["/usr/bin/sleep", "/bin/sleep"].into_iter().find(|p| std::path::Path::new(p).exists()).expect("a sleep to copy");
        let copy = dir.join(owner::SHELL_BINARY);
        std::fs::copy(sleep, &copy).unwrap();
        let mut child = spawn_when_not_busy(&copy, "30");

        let who = from(child.id() as i32);
        assert!(!is_the_shell(who), "a copy named yantrik-ui in a temp directory");
        assert!(owner::must_be_program(who.map(Into::into), owner::SHELL_BINARY).is_ok(), "the name alone would have passed");
        // Through the registry as it is built for an app, with no judge of its own.
        let mut reg = LocalRegistry::new("mail");
        reg.add(Action::new("open_message_id", "x").shell_only(), Box::new(|_| Ok(json!("opened"))));
        let _who = CallerScope::enter(who);
        assert_eq!(press(&reg, "open_message_id").unwrap_err(), PRESS_REFUSED);

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
