//! One change to the machine at a time.
//!
//! Before the framework, every change ran on the UI thread, so a second press waited until the
//! first had finished. In jobs, on a pool of four threads, two changes would run side by side,
//! and which of two quick radio flips network-service applied last would be down to thread
//! timing: the radio could end up off when the person's last word was on. On a Wi-Fi-only
//! machine reached from somewhere else, that is the outcome `wifi_radio` is graded `dangerous`
//! for. So the lane keeps one change in flight. The next one starts when the change in flight
//! lands: its result, or its `failed` command when its job ended without one (a panic).
//!
//! # What waits, and what is refused instead
//!
//! - **A change nobody is waiting on for an answer waits its turn here, in order.** That is a
//!   button press, or `wifi_connect`, which answers as soon as it is accepted and reports how
//!   the join went in `describe`.
//! - **The radio waits as one wanted state.** A newer press replaces the waiting one in its
//!   place, because only the person's last word counts, and running every flip in turn would
//!   flap the radio. The press it replaces came from a button, so no caller is owed an answer
//!   for it, and nothing is settled for it.
//! - **A change an intent waits on is refused with `busy` while another change is in flight.**
//!   These are `wifi_radio`, `wifi_disconnect` and `wifi_forget`. The framework answers an
//!   invocation as soon as nothing of it is running or queued. A change parked in this model
//!   is neither, so its caller would be told "done", with a reply read from a machine it never
//!   touched. Holding the invocation open instead needs a job, and that job would sit on one of
//!   the pool's four threads for up to a whole change. There is a better reason too: a
//!   dangerous change decided against a machine that is about to change under it should be
//!   decided again. So the caller is told that nothing was changed and why, and can read
//!   `describe` and ask again. (Letting these queue with their own answer needs the framework
//!   to hold an invocation for a queued command; that is a framework change, and this app does
//!   not make one.)
//!
//! A scan does not go through the lane. It changes nothing on the machine, it already ran
//! beside the refreshes, and a scan that fails only says so.

use std::collections::VecDeque;

use crate::backend::NetworkApi;
use crate::model::{Change, Outcome};

/// How many changes may wait behind the one in flight. A person pressing faster than the
/// service answers needs a few; past that the press is refused out loud, not held without end.
pub const MAX_WAITING: usize = 4;

/// What one change does off the UI thread: its call to the service. A connect's typed secret is
/// captured in here, so it sits in nothing that can be formatted or described while it waits.
pub type Work = Box<dyn FnOnce(&dyn NetworkApi) -> Outcome + Send + 'static>;

/// A change on its way to the service.
pub struct Next {
    pub change: Change,
    pub work: Work,
}

/// What the lane did with a change it was given.
pub enum Admit {
    /// Nothing was in flight: start it now.
    Now(Next),
    /// It waits its turn, or it replaced the radio state that was waiting.
    Waits,
    /// Too many are waiting already. It is dropped, with its secret if it had one.
    Full,
}

#[derive(Default)]
pub struct Lane {
    in_flight: Option<Change>,
    waiting: VecDeque<Next>,
}

impl Lane {
    /// The change in flight, if one is.
    pub fn in_flight(&self) -> Option<Change> {
        self.in_flight
    }

    /// Changes waiting behind the one in flight.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// A change asks to go: now, after the ones before it, or not at all.
    pub fn admit(&mut self, next: Next) -> Admit {
        if self.in_flight.is_none() {
            self.in_flight = Some(next.change);
            return Admit::Now(next);
        }
        if next.change == Change::Radio {
            if let Some(waiting) = self.waiting.iter_mut().find(|w| w.change == Change::Radio) {
                *waiting = next;
                return Admit::Waits;
            }
        }
        if self.waiting.len() >= MAX_WAITING {
            return Admit::Full;
        }
        self.waiting.push_back(next);
        Admit::Waits
    }

    /// The change in flight landed, with a result or without one. Returns the next change to
    /// start, which is then the one in flight. A landing for a change that is not the one in
    /// flight releases nothing: the one in flight is still talking to the service.
    pub fn landed(&mut self, change: Change) -> Option<Next> {
        if self.in_flight != Some(change) {
            tracing::debug!(?change, in_flight = ?self.in_flight, "a landing for a change not in flight");
            return None;
        }
        self.in_flight = None;
        let next = self.waiting.pop_front()?;
        self.in_flight = Some(next.change);
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radio(enabled: bool) -> Next {
        let result = Err(if enabled { "on" } else { "off" }.to_string());
        Next {
            change: Change::Radio,
            work: Box::new(move |_| Outcome::Radio(result)),
        }
    }

    fn disconnect() -> Next {
        Next {
            change: Change::Disconnect,
            work: Box::new(|_| Outcome::Disconnect(Err("disconnect".into()))),
        }
    }

    /// What a waiting change would ask, told by the error each test closure carries.
    fn said(next: Next) -> String {
        struct Nobody;
        impl NetworkApi for Nobody {
            fn call(&self, _: &str, _: serde_json::Value) -> Result<serde_json::Value, crate::backend::RpcFailure> {
                unreachable!("the test closures never call")
            }
        }
        match (next.work)(&Nobody) {
            Outcome::Radio(Err(s)) | Outcome::Disconnect(Err(s)) => s,
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_first_change_starts_and_the_next_waits_until_it_lands() {
        let mut lane = Lane::default();
        assert!(matches!(lane.admit(disconnect()), Admit::Now(_)));
        assert!(matches!(lane.admit(radio(false)), Admit::Waits));
        assert_eq!(lane.in_flight(), Some(Change::Disconnect));
        let next = lane.landed(Change::Disconnect).expect("the radio is next");
        assert_eq!(lane.in_flight(), Some(Change::Radio));
        assert_eq!(said(next), "off");
        assert!(lane.landed(Change::Radio).is_none());
        assert_eq!(lane.in_flight(), None);
    }

    #[test]
    fn only_the_last_radio_state_waits_and_it_keeps_its_place() {
        let mut lane = Lane::default();
        let _ = lane.admit(radio(false));
        let _ = lane.admit(radio(true));
        let _ = lane.admit(disconnect());
        let _ = lane.admit(radio(false));
        assert_eq!(lane.waiting(), 2);
        assert_eq!(said(lane.landed(Change::Radio).unwrap()), "off", "the last word, where the first waited");
        assert_eq!(said(lane.landed(Change::Radio).unwrap()), "disconnect");
    }

    #[test]
    fn a_full_lane_refuses_and_a_stray_landing_releases_nothing() {
        let mut lane = Lane::default();
        let _ = lane.admit(radio(true));
        for _ in 0..MAX_WAITING {
            assert!(matches!(lane.admit(disconnect()), Admit::Waits));
        }
        assert!(matches!(lane.admit(disconnect()), Admit::Full));
        assert!(lane.landed(Change::Forget).is_none());
        assert_eq!(lane.in_flight(), Some(Change::Radio));
        assert_eq!(lane.waiting(), MAX_WAITING);
    }
}
