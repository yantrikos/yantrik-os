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

/// Settle every agent whose Stop is not settled: `stopping` goes to `stopped` once the harness
/// closed the turn (the Lens's "Stopping..." ends at the same moment), to `stopped waiting` once
/// the grace passed without it; and `stopped waiting` is upgraded if the harness closes the turn
/// after all. An agent with a command the shell still runs for it stays `stopping`: it is not at
/// rest. Called on the Agents tick.
pub fn settle(host: &Host) {
    settle_with(|agent, turns| host.stop_heard(agent, turns));
}

fn settle_with(heard_of: impl Fn(&AgentId, &[u64]) -> Heard) {
    let unsettled: Vec<(AgentId, State, bool)> = super::store().read(|s| {
        s.agents()
            .iter()
            .filter(|a| matches!(a.state, State::Stopping | State::StoppedWaiting))
            .map(|a| (a.meta.id.clone(), a.state, a.cards().any(|c| c.running())))
            .collect()
    });
    for (agent, state, running) in unsettled {
        // No record of the Stop (a restart): the desktop cannot know.
        let heard = turns_of(&agent).map_or(Heard::Unknown, |turns| heard_of(&agent, &turns));
        let next = match (reading(heard), running) {
            (Some(State::Stopped), true) => None,
            (next, _) => next,
        };
        if let Some(next) = next.filter(|n| *n != state) {
            super::store().set_state(&agent, next);
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
        assert_eq!(mind_wire_state(State::StoppedWaiting), MindState::Thinking, "not a failure the person did not cause");
        for unconfirmed in [State::Stopping, State::StoppedWaiting] {
            assert_ne!(mind_wire_state(unconfirmed), MindState::Idle, "{unconfirmed:?}");
        }
        assert_eq!(State::StoppedWaiting.label(), "stopped waiting");
    }

    fn stopped_agent(name: &str) -> AgentId {
        let agent = AgentId::new(name, AgentId::MAIN);
        super::super::store().open_turn(&agent, "browse five sites");
        super::super::store().close_turn_stopped(&agent);
        assert_eq!(state_of(&agent), State::Stopping);
        agent
    }

    fn state_of(agent: &AgentId) -> State {
        super::super::store().read(|s| s.agent(agent).unwrap().state)
    }

    /// The path that matters for Hermes: after the grace, with the flight still open, the agent
    /// reads `stopped waiting`; if the harness closes the turn later it is upgraded to `stopped`;
    /// and a restart (no record of the Stop) reads `stopped waiting`, never at rest.
    #[test]
    fn settle_reads_stopped_waiting_after_the_grace_and_upgrades_when_the_harness_closes() {
        let agent = stopped_agent("settle-grace");
        note(&agent, vec![7]);
        settle_with(|_, _| Heard::Waiting);
        assert_eq!(state_of(&agent), State::Stopping, "within the grace: still stopping, as the Lens says");
        settle_with(|_, _| Heard::Unknown);
        assert_eq!(state_of(&agent), State::StoppedWaiting, "grace over, flight open");
        settle_with(|_, _| Heard::Unknown);
        assert_eq!(state_of(&agent), State::StoppedWaiting, "and it stays so");
        settle_with(|_, _| Heard::Yes);
        assert_eq!(state_of(&agent), State::Stopped, "the harness closed the turn after all");

        let restarted = stopped_agent("settle-restart");
        settle_with(|_, _| panic!("no record of the Stop: the host is not asked"));
        assert_eq!(state_of(&restarted), State::StoppedWaiting);
    }

    /// A stopped turn whose shell command still runs is not at rest, whatever the harness heard.
    #[test]
    fn a_stopped_agent_with_a_command_still_running_is_not_at_rest() {
        let agent = AgentId::new("settle-busy", AgentId::MAIN);
        let store = super::super::store();
        store.open_turn(&agent, "run the build");
        // A command the shell owns (verified): it outlives the turn that was stopped.
        store.command_started(&agent, "j1", "make", "/tmp");
        store.close_turn_stopped(&agent);
        note(&agent, vec![9]);
        settle_with(|_, _| Heard::Yes);
        assert_eq!(state_of(&agent), State::Stopping, "the command runs on: not Stopped, so not idle on the wire");
        store.command_finished(&agent, "j1", "make", Some(0), false);
        settle_with(|_, _| Heard::Yes);
        assert_eq!(state_of(&agent), State::Stopped, "at rest once the command ended");
    }
}
