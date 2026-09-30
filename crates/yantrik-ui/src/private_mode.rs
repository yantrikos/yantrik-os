//! Private mode, the shell's half.
//!
//! While it is on, no mind is listening — the Mind, the agents, the built-in companion — and what
//! the person does is not recorded. The rule agents meet at the door and in every surface's
//! dispatch lives in the transport (`yantrik_ipc_transport::privacy`), read from `privacy.json` on
//! every call. This module is the switch, and everything the shell itself does about it
//! ([`enforce`]):
//!
//! - every turn to every mind is refused by the harness host, the built-in companion included, and
//!   the Lens offers to leave Private mode instead of sending;
//! - the agents that run as the person are frozen (`private_freeze`), their commands killed, and
//!   the approval cards waiting for them withdrawn;
//! - the built-in companion goes incognito and does nothing of its own accord (the worker drops
//!   its thinking, recipes and recording; its socket answers no one);
//! - the event log, the clipboard watcher, the activity feed and the screen watcher record
//!   nothing;
//! - `describe shell` says so, to the person (an agent cannot read it while private).
//!
//! Only a person turns it on or off: [`person_set_private`] is called from the mode menu's
//! callbacks and nowhere reachable from the control surface, which `control_approvals`' test
//! holds. It lasts until turned off, across restarts: [`load`] reads it back before anything in
//! the shell starts. And the file is the shell's word: [`watch`] puts it back if anything else
//! changes it, and says so.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use yantrik_ipc_transport::privacy;

use crate::bridge::CompanionBridge;

/// What the Lens link on the desktop's offer carries: pressing it leaves Private mode. A reserved
/// value of the message's `run`, which is otherwise a run id (`mind:main#n`) and never this; a
/// mind's own stream cannot set it (`streaming`).
pub const LEAVE_LINK: &str = "private:leave";

/// What a turn to any mind is refused with while private.
pub const PAUSED: &str = "Private mode is on: no mind is listening, and nothing was sent.";

/// This process's copy of the switch, for the watchers that ask many times a second. The file
/// stays the truth for everyone else.
static ON: AtomicBool = AtomicBool::new(false);

/// Whether the person is in Private mode.
pub fn is_on() -> bool {
    ON.load(Ordering::SeqCst)
}

/// Read the switch back. Called first thing in `main`, before the companion, the clipboard
/// watcher or the harness socket start, so none of them begins while private thinking it is not.
/// A file that cannot be understood reads as on.
pub fn load() -> bool {
    let on = privacy::is_private();
    ON.store(on, Ordering::SeqCst);
    on
}

/// The person turned Private mode on or off. The file first: an agent is refused from the moment
/// it is written, and if it cannot be written the switch does not claim to have moved.
pub fn person_set_private(on: bool) -> std::io::Result<()> {
    privacy::publish(on, now())?;
    ON.store(on, Ordering::SeqCst);
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Make the desktop what the switch says: everything in this module's header, on or off. Called
/// when the person turns it, and at start with what `load` read. The slow parts (systemd) run on
/// a thread of their own.
pub fn enforce(on: bool, bridge: &Arc<CompanionBridge>) {
    // The door itself first: while private the mind account enters no socket in it, whatever
    // build the app serving there runs (an app opened before an update runs the old code).
    set_door(on);
    bridge.set_private(on);
    bridge.event_bus().set_recording(!on);
    if let Some(host) = crate::wire::harness::host() {
        host.pause(on.then(|| PAUSED.to_string()));
    }
    if on {
        // What was waiting for an answer from the person on an agent's behalf is withdrawn: an
        // Allow pressed during Private mode would be an agent acting in it.
        for card in crate::approvals::pending() {
            let _ = crate::approvals::deny(&card.id);
        }
        // The agents' commands, which run as the person, stop.
        crate::control_agent_terminal::shutdown();
    }
    let _ = std::thread::Builder::new().name("private-mode".into()).spawn(move || {
        if on {
            match crate::private_freeze::freeze() {
                Ok(frozen) if !frozen.is_empty() => tracing::warn!(units = ?frozen, "Private mode: agents frozen"),
                Ok(_) => {}
                Err(why) => {
                    tracing::error!(reason = %why, "Private mode: an agent could not be frozen");
                    crate::wire::notifications::private_mode_notice(
                        "An agent is still running",
                        &format!("Private mode is on, but this could not be paused: {why}"),
                    );
                }
            }
        } else {
            crate::private_freeze::thaw();
        }
    });
}

/// Keep the file saying what the switch says. Something running as the person could remove it or
/// write `"private": false`, and every door would open again while the chip still read Private;
/// every few seconds the file is compared with the switch and, if they differ, put back — and the
/// person is told something changed it.
pub fn watch() {
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(2), || {
        let on = is_on();
        // While private, the door is kept closed: an old app restarted, or a socket bound by a
        // build that does not know Private mode, would otherwise be a way in. Never opened from
        // here — only leaving Private mode opens it (`enforce`).
        if on {
            set_door(true);
        }
        if privacy::is_private() == on {
            return;
        }
        tracing::error!(on, "privacy.json was changed by something other than the shell; putting it back");
        match privacy::publish(on, now()) {
            Ok(()) => crate::wire::notifications::private_mode_notice(
                "Something changed Private mode",
                if on {
                    "A program tried to turn Private mode off without you. It is still on, and has been put back."
                } else {
                    "A program changed Private mode's file without you. It has been put back as you left it."
                },
            ),
            Err(e) => tracing::error!(error = %e, "Private mode's file could not be put back"),
        }
    });
    std::mem::forget(timer);
}

/// Close the mind door (private) or open it. A door that could not be closed is said to the person,
/// once per Private mode, as a failed freeze is: the chip would otherwise read Private with a way
/// in still open. Every door's own refusal still stands behind it.
fn set_door(closed: bool) {
    static TOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let dir = yantrik_ipc_transport::mind_door::dir();
    if !dir.exists() {
        return;
    }
    match yantrik_ipc_transport::mind_door::close_door(&dir, closed) {
        Ok(n) if n > 0 => tracing::info!(dir = %dir.display(), closed, sockets = n, "mind door"),
        Ok(_) => {}
        Err(e) => {
            tracing::error!(dir = %dir.display(), closed, error = %e, "the mind door could not be set");
            if closed && !TOLD.swap(true, std::sync::atomic::Ordering::SeqCst) {
                crate::wire::notifications::private_mode_notice(
                    "The mind door is still open",
                    &format!("Private mode is on, and each app still refuses minds, but the door itself could not be closed: {e}"),
                );
            }
        }
    }
    if !closed {
        TOLD.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// What the desktop says in the Lens when the person writes to a mind while private.
pub fn lens_offer(mind: &str) -> String {
    format!(
        "Private mode is on, so {mind} is off: it cannot see or hear anything on this desktop, and \
         your words were not sent. Leave Private mode to talk to it."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_offer_says_nothing_was_sent() {
        let said = lens_offer("Hermes");
        assert!(said.contains("Hermes") && said.contains("not sent"), "{said}");
        assert!(!LEAVE_LINK.contains('#'), "never mistaken for a run id");
    }
}
