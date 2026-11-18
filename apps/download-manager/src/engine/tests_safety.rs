//! The review of #958 at the engine: concurrent saves (M3), private files (M4), a finished
//! download's file under cancel and retry (M1), and the agent mark kept across a restart.

use super::tests::{saved, Store};
use super::*;

fn put(engine: &Engine, download: Download) {
    let mut state = engine.state.lock().unwrap();
    state.controls.insert(download.id, Arc::new(AtomicU8::new(control::RUN)));
    state.items.push(download);
}

#[test]
fn many_concurrent_saves_and_a_close_leave_the_list_valid_and_whole() {
    let store = Store::new();
    let engine = store.engine();
    for id in 1..=8 {
        put(&engine, saved(id, &format!("f{id}.bin"), Status::Paused, &store.0, 10, 100));
    }
    // A reader parses the list and watches the notice for the whole run: a later good save
    // overwrites a torn file and clears "cannot save", so looking only at the end misses both.
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let (engine, done, file) = (engine.clone(), done.clone(), store.state_file());
        std::thread::spawn(move || {
            let mut bad = Vec::new();
            while !done.load(Ordering::Relaxed) {
                if let Ok(raw) = std::fs::read_to_string(&file) {
                    if decode(&raw).is_err() {
                        bad.push("unparseable state.json".to_string());
                    }
                }
                if !engine.notice().is_empty() {
                    bad.push(engine.notice());
                }
            }
            bad
        })
    };
    let writers: Vec<_> = (0..8)
        .map(|_| {
            let engine = engine.clone();
            std::thread::spawn(move || (0..150).for_each(|_| engine.save()))
        })
        .collect();
    engine.settle_for_exit(Duration::from_millis(50));
    writers.into_iter().for_each(|w| w.join().unwrap());
    done.store(true, Ordering::Relaxed);
    let bad = reader.join().unwrap();
    assert!(bad.is_empty(), "a reader saw a torn list or a failed save: {bad:?}");
    let raw = std::fs::read_to_string(store.state_file()).unwrap();
    let (items, _) = decode(&raw).expect("state.json parses");
    assert_eq!(items.len(), 8, "every row is in it");
    assert_eq!(engine.notice(), "", "no save failed");
    assert_eq!(store.entries(), vec!["state.json".to_string()], "no temporary is left behind");
}

#[cfg(unix)]
mod modes {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn the_list_and_its_folder_are_the_persons_alone() {
        let store = Store::new();
        let nested = store.0.join("a/downloads");
        let engine = Engine::open(nested.clone());
        put(&engine, saved(1, "x.bin", Status::Paused, &store.0, 1, 2));
        engine.save();
        assert_eq!(mode(&nested), 0o700);
        assert_eq!(mode(&nested.join("state.json")), 0o600);
    }

    #[test]
    fn a_folder_that_is_already_open_is_tightened_at_start() {
        let store = Store::new();
        std::fs::set_permissions(&store.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(store.state_file(), "{\"version\":1,\"next_id\":1,\"downloads\":[]}").unwrap();
        std::fs::set_permissions(store.state_file(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let engine = store.engine();
        assert_eq!(mode(&store.0), 0o700);
        engine.save();
        assert_eq!(mode(&store.state_file()), 0o600, "and the next save is private too");
    }
}

#[test]
fn cancel_takes_a_finished_rows_entry_and_never_its_file() {
    let store = Store::new();
    let engine = store.engine();
    store.part("done.bin", 100);
    put(&engine, saved(1, "done.bin", Status::Completed, &store.0, 100, 100));
    engine.cancel(1).unwrap();
    assert!(engine.get(1).is_none());
    assert!(store.0.join("done.bin").exists());
    // And it stays gone after a restart.
    assert!(store.engine().get(1).is_none());
}

#[test]
fn retry_of_a_missing_row_leaves_what_now_sits_at_its_path() {
    let store = Store::new();
    let engine = store.engine();
    let mut gone = saved(1, "moved.bin", Status::Missing, &store.0, 100, 100);
    gone.url = "http://127.0.0.1:1/moved.bin".into();
    put(&engine, gone);
    store.part("moved.bin", 5);
    engine.retry(1).unwrap();
    for _ in 0..500 {
        if engine.get(1).is_some_and(|d| d.status == Status::Failed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(std::fs::metadata(store.0.join("moved.bin")).unwrap().len(), 5, "the other file is untouched");
    assert_eq!(engine.get(1).unwrap().filename, "moved (1).bin");
}

#[test]
fn the_agent_mark_survives_a_restart() {
    let mut d = saved(1, "a.bin", Status::Paused, Path::new("/tmp"), 1, 2);
    d.agent = true;
    let (back, _) = decode(&encode(&[d], 2)).unwrap();
    assert!(back[0].agent);
}

#[test]
fn cancel_and_retry_of_a_failed_row_that_wrote_nothing_keep_the_persons_files() {
    // The row failed before its file was created. Since then the person saved their own files at
    // the download's path (and at a `.part` name beside it).
    let store = Store::new();
    let engine = store.engine();
    let mut row = saved(1, "report.pdf", Status::Failed, &store.0, 0, 100);
    row.error = "HTTP 404".into();
    put(&engine, row.clone());
    put(&engine, { let mut r = row.clone(); r.id = 2; r });
    std::fs::write(store.0.join("report.pdf"), "the person's own report").unwrap();
    std::fs::write(store.0.join("report.pdf.part"), "the person's own part").unwrap();

    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read_to_string(store.0.join("report.pdf")).unwrap(), "the person's own report");
    assert_eq!(std::fs::read_to_string(store.0.join("report.pdf.part")).unwrap(), "the person's own part");

    let _ = engine.retry(2);
    assert_eq!(std::fs::read_to_string(store.0.join("report.pdf")).unwrap(), "the person's own report");
    assert_eq!(std::fs::read_to_string(store.0.join("report.pdf.part")).unwrap(), "the person's own part");
    assert_ne!(engine.get(2).unwrap().filename, "report.pdf", "the retry takes a free name");
}

#[test]
fn cancel_removes_the_partial_file_it_created_and_nothing_swapped_in_after() {
    let store = Store::new();
    let engine = store.engine();
    // Its own partial file: recorded when it was created, still the same plain file.
    std::fs::write(store.0.join("own.bin"), "partial").unwrap();
    let mut own = saved(1, "own.bin", Status::Paused, &store.0, 7, 100);
    own.file_id = stamp_of(&store.0.join("own.bin"));
    put(&engine, own);
    // The same, but the person replaced the file with another one of the same name.
    std::fs::write(store.0.join("swap.bin"), "partial").unwrap();
    let mut swapped = saved(2, "swap.bin", Status::Paused, &store.0, 7, 100);
    swapped.file_id = stamp_of(&store.0.join("swap.bin"));
    put(&engine, swapped);
    // Made before the old one goes, so the two cannot share an inode number.
    std::fs::write(store.0.join("theirs.tmp"), "theirs").unwrap();
    std::fs::rename(store.0.join("theirs.tmp"), store.0.join("swap.bin")).unwrap();

    engine.cancel(1).unwrap();
    engine.cancel(2).unwrap();
    assert!(!store.0.join("own.bin").exists(), "its own partial file goes");
    assert_eq!(std::fs::read_to_string(store.0.join("swap.bin")).unwrap(), "theirs");
}

#[test]
fn resume_never_appends_to_a_file_that_is_not_the_downloads_own() {
    let store = Store::new();
    let engine = store.engine();
    // Its own partial file, then the person replaced it with theirs (made first, so the inode differs).
    std::fs::write(store.0.join("big.iso"), "partial").unwrap();
    let mut row = saved(1, "big.iso", Status::Paused, &store.0, 7, 100);
    row.file_id = stamp_of(&store.0.join("big.iso"));
    put(&engine, row);
    std::fs::write(store.0.join("theirs.tmp"), "the person's own file").unwrap();
    std::fs::rename(store.0.join("theirs.tmp"), store.0.join("big.iso")).unwrap();
    // A failed row that never wrote anything, with the person's file at its path.
    put(&engine, saved(2, "doc.pdf", Status::Failed, &store.0, 0, 100));
    std::fs::write(store.0.join("doc.pdf"), "also theirs").unwrap();

    assert!(engine.resume(1).is_err());
    assert!(engine.resume(2).is_err());
    assert_eq!(std::fs::read_to_string(store.0.join("big.iso")).unwrap(), "the person's own file");
    assert_eq!(std::fs::read_to_string(store.0.join("doc.pdf")).unwrap(), "also theirs");
    assert_eq!(engine.get(1).unwrap().status, Status::Paused, "the row is left as it was");
}

// The review of 252369b5: the open that appends is the check, a stronger identity than dev:ino,
// a clean restart when the server ignores the range, and rows from before files were tracked.

fn stamp_of(path: &Path) -> String {
    stamp_fd(&File::open(path).unwrap()).expect("a plain file has a stamp")
}

/// A paused row owning `name`, which holds `bytes`.
fn own_partial(engine: &Engine, store: &Store, id: i32, name: &str, bytes: &[u8], url: &str) -> PathBuf {
    let path = store.0.join(name);
    std::fs::write(&path, bytes).unwrap();
    let mut row = saved(id, name, Status::Paused, &store.0, bytes.len() as u64, 20);
    row.file_id = stamp_of(&path);
    row.url = url.into();
    put(engine, row);
    path
}

fn wait_for(engine: &Engine, id: i32, done: impl Fn(&Download) -> bool) -> Download {
    for _ in 0..600 {
        if let Some(d) = engine.get(id).filter(|d| done(d)) {
            return d;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("download {id} never got there: {:?}", engine.get(id).map(|d| (d.status, d.error)));
}

fn replace_with_theirs(path: &Path, text: &'static str) -> impl FnOnce() + Send + 'static {
    let path = path.to_path_buf();
    move || {
        let temp = path.with_extension("theirs.tmp");
        std::fs::write(&temp, text).unwrap();
        std::fs::rename(&temp, &path).unwrap();
    }
}

fn finished(d: &Download) -> bool {
    d.status == Status::Completed || d.status == Status::Failed
}

const BODY: &[u8] = b"0123456789abcdefghij";

#[test]
fn a_file_swapped_in_after_resume_is_never_appended_to() {
    use crate::tests::server::{File as Served, Server};
    let server = Server::start(vec![("/big.iso", Served::plain(BODY.to_vec()))]);
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", &BODY[..7], &server.url("/big.iso"));
    // The person's file lands at the path after the proof and before the request returns.
    seam::on("after_open", &path, replace_with_theirs(&path, "the person's own file"));

    engine.resume(1).unwrap();
    wait_for(&engine, 1, finished);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "the person's own file");
}

#[test]
fn a_link_put_at_the_path_is_not_followed_by_resume() {
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    std::fs::write(store.0.join("theirs.txt"), "theirs").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(store.0.join("theirs.txt"), &path).unwrap();
    assert!(engine.resume(1).is_err());
    assert_eq!(std::fs::read_to_string(store.0.join("theirs.txt")).unwrap(), "theirs");
}

#[test]
fn a_recreated_file_is_not_the_partial_even_when_it_takes_the_same_inode() {
    use std::os::unix::fs::MetadataExt;
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    let first = std::fs::metadata(&path).unwrap().ino();
    // Delete and recreate until the filesystem hands the freed inode to the new file, which ext4
    // and tmpfs commonly do at once. Same name, same size, same inode: only the rest differs.
    let mut reused = false;
    for _ in 0..200 {
        std::fs::remove_file(&path).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        std::fs::write(&path, b"theirs!").unwrap();
        if std::fs::metadata(&path).unwrap().ino() == first {
            reused = true;
            break;
        }
    }
    eprintln!("inode reused: {reused}");
    assert!(engine.resume(1).is_err(), "a recreated file is not the partial");
    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"theirs!", "and cancel leaves it");
}

#[test]
fn a_file_swapped_in_between_the_last_look_and_the_unlink_is_kept() {
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    seam::on("before_unlink", &path, replace_with_theirs(&path, "the person's own file"));
    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "the person's own file");
}

/// A server that answers every request with the whole body and a 200, as one without ranges does.
fn ignores_ranges() -> u16 {
    use std::io::Read as _;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
                head.push(byte[0]);
            }
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", BODY.len());
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(BODY);
        }
    });
    port
}

#[test]
fn a_server_that_ignores_the_range_restarts_into_a_clean_file() {
    let port = ignores_ranges();
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", &BODY[..7], &format!("http://127.0.0.1:{port}/big.iso"));
    engine.resume(1).unwrap();
    let done = wait_for(&engine, 1, finished);
    assert_eq!(done.status, Status::Completed, "{}", done.error);
    assert_eq!(std::fs::read(&path).unwrap(), BODY, "the whole file, not the remainder appended");
    assert_eq!(done.file_id, stamp_of(&path), "and the new file's identity is recorded");
}

#[test]
fn a_restart_after_an_ignored_range_takes_a_free_name_when_the_old_file_is_not_the_apps() {
    let port = ignores_ranges();
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", &BODY[..7], &format!("http://127.0.0.1:{port}/big.iso"));
    seam::on("after_open", &path, replace_with_theirs(&path, "the person's own file"));
    engine.resume(1).unwrap();
    let done = wait_for(&engine, 1, finished);
    assert_eq!(done.status, Status::Completed, "{}", done.error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "the person's own file");
    assert_eq!(done.filename, "big (1).iso");
    assert_eq!(std::fs::read(store.0.join("big (1).iso")).unwrap(), BODY);
}

#[test]
fn a_row_saved_before_files_were_tracked_keeps_its_file_and_says_so() {
    let store = Store::new();
    let engine = store.engine();
    std::fs::write(store.0.join("old.iso"), "partial").unwrap();
    std::fs::write(store.0.join("older.iso"), "partial").unwrap();
    // Empty, or the `dev:ino` an earlier build wrote: nothing this build can check.
    for (id, name, file_id) in [(1, "old.iso", ""), (2, "older.iso", "12:34")] {
        let mut legacy = saved(id, name, Status::Paused, &store.0, 7, 20);
        legacy.file_id = file_id.into();
        put(&engine, legacy);
    }
    let why = engine.resume(1).unwrap_err();
    assert!(why.contains("before files were tracked"), "{why}");
    assert!(engine.resume(2).unwrap_err().contains("before files were tracked"));
    engine.cancel(1).unwrap();
    assert!(store.0.join("old.iso").exists(), "the file stays");
    assert!(engine.notice().contains("left on disk"), "{}", engine.notice());
    engine.cancel(2).unwrap();
    assert!(store.0.join("older.iso").exists());
}

/// The review's Probe A: while the app removes its file, the person's atomic save lands on the
/// name again and again. Whatever happens, their file must still exist somewhere afterwards.
#[cfg(target_os = "linux")]
#[test]
fn an_atomic_save_racing_the_removal_never_costs_the_person_their_file() {
    use std::os::unix::ffi::OsStrExt;
    const ROUNDS: usize = 300;
    let store = Store::new();
    let mut lost = 0;
    for round in 0..ROUNDS {
        let app = store.0.join(format!("app{round}.bin"));
        let theirs = store.0.join(format!("theirs{round}.bin"));
        std::fs::write(&app, b"partial").unwrap();
        std::fs::write(&theirs, format!("the person's file {round}")).unwrap();
        let stamp = stamp_of(&app);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let swapper = {
            let (stop, a, b) = (stop.clone(), app.clone(), theirs.clone());
            std::thread::spawn(move || {
                let (a, b) = (std::ffi::CString::new(a.as_os_str().as_bytes()).unwrap(), std::ffi::CString::new(b.as_os_str().as_bytes()).unwrap());
                while !stop.load(Ordering::Relaxed) {
                    // RENAME_EXCHANGE: the two names trade files in one atomic step.
                    unsafe { libc::syscall(libc::SYS_renameat2, libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), 2u32) };
                }
            })
        };
        remove_if_ours(&app, &stamp);
        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();
        let wanted = format!("the person's file {round}");
        let mut found = false;
        let mut folders = vec![store.0.clone()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap().flatten() {
                match entry.file_type() {
                    Ok(t) if t.is_dir() => folders.push(entry.path()),
                    _ => found |= std::fs::read_to_string(entry.path()).is_ok_and(|text| text == wanted),
                }
            }
        }
        lost += usize::from(!found);
    }
    assert_eq!(lost, 0, "the person's file was deleted in {lost} of {ROUNDS} rounds");
}

#[test]
fn a_stamp_without_a_birth_time_proves_nothing() {
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    // The same stamp as the file's own, with the birth time taken out, as a filesystem without
    // one would write it.
    let real = stamp_of(&path);
    let parts: Vec<&str> = real.split(':').collect();
    let bare = format!("{}:{}:{}:-:{}:{}", parts[0], parts[1], parts[2], parts[4], parts[5]);
    assert!(!has_stamp(&bare));
    engine.edit(1, |d| d.file_id = bare.clone());
    assert!(engine.resume(1).is_err(), "no resume without a birth time");
    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"partial", "no delete without a birth time");
    assert_eq!(remove_if_ours(&path, &bare), Removal::Kept);
}

#[test]
fn a_file_with_the_same_inode_and_size_but_another_birth_time_is_not_the_partial() {
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    // The recorded stamp, with a birth time and modification time from another life of the inode.
    let real = stamp_of(&path);
    let mut parts: Vec<String> = real.split(':').map(String::from).collect();
    parts[3] = "1.0".into();
    parts[4] = "1.0".into();
    engine.edit(1, |d| d.file_id = parts.join(":"));
    assert!(engine.resume(1).is_err());
    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"partial");
}

#[test]
fn a_crash_mid_write_resumes_from_the_files_actual_size() {
    use crate::tests::server::{File as Served, Server};
    let server = Server::start(vec![("/big.iso", Served::plain(BODY.to_vec()))]);
    let store = Store::new();
    let engine = store.engine();
    // The stamp was taken at 7 bytes; then the app wrote 6 more and was killed before recording.
    let path = own_partial(&engine, &store, 1, "big.iso", &BODY[..7], &server.url("/big.iso"));
    let recorded = recorded_size(&engine.get(1).unwrap().file_id).unwrap();
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(&BODY[7..13]).unwrap();
    assert_ne!(stamp_of(&path), engine.get(1).unwrap().file_id, "size and mtime moved");
    assert!(std::fs::metadata(&path).unwrap().len() > recorded);
    engine.resume(1).unwrap();
    let done = wait_for(&engine, 1, finished);
    assert_eq!(done.status, Status::Completed, "{}", done.error);
    assert_eq!(done.filename, "big.iso", "the same file continued");
    assert_eq!(std::fs::read(&path).unwrap(), BODY, "no byte lost or repeated");
}

#[test]
fn a_file_cut_shorter_than_recorded_restarts_under_a_new_name() {
    use crate::tests::server::{File as Served, Server};
    let server = Server::start(vec![("/big.iso", Served::plain(BODY.to_vec()))]);
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", &BODY[..7], &server.url("/big.iso"));
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_len(3).unwrap();
    engine.resume(1).unwrap();
    let done = wait_for(&engine, 1, finished);
    assert_eq!(done.status, Status::Completed, "{}", done.error);
    assert_ne!(done.filename, "big.iso");
    assert_eq!(std::fs::read(store.0.join(&done.filename)).unwrap(), BODY);
    assert_eq!(std::fs::read(&path).unwrap(), &BODY[..3], "the cut file is left alone");
}

#[test]
fn a_recreated_file_with_the_same_size_is_refused_by_birth_time() {
    let store = Store::new();
    let engine = store.engine();
    let path = own_partial(&engine, &store, 1, "big.iso", b"partial", "http://127.0.0.1:1/x");
    let real = stamp_of(&path);
    // Same device, inode, size and mtime; only the birth time is another file's.
    let mut parts: Vec<String> = real.split(':').map(String::from).collect();
    parts[3] = "1.0".into();
    engine.edit(1, |d| d.file_id = parts.join(":"));
    assert!(engine.resume(1).is_err());
    engine.cancel(1).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"partial");
}
