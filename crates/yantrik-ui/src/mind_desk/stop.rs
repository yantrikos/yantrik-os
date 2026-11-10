//! A stop asked for through the desk: the person's decision, on the person's desktop.
//!
//! The desk is a window in Mind View, where a Mind can press its buttons. A stop it asked for
//! directly would run as the person (`stop_agent` treats a caller with no agent token as the
//! person, who may stop any agent), so any Mind that could press Stop could end another Mind's turn,
//! kill its commands and withdraw the approvals waiting on the person: a confused deputy. Nothing
//! the desk can press stops anything. It asks (`mind_desk_stop`), and the shell then raises a card
//! on the person's own desktop, outside Mind View, held the way every approval card is held. Only
//! the person's Allow on that card runs `stop_agent`.
//!
//! This module is the bookkeeping and the rules, with every outside thing handed in, so they are
//! tested without a shell: which Mind may be asked about (only the one the desk is showing), one
//! question at a time, a stop run only on an Allow, and what the desk may be told afterwards.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use yantrik_ipc_contracts::mind_desk::StopStatus;

/// How long a "not stopped" is said before it is let go.
pub const NOT_STOPPED_SAID_FOR: Duration = Duration::from_secs(30);

struct Entry {
    status: StopStatus,
    at: Instant,
}

/// The stops asked for, by agent.
#[derive(Default)]
pub struct Stops {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Stops {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Ask the person whether to stop `agent`, if it is the Mind the desk is showing (`shown`) and
    /// no question about it is already on their desktop. `raise` puts the card up and answers its
    /// id; nothing is stopped here, whatever it answers.
    pub fn ask(
        &self,
        agent: &str,
        shown: Option<&str>,
        now: Instant,
        raise: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        if shown != Some(agent) {
            return Err("that is not the Mind the desk is showing, so the desk cannot ask about it.".to_string());
        }
        let mut entries = self.entries();
        if entries.get(agent).is_some_and(|e| e.status == StopStatus::Asked) {
            return Err("the person has already been asked about stopping it, on their desktop.".to_string());
        }
        let id = raise()?;
        entries.insert(agent.to_string(), Entry { status: StopStatus::Asked, at: now });
        Ok(id)
    }

    /// The person's answer came in: `allowed` runs `stop`, and only `allowed` does. What the desk
    /// is then told is the shell's own: stopped only if `stop` said it was.
    pub fn settle(&self, agent: &str, allowed: bool, now: Instant, stop: impl FnOnce() -> Result<(), String>) -> StopStatus {
        let status = match allowed && stop().is_ok() {
            true => StopStatus::Stopped,
            false => StopStatus::NotStopped,
        };
        self.entries().insert(agent.to_string(), Entry { status, at: now });
        status
    }

    /// What the desk is told of `agent`'s stop. A stop that worked is said while the Mind is not
    /// at work again; a "not stopped" is said for a while; a question still open is said until it
    /// is answered.
    pub fn status_of(&self, agent: &str, working: bool, now: Instant) -> Option<StopStatus> {
        let mut entries = self.entries();
        let entry = entries.get(agent)?;
        let keep = match entry.status {
            StopStatus::Asked => true,
            StopStatus::Stopped => !working,
            StopStatus::NotStopped => now.saturating_duration_since(entry.at) < NOT_STOPPED_SAID_FOR,
        };
        let status = entry.status;
        if !keep {
            entries.remove(agent);
            return None;
        }
        Some(status)
    }
}

/// The stops of this shell.
pub fn stops() -> &'static Stops {
    static STOPS: OnceLock<Stops> = OnceLock::new();
    STOPS.get_or_init(Stops::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const HERMES: &str = "hermes:c-1";
    const MIND: &str = "mind:main";

    /// A Mind that is not the one the desk shows cannot be asked about, so a Mind pressing Stop in
    /// a desk that shows another cannot reach `stop_agent` by it: not even a card is raised.
    #[test]
    fn a_stop_for_a_mind_the_desk_is_not_showing_raises_nothing() {
        let stops = Stops::default();
        let raised = Cell::new(0);
        let stopped = Cell::new(0);
        let raise = || {
            raised.set(raised.get() + 1);
            Ok("req-1".to_string())
        };
        assert!(stops.ask(HERMES, Some(MIND), Instant::now(), raise).is_err());
        assert!(stops.ask(HERMES, None, Instant::now(), || Ok("req-1".into())).is_err());
        assert_eq!((raised.get(), stopped.get()), (0, 0));
        stopped.set(0);
        assert_eq!(stops.status_of(HERMES, true, Instant::now()), None);
    }

    /// Asking stops nothing. The Mind is stopped by the person's Allow and by nothing before it.
    #[test]
    fn nothing_stops_until_the_person_allows_it_on_their_desktop() {
        let stops = Stops::default();
        let stopped = Cell::new(0);
        let now = Instant::now();
        let id = stops.ask(MIND, Some(MIND), now, || Ok("req-1".into())).unwrap();
        assert_eq!(id, "req-1");
        assert_eq!(stopped.get(), 0, "asked: nothing has been stopped");
        assert_eq!(stops.status_of(MIND, true, now), Some(StopStatus::Asked));

        // Denied, or left to expire: the stop never runs.
        let status = stops.settle(MIND, false, now, || {
            stopped.set(stopped.get() + 1);
            Ok(())
        });
        assert_eq!((status, stopped.get()), (StopStatus::NotStopped, 0));

        // Allowed: it runs once, and the desk is told "stopped" because the shell did.
        stops.ask(MIND, Some(MIND), now, || Ok("req-2".into())).unwrap();
        let status = stops.settle(MIND, true, now, || {
            stopped.set(stopped.get() + 1);
            Ok(())
        });
        assert_eq!((status, stopped.get()), (StopStatus::Stopped, 1));
    }

    #[test]
    fn an_allow_whose_stop_failed_is_not_said_to_have_stopped_it() {
        let stops = Stops::default();
        let now = Instant::now();
        stops.ask(MIND, Some(MIND), now, || Ok("req-1".into())).unwrap();
        assert_eq!(stops.settle(MIND, true, now, || Err("no harness".into())), StopStatus::NotStopped);
    }

    #[test]
    fn the_person_is_asked_one_question_at_a_time_about_one_mind() {
        let stops = Stops::default();
        let raised = Cell::new(0);
        let now = Instant::now();
        let raise = || {
            raised.set(raised.get() + 1);
            Ok("req".to_string())
        };
        assert!(stops.ask(MIND, Some(MIND), now, raise).is_ok());
        assert!(stops.ask(MIND, Some(MIND), now, raise).is_err(), "a second press raises no second card");
        assert_eq!(raised.get(), 1);
        // A card the shell could not raise leaves nothing asked.
        let other = Stops::default();
        assert!(other.ask(MIND, Some(MIND), now, || Err("cards are off".into())).is_err());
        assert_eq!(other.status_of(MIND, true, now), None);
    }

    #[test]
    fn what_the_desk_is_told_is_let_go_when_it_is_stale() {
        let stops = Stops::default();
        let t0 = Instant::now();
        stops.ask(MIND, Some(MIND), t0, || Ok("req".into())).unwrap();
        stops.settle(MIND, true, t0, || Ok(()));
        assert_eq!(stops.status_of(MIND, false, t0), Some(StopStatus::Stopped), "while it is at rest");
        assert_eq!(stops.status_of(MIND, true, t0), None, "and not once it is at work again");
        assert_eq!(stops.status_of(MIND, false, t0), None, "gone for good");

        stops.ask(HERMES, Some(HERMES), t0, || Ok("req".into())).unwrap();
        stops.settle(HERMES, false, t0, || Ok(()));
        assert_eq!(stops.status_of(HERMES, true, t0 + Duration::from_secs(5)), Some(StopStatus::NotStopped));
        assert_eq!(stops.status_of(HERMES, true, t0 + NOT_STOPPED_SAID_FOR), None);
    }
}
