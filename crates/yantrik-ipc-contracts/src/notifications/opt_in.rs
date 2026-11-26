//! A setting that is off until the person turns it on, kept in the initiative settings (#1416,
//! slice 5: "Prepare for me: research and first drafts").
//!
//! The other moments are on until the person turns them off, and the settings store keeps the
//! off ones (`Initiative::moments_off`). This one has the opposite default, and needs no new field
//! in a file that is already long: its key is in `moments_off` only while the person has
//! **opted in**. A settings file from before it existed has no such key, so it reads as off. The
//! service's `initiative_set` flips the polarity for this key (`services/notifications-service`,
//! `moments::apply`), so the desktop sends `{ moment, on }` as it does for every other switch.

use super::notice::Initiative;

/// Settings → Initiative → "Prepare for me: research and first drafts".
pub const MOMENT_PREPARE_FOR_ME: &str = "prepare_for_me";

/// Whether `key` is off until the person turns it on.
pub fn is_opt_in(key: &str) -> bool {
    key == MOMENT_PREPARE_FOR_ME
}

/// Whether the person turned "Prepare for me" on. Off unless they did.
pub fn prepare_for_me_on(s: &Initiative) -> bool {
    s.moments_off.iter().any(|k| k == MOMENT_PREPARE_FOR_ME)
}

/// Turn "Prepare for me" on or off.
pub fn set_prepare_for_me(s: &mut Initiative, on: bool) {
    // `set_moment(key, false)` records the key, which for an opt-in key means "on".
    s.set_moment(MOMENT_PREPARE_FOR_ME, !on);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_until_turned_on_and_a_file_from_before_reads_as_off() {
        let mut s = Initiative::default();
        assert!(!prepare_for_me_on(&s));
        let old: Initiative = serde_json::from_value(serde_json::json!({ "quiet": true })).unwrap();
        assert!(!prepare_for_me_on(&old));
        set_prepare_for_me(&mut s, true);
        assert!(prepare_for_me_on(&s));
        set_prepare_for_me(&mut s, false);
        assert!(!prepare_for_me_on(&s) && s.moments_off.is_empty());
    }

    #[test]
    fn it_is_the_only_opt_in_key() {
        assert!(is_opt_in("prepare_for_me") && !is_opt_in("brain") && !is_opt_in("prepare"));
    }
}
