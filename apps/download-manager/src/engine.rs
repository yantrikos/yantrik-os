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
// Telling the person a transfer ended is fire-and-forget on a thread of its own, so it is safe
// from a worker and costs this one nothing — see `yantrik_app_runtime::notify`.
use yantrik_app_runtime::notify;
use yantrik_app_runtime::serde_json;

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
        let engine = Self {
            state: Arc::new(Mutex::new(State::default())),
            next_id: Arc::new(AtomicI32::new(1)),
            revision: Arc::new(AtomicU64::new(0)),
            default_dir: default_download_dir(),
            store_path: state_dir.join("state.json"),
            notice: Arc::new(Mutex::new(String::new())),
            notice_seen: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            notice_is_info: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_save: Arc::new(Mutex::new(None)),
        };
        engine.load();
        engine
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

    // ── Keeping the list ────────────────────────────────────────────

    /// Read the saved list and reconcile it with the filesystem.
    ///
    /// Called once, from [`Engine::open`]. Every way this can go wrong ends with a working app
    /// holding an empty list and a notice saying what happened, because a downloads window that
    /// refuses to start is worse than one that has forgotten.
    fn load(&self) {
        let raw = match std::fs::read_to_string(&self.store_path) {
            Ok(raw) => raw,
            // Nothing saved yet is the ordinary first run, not a fault worth a notice.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                self.set_notice(format!("cannot read {}: {e}", self.store_path.display()));
                return;
            }
        };

        let (items, next_id) = match decode(&raw) {
            Ok(loaded) => loaded,
            Err(reason) => {
                self.keep_aside(&reason);
                return;
            }
        };

        let interrupted = items.iter().filter(|d| d.interrupted).count();
        let missing = items.iter().filter(|d| d.status == Status::Missing).count();
        if let Ok(mut state) = self.state.lock() {
            for item in &items {
                // A restored download has no worker, but pause, resume and cancel still have to
                // reach it, and they all go through this map.
                state.controls.insert(item.id, Arc::new(AtomicU8::new(control::RUN)));
            }
            state.items = items;
        }
        self.next_id.store(next_id, Ordering::Relaxed);
        self.touch();

        if interrupted > 0 || missing > 0 {
            // Said out loud rather than left for someone to notice in the list: these are the two
            // states where what is on screen differs from what the person last saw. Information,
            // not a fault: nothing was lost by this app, and every row still says what it is.
            self.set_notice_as(restored_notice(interrupted, missing), true);
        }
    }

    /// Move an unusable state file out of the way and say so.
    ///
    /// It is kept rather than deleted: it is the only record of what the person had, and a human
    /// or a mind can read it even when this build cannot. The app then starts empty, which is
    /// honest — it really does not know what was there.
    fn keep_aside(&self, reason: &str) {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let kept = self.store_path.with_file_name(format!(
            "{}.corrupt-{stamp}",
            self.store_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "state.json".into())
        ));
        match std::fs::rename(&self.store_path, &kept) {
            Ok(()) => self.set_notice(format!(
                "The saved download list could not be read ({reason}). It has been kept as {} and \
                 the list is starting empty.",
                kept.display()
            )),
            // The rename failing is worse than the parse failing: the next save will write over
            // the only copy. Say exactly that, rather than reporting a rescue that did not happen.
            Err(e) => self.set_notice(format!(
                "The saved download list could not be read ({reason}) and could not be moved aside \
                 ({e}). {} will be overwritten by the next change.",
                self.store_path.display()
            )),
        }
    }

    /// Write the list now.
    ///
    /// Called from every transition a person or a mind would notice — added, paused, resumed,
    /// completed, failed, cancelled, cleared, verified — and never from the read paths, so nothing
    /// here can be reached while the state lock is held.
    fn save(&self) {
        if let Ok(mut last) = self.last_save.lock() {
            *last = Some(Instant::now());
        }
        let items = self.snapshot();
        let next_id = self.next_id.load(Ordering::Relaxed);
        if let Err(e) = write_atomically(&self.store_path, &encode(&items, next_id)) {
            self.set_notice(format!("cannot save the download list to {}: {e}", self.store_path.display()));
        } else if self.notice().starts_with("cannot save") {
            // The disk came back. Leaving the old complaint up would have the window reporting a
            // failure that is no longer true.
            self.set_notice("");
        }
    }

    /// Write the list, but no more often than [`PROGRESS_SAVE_INTERVAL`].
    ///
    /// The only caller is the transfer loop's speed window, whose one changing field is a byte
    /// count that reconciliation re-measures anyway.
    fn save_progress(&self) {
        let due = match self.last_save.lock() {
            Ok(last) => last.map(|t| t.elapsed() >= PROGRESS_SAVE_INTERVAL).unwrap_or(true),
            Err(_) => false,
        };
        if due {
            self.save();
        }
    }

    /// The window is closing: stop every writer and put the list down.
    ///
    /// The workers are not waited for. One blocked on a socket read can take the whole connect
    /// timeout to reach its next chunk boundary, and holding the window open for that is worse
    /// than what happens instead: the record goes to disk still saying `downloading`, and the next
    /// start reconciles it against the partial file, which is exact.
    pub fn shutdown(&self) {
        self.pause_all();
        self.save();
    }

    /// Counts and combined speed, for the header line.
    pub fn totals(&self) -> Totals {
        let items = self.snapshot();
        Totals {
            active: items.iter().filter(|d| d.status.is_active()).count(),
            paused: items.iter().filter(|d| d.status == Status::Paused).count(),
            completed: items.iter().filter(|d| d.status == Status::Completed).count(),
            failed: items.iter().filter(|d| d.status == Status::Failed).count(),
            missing: items.iter().filter(|d| d.status == Status::Missing).count(),
            restored: items.iter().filter(|d| d.restored).count(),
            speed_bps: items
                .iter()
                .filter(|d| d.status == Status::Downloading)
                .map(|d| d.speed_bps)
                .sum(),
        }
    }

    // ── Commands ────────────────────────────────────────────────────

    /// Queue a URL and start fetching it. Returns the new id.
    ///
    /// Rejects anything that is not `http`/`https` rather than handing it to the transport and
    /// reporting whatever that says: a caller that mistyped a path deserves to be told it was a
    /// path, and a `file://` "download" would be a copy with none of this machinery's meaning.
    pub fn add(
        &self,
        url: &str,
        checksum: &str,
        save_dir: Option<&str>,
    ) -> Result<i32, String> {
        let url = url.trim();
        if url.is_empty() {
            return Err("no URL given".into());
        }
        let lower = url.to_ascii_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return Err(format!("`{url}` is not an http(s) URL"));
        }

        let dir = match save_dir.map(str::trim).filter(|s| !s.is_empty()) {
            Some(d) => expand_home(d),
            None => self.default_dir.clone(),
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot use {}: {e}", dir.display()))?;

        let filename = unique_in(&dir, &filename_from_url(url));
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let download = Download {
            id,
            url: url.to_string(),
            filename,
            save_dir: dir,
            status: Status::Queued,
            downloaded: 0,
            total: None,
            speed_bps: 0.0,
            eta_secs: None,
            checksum_expected: checksum.trim().to_ascii_lowercase(),
            checksum_status: "none".into(),
            file_hash: String::new(),
            content_type: String::new(),
            started_at: now_stamp(),
            ended_at: String::new(),
            error: String::new(),
            resume_supported: false,
            selected: false,
            restored: false,
            interrupted: false,
        };

        {
            let mut state = self.state.lock().map_err(|_| "engine is poisoned".to_string())?;
            state.items.push(download);
            state.controls.insert(id, Arc::new(AtomicU8::new(control::RUN)));
        }
        self.touch();
        // Written before the thread starts. A crash one second into a 4 GB transfer must still
        // leave a record naming the URL, or the partial file beside it means nothing.
        self.save();
        self.spawn(id, 0);
        Ok(id)
    }

    /// Stop writing at the next chunk, keeping the partial file.
    pub fn pause(&self, id: i32) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        if !download.status.is_active() {
            return Err(format!("download {id} is {}, not running", download.status.as_str()));
        }
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::PAUSE, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Continue a paused or failed download from the bytes already on disk.
    pub fn resume(&self, id: i32) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        match download.status {
            Status::Paused | Status::Failed => {}
            other => return Err(format!("download {id} is {}, nothing to resume", other.as_str())),
        }
        let have = std::fs::symlink_metadata(download.path()).map(|m| m.len()).unwrap_or(0);
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::RUN, Ordering::Relaxed);
        }
        self.edit(id, |d| {
            d.status = Status::Queued;
            d.error.clear();
            d.downloaded = have;
            d.interrupted = false;
        });
        self.save();
        self.spawn(id, have);
        Ok(())
    }

    /// Stop and discard the partial file.
    ///
    /// The record stays in the list as `failed` with the reason, because a download that vanished
    /// from the window tells the person nothing about why their file is not there.
    pub fn cancel(&self, id: i32) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::CANCEL, Ordering::Relaxed);
        }
        // A running worker removes the file itself when it sees the flag; a paused or finished one
        // has no worker left to do it, so do it here.
        if !download.status.is_active() {
            let _ = std::fs::remove_file(download.path());
            self.edit(id, |d| {
                d.status = Status::Failed;
                d.error = "cancelled".into();
                d.ended_at = now_stamp();
                d.speed_bps = 0.0;
                d.eta_secs = None;
                d.interrupted = false;
            });
            self.save();
        }
        Ok(())
    }

    /// Start a failed download again from nothing.
    pub fn retry(&self, id: i32) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        if download.status.is_active() {
            return Err(format!("download {id} is already running"));
        }
        let _ = std::fs::remove_file(download.path());
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::RUN, Ordering::Relaxed);
        }
        self.edit(id, |d| {
            d.status = Status::Queued;
            d.downloaded = 0;
            d.error.clear();
            d.file_hash.clear();
            d.checksum_status = "none".into();
            d.ended_at.clear();
            d.started_at = now_stamp();
            d.interrupted = false;
        });
        self.save();
        self.spawn(id, 0);
        Ok(())
    }

    /// Hash the finished file again and compare it with the expected checksum.
    ///
    /// Worth having separately from the automatic check because the expected hash often arrives
    /// *after* the file — you download the ISO, then find the signature page.
    pub fn verify(&self, id: i32, expected: Option<&str>) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        if download.status != Status::Completed {
            return Err(format!(
                "download {id} is {}; there is nothing finished to verify",
                download.status.as_str()
            ));
        }
        if let Some(expected) = expected {
            let expected = expected.trim().to_ascii_lowercase();
            self.edit(id, |d| d.checksum_expected = expected);
        }
        self.edit(id, |d| d.checksum_status = "verifying".into());
        // An expected hash the caller has just supplied is worth keeping even if the hashing
        // thread never gets to run.
        self.save();

        let engine = self.clone();
        std::thread::Builder::new()
            .name(format!("download-verify-{id}"))
            .spawn(move || engine.hash_and_compare(id))
            .map_err(|e| format!("cannot start verifier: {e}"))?;
        Ok(())
    }

    /// Drop finished downloads from the list. The files stay where they are.
    ///
    /// Only `completed` goes. A `missing` row is not cleared by this: it is the record of a file
    /// that was fetched and is now gone, which is the one thing in the list the person may still
    /// need to be told.
    pub fn clear_completed(&self) -> usize {
        let removed = match self.state.lock() {
            Ok(mut state) => {
                let before = state.items.len();
                let doomed: Vec<i32> = state
                    .items
                    .iter()
                    .filter(|d| d.status == Status::Completed)
                    .map(|d| d.id)
                    .collect();
                state.items.retain(|d| d.status != Status::Completed);
                for id in doomed {
                    state.controls.remove(&id);
                }
                before - state.items.len()
            }
            Err(_) => 0,
        };
        if removed > 0 {
            self.touch();
            self.save();
        }
        removed
    }

    pub fn pause_all(&self) {
        for download in self.snapshot().iter().filter(|d| d.status.is_active()) {
            let _ = self.pause(download.id);
        }
    }

    pub fn resume_all(&self) {
        for download in self.snapshot().iter().filter(|d| d.status == Status::Paused) {
            let _ = self.resume(download.id);
        }
    }

    pub fn set_selected(&self, id: i32, selected: bool) {
        self.edit(id, |d| d.selected = selected);
    }

    pub fn select_all(&self, selected: bool) {
        if let Ok(mut state) = self.state.lock() {
            for item in state.items.iter_mut() {
                item.selected = selected;
            }
        }
        self.touch();
    }

    // ── The worker ──────────────────────────────────────────────────

    fn spawn(&self, id: i32, resume_from: u64) {
        let engine = self.clone();
        let started = std::thread::Builder::new()
            .name(format!("download-{id}"))
            .spawn(move || engine.transfer(id, resume_from));
        if let Err(e) = started {
            self.fail(id, format!("cannot start download thread: {e}"));
        }
    }

    fn fail(&self, id: i32, reason: impl Into<String>) {
        let reason = reason.into();
        self.edit(id, |d| {
            d.status = Status::Failed;
            d.error = reason.clone();
            d.speed_bps = 0.0;
            d.eta_secs = None;
            d.ended_at = now_stamp();
            d.interrupted = false;
        });
        self.save();

        // A transfer that failed while the window was behind something, or closed, was a red
        // row nobody ever looked at. Every failure path in this file funnels through here, with
        // one deliberate exception: cancelling sets `Failed` inline a few hundred lines below,
        // because a person who pressed Cancel does not need to be told it stopped.
        if let Some(name) = self.get(id).map(|d| d.filename) {
            notify::send(notify::Notification::new("Downloads", format!("{name} failed")).body(reason));
        }
    }

    /// One transfer, start to finish, on its own thread.
    fn transfer(&self, id: i32, resume_from: u64) {
        let Some(download) = self.get(id) else { return };
        let Some(ctl) = self.control_for(id) else { return };
        let path = download.path();

        let mut request = ureq::get(&download.url);
        if resume_from > 0 {
            request = request.set("Range", &format!("bytes={resume_from}-"));
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => {
                self.fail(id, format!("server said HTTP {code}"));
                return;
            }
            Err(e) => {
                self.fail(id, format!("{e}"));
                return;
            }
        };

        // A ranged request answered 200 means the body is the whole file, not the remainder.
        let appending = resume_from > 0 && response.status() == 206;
        let resume_supported = response
            .header("accept-ranges")
            .map(|v| v.to_ascii_lowercase().contains("bytes"))
            .unwrap_or(response.status() == 206);
        let content_type = response
            .header("content-type")
            .and_then(|v| v.split(';').next())
            .unwrap_or("")
            .trim()
            .to_string();
        let body_len = response.header("content-length").and_then(|v| v.parse::<u64>().ok());
        let start_at = if appending { resume_from } else { 0 };
        // `content-length` is the length of *this* body — the remainder when the server honoured
        // the range, the whole file when it did not. `start_at` is zero in the second case, so one
        // expression covers both.
        let total = body_len.map(|len| start_at + len);

        // Never through a link: a link put where the download lands would send the bytes to
        // wherever it points, past the rule that chose this place (agent_rule.rs).
        let file = if appending {
            no_follow(OpenOptions::new().append(true)).open(&path)
        } else {
            no_follow(OpenOptions::new().write(true).create(true).truncate(true)).open(&path)
        };
        let mut file = match file {
            Ok(file) => file,
            Err(e) => {
                self.fail(id, format!("cannot write {}: {e}", path.display()));
                return;
            }
        };

        self.edit(id, |d| {
            d.status = Status::Downloading;
            d.downloaded = start_at;
            d.total = total;
            d.content_type = content_type;
            d.resume_supported = resume_supported;
            d.error.clear();
            d.interrupted = false;
        });
        // The size and whether this server honours a range are the two things a later resume
        // depends on, and both are known only now.
        self.save();

        let mut reader = response.into_reader();
        let mut buffer = vec![0u8; CHUNK];
        let mut written = start_at;
        let mut window_start = Instant::now();
        let mut window_bytes = 0u64;

        loop {
            match ctl.load(Ordering::Relaxed) {
                control::PAUSE => {
                    let _ = file.flush();
                    self.edit(id, |d| {
                        d.status = Status::Paused;
                        d.downloaded = written;
                        d.speed_bps = 0.0;
                        d.eta_secs = None;
                    });
                    // Always saved, never throttled: a pause is the state most likely to be the
                    // last thing that happens before the process goes away.
                    self.save();
                    return;
                }
                control::CANCEL => {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    self.edit(id, |d| {
                        d.status = Status::Failed;
                        d.error = "cancelled".into();
                        d.speed_bps = 0.0;
                        d.eta_secs = None;
                        d.ended_at = now_stamp();
                    });
                    self.save();
                    return;
                }
                _ => {}
            }

            let read = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    let _ = file.flush();
                    self.fail(id, format!("transfer broke: {e}"));
                    return;
                }
            };
            if let Err(e) = file.write_all(&buffer[..read]) {
                self.fail(id, format!("cannot write {}: {e}", path.display()));
                return;
            }
            written += read as u64;
            window_bytes += read as u64;

            let elapsed = window_start.elapsed();
            if elapsed >= SPEED_WINDOW {
                let speed = window_bytes as f64 / elapsed.as_secs_f64();
                let eta = total.and_then(|total| {
                    if speed > 0.0 && total > written {
                        Some(((total - written) as f64 / speed) as u64)
                    } else {
                        None
                    }
                });
                self.edit(id, |d| {
                    d.downloaded = written;
                    d.speed_bps = speed;
                    d.eta_secs = eta;
                });
                self.save_progress();
                window_start = Instant::now();
                window_bytes = 0;
            }
        }

        if let Err(e) = file.flush() {
            self.fail(id, format!("cannot finish writing {}: {e}", path.display()));
            return;
        }
        drop(file);

        // A truncated transfer that ended cleanly is still a broken file, and saying "completed"
        // over it would be the confident lie the surface rules warn about.
        if let Some(total) = total {
            if written < total {
                self.fail(
                    id,
                    format!("ended early — {} of {}", format_bytes(written), format_bytes(total)),
                );
                return;
            }
        }

        self.edit(id, |d| {
            d.status = Status::Completed;
            d.downloaded = written;
            d.total = Some(written);
            d.speed_bps = 0.0;
            d.eta_secs = None;
            d.ended_at = now_stamp();
            d.interrupted = false;
            if !d.checksum_expected.is_empty() {
                d.checksum_status = "verifying".into();
            }
        });
        self.save();
        self.hash_and_compare(id);

        // Told after hashing, not before: on a file with an expected checksum the interesting
        // news is whether it matched, and a "finished" that arrives a second before "checksum
        // failed" invites somebody to open a file we are about to say is wrong.
        if let Some(d) = self.get(id) {
            let (title, body) = match d.checksum_status.as_str() {
                "fail" => (
                    format!("{} finished, but the checksum does not match", d.filename),
                    format!("Expected {}, got {}", d.checksum_expected, d.file_hash),
                ),
                _ => (
                    format!("{} finished", d.filename),
                    format!("Saved to {}", d.save_dir.display()),
                ),
            };
            // "Open folder" is a real action on this app's own control surface, taking this id
            // — the same call the button in the window makes. The shell presses it on our
            // behalf, starting this app again if the transfer outlived the window, which is the
            // usual case for a long download.
            notify::send(
                notify::Notification::new("Downloads", title)
                    .body(body)
                    .action_with("open_folder", "Open folder", serde_json::json!({ "id": id })),
            );
        }
    }

    /// Hash the file on disk, record it, and compare it with the expected value if there is one.
    ///
    /// The hash is recorded whether or not anything was expected: an agent that fetched a file
    /// usually needs its digest next, and computing it here means it never has to shell out.
    fn hash_and_compare(&self, id: i32) {
        let Some(download) = self.get(id) else { return };
        let digest = match sha256_file(&download.path()) {
            Ok(digest) => digest,
            Err(e) => {
                self.edit(id, |d| {
                    d.checksum_status = if d.checksum_expected.is_empty() { "none".into() } else { "fail".into() };
                    d.error = format!("cannot hash the file: {e}");
                });
                self.save();
                return;
            }
        };
        self.edit(id, |d| {
            d.file_hash = digest.clone();
            d.checksum_status = if d.checksum_expected.is_empty() {
                "none".into()
            } else if d.checksum_expected == digest {
                "pass".into()
            } else {
                "fail".into()
            };
        });
        // The digest is the answer an agent came for, and recomputing it on the next start would
        // mean re-reading a file that may be gigabytes.
        self.save();
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

// ── The state file ──────────────────────────────────────────────────

/// The whole file. `version` comes first so a reader that does not understand this shape can find
/// that out without having to parse the rest.
#[derive(Serialize, Deserialize)]
struct StoredState {
    version: u32,
    /// Kept so a restart does not hand a new download an id the list already used — the id is
    /// what every caller and every button refers to.
    next_id: i32,
    downloads: Vec<StoredDownload>,
}

/// One download as it goes to disk.
///
/// Only the durable half. Speed, ETA and the selection checkbox are facts about a moment and
/// writing them would be recording a measurement that is false by the time it is read.
#[derive(Serialize, Deserialize)]
struct StoredDownload {
    id: i32,
    url: String,
    filename: String,
    save_dir: String,
    status: String,
    // The rest default rather than refuse: a record that still names a file and a URL is worth
    // keeping even if a field was added since it was written. Identity is not optional, because a
    // record with no id or no path cannot be acted on at all.
    #[serde(default)]
    downloaded: u64,
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    checksum_expected: String,
    #[serde(default)]
    checksum_status: String,
    #[serde(default)]
    file_hash: String,
    #[serde(default)]
    content_type: String,
    #[serde(default)]
    started_at: String,
    #[serde(default)]
    ended_at: String,
    #[serde(default)]
    error: String,
    #[serde(default)]
    resume_supported: bool,
}

/// Where the list lives between runs.
///
/// `~/.local/share/yantrik/downloads`, beside every other app's store, and overridable by
/// `YANTRIK_DOWNLOADS_DIR` the way Notes takes `YANTRIK_NOTES_DIR` — the conformance probe needs
/// somewhere to work that is not the person's real list.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("YANTRIK_DOWNLOADS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join(".local/share/yantrik/downloads")
}

/// The list as JSON.
///
/// Pretty-printed on purpose: this is a file a person or a mind may have to read by hand on the
/// day the app will not start, and it is a few hundred bytes a download.
fn encode(items: &[Download], next_id: i32) -> String {
    let stored = StoredState {
        version: STATE_VERSION,
        next_id,
        downloads: items
            .iter()
            .map(|d| StoredDownload {
                id: d.id,
                url: d.url.clone(),
                filename: d.filename.clone(),
                save_dir: d.save_dir.to_string_lossy().to_string(),
                status: d.status.as_str().to_string(),
                downloaded: d.downloaded,
                total: d.total,
                checksum_expected: d.checksum_expected.clone(),
                checksum_status: d.checksum_status.clone(),
                file_hash: d.file_hash.clone(),
                content_type: d.content_type.clone(),
                started_at: d.started_at.clone(),
                ended_at: d.ended_at.clone(),
                error: d.error.clone(),
                resume_supported: d.resume_supported,
            })
            .collect(),
    };
    // A serializer failing on a struct of owned strings and integers is not a thing that happens,
    // but the fallback is still a valid empty file rather than a panic in a save path.
    serde_json::to_string_pretty(&stored)
        .unwrap_or_else(|_| format!("{{\"version\":{STATE_VERSION},\"next_id\":1,\"downloads\":[]}}"))
}

/// Read a saved list and reconcile it with what is actually on the filesystem.
///
/// The returned records are what the app will show, not what the file said. Three rules, and all
/// three exist because trusting the file would mean showing the person something that is not true:
///
/// - A download that was still running when the process died comes back **paused and interrupted**
///   with the byte count measured off its partial file. It is not restarted: a transfer resuming
///   by itself because a window opened is a surprise, and on a metered connection an expensive
///   one. `resume` continues it from exactly those bytes.
/// - A download recorded as **completed** whose file is no longer there comes back as
///   [`Status::Missing`], because the whole value of a completed row is the path it points at.
/// - A paused or failed record's byte count is re-measured too — the partial file may have been
///   deleted or truncated by something else between runs.
///
/// Nothing is written and nothing is removed. The directory is never scanned either, so a stray
/// partial file whose record has gone is simply left alone; a forgotten record is not permission
/// to delete somebody's data.
///
/// `Err` means the file cannot be used at all and should be kept aside.
fn decode(raw: &str) -> Result<(Vec<Download>, i32), String> {
    let stored: StoredState =
        serde_json::from_str(raw).map_err(|e| format!("it is not valid state JSON: {e}"))?;
    if stored.version != STATE_VERSION {
        return Err(format!(
            "it is version {}, and this build reads version {STATE_VERSION}",
            stored.version
        ));
    }

    let mut items = Vec::with_capacity(stored.downloads.len());
    for record in stored.downloads {
        let Some(status) = Status::parse(&record.status) else {
            // One unreadable row is not a reason to lose the other nineteen. It is dropped rather
            // than guessed at, and the count the caller sees is of what actually loaded.
            continue;
        };
        let mut download = Download {
            id: record.id,
            url: record.url,
            filename: record.filename,
            save_dir: PathBuf::from(record.save_dir),
            status,
            downloaded: record.downloaded,
            total: record.total,
            speed_bps: 0.0,
            eta_secs: None,
            checksum_expected: record.checksum_expected,
            checksum_status: if record.checksum_status.is_empty() {
                "none".to_string()
            } else {
                record.checksum_status
            },
            file_hash: record.file_hash,
            content_type: record.content_type,
            started_at: record.started_at,
            ended_at: record.ended_at,
            error: record.error,
            resume_supported: record.resume_supported,
            selected: false,
            restored: true,
            interrupted: false,
        };

        let on_disk = std::fs::metadata(download.path()).map(|m| m.len()).ok();
        match status {
            Status::Queued | Status::Downloading => {
                download.status = Status::Paused;
                download.interrupted = true;
                download.downloaded = on_disk.unwrap_or(0);
            }
            Status::Paused | Status::Failed => {
                download.downloaded = on_disk.unwrap_or(0);
            }
            Status::Completed => {
                if on_disk.is_none() {
                    download.status = Status::Missing;
                    download.error =
                        format!("the file is no longer at {}", download.path().display());
                }
            }
            Status::Missing => {
                // A file that was put back is completed again. The alternative — a row that says
                // `missing` over a file that is plainly there — is the same class of lie pointing
                // the other way.
                if on_disk.is_some() {
                    download.status = Status::Completed;
                    download.error.clear();
                }
            }
        }
        // A `verifying` row belongs to a hashing thread that no longer exists. What is true now is
        // that nothing has been checked since.
        if download.checksum_status == "verifying" {
            download.checksum_status = "none".into();
        }
        items.push(download);
    }

    let highest = items.iter().map(|d| d.id).max().unwrap_or(0);
    let next_id = stored.next_id.max(highest + 1).max(1);
    Ok((items, next_id))
}

/// Replace a file in one step, or leave the old one exactly as it was.
///
/// Writing over the live file would mean a power cut mid-write leaves a half-written list, which
/// is precisely the unreadable file the corrupt path then has to rescue. The temp name carries the
/// pid so two processes cannot scribble over each other's half-written copy, and it sits in the
/// same directory so the rename stays on one filesystem — a rename across devices is a copy, and a
/// copy is not atomic.
fn write_atomically(path: &Path, body: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let temp = dir.join(format!(
        "{}.tmp-{}",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "state.json".into()),
        std::process::id()
    ));
    let write = (|| -> std::io::Result<()> {
        let mut file = File::create(&temp)?;
        file.write_all(body.as_bytes())?;
        // The rename is only atomic with respect to a file whose bytes have landed.
        file.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&temp, path) {
        // Leaving the temp file behind would have the next start find a directory full of
        // half-written lists and no way to tell which is which.
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

// ── Plain functions ─────────────────────────────────────────────────

/// Which rows the window shows, in the order it shows them.
///
/// Kept out of the window because it is the one piece of display logic with edge cases worth
/// asserting — a filter and a search box that disagree, a sort over a size nobody reported — and a
/// function taking a slice can be tested where one reading Slint properties cannot.
///
/// `filter` is the tab: 0 all, 1 active, 2 completed, 3 failed. `sort` is 0 date, 1 name, 2 size,
/// 3 status, matching the numbers the UI already sends.
pub fn visible_rows(items: &[Download], filter: i32, query: &str, sort: i32) -> Vec<Download> {
    let query = query.trim().to_lowercase();
    let mut rows: Vec<Download> = items
        .iter()
        .filter(|d| match filter {
            1 => d.status.is_active() || d.status == Status::Paused,
            2 => d.status == Status::Completed,
            // The "Failed" tab is where a person goes to find what went wrong, and a file that
            // finished and then vanished belongs there rather than nowhere.
            3 => d.status == Status::Failed || d.status == Status::Missing,
            _ => true,
        })
        .filter(|d| {
            query.is_empty()
                || d.filename.to_lowercase().contains(&query)
                || d.url.to_lowercase().contains(&query)
        })
        .cloned()
        .collect();

    match sort {
        1 => rows.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase())),
        // Largest first: the reason to sort a download list by size is to find the big one.
        2 => rows.sort_by(|a, b| b.total.unwrap_or(b.downloaded).cmp(&a.total.unwrap_or(a.downloaded))),
        3 => rows.sort_by(|a, b| a.status.as_str().cmp(b.status.as_str())),
        // Newest first, which is what a downloads list is for.
        _ => rows.sort_by(|a, b| b.id.cmp(&a.id)),
    }
    rows
}

/// SHA-256 of a file, streamed so the size of the file does not become the size of a buffer.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Where a download goes when nobody says: `~/Downloads`, the same place every other desktop
/// puts it, so a file an agent fetched is where the person would look for it.
pub fn default_download_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&home).join("Downloads")
}

/// Open options that refuse a path whose last part is a link (`O_NOFOLLOW`).
fn no_follow(options: &mut OpenOptions) -> &mut OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
}

/// `~/x` means what it does in a shell; anything else is taken as given.
pub fn expand_home(dir: &str) -> PathBuf {
    if let Some(rest) = dir.strip_prefix("~/").or_else(|| dir.strip_prefix("~\\")) {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Path::new(&home).join(rest)
    } else if dir == "~" {
        PathBuf::from(
            std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string()),
        )
    } else {
        PathBuf::from(dir)
    }
}

/// The name to save a URL under.
///
/// The last non-empty path segment, minus query and fragment, with anything that is not obviously
/// safe in a filename replaced. A URL that ends in a slash, or carries a name made entirely of
/// separators, gets `download` rather than an empty name or a dotfile.
pub fn filename_from_url(url: &str) -> String {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let path = without_scheme.split(['?', '#']).next().unwrap_or("");
    // Skip the host: a bare `https://example.com` must not save as `example.com`.
    let last = path.split('/').skip(1).filter(|s| !s.is_empty()).last().unwrap_or("");
    let cleaned: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '(' | ')' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "download".to_string()
    } else {
        cleaned
    }
}

/// A name that is not already taken in `dir`, by the `name (2).ext` convention.
///
/// Downloading over a file the person already has would be the one irreversible thing this app
/// could do by accident, so it does not.
pub fn unique_in(dir: &Path, name: &str) -> String {
    // `symlink_metadata`, not `exists`: a link that points at nothing is not a free name. Taken
    // as one, the transfer would write through it to wherever it points.
    let taken = |n: &str| dir.join(n).symlink_metadata().is_ok();
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        // A leading dot is the whole name of a dotfile, not a separator.
        Some(index) if index > 0 => (&name[..index], &name[index..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = format!("{stem} ({n}){extension}");
        if !taken(&candidate) {
            return candidate;
        }
    }
    format!("{stem} ({}){extension}", std::process::id())
}

/// `1.4 KB`, `45.2 MB` — one decimal, because two is noise at these magnitudes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `45s`, `2m 30s`, `1h 05m` — as long as it is useful and no longer.
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// `one` or `many` by count. A person reads "1 download is" and "6 downloads are", never
/// "file(s)": the hedge makes them do the grammar the app already knew.
pub fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// What a restored list says about the rows that came back different from how they were left.
fn restored_notice(interrupted: usize, missing: usize) -> String {
    let mut text = String::from("Restored the download list.");
    if interrupted > 0 {
        text.push_str(&format!(
            " {interrupted} {} interrupted by the last shutdown and {} now paused.",
            plural(interrupted, "download was", "downloads were"),
            plural(interrupted, "is", "are"),
        ));
    }
    if missing > 0 {
        text.push_str(&format!(
            " {missing} finished {} no longer on disk.",
            plural(missing, "download is", "downloads are"),
        ));
    }
    text
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static NEXT_STORE: AtomicUsize = AtomicUsize::new(0);

    /// A state directory of this test's own.
    ///
    /// Every test that builds an [`Engine`] goes through one: an engine made with `new()` would
    /// read and then write the person's real download list, and a test that can damage the
    /// machine it runs on is not a test anybody will keep running.
    struct Store(PathBuf);

    impl Store {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yantrik-dl-state-{}-{}",
                std::process::id(),
                NEXT_STORE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn state_file(&self) -> PathBuf {
            self.0.join("state.json")
        }

        fn engine(&self) -> Engine {
            Engine::open(self.0.clone())
        }

        /// Put a file of `bytes` zeroes where a download of that name would have written it.
        fn part(&self, name: &str, bytes: usize) {
            std::fs::write(self.0.join(name), vec![0u8; bytes]).unwrap();
        }

        /// Names in the directory, sorted, so an assertion can be about the whole directory.
        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// One record as it would be after a real transfer, saving into `dir`.
    fn saved(id: i32, filename: &str, status: Status, dir: &Path, downloaded: u64, total: u64) -> Download {
        Download {
            id,
            url: format!("https://example.com/{filename}"),
            filename: filename.into(),
            save_dir: dir.to_path_buf(),
            status,
            downloaded,
            total: Some(total),
            speed_bps: 0.0,
            eta_secs: None,
            checksum_expected: String::new(),
            checksum_status: "none".into(),
            file_hash: String::new(),
            content_type: "application/octet-stream".into(),
            started_at: "2026-09-20 10:00:00".into(),
            ended_at: String::new(),
            error: String::new(),
            resume_supported: true,
            selected: false,
            restored: false,
            interrupted: false,
        }
    }

    #[test]
    fn names_a_file_after_the_last_path_segment() {
        assert_eq!(filename_from_url("https://example.com/dist/yantrik.iso"), "yantrik.iso");
        assert_eq!(filename_from_url("https://example.com/a/b/c.tar.gz?sig=1#x"), "c.tar.gz");
    }

    #[test]
    fn never_saves_a_file_under_the_host_name() {
        // A bare host has no path segment to take; `example.com` would look like a real filename
        // and be wrong every time.
        assert_eq!(filename_from_url("https://example.com"), "download");
        assert_eq!(filename_from_url("https://example.com/"), "download");
    }

    #[test]
    fn strips_what_cannot_go_in_a_filename() {
        assert_eq!(filename_from_url("https://x.dev/a:b*c?q=1"), "a_b_c");
        assert_eq!(filename_from_url("https://x.dev/%2e%2e/etc/passwd"), "passwd");
        assert_eq!(filename_from_url("https://x.dev/..."), "download");
    }

    #[test]
    fn sidesteps_a_name_already_on_disk() {
        let dir = std::env::temp_dir().join(format!("yantrik-dl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_in(&dir, "a.txt"), "a.txt");

        File::create(dir.join("a.txt")).unwrap();
        assert_eq!(unique_in(&dir, "a.txt"), "a (1).txt");

        File::create(dir.join("a (1).txt")).unwrap();
        assert_eq!(unique_in(&dir, "a.txt"), "a (2).txt");

        File::create(dir.join("noext")).unwrap();
        assert_eq!(unique_in(&dir, "noext"), "noext (1)");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_never_a_free_name_and_a_download_never_writes_through_one() {
        let dir = std::env::temp_dir().join(format!("yantrik-dl-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let elsewhere = dir.join("elsewhere.desktop");
        std::os::unix::fs::symlink(&elsewhere, dir.join("a.desktop")).unwrap();
        assert_eq!(unique_in(&dir, "a.desktop"), "a (1).desktop", "a dangling link is taken");
        let refused = no_follow(OpenOptions::new().write(true).create(true).truncate(true))
            .open(dir.join("a.desktop"));
        assert!(refused.is_err(), "the open refuses a link");
        assert!(!elsewhere.exists(), "nothing was written where the link points");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_what_is_not_a_web_url() {
        let store = Store::new();
        let engine = store.engine();
        assert!(engine.add("", "", None).is_err());
        assert!(engine.add("/etc/passwd", "", None).unwrap_err().contains("not an http"));
        assert!(engine.add("file:///etc/passwd", "", None).is_err());
        assert!(engine.add("ftp://example.com/x", "", None).is_err());
    }

    #[test]
    fn names_the_download_a_command_could_not_find() {
        let store = Store::new();
        let engine = store.engine();
        assert!(engine.pause(99).unwrap_err().contains("no download 99"));
        assert!(engine.resume(99).is_err());
        assert!(engine.cancel(99).is_err());
        assert!(engine.retry(99).is_err());
        assert!(engine.verify(99, None).is_err());
    }

    #[test]
    fn formats_sizes_and_times_the_way_the_window_shows_them() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(150), "2m 30s");
        assert_eq!(format_duration(3960), "1h 06m");
    }

    #[test]
    fn reports_no_percentage_when_the_server_gave_no_length() {
        let mut download = Download {
            id: 1,
            url: "https://x.dev/a".into(),
            filename: "a".into(),
            save_dir: PathBuf::from("/tmp"),
            status: Status::Downloading,
            downloaded: 4096,
            total: None,
            speed_bps: 1024.0,
            eta_secs: None,
            checksum_expected: String::new(),
            checksum_status: "none".into(),
            file_hash: String::new(),
            content_type: String::new(),
            started_at: String::new(),
            ended_at: String::new(),
            error: String::new(),
            resume_supported: false,
            selected: false,
            restored: false,
            interrupted: false,
        };
        assert_eq!(download.progress(), 0.0);
        assert_eq!(download.size_text(), "4.0 KB");
        assert_eq!(download.eta_text(), "");

        download.total = Some(8192);
        assert_eq!(download.progress(), 0.5);
        assert_eq!(download.size_text(), "4.0 KB / 8.0 KB");
    }

    fn row(id: i32, filename: &str, status: Status, total: Option<u64>) -> Download {
        Download {
            id,
            url: format!("https://example.com/{filename}"),
            filename: filename.into(),
            save_dir: PathBuf::from("/tmp"),
            status,
            downloaded: total.unwrap_or(0),
            total,
            speed_bps: 0.0,
            eta_secs: None,
            checksum_expected: String::new(),
            checksum_status: "none".into(),
            file_hash: String::new(),
            content_type: String::new(),
            started_at: String::new(),
            ended_at: String::new(),
            error: String::new(),
            resume_supported: false,
            selected: false,
            restored: false,
            interrupted: false,
        }
    }

    fn names(rows: &[Download]) -> Vec<&str> {
        rows.iter().map(|d| d.filename.as_str()).collect()
    }

    #[test]
    fn shows_newest_first_and_filters_by_tab() {
        let items = vec![
            row(1, "old.iso", Status::Completed, Some(10)),
            row(2, "running.zip", Status::Downloading, Some(20)),
            row(3, "broken.tar", Status::Failed, None),
            row(4, "held.bin", Status::Paused, Some(5)),
        ];
        assert_eq!(names(&visible_rows(&items, 0, "", 0)), ["held.bin", "broken.tar", "running.zip", "old.iso"]);
        // "Active" includes paused: a download the person stopped is still one they are dealing
        // with, and hiding it there is how a paused transfer gets forgotten.
        assert_eq!(names(&visible_rows(&items, 1, "", 0)), ["held.bin", "running.zip"]);
        assert_eq!(names(&visible_rows(&items, 2, "", 0)), ["old.iso"]);
        assert_eq!(names(&visible_rows(&items, 3, "", 0)), ["broken.tar"]);
    }

    #[test]
    fn searches_the_name_and_the_url_together() {
        let items = vec![
            row(1, "yantrik.iso", Status::Completed, Some(10)),
            row(2, "notes.zip", Status::Completed, Some(20)),
        ];
        assert_eq!(names(&visible_rows(&items, 0, "YANTRIK", 0)), ["yantrik.iso"]);
        assert_eq!(names(&visible_rows(&items, 0, "example.com", 0)).len(), 2);
        assert!(visible_rows(&items, 0, "nothing like it", 0).is_empty());
    }

    #[test]
    fn sorts_by_name_size_and_status() {
        let items = vec![
            row(1, "b.iso", Status::Downloading, Some(10)),
            row(2, "a.iso", Status::Completed, Some(900)),
            row(3, "c.iso", Status::Failed, None),
        ];
        assert_eq!(names(&visible_rows(&items, 0, "", 1)), ["a.iso", "b.iso", "c.iso"]);
        // The one with no reported size sorts last rather than being dropped or crashing.
        assert_eq!(names(&visible_rows(&items, 0, "", 2)), ["a.iso", "b.iso", "c.iso"]);
        assert_eq!(names(&visible_rows(&items, 0, "", 3)), ["a.iso", "b.iso", "c.iso"]);
    }

    #[test]
    fn hashes_a_file_the_way_sha256sum_does() {
        let path = std::env::temp_dir().join(format!("yantrik-dl-hash-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        std::fs::remove_file(&path).ok();
    }

    // ── Keeping the list ────────────────────────────────────────────

    #[test]
    fn the_list_comes_back_the_way_it_went_out() {
        let store = Store::new();
        store.part("done.iso", 400);
        store.part("held.bin", 120);

        let mut finished = saved(3, "done.iso", Status::Completed, &store.0, 400, 400);
        finished.checksum_expected = "abc".into();
        finished.checksum_status = "fail".into();
        finished.file_hash = "def".into();
        finished.ended_at = "2026-09-20 10:04:00".into();
        let held = saved(7, "held.bin", Status::Paused, &store.0, 120, 999);

        std::fs::write(store.state_file(), encode(&[finished.clone(), held.clone()], 8)).unwrap();

        let engine = store.engine();
        let back = engine.snapshot();
        assert_eq!(back.len(), 2);
        // Everything a later command needs: which URL, where it went, how far it got, what was
        // expected of it and what it actually hashed to.
        assert_eq!(back[0].url, finished.url);
        assert_eq!(back[0].status, Status::Completed);
        assert_eq!(back[0].downloaded, 400);
        assert_eq!(back[0].total, Some(400));
        assert_eq!(back[0].checksum_expected, "abc");
        assert_eq!(back[0].checksum_status, "fail");
        assert_eq!(back[0].file_hash, "def");
        assert_eq!(back[0].content_type, "application/octet-stream");
        assert_eq!(back[0].started_at, "2026-09-20 10:00:00");
        assert_eq!(back[0].ended_at, "2026-09-20 10:04:00");
        assert!(back[0].resume_supported);
        assert_eq!(back[1].id, 7);
        assert_eq!(back[1].status, Status::Paused);
        assert_eq!(back[1].downloaded, 120);
        assert_eq!(back[1].total, Some(999));
        // Both came from the file, and neither was running when the process last stopped.
        assert!(back.iter().all(|d| d.restored && !d.interrupted));
        assert_eq!(engine.totals().restored, 2);
    }

    #[test]
    fn a_transfer_that_was_running_comes_back_paused_at_the_bytes_on_disk() {
        let store = Store::new();
        // 90 bytes landed; the record was last written when 40 had.
        store.part("big.iso", 90);
        let running = saved(1, "big.iso", Status::Downloading, &store.0, 40, 1000);
        std::fs::write(store.state_file(), encode(&[running], 2)).unwrap();

        let engine = store.engine();
        let back = engine.snapshot();
        assert_eq!(back[0].status, Status::Paused, "a dead transfer must not resume by itself");
        assert!(back[0].interrupted, "and the caller has to be able to tell it apart from a pause");
        // The file is the measurement, not the record. Resume sends `Range: bytes=90-`; believing
        // the record's 40 would append the same 50 bytes twice and corrupt the result.
        assert_eq!(back[0].downloaded, 90);
        assert!(!engine.notice().is_empty(), "a changed row is said out loud, not left to be noticed");

        // And the restored row is a real download: the commands reach it.
        assert!(engine.pause(1).unwrap_err().contains("not running"));
        assert!(engine.get(1).is_some());
    }

    #[test]
    fn a_queued_transfer_whose_file_never_appeared_comes_back_at_zero() {
        let store = Store::new();
        let queued = saved(1, "never-started.iso", Status::Queued, &store.0, 0, 1000);
        std::fs::write(store.state_file(), encode(&[queued], 2)).unwrap();

        let back = store.engine().snapshot();
        assert_eq!(back[0].status, Status::Paused);
        assert!(back[0].interrupted);
        assert_eq!(back[0].downloaded, 0);
    }

    #[test]
    fn a_finished_file_that_is_gone_is_missing_rather_than_completed() {
        let store = Store::new();
        // No file written: the person moved or deleted it between runs.
        let finished = saved(4, "moved.iso", Status::Completed, &store.0, 400, 400);
        std::fs::write(store.state_file(), encode(&[finished], 5)).unwrap();

        let engine = store.engine();
        let back = engine.snapshot();
        assert_eq!(back[0].status, Status::Missing);
        assert!(back[0].error.contains("no longer at"), "and it says where it looked");
        assert_eq!(engine.totals().missing, 1);
        assert_eq!(engine.totals().completed, 0);
        // A missing row is a problem, so it is where a person looks for problems.
        assert_eq!(visible_rows(&back, 3, "", 0).len(), 1);
        assert_eq!(visible_rows(&back, 2, "", 0).len(), 0);
        // Clearing finished downloads does not sweep it away with them.
        assert_eq!(engine.clear_completed(), 0);
        assert_eq!(engine.snapshot().len(), 1);
        // Nothing finished means nothing to verify, and it says which state it is in.
        assert!(engine.verify(4, None).unwrap_err().contains("missing"));
        // Said as information, in whole words, not as a fault with a "file(s)" in it.
        assert_eq!(
            engine.notice(),
            "Restored the download list. 1 finished download is no longer on disk."
        );
        assert!(engine.notice_is_info());

        // Put the file back and the next start says completed again.
        store.part("moved.iso", 400);
        assert_eq!(store.engine().snapshot()[0].status, Status::Completed);
    }

    #[test]
    fn the_restored_notice_counts_in_words() {
        assert_eq!(
            restored_notice(0, 6),
            "Restored the download list. 6 finished downloads are no longer on disk."
        );
        assert_eq!(
            restored_notice(1, 2),
            "Restored the download list. 1 download was interrupted by the last shutdown and is now \
             paused. 2 finished downloads are no longer on disk."
        );
        assert!(!restored_notice(3, 0).contains("(s)"));
    }

    #[test]
    fn a_state_file_it_cannot_read_is_kept_aside_and_the_app_still_starts() {
        let store = Store::new();
        std::fs::write(store.state_file(), "{ this is not json").unwrap();

        let engine = store.engine();
        assert!(engine.snapshot().is_empty(), "an unreadable list starts empty, not broken");
        let notice = engine.notice();
        assert!(notice.contains("could not be read"), "and the app says so: {notice}");
        assert!(!engine.notice_is_info(), "a list it could not read is a fault, not information");

        let kept: Vec<String> = store
            .entries()
            .into_iter()
            .filter(|name| name.starts_with("state.json.corrupt-"))
            .collect();
        assert_eq!(kept.len(), 1, "the only copy of what the person had is not deleted");
        assert!(!store.state_file().exists(), "and it is out of the way of the next save");
        assert_eq!(
            std::fs::read_to_string(store.0.join(&kept[0])).unwrap(),
            "{ this is not json"
        );
    }

    #[test]
    fn the_file_carries_a_version_and_one_it_does_not_know_is_not_guessed_at() {
        let store = Store::new();
        let written = encode(&[saved(1, "a.iso", Status::Completed, &store.0, 1, 1)], 2);
        assert!(written.contains("\"version\""), "every file says which shape it is");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&written).unwrap()["version"],
            serde_json::json!(STATE_VERSION)
        );

        // A file from a build that comes after this one is refused rather than half-read: its
        // records may mean something else, and acting on a misread record touches a real file.
        let ahead = written.replace("\"version\": 1", "\"version\": 99");
        assert!(decode(&ahead).unwrap_err().contains("version 99"));
        std::fs::write(store.state_file(), ahead).unwrap();
        let engine = store.engine();
        assert!(engine.snapshot().is_empty());
        assert!(engine.notice().contains("version 99"));
    }

    #[test]
    fn a_row_it_cannot_read_does_not_cost_the_rows_it_can() {
        let store = Store::new();
        store.part("good.iso", 10);
        let written = encode(
            &[
                saved(1, "odd.iso", Status::Completed, &store.0, 10, 10),
                saved(2, "good.iso", Status::Completed, &store.0, 10, 10),
            ],
            3,
        );
        // A status word this build does not know: the row is dropped rather than guessed at, and
        // the nineteen good rows beside it are not lost with it.
        let damaged = written.replacen("\"status\": \"completed\"", "\"status\": \"elsewhere\"", 1);
        std::fs::write(store.state_file(), damaged).unwrap();

        let items = store.engine().snapshot();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].filename, "good.iso");
    }

    #[test]
    fn saving_replaces_the_file_in_one_step_and_leaves_no_temp_behind() {
        let store = Store::new();
        store.part("held.bin", 64);
        std::fs::write(
            store.state_file(),
            encode(&[saved(1, "held.bin", Status::Paused, &store.0, 64, 512)], 2),
        )
        .unwrap();

        let engine = store.engine();
        engine.shutdown();
        assert_eq!(
            store.entries(),
            vec!["held.bin".to_string(), "state.json".to_string()],
            "a half-written list under another name is the next start's corrupt file"
        );
        // And what it wrote is what the next start reads.
        let reopened = store.engine().snapshot();
        assert_eq!(reopened.len(), 1);
        assert_eq!(reopened[0].status, Status::Paused);
        assert_eq!(reopened[0].downloaded, 64);
    }

    #[test]
    fn a_partial_file_whose_record_is_gone_is_left_where_it_is() {
        let store = Store::new();
        // Someone cleared this row from the list; the bytes on disk are still the person's.
        store.part("orphan.iso", 2048);
        std::fs::write(store.state_file(), encode(&[], 1)).unwrap();

        let engine = store.engine();
        engine.shutdown();
        assert!(store.0.join("orphan.iso").exists(), "the app never deletes what it no longer knows about");
        assert_eq!(std::fs::metadata(store.0.join("orphan.iso")).unwrap().len(), 2048);
    }

    #[test]
    fn a_new_download_never_takes_an_id_the_restored_list_is_using() {
        let store = Store::new();
        store.part("a.iso", 1);
        // `next_id` behind the ids in the file — what a crash between the two writes would leave.
        std::fs::write(
            store.state_file(),
            encode(&[saved(12, "a.iso", Status::Completed, &store.0, 1, 1)], 2),
        )
        .unwrap();

        let engine = store.engine();
        // An id collision would point pause, cancel and retry at the wrong row, and cancel deletes
        // a file. The next id is past everything restored, whatever the file claimed.
        assert!(engine.next_id.load(Ordering::Relaxed) > 12);
    }

    #[test]
    fn a_notice_is_raised_once_and_stays_readable_afterwards() {
        let store = Store::new();
        std::fs::write(store.state_file(), "{ not json").unwrap();

        let engine = store.engine();
        assert!(engine.unseen_notice().is_some(), "the window is told");
        engine.acknowledge_notice();
        // Otherwise the redraw timer puts it straight back and nothing the person does clears it.
        assert!(engine.unseen_notice().is_none(), "and not told again every quarter second");
        // `describe` still carries it: a mind may ask about this long after the person clicked
        // past the banner, and the state file is still the one that could not be read.
        assert!(engine.notice().contains("could not be read"));
    }

    #[test]
    fn nothing_is_saved_until_there_is_something_to_say() {
        let store = Store::new();
        // A first run writes no file at all, so a machine that never downloads anything has no
        // stray state to explain — and no notice, because a missing file is not a fault.
        let engine = store.engine();
        assert!(engine.notice().is_empty());
        assert!(store.entries().is_empty());
    }
}
