//! The only code that changes the model.
//!
//! The desk asks the shell for what changed, and the shell answers when something has (or after
//! its wait), so there is no timer while all is well: one ask is out, blocked on a socket. A timer
//! exists only to ask again after a failure, or after an answer that did not wait.
//!
//! Stop stops nothing. It asks the shell to ask the person, on their own desktop, and "Stopped" is
//! said only from the shell's word in a snapshot that it did.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;
use yantrik_ipc_contracts::mind_desk::{DeskSnapshot, StopStatus};
use yantrik_services::ServiceError;

use crate::model::{Command, Link, Model, Stop, ASK_AGAIN, QUICK, RETRY_FROM, RETRY_UP_TO, SLOW};
use crate::MindDesk;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<MindDesk>) -> Result<(), Refused> {
    match command {
        Command::Ask => ask(m, fx),
        Command::Heard(Ok(snapshot)) => heard(m, snapshot, fx),
        Command::Heard(Err(why)) => lost(m, &why, fx),
        Command::StopAsked => {
            let mind = m.snapshot.as_ref().and_then(|s| s.mind.as_ref()).filter(|_| m.link == Link::Live);
            let Some(mind) = mind.filter(|mind| mind.state.working() && mind.stop != Some(StopStatus::Asked)) else {
                return Err(Refused::new("nothing_to_stop", "No Mind is at work, or the person is already being asked."));
            };
            if matches!(m.stop, Stop::Asking { .. }) {
                return Err(Refused::new("asking", "Stop was asked a moment ago."));
            }
            let agent = mind.agent.clone();
            m.stop = Stop::Asking { agent: agent.clone() };
            m.note.clear();
            let desk = m.desk.clone();
            fx.spawn(move |_| Command::StopAnswered { answer: desk.ask_stop(&agent), agent });
        }
        Command::StopAnswered { agent, answer } => {
            if m.stop != (Stop::Asking { agent: agent.clone() }) {
                fx.unchanged();
                return Ok(());
            }
            // The shell naming the agent says the person has been asked, and nothing more: what
            // comes of it arrives in a snapshot.
            m.stop = match answer {
                Ok(ack) if ack.agent == agent => Stop::Resting,
                Err(ServiceError::Refused { sentence, .. }) if sentence.contains("the person denied") => {
                    Stop::NotTaken { agent, kept_going: true }
                }
                _ => Stop::NotTaken { agent, kept_going: false },
            };
        }
        Command::AgentsAsked => {
            m.note.clear();
            let desk = m.desk.clone();
            fx.spawn(move |_| Command::AgentsAnswered(desk.open_agents()));
        }
        Command::AgentsAnswered(Ok(())) => fx.unchanged(),
        Command::AgentsAnswered(Err(why)) => m.note = agents_failed(&why).to_string(),
        Command::CloseAsked => fx.unchanged(),
    }
    Ok(())
}

/// What is said when the shell would not put the Agents screen up. The shell holds every window
/// it would move while an approval card waits for the person (`card_watch::hold_windows`), which
/// is the common reason when a Mind is waiting for them: the card is already on the desktop, and
/// the person is told so. The shell's own sentence is not shown: it says more than a strip can.
fn agents_failed(why: &ServiceError) -> &'static str {
    match why {
        ServiceError::Refused { sentence, .. } if sentence.contains("approval card is waiting") => {
            "An approval card is waiting on your desktop"
        }
        ServiceError::Refused { sentence, .. } if sentence.contains("at most once") => "Agents was opened a moment ago",
        ServiceError::Refused { sentence, .. } if sentence.contains("in front") => "Agents opens while Mind View is in front",
        _ => "Agents did not open",
    }
}

/// Ask the shell for what changed since the snapshot held, unless an ask is out.
fn ask(m: &mut Model, fx: &mut Effects<MindDesk>) {
    if m.asking {
        fx.unchanged();
        return;
    }
    m.asking = true;
    m.asked_at = Some(fx.now());
    let desk = m.desk.clone();
    let since = m.snapshot.as_ref().map(|s| s.tag);
    fx.spawn(move |_| Command::Heard(desk.snapshot(since)));
}

fn heard(m: &mut Model, snapshot: DeskSnapshot, fx: &mut Effects<MindDesk>) {
    m.asking = false;
    m.retry = RETRY_FROM;
    let same = m.snapshot.as_ref().is_some_and(|s| s.tag == snapshot.tag);
    // A line about the Agents screen is about what was pressed; a stop that has come out another
    // way is news that replaces it.
    let stop_of = |s: &DeskSnapshot| s.mind.as_ref().and_then(|mind| mind.stop);
    if m.snapshot.as_ref().map(stop_of) != Some(stop_of(&snapshot)) {
        m.note.clear();
    }
    let quick = m.asked_at.is_some_and(|at| fx.now().saturating_duration_since(at) < QUICK);
    let drawn_the_same = same && m.link == Link::Live;
    m.link = Link::Live;
    m.snapshot = Some(snapshot);
    settle_stop(m);
    // At once, for the next change; slowly after an answer that said nothing new without waiting.
    if same && quick {
        fx.after(ASK_AGAIN, SLOW, Command::Ask);
    } else {
        ask(m, fx);
    }
    if drawn_the_same {
        // The wait ran its course with the same words: nothing on the screen moves.
        fx.unchanged();
    }
}

fn lost(m: &mut Model, _why: &ServiceError, fx: &mut Effects<MindDesk>) {
    m.asking = false;
    m.link = Link::Lost;
    // A question already asked is not taken back: its own answer, or its own deadline, says how it
    // came out.
    fx.after(ASK_AGAIN, m.retry, Command::Ask);
    m.retry = (m.retry * 2).min(RETRY_UP_TO);
}

/// What a new snapshot does to a Stop in progress: a question the shell could not take is for the
/// Mind that was showing, and is let go when another is, or when it is no longer at work.
fn settle_stop(m: &mut Model) {
    let mind = m.snapshot.as_ref().and_then(|s| s.mind.as_ref());
    let current = mind.map(|mind| (mind.agent.as_str(), mind.state.working()));
    m.stop = match std::mem::replace(&mut m.stop, Stop::Resting) {
        // The answer is still to come; it is for this agent, whatever the desk shows meanwhile.
        asking @ Stop::Asking { .. } => asking,
        Stop::NotTaken { agent, kept_going } if current.is_some_and(|(a, working)| a == agent && working) => {
            Stop::NotTaken { agent, kept_going }
        }
        _ => Stop::Resting,
    };
}
