//! Who created an event, and what that lets the creator do to it without asking anybody.
//!
//! The own-creation rule of issue #201, and nothing else. An unattended harness that puts
//! events on the calendar to test against could not take them off again: `delete_event` is
//! graded `sensitive` and its own description says the event is not recoverable, so every
//! delete raised an approval card nobody was there to answer. The door the issue chose is
//! that a requester may delete, at `standard`, the events it created ITSELF — so the store
//! records who created each event, and this module is the whole of what that record is worth.
//!
//! Pure on purpose. Both sides of the comparison are strings the machine established: the
//! creator is what the calendar service kept at creation, taken from the kernel's account of
//! the caller (its peer credentials walked to the program a person would recognise) or from
//! the agent an agent token belongs to; the caller is the same question asked of the request
//! now. Neither side is ever something the request claims — a caller cannot say who it is,
//! only be recognised — and the tests for the rule are a table over strings, with no socket,
//! no `/proc` and no desktop in them.

/// How an agent's identity is spelled in the record: `agent <mind>:<conversation>`, from the
/// reach the shell publishes for its token. The prefix keeps the two kinds of identity apart —
/// a program is named by its binary or the script it was given to run, which never contains a
/// space — and one spelling is written down here so the side that records and the side that
/// compares cannot drift.
pub fn agent_identity(agent: &str) -> String {
    format!("agent {agent}")
}

/// May this caller take this event off the calendar without a person being asked?
///
/// One rule: the event is on record as created by exactly the identity this caller was
/// verified to be. Everything else — somebody else's event, an event created before the
/// record existed, a caller nothing could identify, an event with no creator on file — is
/// `false`, and stays with `delete_event`, which is graded `sensitive` and asks.
///
/// The comparison is exact and both sides must be there. Two blanks are not the same
/// somebody: an event stored with an empty record and a caller the machine could not name
/// must not fall into each other's arms, which is what an `==` on the raw strings would do.
pub fn may_delete_unasked(creator: Option<&str>, caller: Option<&str>) -> bool {
    match (creator, caller) {
        (Some(creator), Some(caller)) => !creator.trim().is_empty() && creator == caller,
        _ => false,
    }
}
