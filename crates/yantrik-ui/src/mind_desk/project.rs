//! Which Mind the desk shows, and what of it.
//!
//! One Mind at a time in phase 1: there is one Mind View, and nothing yet says which of the
//! Minds at work owns which window (phase 4 adds that, and the desk's tabs with it). So the desk
//! follows the Mind that most needs the person's eye: first one waiting on them, then one at
//! work, then the one heard from last.
//!
//! Only display-safe things leave here. The tool is its *name* (`Card::name`, checked to be a
//! plain name), never its target, its arguments, its preview, its summary or its output; the Mind
//! is its name, never the title (which is what the person asked it), and **never its `status`**:
//! what a Mind says it is doing is withheld from every other Mind (`PERSONS_FIELDS`), and the desk
//! is on a screen all of them can read. That line is for phase 4, with attribution.

use yantrik_harness::AgentId;
use yantrik_ipc_contracts::mind_desk::{
    self as wire, Account, Counts, DeskMind, DeskSnapshot, DeskTool, Evidence, MindState, Outcome,
};

use crate::agents::model::{Agent, CallState, Card, Provenance, State, Turn};

/// The snapshot of `agents`, as of `now` (unix seconds).
///
/// `apps` are the apps open in Mind View, by the names a person reads. The Mind's account is left
/// `Unknown`: it is a fact of the harness host, which has a lock of its own, so the caller asks
/// the host after this has let go of the store ([`with_account`]).
pub fn project(agents: &[Agent], now: u64, apps: &[String]) -> DeskSnapshot {
    DeskSnapshot {
        tag: 0,
        as_of: hm(now),
        mind: focus(agents).map(|a| mind_of(a, now)),
        apps: apps
            .iter()
            .map(|a| wire::display_name(a))
            .filter(|a| !a.is_empty())
            .take(wire::APPS_MAX)
            .collect(),
    }
    .tagged()
}

/// `snapshot`, with how a stop asked of the person stands (`stop_of`: the agent, whether it is at
/// work), tagged again. Asked of the stops after the store has been let go of.
pub fn with_stop(mut snapshot: DeskSnapshot, stop_of: impl Fn(&str, bool) -> Option<wire::StopStatus>) -> DeskSnapshot {
    if let Some(mind) = snapshot.mind.as_mut() {
        mind.stop = stop_of(&mind.agent, mind.state.working());
    }
    snapshot.tagged()
}

/// `snapshot`, with the account of its Mind's harness as `account_of` says it, and tagged again.
pub fn with_account(mut snapshot: DeskSnapshot, account_of: impl Fn(&str) -> Account) -> DeskSnapshot {
    if let Some(mind) = snapshot.mind.as_mut() {
        mind.account = account_of(AgentId(mind.agent.clone()).harness());
    }
    snapshot.tagged()
}

/// The agent the desk follows: one waiting on the person, else one at work, else the one heard
/// from last; the newest first within each.
pub fn focus(agents: &[Agent]) -> Option<&Agent> {
    let rank = |a: &Agent| match a.state {
        State::WaitingForYou => 2u8,
        State::Thinking | State::RunningTool => 1,
        _ => 0,
    };
    agents.iter().max_by_key(|a| (rank(a), a.touched, a.seq))
}

fn state_of(state: State) -> MindState {
    match state {
        State::Thinking => MindState::Thinking,
        State::RunningTool => MindState::RunningTool,
        State::WaitingForYou => MindState::WaitingForYou,
        State::Idle => MindState::Idle,
        State::Done => MindState::Done,
        State::Failed => MindState::Failed,
        State::HarnessGone => MindState::HarnessGone,
    }
}

fn outcome_of(state: CallState) -> Outcome {
    match state {
        CallState::Running => Outcome::Running,
        CallState::Ok => Outcome::Ok,
        CallState::Failed => Outcome::Failed,
        CallState::Interrupted => Outcome::Interrupted,
        CallState::Untold => Outcome::Untold,
    }
}

fn evidence_of(p: Provenance) -> Evidence {
    match p {
        Provenance::Verified => Evidence::Verified,
        Provenance::Reported => Evidence::Reported,
    }
}

/// The run the desk counts: the turn that is open, else the last one that did work.
fn current_run(agent: &Agent) -> Option<&Turn> {
    agent.open_turn().or_else(|| agent.turns.iter().rev().find(|t| t.did_work()))
}

/// Local `HH:MM` for a unix time.
fn hm(unix: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix as i64, 0)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

fn mind_of(agent: &Agent, now: u64) -> DeskMind {
    let run = current_run(agent);
    // As the Agents screen draws them: a call holding words the person erased reads masked, and
    // only its name and state are taken from it either way.
    let shown: Vec<_> = run.map(|t| t.cards().map(Card::shown).collect()).unwrap_or_default();
    let cards: Vec<&Card> = shown.iter().map(|c| &**c).collect();
    let count = |state: CallState| cards.iter().filter(|c| c.state == state).count() as u32;
    let tool = cards
        .iter()
        .rev()
        .find(|c| c.running())
        .or_else(|| cards.last())
        .and_then(|c| {
            Some(DeskTool { name: wire::tool_name(&c.name)?, evidence: evidence_of(c.provenance), outcome: outcome_of(c.state) })
        });
    let elapsed_secs = run.map(|t| t.ended.unwrap_or(now).saturating_sub(t.started));
    let model = [agent.meta.model.as_str(), agent.usage.model.as_str()].into_iter().find_map(wire::model_label);
    DeskMind {
        agent: agent.meta.id.0.clone(),
        name: wire::display_name(&agent.meta.mind),
        state: state_of(agent.state),
        since: agent.since,
        // Nothing it says it is doing, ever: `status` is the person's, and this is read by every
        // Mind in Mind View. A stop asked of the person is filled in by the caller.
        stop: None,
        tool,
        counts: Counts { calls: cards.len() as u32, failed: count(CallState::Failed), interrupted: count(CallState::Interrupted) },
        elapsed_secs,
        model,
        account: Account::Unknown,
    }
}
