//! Whose a shell is (#1124, round 5, finding 7).
//!
//! A tab a Mind opens is that Mind's: another Mind's `run`, `send_input` and `describe` must not
//! reach its keys, its screen or its history. The key is a hash of the agent token the shell
//! already verified for the call; the token itself is never stored.

use std::hash::{Hash, Hasher};

/// A Mind, as the shell's token check named it for one call.
pub type Owner = u64;

/// The Mind whose token the call being handled carries, if it carries one.
pub fn of_caller() -> Option<Owner> {
    yantrik_app::control::agent_token().map(|t| of_token(&t))
}

pub fn of_token(token: &str) -> Owner {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}
