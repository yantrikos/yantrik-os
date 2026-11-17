//! The four questions the companion asks before it changes something (#1311), in the shell's
//! own words, beside `send_mail` ([`crate::control_mail`]) and built the same way: each is a
//! name, a grade, a sentence and its parameters, and the handler refuses, always.
//!
//! - `delete_for_good`, **sensitive**, "cannot be undone": deleting mail, deleting a calendar
//!   event, overwriting or editing a file, forgetting a whole topic.
//! - `send_message`, **sensitive**, "cannot be undone": a chat message leaving the machine.
//! - `plan_work`, **sensitive**: creating or changing a recipe, a schedule or an automation,
//!   queueing a task, starting a recipe.
//! - `change_item`, **standard**: moving or archiving mail, changing a calendar event, forgetting
//!   one memory.
//!
//! Nothing here decides which mode shows a card for which grade: that is `gate::decide` and
//! `mind_mode`, unchanged, reading the grade and the sentence published here. The shell does not
//! change anything: the companion asks with `request_approval`, waits for the person, spends the
//! grant with `consume_approval`, and only then changes the thing itself, in its own tool, right
//! before the change. One approvals module, one card, one place a person says yes.
//!
//! The parameters are one vocabulary for all four (`yantrik_companion_tools::asks::fields`), so
//! the card can name the thing in the person's terms whichever question it is: the companion
//! fills the fields it has. A request naming any other field is refused by the dispatch.

use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

/// One question as published.
pub struct Published {
    pub name: &'static str,
    pub grade: &'static str,
    pub sentence: &'static str,
}

/// The tail every sentence carries: what the action does itself, which is nothing.
macro_rules! asks_only {
    ($lead:literal) => {
        concat!(
            $lead,
            " This action changes nothing itself: the mind asks it to put a card in front of the person, \
             and acts only after their Allow."
        )
    };
}

/// The four, in the order a card lists them. "Cannot be undone" is the phrase
/// `gate::unrecoverable` reads: it is what makes every mode but full bypass raise a card for the
/// first two, and plan mode refuse.
pub const QUESTIONS: [Published; 4] = [
    Published {
        name: "delete_for_good",
        grade: "sensitive",
        sentence: asks_only!(
            "Delete or overwrite something of the person's for good: a mail, a calendar event, what a file holds, or every \
             memory on a topic. Once done it cannot be undone."
        ),
    },
    Published {
        name: "send_message",
        grade: "sensitive",
        sentence: asks_only!(
            "Send a chat message from the person's WhatsApp or Telegram to someone outside this machine. Once sent it \
             cannot be undone."
        ),
    },
    Published {
        name: "plan_work",
        grade: "sensitive",
        sentence: asks_only!(
            "Plan work for the companion to do later or on its own: create or change a recipe, a schedule or an \
             automation, queue a task, or start a recipe."
        ),
    },
    Published {
        name: "change_item",
        grade: "standard",
        sentence: asks_only!(
            "Change one thing of the person's: move or archive a mail, change a calendar event, or forget one memory."
        ),
    },
];

/// What the handler says: always a refusal. The sentence names the question so a caller that
/// mistook it for the change itself knows what to do instead.
pub fn refusal(name: &str) -> String {
    format!(
        "{name} is a question and not a way to change anything: the mind asks it, waits for the person's Allow, and \
         makes the change itself. Nothing was changed."
    )
}

/// One question as an action: its grade, its sentence and the whole vocabulary of fields, all optional.
fn publish(question: &'static Published) -> Action {
    yantrik_companion::tools::asks::fields().iter().fold(
        Action::new(question.name, question.sentence).risk(question.grade),
        |action, (field, meaning)| action.arg(Param::text(field).describe(meaning).optional()),
    )
}

/// Put the four on the surface, each through the one helper.
pub fn actions(surface: ControlSurface) -> ControlSurface {
    QUESTIONS.iter().fold(surface, |surface, question| {
        let name = question.name;
        surface.action(publish(question), move |_| Err(refusal(name)))
    })
}

#[cfg(test)]
#[path = "questions_tests.rs"]
mod tests;
