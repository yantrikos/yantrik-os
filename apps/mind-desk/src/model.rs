//! What the desk is, and every change to it.
//!
//! The model holds the last snapshot the shell gave, how the desk stands with the shell, and
//! where a Stop is. It holds no Mind's data beyond that snapshot, which has no field for an
//! argument, a path or a prompt (`yantrik_ipc_contracts::mind_desk`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use yantrik_app::timers::TimerId;
use yantrik_ipc_contracts::mind_desk::{DeskSnapshot, StopAck};
use yantrik_services::mind_desk::MindDeskApi;
use yantrik_services::ServiceError;

/// The pause before the shell is asked again after it did not answer, doubling to the longest.
pub const RETRY_FROM: Duration = Duration::from_secs(3);
pub const RETRY_UP_TO: Duration = Duration::from_secs(30);
/// An answer that came back faster than this, saying what the desk already had, is not a wait
/// that ran its course: the shell is asked again after [`SLOW`], and not at once, so a shell that
/// answers without waiting is not a loop.
pub const QUICK: Duration = Duration::from_secs(2);
pub const SLOW: Duration = Duration::from_secs(5);

/// The one timer: the next ask of a shell that did not answer, or answered too quickly.
pub const ASK_AGAIN: TimerId = TimerId("mind-desk.ask-again");

/// A line about the Agents screen not opening is about the moment it was pressed, and is let go
/// after this long: a card that held the screen may be answered, and nothing in a snapshot says so.
/// A one-shot, armed by the press and by nothing else.
pub const NOTE_GONE: TimerId = TimerId("mind-desk.note-gone");
pub const NOTE_FOR: Duration = Duration::from_secs(6);

/// How the desk stands with the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The first ask is out.
    Connecting,
    /// The shell answered the last ask.
    Live,
    /// It did not. The last snapshot is kept but not trusted: it is said to be old.
    Lost,
}

/// Where a Stop the desk asked for is, on the desk's side. The desk stops nothing: it asks the
/// shell, which asks the person on their own desktop, and everything after that arrives in a
/// snapshot (`DeskMind::stop`). "Stopped" is said from there and from nowhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Nothing asked.
    Resting,
    /// Stop… was pressed and the shell has not answered.
    Asking { agent: String },
    /// The shell would not take the question. Nothing is said to have happened to the Mind.
    /// `kept_going` when the person answered the last one "Keep going": the shell does not put the
    /// same question in front of them again for two minutes (`approvals::DENIAL_QUIET`).
    NotTaken { agent: String, kept_going: bool },
}

pub struct Model {
    pub desk: Arc<dyn MindDeskApi>,
    pub link: Link,
    /// The last the shell said, kept through a lost link.
    pub snapshot: Option<DeskSnapshot>,
    pub stop: Stop,
    /// An ask of the shell is out.
    pub asking: bool,
    /// When it was made.
    pub asked_at: Option<Instant>,
    /// The pause the next failed ask waits.
    pub retry: Duration,
    /// Said beside the buttons when the Agents screen could not be put up.
    pub note: String,
}

/// Every change to the desk.
#[derive(Clone, Debug)]
pub enum Command {
    /// Ask the shell for what changed.
    Ask,
    /// What the shell said.
    Heard(Result<DeskSnapshot, ServiceError>),
    /// Stop… was pressed: the shell is asked to ask the person, on their desktop.
    StopAsked,
    /// What the shell said to that.
    StopAnswered { agent: String, answer: Result<StopAck, ServiceError> },
    /// Open in Agents was pressed.
    AgentsAsked,
    /// What the shell said to that.
    AgentsAnswered(Result<(), ServiceError>),
    /// The line about the Agents screen has been said long enough.
    NoteGone,
    /// A window was asked to close. It is not an app a person closes: nothing happens.
    CloseAsked,
}
