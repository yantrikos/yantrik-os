//! Every change to the model is one of these, wherever it came from: a button, a call from a Mind,
//! or what a worker found.
//!
//! The person's buttons and the Mind's actions are the same requests, so a person and an agent
//! cannot get different answers about whether something worked. Where the two read their
//! arguments differently (the window's fields are text; a call's are JSON) each builds the same
//! command from its own, and the update does the rest once.

use std::path::PathBuf;

use yantrik_app::app::WindowId;

use crate::config::Config;
use crate::gallery::{Listed, Record, Sidecar};
use crate::model::Field;
use crate::plan::Plan;
use crate::said::Said;
use crate::trash::Moved;

/// What a Mind asks for when it cancels.
#[derive(Debug, Clone, PartialEq)]
pub enum Which {
    /// Every job: the window's Cancel button, and `cancel_all`.
    All,
    /// One job, by the id `describe` lists.
    One(i32),
    /// `cancel` without a usable `job`; `given` says whether one was sent at all.
    Unusable { given: bool },
}

/// What a resolved picture is for.
#[derive(Debug, Clone)]
pub enum Next {
    Open,
    /// More pictures of it, from the record beside it. `fill` puts the sentence back in the
    /// prompt box, as the window's button does.
    Variations { of: String, count: u32, fill: bool, generation: u64 },
    Upscale { of: String, factor: u32 },
    /// Choosing its row: the record is read for the right-hand pane.
    Choose { ticket: u64 },
}

/// A picture a name was resolved to, with its record when the next step needs it.
#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub sidecar: Option<Sidecar>,
}

/// A picture that was made and filed, as the worker that made it found it.
#[derive(Debug, Clone)]
pub struct Saved {
    pub record: Record,
    pub stamp: String,
    pub seconds: f64,
}

/// What `set_backend` asked for, as text.
#[derive(Debug, Clone)]
pub struct BackendAsk {
    pub kind: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub workflow: String,
}

/// A backend change that was written down (or could not be, with nowhere to write it).
#[derive(Debug, Clone)]
pub struct BackendSet {
    pub config: Config,
    pub written: bool,
}

#[derive(Clone)]
pub enum Command {
    // ── The window ──
    /// A field was typed into.
    Edit(Field, String),
    Choose(i32),
    /// The window's width in logical pixels.
    Resized(i32),
    /// A button the window does not have, by the id it sent.
    Unknown(String),
    /// "Where pictures are made": says where the choice lives, changes nothing.
    ExplainBackend,
    OpenFolder,

    // ── The window and the surface ──
    /// `None` makes the ask from the window's fields.
    Generate(Option<Box<Plan>>),
    Variations { of: String, count: u32, fill: bool },
    Upscale { of: String, factor: u32 },
    Open { of: String },
    Delete { of: String },
    SetBackend(Box<BackendAsk>),
    Cancel(Which),
    Refresh,

    // ── What the workers found ──
    Resolved { next: Next, result: Result<Found, Said> },
    Trashed(Result<Trashed, Said>),
    Opened { shown: String, outcome: yantrik_app::DesktopOutcome },
    /// A picture of a job: made and filed, stopped (`None`), or failed with the reason.
    Made { id: i32, outcome: Result<Saved, Option<String>> },
    /// The worker making a picture never answered: a panic, or the pool at its bound.
    MadeLost(i32),
    BackendSet(Result<BackendSet, String>),
    Listed(Box<Listed>),
    /// A worker that was asked for something never answered: a panic, or the pool at its bound.
    Lost(&'static str),
    /// The folder was looked at and had not changed.
    Unchanged,

    // ── The clock and the app's life ──
    /// Look at the output folder and read it again if it moved.
    Poll,
    /// The surface is served: `generate` and `variations` are brought to the grade the backend on
    /// disk deserves.
    Regrade,
    /// Brought to the front: the folder is looked at again.
    Activated,
    /// The person closed the window: an unsent prompt is kept, and the window goes.
    Closing(WindowId),
    /// A quit, a signal or the display going: an unsent prompt is kept, and the app ends.
    Ending,
}

/// A picture moved to the Trash, with the folder's stamp afterwards.
#[derive(Debug, Clone)]
pub struct Trashed {
    pub path: PathBuf,
    pub moved: Vec<Moved>,
    pub stamp: String,
}
