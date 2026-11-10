//! What the Lens says while the default mind is not yet the one answering (#1166).
//!
//! On an image that ships Hermes (`hermes-bake.sh`) the first question is answered by Hermes and
//! this says nothing. On a machine that has to install it first (an older image, or a bake that
//! was refused), the built-in Companion answers for the minutes that takes, and the conversation
//! then moves to Hermes. The person is told both ends of that, in the conversation, as the
//! desktop speaking (the same `"desktop"` bubble the picker and Private mode use): that the
//! Companion is answering for now, and that Hermes has taken over.
//!
//! The rules are a pure function of what the desktop can see; [`tick`] reads it off the machine
//! and puts the words in the Lens.

use slint::ComponentHandle;

use crate::wire::settings::DEFAULT_MIND;

/// About how long a fresh install takes (4 min 40 s on VM 541, rounded up to a whole minute).
pub const MINUTES: u32 = 5;

/// Said once when the install starts and the Companion is the one answering.
pub fn getting_ready() -> String {
    format!("Hermes is getting ready (about {MINUTES} minutes); the Companion is answering for now.")
}

/// Said once when the conversation has moved to Hermes after the above.
pub const TAKEN_OVER: &str = "Hermes has taken over.";

/// What the desktop can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// The person picked a mind themselves; the default is not theirs to announce.
    pub chose_a_mind: bool,
    /// The default mind passed its three proofs.
    pub proven: bool,
    /// Its install is running.
    pub installing: bool,
    /// The default mind is the one answering.
    pub answering: bool,
}

/// What has been said, so each sentence is said once per wait.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Told {
    waiting: bool,
}

/// What to say now, given what has been said. `None` for nothing.
pub fn next(told: &mut Told, facts: Facts) -> Option<String> {
    if facts.chose_a_mind {
        return None;
    }
    if told.waiting && facts.proven && facts.answering {
        told.waiting = false;
        return Some(TAKEN_OVER.to_string());
    }
    if facts.installing && !facts.proven && !facts.answering && !told.waiting {
        told.waiting = true;
        return Some(getting_ready());
    }
    // An install that ended without Hermes ready: the next try says it again.
    if told.waiting && !facts.installing && !facts.proven {
        told.waiting = false;
    }
    None
}

static TOLD: std::sync::Mutex<Told> = std::sync::Mutex::new(Told { waiting: false });

/// The first run (`onboarding_first_run.slint`), which says how the default mind's install is going;
/// its line is read only while that screen is up.
const FIRST_RUN_SCREEN: i32 = 2;

/// Once per refresh: the first run's line about the default mind, and what [`next`] says, in the
/// Lens.
pub fn tick(ui: &crate::App, host: &yantrik_harness::Host) {
    if ui.get_current_screen() == FIRST_RUN_SCREEN {
        let words: slint::SharedString = crate::default_mind::words_now().into();
        if ui.get_fr_default_mind_words() != words {
            ui.set_fr_default_mind_words(words);
        }
    }
    let facts = Facts {
        chose_a_mind: !crate::wire::settings::chosen_mind().is_empty(),
        proven: crate::default_mind::proven(),
        installing: crate::harness_install::running(DEFAULT_MIND),
        answering: host.active_id() == DEFAULT_MIND,
    };
    let said = next(&mut TOLD.lock().unwrap_or_else(|e| e.into_inner()), facts);
    if let Some(words) = said {
        crate::streaming::say(&ui.as_weak(), None, "desktop", &words);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting() -> Facts {
        Facts { chose_a_mind: false, proven: false, installing: true, answering: false }
    }

    #[test]
    fn the_wait_is_said_once_with_the_minutes_and_the_companion() {
        let mut told = Told::default();
        let said = next(&mut told, waiting()).expect("said when the install starts");
        assert_eq!(said, "Hermes is getting ready (about 5 minutes); the Companion is answering for now.");
        assert_eq!(next(&mut told, waiting()), None, "not at every refresh");
    }

    #[test]
    fn the_switch_is_announced_once_when_hermes_attaches_and_takes_over() {
        let mut told = Told::default();
        next(&mut told, waiting());
        let done = Facts { proven: true, installing: false, answering: true, ..waiting() };
        assert_eq!(next(&mut told, done).as_deref(), Some(TAKEN_OVER));
        assert_eq!(next(&mut told, done), None);
    }

    #[test]
    fn hermes_proven_but_not_yet_answering_is_not_announced() {
        let mut told = Told::default();
        next(&mut told, waiting());
        let proven = Facts { proven: true, installing: false, answering: false, ..waiting() };
        assert_eq!(next(&mut told, proven), None, "the Companion is still the one answering");
    }

    #[test]
    fn an_image_with_hermes_in_it_says_nothing() {
        // Already answering when the shell first looks: no wait, so no notice of one.
        let mut told = Told::default();
        let ready = Facts { proven: true, installing: false, answering: true, ..waiting() };
        assert_eq!(next(&mut told, ready), None);
    }

    #[test]
    fn a_chosen_mind_is_never_announced_over() {
        let mut told = Told::default();
        assert_eq!(next(&mut told, Facts { chose_a_mind: true, ..waiting() }), None);
    }

    #[test]
    fn a_failed_install_that_is_tried_again_says_the_wait_again() {
        let mut told = Told::default();
        next(&mut told, waiting());
        let failed = Facts { installing: false, ..waiting() };
        assert_eq!(next(&mut told, failed), None);
        assert!(next(&mut told, waiting()).is_some());
    }
}
