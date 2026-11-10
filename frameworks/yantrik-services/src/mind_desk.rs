//! The shell, as the Mind desk reaches it (#1023): what to draw, and the two things it may ask.
//!
//! All of it is the shell's own control surface (`app-shell`, `app.act`): the desk is not given a
//! socket of its own to the Agents store. The three calls are exactly the ones its threat model
//! allows. A Mind acting in Mind View can press the desk's buttons, so the desk can ask for
//! nothing a Mind should not be able to ask for by pressing, and the shell answers all three to
//! the one desk process it started and to no other:
//!
//! - [`MindDeskApi::snapshot`]: `mind_desk`, a read, waiting for a change.
//! - [`MindDeskApi::ask_stop`]: `mind_desk_stop`, which asks the *person*, on their own desktop.
//!   It stops nothing: `stop_agent` takes the desk for the person, who may stop any agent, so a
//!   direct stop would let any Mind end another's turn.
//! - [`MindDeskApi::open_agents`]: `mind_desk_open_agents`, the person's own Agents screen, rate
//!   limited and only while Mind View is in front.
//!
//! There is no method here to approve, deny, grant, take over or hand back, and no way to add one
//! without adding it to this trait, where a test reads the list.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use yantrik_ipc_contracts::mind_desk::{
    DeskSnapshot, StopAck, AGENTS_ACTION, LONGEST_WAIT_SECS, SNAPSHOT_ACTION, STOP_ACTION,
};
use yantrik_ipc_transport::SyncRpcClient;

use crate::{classify, shape_of, ServiceError};

/// The shell's control surface, as its socket is named.
pub const SERVICE: &str = "app-shell";

/// A stop and a showing are quick; a snapshot waits up to the longest a call may.
pub const QUICK_DEADLINE: Duration = Duration::from_secs(10);
/// A wait's own length, and the margin the transport waits past it.
pub const WAIT: Duration = Duration::from_secs(LONGEST_WAIT_SECS);
pub const SNAPSHOT_DEADLINE: Duration = Duration::from_secs(LONGEST_WAIT_SECS + 10);

/// What the desk may ask of the shell. Blocking, with a deadline each: from a job, never the UI
/// thread.
pub trait MindDeskApi: Send + Sync {
    /// The snapshot, once it is not the one tagged `since`, or when the wait ends with the same.
    fn snapshot(&self, since: Option<u64>) -> Result<DeskSnapshot, ServiceError>;
    /// Ask the person whether to stop `agent`. `Ok` says the shell has put the question on their
    /// desktop, naming the agent: **not** that anything stopped. An `Err` is a question that was
    /// not asked.
    fn ask_stop(&self, agent: &str) -> Result<StopAck, ServiceError>;
    /// Put the person's Agents screen up.
    fn open_agents(&self) -> Result<(), ServiceError>;
}

/// The live shell.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindDesk;

/// One `app.act`, and the `result` of its answer.
fn act(action: &str, args: serde_json::Value, deadline: Duration) -> Result<serde_json::Value, ServiceError> {
    let reply = SyncRpcClient::for_service(SERVICE)
        .with_timeout(deadline)
        .call("app.act", json!({ "action": action, "args": args }))
        .map_err(|e| classify(SERVICE, deadline, e))?;
    match reply.get("result") {
        Some(result) if result.is_object() => Ok(result.clone()),
        _ => Err(ServiceError::WrongShape { service: SERVICE, what: "has no result".to_string() }),
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, ServiceError> {
    serde_json::from_value(value).map_err(|e| ServiceError::WrongShape { service: SERVICE, what: shape_of(&e) })
}

impl MindDeskApi for MindDesk {
    fn snapshot(&self, since: Option<u64>) -> Result<DeskSnapshot, ServiceError> {
        let mut args = json!({ "wait_seconds": WAIT.as_secs() });
        if let Some(tag) = since {
            args["since"] = json!(tag);
        }
        decode(act(SNAPSHOT_ACTION, args, SNAPSHOT_DEADLINE)?)
    }

    fn ask_stop(&self, agent: &str) -> Result<StopAck, ServiceError> {
        decode(act(STOP_ACTION, json!({ "agent": agent }), QUICK_DEADLINE)?)
    }

    fn open_agents(&self) -> Result<(), ServiceError> {
        act(AGENTS_ACTION, json!({}), QUICK_DEADLINE).map(|_| ())
    }
}

/// A shell for tests: it answers what it was given, in order, and records what it was asked.
/// With nothing left to answer it is not running, so a test that has not queued an answer does
/// not poll forever.
#[derive(Debug, Default)]
pub struct FakeMindDesk {
    snapshots: Mutex<VecDeque<Result<DeskSnapshot, ServiceError>>>,
    stops: Mutex<VecDeque<Result<StopAck, ServiceError>>>,
    opened: Mutex<VecDeque<Result<(), ServiceError>>>,
    asked: Mutex<Vec<Option<u64>>>,
    asked_to_stop: Mutex<Vec<String>>,
    showed: Mutex<usize>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl FakeMindDesk {
    pub fn new() -> FakeMindDesk {
        FakeMindDesk::default()
    }

    /// The next snapshot asked for is this.
    pub fn answering(self, snapshot: DeskSnapshot) -> FakeMindDesk {
        self.then(Ok(snapshot))
    }

    /// The next snapshot asked for is this answer.
    pub fn then(self, answer: Result<DeskSnapshot, ServiceError>) -> FakeMindDesk {
        lock(&self.snapshots).push_back(answer);
        self
    }

    /// Queue another snapshot, on a fake a test already shares.
    pub fn push(&self, answer: Result<DeskSnapshot, ServiceError>) {
        lock(&self.snapshots).push_back(answer);
    }

    /// The next stop question is answered with this.
    pub fn stop_answers(&self, answer: Result<StopAck, ServiceError>) {
        lock(&self.stops).push_back(answer);
    }

    /// The next showing is answered with this.
    pub fn show_answers(&self, answer: Result<(), ServiceError>) {
        lock(&self.opened).push_back(answer);
    }

    /// The `since` of every snapshot asked for.
    pub fn asked(&self) -> Vec<Option<u64>> {
        lock(&self.asked).clone()
    }

    /// Every agent a stop was asked about.
    pub fn asked_to_stop(&self) -> Vec<String> {
        lock(&self.asked_to_stop).clone()
    }

    /// How many times the Agents screen was asked for.
    pub fn showed(&self) -> usize {
        *lock(&self.showed)
    }
}

impl MindDeskApi for FakeMindDesk {
    fn snapshot(&self, since: Option<u64>) -> Result<DeskSnapshot, ServiceError> {
        lock(&self.asked).push(since);
        lock(&self.snapshots).pop_front().unwrap_or(Err(ServiceError::Unreachable { service: SERVICE }))
    }

    fn ask_stop(&self, agent: &str) -> Result<StopAck, ServiceError> {
        lock(&self.asked_to_stop).push(agent.to_string());
        lock(&self.stops).pop_front().unwrap_or(Err(ServiceError::Unreachable { service: SERVICE }))
    }

    fn open_agents(&self) -> Result<(), ServiceError> {
        *lock(&self.showed) += 1;
        lock(&self.opened).pop_front().unwrap_or(Err(ServiceError::Unreachable { service: SERVICE }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desk_may_ask_three_things_and_none_of_them_decides_for_the_person() {
        // The trait is the whole of what the desk can ask. A method added to it has to be added
        // here, where the person's decisions are named as the things it must never be.
        let methods = ["snapshot", "ask_stop", "open_agents"];
        let source = include_str!("mind_desk.rs");
        let trait_text = source.split("pub trait MindDeskApi").nth(1).unwrap().split("\n}\n").next().unwrap();
        let declared: Vec<&str> = trait_text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("fn "))
            .filter_map(|l| l.split('(').next())
            .collect();
        assert_eq!(declared, methods);
        for forbidden in ["approve", "deny", "grant", "allow", "take_over", "hand_back", "decide"] {
            assert!(!declared.iter().any(|m| m.contains(forbidden)), "{forbidden}");
        }
    }

    #[test]
    fn the_fake_answers_in_order_and_is_not_running_when_it_has_nothing_to_say() {
        let fake = FakeMindDesk::new().answering(DeskSnapshot { tag: 1, ..Default::default() });
        assert_eq!(fake.snapshot(None).unwrap().tag, 1);
        assert_eq!(fake.snapshot(Some(1)), Err(ServiceError::Unreachable { service: SERVICE }));
        assert_eq!(fake.asked(), [None, Some(1)]);
        assert!(fake.ask_stop("pi:main").is_err());
        assert_eq!(fake.asked_to_stop(), ["pi:main"]);
        fake.stop_answers(Ok(StopAck { agent: "pi:main".into() }));
        assert_eq!(fake.ask_stop("pi:main").unwrap().agent, "pi:main");
        assert!(fake.open_agents().is_err());
        fake.show_answers(Ok(()));
        assert!(fake.open_agents().is_ok());
        assert_eq!(fake.showed(), 2);
    }
}
