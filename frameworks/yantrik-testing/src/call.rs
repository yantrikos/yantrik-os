//! One call of an intent, as a test wants it dispatched, and who it comes from.
//!
//! The actions only the shell may press (`Action::shell_only`, `app.press`) need a caller. A test
//! cannot be the shell program the kernel would vouch for, so the app under test judges the shell
//! by a pid of its own, and a call says whose it is.

use serde_json::Value;
use yantrik_surface::Caller;

/// One call of an intent, as the test wants it dispatched. By default the person's own, in a
/// mode that asks about nothing, with no revision expected.
#[derive(Debug, Clone)]
pub struct Call {
    pub name: String,
    pub args: Value,
    pub agent_token: Option<String>,
    pub(crate) caller: Option<Caller>,
    /// Sent as `app.press` (the shell's own press) and not as `app.act`.
    pub(crate) press: bool,
    pub mode: String,
    pub granted: bool,
    pub expect_revision: Option<String>,
}

impl Call {
    pub fn new(name: &str, args: Value) -> Call {
        Call {
            name: name.into(),
            args,
            agent_token: None,
            caller: None,
            press: false,
            mode: "bypass_all".into(),
            granted: false,
            expect_revision: None,
        }
    }

    /// The call arrives as an agent's, with this token.
    pub fn as_agent(mut self, token: &str) -> Call {
        self.agent_token = Some(token.into());
        self
    }

    /// The machine's mode (`ask`, `auto`, `bypass`, `bypass_all`).
    pub fn in_mode(mut self, mode: &str) -> Call {
        self.mode = mode.into();
        self
    }

    /// A person's Allow was spent for exactly this call.
    pub fn granted(mut self) -> Call {
        self.granted = true;
        self
    }

    /// The caller acted on this revision of the app; a different one is refused as stale.
    pub fn expecting(mut self, revision: &str) -> Call {
        self.expect_revision = Some(revision.into());
        self
    }
}


/// The pid a test's stand-in shell calls from.
const SHELL_PID: i32 = 4242;
/// The pid of `yos` run by the person: a caller of the same account, and not the shell.
const YOS_PID: i32 = 4343;

/// The judge the app under test is given in place of the kernel's peer credentials.
pub(crate) fn is_the_shell(who: Option<Caller>) -> bool {
    who.is_some_and(|c| c.pid == SHELL_PID)
}

fn peer(pid: i32) -> Caller {
    Caller { pid, uid: yantrik_ipc_transport::owner::own_euid(), gid: 0, the_mind: false }
}

impl Call {
    /// The call is sent as `app.press`, the one method that runs an action only the shell may
    /// press, and not as `app.act`, which refuses such an action to everyone.
    pub fn pressed(mut self) -> Call {
        self.press = true;
        self
    }

    /// The call is the desktop's shell's own: the person's pointer on the shell's screen.
    pub fn as_the_shell(mut self) -> Call {
        self.caller = Some(peer(SHELL_PID));
        self
    }

    /// The call is the person's own `yos act`: the same account, not an agent, not the shell.
    pub fn as_yos(mut self) -> Call {
        self.caller = Some(peer(YOS_PID));
        self
    }
}
