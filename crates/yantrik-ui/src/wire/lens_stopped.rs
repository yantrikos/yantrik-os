//! The Lens's own line under a turn that ended without an answer.
//!
//! On the test VM Pi's browser call was refused, it tried the desktop four times and each failed,
//! and the turn ended: the Lens drew the step rows and then nothing, so nothing told the person
//! the mind had stopped, why, or what to do. When a finished turn has no prose of the mind's, the
//! Lens adds one line of its own, [`Entry::Stopped`], drawn as the OS's line (the live line's
//! style, not the answer's) with a Retry (`lens_retry`). The words are
//! `mind_panel_words::stopped_sentence`. A turn that ended in prose is left as it is, and a turn
//! still running never has the line: this is only asked of a reply that has ended.

use super::lens_steps::StepState;
use super::lens_timeline::{self, Entry};
use crate::mind_panel_words::stopped_sentence;

/// The finished reply at `row`'s timeline, with the stopped line at its end when `content` (the
/// reply's own text, step trace and all) holds no words. Nothing is added to a reply the Lens
/// did not follow: it has no steps to account for.
pub fn fold(row: usize, mind: &str, content: &str) -> Vec<Entry> {
    let mut entries = lens_timeline::fold(row);
    if !entries.is_empty() && !has_prose(content) {
        entries.push(Entry::Stopped(line(mind, &entries)));
    }
    entries
}

/// Whether the reply has words of the mind's once the step trace is left out.
pub fn has_prose(content: &str) -> bool {
    !crate::trail::without_trail(content).trim().is_empty()
}

/// The line for a turn whose steps are `entries`: the mind, and the first failed step.
pub fn line(mind: &str, entries: &[Entry]) -> String {
    let first = entries.iter().find_map(|e| match e {
        Entry::Step { step, .. } if step.state == StepState::Failed => Some(step.text.as_str()),
        _ => None,
    });
    stopped_sentence(mind, first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::lens_steps::Step;

    fn step(text: &str, state: StepState) -> Entry {
        Entry::Step { step: Step { text: text.into(), state, action: true }, detail: None }
    }

    #[test]
    fn a_turn_with_only_failed_steps_names_the_first_failure() {
        let entries = [
            Entry::Thinking { secs: 180, text: String::new() },
            step("Couldn\u{2019}t use web go \u{2014} REFUSED: no card could be raised", StepState::Failed),
            step("Couldn\u{2019}t use the desktop \u{2014} failed (exit 1)", StepState::Failed),
        ];
        assert_eq!(
            line("Pi", &entries),
            "Pi stopped without an answer. First problem: Couldn\u{2019}t use web go \u{2014} REFUSED: no card could be raised"
        );
    }

    #[test]
    fn with_no_failed_step_it_only_says_it_stopped() {
        assert_eq!(line("Pi", &[step("Looked at the desktop", StepState::Done)]), "Pi stopped without an answer.");
    }

    #[test]
    fn a_turn_with_prose_gets_no_line_and_a_trail_alone_is_not_prose() {
        assert!(has_prose("Dallas: sunny, 29\u{b0}C."));
        assert!(!has_prose("  \n"));
        assert!(!has_prose("\u{2699}\u{fe0f} os_act(['app'])\n{\"app\": \"notes\"}\n"));
    }
}
