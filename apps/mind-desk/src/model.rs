//! What the desk is, and every change to it.
//!
//! The model holds the last snapshot the shell gave, how the desk stands with the shell, and
//! where a Stop is. It holds no Mind's data beyond that snapshot, which has no field for an
//! argument, a path or a prompt (`yantrik_ipc_contracts::mind_desk`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use yantrik_app::app::WindowId;
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

/// Where a Stop is. It claims "Stopped" only from [`Stop::Stopped`], which only the shell's own
/// acknowledgement reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Nothing asked.
    Resting,
    /// The person pressed Stop…, and has not yet said yes.
    Confirming { agent: String },
    /// Yes was said and the shell has not answered.
    Stopping { agent: String },
    /// The shell acknowledged it stopped `agent`, whose state changed last at `since`.
    Stopped { agent: String, since: u64 },
    /// The shell did not acknowledge it. Nothing is claimed: the Mind may or may not have stopped.
    NotConfirmed { agent: String },
}

pub struct Model {
    pub desk: Arc<dyn MindDeskApi>,
    pub link: Link,
    /// The last the shell said, kept through a lost link.
    pub snapshot: Option<DeskSnapshot>,
    pub stop: Stop,
    /// The footer's window, once opened.
    pub bottom: Option<WindowId>,
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
    /// The footer's window is opened.
    OpenFooter,
    /// A window `OpenFooter` asked for could not be made.
    FooterNotOpened,
    /// Ask the shell for what changed.
    Ask,
    /// What the shell said.
    Heard(Result<DeskSnapshot, ServiceError>),
    /// Stop… was pressed.
    StopAsked,
    /// Keep going was pressed.
    StopCancelled,
    /// Stop was pressed on the confirmation.
    StopConfirmed,
    /// What the shell said to the stop.
    StopAnswered { agent: String, answer: Result<StopAck, ServiceError> },
    /// Open in Agents was pressed.
    AgentsAsked,
    /// What the shell said to that.
    AgentsAnswered(Result<(), ServiceError>),
    /// A window was asked to close. It is not an app a person closes: nothing happens.
    CloseAsked,
}
