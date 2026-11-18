//! What Network Manager is: the model every command changes and every render reads, and the
//! commands.

use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use yantrik_app::mind::{AskError, Mind};
use yantrik_app::rail::{Rail, RailCommand};
use yantrik_ipc_contracts::network::{ScannedNetwork, WifiConnectParams, WifiForgetResult, WifiState};

use crate::backend::NetworkApi;
use crate::lane::Lane;
use crate::reading::Reading;

/// The parts of the model `render` redraws by name.
pub const PART_READING: &str = "reading";
pub const PART_NOTICE: &str = "notice";
pub const PART_SCAN: &str = "scan";
pub const PART_AI: &str = "ai";
/// The password prompt's status line.
pub const PART_PROMPT_STATUS: &str = "prompt.status";
/// The parts below write properties the person also changes (the window's in-out properties: the
/// open section, the prompt and what is typed in it). They are drawn only when a command names
/// them, never on a redraw of everything, so a redraw cannot reopen a prompt the person closed
/// or put back a section they left.
pub const PART_PROMPT_OPEN: &str = "prompt.open";
pub const PART_PROMPT_CLOSE: &str = "prompt.close";
pub const PART_PROMPT_CLEAR: &str = "prompt.clear";
pub const PART_SECTION: &str = "section";

/// One network the last scan found, as the Wi-Fi list draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InRange {
    /// A hidden network has no name to show. Said rather than shown blank, and still listed: a
    /// row dropped from this list is how a pane comes to say "no networks in range" when there
    /// are some.
    pub ssid: String,
    pub signal: i32,
    pub security: String,
    pub is_connected: bool,
    pub is_saved: bool,
}

impl InRange {
    pub fn from_scan(n: &ScannedNetwork) -> InRange {
        InRange {
            ssid: if n.hidden { "(hidden network)".into() } else { n.ssid.clone() },
            signal: n.signal,
            security: n.security.clone(),
            is_connected: n.is_connected,
            is_saved: n.is_saved,
        }
    }
}

/// The header's AI panel: what the companion said about this machine's network.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    pub working: bool,
    pub response: String,
}

/// What the last of each change answered, for the intent that asked for it.
#[derive(Debug, Clone, Default)]
pub struct Answers {
    pub radio: Option<WifiState>,
    pub disconnect: Option<WifiState>,
    pub forget: Option<WifiForgetResult>,
    /// `connect::joining_answer` or `connect::waiting_answer` for the network last asked for.
    pub connect: serde_json::Value,
}

pub struct Model {
    /// The last reading applied, and whether one has been.
    pub reading: Reading,
    pub read_once: bool,
    /// The sequence number of the reading on screen. A reading taken earlier than it is stale,
    /// whatever order the jobs landed in, and is dropped.
    pub shown: u64,
    /// Numbers readings in the order they are taken (shared with the jobs that take them).
    pub sequence: Arc<AtomicU64>,
    /// Plain refreshes in flight: the three-second tick waits while one is, rather than piling
    /// a second read onto a slow service.
    pub refreshing: u32,
    /// Whether the window has chosen its opening section (once, from the first reading).
    pub opened: bool,
    /// The section a command last asked for, by `active-tab` number.
    pub section: i32,
    /// What went wrong, in the strip under the header and in `describe.notice`.
    pub notice: String,
    pub scanning: bool,
    pub in_range: Vec<InRange>,
    /// The network the password prompt is for.
    pub prompt_ssid: String,
    /// The prompt's status line: connecting, connected, or why not.
    pub prompt_status: String,
    pub ai: Ai,
    pub answers: Answers,
    pub rail: Rail,
    pub mind: Arc<dyn Mind>,
    pub net: Arc<dyn NetworkApi>,
    /// The change to the machine in flight, and the ones waiting behind it (`lane.rs`).
    pub lane: Lane,
}

/// A reading, numbered in the order it was taken.
#[derive(Debug, Clone)]
pub struct Taken {
    pub seq: u64,
    pub reading: Reading,
}

/// What one change asked of the service came to.
#[derive(Debug, Clone)]
pub enum Outcome {
    Radio(Result<WifiState, String>),
    Disconnect(Result<WifiState, String>),
    Forget(Result<WifiForgetResult, String>),
    Scan(Result<Vec<ScannedNetwork>, String>),
    Connect {
        ssid: String,
        result: Result<WifiState, String>,
    },
}

impl Outcome {
    /// The change this is the outcome of.
    pub fn change(&self) -> Change {
        match self {
            Outcome::Radio(_) => Change::Radio,
            Outcome::Disconnect(_) => Change::Disconnect,
            Outcome::Forget(_) => Change::Forget,
            Outcome::Scan(_) => Change::Scan,
            Outcome::Connect { .. } => Change::Connect,
        }
    }
}

/// Which change a job that ended without a result was making.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Radio,
    Disconnect,
    Forget,
    Scan,
    Connect,
}

impl Change {
    /// Whether it changes the machine, and so goes through the lane one at a time. A scan only
    /// looks (`lane.rs`).
    pub fn uses_lane(self) -> bool {
        self != Change::Scan
    }

    /// What it is called in a sentence that starts with it.
    pub fn named(self) -> &'static str {
        match self {
            Change::Radio => "The radio change",
            Change::Disconnect => "The disconnect",
            Change::Forget => "Forgetting a network",
            Change::Scan => "The scan",
            Change::Connect => "The connect",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    /// The three-second refresh: a read, unless one is already in flight.
    Tick,
    /// Re-read the machine now (start, and the `refresh` intent).
    Refresh,
    /// A refresh landed.
    Read(Box<Taken>),
    /// A refresh ended without a reading (a panic, the pool's bound).
    ReadFailed,
    /// Turn the radio on or off: the state asked for, never a flip.
    SetRadio(bool),
    Disconnect,
    /// Delete a saved network, by name.
    Forget(String),
    /// Sweep for networks in range (the Scan button).
    Scan,
    /// The `wifi_scan` intent: the same sweep, refused first on a machine with no adapter.
    ScanAsked,
    /// The password prompt's Connect, or a saved row pressed: the request the typed secret, if
    /// any, travels in. `WifiConnectParams`' own `Debug` never prints it.
    Connect(WifiConnectParams),
    /// The `wifi_connect` intent: join a saved network, or raise the prompt for an unsaved one.
    Join(String),
    /// A change landed, with the reading taken after it.
    Changed(Box<Outcome>, Box<Taken>),
    /// A change's job ended without a result.
    ChangeFailed(Change),
    /// A rail row pressed: go to the section it is about.
    OpenSection(String),
    /// The header's Explain.
    Explain,
    Explained(Result<String, AskError>),
    DismissAi,
    Rail(RailCommand),
}
