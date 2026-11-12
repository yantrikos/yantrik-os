//! What the approvals store lends the bond (#1185), kept beside it so `approvals.rs` does not
//! grow: the test that an offer is held to, a withdrawal that is not an answer, and the id of
//! this boot.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use crate::approvals;

/// The grade and whether the app says the action cannot be undone: whether the card draws "Allow
/// for this session". The bond's offer is held to this same test, so it is never wider than the
/// button that would apply it (`approvals::may_offer_session_rule` is this, with the purpose read).
pub fn session_rule_allowed(grade: &str, cannot_be_undone: bool) -> bool {
    grade != "dangerous" && !cannot_be_undone
}

/// Take a card away because the person's situation changed (Private mode), not because they
/// answered it: the same as a refusal at the gate, and no bond receipt. A real answer is
/// `approvals::deny`, which is the Deny button's.
pub(crate) fn withdraw(id: &str) -> Result<(), String> {
    approvals::locked().deny(id, Instant::now(), &approvals::hhmm())
}

/// A random 128-bit id for one boot, in hex. A card id carries it, so the counter that starts
/// again at 1 on every start cannot raise an id an earlier boot used, whatever the clock says.
/// The std hasher is seeded by the OS for every `RandomState`.
pub(crate) fn fresh_boot() -> String {
    let word = || RandomState::new().build_hasher().finish();
    format!("{:016x}{:016x}", word(), word())
}

/// This boot's id, drawn once.
pub(crate) fn boot() -> &'static str {
    static BOOT: OnceLock<String> = OnceLock::new();
    BOOT.get_or_init(fresh_boot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_boots_do_not_share_an_id_and_one_boot_keeps_its_own() {
        assert_ne!(fresh_boot(), fresh_boot());
        assert_eq!(fresh_boot().len(), 32);
        assert_eq!(boot(), boot());
    }

    #[test]
    fn the_session_button_test() {
        assert!(session_rule_allowed("sensitive", false));
        assert!(!session_rule_allowed("dangerous", false));
        assert!(!session_rule_allowed("sensitive", true));
    }
}
