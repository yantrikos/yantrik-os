//! The vault's version marker, `.vault-version`: written once on a vault without it, new or old;
//! never rewritten; whole or not at all; one valid file when two copies make it at once; read at
//! every load; and never a note.

use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::sync::{Arc, Barrier};

use serde_json::json;
use yantrik_app::documents::disk::{Disk, DiskError, FaultyDisk, RealDisk, Step};

use super::store::fixture;
use super::{act, note, settle, started, state, vault};
use crate::command::{Command, WindowAction};
use crate::store::version::{Format, MARKER};
use crate::store::Vault;

fn marker(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(MARKER)).ok()
}

/// The marker's inode and bytes: the same pair after a load means it was not written again.
fn identity(dir: &Path) -> (u64, String) {
    (std::fs::metadata(dir.join(MARKER)).unwrap().ino(), marker(dir).unwrap())
}

#[test]
fn a_new_vault_is_marked_at_the_first_start() {
    let app = started();
    assert_eq!(marker(&vault(&app)).as_deref(), Some("2\n"));
    assert_eq!(Vault::real(vault(&app)).format(), Format::Current);
    assert_eq!(state(&app)["notice"], "", "a writable library says nothing about it");
}

#[test]
fn an_old_vault_without_a_marker_is_marked_and_nothing_else_changes() {
    let dir = fixture("old-vault");
    std::fs::write(dir.join("old.md"), "# Existing\n\nHello\n").unwrap();
    std::fs::write(dir.join("old.meta"), "pinned:true\n").unwrap();
    let loaded = Vault::real(dir.clone()).load().unwrap();
    assert_eq!(loaded.read_only, None);
    assert_eq!(marker(&dir).as_deref(), Some("2\n"));
    assert_eq!(loaded.notes.len(), 1, "the marker is not a note");
    assert_eq!(std::fs::read_to_string(dir.join("old.md")).unwrap(), "# Existing\n\nHello\n");
    assert_eq!(std::fs::read_to_string(dir.join("old.meta")).unwrap(), "pinned:true\n");
}

#[test]
fn the_marker_is_read_at_every_load_and_never_written_again() {
    let dir = fixture("never-rewritten");
    // As a person or an older copy might have left it: no line break, still format 2.
    std::fs::write(dir.join(MARKER), "2").unwrap();
    let before = identity(&dir);
    let by_hand = Vault::real(dir.clone());
    for _ in 0..2 {
        assert_eq!(by_hand.load().unwrap().read_only, None);
        assert_eq!(identity(&dir), before, "the marker was written again");
    }

    let app = started();
    let made = identity(&vault(&app));
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!(identity(&vault(&app)), made, "a reload wrote the marker again");
}

#[test]
fn a_marker_that_cannot_be_made_is_not_left_half_made_and_the_vault_is_only_read() {
    let dir = fixture("unmade");
    std::fs::write(dir.join("kept.md"), "# Kept\n").unwrap();
    let disk = Arc::new(FaultyDisk::new());
    disk.fail(Step::Write, MARKER, None);
    let loaded = Vault::new(dir.clone(), disk).load().unwrap();
    assert_eq!(
        loaded.read_only.as_deref(),
        Some(
            "This library is open read-only: Notes could not write its version file (injected \
             failure at Write). Make the folder writable, then Refresh."
        )
    );
    assert_eq!(loaded.notes.len(), 1, "reads still work");
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names, ["kept.md"], "no marker, no temp file, no folders made");
}

#[test]
fn an_interrupted_save_waits_in_its_journal_while_the_vault_is_read_only() {
    let dir = fixture("journal-waits");
    let journal = dir.join(".recovery-v2/pending.json");
    let vault = Vault::real(dir.clone());
    vault.load().unwrap();
    std::fs::write(dir.join("a.md"), "# A\n").unwrap();
    let disk = Arc::new(FaultyDisk::new());
    disk.fail(Step::Rename, ".md", None);
    let interrupted = Vault::new(dir.clone(), disk.clone());
    let on_disk = crate::note::NoteText { text: "# A\n".into(), meta: String::new() };
    let draft = crate::note::NoteText { text: "# A\n\ndraft\n".into(), meta: String::new() };
    super::store::save(&interrupted, "a.md", Some(&on_disk), &draft).unwrap_err();
    assert!(journal.exists(), "the save left its journal");

    std::fs::remove_file(dir.join(MARKER)).unwrap();
    disk.heal();
    disk.fail(Step::CreateTemp, MARKER, None);
    let read_only = interrupted.load().unwrap();
    assert!(read_only.read_only.is_some());
    assert_eq!(read_only.notes.len(), 1, "no recovered note is made");
    assert!(journal.exists(), "the journal is kept for a load that may write");

    disk.heal();
    let writable = interrupted.with_access(read_only.read_only).load().unwrap();
    assert_eq!(writable.read_only, None);
    assert!(writable.notice.starts_with("Recovered the interrupted save"), "{}", writable.notice);
    assert!(!journal.exists());
}

#[test]
fn a_newer_or_unreadable_marker_found_at_a_load_opens_the_vault_read_only_and_untouched() {
    for (written, said) in [
        (
            "3\n",
            "This library is open read-only: a newer Notes (format 3) has written it, and this Notes \
             writes format 2. Nothing in it was changed.",
        ),
        (
            "two\n",
            "This library is open read-only: its version file cannot be read (it does not hold a \
             format number). Nothing in it was changed.",
        ),
        (
            "1\n",
            "This library is open read-only: its version file cannot be read (it says format 1, \
             which no Notes wrote). Nothing in it was changed.",
        ),
    ] {
        let dir = fixture("not-ours");
        std::fs::write(dir.join(MARKER), written).unwrap();
        std::fs::write(dir.join("a.md"), "# A\n").unwrap();
        let before = super::snapshot(&dir);
        let loaded = Vault::real(dir.clone()).load().unwrap();
        assert_eq!(loaded.read_only.as_deref(), Some(said));
        assert_eq!(loaded.notes.len(), 1);
        assert_eq!(super::snapshot(&dir), before, "a load of {written:?} changed the vault");
    }
}

/// A disk on which another copy makes the marker a moment before this one does.
struct BeatenToIt;

impl Disk for BeatenToIt {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.write_atomically(path, bytes)
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        if path.ends_with(MARKER) {
            RealDisk.create_new(path, b"2\n")?;
        }
        RealDisk.create_new(path, bytes)
    }

    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        RealDisk.read(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        RealDisk.remove(path)
    }
}

#[test]
fn two_copies_making_the_marker_at_once_end_with_one_valid_file() {
    // The other copy wins: this one's create fails, nothing is replaced, and its marker is read.
    let dir = fixture("beaten");
    assert_eq!(Vault::new(dir.clone(), Arc::new(BeatenToIt)).mark(), Ok(()));
    assert_eq!(marker(&dir).as_deref(), Some("2\n"));

    // And for real, eight at once.
    let dir = fixture("race");
    let start = Arc::new(Barrier::new(8));
    let copies: Vec<_> = (0..8)
        .map(|_| {
            let (vault, start) = (Vault::real(dir.clone()), start.clone());
            std::thread::spawn(move || {
                start.wait();
                vault.mark()
            })
        })
        .collect();
    for copy in copies {
        assert_eq!(copy.join().unwrap(), Ok(()), "every copy may write the vault");
    }
    let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names, [MARKER], "one marker, and no temp file left");
    assert_eq!(marker(&dir).as_deref(), Some("2\n"));
}

#[test]
fn the_marker_is_never_a_note() {
    let app = started();
    note(&app, "Groceries", "milk");
    assert!(vault(&app).join(MARKER).exists());
    assert_eq!(state(&app)["note_count"], 1, "the listing holds the note and not the marker");
    assert_eq!(act(&app, "read_notes", json!({})).unwrap()["matching"], 1);
    assert_eq!(act(&app, "search", json!({ "query": "vault-version" })).unwrap()["matches"], 0);
    let refused = act(&app, "open_note", json!({ "title": MARKER })).unwrap_err();
    assert!(refused.starts_with("No note is called \".vault-version\""), "{refused}");

    act(&app, "trash", json!({})).unwrap();
    assert_eq!(state(&app)["trash_count"], 1, "the Trash holds the note, not the marker");
    let trash: Vec<_> = std::fs::read_dir(vault(&app).join(".trash-v2")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(trash.len(), 1);
    assert!(marker(&vault(&app)).is_some(), "trashing a note leaves the marker where it is");
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!((state(&app)["note_count"].clone(), state(&app)["notice"].clone()), (json!(0), json!("")));
}
