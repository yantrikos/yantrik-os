//! Files past the exchange's 64 MiB read-back limit. A save's exchange never reads such a file
//! back; it leaves it under a hidden name beside the original. The editor then keeps it: given a
//! visible name and said, unless it is exactly the file the caller agreed to replace.

use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use yantrik_app::documents::disk::{Disk, DiskError, Exchanged, RealDisk};

use super::{act, file, notice, press, started, typed, with_file};
use crate::command::{Command, WindowAction};

/// Past the read-back limit; sparse, so it costs no disk.
const BIG: u64 = 65 * 1024 * 1024;

fn big_file(path: &Path) {
    std::fs::File::create(path).unwrap().set_len(BIG).unwrap();
}

#[test]
fn overwriting_a_file_past_the_limit_replaces_it_as_agreed_and_leaves_nothing_hidden() {
    let app = started();
    let dir = super::scratch("big-overwrite");
    let target = dir.join("huge.log");
    big_file(&target);
    act(&app, "new", json!({ "text": "small now\n" })).unwrap();
    let refused = act(&app, "save_as", json!({ "path": target.display().to_string() })).unwrap_err();
    assert!(refused.contains("already exists"), "{refused}");
    act(&app, "save_as", json!({ "path": target.display().to_string(), "overwrite": true })).unwrap();
    assert_eq!(file(&target), "small now\n");
    let left: Vec<String> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, vec!["huge.log".to_string()], "the file agreed to was replaced, and nothing kept unread");
}

/// A disk where another program puts a file past the limit at the path, as a new file, in the
/// instant between the save's check and its replace.
struct BigRace;

impl Disk for BigRace {
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
        let theirs = path.with_file_name("theirs.part");
        big_file(&theirs);
        std::fs::rename(&theirs, path).unwrap();
        RealDisk.exchange(path, bytes)
    }
}

#[test]
fn a_file_past_the_limit_another_program_put_there_mid_save_is_kept_and_named() {
    let (app, path) = with_file("big-race", "first\n");
    app.run(Command::UseDisk(Arc::new(BigRace)));
    typed(&app, "mine\n");
    press(&app, WindowAction::Save);
    assert_eq!(file(&path), "mine\n");
    let kept = path.with_file_name("notes (outside version).txt");
    assert_eq!(std::fs::metadata(&kept).unwrap().len(), BIG, "kept unread, under a name a person sees");
    assert!(notice(&app).contains("its version is kept as"), "{}", notice(&app));
    let hidden: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(hidden.is_empty(), "no hidden copy left behind: {hidden:?}");
}

#[test]
fn a_file_past_the_limit_is_not_opened() {
    let app = started();
    let dir = super::scratch("big-open");
    let huge = dir.join("large");
    std::fs::File::create(&huge).unwrap().set_len(2 * 1024 * 1024 * 1024).unwrap();
    let err = act(&app, "open", json!({ "path": huge.display().to_string() })).unwrap_err();
    assert!(err.contains("1 MiB editing limit"), "{err}");
}
