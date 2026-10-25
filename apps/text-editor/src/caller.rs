//! Who is asking: the person at the window, or an agent through the control surface.
//!
//! Read once, inside the dispatch, where the caller and its agent token are in scope, and carried
//! with the command; `describe` reads it for its own call. Nothing here reads a request's
//! arguments: a caller cannot say who it is, only be recognised.

use yantrik_app::control;

use crate::owner::Opener;

/// The caller of one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// An agent's call, by the dispatch's word: the mind's account, or an agent token.
    pub agent: bool,
    /// Who a tab this call opens belongs to.
    pub opener: Opener,
}

impl Caller {
    /// The call being dispatched on this thread. Outside a dispatch it is the person.
    pub fn now() -> Caller {
        let agent = control::agent_is_calling();
        Caller { agent, opener: Opener::of(agent, control::agent_token()) }
    }

    /// The person at the window: buttons, keys, a launch.
    pub fn person() -> Caller {
        Caller { agent: false, opener: Opener::Person }
    }
}
