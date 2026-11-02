//! Dropping a turn that is running (#234, #1058).
//!
//! The harness does not poll for new turns while one runs. It sends an empty chunk every
//! heartbeat (30 s) and stops the turn when the desktop answers one `{"dropped": true}`
//! (`Host::chunk`: a flight with nowhere to send its chunks is answered so). Everything that
//! ends a running turn on the person's word ends in [`Host::drop_turn`]: the Lens's Stop, a word
//! to a stuck task, and [`Host::stop_agent`], which the Mind desk's stop runs.

use super::{Attached, Host, RunState, INTERRUPTED_TO_TELL, STOPPED};
use crate::event::AgentId;

impl Host {
    /// Interrupt what `agent` is doing, without ending it (#234): the turn in flight in its
    /// conversation is cancelled (the harness hears on its next poll, the listener is settled, the
    /// run ends `cancelled`), but the agent, its conversation and its token stay, so the next turn
    /// (the person's word to a stuck task) goes to the same mind with its context. Unlike
    /// [`Host::stop_agent`], nothing is ended. `false` when it had nothing in flight.
    pub fn interrupt(&self, agent: &AgentId) -> bool {
        self.drop_turn(agent, INTERRUPTED_TO_TELL)
    }

    /// Stop the turn in flight for `agent` and nothing else (#1058): the person pressed Stop in the
    /// Lens. Like [`Host::interrupt`] the agent, its conversation and its token stay; unlike it,
    /// the reader is told the turn was [`STOPPED`]. The harness learns on its next call on the
    /// turn, which for a streaming turn is its next heartbeat: that call is answered
    /// `{"dropped": true}`. `false` when it had nothing in flight.
    pub fn stop_turn(&self, agent: &AgentId) -> bool {
        self.drop_turn(agent, STOPPED)
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
    /// the harness's cancel.
    fn drop_turn(&self, agent: &AgentId, failure: &str) -> bool {
        let mut state = self.lock();
        self.reap(&mut state);
        let Some(harness) = state.attached.get_mut(agent.harness()) else { return false };
        let cancelled = Self::drop_flights(harness, agent.conversation(), failure);
        drop(state);
        self.end_runs(&cancelled, RunState::Cancelled);
        !cancelled.is_empty()
    }

    /// Abandon every turn in flight in `conversation` with `failure` and tell the harness to
    /// cancel them; returns their ids. Also what [`Host::stop_agent`] does to the turns it stops.
    pub(super) fn drop_flights(harness: &mut Attached, conversation: &str, failure: &str) -> Vec<u64> {
        let mut cancelled = Vec::new();
        for (turn_id, flight) in harness.in_flight.iter_mut() {
            if flight.conversation == conversation && flight.tx.is_some() {
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

        assert!(host.stop_turn(&agent));
        assert_eq!(answer.iter().last(), Some(Chunk::Failed(STOPPED.into())));
        assert!(host.stopping(&agent), "dropped, and the harness has not closed it");
        assert_eq!(beat(&host, &session, turn_id), json!({ "dropped": true }), "the next heartbeat is how the harness learns");
        let told = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(told["cancelled"], json!([turn_id]));
        assert!(told.get("ended").is_none(), "the conversation is not ended");
        assert!(host.send_to(&agent, Turn::new("again")).is_ok(), "the agent stays");

        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn_id })).unwrap();
        assert!(!host.stopping(&agent), "closed: no longer stopping");
        assert!(!host.stop_turn(&agent), "nothing left in flight to stop");
    }

    #[test]
    fn stopping_a_turn_is_not_stopping_the_agent_and_a_stop_with_nothing_running_is_false() {
        let (host, _session, agent) = attached();
        assert!(!host.stop_turn(&agent), "nothing in flight");
        assert!(!host.stopping(&agent));
        assert_eq!(host.agents().len(), 1);
    }
}
