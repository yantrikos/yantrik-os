//! Every change to the model is one of these, wherever it came from: a button, the Mind, a timer,
//! or what the engine's desk found.

use serde_json::Value;
use yantrik_app::app::WindowId;
use yantrik_app::rail::RailCommand;

/// Which command to the engine a landing answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Add,
    Pause,
    Resume,
    Cancel,
    Retry,
    Verify,
    OpenFolder,
    PauseAll,
    ResumeAll,
    ClearCompleted,
}

impl Call {
    /// A call that starts a transfer or a check: refused once the app is closing.
    pub fn starts_work(self) -> bool {
        matches!(self, Call::Add | Call::Resume | Call::Retry | Call::Verify | Call::ResumeAll)
    }
}

/// What the desk came back with: the call, and its answer built from its own result.
#[derive(Debug, Clone)]
pub struct Landed {
    pub call: Call,
    pub answer: Result<Value, String>,
}

#[derive(Debug, Clone)]
pub enum Command {
    // ── The window and the surface ──
    /// `save_dir` empty means the default folder.
    Add { url: String, sha256: String, save_dir: String },
    Pause(i32),
    Resume(i32),
    Cancel(i32),
    Retry(i32),
    Verify { id: i32, sha256: Option<String> },
    OpenFolder(i32),
    PauseAll,
    ResumeAll,
    ClearCompleted,

    // ── The window ──
    Filter(i32),
    Sort(i32),
    Search(String),
    ToggleSelect(i32),
    SelectAll(bool),
    AiExplain,
    AiDismiss,
    Rail(RailCommand),

    // ── The clock and the window coming forward ──
    /// Read what the workers did since the last look. Only a revision that moved is drawn.
    Tick,

    // ── What the desk found ──
    Landed(Box<Landed>),
    /// A call ended without its answer: a panic, or the pool at its bound.
    WorkFailed(Call),
    /// The writers are paused and the list is written; the app may end.
    Settled,

    // ── The app's life ──
    /// The person closed the window while a download is being written: it closes, and the app
    /// runs on without it until the writers have stopped (`fx.linger`).
    Closing(WindowId),
    /// A second launch opened the window again while that close waited.
    Reopened,
    /// The process is asked to end: a logout, a signal, the display going, or a person's quit.
    Ending,
}
