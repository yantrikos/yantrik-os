//! The only code that changes the model.
//!
//! The desk asks the shell for what changed, and the shell answers when something has (or after
//! its wait), so there is no timer while all is well: one ask is out, blocked on a socket. A timer
//! exists only to ask again after a failure, or after an answer that did not wait.
//!
//! Stop claims nothing the shell has not said. "Stopped" is reached from the shell's own
//! acknowledgement, naming the agent it stopped, and from nothing else.

use yantrik_app::effects::Effects;
use yantrik_app::Refused;
use yantrik_ipc_contracts::mind_desk::DeskSnapshot;
use yantrik_services::ServiceError;

use crate::model::{Command, Link, Model, Stop, ASK_AGAIN, QUICK, RETRY_FROM, RETRY_UP_TO, SLOW};
use crate::MindDesk;

pub fn update(m: &mut Model, command: Command, fx: &mut Effects<MindDesk>) -> Result<(), Refused> {
    match command {
        Command::OpenFooter => {
            if m.bottom.is_none() {
                m.bottom = Some(fx.open_window());
            }
        }
        Command::FooterNotOpened => m.bottom = None,
        Command::Ask => ask(m, fx),
        Command::Heard(Ok(snapshot)) => heard(m, snapshot, fx),
        Command::Heard(Err(why)) => lost(m, &why, fx),
        Command::StopAsked => {
            let mind = m.snapshot.as_ref().and_then(|s| s.mind.as_ref()).filter(|_| m.link == Link::Live);
            let Some(mind) = mind.filter(|mind| mind.state.working()) else {
                return Err(Refused::new("nothing_to_stop", "No Mind is at work, so there is nothing to stop."));
            };
            m.stop = Stop::Confirming { agent: mind.agent.clone() };
            m.note.clear();
        }
        Command::StopCancelled => m.stop = Stop::Resting,
        Command::StopConfirmed => {
            let Stop::Confirming { agent } = m.stop.clone() else {
                return Err(Refused::new("not_confirming", "Stop was not asked for."));
            };
            m.stop = Stop::Stopping { agent: agent.clone() };
            let desk = m.desk.clone();
            fx.spawn(move |_| Command::StopAnswered { answer: desk.stop(&agent), agent });
        }
        Command::StopAnswered { agent, answer } => {
            if m.stop != (Stop::Stopping { agent: agent.clone() }) {
                // Not the stop this desk is waiting on: nothing is said of it.
                fx.unchanged();
                return Ok(());
            }
            m.stop = match answer {
                // The shell named the agent it stopped: that is the acknowledgement.
                Ok(ack) if ack.agent == agent => {
                    let since = m.snapshot.as_ref().and_then(|s| s.mind.as_ref()).map_or(0, |mind| mind.since);
                    Stop::Stopped { agent, since }
                }
                // Anything else is a stop nobody confirmed, and nothing is claimed about it.
                _ => Stop::NotConfirmed { agent },
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
    // A confirmation is for a Mind the desk can no longer see. A stop already asked is not
    // taken back: its own answer, or its own deadline, will say how it came out.
    if matches!(m.stop, Stop::Confirming { .. }) {
        m.stop = Stop::Resting;
    }
    fx.after(ASK_AGAIN, m.retry, Command::Ask);
    m.retry = (m.retry * 2).min(RETRY_UP_TO);
}

/// What a new snapshot does to a Stop in progress: a confirmation is for the Mind that was
/// showing, and a stop that was acknowledged is said until that Mind is at work again.
fn settle_stop(m: &mut Model) {
    let mind = m.snapshot.as_ref().and_then(|s| s.mind.as_ref());
    let current = mind.map(|mind| (mind.agent.as_str(), mind.state.working(), mind.since));
    m.stop = match std::mem::replace(&mut m.stop, Stop::Resting) {
        Stop::Confirming { agent } if current.is_some_and(|(a, working, _)| a == agent && working) => Stop::Confirming { agent },
        Stop::Confirming { .. } => Stop::Resting,
        // The answer is still to come; it is for this agent, whatever the desk shows meanwhile.
        stopping @ Stop::Stopping { .. } => stopping,
        Stop::Stopped { agent, since } if current.is_some_and(|(a, working, at)| a == agent && !(working && at != since)) => {
            Stop::Stopped { agent, since }
        }
        Stop::NotConfirmed { agent } if current.is_some_and(|(a, working, _)| a == agent && working) => Stop::NotConfirmed { agent },
        _ => Stop::Resting,
    };
}
