//! How a turn closes: finished, failed, or stopped by the person.

use super::*;

impl Store {
    /// The turn ended — completed (`ok`) or failed.
    pub fn close_turn(&mut self, id: &AgentId, ok: bool) {
        self.close_turn_as(id, ok, false)
    }

    /// The turn ended because the person stopped it: failed as a turn (it did not finish), but the
    /// agent reads `stopping` until the host says the harness heard it, never `failed` or at rest.
    pub fn close_turn_stopped(&mut self, id: &AgentId) {
        self.close_turn_as(id, false, true)
    }

    fn close_turn_as(&mut self, id: &AgentId, ok: bool, stopped: bool) {
        let Some(i) = self.index(id) else { return };
        let now = self.now();
        let agent = &mut self.agents[i];
        if agent.open_turn().is_none() {
            return;
        }
        settle_turn(agent, ok, now, None);
        agent.status.clear();
        let next = if agent.state == State::HarnessGone {
            State::HarnessGone
        } else if stopped {
            // The person stopped it; whether the harness heard is settled by the host's word
            // (`agents/stopped.rs`), even while a command the shell owns is still running.
            State::Stopping
        } else if agent.cards().any(Card::running) {
            // A command the shell owns outlives the turn that asked for it.
            State::RunningTool
        } else if ok {
            State::Done
        } else {
            State::Failed
        };
        set_state(agent, next, now);
        compact(agent);
        self.mark(i);
    }
}
