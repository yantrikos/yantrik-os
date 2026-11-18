use super::*;
use std::sync::atomic::AtomicUsize;

static NEXT_STORE: AtomicUsize = AtomicUsize::new(0);

/// A state directory of this test's own.
///
/// Every test that builds an [`Engine`] goes through one: an engine made with `new()` would
/// read and then write the person's real download list, and a test that can damage the
/// machine it runs on is not a test anybody will keep running.
pub(super) struct Store(pub(super) PathBuf);

impl Store {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yantrik-dl-state-{}-{}",
            std::process::id(),
            NEXT_STORE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn state_file(&self) -> PathBuf {
        self.0.join("state.json")
    }

    pub(super) fn engine(&self) -> Engine {
        Engine::open(self.0.clone())
    }

    /// Put a file of `bytes` zeroes where a download of that name would have written it.
    pub(super) fn part(&self, name: &str, bytes: usize) {
        std::fs::write(self.0.join(name), vec![0u8; bytes]).unwrap();
    }

    /// Names in the directory, sorted, so an assertion can be about the whole directory.
    pub(super) fn entries(&self) -> Vec<String> {
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
pub(super) fn saved(id: i32, filename: &str, status: Status, dir: &Path, downloaded: u64, total: u64) -> Download {
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
        agent: false,
        file_id: String::new(),
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
        agent: false,
        file_id: String::new(),
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
        agent: false,
        file_id: String::new(),
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
