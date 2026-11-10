//! What the bar says, as plain data: the model, put into the words of
//! [`crate::words`]. `view` writes these to the windows and `describe` reads them back to a Mind,
//! so what is drawn and what is said are one thing.

use yantrik_ipc_contracts::mind_desk::{DeskMind, StopStatus};

use crate::model::{Link, Model, Stop};
use crate::words;

/// The header and the tool strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Top {
    pub label: String,
    pub tone: i32,
    pub headline: String,
    /// Stop… is drawn.
    pub can_stop: bool,
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
pub struct Footer {
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
        can_stop: false,
        stop_note: String::new(),
        agents_offered: false,
        strip_heading: String::new(),
        strip_tool: String::new(),
        strip_mark: String::new(),
        strip_stats: String::new(),
    }
}

/// What the person is told of a stop. Nothing until one is asked; "Stopped" only when the shell
/// says it stopped the Mind, which it does only after the person's yes on their own desktop.
///
/// What was last pressed comes first: a stop being asked, or the shell not taking it, then what
/// the person was told about the Agents screen (which says why it did not open, whatever else is
/// waiting), and last how a stop already asked stands.
fn stop_note(m: &Model, mind: &DeskMind) -> String {
    match (&m.stop, mind.stop) {
        (Stop::Asking { agent }, _) if *agent == mind.agent => "Asking…".to_string(),
        (Stop::NotTaken { agent, kept_going: true }, _) if *agent == mind.agent => {
            "You kept it going; it can be asked again in two minutes".to_string()
        }
        (Stop::NotTaken { agent, .. }, _) if *agent == mind.agent => "The shell did not take the request".to_string(),
        _ if !m.note.is_empty() => m.note.clone(),
        (_, Some(StopStatus::Asked)) => "Waiting for your answer on the desktop".to_string(),
        (_, Some(StopStatus::Stopped)) => "Stopped".to_string(),
        (_, Some(StopStatus::NotStopped)) => "Not stopped".to_string(),
        _ => String::new(),
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
    let strip = words::strip(mind);
    // Stop… is there for a Mind at work that nobody has been asked about, while no ask is out.
    let resting = matches!(&m.stop, Stop::Resting | Stop::NotTaken { .. }) && mind.stop != Some(StopStatus::Asked);
    Top {
        label: words::label(mind),
        tone: words::tone_of(mind.state).index(),
        headline: words::headline(mind),
        // A Mind that is not at work has nothing to stop, and one whose connection is lost has no
        // backend to stop it with.
        can_stop: mind.state.working() && resting,
        stop_note: stop_note(m, mind),
        agents_offered: true,
        strip_heading: strip.heading,
        strip_tool: strip.tool,
        strip_mark: strip.mark,
        strip_stats: words::stats(mind, &snapshot.as_of),
    }
}

pub fn footer(m: &Model) -> Footer {
    let live = if m.link == Link::Live { m.snapshot.as_ref() } else { None };
    let mind = live.and_then(|s| s.mind.as_ref());
    let account = mind.map_or(yantrik_ipc_contracts::mind_desk::Account::Unknown, |mind| mind.account);
    let (model, model_mark) = words::model_line(mind.and_then(|mind| mind.model.as_deref()));
    Footer {
        // What is not known is said so, and not filled in from a snapshot that may be old.
        apps: live.map_or_else(|| "Apps unavailable".to_string(), |s| words::apps_line(&s.apps)),
        safety: words::safety_line(account).to_string(),
        model,
        model_mark: model_mark.to_string(),
    }
}
