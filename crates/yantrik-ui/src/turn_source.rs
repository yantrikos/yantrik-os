//! Who is behind the words a chat turn carries: the person at the Lens, or a program calling the shell.
//!
//! A turn is the person's only when the sender says so (`CompanionCommand::SendMessage::person`),
//! and only the Lens, the chat, voice and the person's own channel say it. The shell's
//! `send_message` action drives the same callback the Lens does, so that the answer streams the
//! same way; that call is a program on the control socket and must not read as someone typing. It
//! says so here, for the length of the callback, and `typed()` reads it where the turn is sent.

use std::cell::Cell;

use crate::mind_view::{requester_now, Requester};

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
}
