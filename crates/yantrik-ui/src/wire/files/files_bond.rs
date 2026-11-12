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

impl Browser {
    /// A job finished: take its undo ids, and with them whether a Mind trashed them. `flags` is
    /// `(a Mind trashed these, a Mind's trash was put back)`.
    pub(super) fn settle_undo(&mut self, undo: Vec<String>, flags: (bool, bool)) {
        if !undo.is_empty() {
            self.undo = undo;
            self.undo_by_mind = flags.0;
        }
        if flags.1 {
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
