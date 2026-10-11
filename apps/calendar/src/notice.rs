//! The one line the window shows the person, and the two kinds of thing it can say.
//!
//! A failure — something the person asked for could not be done — is `say`, and draws as the
//! kit's danger notice. A refusal of somebody else's request is `kept`: an agent asked to delete
//! or change an event it did not create, the own-creation rule in `ownership` said no, and the
//! event is untouched. Nothing failed and nothing waits on the person, so it draws as plain
//! information with a way to the event.
//!
//! The refusal used to end "so that needs your OK" on a red card whose only control was a close
//! button: it promised an approval that does not exist — the refusal raises no request, and the
//! only door that asks a person is `delete_event` / `update_event`, which the caller has to use
//! itself — and it coloured as destruction a call that destroyed nothing. Both setters live here
//! so a failure cannot leave the previous refusal's "Open event" behind it.

use slint::SharedString;

use crate::CalendarApp;

/// What a caller was refused: the two verbs `ownership` guards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Delete,
    Change,
}

/// Show what went wrong with something the person asked for (empty clears the line).
pub fn say(ui: &CalendarApp, text: SharedString) {
    ui.set_notice(text);
    ui.set_notice_event(SharedString::new());
}

/// Tell the person a caller was refused event `id`, and offer the way to it.
pub fn kept(ui: &CalendarApp, refused: Refused, title: &str, who: Option<&str>, id: &str) {
    ui.set_notice(kept_line(refused, title, who).into());
    ui.set_notice_event(id.into());
}

/// The sentence for the person: which event, that it is untouched, who asked, the rule in one
/// clause, and what the person can do. Not an id and not an action name — the caller's own
/// answer carries those.
///
/// "On its own" because that is the whole of the rule: a caller can still ask for any event
/// through the door that puts a card in front of the person, and saying "it may only delete
/// events it made" would be false about that door.
pub fn kept_line(refused: Refused, title: &str, who: Option<&str>) -> String {
    let who = who
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .unwrap_or("Something this machine could not identify");
    match refused {
        Refused::Delete => format!(
            "Kept “{title}”. {who} asked to delete it, but on its own it may only delete events \
             it made. Delete it yourself if you want it gone."
        ),
        Refused::Change => format!(
            "Left “{title}” as it was. {who} asked to change it, but on its own it may only \
             change events it made. Change it yourself if you want it different."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_delete_says_the_event_was_kept_and_asks_for_nothing() {
        // What the window showed before: "“Arena min0sa” was not deleted: harness_arena.py
        // asked, and it did not create it, so that needs your OK." -- on a red card with only
        // a close button, so the OK it asked for could not be given.
        let line = kept_line(Refused::Delete, "Arena min0sa", Some("harness_arena.py"));
        assert_eq!(
            line,
            "Kept “Arena min0sa”. harness_arena.py asked to delete it, but on its own it may only \
             delete events it made. Delete it yourself if you want it gone."
        );
        assert!(!line.contains("OK") && !line.contains("approv"), "promises no approval");
        assert!(!line.contains("delete_event") && !line.contains('`'), "no action names for a person");
    }

    #[test]
    fn a_refused_change_says_the_event_was_left_as_it_was() {
        let line = kept_line(Refused::Change, "Lunch with Sam", Some("agent hermes:c1"));
        assert_eq!(
            line,
            "Left “Lunch with Sam” as it was. agent hermes:c1 asked to change it, but on its own \
             it may only change events it made. Change it yourself if you want it different."
        );
        assert!(!line.contains("OK") && !line.contains("update_event"));
    }

    #[test]
    fn a_caller_nobody_could_name_is_not_a_blank_in_the_sentence() {
        for who in [None, Some(""), Some("  ")] {
            let line = kept_line(Refused::Delete, "Dentist", who);
            assert!(
                line.starts_with("Kept “Dentist”. Something this machine could not identify asked"),
                "{line}"
            );
        }
    }
}
