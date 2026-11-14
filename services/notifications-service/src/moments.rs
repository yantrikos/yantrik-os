//! The moments the person can switch off: things the OS prepares for them itself, such as the
//! meeting brief. One list of keys in the initiative settings (`Initiative::moments_off`), one
//! row in Settings → Minds → Initiative for each, changed through the same `initiative_set`
//! call as every other setting. No mind is told about them: a read by anyone but the desktop is
//! cut down to what a harness is told (`Initiative::wire`).

use yantrik_ipc_contracts::notifications::notice::Initiative;

/// The longest key a moment may have.
const MAX_KEY: usize = 48;
/// The most moments that may be off at once. The list is a line of a settings file.
const MAX_OFF: usize = 32;

/// Whether `key` is one a moment is named by: `[a-z_]{1,48}`.
fn is_key(key: &str) -> bool {
    (1..=MAX_KEY).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// The initiative settings as `desktop` or any other caller may read them: the desktop reads all
/// of them, and anyone else what a harness is told (`Initiative::wire`), which carries none of the
/// moments the person switched off, and not their quiet hours.
pub fn read_by(settings: Initiative, desktop: bool) -> serde_json::Value {
    if desktop {
        serde_json::to_value(settings).unwrap_or_default()
    } else {
        settings.wire()
    }
}

/// Apply `{ moment, on }` from an `initiative_set` request, if it has both. A key that is not a
/// moment's name, or one more than [`MAX_OFF`] moments off, is refused; turning one back on never is.
pub fn apply(settings: &mut Initiative, params: &serde_json::Value) -> Result<(), String> {
    if let (Some(key), Some(on)) = (params["moment"].as_str(), params["on"].as_bool()) {
        if !is_key(key) {
            return Err(format!("`moment` is a name of lower-case letters and underscores, 1 to {MAX_KEY} of them"));
        }
        if !on && settings.moment_on(key) && settings.moments_off.len() >= MAX_OFF {
            return Err(format!("at most {MAX_OFF} moments can be off"));
        }
        settings.set_moment(key, on);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::notice::MOMENT_MEETING_BRIEF as KEY;

    fn set(settings: &mut Initiative, key: &str, on: bool) {
        apply(settings, &serde_json::json!({ "moment": key, "on": on })).expect("a valid request");
    }

    #[test]
    fn moments_are_on_until_the_person_turns_them_off() {
        let mut settings = Initiative::default();
        assert!(settings.moment_on(KEY), "on by default");
        let older: Initiative = serde_json::from_value(serde_json::json!({ "quiet": true })).unwrap();
        assert!(older.moment_on(KEY), "a settings file from before the field reads as everything on");
        set(&mut settings, KEY, false);
        assert!(!settings.moment_on(KEY));
        assert!(!settings.quiet, "only that switch moved");
        assert_eq!(settings.wire(), Initiative::default().wire(), "no mind is told about it");
    }

    #[test]
    fn turning_one_off_and_on_round_trips_through_the_file_without_repeats() {
        let mut settings = Initiative::default();
        set(&mut settings, KEY, false);
        set(&mut settings, KEY, false);
        let back: Initiative = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(back.moments_off, [KEY], "kept with the other settings, once");
        assert!(!back.moment_on(KEY));
        set(&mut settings, KEY, true);
        assert!(settings.moment_on(KEY) && settings.moments_off.is_empty());
    }

    #[test]
    fn a_key_from_a_later_build_is_kept_and_the_list_stays_sorted() {
        let mut settings: Initiative = serde_json::from_value(serde_json::json!({ "moments_off": ["welcome_back", "ai_down"] })).unwrap();
        set(&mut settings, KEY, false);
        assert_eq!(settings.moments_off, ["ai_down", KEY, "welcome_back"]);
        set(&mut settings, KEY, true);
        assert_eq!(settings.moments_off, ["ai_down", "welcome_back"], "an unknown key is not dropped");
    }

    #[test]
    fn a_request_without_both_words_changes_nothing() {
        let mut settings = Initiative::default();
        apply(&mut settings, &serde_json::json!({ "moment": KEY })).unwrap();
        apply(&mut settings, &serde_json::json!({ "on": false })).unwrap();
        assert!(settings.moments_off.is_empty());
    }

    #[test]
    fn only_the_desktop_reads_the_moments_and_the_quiet_hours() {
        let mut settings = Initiative::default();
        set(&mut settings, KEY, false);
        settings.quiet_hours = Some(yantrik_ipc_contracts::notifications::notice::QuietHours { start: 22, end: 7 });
        let desktop = read_by(settings.clone(), true);
        assert_eq!(desktop["moments_off"], serde_json::json!([KEY]));
        assert_eq!(desktop["quiet_hours"]["start"], 22);
        let mind = read_by(settings.clone(), false);
        assert_eq!(mind, settings.wire(), "what a harness is told, and nothing more");
        assert!(mind.get("moments_off").is_none() && mind.get("quiet_hours").is_none(), "{mind}");
    }

    #[test]
    fn a_key_is_a_short_lower_case_name_and_nothing_else_is_stored() {
        let mut settings = Initiative::default();
        let long = "a".repeat(MAX_KEY + 1);
        for bad in ["", "Meeting_brief", "meeting brief", "meeting.brief", "meeting-brief", "../x", long.as_str()] {
            assert!(apply(&mut settings, &serde_json::json!({ "moment": bad, "on": false })).is_err(), "{bad:?}");
        }
        assert!(settings.moments_off.is_empty(), "none of them was kept");
        set(&mut settings, &"a".repeat(MAX_KEY), false);
        assert_eq!(settings.moments_off.len(), 1, "48 is the longest that fits");
    }

    #[test]
    fn at_most_thirty_two_moments_can_be_off_and_one_can_always_be_turned_on() {
        let mut settings = Initiative::default();
        for i in 0..MAX_OFF {
            set(&mut settings, &format!("moment_{}", "x".repeat(i + 1)), false);
        }
        assert_eq!(settings.moments_off.len(), MAX_OFF);
        assert!(apply(&mut settings, &serde_json::json!({ "moment": "one_more", "on": false })).is_err());
        assert_eq!(settings.moments_off.len(), MAX_OFF);
        let known = settings.moments_off[0].clone();
        set(&mut settings, &known, false);
        assert_eq!(settings.moments_off.len(), MAX_OFF, "one already off is not a new entry");
        set(&mut settings, &known, true);
        assert_eq!(settings.moments_off.len(), MAX_OFF - 1, "and turning one back on is never refused");
    }
}
