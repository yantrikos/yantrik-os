//! What the person was shown as a granted call's target, held to by the handler that acts on it.
//!
//! An approval card names what a destructive call acts on (`app.name_target`): "Deletes:
//! thesis · ~/tmp/thesis". The grant is bound to the arguments, and the arguments are resolved
//! again when the call runs — against the folder on screen then, the titles the calendar holds
//! then, the process that has that pid then. Between the Allow and the run, a caller could move
//! Files to another folder, rename another event into the name, or let the pid be reused, and the
//! card would have named one thing while another was deleted (security review of #652, H1).
//!
//! So the namer answers an opaque identity beside its rows (`Target::identity`), the shell keeps
//! it on the approval and hands it back when the grant is spent ([`crate::Authority::target`]),
//! and the dispatch installs it here for exactly the one handler call. The handler resolves its
//! arguments to the thing it is about to act on, and asks [`held_to_grant`] with that thing's
//! identity before it acts: a different one, or none, is refused and nothing is done.

use std::cell::RefCell;

use yantrik_ipc_contracts::control_surface::identity_matches;

thread_local! {
    /// The identity the grant spent for the call being dispatched on THIS thread carries.
    static GRANTED_TARGET: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The identity of the target the person allowed, for the call being handled on this thread:
/// `None` outside a dispatch, for a call that spent no grant, and for a card that named nothing.
pub fn granted_target() -> Option<String> {
    GRANTED_TARGET.with(|cell| cell.borrow().clone())
}

/// The one check every handler of a named action makes before it acts: `now` is the identity of
/// what its arguments resolve to at this moment (`None` when they resolve to nothing). Refuses,
/// in one plain sentence, when a grant named one thing and the call now points at another.
pub fn held_to_grant(now: Option<&str>) -> Result<(), String> {
    let granted = granted_target();
    if identity_matches(granted.as_deref(), now) {
        return Ok(());
    }
    Err(format!(
        "the target changed after you allowed it: {} Nothing was done. Ask again, and the card \
         will name what is there now.",
        if now.is_some() {
            "the card named one thing, and this call now points at another."
        } else {
            "what the card named is no longer where the call points."
        }
    ))
}

/// Installs [`granted_target`] for one dispatch and puts back what was there, panic or not.
#[must_use = "the target is forgotten when this guard is dropped"]
pub struct GrantedTargetScope(Option<String>);

impl GrantedTargetScope {
    pub fn enter(target: Option<String>) -> GrantedTargetScope {
        GrantedTargetScope(GRANTED_TARGET.with(|cell| cell.replace(target)))
    }
}

impl Drop for GrantedTargetScope {
    fn drop(&mut self) {
        let previous = self.0.take();
        GRANTED_TARGET.with(|cell| *cell.borrow_mut() = previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handler_is_held_to_the_target_its_grant_named_and_only_inside_the_dispatch() {
        assert!(held_to_grant(Some("anything")).is_ok(), "outside a dispatch nothing is held");
        {
            let _scope = GrantedTargetScope::enter(Some("inode:1".into()));
            assert!(held_to_grant(Some("inode:1")).is_ok());
            let moved = held_to_grant(Some("inode:2")).unwrap_err();
            assert!(moved.starts_with("the target changed after you allowed it:"), "{moved}");
            assert!(moved.contains("Nothing was done"), "{moved}");
            assert!(held_to_grant(None).is_err(), "gone is changed too");
            {
                let _inner = GrantedTargetScope::enter(None);
                assert!(held_to_grant(Some("inode:2")).is_ok(), "an inner call that named nothing");
            }
            assert_eq!(granted_target().as_deref(), Some("inode:1"), "restored");
        }
        assert_eq!(granted_target(), None, "and nothing is left for the next dispatch");
    }
}
