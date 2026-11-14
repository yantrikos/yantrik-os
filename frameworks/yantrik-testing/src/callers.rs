//! Who a test's call comes from, for the actions only the shell may raise
//! (`Action::shell_only`). A test cannot be the shell program the kernel would vouch for, so the
//! app under test judges the shell by a pid of its own, and a call says whose it is.

use yantrik_surface::Caller;

use crate::Call;

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
