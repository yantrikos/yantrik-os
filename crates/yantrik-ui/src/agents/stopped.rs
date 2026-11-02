//! What a Stop came to for the agent: stopping, stopped, or stopped waiting.
//!
//! The Lens's Stop drops the turn on the desktop's side at once. Whether the harness stopped is
//! the harness's to say, and a harness that does not heartbeat (Hermes on a tokenless bridge) may
//! carry on for minutes. So the agent reads `stopping` from the press; `stopped` (at rest) only
//! when the host says the harness heard it, and `stopped waiting` when the grace passed without
//! it: the desktop stopped waiting and cannot say the harness stopped.

use std::collections::HashMap;
use std::sync::Mutex;

use yantrik_harness::{Heard, Host};

use super::model::{AgentId, State};

/// The turn each agent's Stop dropped, for the host to be asked about.
static STOPS: Mutex<Option<HashMap<String, Vec<u64>>>> = Mutex::new(None);

/// A Stop dropped `turns` of `agent` that the harness held (none: it had collected nothing).
pub fn note(agent: &AgentId, turns: Vec<u64>) {
    STOPS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(agent.0.clone(), turns);
}

fn turns_of(agent: &AgentId) -> Option<Vec<u64>> {
    STOPS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&agent.0).cloned())
}

/// What the agent reads once the host has said whether the harness heard the Stop. `None`: not yet.
pub fn reading(heard: Heard) -> Option<State> {
    match heard {
        Heard::Yes => Some(State::Stopped),
        Heard::Waiting => None,
        Heard::Unknown => Some(State::StoppedWaiting),
    }
}

/// Settle every agent that reads `stopping`. Called on the Agents tick.
pub fn settle(host: &Host) {
    let stopping: Vec<AgentId> = super::store().read(|s| s.agents().iter().filter(|a| a.state == State::Stopping).map(|a| a.meta.id.clone()).collect());
    for agent in stopping {
        // No record of the Stop (a restart): the desktop cannot know.
        let heard = turns_of(&agent).map_or(Heard::Unknown, |turns| host.stop_heard(&agent, &turns));
        if let Some(state) = reading(heard) {
            super::store().set_state(&agent, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mind_desk::project::state_of as mind_wire_state;

    /// Only a harness that confirmed it heard the Stop is at rest. Stopping, and a Stop nobody
    /// confirmed, are never `idle` to a Mind reading the wire.
    #[test]
    fn only_a_stop_the_harness_heard_is_at_rest_on_the_mind_wire() {
        assert_eq!(reading(Heard::Waiting), None, "still stopping");
        assert_eq!(reading(Heard::Yes), Some(State::Stopped));
        assert_eq!(reading(Heard::Unknown), Some(State::StoppedWaiting));
        use yantrik_ipc_contracts::mind_desk::MindState;
        assert_eq!(mind_wire_state(State::Stopped), MindState::Idle);
        for unconfirmed in [State::Stopping, State::StoppedWaiting] {
            assert_ne!(mind_wire_state(unconfirmed), MindState::Idle, "{unconfirmed:?}");
        }
        assert_eq!(State::StoppedWaiting.label(), "stopped waiting");
    }
}
