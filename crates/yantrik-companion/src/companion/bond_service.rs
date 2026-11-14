//! The companion's side of the bond: receipts in, the table and the reasons out, and the
//! person's one act of forgetting. Kept apart so `companion.rs` does not grow for it.

use super::CompanionService;
use crate::bond::{BondRow, BondState, BondTracker};

impl CompanionService {
    /// Re-read the level and score from the store. The shell's door (`bond_receipts`) writes the
    /// person's receipts and Forget itself, on its own connection, so the worker only has to look.
    pub fn refresh_bond(&mut self) {
        let conn = self.db.conn();
        // The retention prune lives here, off the press path (it also runs at start, in
        // `ensure_tables`).
        BondTracker::prune_old_receipts(&conn);
        let state = BondTracker::get_state(&conn);
        drop(conn);
        self.take_bond(&state);
    }

    fn take_bond(&mut self, state: &BondState) {
        self.bond_level = state.bond_level;
        self.bond_score = state.bond_score;
    }

    /// The table of what each app was asked and answered.
    pub fn bond_table(&self) -> Vec<BondRow> {
        crate::bond_table::rows(&self.db.conn())
    }

    /// Why the score last fell, newest first.
    pub fn bond_why_it_fell(&self, limit: usize) -> Vec<String> {
        BondTracker::why_it_fell(&self.db.conn(), limit)
    }
}
