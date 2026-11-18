//! What Containers is: the model every command changes and every render reads, and the commands.

use std::sync::Arc;

use serde_json::Value;
use yantrik_app::mind::{AskError, Mind};
use yantrik_app::rail::{Rail, RailCommand};

use crate::machine::{Listing, Machine};
use crate::runtime::{Availability, Container, Exit, Image, Volume};

/// The parts of the model `render` redraws by name.
pub const PART_LISTING: &str = "listing";
pub const PART_NOTICE: &str = "notice";
pub const PART_TAB: &str = "tab";
pub const PART_LOGS: &str = "logs";
pub const PART_AI: &str = "ai";

/// The window's tabs, in the order the tab bar draws them: `active-tab` is an index into this.
pub const TABS: [&str; 3] = ["containers", "images", "volumes"];

/// The log pane: what was read last, for which container, and whether it is open. The text
/// outlives the pane: the rail and the AI button still have it to read after Close.
#[derive(Debug, Clone, Default)]
pub struct Logs {
    pub text: String,
    /// The container's id, as the pane's title says it.
    pub container: String,
    pub open: bool,
    /// The turn the text was read in, so an older read that lands later is not drawn over it.
    pub turn: u64,
}

/// The header's AI button: the one question about the log on screen.
#[derive(Debug, Clone, Default)]
pub struct Ai {
    pub working: bool,
    pub response: String,
}

pub struct Model {
    /// The runtime the last listing asked: `docker` until one has.
    pub runtime: String,
    /// What the app last learned about the runtime itself.
    ///
    /// `describe` has to answer a question the container list cannot: an empty list used to
    /// mean "docker is not installed", "the daemon is down" and "there is nothing running here"
    /// all at once, and a mind reading it concluded the machine was empty.
    pub health: Availability,
    pub containers: Vec<Container>,
    pub images: Vec<Image>,
    pub volumes: Vec<Volume>,
    /// The turn the listing on screen was read in.
    pub listed_turn: u64,
    /// What could not be done, in one line under the header. Set on failure, cleared on
    /// success, and carried in `describe`.
    pub notice: String,
    pub tab: usize,
    pub logs: Logs,
    pub ai: Ai,
    /// Commands the person started and that have not landed, by their arguments: a second press
    /// of the same button meanwhile is the same request, not another one.
    pub in_flight: Vec<Vec<String>>,
    /// The answer owed to the call that settles next, written by the command that settles it.
    pub answer: Value,
    pub rail: Rail,
    pub machine: Machine,
    pub mind: Arc<dyn Mind>,
}

impl Model {
    pub fn running(&self) -> usize {
        self.containers.iter().filter(|c| c.state == "running").count()
    }

    pub fn total(&self) -> usize {
        self.containers.len()
    }

    pub fn stopped(&self) -> usize {
        self.total() - self.running()
    }

    /// Every container listed, as `(id, name)`: what a caller's name is resolved against.
    pub fn rows(&self) -> Vec<(String, String)> {
        self.containers
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect()
    }

    /// How the list describes this container now, or `None` when it is no longer in it.
    pub fn observed_state(&self, id: &str) -> Option<String> {
        self.containers
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.state.clone())
    }
}

/// What can be done to one container, by a row's button or by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Start,
    Stop,
    Restart,
    Remove,
    Logs,
}

impl Verb {
    /// The runtime's arguments for this verb on container `id`.
    pub fn argv(self, id: &str) -> Vec<String> {
        let words: &[&str] = match self {
            Verb::Start => &["start"],
            Verb::Stop => &["stop"],
            Verb::Restart => &["restart"],
            Verb::Remove => &["rm", "-f"],
            Verb::Logs => &["logs", "--tail", "200"],
        };
        words
            .iter()
            .map(|w| w.to_string())
            .chain(std::iter::once(id.to_string()))
            .collect()
    }
}

/// A container the Mind named, resolved: the action then checks what the runtime shows after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub verb: Verb,
    pub id: String,
    pub name: String,
}

/// What a job was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Re-read the listing.
    Refresh,
    /// One runtime command, then the listing. `named` when the Mind asked by name.
    Change {
        argv: Vec<String>,
        named: Option<Named>,
    },
    /// Read a container's log into the pane, then the listing.
    Logs { id: String, named: Option<Named> },
}

impl Job {
    /// The runtime's arguments, for a job that runs a command.
    pub fn argv(&self) -> Option<Vec<String>> {
        match self {
            Job::Refresh => None,
            Job::Change { argv, .. } => Some(argv.clone()),
            Job::Logs { id, .. } => Some(Verb::Logs.argv(id)),
        }
    }
}

/// A job that landed: what it was asked, what the command said, and the listing after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landed {
    pub job: Job,
    /// `None` for a refresh, which runs no command of its own.
    pub exit: Option<Exit>,
    pub listing: Listing,
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Re-read containers, images and volumes.
    Refresh,
    /// The tab bar switched (it switches itself), or the Mind asked for a tab.
    ShowTab(usize),
    /// A row's button, on the row's container id.
    Row { verb: Verb, id: String },
    /// A container the Mind named: resolved against the list, refused when it is not there.
    Named { verb: Verb, needle: String },
    /// "Run New", with the image typed.
    RunImage(String),
    PullImage(String),
    RemoveImage(String),
    CreateVolume(String),
    RemoveVolume(String),
    /// A job landed.
    Landed(Box<Landed>),
    /// A job ended without a result: a panic, or the pool at its bound.
    Lost(Job),
    /// The screen closed its log pane.
    LogsClosed,
    /// The header's AI button.
    Summarize,
    Summarized(Result<String, AskError>),
    DismissAi,
    Rail(RailCommand),
}
