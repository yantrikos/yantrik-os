//! Sentences that look like a promise and are not the person's (#1184).
//!
//! A promise is prepared for: files are opened and a draft is made. So a sentence that only
//! sounds like one must not become one: speech that is quoted or reported ("Priya wrote: I'm
//! sending you the contract"), a promise that depends on something ("If Maya asks, I'll ..."), a
//! sarcastic one ("Oh sure, ... right after pigs fly"), and a weekday that belongs to something
//! already done ("the report I finished on Monday") rather than to the promise.

use std::sync::OnceLock;

use regex::Regex;

fn re_reported() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)["\u{201c}\u{201d}]|\b(?:said|says|wrote|writes|replied|texted|messaged|quoted?|according\s+to)\b"#).expect("reported pattern")
    })
}

fn re_conditional() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:if|unless|in\s+case|assuming|provided\s+that|depending\s+on|whenever|when\s+\w+\s+(?:asks?|says?|wants?))\b").expect("conditional pattern")
    })
}

fn re_sarcasm() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:oh\s+sure|yeah,?\s+right|pigs\s+fly|hell\s+freezes|as\s+if|in\s+your\s+dreams|not\s+gonna\s+happen|sure\s+thing,?\s+boss|ha\s*ha+|lol|sarcasm)\b|/s\s*$",
        )
        .expect("sarcasm pattern")
    })
}

/// A finished action: a subject and a past-tense verb ("I finished", "she sent", "we wrote").
fn re_past_clause() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:I|we|he|she|they|that|which|who)\s+(?:\w+ed|wrote|sent|made|had|got|saw|did|went|took|gave|found|met|heard|ran|came|began|built|held)\b",
        )
        .expect("past clause pattern")
    })
}

/// Whether the sentence is not a promise of the person's own to act on.
pub fn is_not_a_promise(sentence: &str) -> bool {
    re_reported().is_match(sentence) || re_conditional().is_match(sentence) || re_sarcasm().is_match(sentence)
}

/// Whether `before`, the words ahead of a weekday, already hold a finished action: the weekday
/// then says when that happened, not when the promise falls due.
pub fn weekday_belongs_to_a_past_action(before: &str) -> bool {
    re_past_clause().is_match(before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_and_reported_speech_is_not_the_persons_promise() {
        assert!(is_not_a_promise("Priya wrote: I'm sending you the contract on Friday"));
        assert!(is_not_a_promise("He said \"I'll send the deck on Thursday\""));
        assert!(is_not_a_promise("According to Sam I'll send the report tomorrow"));
    }

    #[test]
    fn a_promise_that_depends_on_something_is_not_one_yet() {
        assert!(is_not_a_promise("If Maya asks, I'll send her the Q4 deck on Thursday"));
        assert!(is_not_a_promise("I'll send the Q4 deck on Thursday unless Maya says otherwise"));
        assert!(is_not_a_promise("When Maya asks I'll send the deck"));
    }

    #[test]
    fn sarcasm_is_not_a_promise() {
        assert!(is_not_a_promise("Oh sure, I'll send Maya the Q4 deck on Thursday, right after pigs fly"));
        assert!(is_not_a_promise("Yeah right, I'll email Bob the report /s"));
    }

    #[test]
    fn a_plain_promise_passes() {
        assert!(!is_not_a_promise("I'm sending Maya the Q4 deck on Thursday"));
        assert!(!is_not_a_promise("I'll send the budget spreadsheet to Sam by Friday"));
    }

    #[test]
    fn a_weekday_after_a_finished_action_is_not_a_deadline() {
        assert!(weekday_belongs_to_a_past_action("I'll tell Sam about the report I finished "));
        assert!(!weekday_belongs_to_a_past_action("I'm sending Maya the Q4 deck "));
    }
}
