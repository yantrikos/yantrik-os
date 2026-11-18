//! The download engine — what the buttons in this window actually do.
//!
//! Until now this app was a window with no machinery behind it: every callback logged a line and
//! returned, so the list was always empty and nothing ever reached disk. That is why it had no
//! control surface. `docs/app-control.md` is explicit that an app whose data is a stub should not
//! publish one, because a confident lie in JSON is worse than a screenshot a model has to guess
//! at. So the engine comes first and the surface follows it.
//!
//! # Shape
//!
//! One [`Engine`] holds every download and is cheap to clone — it is a handle onto shared state,
//! not the state itself. Each transfer runs on its own thread, writing to the file and to the
//! shared record as it goes; the UI thread never blocks on the network and never touches a
//! socket. Threads talk to the window only by bumping [`Engine::revision`], which the window
//! polls: a worker that tried to push into Slint would need a weak handle and an
//! `invoke_from_event_loop` per chunk, and would repaint the list sixty times a second for a fast
//! transfer. Polling a revision costs one atomic load per tick and repaints only on real change.
//!
//! # Stopping
//!
//! Pause, cancel and "the window is closing" are the same question — *should this thread still be
//! writing?* — so they share one [`Control`] flag per download, checked once per chunk. A paused
//! download keeps its partial file and its byte count, which is the entire reason resume can be
//! more than a restart.
//!
//! # Resume is a request, not a guarantee
//!
//! Resuming sends `Range: bytes=N-` and hopes. A server that honours it answers `206` and the
//! remaining bytes get appended. A server that does not answers `200` and the body it sends is the
//! *whole file* — appending that to a partial one produces a corrupt file that still looks
//! complete. So a `200` to a ranged request truncates and starts over, and `resume_supported`
//! records which answer came back so the window can stop promising something this server will not
//! do.
//!
//! # Keeping the list
//!
//! Until now the list lived only in this process, so closing the window lost every record and left
//! every partial transfer on disk as a file nobody could explain or continue. A mind that started a
//! 4 GB image and came back after a restart found nothing. The list is now written to
//! `~/.local/share/yantrik/downloads/state.json` on every transition worth noticing, and read back
//! at start.
//!
//! The file is a record of intent, not a measurement, and it is never trusted as one. On the way
//! back in every record is reconciled against what is actually on the filesystem: the byte count
//! comes from the partial file itself, a download that was still running when the process died
//! comes back **paused and interrupted** rather than silently restarted, and a completed file that
//! is no longer where it was recorded comes back as [`Status::Missing`] rather than as a success
//! the person cannot open. Nothing on disk is deleted by reconciliation — a stray partial file
//! whose record is gone is left exactly where it is, because it is the person's data and a
//! forgotten record is not permission to remove it.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use yantrik_app::Notification;

/// Read size per loop turn. Also the granularity at which a pause is noticed, which is why it is
/// not larger: 64 KiB on a slow link is a fraction of a second of latency on the stop button.
const CHUNK: usize = 64 * 1024;

/// How often speed and ETA are recomputed. Shorter than this and the number jitters with every
/// TCP window; longer and it lags behind a transfer that has actually stalled.
const SPEED_WINDOW: Duration = Duration::from_millis(500);

/// The shape of `state.json`. A file carrying any other number is not guessed at.
const STATE_VERSION: u32 = 1;

/// How often a *running* transfer's byte count reaches the disk.
///
/// Every transition a person or a mind would notice is written immediately; progress is not. At a
/// 500 ms speed window a fast download would otherwise rewrite the whole list twice a second for
/// the sake of one integer. Nothing is lost by writing it every few seconds, because the byte
/// count in the file is never the one that is believed on the way back in — reconciliation
/// measures the partial file itself.
const PROGRESS_SAVE_INTERVAL: Duration = Duration::from_secs(5);

// ── Status ──────────────────────────────────────────────────────────

/// Where one download is in its life.
///
/// The strings are the ones `download_manager.slint` already switches on; they are part of the
/// UI contract, not an implementation detail, and the control surface publishes the same words so
/// a caller and a person are reading the same vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
    /// It finished once and the file is not there now.
    ///
    /// Its own word rather than `failed`, because the two call for different things: a failed
    /// transfer never arrived, while this one did and then the file was moved, renamed or deleted
    /// after the fact. Showing it as `completed` would point the person at a path that holds
    /// nothing, which is the confident lie the surface rules exist to prevent.
    Missing,
}

impl Status {
    /// Finished once: `completed`, or `missing` because the file went away afterwards. Its path
    /// holds the person's file, not a partial one.
    pub fn is_finished(self) -> bool {
        matches!(self, Status::Completed | Status::Missing)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Downloading => "downloading",
            Status::Paused => "paused",
            Status::Completed => "completed",
            Status::Failed => "failed",
            Status::Missing => "missing",
        }
    }

    /// The inverse of [`Status::as_str`], for reading a record back off disk.
    ///
    /// `None` rather than a default: a status this build does not recognise means the file was
    /// written by something else, and guessing at it is how a record ends up pointing the wrong
    /// command at a real file.
    pub fn parse(word: &str) -> Option<Status> {
        Some(match word {
            "queued" => Status::Queued,
            "downloading" => Status::Downloading,
            "paused" => Status::Paused,
            "completed" => Status::Completed,
            "failed" => Status::Failed,
            "missing" => Status::Missing,
            _ => return None,
        })
    }

    /// Whether this download is still going to move on its own.
    pub fn is_active(self) -> bool {
        matches!(self, Status::Queued | Status::Downloading)
    }
}

/// What a worker thread should do at its next chunk boundary.
mod control {
    pub const RUN: u8 = 0;
    pub const PAUSE: u8 = 1;
    pub const CANCEL: u8 = 2;
}

// ── One download ────────────────────────────────────────────────────

/// Everything known about one transfer.
///
/// Cloned out of the engine for reading, so neither the window nor the control surface holds the
/// lock while it formats a view.
#[derive(Clone, Debug)]
pub struct Download {
    pub id: i32,
    pub url: String,
    pub filename: String,
    pub save_dir: PathBuf,
    pub status: Status,
    pub downloaded: u64,
    /// `None` when the server never said — a chunked response has no length, and reporting a
    /// percentage for it would be inventing one.
    pub total: Option<u64>,
    pub speed_bps: f64,
    pub eta_secs: Option<u64>,
    pub checksum_expected: String,
    /// `none`, `verifying`, `pass`, `fail` — the words the UI switches on.
    pub checksum_status: String,
    pub file_hash: String,
    pub content_type: String,
    pub started_at: String,
    pub ended_at: String,
    pub error: String,
    pub resume_supported: bool,
    pub selected: bool,
    /// Read back from the state file at start rather than queued in this session.
    ///
    /// Not itself stored — it is a fact about *this* run, and a caller asking "is anything here
    /// left over from before?" needs it to mean that and not "was ever restored".
    pub restored: bool,
    /// Was still running when the process last stopped.
    ///
    /// It is paused now, holding whatever reached the disk; nothing was resumed on its behalf.
    /// Cleared the moment someone resumes or retries it, because from then on it is an ordinary
    /// transfer again.
    pub interrupted: bool,
    /// An agent asked for it. Its fetch is held to the public internet (`fetch.rs`), also after a
    /// restart and a resume, so this is stored.
    pub agent: bool,
    /// The stamp (device, inode, birth time, modification time, size) of the file this download
    /// created, as the app last wrote it; empty until it has created one (`identity.rs`).
    ///
    /// Cancel, retry and resume touch a file only if it still has exactly this stamp;
    /// anything else there is the person's, and is left alone.
    pub file_id: String,
}

impl Download {
    pub fn path(&self) -> PathBuf {
        self.save_dir.join(&self.filename)
    }

    /// 0.0–1.0, or 0.0 when the total is unknown.
    pub fn progress(&self) -> f32 {
        match self.total {
            Some(total) if total > 0 => (self.downloaded as f64 / total as f64).min(1.0) as f32,
            _ => 0.0,
        }
    }

    /// `"45.2 MB / 128.0 MB"`, or just what has arrived when the size is unknown.
    pub fn size_text(&self) -> String {
        match self.total {
            Some(total) => format!("{} / {}", format_bytes(self.downloaded), format_bytes(total)),
            None => format_bytes(self.downloaded),
        }
    }

    pub fn speed_text(&self) -> String {
        if self.status == Status::Downloading && self.speed_bps > 0.0 {
            format!("{}/s", format_bytes(self.speed_bps as u64))
        } else {
            String::new()
        }
    }

    pub fn eta_text(&self) -> String {
        match self.eta_secs {
            Some(secs) if self.status == Status::Downloading => format_duration(secs),
            _ => String::new(),
        }
    }
}

// ── The engine ──────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    /// Insertion order is display order; there are never enough downloads for the linear scans
    /// this implies to matter, and a `Vec` keeps "newest last" free.
    items: Vec<Download>,
    controls: HashMap<i32, Arc<AtomicU8>>,
}

/// A handle onto every download in this window. Clone it freely — clones share one state.
#[derive(Clone)]
pub struct Engine {
    state: Arc<Mutex<State>>,
    next_id: Arc<AtomicI32>,
    revision: Arc<AtomicU64>,
    default_dir: PathBuf,
    /// Where the list is kept between runs.
    store_path: PathBuf,
    /// Held from taking the snapshot to the end of the write, so the last list taken is the last
    /// list written, however many workers save at once.
    save_lock: Arc<Mutex<()>>,
    /// What went wrong reading or writing that file, or what changed about the list while nobody
    /// was looking — said on screen and in `describe`. An app that cannot keep its list has to
    /// admit it; silently forgetting is the fault this whole section exists to fix.
    notice: Arc<Mutex<String>>,
    /// Whether the current notice has already been put in front of the person.
    ///
    /// `describe` keeps reporting the notice for as long as it is true, because a mind may ask
    /// long after it happened. The window does not: a banner the person has already read and
    /// then acted past must not come back on the next timer tick, or nothing they do can ever
    /// clear it. A new notice raises it again.
    notice_seen: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the current notice is information (the list came back changed) rather than a
    /// fault. The window leads the one with an info glyph and the other with a warning glyph.
    notice_is_info: Arc<std::sync::atomic::AtomicBool>,
    /// When the list was last written. Read only by the progress throttle.
    last_save: Arc<Mutex<Option<Instant>>>,
    /// What a worker would tell the person (a transfer ended or failed), waiting for the app to
    /// send it. A worker has no `Effects` to ask with, so it leaves the notification here and the
    /// app takes it on its next look (`Engine::announcements`).
    announced: Arc<Mutex<Vec<Notification>>>,
}

impl Engine {
    /// The engine a window gets: the real store, loaded.
    pub fn new() -> Self {
        Self::open(state_dir())
    }

    /// The same engine pointed at a directory of the caller's choosing.
    ///
    /// The seam [`Engine::new`] goes through, and the one the tests use: persistence that can only
    /// be exercised against the person's own `~/.local/share` is persistence nobody checks.
    pub fn open(state_dir: PathBuf) -> Self {
        Self::open_in(state_dir, default_download_dir())
    }

    /// [`Engine::open`], with the folder downloads land in when the caller names none.
    pub fn open_in(state_dir: PathBuf, default_dir: PathBuf) -> Self {
        let engine = Self {
            state: Arc::new(Mutex::new(State::default())),
            next_id: Arc::new(AtomicI32::new(1)),
            revision: Arc::new(AtomicU64::new(0)),
            default_dir,
            store_path: state_dir.join("state.json"),
            save_lock: Arc::new(Mutex::new(())),
            notice: Arc::new(Mutex::new(String::new())),
            notice_seen: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            notice_is_info: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_save: Arc::new(Mutex::new(None)),
            announced: Arc::new(Mutex::new(Vec::new())),
        };
        // Owner-only before anything is read or written: the list holds URLs, which can carry a
        // token, and the folders they went to.
        if let Err(e) = private_folder(&state_dir) {
            engine.set_notice(format!("cannot keep {} private: {e}", state_dir.display()));
        }
        engine.load();
        engine
    }

    /// What the workers have to tell the person since the last call, oldest first.
    pub fn announcements(&self) -> Vec<Notification> {
        self.announced.lock().map(|mut a| std::mem::take(&mut *a)).unwrap_or_default()
    }

    pub(super) fn announce(&self, notification: Notification) {
        if let Ok(mut announced) = self.announced.lock() {
            announced.push(notification);
        }
    }

    /// Whether any download is moving bytes or being hashed: what keeps the window's clock running
    /// and what a close has to wait for.
    pub fn working(&self) -> bool {
        self.state.lock().map(|s| s.items.iter().any(|d| d.status.is_active() || d.checksum_status == "verifying")).unwrap_or(false)
    }

    /// Whether any download is still moving bytes into a file: queued, or downloading.
    pub fn writing(&self) -> bool {
        self.state.lock().map(|s| s.items.iter().any(|d| d.status.is_active())).unwrap_or(false)
    }

    /// Where downloads land when the caller does not choose.
    pub fn default_dir(&self) -> &Path {
        &self.default_dir
    }

    /// The file the list is kept in, so `describe` can name it.
    pub fn state_path(&self) -> &Path {
        &self.store_path
    }

    /// What went wrong with the state file, or what it had to change on the way in. Empty when
    /// there is nothing to say.
    pub fn notice(&self) -> String {
        self.notice.lock().map(|n| n.clone()).unwrap_or_default()
    }

    /// The notice, if the window has not shown it yet. For the banner, not for `describe`.
    pub fn unseen_notice(&self) -> Option<String> {
        if self.notice_seen.load(Ordering::Relaxed) {
            return None;
        }
        Some(self.notice()).filter(|n| !n.is_empty())
    }

    /// The person has seen it and done something since. The notice itself stands.
    pub fn acknowledge_notice(&self) {
        self.notice_seen.store(true, Ordering::Relaxed);
    }

    /// Whether the notice is information rather than a fault.
    pub fn notice_is_info(&self) -> bool {
        self.notice_is_info.load(Ordering::Relaxed)
    }

    fn set_notice(&self, text: impl Into<String>) {
        self.set_notice_as(text, false);
    }

    fn set_notice_as(&self, text: impl Into<String>, info: bool) {
        let text = text.into();
        let raise = !text.is_empty();
        self.notice_is_info.store(info, Ordering::Relaxed);
        if let Ok(mut notice) = self.notice.lock() {
            *notice = text;
        }
        if raise {
            self.notice_seen.store(false, Ordering::Relaxed);
        }
        self.touch();
    }

    /// Bumped on every change. The window redraws when this moves and not otherwise.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn touch(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// Every download, in the order they were added.
    pub fn snapshot(&self) -> Vec<Download> {
        self.state.lock().map(|s| s.items.clone()).unwrap_or_default()
    }

    pub fn get(&self, id: i32) -> Option<Download> {
        let state = self.state.lock().ok()?;
        state.items.iter().find(|d| d.id == id).cloned()
    }

    /// Edit one download in place and bump the revision. `false` if there is no such id.
    fn edit(&self, id: i32, f: impl FnOnce(&mut Download)) -> bool {
        let found = match self.state.lock() {
            Ok(mut state) => match state.items.iter_mut().find(|d| d.id == id) {
                Some(item) => {
                    f(item);
                    true
                }
                None => false,
            },
            Err(_) => false,
        };
        if found {
            self.touch();
        }
        found
    }

    fn control_for(&self, id: i32) -> Option<Arc<AtomicU8>> {
        self.state.lock().ok()?.controls.get(&id).cloned()
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// The header line's numbers.
#[derive(Clone, Copy, Debug, Default)]
pub struct Totals {
    pub active: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
    /// Finished once, and the file is not where it was recorded.
    pub missing: usize,
    /// How many of these rows came back from the last run rather than being queued in this one.
    pub restored: usize,
    pub speed_bps: f64,
}

mod commands;
mod fetch;
mod identity;
mod plain;
mod store;
mod transfer;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_safety;

pub use plain::*;
pub use store::state_dir;
pub(crate) use fetch::literal_refusal;
#[cfg(test)]
pub(crate) use fetch::listed;
#[allow(unused_imports)]
use identity::*;
#[allow(unused_imports)]
use store::*;
