//! What the watcher does with each sorted signal, decided without asking anyone (#1257 review,
//! N2, N1 and N7).
//!
//! `signals` sorts what comes off the bus; the watcher (`watch`) asks applications questions and
//! feeds the model. In between is a decision: does this signal need a question at all, and of
//! whom? It is made here, as a pure function of the signal and of what the model already knows,
//! so each rule has a test.
//!
//! # Only connections the model knows are asked
//!
//! Any program can join the bus and send a focus signal for an object that never answers; each
//! such question costs the watcher up to its limit. So nothing is asked of a connection the model
//! has no window of and the last listing did not name as an application. A `window:create` or a
//! root `children-changed:add` from such a connection is a reason to list instead, and listings
//! are rationed already.

use std::time::Duration;

use crate::screen::{Fact, Key, What};
use crate::typing::UNKNOWN_ROLE;
use crate::signals::{is_root, root_change, Raw, RootChange};

/// What the model and the last listing know about a connection.
pub trait Known {
    /// The model holds a window of this connection.
    fn has_window(&self, connection: &str) -> bool;
    /// The last listing named it as an application, with or without windows.
    fn named(&self, connection: &str) -> bool;
}

/// What to do about one signal.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Ask the window itself what it is, and learn it.
    Learn(Key),
    /// The same, and then take it as the front window.
    LearnAndActivate(Key),
    /// Tell the model, without asking anyone.
    Apply(Fact),
    /// The focus moved to this element: ask its role and name.
    Focus(Key),
    /// Something changed inside this element.
    Touch(Key, What),
    /// A window or something in it was renamed; the signal may carry the new name.
    Rename(Key, Option<String>),
    /// The model may be wrong about who is on screen: list again.
    Relist,
    /// A window nobody knows yet took the front: list, and then take it as the front if the
    /// listing found it. Without this a new application's first window never takes the front.
    RelistThenActivate(Key),
    /// The registry has a new owner.
    RegistryRestarted,
    /// Nothing to do.
    Nothing,
}

/// Decide what one signal needs. `registry` is the connection believed about applications.
pub fn plan(raw: Raw, registry: &str, known: &impl Known) -> Step {
    let may_ask = |connection: &str| known.has_window(connection) || known.named(connection);
    match raw {
        Raw::Created(source) if may_ask(&source.0) => Step::Learn(source),
        Raw::Activated(source) if may_ask(&source.0) => Step::LearnAndActivate(source),
        Raw::Created(_) => Step::Relist,
        Raw::Activated(source) => Step::RelistThenActivate(source),
        Raw::Deactivated(source) => Step::Apply(Fact::Deactivated(source)),
        Raw::Destroyed(source) => Step::Apply(Fact::Closed(source)),
        Raw::Focused(source) if may_ask(&source.0) => Step::Focus(source),
        Raw::Focused(_) => Step::Nothing,
        Raw::Children { source, added, child } if is_root(&source.1) => match root_change(&source, added, child, registry) {
            RootChange::Relist => Step::Relist,
            RootChange::Learn(window) if may_ask(&window.0) => Step::Learn(window),
            RootChange::Learn(_) => Step::Relist,
            RootChange::Closed(window) => Step::Apply(Fact::Closed(window)),
            RootChange::Ignore => Step::Nothing,
        },
        Raw::Children { source, .. } => Step::Touch(source, What::Children),
        Raw::Text(source) => Step::Touch(source, What::Text),
        Raw::Name { source, name } => Step::Rename(source, name),
        Raw::Vanished(connection) if known.has_window(&connection) => Step::Relist,
        Raw::Vanished(_) => Step::Nothing,
        Raw::RegistryRestarted => Step::RegistryRestarted,
    }
}

/// What the model is told when the focus moved to `source`, whether or not its role could be
/// learned. A focus whose role could not be learned is told as one with an unknown role, which
/// the model treats as a password: leaving the old focus in place would let a password field be
/// typed in under the name of the field before it (#1257 review, N1).
pub fn focus_fact(source: Key, window: Option<Key>, learned: Option<(String, String)>) -> Fact {
    let (role, name) = learned.unwrap_or_else(|| (UNKNOWN_ROLE.to_string(), String::new()));
    Fact::Focus { source, window, role, name }
}

/// Whether a climb up an element's parents that failed on one hop is that application's fault. A
/// climb that ran out of its shared limit because every hop took a little (24 hops of 17 ms) is
/// not; one hop that took more than half the limit is (#1257 review, N7).
pub fn climb_blames_application(hop_took: Duration, limit: Duration) -> bool {
    hop_took * 2 > limit
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
