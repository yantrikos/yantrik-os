//! The name of a reader's memory: the display, and who is reading it (#1256).
//!
//! The reader remembers what each line said, per space, and a read that finds a line
//! remembered is faster than one that does not. So whoever shares a space can learn, from how
//! long a read took, whether the other drew that line. A space is therefore never shared by
//! two callers: it is the display plus the caller, taken on the dispatch thread (the only
//! place the caller is in scope) before the read is handed to a worker.
//!
//! The caller's name goes to the reader only inside this string, as a key. It is cut to a safe
//! set of characters and a bounded length first; a name that had to be changed keeps a hash of
//! the whole of it, so two different names never come out as one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// The longest a caller's part of a space is.
const MOST_NAME: usize = 48;

/// The space for `display` (its own short name) read by `caller` (as `mind_identity` keys it,
/// or "person").
pub fn space(display: &str, caller: &str) -> String {
    format!("{display}/{}", name(caller))
}

fn name(caller: &str) -> String {
    let clean: String = caller.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':') { c } else { '_' }).collect();
    if clean == caller && !clean.is_empty() && clean.len() <= MOST_NAME {
        return clean;
    }
    let mut whole = DefaultHasher::new();
    caller.hash(&mut whole);
    let head: String = clean.chars().take(MOST_NAME - 17).collect();
    format!("{head}~{:016x}", whole.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_is_the_display_and_the_caller() {
        assert_eq!(space("mind_view", "mind:Hermes"), "mind_view/mind:Hermes");
        assert_ne!(space("mind_view", "agent:a1"), space("mind_view", "agent:a2"), "one caller never remembers for another");
        assert_ne!(space("mind_view", "person"), space("desktop", "person"));
    }

    #[test]
    fn a_name_is_cut_to_safe_characters_and_a_bounded_length() {
        let odd = space("desktop", "mind:Evil\"}\n{\"image\": \"/etc/passwd\"");
        assert!(odd.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:/~".contains(c)), "{odd}");
        let long = space("desktop", &"m".repeat(10_000));
        assert!(long.len() <= "desktop/".len() + MOST_NAME, "{}", long.len());
    }

    #[test]
    fn names_that_had_to_be_changed_are_still_different_names() {
        assert_ne!(space("d", "a b"), space("d", "a_b"));
        assert_ne!(space("d", &format!("{}x", "m".repeat(100))), space("d", &format!("{}y", "m".repeat(100))));
        assert_ne!(space("d", ""), space("d", "_"));
    }
}
