//! What Music is: the model every command changes and every render and `describe` reads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use yantrik_app::cx::Inbound;
use yantrik_app::queue::Post;

use crate::library::Track;
use crate::mpv::Observed;
use crate::player::{Forward, Player, Rig, Start};
use crate::queue::Queue;
use crate::Music;

/// The parts of the model `render` redraws by name.
///
/// The library list, the queue and the folders: redrawn when they change, never on a tick.
pub const PART_LISTS: &str = "lists";
/// The status line, the notice and the scanning flag: cheap, so a scan's progress redraws only
/// this and never rebuilds the library list.
pub const PART_STATUS: &str = "status";
/// The transport, from what mpv last reported: the only part the playing position redraws.
pub const PART_TRANSPORT: &str = "transport";
/// What a command names when nothing on screen changed.
pub const NOTHING: &str = "nothing";

/// Whether there is a player, in the words `describe` uses.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayerState {
    NotStarted,
    Running,
    /// mpv could not be started; the sentence says why.
    Unavailable(String),
    /// mpv was running and went away.
    Stopped,
}

impl PlayerState {
    pub fn word(&self) -> &'static str {
        match self {
            PlayerState::NotStarted => "not started",
            PlayerState::Running => "running",
            PlayerState::Unavailable(_) => "unavailable",
            PlayerState::Stopped => "stopped",
        }
    }
}

/// The scans: numbered in the order they start, so of two that land the later one's library is
/// what is shown, whichever lands last.
#[derive(Debug, Default)]
pub struct Scans {
    /// Scans started and not landed.
    pub running: u32,
    /// The number the last scan started was given.
    pub started: u64,
    /// The number of the scan whose library is in hand.
    pub shown: u64,
    /// A folder changed while a scan ran: scan again when it lands.
    pub again: bool,
}

/// The core's endpoint, handed over once at wire time: the player's reader thread and the scan's
/// progress post through it.
#[derive(Clone)]
pub struct Inbox(pub Post<Inbound<Music>>);

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Inbox")
    }
}

pub struct Model {
    /// Where the folder list and the volume are kept, as a save failure names it.
    pub config_file: PathBuf,
    pub folders: Vec<String>,
    /// How many tracks each folder held at the last scan, in the order of `folders`.
    pub per_folder: Vec<usize>,
    pub library: Vec<Track>,
    pub filter: String,
    /// Indices into `library` that match `filter`, in the order the window lists them.
    pub shown: Vec<usize>,
    pub scans: Scans,

    pub queue: Queue,
    pub player: Arc<Player>,
    pub state: PlayerState,
    /// The number of the player whose reports are in `observed`; what an older one says is not
    /// heard. Reports from players numbered `deaf_to` or below are not heard at all: they were
    /// stopped by a close or a logout.
    pub hearing: u64,
    pub deaf_to: u64,
    /// What mpv last reported, as it reached the UI thread.
    pub observed: Observed,
    /// The volume, 0–100: what mpv reported last, or what it will start with.
    pub volume: f64,
    pub muted: bool,
    /// Tickets handed out to the player's jobs (`player::turn`); the next one is this plus one.
    pub tickets: u64,
    /// The player's jobs started and not landed. A close with none and no player running ends at
    /// once, as it always did.
    pub player_jobs: u32,
    /// A volume change of the person's is on its way to the player. The newest one asked for
    /// meanwhile waits in `volume_next` until it lands, so a drag is one change at a time and the
    /// last one always reaches mpv. (`volume` follows what mpv reports, so it cannot hold it.)
    pub volume_sending: bool,
    pub volume_next: Option<f64>,

    pub status: String,
    /// What an agent reads in place of `status` when it names folders the agent may not see.
    pub agent_status: Option<String>,
    pub notice: String,
    /// Which paths an agent may be told (`seen.rs`): every folder the last scan looked at, and
    /// the file the player last reported on.
    pub folders_for_agents: HashMap<String, bool>,
    pub playing_for_agents: Option<(String, bool)>,

    /// What the surface call being settled is answered with, written in the command that settles
    /// it (`answers.rs`).
    pub answer: Value,
    pub inbox: Option<Inbox>,
    /// The person closed the window: it is closed, and the app lingers without one until the
    /// player has been stopped and reaped (`fx.linger`), unless a second launch opens the window
    /// again first (`Event::Presented`).
    pub closing: bool,
    /// The session or a person asked the app to end (`Event::ExitRequested`): it ends once the
    /// player has been stopped. Nothing cancels it.
    pub ending: bool,
}

impl Model {
    pub fn new(rig: &Rig, kept: &crate::kept::Kept, config_file: PathBuf) -> Model {
        let folders = kept.folders.clone().unwrap_or_else(|| {
            rig.music.iter().map(|m| m.to_string_lossy().into_owned()).collect()
        });
        let mut m = Model {
            config_file,
            per_folder: vec![0; folders.len()],
            folders,
            library: Vec::new(),
            filter: String::new(),
            shown: Vec::new(),
            scans: Scans::default(),
            queue: Queue::default(),
            player: Player::new(rig),
            state: PlayerState::NotStarted,
            hearing: 0,
            deaf_to: 0,
            observed: Observed::default(),
            volume: kept.volume,
            muted: false,
            tickets: 0,
            player_jobs: 0,
            volume_sending: false,
            volume_next: None,
            status: String::new(),
            agent_status: None,
            notice: String::new(),
            folders_for_agents: HashMap::new(),
            playing_for_agents: None,
            answer: Value::Null,
            inbox: None,
            closing: false,
            ending: false,
        };
        m.refilter();
        m
    }

    /// Whether there is a player, by the last word that reached the UI thread.
    pub fn running(&self) -> bool {
        self.state == PlayerState::Running
    }

    /// What mpv last reported, or nothing when there is no player.
    pub fn observed(&self) -> Observed {
        if self.running() {
            self.observed.clone()
        } else {
            Observed::default()
        }
    }

    /// A file loaded in the player, by its last report.
    pub fn loaded(&self) -> bool {
        let o = &self.observed;
        self.running() && o.path.is_some() && !o.idle && !o.gone
    }

    pub fn playing(&self) -> bool {
        self.running() && self.observed.playing()
    }

    pub fn shown_tracks(&self) -> Vec<Track> {
        self.shown.iter().filter_map(|&i| self.library.get(i).cloned()).collect()
    }

    pub fn refilter(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.shown = self
            .library
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                needle.is_empty() || t.title.to_lowercase().contains(&needle) || t.album.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
    }

    /// How the next player starts, if a job has to start one.
    pub fn start(&self) -> Start {
        let inbox = self.inbox.clone();
        let forward: Forward = Arc::new(move |player, event| {
            if let Some(Inbox(post)) = &inbox {
                if let Err(e) = post.post(Inbound::Command(crate::Command::Mpv { player, event })) {
                    tracing::debug!(?e, "a report from the player was not taken");
                }
            }
        });
        Start { volume: self.volume, muted: self.muted, forward }
    }

    /// The volume as the window draws it and `describe` reports it.
    pub fn volume_now(&self) -> f64 {
        self.observed().volume.unwrap_or(self.volume)
    }

    pub fn muted_now(&self) -> bool {
        if self.running() {
            self.observed.muted
        } else {
            self.muted
        }
    }
}
