//! The companion's side of the bond: receipts in, the table and the reasons out, and the
//! person's one act of forgetting. Kept apart so `companion.rs` does not grow for it.

use super::CompanionService;
use crate::bond::{BondRow, BondState, BondTracker, Receipt};

impl CompanionService {
    /// Write down one receipt — a thing the person did — and take the level it gives. Nothing
    /// else changes the bond.
    pub fn record_bond_receipt(&mut self, receipt: &Receipt) {
        if !self.config.bond.enabled || self.incognito {
            return;
        }
        let conn = self.db.conn();
        BondTracker::record_receipt(&conn, receipt);
        let state = BondTracker::get_state(&conn);
        drop(conn);
        self.take_bond(&state);
    }

    /// Forget every receipt. The person's own act: the shell calls it from a UI callback, and
    /// no socket action or tool reaches it. The bond returns to Stranger and the table empties.
    pub fn forget_bond_history(&mut self) -> usize {
        let conn = self.db.conn();
        let dropped = BondTracker::forget_receipts(&conn);
        let state = BondTracker::get_state(&conn);
        drop(conn);
        self.take_bond(&state);
        dropped
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
