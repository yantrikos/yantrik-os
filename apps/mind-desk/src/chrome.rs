//! What each of the two windows says, as plain data: the model, put into the words of
//! [`crate::words`]. `view` writes these to the windows and `describe` reads them back to a Mind,
//! so what is drawn and what is said are one thing.

use yantrik_ipc_contracts::mind_desk::DeskMind;

use crate::model::{Link, Model, Stop};
use crate::words;

/// The header and the tool strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Top {
    pub label: String,
    pub tone: i32,
    pub headline: String,
    pub quoted: bool,
    pub reported: String,
    /// Stop… is drawn.
    pub can_stop: bool,
    /// The question is asked, with Stop and Keep going.
    pub confirming: bool,
    pub confirm_question: String,
    /// What is said of a stop, or of the Agents screen not opening.
    pub stop_note: String,
    /// Open in Agents is drawn.
    pub agents_offered: bool,
    pub strip_heading: String,
    pub strip_tool: String,
    pub strip_mark: String,
    pub strip_stats: String,
}

/// The footer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bottom {
    pub apps: String,
    pub safety: String,
    pub model: String,
    pub model_mark: String,
}

fn quiet(label: &str, headline: &str) -> Top {
    Top {
        label: label.to_string(),
        tone: words::Tone::Quiet.index(),
        headline: headline.to_string(),
        quoted: false,
        reported: String::new(),
        can_stop: false,
        confirming: false,
        confirm_question: String::new(),
        stop_note: String::new(),
        agents_offered: false,
        strip_heading: String::new(),
        strip_tool: String::new(),
        strip_mark: String::new(),
        strip_stats: String::new(),
    }
}

/// What the person is told of a stop: nothing until one is asked, and "Stopped" only once the
/// shell has said it stopped the Mind.
fn stop_note(m: &Model, mind: &DeskMind) -> String {
    match &m.stop {
        Stop::Stopping { agent } if *agent == mind.agent => "Stopping…".to_string(),
        Stop::Stopped { agent, .. } if *agent == mind.agent => "Stopped".to_string(),
        Stop::NotConfirmed { agent } if *agent == mind.agent => "The shell did not confirm the stop".to_string(),
        _ => m.note.clone(),
    }
}

pub fn top(m: &Model) -> Top {
    if m.link == Link::Lost {
        return quiet("MIND VIEW · NOT CONNECTED", "The shell is not answering");
    }
    let Some(snapshot) = &m.snapshot else {
        return quiet("MIND VIEW · CONNECTING", "Connecting to the shell");
    };
    let Some(mind) = &snapshot.mind else {
        let mut idle = quiet("MIND VIEW · IDLE", "No Mind is at work here");
        idle.agents_offered = true;
        idle.strip_heading = "NO TOOL YET".to_string();
        return idle;
    };
    let said = words::headline(mind);
    let strip = words::strip(mind);
    let confirming = matches!(&m.stop, Stop::Confirming { agent } if *agent == mind.agent);
    let resting = matches!(&m.stop, Stop::Resting | Stop::NotConfirmed { .. });
    Top {
        label: words::label(mind),
        tone: words::tone_of(mind.state).index(),
        headline: said.text,
        quoted: said.quoted,
        reported: said.reported,
        // A Mind that is not at work has nothing to stop, and one whose connection is lost has no
        // backend to stop it with.
        can_stop: mind.state.working() && resting,
        confirming,
        confirm_question: if confirming { format!("Stop {}?", yantrik_ipc_contracts::mind_desk::display_name(&mind.name)) } else { String::new() },
        stop_note: stop_note(m, mind),
        agents_offered: true,
        strip_heading: strip.heading,
        strip_tool: strip.tool,
        strip_mark: strip.mark,
        strip_stats: words::stats(mind, &snapshot.as_of),
    }
}

pub fn bottom(m: &Model) -> Bottom {
    let live = if m.link == Link::Live { m.snapshot.as_ref() } else { None };
    let mind = live.and_then(|s| s.mind.as_ref());
    let account = mind.map_or(yantrik_ipc_contracts::mind_desk::Account::Unknown, |mind| mind.account);
    let (model, model_mark) = words::model_line(mind.and_then(|mind| mind.model.as_deref()));
    Bottom {
        // What is not known is said so, and not filled in from a snapshot that may be old.
        apps: live.map_or_else(|| "Apps unavailable".to_string(), |s| words::apps_line(&s.apps)),
        safety: words::safety_line(account).to_string(),
        model,
        model_mark: model_mark.to_string(),
    }
}
