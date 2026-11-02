//! What an agent is doing, as the list and the tabs read it.

use serde::{Deserialize, Serialize};

/// What an agent is doing, as the list and the tabs read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// A turn is open and the mind is producing text or thinking.
    Thinking,
    /// A turn is open and at least one call is running.
    RunningTool,
    /// The agent cannot go on until the person answers something: an approval, a password.
    WaitingForYou,
    /// Known, and nothing is running: not started yet, or picked up again after a restart.
    Idle,
    /// Its last turn finished.
    Done,
    /// Its last turn failed.
    Failed,
    /// The person stopped its turn and the harness has not yet said it heard: it may still be
    /// working. Settled to one of the next two by the host's word (`agents/stopped.rs`).
    Stopping,
    /// The person stopped its turn and the harness closed it: the one word that it stopped. At
    /// rest; the agent stays and takes the next turn.
    Stopped,
    /// The person stopped its turn, and the harness has not closed it within the grace (it does
    /// not heartbeat, or is gone): the desktop stopped waiting, and does not know it stopped.
    /// Upgraded to `Stopped` if the harness closes the turn later.
    StoppedWaiting,
    /// The harness that ran it stopped polling (design: "When a harness dies").
    HarnessGone,
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            State::Thinking => "thinking",
            State::RunningTool => "running_tool",
            State::WaitingForYou => "waiting_for_you",
            State::Idle => "idle",
            State::Done => "done",
            State::Failed => "failed",
            State::Stopping => "stopping",
            State::Stopped => "stopped",
            State::StoppedWaiting => "stopped_waiting",
            State::HarnessGone => "harness_gone",
        }
    }

    /// How the list says it.
    pub fn label(self) -> &'static str {
        match self {
            State::Thinking => "thinking",
            State::RunningTool => "running a tool",
            State::WaitingForYou => "waiting for you",
            State::Idle => "idle",
            State::Done => "done",
            State::Failed => "failed",
            State::Stopping => "stopping",
            State::Stopped => "stopped",
            State::StoppedWaiting => "stopped waiting",
            State::HarnessGone => "harness gone",
        }
    }

    /// Doing something right now — the states a Close has to ask about.
    pub fn working(self) -> bool {
        matches!(self, State::Thinking | State::RunningTool | State::WaitingForYou | State::Stopping)
    }

    /// Still able to move on its own or on the next prompt: the Active tab.
    pub fn live(self) -> bool {
        self.working() || matches!(self, State::Idle | State::Stopped | State::StoppedWaiting)
    }
}
