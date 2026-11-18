use super::*;

impl Engine {
    /// `owned` is the partial file a resume has already proved to be its own, still open.
    pub(super) fn spawn(&self, id: i32, resume_from: u64, owned: Option<File>) {
        let engine = self.clone();
        let started = std::thread::Builder::new()
            .name(format!("download-{id}"))
            .spawn(move || engine.transfer(id, resume_from, owned));
        if let Err(e) = started {
            self.fail(id, format!("cannot start download thread: {e}"));
        }
    }

    pub(super) fn fail(&self, id: i32, reason: impl Into<String>) {
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
            self.announce(Notification::new("Downloads", format!("{name} failed")).body(reason));
        }
    }

    /// Record what the app has now written to `file`: the stamp a later resume or cancel must match.
    fn record_stamp(&self, id: i32, file: &File) {
        if let Some(stamp) = stamp_fd(file) {
            self.edit(id, |d| d.file_id = stamp);
        }
    }

    /// One transfer, start to finish, on its own thread.
    pub(super) fn transfer(&self, id: i32, resume_from: u64, owned: Option<File>) {
        let Some(download) = self.get(id) else { return };
        let Some(ctl) = self.control_for(id) else { return };
        let mut path = download.path();
        #[cfg(test)]
        seam::run("after_open", &path);

        let response = match fetch::open(&download.url, resume_from, download.agent) {
            Ok(response) => response,
            Err(reason) => {
                self.fail(id, reason);
                return;
            }
        };

        // A 206 for another part than asked for would put the wrong bytes in the app's own file.
        if resume_from > 0 && response.status() == 206 {
            let right = response
                .header("content-range")
                .is_none_or(|r| r.trim().starts_with(format!("bytes {resume_from}-").as_str()));
            if !right {
                self.fail(id, "the server sent a different part of the file than was asked for");
                return;
            }
        }
        // A ranged request answered 200 means the body is the whole file, not the remainder.
        let appending = owned.is_some() && (resume_from == 0 || response.status() == 206);
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

        let mut file = match (owned, appending) {
            // The descriptor the resume proved: the only way bytes reach the partial file.
            (Some(file), true) => file,
            (old, _) => {
                // A new file, never through a link: one put where the download lands would send
                // the bytes to wherever it points, past the rule that chose this place.
                if let Some(old) = old {
                    // The server ignored the range. The old partial is the app's own and now
                    // useless; take it away if it is still provably that, and start into a new
                    // file whose name collides with nothing.
                    drop(old);
                    self.remove_owned(&path, &download.file_id);
                    let name = unique_in(&download.save_dir, &download.filename);
                    path = download.save_dir.join(&name);
                    self.edit(id, |d| d.filename = name.clone());
                }
                match no_follow(OpenOptions::new().write(true).create_new(true)).open(&path) {
                    Ok(file) => file,
                    Err(e) => {
                        self.fail(id, format!("cannot write {}: {e}", path.display()));
                        return;
                    }
                }
            }
        };

        // Remember which file this is, so a later cancel or resume can tell it from one the
        // person put there.
        let stamp = stamp_fd(&file);
        self.edit(id, |d| {
            d.file_id = stamp.clone().unwrap_or_default();
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
                    let stamp = stamp_fd(&file);
                    self.edit(id, |d| {
                        if let Some(stamp) = stamp {
                            d.file_id = stamp;
                        }
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
                    // Proved on the descriptor it wrote through, so the file taken away is that one
                    // and not whatever sits at the name now.
                    let stamp = stamp_fd(&file);
                    drop(file);
                    if let Some(stamp) = stamp {
                        self.remove_owned(&path, &stamp);
                    }
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
                    self.record_stamp(id, &file);
                    self.fail(id, format!("transfer broke: {e}"));
                    return;
                }
            };
            if let Err(e) = file.write_all(&buffer[..read]) {
                self.record_stamp(id, &file);
                self.fail(id, format!("cannot write {}: {e}", path.display()));
                return;
            }
            // The stamp follows every write, so whatever is saved at a quit matches the file.
            self.record_stamp(id, &file);
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
                let stamp = stamp_fd(&file);
                self.edit(id, |d| {
                    if let Some(stamp) = stamp {
                        d.file_id = stamp;
                    }
                    d.downloaded = written;
                    d.speed_bps = speed;
                    d.eta_secs = eta;
                });
                self.save_progress();
                window_start = Instant::now();
                window_bytes = 0;
            }
        }

        self.record_stamp(id, &file);
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
            // behalf, starting this app again if it is not running.
            self.announce(
                Notification::new("Downloads", title)
                    .body(body)
                    .action_with("open_folder", "Open folder", serde_json::json!({ "id": id })),
            );
        }
    }

    /// Hash the file on disk, record it, and compare it with the expected value if there is one.
    ///
    /// The hash is recorded whether or not anything was expected: an agent that fetched a file
    /// usually needs its digest next, and computing it here means it never has to shell out.
    pub(super) fn hash_and_compare(&self, id: i32) {
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
