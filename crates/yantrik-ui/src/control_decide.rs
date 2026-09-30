//! `decide`: the decision model in use, asked by surfaces in other processes.
//!
//! The browser surface (apps/browser) holds the page and asks whether a press would buy, send or
//! delete something; an agent such as the Mind asks its own quick questions. The model is the
//! companion's, chosen in Settings. This is the door to it. It answers in the verdict's wire form
//! (`docs/decisions.md`): the same shape whichever model answered, abstentions included, so the
//! caller decides as it would without a model whenever one is off, down, switched off for that
//! use, or the desktop is incognito.
//!
//! Each `purpose` is a use from `JUDGE_USES`, with its own switch in Settings. Who may ask for
//! which (security review, 29 Sep 2026):
//! - `browser_commitment` only from the browser service itself: the process the shell started
//!   for it, matched by the pid the kernel gives for the call;
//! - `agent` from anyone, and for an agent only of a model on this machine or the home network,
//!   since a cloud model would be a way to carry out whatever the agent had read that no taint
//!   rule sees (`yantrik_companion::decisions`).
//!
//! An agent is anything `mind_view::requester_now` does not call the person: a token, the mind
//! account, the built-in companion (the shell's own pid), or a process an attached mind started.
//!
//! `safe`: it changes nothing. Off the UI thread, and bounded: every refusal is decided before a
//! place is taken, at most `MAX_AT_ONCE` are asked at a time, and agents hold at most
//! `MAX_FOR_AGENTS` of them, so an agent keeping the model busy cannot starve the browser's check.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use yantrik_app_runtime::control::{self, Action, App as ControlSurface, Param};
use yantrik_companion::config::JUDGE_USES;
use yantrik_companion::decisions::Caller;
use yantrik_shell_core::service_manager::ServiceManager;

use crate::bridge::CompanionHandle;
use crate::mind_view::{requester_now, Requester};

/// How long a caller is kept waiting for the model. The browser gives its own shorter budget
/// on top; past it, it decides with its word list alone.
const WAIT: Duration = Duration::from_secs(8);

/// Largest request accepted, `state` and `questions` together, in bytes of JSON: a control and
/// its page context, not a document.
const MAX_REQUEST: usize = 32 * 1024;

/// Decisions in flight at once, from every caller together, and how many of them agents may hold.
const MAX_AT_ONCE: usize = 4;
const MAX_FOR_AGENTS: usize = 2;
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static AGENTS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// The service whose own process may ask for `browser_commitment`.
const BROWSER_SERVICE: &str = "browser";

/// One place in the `MAX_AT_ONCE`, given back when dropped.
struct Place {
    agent: bool,
}

impl Place {
    fn take(agent: bool) -> Option<Place> {
        let within = |counter: &AtomicUsize, cap: usize| {
            counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| (n < cap).then_some(n + 1)).is_ok()
        };
        if agent && !within(&AGENTS_IN_FLIGHT, MAX_FOR_AGENTS) {
            return None;
        }
        if !within(&IN_FLIGHT, MAX_AT_ONCE) {
            if agent {
                AGENTS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            }
            return None;
        }
        Some(Place { agent })
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        if self.agent {
            AGENTS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Who is calling, for the decision desk.
fn caller_now() -> Caller {
    if control::agent_is_calling() || requester_now() != Requester::Person {
        Caller::Agent
    } else {
        Caller::Person
    }
}

/// Whether the call on this thread comes from the browser service the shell started.
fn from_browser_service(services: &ServiceManager) -> bool {
    let caller = control::caller().and_then(|c| u32::try_from(c.pid).ok()).filter(|p| *p > 0);
    caller.is_some() && caller == services.pid(BROWSER_SERVICE)
}

pub fn actions(surface: ControlSurface, companion: CompanionHandle, services: ServiceManager) -> ControlSurface {
    let purposes: Vec<&'static str> = JUDGE_USES.iter().filter(|u| u.door).map(|u| u.id).collect();
    surface.action(
        Action::new(
            "decide",
            "Put typed questions (noul, choice, score) about a state to the decision model chosen \
             in Settings, for one of its uses, and answer with its verdict. An agent asks for \
             `agent`, and only a model on this machine or the home network answers it",
        )
        .risk("safe")
        .arg(Param::one_of("purpose", &purposes).describe("Which use of the decision model this is"))
        .arg(Param::object("state").describe("What is being judged, as a JSON object"))
        .arg(Param::object("questions").describe("Question id to {type, instructions, criteria?}, as docs/decisions.md")),
        move |args| {
            let caller = caller_now();
            let purpose = args["purpose"].as_str().unwrap_or_default().trim().to_string();
            let state = args["state"].clone();
            let questions = args["questions"].clone();
            if !state.is_object() {
                return Err("`state` is a JSON object".into());
            }
            let size = serde_json::to_string(&state).map(|s| s.len()).unwrap_or(usize::MAX)
                .saturating_add(serde_json::to_string(&questions).map(|s| s.len()).unwrap_or(usize::MAX));
            if size > MAX_REQUEST {
                return Err(format!(
                    "the request is larger than {} KB, state and questions together; send what is judged and its context, not a document",
                    MAX_REQUEST / 1024
                ));
            }
            if purpose == "browser_commitment" && (caller == Caller::Agent || !from_browser_service(&services)) {
                return Err("browser_commitment is the browser service's own check, asked only by it: refused, nothing was sent.".into());
            }
            yantrik_ml::judge::questions_from_json(&questions)?;
            let decisions = companion.decisions().clone();
            // Every refusal before a place is taken: a call that will be refused holds nothing.
            decisions.admit(&purpose, caller)?;
            let place = Place::take(caller == Caller::Agent).ok_or_else(|| {
                "the decision model is already answering as many questions as it may; nothing was sent, ask again shortly".to_string()
            })?;
            let work = move || {
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let _place = place;
                    let _ = tx.send(decisions.ask_json(&state, &questions, &purpose, caller));
                });
                rx.recv_timeout(WAIT)
                    .unwrap_or_else(|_| Err(format!("the decision model did not answer within {} s", WAIT.as_secs())))
            };
            control::answer_later(work)
                .map(|()| serde_json::json!({ "answering": "off the UI thread" }))
                .or_else(|work| work())
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_never_hold_every_place() {
        let agents: Vec<Place> = std::iter::from_fn(|| Place::take(true)).take(MAX_AT_ONCE).collect();
        assert_eq!(agents.len(), MAX_FOR_AGENTS, "agents stop at their share");
        let people: Vec<Place> = std::iter::from_fn(|| Place::take(false)).take(MAX_AT_ONCE).collect();
        assert_eq!(people.len(), MAX_AT_ONCE - MAX_FOR_AGENTS, "and the rest is left for the person's surfaces");
        assert!(Place::take(false).is_none() && Place::take(true).is_none());
        drop(agents);
        drop(people);
        assert!(Place::take(false).is_some(), "places come back when the decisions finish");
    }
}
