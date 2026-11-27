//! A setting that is off until the person turns it on, kept in the initiative settings (#1416,
//! slice 5: "Prepare for me: research and first drafts").
//!
//! The other moments are on until the person turns them off, and the settings store keeps the
//! off ones (`Initiative::moments_off`). This one has the opposite default, and needs no new field
//! in a file that is already long: its key is in `moments_off` only while the person has
//! **opted in**. A settings file from before it existed has no such key, so it reads as off. The
//! service's `initiative_set` flips the polarity for this key (`services/notifications-service`,
//! `moments::apply`), so the desktop sends `{ moment, on }` as it does for every other switch.
//!
//! **Nothing else may read `moments_off` for this key.** Everything about the polarity is in this
//! file ([`prepare_for_me_on`], [`set_prepare_for_me`]) and in `moments::apply`, which routes the
//! key here; `Initiative::moment_on(MOMENT_PREPARE_FOR_ME)` would answer the wrong way round. The
//! test `only_this_file_reads_the_key` scans the workspace for any other use.

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

/// What `describe shell` says for the switch: `"on"`, `"off"`, or `"unknown"` when the store has
/// not answered the shell yet (unknown is never "on").
pub fn switch_word(settings: Option<&Initiative>) -> &'static str {
    match settings {
        None => "unknown",
        Some(s) if prepare_for_me_on(s) => "on",
        Some(_) => "off",
    }
}

/// Why a job may not run now, from the three switches: Private mode on, the Brain switch off, or
/// "Prepare for me" off or not yet known. `None` when it may. Pure, so a test can start from the
/// store's file and reach the same answer the shell gives.
pub fn forbidden_for(private: bool, settings: Option<&Initiative>) -> Option<&'static str> {
    if private {
        return Some("Private mode");
    }
    // The Brain switch is on until turned off; an unknown answer reads as on, as the shell does.
    if settings.is_some_and(|s| !s.moment_on(super::notice::MOMENT_BRAIN)) {
        return Some("the Brain switch");
    }
    if !settings.is_some_and(prepare_for_me_on) {
        return Some("the Prepare for me switch");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_and_the_reasons_follow_the_settings() {
        let mut s = Initiative::default();
        assert_eq!(switch_word(None), "unknown");
        assert_eq!(switch_word(Some(&s)), "off");
        assert_eq!(forbidden_for(false, None), Some("the Prepare for me switch"));
        set_prepare_for_me(&mut s, true);
        assert_eq!(switch_word(Some(&s)), "on");
        assert_eq!(forbidden_for(false, Some(&s)), None);
        assert_eq!(forbidden_for(true, Some(&s)), Some("Private mode"));
        s.set_moment(super::super::notice::MOMENT_BRAIN, false);
        assert_eq!(forbidden_for(false, Some(&s)), Some("the Brain switch"));
    }

    /// The polarity is confined: no other source file names the key.
    #[test]
    fn only_this_file_reads_the_key() {
        fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                if p.is_dir() {
                    if name != "target" && !name.starts_with('.') {
                        scan(&p, hits);
                    }
                } else if name.ends_with(".rs") && name != "opt_in.rs" {
                    let text = std::fs::read_to_string(&p).unwrap_or_default();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    if code.contains("\"prepare_for_me\"") {
                        hits.push(p.display().to_string());
                    }
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
        let services = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../services");
        let mut hits = Vec::new();
        scan(&root, &mut hits);
        scan(&services, &mut hits);
        hits.retain(|h| !h.contains("/prepare/"));
        assert!(hits.is_empty(), "the literal key is named outside opt_in.rs: {hits:?}");
    }

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
