//! Who is asking, in the one spelling the store records it under.
//!
//! Two halves. [`Caller`] is what the dispatch knows about the call being handled: the agent token
//! it carried and the peer process the kernel saw. It is read inside the dispatch, where those are
//! in scope, and travels with the command. [`Who`] turns it into the identity `ownership` compares:
//! the live one reads the shell's reach file and walks `/proc`, and a test's fake answers from a
//! table. Nothing here reads the request's arguments — a caller cannot say who it is, only be
//! recognised.

use yantrik_app::control;
use yantrik_ipc_transport::{peer_identity, reach};

use crate::ownership;

/// What the dispatch knows about the call being handled. Empty for the window's own buttons.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caller {
    pub agent_token: Option<String>,
    pub pid: Option<i32>,
}

impl Caller {
    /// The call being dispatched on this thread. Outside a dispatch both halves are empty.
    pub fn now() -> Caller {
        Caller {
            agent_token: control::agent_token(),
            pid: control::caller().map(|c| c.pid),
        }
    }
}

/// Turns a caller into the identity the store records.
pub trait Who: Send + Sync {
    fn identify(&self, caller: &Caller) -> Option<String>;
}

/// The machine's account of a caller.
#[derive(Debug, Clone, Copy, Default)]
pub struct Machine;

impl Who for Machine {
    /// The agent an agent token belongs to, where the call carried one and the shell's reach file
    /// knows it; else the program the kernel's peer credentials lead to — `peer_identity`'s walk,
    /// which steps over our own `yos`/`yos-mcp` plumbing to the first thing a person would
    /// recognise (#221). `None` when nothing could be established: a call from the window's own
    /// form, a TCP dev connection, a `/proc` that said nothing.
    ///
    /// Nothing is invented when the machine cannot tell: an event stored with no creator is one
    /// nobody may delete unasked, and a caller with no identity deletes nothing unasked.
    /// `ownership::may_delete_unasked` is the rule those two facts are worth.
    fn identify(&self, caller: &Caller) -> Option<String> {
        if let Some(token) = &caller.agent_token {
            // A token the reach file does not know is no agent — the shell refuses a token whose
            // reach it cannot read before the call gets here at all — so this falls through to
            // the program rather than refusing again.
            if let Ok(Some(reach)) = reach::reach_of(token) {
                return Some(ownership::agent_identity(&reach.agent));
            }
        }
        let pid = caller.pid?;
        let name = peer_identity::resolve(Some(pid)).name();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }
}

/// The transport's one spelling for "nothing could be established", for `describe.you`.
pub const UNIDENTIFIED: &str = peer_identity::UNIDENTIFIED;
