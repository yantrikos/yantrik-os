//! The moments the person can switch off: things the OS prepares for them itself, such as the
//! meeting brief. One list of keys in the initiative settings (`Initiative::moments_off`), one
//! row in Settings → Minds → Initiative for each, changed through the same `initiative_set`
//! call as every other setting. No mind is told about them.

use yantrik_ipc_contracts::notifications::notice::Initiative;

/// Apply `{ moment, on }` from an `initiative_set` request, if it has both.
pub fn apply(settings: &mut Initiative, params: &serde_json::Value) {
    if let (Some(key), Some(on)) = (params["moment"].as_str(), params["on"].as_bool()) {
        settings.set_moment(key, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::notice::MOMENT_MEETING_BRIEF as KEY;

    fn set(settings: &mut Initiative, key: &str, on: bool) {
        apply(settings, &serde_json::json!({ "moment": key, "on": on }));
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
        apply(&mut settings, &serde_json::json!({ "moment": KEY }));
        apply(&mut settings, &serde_json::json!({ "on": false }));
        assert!(settings.moments_off.is_empty());
    }
}
