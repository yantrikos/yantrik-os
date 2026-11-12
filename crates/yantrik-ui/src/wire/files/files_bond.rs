//! What the Files browser tells the bond about undo (#1185), kept here so `files.rs` does not grow.
//!
//! The person's Undo of something a Mind trashed is a receipt ("You undid something the machine
//! did"); their Undo of what they trashed themselves is not. Whether a Mind trashed what `undo`
//! would put back is therefore kept *with* the ids (`Browser::undo_by_mind`) and changed only
//! where they change: when a trash job finishes with something to undo. A Mind's trash that
//! fails, is cancelled or moves nothing leaves both alone. The receipt is written when the undo
//! job finishes, and only if it put something back.

use super::{Browser, Job};
use crate::App;

/// The undo ids and whether a Mind trashed them, after a job finished. `flags` is `(a Mind
/// trashed these, a Mind's trash was put back)`; the last is `true` when a receipt is due. A job
/// with nothing to undo leaves the current ids and flag alone.
pub(super) fn settled(
    current: (Vec<String>, bool),
    undo: Vec<String>,
    flags: (bool, bool),
) -> ((Vec<String>, bool), bool) {
    let next = if undo.is_empty() {
        current
    } else {
        (undo, flags.0)
    };
    (next, flags.1)
}

impl Browser {
    /// A job finished: take its undo ids, and with them whether a Mind trashed them. `flags` is
    /// `(a Mind trashed these, a Mind's trash was put back)`.
    pub(super) fn settle_undo(&mut self, undo: Vec<String>, flags: (bool, bool)) {
        let ((ids, by_mind), receipt) = settled(
            (std::mem::take(&mut self.undo), self.undo_by_mind),
            undo,
            flags,
        );
        (self.undo, self.undo_by_mind) = (ids, by_mind);
        if receipt {
            crate::bond_receipts::undone("files", "trash");
        }
    }

    /// The person pressed Undo on the last trash. A Mind pressing it undoes nothing of the
    /// person's, so it counts as none.
    pub(super) fn undo_trash(&mut self, ui: &App) {
        let ids = self.undo.clone();
        if !ids.is_empty() {
            let mind = self.undo_by_mind && !crate::shell_view::asked_by_mind();
            self.job(ui, Job::Undo(ids, mind));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::settled;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_mind_trash_that_moved_nothing_leaves_the_persons_undo_alone() {
        let kept = (ids(&["a"]), false);
        assert_eq!(settled(kept.clone(), vec![], (true, false)), (kept, false));
    }

    #[test]
    fn a_trash_that_moved_something_replaces_the_ids_and_says_who() {
        assert_eq!(
            settled((ids(&["a"]), false), ids(&["b"]), (true, false)),
            ((ids(&["b"]), true), false)
        );
        assert_eq!(
            settled((ids(&["b"]), true), ids(&["c"]), (false, false)),
            ((ids(&["c"]), false), false)
        );
    }

    #[test]
    fn a_receipt_is_due_only_when_a_minds_trash_was_put_back() {
        assert!(settled((ids(&["b"]), true), vec![], (false, true)).1);
        assert!(!settled((ids(&["b"]), true), vec![], (false, false)).1);
    }
}
