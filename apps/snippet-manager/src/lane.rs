//! The write lane: every change to the library is written as the whole file, off the UI thread,
//! and the window never shows a change the file refused.
//!
//! The old app wrote on the UI thread inside every change, and put memory back when the write
//! failed. Here a change is made in memory at once (so the window follows the press), and its
//! write runs as a job with a ticket: a revision, numbered in the order the changes were made, and
//! the epoch it was made in. The jobs may run on any of the pool's threads in any order, so the
//! writer below decides under one lock:
//!
//! - a write whose revision is already covered by a newer one on disk is not made again: the file
//!   holds a library that includes it (`Covered`);
//! - a write that fails reads the file back, still under the lock, and the app puts that library
//!   on screen, so memory is the disk again. The epoch it failed in is spent: every write of that
//!   epoch still waiting is not made (`Undone`), because each carries a library built on the
//!   change that failed;
//! - the next change after the failure is landed is made in a new epoch, and writes again.
//!
//! Each change answers for itself when its own write lands, so the answer a caller gets is about
//! its own change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::disk::Disk;
use crate::store::Library;

/// What a write of an earlier epoch is told when a later epoch has begun.
const EARLIER: &str = "an earlier change could not be written, so this one was not kept";

/// Which write this is: its place in the order the changes were made, and its epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket {
    pub revision: u64,
    pub epoch: u64,
}

/// What came of one write.
#[derive(Clone)]
pub enum Landed {
    /// The file now holds this change.
    Saved,
    /// A newer write, which included this change, was on disk already.
    Covered,
    /// The write failed. `on_disk` is the file read back after it: the library as last kept.
    Failed {
        why: String,
        on_disk: Result<Library, String>,
    },
    /// Not written: a write of this epoch had failed, or a later epoch had begun.
    Undone(String),
}

/// The writer's own state, shared by every write job.
#[derive(Default)]
struct Writer {
    /// The newest revision on disk.
    written: u64,
    /// The epoch the writes being made belong to.
    epoch: u64,
    /// Why this epoch's writes stopped, once one failed.
    failed: Option<String>,
}

/// The app's side of the lane, kept in the model.
pub struct Lane {
    writer: Arc<Mutex<Writer>>,
    disk: Arc<dyn Disk>,
    path: PathBuf,
    revision: u64,
    epoch: u64,
    in_flight: usize,
    /// The newest revision known to be on disk: a write of it, or of a newer one, has landed.
    confirmed: u64,
}

impl Lane {
    pub fn new(disk: Arc<dyn Disk>, path: PathBuf) -> Lane {
        Lane {
            writer: Arc::default(),
            disk,
            path,
            revision: 0,
            epoch: 0,
            in_flight: 0,
            confirmed: 0,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn disk(&self) -> Arc<dyn Disk> {
        self.disk.clone()
    }

    /// No write is waiting to land.
    pub fn idle(&self) -> bool {
        self.in_flight == 0
    }

    /// The next write's ticket, counted in flight until [`Lane::landed`].
    pub fn ticket(&mut self) -> Ticket {
        self.revision += 1;
        self.in_flight += 1;
        Ticket { revision: self.revision, epoch: self.epoch }
    }

    pub fn landed(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    /// A write of `revision` landed `Saved` or `Covered`: the file holds it, or a newer one.
    pub fn confirm(&mut self, revision: u64) {
        self.confirmed = self.confirmed.max(revision);
    }

    /// The newest revision known to be on disk. 0 is the file as the start read it.
    pub fn confirmed(&self) -> u64 {
        self.confirmed
    }

    /// The newest revision issued.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// A write of `ticket`'s epoch failed and its read-back is being put on screen: true the
    /// first time for an epoch, when the window must take the file's library. Later changes are
    /// made in the next epoch.
    pub fn spend(&mut self, ticket: Ticket) -> bool {
        if ticket.epoch != self.epoch {
            return false;
        }
        self.epoch += 1;
        true
    }

    /// The job that writes `bytes` for `ticket`. Run off the UI thread.
    pub fn job(&self, ticket: Ticket, bytes: Vec<u8>) -> impl FnOnce() -> Landed + Send + 'static {
        let writer = self.writer.clone();
        let disk = self.disk.clone();
        let path = self.path.clone();
        move || {
            let mut w = writer.lock().unwrap_or_else(|p| p.into_inner());
            if ticket.epoch > w.epoch {
                w.epoch = ticket.epoch;
                w.failed = None;
            }
            if ticket.epoch < w.epoch {
                return Landed::Undone(EARLIER.into());
            }
            if let Some(why) = &w.failed {
                return Landed::Undone(why.clone());
            }
            if w.written >= ticket.revision {
                return Landed::Covered;
            }
            match disk.write(&path, &bytes) {
                Ok(()) => {
                    w.written = ticket.revision;
                    Landed::Saved
                }
                Err(why) => {
                    w.failed = Some(why.clone());
                    Landed::Failed { on_disk: disk.read_back(&path), why }
                }
            }
        }
    }
}
