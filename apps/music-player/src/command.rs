//! Every change to the model is one of these, wherever it came from: a button, the Mind, a
//! launch, a job's result, or what the player said.

use std::path::PathBuf;

use yantrik_app::app::WindowId;

use crate::kept::Kept;
use crate::library::{Library, Track};
use crate::model::Inbox;
use crate::mpv::Event;
use crate::player::ops::{Back, Loaded, Report, Resumed, VolumeSet};

/// A scan landed.
#[derive(Debug, Clone)]
pub struct Scanned {
    /// Its number (`model::Scans`).
    pub scan: u64,
    /// The folders it looked at, as the list stood when it started.
    pub folders: Vec<String>,
    pub found: Library,
    /// Whether an agent may be told each folder it looked at (`seen.rs`).
    pub for_agents: Vec<(String, bool)>,
}

/// Which of the player's jobs ended without a result (a panic, or the pool at its bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Work {
    Load,
    Resume,
    Pause,
    Back,
    Seek,
    Volume,
    Mute,
    Stop,
    Halt,
    Retire,
    Check,
    Folder,
    Scan,
}

impl Work {
    /// A job of the player's own, in its line of tickets.
    pub fn is_player(self) -> bool {
        !matches!(self, Work::Check | Work::Folder | Work::Scan)
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    // ── The window ──
    PlayPause,
    Stop,
    /// Seek to this fraction of the track: the progress bar.
    SeekTo(f32),
    /// The slider: 0 to 1.
    VolumeTo(f32),
    ToggleMute,
    /// A row of the library list, from 0.
    PlayRow(i32),
    /// A row of the queue, from 0.
    QueueRow(i32),
    Filter(String),
    AddFolder(String),
    RemoveFolder(i32),
    DismissNotice,
    /// The core's endpoint, handed over once at wire time (`model::Inbox`).
    Attach(Inbox),

    // ── The window and the surface ──
    Next,
    Previous,
    Rescan,

    // ── The surface ──
    /// `play`, with `index` from 1 when it was given.
    Play(Option<i64>),
    Pause,
    Seek(f64),
    SetVolume(f64),
    /// `play_file`, with the path as it was given.
    PlayFile(String),

    // ── A launch ──
    /// A file named on the command line, or by a second launch.
    Open(PathBuf),

    // ── What the jobs found ──
    /// A track handed to the player. `stale` is the complaint about mpv on the notice line when
    /// it was asked, which a player started for it puts to rest.
    Loaded { track: Track, loaded: Box<Loaded>, stale: Option<String> },
    Resumed(Box<Resumed>),
    Paused(Result<Option<Report>, String>),
    WentBack(Box<Back>),
    Rewound(Result<Option<Report>, String>),
    Sought(Result<Option<Report>, String>),
    VolumeSet { percent: f64, result: Result<VolumeSet, String> },
    Muted { muted: bool, result: Result<bool, String> },
    Stopped(Result<(), String>),
    /// A stop or a reap the player was asked for is done; nothing changes on screen.
    Tidied,
    /// The player was stopped and reaped; the number of the last one started.
    Retired(u64),
    /// `play_file`'s path checked: the track it names, or why it is refused.
    Checked(Result<Track, String>),
    /// A folder to add, looked at: its full path, and whether it is a folder.
    FolderChecked { path: String, is_folder: bool },
    Scanned(Box<Scanned>),
    /// A scan is this far: tracks found so far.
    Scanning { scan: u64, found: usize },
    /// A job ended without its result.
    WorkFailed(Work),

    // ── What the player said ──
    Mpv { player: u64, event: Event },

    // ── The app's life ──
    /// The person closed the window while there is a player to stop: it closes, and the app
    /// runs on without it until the player is reaped (`fx.linger`).
    Closing(WindowId),
    /// A second launch opened the window again while that close waited (`Event::Presented`).
    Reopened,
    /// The process is asked to end (`Event::ExitRequested`): a logout, a signal, the display
    /// going, or a person's quit.
    Ending,
    /// The settings file was edited outside the app.
    KeptOutside(Kept),
    /// A save of the folder list and the volume did not reach the disk.
    NotSaved(String),
}
