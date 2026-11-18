//! Who is asking: the person at the window, or an agent through the control surface.
//!
//! Read once, inside the dispatch, where the caller and its agent token are in scope, and carried
//! with the command; `describe` reads it for its own call. Nothing here reads a request's
//! arguments: a caller cannot say who it is, only be recognised.

use yantrik_app::control;

/// The caller of one command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caller {
    /// An agent's call, by the dispatch's word: the mind's account, or an agent token.
    pub agent: bool,
}

impl Caller {
    /// The call being dispatched on this thread. Outside a dispatch it is the person, and so is
    /// the rail's Replace: it rides the agent path for the gate's sake (private mode, the ceiling)
    /// but it is the person's press on the person's own document. Agents over the socket are not
    /// helped by it: only the core sets the mark.
    pub fn now() -> Caller {
        Caller { agent: control::agent_is_calling() && !yantrik_app::rail::is_person_applying() }
    }

    /// The person at the window: buttons, keys, a launch.
    pub fn person() -> Caller {
        Caller { agent: false }
    }
}
