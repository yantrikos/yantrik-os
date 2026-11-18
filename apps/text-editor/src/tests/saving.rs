//! Saving: whole or not at all, never over another program's edit, never through a link, and a
//! failure said with how much of it reached the disk.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use yantrik_app::documents::disk::{Disk, DiskError, Exchanged, RealDisk, Step};

use super::{act, failing, file, modified, notice, opened, press, scratch, started, state, text, typed, with_file};
use crate::command::{Command, WindowAction};

fn no_temps(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
}

#[test]
fn a_save_keeps_the_files_mode_and_its_bytes_and_leaves_no_temp_behind() {
    let (app, path) = with_file("atomic", "hello 🦀\r\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    typed(&app, "hello 🦀\r\nsecond\r\n");
    assert!(modified(&app));
    press(&app, WindowAction::Save);
    assert_eq!(file(&path), "hello 🦀\r\nsecond\r\n");
    assert!(!modified(&app), "{}", notice(&app));
    assert_eq!(notice(&app), "Saved");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    assert!(no_temps(path.parent().unwrap()));
    assert_eq!(app.ui(|ui| ui.get_ending().to_string()), "CRLF");
}

#[test]
fn another_programs_edit_is_never_written_over() {
    let (app, path) = with_file("conflict", "first\n");
    typed(&app, "mine\n");
    // Written while the watch has not looked: the save's own check finds it.
    std::fs::write(&path, "external edit").unwrap();
    press(&app, WindowAction::Save);
    assert!(notice(&app).contains("changed on disk"), "{}", notice(&app));
    assert_eq!(file(&path), "external edit");
    assert!(modified(&app) && text(&app) == "mine\n", "the draft is intact");
    let refused = act(&app, "save", json!({})).unwrap_err();
    assert!(refused.contains("Your draft is intact; use Save As"), "{refused}");
    // Save As keeps both versions.
    let copy = path.with_file_name("mine.txt");
    act(&app, "save_as", json!({ "path": copy.display().to_string() })).unwrap();
    assert_eq!((file(&path), file(&copy)), ("external edit".into(), "mine\n".into()));
    assert!(!modified(&app));
}

#[test]
fn a_link_or_a_second_name_or_a_read_only_file_is_not_replaced() {
    let app = started();
    let dir = scratch("links");
    let path = dir.join("original.txt");
    std::fs::write(&path, "kept\n").unwrap();
    std::os::unix::fs::symlink(&path, dir.join("linked.txt")).unwrap();
    act(&app, "new", json!({ "text": "over the link" })).unwrap();
    let err = act(&app, "save_as", json!({ "path": dir.join("linked.txt").display().to_string(), "overwrite": true }))
        .unwrap_err();
    assert!(err.contains("linked or is not a regular file"), "{err}");
    std::fs::hard_link(&path, dir.join("hard.txt")).unwrap();
    let err = act(&app, "save_as", json!({ "path": path.display().to_string(), "overwrite": true })).unwrap_err();
    assert!(err.contains("linked or is not a regular file"), "{err}");
    let ro = dir.join("read-only.txt");
    std::fs::write(&ro, "ro\n").unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    let err = act(&app, "save_as", json!({ "path": ro.display().to_string(), "overwrite": true })).unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    assert_eq!((file(&path), file(&ro)), ("kept\n".into(), "ro\n".into()));
    assert!(no_temps(&dir));
    // Not a text file: never opened.
    std::fs::write(dir.join("binary"), [0xff, 0xfe]).unwrap();
    assert!(act(&app, "open", json!({ "path": dir.join("binary").display().to_string() })).is_err());
}

#[test]
fn a_failed_write_says_so_keeps_the_draft_and_changes_nothing_on_disk() {
    for step in [Step::CreateTemp, Step::Write, Step::SyncFile, Step::Rename] {
        let (app, path) = with_file("fails", "on disk\n");
        let disk = failing(&app);
        disk.fail(step, "notes.txt", Some(1));
        typed(&app, "typed\n");
        press(&app, WindowAction::Save);
        assert_eq!(file(&path), "on disk\n", "{step:?}: the file is as it was");
        assert!(notice(&app).starts_with("Save failed: injected failure"), "{step:?}: {}", notice(&app));
        assert!(notice(&app).ends_with("Your draft is still open."));
        assert!(modified(&app), "{step:?}: nothing is called saved");
        app.run_writes();
        assert!(super::draft_file(&app).exists(), "{step:?}: the text is in its draft");
        press(&app, WindowAction::Save);
        assert_eq!(file(&path), "typed\n", "{step:?}: the next Save writes it");
        assert!(!modified(&app));
    }
}

#[test]
fn a_save_whose_folder_sync_failed_is_not_called_saved_and_its_bytes_are_ours_after() {
    let (app, path) = with_file("syncdir", "on disk\n");
    let disk = failing(&app);
    disk.fail(Step::SyncDir, "notes.txt", Some(1));
    typed(&app, "typed\n");
    press(&app, WindowAction::Save);
    assert_eq!(file(&path), "typed\n", "the bytes are in place");
    assert!(modified(&app), "but their durability is not known, so it is not saved");
    // Our own unsure bytes are not another program's edit: the retry replaces them.
    press(&app, WindowAction::Save);
    assert!(!modified(&app), "{}", notice(&app));
    assert_eq!(notice(&app), "Saved");
}

/// A disk where another program writes the file in the instant between the save's check and its
/// replace.
struct Racing(Vec<u8>);

impl Disk for Racing {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.write_atomically(path, bytes)
    }
    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.create_new(path, bytes)
    }
    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        RealDisk.read(path)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        RealDisk.remove(path)
    }
    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        std::fs::write(path, &self.0).unwrap();
        RealDisk.exchange(path, bytes)
    }
}

#[test]
fn another_programs_write_that_a_save_replaced_is_kept_beside_the_file() {
    let (app, path) = with_file("race", "first\n");
    app.run(Command::UseDisk(Arc::new(Racing(b"theirs, in that instant\n".to_vec()))));
    typed(&app, "mine\n");
    press(&app, WindowAction::Save);
    let kept = path.with_file_name("notes (outside version).txt");
    assert_eq!(file(&path), "mine\n");
    assert_eq!(file(&kept), "theirs, in that instant\n", "never dropped");
    assert_eq!(std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o600, "kept private");
    assert!(notice(&app).contains("its version is kept as"), "{}", notice(&app));
    assert!(notice(&app).contains("notes (outside version).txt"));
}

#[test]
fn a_tab_with_no_file_asks_where_and_a_folder_that_is_not_there_is_said_in_the_dialog() {
    let app = started();
    typed(&app, "draft");
    press(&app, WindowAction::Save);
    assert_eq!(state(&app)["dialog"], 2, "Save on a tab with no file is Save As");
    let dir = scratch("dialog");
    press(&app, WindowAction::Confirm { path: dir.join("missing/no.txt").display().to_string() });
    assert_eq!(state(&app)["dialog"], 2, "the dialog stays with the reason");
    assert!(app.model(|m| m.dialog_error.contains("Cannot open folder")), "{}", notice(&app));
    assert!(modified(&app));
    press(&app, WindowAction::Confirm { path: dir.join("saved.txt").display().to_string() });
    assert_eq!(state(&app)["dialog"], 0);
    assert_eq!(file(&dir.join("saved.txt")), "draft");
    assert!(!modified(&app));
}

#[test]
fn a_file_moved_while_open_can_still_be_saved_to_where_it_went() {
    let (app, from) = with_file("moved", "first\n");
    typed(&app, "second\n");
    let archive = from.parent().unwrap().join("archive");
    std::fs::create_dir(&archive).unwrap();
    let to = archive.join("notes.txt");
    std::fs::rename(&from, &to).unwrap(); // what Files does, before the watch has looked
    let refused = act(&app, "save", json!({})).unwrap_err();
    assert!(refused.contains(&to.display().to_string()), "the refusal names where it went: {refused}");
    assert!(refused.contains("Save As") && refused.contains("draft is intact"), "{refused}");
    // That Save As is not refused as a clobber: the file at `to` is this tab's own, moved.
    act(&app, "save_as", json!({ "path": to.display().to_string() })).expect("its own file, moved");
    assert_eq!(file(&to), "second\n");
    assert!(!modified(&app));
}

#[test]
fn a_file_really_deleted_under_the_draft_is_written_back_with_permission() {
    let (app, path) = with_file("deleted", "kept\n");
    std::fs::remove_file(&path).unwrap();
    let refused = act(&app, "save", json!({})).unwrap_err();
    assert!(refused.contains("moved or deleted") && refused.contains("overwrite=true"), "{refused}");
    act(&app, "save_as", json!({ "path": path.display().to_string(), "overwrite": true })).expect("written back");
    assert_eq!(file(&path), "kept\n");
}

#[test]
fn a_file_opened_twice_is_one_tab() {
    let app = started();
    let path = scratch("twice").join("a.txt");
    opened(&app, &path, "a\n");
    act(&app, "new", json!({})).unwrap();
    let answer = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(answer["tab"], 0, "brought forward: {answer}");
    assert_eq!(answer["tabs"], 2);
}
