//! Every change to the model is one of these, wherever it came from: a key, a button, the Mind,
//! or what a worker or a shell found.

use std::sync::Arc;

use serde_json::Value;
use yantrik_app::app::WindowId;

use crate::model::Facts;
use crate::session::Session;

/// What the header, the tab strip, the find bar and the prompt can ask for, by the id the window
/// sends. A shortcut asks for the same ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    New,
    Close,
    ConfirmClose,
    CancelClose,
    Restart,
    Live,
    Dismiss,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NextTab,
    PreviousTab,
    NextMatch,
    PreviousMatch,
    Explain,
}

impl Do {
    pub fn of(id: &str) -> Option<Do> {
        Some(match id {
            "new" => Do::New,
            "close" => Do::Close,
            "confirm-close" => Do::ConfirmClose,
            "cancel-close" => Do::CancelClose,
            "restart" => Do::Restart,
            "live" => Do::Live,
            "dismiss" => Do::Dismiss,
            "zoom-in" => Do::ZoomIn,
            "zoom-out" => Do::ZoomOut,
            "zoom-reset" => Do::ZoomReset,
            "next-tab" => Do::NextTab,
            "previous-tab" => Do::PreviousTab,
            "next-match" => Do::NextMatch,
            "previous-match" => Do::PreviousMatch,
            "explain" => Do::Explain,
            _ => return None,
        })
    }
}

/// Why a tab is being started, which says what the person or the caller is told of it.
#[derive(Debug, Clone, PartialEq)]
pub enum Why {
    /// The first tab, at start.
    First,
    /// The New tab button or its shortcut.
    Person,
    /// Restart on an exited shell: its tab has gone and this one takes its place.
    Restart,
    /// The `new_tab` action.
    NewTab,
    /// The `open_directory` action, with the path as it was asked for.
    Directory(String),
}

/// A shell that was started on a worker, or why it could not be.
#[derive(Clone)]
pub struct Spawned {
    pub why: Why,
    pub result: Result<(Arc<Session>, Facts), String>,
}

/// What `run` or `send_input` found out, read by the worker that waited for the shell.
#[derive(Clone)]
pub struct Reply {
    /// The tab the input went to.
    pub tab: u64,
    pub body: Value,
}

#[derive(Clone)]
pub enum Command {
    // ── The window ──
    Do(Do),
    SelectTab(i32),
    CloseTab(i32),
    Resized(i32, i32),
    Scroll(i32),
    Search(String),
    Paste(String),
    /// Bytes for the active shell, from a key.
    Input(Vec<u8>),

    // ── The surface ──
    Run(String),
    SendInput(String),
    NewTab,
    OpenDirectory(String),

    // ── What the workers and the shells found ──
    Spawned(Box<Spawned>),
    /// A shell was started and never answered: a panic, or the pool at its bound.
    SpawnLost,
    Answered(Box<Reply>),
    AnswerLost,
    Probed { tab: u64, facts: Facts },
    ProbeLost { tab: u64 },
    /// What a close or a quit was waiting to know: how many shells have something running.
    Checked { ticket: u64, busy: usize },
    CheckLost { ticket: u64 },
    Explained { generation: u64, answer: Result<String, String> },

    // ── The clock ──
    /// A shell has something new. At most one is queued a frame.
    Output,
    /// The frame after output: the window and the probes catch up.
    Frame,
    /// The probes that are due are run.
    Probe,
    /// The backstop of a logout or a close.
    EndNow,

    // ── The app's life ──
    CloseRequested(WindowId),
    QuitRequested,
    /// A second launch opened the window again while a close waited.
    Reopened,
    /// The process is asked to end: a logout, a signal, the display going.
    Ending,
    Activated,
}
