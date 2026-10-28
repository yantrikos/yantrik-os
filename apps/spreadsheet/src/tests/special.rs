//! Files that are not what they seem: pipes, devices, links, the mode a new file is given. Open
//! reads regular files only, and a save is never through a link.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use serde_json::json;

use super::{act, file, raw, scratch, started};
use crate::files::open::{open_sheet, OpenedSheet};

/// Open `path` on a thread of its own, so a regression fails the test instead of hanging it.
fn open_within_seconds(path: PathBuf) -> Result<OpenedSheet, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(open_sheet(&path, false));
    });
    rx.recv_timeout(std::time::Duration::from_secs(5)).expect("open blocked or read without end")
}

#[test]
fn a_fifo_a_device_and_a_link_to_a_device_are_refused_at_once() {
    let dir = scratch("special");
    let fifo = dir.join("pipe.csv");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let zero = dir.join("zero.csv");
    std::os::unix::fs::symlink("/dev/zero", &zero).unwrap();
    for path in [fifo, zero, "/dev/zero".into()] {
        let e = open_within_seconds(path.clone()).unwrap_err();
        assert!(e.contains("not a regular file"), "{}: {e}", path.display());
    }
    let folder = open_within_seconds(dir.clone()).unwrap_err();
    assert!(folder.contains("not a regular file (a folder"), "{folder}");
}

#[test]
fn the_same_refusals_come_through_the_surface_and_change_nothing() {
    let app = started();
    let dir = scratch("special-surface");
    let fifo = dir.join("pipe.csv");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let e = act(&app, "open", json!({ "path": fifo.display().to_string() })).unwrap_err();
    assert!(e.contains("not a regular file"), "{e}");
    assert_eq!(super::state(&app)["path"], serde_json::Value::Null);
}

#[test]
fn a_new_csv_is_written_by_this_user_and_readable_by_others_less_the_umask() {
    let app = started();
    let dir = scratch("mode");
    let path = dir.join("new.csv");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "1" })).unwrap();
    act(&app, "save", json!({ "path": path.display().to_string() })).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & !0o644, 0, "never more than 0o644: {mode:o}");
    assert_eq!(mode & 0o600, 0o600, "this user reads and writes it: {mode:o}");

    // A replaced file keeps the mode it had.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    act(&app, "save_over", json!({ "path": path.display().to_string() })).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(file(&path), "2\r\n");
}

#[test]
fn a_save_is_never_through_a_link_and_the_target_it_pointed_at_is_untouched() {
    let app = started();
    let dir = scratch("link");
    let target = dir.join("target.csv");
    std::fs::write(&target, "precious\r\n").unwrap();
    let link = dir.join("link.csv");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "overwritten" })).unwrap();
    for name in ["save", "save_over"] {
        let e = act(&app, name, json!({ "path": link.display().to_string() })).unwrap_err();
        assert!(!e.is_empty(), "{name}");
        assert_eq!(file(&target), "precious\r\n", "{name}: {e}");
    }
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "and the link is still a link");
}

#[test]
fn a_file_with_a_second_name_is_not_replaced_in_place() {
    let app = started();
    let dir = scratch("hardlink");
    let one = dir.join("one.csv");
    std::fs::write(&one, "shared\r\n").unwrap();
    std::fs::hard_link(&one, dir.join("two.csv")).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    let e = act(&app, "save_over", json!({ "path": one.display().to_string() })).unwrap_err();
    assert!(e.contains("not a regular file") || e.contains("link"), "{e}");
    assert_eq!(file(&dir.join("two.csv")), "shared\r\n");
}

#[test]
fn a_read_only_file_is_not_written() {
    let app = started();
    let dir = scratch("readonly");
    let path = dir.join("locked.csv");
    std::fs::write(&path, "locked\r\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "mine" })).unwrap();
    let e = act(&app, "save_over", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(e.contains("read-only"), "{e}");
    assert_eq!(file(&path), "locked\r\n");
    assert_eq!(raw(&app, "A1"), "mine");
}
