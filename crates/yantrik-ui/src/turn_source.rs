//! Who is behind the words a chat turn carries: the person at the Lens, or a program calling the shell.
//!
//! A turn is the person's only when the sender says so (`CompanionCommand::SendMessage::person`),
//! and only the Lens, the chat, voice and the person's own channel say it. The shell's
//! `send_message` action drives the same callback the Lens does, so that the answer streams the
//! same way; that call is a program on the control socket and must not read as someone typing. It
//! says so here, for the length of the callback, and `typed()` reads it where the turn is sent.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::mind_view::{requester_now, Requester};

/// How many turns the person started are being answered by the companion's worker right now. The worker
/// takes one command at a time, so this is 0 or 1; a count, not a flag, so that a nested guard cannot end
/// the outer one.
static PERSONS_TURNS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static FROM_THE_SOCKET: Cell<bool> = const { Cell::new(false) };
}

/// Run `f`, the shell's own callback, for a caller that is not at the keyboard: whatever turn it
/// starts is not the person's.
pub fn from_the_socket<R>(f: impl FnOnce() -> R) -> R {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            FROM_THE_SOCKET.with(|c| c.set(self.0));
        }
    }
    let _reset = Reset(FROM_THE_SOCKET.with(|c| c.replace(true)));
    f()
}

/// Whether the turn being sent from this thread is the person's own words.
pub fn typed() -> bool {
    !FROM_THE_SOCKET.with(Cell::get)
}

/// Whether the control call being dispatched on this thread may put words in the person's chat:
/// only the person's own `yos`. The companion (this process), an agent, a mind and anything that
/// descends from one are refused, and so is any caller the shell cannot place.
pub fn refuse_unless_the_person() -> Result<(), String> {
    match requester_now() {
        Requester::Person => Ok(()),
        Requester::Mind(who) => Err(format!(
            "send_message puts words in the person's chat, and {} is calling: refused, nothing was sent. \
             An agent talks to another agent with new_agent or send_to_agent.",
            who
        )),
    }
}

/// Held by the companion's worker for the length of a turn the person started: its sender said so
/// (`CompanionCommand::SendMessage::person`), which only the Lens, chat, voice and the person's own channel
/// do. The shell reads it from its own handler threads, where the companion's thread-local mark is out of reach.
pub struct PersonsTurn(());

/// The worker begins answering a turn the person started.
pub fn the_persons_turn_begins() -> PersonsTurn {
    PERSONS_TURNS.fetch_add(1, Ordering::SeqCst);
    PersonsTurn(())
}

impl Drop for PersonsTurn {
    fn drop(&mut self) {
        PERSONS_TURNS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether the companion is answering a turn the person started, as the shell itself recorded when it
/// dispatched it. Never anything the companion says about itself.
pub fn a_persons_turn_is_running() -> bool {
    PERSONS_TURNS.load(Ordering::SeqCst) > 0
}

/// Refused when the caller is the companion (this process) and no turn the person started is being answered:
/// with nobody there it must not put words in another mind's conversation, start an agent or start or steer a
/// recipe. In the person's own turn ("ask Hermes to do X" at the Lens) it may, as the companion's own gate
/// allows (`person_started`); this is the shell's second line, from what the shell recorded. An agent, which
/// has a token and a place in the Agents screen, is not the companion and is held by its own rules.
pub fn refuse_the_companion(who: &Requester) -> Result<(), String> {
    refuse_the_companion_when(who, a_persons_turn_is_running())
}

fn refuse_the_companion_when(who: &Requester, persons_turn: bool) -> Result<(), String> {
    match who {
        Requester::Mind(name) if name == "the companion" && !persons_turn => Err(
            "the companion hands no work to another agent and starts or steers no recipe with nobody there: it is \
             the person who does that, from the Agents screen or `yos`, or by asking in their own turn. Nothing was sent."
                .to_string(),
        ),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_is_typed_unless_a_program_on_the_socket_is_driving_the_callback() {
        assert!(typed());
        from_the_socket(|| {
            assert!(!typed());
            from_the_socket(|| assert!(!typed()));
            assert!(!typed(), "the inner scope does not end the outer one");
        });
        assert!(typed(), "the mark ends with the call");
    }

    #[test]
    fn the_mark_ends_when_the_callback_panics() {
        let _ = std::panic::catch_unwind(|| from_the_socket(|| panic!("a handler failed")));
        assert!(typed());
    }

    #[test]
    fn the_shell_refuses_the_companion_unless_the_persons_turn_is_running_and_never_an_agent_or_the_person() {
        let companion = Requester::Mind("the companion".into());
        let said = refuse_the_companion_when(&companion, false).unwrap_err();
        assert!(said.contains("Nothing was sent"), "{said}");
        assert!(refuse_the_companion_when(&companion, true).is_ok(), "in the person's own turn");
        assert!(refuse_the_companion_when(&Requester::Person, false).is_ok());
        assert!(refuse_the_companion_when(&Requester::Mind("an agent".into()), false).is_ok());
    }

    #[test]
    fn the_persons_turn_is_recorded_for_its_length_only_and_by_the_shell() {
        assert!(!a_persons_turn_is_running());
        let outer = the_persons_turn_begins();
        assert!(a_persons_turn_is_running());
        {
            let _inner = the_persons_turn_begins();
        }
        assert!(a_persons_turn_is_running(), "the inner scope does not end the outer one");
        drop(outer);
        assert!(!a_persons_turn_is_running());
        // The record is set where the shell dispatches a turn from `person`, which only the person's surfaces say.
        let bridge = include_str!("bridge.rs");
        assert!(bridge.contains("the_persons_turn_begins()"), "the worker records the turn from the sender's `person`");
        assert!(bridge.contains("let _person_turn = (!is_system_generated)"), "and only for a turn that is not system generated");
    }

    #[test]
    fn every_hand_off_goes_through_the_companion_check_and_send_to_agent_is_sensitive() {
        let production = include_str!("control_agents.rs").split("#[cfg(test)]").next().unwrap();
        assert!(production.contains("turn_source::refuse_the_companion(&crate::mind_view::requester_now())?;\n    let caller = caller()?;"), "delegating_caller asks the shell who is calling");
        assert_eq!(production.matches("delegating_caller()?").count(), 3, "new_agent, send_to_agent, hand_off");
        let send = production.split("\"send_to_agent\",").nth(1).unwrap().split("Action::new(").next().unwrap();
        assert!(send.contains(".risk(\"sensitive\")"), "send_to_agent is graded sensitive");
    }
}
