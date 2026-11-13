//! The Lens's Stop (#1058): the running turn of the mind answering, dropped on the person's word.
//!
//! The harness does not poll for new turns while one runs. It sends an empty chunk every
//! heartbeat (30 s) and ends the turn when the desktop answers one "dropped". So Stop only has to
//! mark the turn dropped (`launch::stop_turn`, the host's one drop path, which the Mind desk's stop
//! also ends in), and the harness stops by itself within a heartbeat. Until it does, the Lens says
//! "Stopping…". That is all it can promise: a harness that does not stream sends no heartbeat, so
//! after the grace period ("Stopping…" ends) the desktop has stopped waiting and ended what it
//! started, and cannot say the harness has stopped. The line under a Stop says so.
//!
//! Two doors, one act: the Stop button, and a bare "stop" typed while a turn is in flight
//! (`lens_queue::is_bare_stop`). A typed stop is consumed here. It used to be held as a new turn
//! and delivered after the turn it was meant to end, and the mind answered it as a greeting.

use std::sync::atomic::{AtomicU64, Ordering};

use yantrik_harness::Host;

use crate::agents::{launch, model::AgentId, model::Question};
use crate::{streaming, App};

/// What the desktop says once the turn is dropped. It says what is known: the turn is no longer
/// waited on and what the desktop started for it is ended. When the mind hears of it depends on
/// the mind: one that streams, within half a minute; one that does not, at its next call, which
/// may be much later, and until then anything it runs on its own may carry on.
const STOPPED_LINE: &str = concat!(
    "Stopped waiting for it, and ended what it started here. A mind that streams hears it within ",
    "half a minute; one that does not hears it at its next call, which may be later. Until it does, ",
    "anything it runs on its own may carry on. If more than one message was waiting on it, they ",
    "were stopped too."
);

/// Added when a message the person had queued in the Lens behind the turn went with it.
const QUEUED_LINE: &str = " The message you had queued was not sent.";

/// A second Stop while the first waits for the mind to hear it.
const ALREADY_LINE: &str =
    "Already stopping. The mind has not confirmed it yet, so anything it runs on its own may still carry on.";

/// The turn the person saw has ended, but the Agents store still shows a turn open: another has
/// begun, or this one is closing. Nothing was stopped, and the person is told so, not that
/// nothing ran.
const ENDED_LINE: &str = "That turn had already ended. If another is running, press Stop again to stop it.";

/// There is no turn, in flight, queued or open in the store.
const NOTHING_LINE: &str = "Nothing was running.";

/// What a Stop came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Turns were dropped.
    Stopped,
    /// An earlier Stop is still waiting for the mind to hear it.
    Already,
    /// The turn seen was gone, and the store still shows one open.
    Ended,
    /// Nothing at all was running.
    Nothing,
}

impl Outcome {
    /// What the desktop says; `queued` when a message queued in the Lens was taken back too.
    fn line(self, queued: bool) -> String {
        let line = match self {
            Outcome::Stopped => STOPPED_LINE,
            Outcome::Already => ALREADY_LINE,
            Outcome::Ended => ENDED_LINE,
            Outcome::Nothing => NOTHING_LINE,
        };
        if queued { format!("{line}{QUEUED_LINE}") } else { line.to_string() }
    }
}

/// The turn the person last saw running, as the composer published it (0: none; the host's turn
/// ids start at 1). A Stop acts on the turns up to this one and no later. It can be a tick old; a
/// press over a turn that has been replaced stops nothing and says so (`Outcome::Ended`), because
/// the store still shows the replacement open.
static SEEN: AtomicU64 = AtomicU64::new(0);

/// Record the turn the composer shows as running, for the Stop pressed over it.
pub fn seen(turn: Option<u64>) {
    SEEN.store(turn.unwrap_or(0), Ordering::Relaxed);
}

/// Drop the turns up to `observed`, the newest the person saw, in flight or queued for the
/// harness; and then close the question the mind asked in them, as `/stop` does, so the composer
/// does not stay in Answer mode for a turn that no longer exists. "Nothing was running." is only
/// the answer when the store shows no turn open either.
fn stop_now(host: &Host, agent: &AgentId, observed: Option<u64>) -> Outcome {
    if let Some(turn) = observed {
        // Asked before the drop: which of them the harness holds, for the host to say later
        // whether it heard (`agents/stopped.rs`).
        let handed = host.flights_up_to(agent, turn);
        if launch::stop_turn(host, agent, turn).stopped {
            crate::agents::stopped::note(agent, handed);
            super::lens_queue::close_questions(agent, Question::STOPPED_BY_YOU);
            return Outcome::Stopped;
        }
    }
    if host.stopping(agent) {
        Outcome::Already
    } else if crate::agents::store().read(|s| s.agent(agent).is_some_and(|a| a.open_turn().is_some())) {
        Outcome::Ended
    } else {
        Outcome::Nothing
    }
}

/// Stop the turn of the mind answering in the Lens. `observed` is the newest turn the person saw
/// (the button: the last one published; typed: the composer the word was read against). `typed` is
/// the person's own word when they wrote it instead of pressing the button: it is shown in the
/// conversation, once, and never sent. The desktop says what came of it, and the composer is
/// republished at once so "Stopping..." is there before the next tick.
///
/// Stop means stop: a message the person queued in the Lens behind the turn (`lens_queue`) is
/// taken back with it, never sent when the turn ends.
pub fn press(ui_weak: &slint::Weak<App>, observed: Option<u64>, typed: Option<&str>) {
    let outcome = match super::harness::host() {
        Some(host) => stop_now(host, &crate::agents::feed::main_agent(&host.active_id()), observed),
        None => Outcome::Nothing,
    };
    let queued = ui_weak.upgrade().zip(super::harness::host()).is_some_and(|(ui, host)| {
        super::lens_queue::take_back(&ui, &crate::agents::feed::main_agent(&host.active_id()))
    });
    streaming::say(ui_weak, typed, "desktop", &outcome.line(queued));
    if let Some(ui) = ui_weak.upgrade() {
        super::lens_composer::publish(&ui);
    }
}

/// The button's turn: the one the composer last showed.
pub fn pressed(ui_weak: &slint::Weak<App>) {
    let seen = SEEN.load(Ordering::Relaxed);
    press(ui_weak, (seen != 0).then_some(seen), None);
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yantrik_harness::{protocol, Chunk, Host, Turn};

    use crate::agents::{launch, model::AgentId, model::Provenance, model::Question, reaches, store};

    use super::{stop_now, Outcome, ALREADY_LINE, NOTHING_LINE, STOPPED_LINE};

    /// A host with one attached harness, a turn handed to it, and the agent that turn is for.
    fn running(id: &str) -> (Host, String, AgentId, u64) {
        let host = Host::new(vec![]);
        let attach = json!({ "id": id, "name": id, "conversations": true });
        let session = host.handle(protocol::ATTACH, &attach).unwrap()["session"].as_str().unwrap().to_string();
        let agent = AgentId::new(id, AgentId::MAIN);
        let _answer = host.send_to(&agent, Turn::new("a long task")).unwrap();
        let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        (host, session, agent, turn)
    }

    /// The Lens's Stop button: the turn is dropped, the harness's next heartbeat is answered
    /// "dropped", the agent stays, and the host says "stopping" until the harness closes the turn.
    #[test]
    fn the_stop_button_drops_the_turn_and_the_next_heartbeat_is_told() {
        let host = Host::new(vec![]);
        let attach = json!({ "id": "stop-probe", "name": "Probe", "conversations": false });
        let session = host.handle_from(protocol::ATTACH, &attach, Some(std::process::id()), None).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = AgentId::new("stop-probe", AgentId::MAIN);
        let answer = host.send_to(&agent, Turn::new("a long task")).unwrap();
        let turn = host.handle_from(protocol::POLL, &json!({ "session": session }), Some(std::process::id()), None).unwrap();
        let turn_id = turn["turn_id"].as_u64().expect("handed the turn");
        let beat = || {
            host.handle_from(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "" }), Some(std::process::id()), None)
                .unwrap()
        };
        assert_eq!(beat(), json!({}));
        assert!(!host.stopping(&agent));

        let turn = host.live_turn(&agent).expect("the turn in flight");
        assert!(launch::stop_turn(&host, &agent, turn).stopped);

        assert_eq!(beat(), json!({ "dropped": true }));
        assert!(host.stopping(&agent), "Stopping… until the harness closes it");
        assert!(matches!(answer.iter().last(), Some(Chunk::Failed(_))), "the reader is told it was stopped");
        assert!(host.send_to(&agent, Turn::new("next")).is_ok(), "the agent stays");
    }

    /// M1: the mind's open question is closed with the person's reason, as `/stop` does it, and
    /// the composer leaves Answer mode.
    #[test]
    fn stop_closes_the_question_the_mind_is_waiting_on() {
        let (host, _session, agent, turn) = running("stop-asks");
        store().open_turn(&agent, "clean up Downloads");
        let ask = yantrik_harness::Event::Request {
            request_id: "r1".into(),
            prompt: "Delete 3 old installers?".into(),
            options: vec!["Yes".into(), "No".into()],
            recommended: None,
        };
        store().event(&agent, &ask, Provenance::Reported);
        assert_eq!(store().read(|s| s.agent(&agent).unwrap().waiting_questions().count()), 1);

        assert_eq!(stop_now(&host, &agent, Some(turn)), Outcome::Stopped);

        let closed = store().read(|s| {
            let a = s.agent(&agent).unwrap();
            (a.waiting_questions().count(), a.turns.iter().flat_map(|t| t.items.iter()).find_map(|i| match i {
                crate::agents::model::Item::Question(q) => Some(q.closed.clone()),
                _ => None,
            }))
        });
        assert_eq!(closed, (0, Some(Question::STOPPED_BY_YOU.to_string())));
    }

    /// A Stop over a turn that ended: nothing is stopped, the newer turn runs on, and the line
    /// says the turn ended while the store shows one open, "Nothing was running." only when it
    /// shows none.
    #[test]
    fn stop_over_a_turn_that_is_gone_stops_nothing_and_says_what_is_true() {
        let (host, session, agent, turn) = running("stop-gone");
        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn })).unwrap();
        let _next = host.send_to(&agent, Turn::new("the next one")).unwrap();
        let next = host.live_turn(&agent).unwrap();
        assert!(next > turn);

        assert_eq!(stop_now(&host, &agent, Some(turn)), Outcome::Nothing, "the store has no turn open");
        assert_eq!(host.live_turn(&agent), Some(next), "the newer turn is untouched");
        store().open_turn(&agent, "the next one");
        assert_eq!(stop_now(&host, &agent, Some(turn)), Outcome::Ended, "the store shows a turn open");
        assert_eq!(stop_now(&host, &agent, None), Outcome::Ended, "no turn seen yet, one open");
        assert_eq!(NOTHING_LINE, "Nothing was running.");
    }

    /// A Stop before the harness has collected the turn stops it. The reader is told, the poll
    /// hands nothing over, and the outcome is Stopped, never "Nothing was running.".
    #[test]
    fn stop_before_the_harness_polls_stops_the_queued_turn() {
        let host = Host::new(vec![]);
        let attach = json!({ "id": "stop-queued", "name": "q", "conversations": true });
        let session = host.handle(protocol::ATTACH, &attach).unwrap()["session"].as_str().unwrap().to_string();
        let agent = AgentId::new("stop-queued", AgentId::MAIN);
        let answer = host.send_to(&agent, Turn::new("delete the old backups")).unwrap();
        store().open_turn(&agent, "delete the old backups");
        let seen = host.live_turn(&agent);
        assert!(seen.is_some(), "the Lens would show Stop with this turn");

        assert_eq!(stop_now(&host, &agent, seen), Outcome::Stopped);
        assert!(matches!(answer.iter().last(), Some(Chunk::Failed(_))), "the reader is told it was stopped");
        let polled = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert!(polled.get("turn_id").map_or(true, |v| v.is_null()), "never handed over: {polled}");
    }

    /// A second Stop while the first waits for the mind says it is already stopping.
    #[test]
    fn stop_while_stopping_says_already_stopping() {
        let (host, _session, agent, turn) = running("stop-twice");
        assert_eq!(stop_now(&host, &agent, Some(turn)), Outcome::Stopped);
        assert!(host.stopping(&agent));
        assert_eq!(stop_now(&host, &agent, None), Outcome::Already);
        assert!(ALREADY_LINE.starts_with("Already stopping"));
        assert!(!STOPPED_LINE.contains("  "), "no run of spaces: {STOPPED_LINE}");
        assert!(!STOPPED_LINE.contains("has stopped"), "never claims the mind has stopped");
    }

    /// The typed path: "stop" during "Stopping..." is read as a Stop (the composer sends it to
    /// `press`, not to the mind), and what is said is "Already stopping.".
    #[test]
    fn a_typed_stop_during_stopping_is_a_stop_and_says_already_stopping() {
        use crate::wire::lens_queue::{Composer, Enter};
        let (host, _session, agent, turn) = running("stop-typed");
        assert_eq!(stop_now(&host, &agent, Some(turn)), Outcome::Stopped);
        let composer = Composer { agent: Some(agent.clone()), stopping: host.stopping(&agent), turn_id: host.live_turn(&agent), ..Composer::default() };
        assert_eq!(composer.enter("stop"), Enter::Stop, "typed, while stopping, it is the Stop");
        assert_eq!(composer.enter("Cancel."), Enter::Stop);
        let said = stop_now(&host, &agent, composer.turn_id).line(false);
        assert!(said.starts_with("Already stopping"), "{said}");
    }

    /// Stop means stop, for the mind being stopped and only that one: the message queued behind
    /// its turn is taken back (and so never due), one queued behind another mind is kept. The line
    /// says "not sent", not that the words came back.
    #[test]
    fn stop_takes_back_only_the_stopped_minds_queued_message_and_the_line_says_not_sent() {
        use crate::wire::lens_queue::Queue;
        let hermes = AgentId::new("hermes", AgentId::MAIN);
        let mind = AgentId::new("mind", AgentId::MAIN);

        let mut queue = Queue::default();
        queue.hold("After that, tell me PINEAPPLE.", &mind, "Mind", 7, true, || 3);
        assert!(queue.cancel_for(&hermes).is_none(), "queued behind another mind: kept");
        assert_eq!(queue.due(|_| false, 7).map(|q| q.text), Some("After that, tell me PINEAPPLE.".to_string()), "still due");

        queue.hold("After that, tell me PINEAPPLE.", &mind, "Mind", 7, true, || 3);
        let taken = queue.cancel_for(&mind).expect("held for the mind being stopped");
        assert_eq!(taken.text, "After that, tell me PINEAPPLE.");
        assert_eq!(queue.due(|_| false, 7), None, "never sent when the turn ends");
        assert!(queue.cancel_for(&mind).is_none(), "and nothing the second time");

        let said = Outcome::Stopped.line(true);
        assert!(said.starts_with(STOPPED_LINE) && said.ends_with("was not sent."), "{said}");
        assert!(!said.contains("taken back"), "does not read as returned: {said}");
        assert_eq!(Outcome::Stopped.line(false), STOPPED_LINE, "no queued message: the line is unchanged");
    }

    /// A message typed after the press: while the turn the Stop dropped still reads working it is
    /// queued and sent when it closes; once the store has closed it, it is an ordinary message.
    /// Either way it is not lost, and not taken for a second Stop.
    #[test]
    fn a_message_typed_after_the_press_is_queued_or_sent_never_lost() {
        use crate::wire::lens_queue::{Composer, Enter};
        let agent = AgentId::new("hermes", AgentId::MAIN);
        let base = Composer { agent: Some(agent), stopping: true, ..Composer::default() };
        assert_eq!(Composer { working: true, ..base.clone() }.enter("and Austin?"), Enter::Queue);
        assert_eq!(base.enter("and Austin?"), Enter::Send);
    }

    /// The agent reads `stopping` after a Stop, never `failed`; the next turn works as for any agent.
    #[test]
    fn the_agent_reads_stopping_after_a_stop_until_the_harness_is_heard() {
        use crate::agents::State;
        let agent = AgentId::new("stop-state", AgentId::MAIN);
        store().open_turn(&agent, "a long task");
        store().close_turn_stopped(&agent);
        let (state, ok) = store().read(|s| {
            let a = s.agent(&agent).unwrap();
            (a.state, a.turns.last().and_then(|t| t.ok))
        });
        assert_eq!((state, state.key()), (State::Stopping, "stopping"), "not at rest until the harness is heard");
        assert_eq!(ok, Some(false), "the turn did not finish");
        store().open_turn(&agent, "again");
        assert_ne!(store().read(|s| s.agent(&agent).unwrap().state), State::Stopping);
    }

    /// The Note: a Stop of one turn keeps the agent, so it keeps its reach too. Releasing it left
    /// the next turn unheld: a phone's turn at "sensitive" ran with no ceiling at all.
    #[test]
    fn a_turn_after_stop_still_has_its_reach() {
        let (host, session, agent, turn) = running("stop-reach");
        let token = host.with_agent_token(&agent, |t| t.to_string()).unwrap();
        let held = reaches::hold_remote(&host, &agent).unwrap();
        assert!(reaches::lookup(&token).is_some(), "held while the turn runs");

        assert!(launch::stop_turn(&host, &agent, turn).stopped);
        assert_eq!(host.with_agent_token(&agent, |t| t.to_string()), Some(token.clone()), "the token stays");
        assert!(reaches::lookup(&token).is_some(), "so does the reach it names");

        host.handle(protocol::COMPLETE, &json!({ "session": session, "turn_id": turn })).unwrap();
        let _next = host.send_to(&agent, Turn::new("again")).unwrap();
        let again = reaches::hold_remote(&host, &agent).unwrap();
        assert!(reaches::lookup(&token).is_some(), "the next turn runs under it");
        drop(held);
        drop(again);
        assert!(reaches::lookup(&token).is_none(), "and it ends with the last turn, as before");
    }
}
