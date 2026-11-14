//! What the shell says back after the person presses a button about preparing (#1184, round 6).
//!
//! A pure function of what was pressed and what the companion answered, so the words are tested
//! directly. It only describes; the answer itself is given by `commitment_consent::person_answer_at`.

use crate::commitment_consent::{Question, Reply, NO, SEEN, TURN_OFF, TURN_ON, WHERE, YES};

/// What the shell shows for a press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The toast: what the press did, or why it did nothing, in a true sentence.
    pub toast: String,
    /// The question to put back on the brief card (after its answered rows are taken off): only when
    /// the press answered nothing and a question is outstanding now.
    pub brief_question: Option<Question>,
}

/// The reply to a press of `pressed`.
pub fn shown(pressed: &str, reply: &Reply) -> Shown {
    let (said, status, _) = reply;
    match said {
        Some(said) => Shown {
            toast: said.clone(),
            brief_question: None,
        },
        None => Shown {
            toast: nothing_happened(pressed),
            brief_question: status.question.clone(),
        },
    }
}

/// Why a press answered nothing, for the button that was pressed.
fn nothing_happened(pressed: &str) -> String {
    if pressed == TURN_ON {
        "It is already on.".into()
    } else if pressed == TURN_OFF {
        "It is already off.".into()
    } else if pressed.starts_with(SEEN) {
        "That card is already gone.".into()
    } else if pressed.starts_with(YES) || pressed.starts_with(NO) {
        format!("That question has changed; the current one is in {WHERE}.")
    } else {
        "Nothing changed.".into()
    }
}
