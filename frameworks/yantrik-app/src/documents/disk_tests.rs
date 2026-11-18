//! The disk seam and the watched files, each rule as a test (design/notes-gate-2026-10-21.md §3).

use std::path::PathBuf;

use super::disk::{Disk, FaultyDisk, RealDisk, Step};
use super::*;

fn fp(text: &str) -> Fingerprint {
    Fingerprint::of_bytes(text.as_bytes())
}

/// One poll's sightings, without the arming each was taken under.
fn polled(watched: &Watched) -> Vec<(String, Observed)> {
    watched.poll().into_iter().map(|(key, seen, _)| (key, seen)).collect()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yantrik-documents-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_failure_before_the_rename_leaves_the_file_as_it_was() {
    let dir = scratch("before-rename");
    let path = dir.join("note.md");
    RealDisk.write_atomically(&path, b"old").unwrap();
    let disk = FaultyDisk::new();
    for step in [Step::CreateTemp, Step::Write, Step::SyncFile, Step::Rename] {
        disk.fail(step, "note.md", Some(1));
        let err = disk.write_atomically(&path, b"new").unwrap_err();
        assert_eq!(err.step, step);
        assert_eq!(step.durability(), Durability::NotWritten);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(leftovers.len(), 1, "no temp file is left behind");
    disk.write_atomically(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new", "a fault used up, the disk behaves");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failure_after_the_rename_has_the_bytes_but_not_their_durability() {
    let dir = scratch("after-rename");
    let path = dir.join("note.md");
    let disk = FaultyDisk::new();
    disk.fail(Step::SyncDir, ".md", None);
    let err = disk.write_atomically(&path, b"new").unwrap_err();
    assert_eq!(err.step.durability(), Durability::Unknown);
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    // A fault scoped to one suffix spares the rest: metadata and content fail apart.
    RealDisk.write_atomically(&dir.join("note.meta"), b"m").unwrap();
    assert!(disk.write_atomically(&dir.join("note.meta"), b"m2").is_ok());
    assert_eq!(disk.hits.lock().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- the watched files ----

#[test]
fn the_watch_reports_a_change_from_the_baseline_once_and_a_deletion() {
    let dir = scratch("watched");
    let path = dir.join("note.md");
    std::fs::write(&path, "v0").unwrap();
    let watched = Watched::new();
    watched.add("doc-1", path.clone(), Some(Observed::File(fp("v0"))));
    assert!(polled(&watched).is_empty(), "the file it was read from is not a change");
    std::fs::write(&path, "theirs").unwrap();
    assert_eq!(polled(&watched), vec![("doc-1".to_string(), Observed::File(fp("theirs")))]);
    assert!(polled(&watched).is_empty(), "reported once");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(polled(&watched), vec![("doc-1".to_string(), Observed::Gone)]);
    watched.remove("doc-1");
    std::fs::write(&path, "back").unwrap();
    assert!(polled(&watched).is_empty(), "an unwatched file is not reported");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_looked_at_is_not_a_change() {
    let dir = scratch("unreadable");
    // A path under a plain file cannot be looked at (not a directory): skipped, not "gone".
    std::fs::write(dir.join("plain"), "a file, not a directory").unwrap();
    let watched = Watched::new();
    watched.add("doc-1", dir.join("plain").join("note.md"), Some(Observed::File(fp("v0"))));
    assert!(polled(&watched).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_replace_keeps_the_files_mode_and_a_new_file_is_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("mode");
    let path = dir.join("note.md");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    RealDisk.write_atomically(&path, b"new").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "a private note stays private");
    let fresh = dir.join("fresh.md");
    RealDisk.write_atomically(&fresh, b"x").unwrap();
    let mode = std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_link_is_not_replaced() {
    let dir = scratch("link");
    let target = dir.join("target.md");
    std::fs::write(&target, "kept").unwrap();
    let link = dir.join("note.md");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let err = RealDisk.write_atomically(&link, b"new").unwrap_err();
    assert_eq!(err.step, Step::Check);
    assert_eq!(err.step.durability(), Durability::NotWritten);
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_create_never_replaces_what_is_there_and_a_new_file_is_private() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = scratch("create");
    let path = dir.join("note.md");
    RealDisk.create_new(&path, b"first").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let err = RealDisk.create_new(&path, b"second").unwrap_err();
    assert_eq!(err.step, Step::Rename);
    assert_eq!(err.step.durability(), Durability::NotWritten);
    assert_eq!(std::fs::read(&path).unwrap(), b"first", "nothing already there is replaced");
    // Nor a link, even one that points nowhere.
    let link = dir.join("dangling.md");
    std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
    assert!(RealDisk.create_new(&link, b"x").is_err());
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    let names: Vec<String> =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.len(), 2, "no temp file is left behind: {names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_create_fails_at_the_chosen_step() {
    let dir = scratch("create-faults");
    let path = dir.join("note.md");
    let disk = FaultyDisk::new();
    disk.fail(Step::Write, ".md", Some(1));
    assert_eq!(disk.create_new(&path, b"x").unwrap_err().step, Step::Write);
    assert!(!path.exists(), "a failure before the link leaves nothing");
    disk.fail(Step::SyncDir, ".md", Some(1));
    assert_eq!(disk.create_new(&path, b"x").unwrap_err().step.durability(), Durability::Unknown);
    assert_eq!(std::fs::read(&path).unwrap(), b"x", "after the link the bytes are there");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_large_file_unchanged_is_not_reported_and_a_change_to_it_is() {
    let dir = scratch("large");
    let path = dir.join("big.md");
    let big = vec![b'a'; 17 * 1024 * 1024];
    std::fs::write(&path, &big).unwrap();
    let watched = Watched::new();
    watched.add("doc-1", path.clone(), Some(Observed::File(Fingerprint::of_bytes(&big))));
    assert!(polled(&watched).is_empty(), "compared like with like, at any size");
    assert!(polled(&watched).is_empty());
    let mut changed = big.clone();
    changed.push(b'b');
    std::fs::write(&path, &changed).unwrap();
    assert_eq!(polled(&watched), vec![("doc-1".to_string(), Observed::File(Fingerprint::of_bytes(&changed)))]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_look_taken_before_a_re_arm_is_dropped() {
    let dir = scratch("rearm");
    let path = dir.join("note.md");
    std::fs::write(&path, "v1").unwrap();
    let watched = Watched::new();
    watched.add("doc-1", path.clone(), Some(Observed::File(fp("v0"))));
    assert_eq!(polled(&watched), vec![("doc-1".to_string(), Observed::File(fp("v1")))]);
    // Re-armed at what is there: a look from now on reports only a change from it.
    watched.add("doc-1", path.clone(), Some(Observed::File(fp("v1"))));
    assert!(polled(&watched).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_sighting_names_the_arming_it_was_taken_under() {
    let dir = scratch("arming");
    let path = dir.join("note.md");
    std::fs::write(&path, "v1").unwrap();
    let watched = Watched::new();
    watched.add("doc-1", path.clone(), Some(Observed::File(fp("v0"))));
    let (_, _, arming) = watched.poll().pop().expect("a change");
    assert!(watched.is_current("doc-1", arming));
    watched.add("doc-1", path, Some(Observed::File(fp("v1"))));
    assert!(!watched.is_current("doc-1", arming), "re-armed since");
    assert!(!watched.is_current("doc-2", arming));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn a_replace_that_keeps_the_length_and_the_time_is_still_seen() {
    let dir = scratch("same-stamp");
    let path = dir.join("note.md");
    std::fs::write(&path, "aaaa").unwrap();
    let watched = Watched::new();
    watched.add("doc-1", path.clone(), Some(Observed::File(fp("aaaa"))));
    assert!(polled(&watched).is_empty());
    // Another program restores a copy: same length, the old modification time, a new inode.
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let other = dir.join("restore.tmp");
    std::fs::write(&other, "bbbb").unwrap();
    std::fs::File::options().write(true).open(&other).unwrap().set_modified(mtime).unwrap();
    std::fs::rename(&other, &path).unwrap();
    assert_eq!(polled(&watched), vec![("doc-1".to_string(), Observed::File(fp("bbbb")))]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether this directory's filesystem has the atomic exchange (tmpfs and ext4 do; NFS, 9p and
/// FUSE do not). A test that needs it says so and passes on one without, where `Unknown` is the
/// right answer.
fn exchanges_here(dir: &std::path::Path) -> bool {
    let probe = dir.join(".probe");
    std::fs::write(&probe, "a").unwrap();
    let got = RealDisk.exchange(&probe, b"b").unwrap();
    let _ = std::fs::remove_file(&probe);
    !matches!(got.displaced, super::disk::Displaced::Unknown)
}

#[test]
fn an_exchange_hands_back_what_it_replaced() {
    use super::disk::Displaced;
    let dir = scratch("exchange");
    let path = dir.join("note.md");
    let first = RealDisk.exchange(&path, b"first").unwrap();
    assert!(matches!(first.displaced, Displaced::Nothing | Displaced::Unknown), "{first:?}");
    assert!(first.unsure.is_none());
    let second = RealDisk.exchange(&path, b"second").unwrap();
    if exchanges_here(&dir) {
        assert_eq!(first.displaced, Displaced::Nothing, "created, nothing replaced");
        assert_eq!(second.displaced, Displaced::Bytes(b"first".to_vec()));
    } else {
        eprintln!("no atomic exchange on this filesystem: Unknown is the answer");
        assert_eq!(second.displaced, Displaced::Unknown);
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(leftovers.len(), 1, "the temp that held the old bytes is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn a_write_between_the_check_and_the_exchange_is_kept_and_judged() {
    let dir = scratch("exchange-race");
    if !exchanges_here(&dir) {
        eprintln!("no atomic exchange on this filesystem; nothing to show here");
        return;
    }
    let path = dir.join("note.md");
    std::fs::write(&path, "v0").unwrap();
    let mut docs: Documents<String> = Documents::new();
    let id = docs.open(Source::File(path.clone()), "v0".into(), Some(fp("v0")));
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "mine".into());
    let ticket = match d.begin_save() {
        BeginSave::Ticket(t) => t,
        other => panic!("{other:?}"),
    };
    // The store's check passes: the file is the baseline.
    assert!(ticket.may_replace(super::Fingerprint::of(&path).as_ref()));
    // Another program writes now, after the check and before the replace.
    std::fs::write(&path, "theirs").unwrap();
    let done = RealDisk.exchange(&path, b"mine").unwrap();
    assert_eq!(done.displaced, super::disk::Displaced::Bytes(b"theirs".to_vec()), "their bytes are not lost");
    let witnessed = ticket.judge(&done.displaced);
    assert_eq!(witnessed, Witnessed::Replaced(Replaced::OutsideEdit));
    let outcome = d.save_landed_witnessed(ticket.seq, Ok(fp("mine")), witnessed);
    assert!(matches!(outcome, SaveOutcome::Saved { replaced: Some(Replaced::OutsideEdit), .. }));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_exchange_refuses_a_link_and_faults_inject_as_for_a_replace() {
    let dir = scratch("exchange-faults");
    let path = dir.join("note.md");
    std::fs::write(&path, "old").unwrap();
    let disk = FaultyDisk::new();
    disk.fail(Step::Rename, "note.md", Some(1));
    assert_eq!(disk.exchange(&path, b"new").unwrap_err().step, Step::Rename);
    assert_eq!(std::fs::read(&path).unwrap(), b"old", "a failure before the swap changes nothing");
    #[cfg(unix)]
    {
        let link = dir.join("linked.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(RealDisk.exchange(&link, b"x").unwrap_err().step, Step::Check);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_directory_sync_still_hands_back_what_was_replaced() {
    use super::disk::Displaced;
    let dir = scratch("exchange-syncdir");
    let path = dir.join("note.md");
    std::fs::write(&path, "theirs").unwrap();
    let disk = FaultyDisk::new();
    disk.fail(Step::SyncDir, "note.md", Some(1));
    let done = disk.exchange(&path, b"mine").unwrap();
    assert_eq!(done.unsure.as_ref().map(|e| e.step), Some(Step::SyncDir), "in place, durability unknown");
    if exchanges_here(&dir) {
        assert_eq!(done.displaced, Displaced::Bytes(b"theirs".to_vec()), "their bytes survive the failed sync");
    }
    assert_eq!(std::fs::read(&path).unwrap(), b"mine");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn a_fifo_or_a_directory_at_the_path_is_put_back_and_the_save_fails() {
    let dir = scratch("exchange-odd");
    if !exchanges_here(&dir) {
        eprintln!("no atomic exchange on this filesystem; a plain rename refuses these itself");
        return;
    }
    use std::os::unix::fs::FileTypeExt as _;
    let fifo = dir.join("fifo.md");
    let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let err = RealDisk.exchange(&fifo, b"x").unwrap_err();
    assert_eq!(err.step, Step::Rename, "{err}");
    assert!(std::fs::symlink_metadata(&fifo).unwrap().file_type().is_fifo(), "the FIFO is back, nothing hangs");
    let folder = dir.join("folder.md");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("inside"), "kept").unwrap();
    assert_eq!(RealDisk.exchange(&folder, b"x").unwrap_err().step, Step::Rename);
    assert_eq!(std::fs::read_to_string(folder.join("inside")).unwrap(), "kept", "the folder is back where it was");
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert!(!names.iter().any(|n| n.ends_with(".tmp")), "nothing hidden left behind: {names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn a_link_swapped_in_after_the_check_is_not_followed() {
    let dir = scratch("exchange-late-link");
    if !exchanges_here(&dir) {
        return;
    }
    let elsewhere = dir.join("elsewhere");
    std::fs::write(&elsewhere, "not ours to read").unwrap();
    let path = dir.join("note.md");
    std::os::unix::fs::symlink(&elsewhere, &path).unwrap();
    // As if the link appeared after `exchange`'s own check: the swap itself.
    let temp = dir.join(".note.md.test.tmp");
    std::fs::write(&temp, "mine").unwrap();
    let err = super::exchange::swap_in(&temp, &path, 1024).unwrap_err();
    assert_eq!(err.step, Step::Rename);
    assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink(), "the link is back");
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "not ours to read");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn a_replaced_file_too_large_to_read_back_is_kept_unread() {
    use super::disk::Displaced;
    let dir = scratch("exchange-large");
    if !exchanges_here(&dir) {
        return;
    }
    let path = dir.join("note.md");
    std::fs::write(&path, "a long note from another program").unwrap();
    let temp = dir.join(".note.md.test.tmp");
    std::fs::write(&temp, "mine").unwrap();
    match super::exchange::swap_in(&temp, &path, 4).unwrap() {
        Displaced::Unread(kept) => {
            assert_eq!(std::fs::read_to_string(&kept).unwrap(), "a long note from another program", "never deleted unread");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(target_os = "linux")]
#[test]
fn a_fingerprint_checked_replace_only_lands_over_the_file_it_expected() {
    use super::exchange::{replace_if_fingerprint, Guarded};
    let dir = scratch("exchange-fingerprint");
    if !exchanges_here(&dir) {
        eprintln!("no atomic exchange on this filesystem; a plain rename replaces whatever arrives");
        return;
    }
    let path = dir.join("note.md");
    // What is at the path is not the file the caller agreed to replace — another program wrote it
    // between the caller's look and this replace.
    std::fs::write(&path, "theirs").unwrap();
    let refused = replace_if_fingerprint(&path, b"mine", 1024, Some(fp("expected"))).unwrap();
    assert_eq!(refused, Guarded::Refused);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs", "nothing was written");
    // What did arrive is back at the path, and ours is gone with it: nothing of anyone's is left
    // under a temp name beside it.
    let names: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["note.md"], "nothing else is left in the folder");
    // The file the caller did agree to replace: it lands.
    let placed = replace_if_fingerprint(&path, b"mine", 1024, Some(fp("theirs"))).unwrap();
    assert_eq!(placed, Guarded::Placed);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    let _ = std::fs::remove_dir_all(&dir);
}
