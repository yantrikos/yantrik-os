//! Which first-run pages draw the default mind's line (#1167).

use super::BEFORE_NEXT;

/// "Getting your minds ready" and "Your minds": the pages after the choice.
const AFTER_THE_CHOICE: i32 = 4;

/// What the page at `step` draws of `words` ([`super::words`]). [`BEFORE_NEXT`] is the sentence before a
/// choice that starts the install: after the choice it is no longer so. When a model was picked from a
/// card the install has started and `words` is the install's own; when none was (Not now) or the choice
/// was "Start free", which does not set Hermes up, nothing is downloading and the sentence is not drawn.
pub fn drawn_on(step: i32, words: String) -> String {
    if step >= AFTER_THE_CHOICE && words == BEFORE_NEXT {
        String::new()
    } else {
        words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_before_the_choice_is_not_drawn_after_it() {
        for step in [1, 2, 3] {
            assert_eq!(drawn_on(step, BEFORE_NEXT.into()), BEFORE_NEXT, "step {step}: before the choice it is said");
        }
        for step in [4, 5] {
            assert_eq!(drawn_on(step, BEFORE_NEXT.into()), "", "step {step}: after a free press or Not now nothing is downloading");
            // The install's own words are drawn wherever the install is under way.
            assert_eq!(drawn_on(step, super::super::SETTING_UP.into()), super::super::SETTING_UP);
        }
    }
}
