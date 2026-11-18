use super::*;

impl Engine {
    /// Queue a URL and start fetching it. Returns the new id.
    ///
    /// Rejects anything that is not `http`/`https` rather than handing it to the transport and
    /// reporting whatever that says: a caller that mistyped a path deserves to be told it was a
    /// path, and a `file://` "download" would be a copy with none of this machinery's meaning.
    pub fn add(&self, url: &str, checksum: &str, save_dir: Option<&str>) -> Result<i32, String> {
        self.add_as(url, checksum, save_dir, false)
    }

    /// [`Engine::add`], for `agent` when an agent asked: its fetch is held to the public internet.
    pub fn add_as(&self, url: &str, checksum: &str, save_dir: Option<&str>, agent: bool) -> Result<i32, String> {
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
            agent,
            file_id: String::new(),
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
        self.spawn(id, 0, None);
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
        // Append only to the file this download created and last wrote. The proof is made on the
        // descriptor the transfer then writes through, here and before any request goes out, so
        // nothing done to the path afterwards can redirect the bytes. Anything else at the path is
        // the person's: refuse and leave it untouched.
        let (have, owned) = match open_owned(&download) {
            Ok(Some(file)) => {
                // After a crash the file holds more than the last stamp recorded; those bytes are
                // the app's own. The size is read on the verified descriptor, never from the path.
                let actual = stamp_fd(&file).as_deref().and_then(recorded_size).unwrap_or(0);
                let recorded = recorded_size(&download.file_id).unwrap_or(0);
                if actual >= recorded {
                    (actual, Some(file))
                } else {
                    // Truncated since the app last wrote: start again under a new name, leaving
                    // the file where it is.
                    drop(file);
                    let name = unique_in(&download.save_dir, &download.filename);
                    self.edit(id, |d| {
                        d.filename = name;
                        d.file_id.clear();
                    });
                    (0, None)
                }
            }
            Ok(None) => (0, None),
            Err(why) => {
                let legacy = if !has_stamp(&download.file_id) {
                    "; a download saved before files were tracked, or on a filesystem with no creation time, has no record to check against"
                } else {
                    ""
                };
                return Err(format!(
                    "download {id}: {why}{legacy}; retry starts it again under a new name and leaves that file alone"
                ));
            }
        };
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
        self.spawn(id, have, owned);
        Ok(())
    }

    /// Stop and discard the partial file.
    ///
    /// The record stays in the list as `failed` with the reason, because a download that vanished
    /// from the window tells the person nothing about why their file is not there.
    ///
    /// A finished download (`completed`, or `missing`) has no partial file: what is at its path is
    /// the person's file, or something else that now sits there. Cancelling it only takes the row
    /// out of the list and never touches the file.
    pub fn cancel(&self, id: i32) -> Result<(), String> {
        let download = self.get(id).ok_or_else(|| format!("no download {id}"))?;
        if download.status.is_finished() {
            if let Ok(mut state) = self.state.lock() {
                state.items.retain(|d| d.id != id);
                state.controls.remove(&id);
            }
            self.touch();
            self.save();
            return Ok(());
        }
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::CANCEL, Ordering::Relaxed);
        }
        // A running worker removes the file itself when it sees the flag; a paused or failed one
        // has no worker left to do it, so do it here, but only if the file is provably the one
        // this download created. A failed row may never have written anything, and what sits at
        // its path now is then the person's.
        if !download.status.is_active() {
            let gone = self.remove_owned(&download.path(), &download.file_id);
            // A row saved before files were tracked has nothing to check a file against, so its
            // file stays. Said out loud, because the person asked for it to go.
            if !gone && !has_stamp(&download.file_id) && download.downloaded > 0 && download.path().symlink_metadata().is_ok() {
                self.set_notice(format!(
                    "{} was left on disk: it cannot be shown to be this download's own (it was saved before files were tracked, or the filesystem records no creation time)",
                    download.path().display()
                ));
            }
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
        // A file this download did not create (a `missing` one, or one that appeared since) is
        // left alone and the new fetch takes a name that is free.
        let renamed = if download.status != Status::Missing && self.remove_owned(&download.path(), &download.file_id) {
            None
        } else if std::fs::symlink_metadata(download.path()).is_ok() {
            if download.status != Status::Missing && download.downloaded > 0 && self.notice().is_empty() {
                self.set_notice(format!("{} was left where it is; the new download takes another name", download.path().display()));
            }
            Some(unique_in(&download.save_dir, &download.filename))
        } else {
            None
        };
        if let Some(ctl) = self.control_for(id) {
            ctl.store(control::RUN, Ordering::Relaxed);
        }
        self.edit(id, |d| {
            if let Some(name) = renamed {
                d.filename = name;
            }
            d.status = Status::Queued;
            d.downloaded = 0;
            d.error.clear();
            d.file_hash.clear();
            d.file_id.clear();
            d.checksum_status = "none".into();
            d.ended_at.clear();
            d.started_at = now_stamp();
            d.interrupted = false;
        });
        self.save();
        self.spawn(id, 0, None);
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
}
