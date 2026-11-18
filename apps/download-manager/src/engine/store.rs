use super::*;

impl Engine {

    /// Read the saved list and reconcile it with the filesystem.
    ///
    /// Called once, from [`Engine::open`]. Every way this can go wrong ends with a working app
    /// holding an empty list and a notice saying what happened, because a downloads window that
    /// refuses to start is worse than one that has forgotten.
    pub(super) fn load(&self) {
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
    pub(super) fn keep_aside(&self, reason: &str) {
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
    pub(super) fn save(&self) {
        if let Ok(mut last) = self.last_save.lock() {
            *last = Some(Instant::now());
        }
        // One writer at a time, with the snapshot taken inside the lock: workers all save together
        // when a close pauses them, and a list taken early but written late would put an old one
        // over a newer.
        let _one_writer = self.save_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let items = self.snapshot();
        let next_id = self.next_id.load(Ordering::Relaxed);
        let written = self
            .store_path
            .parent()
            .map_or(Ok(()), private_folder)
            .and_then(|()| {
                yantrik_foundation::json_store::write_atomically(&self.store_path, encode(&items, next_id).as_bytes())
            });
        if let Err(e) = written {
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
    pub(super) fn save_progress(&self) {
        let due = match self.last_save.lock() {
            Ok(last) => last.map(|t| t.elapsed() >= PROGRESS_SAVE_INTERVAL).unwrap_or(true),
            Err(_) => false,
        };
        if due {
            self.save();
        }
    }

    /// Stop every writer and put the list down, without waiting for the workers: the tests' way of
    /// ending an engine. The app itself ends through [`Engine::settle_for_exit`].
    ///
    /// A worker blocked on a socket read can take the whole connect timeout to reach its next
    /// chunk boundary. The record then goes to disk still saying `downloading`, and the next start
    /// reconciles it against the partial file, which is exact.
    #[cfg(test)]
    pub fn shutdown(&self) {
        self.pause_all();
        self.save();
    }

    /// The app is ending: stop every writer, give each up to `backstop` to reach its next chunk
    /// boundary (where it flushes the file and records itself as paused), then write the list.
    /// Whether every writer had stopped.
    ///
    /// [`Engine::shutdown`] is this without the wait. A writer still blocked on a socket when the
    /// backstop passes leaves a file that ends at a whole chunk and a record the next start
    /// reconciles against it, so nothing is lost either way; the wait only lets the record say
    /// `paused` where it would have said `downloading`.
    pub fn settle_for_exit(&self, backstop: Duration) -> bool {
        self.pause_all();
        let until = Instant::now() + backstop;
        while self.writing() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.save();
        !self.writing()
    }

    /// Counts and combined speed, for the header line.
    pub fn totals(&self) -> Totals {
        Totals::of(&self.snapshot())
    }
}

impl Totals {
    /// The numbers for a list as it stands.
    pub fn of(items: &[Download]) -> Totals {
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
}

// ── The state file ──────────────────────────────────────────────────

/// The whole file. `version` comes first so a reader that does not understand this shape can find
/// that out without having to parse the rest.
#[derive(Serialize, Deserialize)]
pub(super) struct StoredState {
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
    #[serde(default)]
    agent: bool,
    #[serde(default)]
    file_id: String,
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
pub(super) fn encode(items: &[Download], next_id: i32) -> String {
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
                agent: d.agent,
                file_id: d.file_id.clone(),
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
pub(super) fn decode(raw: &str) -> Result<(Vec<Download>, i32), String> {
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
            agent: record.agent,
            file_id: record.file_id,
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

/// Make `dir` if it is missing and keep it the person's alone: 0700, as the list in it is 0600
/// (the atomic writer makes it so). A folder already there with looser rights is tightened, unless
/// it is a link: one the person pointed elsewhere is theirs to set.
pub(super) fn private_folder(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = std::fs::symlink_metadata(dir)?;
        if meta.is_dir() && meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)
}

/// What a restored list says about the rows that came back different from how they were left.
pub(super) fn restored_notice(interrupted: usize, missing: usize) -> String {
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
