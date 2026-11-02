//! Dropping a turn that is running (#234, #1058).
//!
//! The harness does not poll for new turns while one runs. It sends an empty chunk every
//! heartbeat (30 s) and stops the turn when the desktop answers one `{"dropped": true}`
//! (`Host::chunk`: a flight with nowhere to send its chunks is answered so). Everything that
//! ends a running turn on the person's word ends in [`Host::drop_turn`]: the Lens's Stop, a word
//! to a stuck task, and [`Host::stop_agent`], which the Mind desk's stop runs.

use std::collections::VecDeque;

use super::{Attached, Host, RunState, Waiting, INTERRUPTED_TO_TELL, STOPPED};
use crate::Chunk;
use crate::event::AgentId;

/// Whether the harness has heard that a turn was dropped ([`Host::stop_heard`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// It called on the turn and was told, or it closed the turn: it knows.
    Yes,
    /// Not yet, and the grace for it has not passed.
    Waiting,
    /// The grace passed (or the harness is gone) without a call: the desktop does not know whether
    /// it stopped.
    Unknown,
}

impl Host {
    /// Interrupt what `agent` is doing, without ending it (#234): the turn in flight in its
    /// conversation is cancelled (the harness hears on its next poll, the listener is settled, the
    /// run ends `cancelled`), but the agent, its conversation and its token stay, so the next turn
    /// (the person's word to a stuck task) goes to the same mind with its context. Unlike
    /// [`Host::stop_agent`], nothing is ended. `false` when it had nothing in flight.
    pub fn interrupt(&self, agent: &AgentId) -> bool {
        self.drop_turn(agent, INTERRUPTED_TO_TELL, None)
    }

    /// Stop the turns of `agent` the person could see and nothing else (#1058): the person pressed
    /// Stop in the Lens. Like [`Host::interrupt`] the agent, its conversation and its token stay;
    /// unlike it, the reader is told the turn was [`STOPPED`]. The harness learns on its next call
    /// on a turn it has taken, which for a streaming turn is its next heartbeat: that call is
    /// answered `{"dropped": true}`. A turn the harness has not collected yet is failed and taken
    /// off the queue, so the harness's next poll never hands it over.
    ///
    /// `upto` is the newest turn the person saw ([`Host::live_turn`]). Every turn of the
    /// conversation up to it is dropped, whether in flight or still queued: a conversation can have
    /// two (a word sent to a working agent), and the person cannot tell which one Stop is for.
    /// A turn newer than `upto` is left alone, so a Stop pressed over a turn that has just ended
    /// cannot land on the one that started in its place. `false` when nothing was dropped.
    pub fn stop_turn(&self, agent: &AgentId, upto: u64) -> bool {
        self.drop_turn(agent, STOPPED, Some(upto))
    }

    /// The id of the newest turn of `agent` that is still waited on, in flight or queued for the
    /// harness to collect: what a Stop pressed now is for ([`Host::stop_turn`]).
    pub fn live_turn(&self, agent: &AgentId) -> Option<u64> {
        let mut state = self.lock();
        self.reap(&mut state);
        let harness = state.attached.get(agent.harness())?;
        let conversation = agent.conversation();
        let flying = harness.in_flight.iter().filter(|(_, f)| f.conversation == conversation && f.tx.is_some()).map(|(id, _)| *id);
        let waiting = harness.queued.iter().filter(|w| w.assignment.conversation == conversation).map(|w| w.assignment.turn_id);
        flying.chain(waiting).max()
    }

    /// The turns of `agent` the harness holds, in flight and waited on, up to `upto`: what a
    /// [`Host::stop_turn`] with that bound will hand a `dropped` to. Ask before stopping.
    pub fn flights_up_to(&self, agent: &AgentId, upto: u64) -> Vec<u64> {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get(agent.harness()) else { return Vec::new() };
        let mut turns: Vec<u64> = harness
            .in_flight
            .iter()
            .filter(|(id, f)| f.conversation == agent.conversation() && f.tx.is_some() && **id <= upto)
            .map(|(id, _)| *id)
            .collect();
        turns.sort_unstable();
        turns
    }

    /// Whether the harness has heard that `turns` of `agent` were dropped: it called on each
    /// (and was answered `dropped`) or closed it. A harness that streams does so within a
    /// heartbeat; one that does not may never, and after [`STOP_GRACE`] the desktop stops waiting
    /// for it without knowing either way. No turns (what was stopped never reached the harness):
    /// there is nothing to hear.
    pub fn stop_heard(&self, agent: &AgentId, turns: &[u64]) -> Heard {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get(agent.harness()) else {
            return if turns.is_empty() { Heard::Yes } else { Heard::Unknown };
        };
        let mut all = Heard::Yes;
        for turn in turns {
            let one = match harness.in_flight.get(turn) {
                Some(f) if f.drop_logged => Heard::Yes,
                Some(f) if f.holds_conversation() => Heard::Waiting,
                Some(_) => Heard::Unknown,
                None if harness.finished.contains(turn) => Heard::Yes,
                None => Heard::Unknown,
            };
            all = match (all, one) {
                (Heard::Waiting, _) | (_, Heard::Waiting) => Heard::Waiting,
                (Heard::Unknown, _) | (_, Heard::Unknown) => Heard::Unknown,
                _ => Heard::Yes,
            };
        }
        all
    }

    /// Whether a turn of `agent` was dropped but its harness has not closed it yet, so the
    /// conversation is still held: the span in which the Lens says "Stopping…". False once the
    /// harness closes the turn, or the grace for it passes.
    pub fn stopping(&self, agent: &AgentId) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        state.attached.get(agent.harness()).is_some_and(|h| {
            h.in_flight.values().any(|f| {
                f.conversation == agent.conversation() && f.tx.is_none() && f.abandoned.is_some() && f.holds_conversation()
            })
        })
    }

    /// The one way a running turn is dropped: settle it for the reader with `failure`, and queue
    /// the harness's cancel. With `upto`, the turns not yet collected are failed too.
    fn drop_turn(&self, agent: &AgentId, failure: &str, upto: Option<u64>) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get_mut(agent.harness()) else { return false };
        let conversation = agent.conversation();
        let mut dropped = false;
        if let Some(upto) = upto {
            let (theirs, others): (VecDeque<Waiting>, VecDeque<Waiting>) = std::mem::take(&mut harness.queued)
                .into_iter()
                .partition(|w| w.assignment.conversation == conversation && w.assignment.turn_id <= upto);
            harness.queued = others;
            for waiting in theirs {
                let _ = waiting.tx.send(Chunk::Failed(failure.to_string()));
                dropped = true;
            }
        }
        let cancelled = Self::drop_flights(harness, conversation, failure, upto);
        drop(state);
        self.end_runs(&cancelled, RunState::Cancelled);
        dropped || !cancelled.is_empty()
    }

    /// Abandon every turn in flight in `conversation` with `failure` and tell the harness to
    /// cancel them (only those up to turn `upto`, when given); returns their ids. Also what
    /// [`Host::stop_agent`] does to the turns it stops.
    pub(super) fn drop_flights(harness: &mut Attached, conversation: &str, failure: &str, upto: Option<u64>) -> Vec<u64> {
        let mut cancelled = Vec::new();
        for (turn_id, flight) in harness.in_flight.iter_mut() {
            if flight.conversation == conversation && flight.tx.is_some() && upto.map_or(true, |id| *turn_id <= id) {
                flight.abandon(Some(failure.to_string()));
                harness.cancelled.push(*turn_id);
                cancelled.push(*turn_id);
            }
        }
        cancelled
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{protocol, Chunk, Turn};

    fn attached() -> (Host, String, AgentId) {
        let host = Host::new(vec![]);
        let attach = json!({ "id": "pi", "name": "Pi", "conversations": true });
        let session = host.handle(protocol::ATTACH, &attach).unwrap()["session"].as_str().unwrap().to_string();
        let agent = host.start_agent("pi").unwrap();
        (host, session, agent)
    }

    fn beat(host: &Host, session: &str, turn_id: u64) -> serde_json::Value {
        host.handle(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "" })).unwrap()
    }

    #[test]
    fn a_stopped_turn_answers_the_next_heartbeat_dropped_and_keeps_the_agent_until_the_harness_closes_it() {
        let (host, session, agent) = attached();
        let answer = host.send_to(&agent, Turn::new("tidy the photos")).unwrap();
        let turn_id = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        assert_eq!(beat(&host, &session, turn_id), json!({}), "a running turn's heartbeat is answered plainly");
        assert!(!host.stopping(&agent), "nothing is stopping yet");

        assert_eq!(host.live_turn(&agent), Some(turn_id));
        assert!(host.stop_turn(&agent, turn_id));
        assert_eq!(answer.iter().last(), Some(Chunk::Failed(STOPPED.into())));
        assert!(host.stopping(&agent), "dropped, and the harness has not closed it");
        assert_eq!(beat(&host, &session, turn_id), json!({ "dropped": true }), "the next heartbeat is how the harness learns");
        let told = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(told["cancelled"], json!([turn_id]));
        assert!(told.get("ended").is_none(), "the conversation is not ended");
        assert!(host.send_to(&agent, Turn::new("again")).is_ok(), "the agent stays");

        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn_id })).unwrap();
        assert!(!host.stopping(&agent), "closed: no longer stopping");
        assert!(!host.stop_turn(&agent, turn_id), "nothing left in flight to stop");
    }

    #[test]
    fn stopping_a_turn_is_not_stopping_the_agent_and_a_stop_with_nothing_running_is_false() {
        let (host, _session, agent) = attached();
        assert!(!host.stop_turn(&agent, 1), "nothing in flight");
        assert_eq!(host.live_turn(&agent), None);
        assert!(!host.stopping(&agent));
        assert_eq!(host.agents().len(), 1);
    }

    /// A second Stop while the first is waiting out its grace is false, and the harness is told
    /// to cancel the turn once, not twice.
    #[test]
    fn a_second_stop_before_the_harness_closes_the_turn_is_false_and_cancels_once() {
        let (host, session, agent) = attached();
        let _answer = host.send_to(&agent, Turn::new("tidy the photos")).unwrap();
        let turn_id = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();

        assert!(host.stop_turn(&agent, turn_id));
        assert!(!host.stop_turn(&agent, turn_id), "already dropped; the harness has not closed it");
        assert_eq!(host.live_turn(&agent), None, "nothing is being waited on");
        let told = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(told["cancelled"], json!([turn_id]), "one cancel, not two");
    }

    /// A Stop for the turn the person saw cannot land on the turn that started after it.
    #[test]
    fn a_stop_for_a_turn_that_ended_does_not_drop_the_next_one() {
        let (host, session, agent) = attached();
        let _first = host.send_to(&agent, Turn::new("one")).unwrap();
        let first = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": first })).unwrap();
        let _second_answer = host.send_to(&agent, Turn::new("two")).unwrap();
        let second = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        assert_ne!(first, second);
        assert_eq!(host.live_turn(&agent), Some(second));

        assert!(!host.stop_turn(&agent, first), "the turn the person saw is gone; the newer one is not theirs");
        assert_eq!(host.live_turn(&agent), Some(second), "the new turn runs on");
        assert!(host.stop_turn(&agent, second));
    }

    /// A turn the harness has not collected yet is stopped too: failed for the reader and taken
    /// off the queue, so the next poll does not hand it over (the "delete the old backups" case).
    #[test]
    fn a_stop_before_the_harness_polls_fails_the_queued_turn_and_it_is_never_handed_over() {
        let (host, session, agent) = attached();
        let answer = host.send_to(&agent, Turn::new("delete the old backups")).unwrap();
        let queued = host.live_turn(&agent).expect("a queued turn is a turn the person can see");

        assert!(host.stop_turn(&agent, queued));
        assert_eq!(answer.iter().last(), Some(Chunk::Failed(STOPPED.into())), "the reader is told it stopped");
        assert_eq!(host.live_turn(&agent), None);
        let polled = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert!(polled.get("turn_id").map_or(true, |v| v.is_null()), "nothing is handed to the harness: {polled}");
        assert!(!host.stop_turn(&agent, queued), "a second Stop has nothing left");
    }

    /// Two turns of one conversation (a word to a working agent): Stop drops both the person
    /// could see, newest id first or not, and leaves a later one.
    #[test]
    fn a_stop_drops_every_turn_up_to_the_newest_seen_and_leaves_a_later_one() {
        let (host, session, agent) = attached();
        let _one = host.send_to(&agent, Turn::new("one")).unwrap();
        let first = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        let two = host.send_to(&agent, Turn::new("two")).unwrap();
        let seen = host.live_turn(&agent).unwrap();
        assert!(seen > first, "the queued turn is the newest");
        let _three = host.send_to(&agent, Turn::new("three")).unwrap();

        assert!(host.stop_turn(&agent, seen));
        assert_eq!(two.iter().last(), Some(Chunk::Failed(STOPPED.into())));
        assert_eq!(host.live_turn(&agent), Some(seen + 1), "the turn after the one seen is untouched");
        let told = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(told["cancelled"], json!([first]), "the one in flight is cancelled");
    }

    /// The host says whether the harness heard a Stop: not yet, then yes once it calls on the
    /// dropped turn (the heartbeat answered `dropped`); a harness that never calls is `Unknown`
    /// after the grace, and a Stop of what it never collected has nothing to hear.
    #[test]
    fn the_host_says_when_the_harness_heard_a_stop() {
        let (host, session, agent) = attached();
        let _answer = host.send_to(&agent, Turn::new("tidy the photos")).unwrap();
        let turn_id = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        let handed = host.flights_up_to(&agent, turn_id);
        assert_eq!(handed, vec![turn_id]);
        assert_eq!(host.flights_up_to(&agent, turn_id - 1), Vec::<u64>::new(), "the bound is respected");

        assert!(host.stop_turn(&agent, turn_id));
        assert_eq!(host.stop_heard(&agent, &handed), Heard::Waiting, "dropped, and the harness has not called");
        assert_eq!(beat(&host, &session, turn_id), json!({ "dropped": true }));
        assert_eq!(host.stop_heard(&agent, &handed), Heard::Yes, "it called and was told");
        assert_eq!(host.stop_heard(&agent, &[]), Heard::Yes, "nothing handed to the harness: nothing to hear");
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn_id })).unwrap();
        assert_eq!(host.stop_heard(&agent, &handed), Heard::Yes, "closed: heard");
        assert_eq!(host.stop_heard(&agent, &[turn_id + 50]), Heard::Unknown, "a turn the host never saw closed");
    }
}
