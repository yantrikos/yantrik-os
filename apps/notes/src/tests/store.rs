//! The vault's behaviours, through the store and the disk seam: compatibility with the files
//! Notes always wrote, the refusals, the Trash, and every way a save can fail, each saying how
//! much reached the disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yantrik_app::documents::disk::{FaultyDisk, Step};
use yantrik_app::documents::{DocumentId, Durability, Fingerprint, SaveTicket};

use crate::note::{Note, NoteText};
use crate::store::{read, validate, SaveFailed, SaveJob, Saved, Vault, LIMIT};

pub fn fixture(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "yantrik-notes-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Saves `content` as note `id`, whose files hold `on_disk`, as the document's save job would.
pub fn save(vault: &Vault, id: &str, on_disk: Option<&NoteText>, content: &NoteText) -> Result<Saved, SaveFailed> {
    vault.save(&SaveJob {
        id: id.into(),
        ticket: SaveTicket {
            document: DocumentId(1),
            seq: 1,
            content: content.clone(),
            baseline: on_disk.map(|d| Fingerprint::of_bytes(d.text.as_bytes())),
            unsure: None,
        },
        on_disk: on_disk.cloned(),
        meta_unsure: None,
        modified: 0,
        alike: Vec::new(),
    })
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn vault_compatibility_atomic_conflicts_and_recovery() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = fixture("store");
    let vault = Vault::real(dir.clone());
    vault.initialize().unwrap();
    std::fs::write(dir.join("old.md"), "# Existing\n\nHello 🦀\n").unwrap();
    std::fs::write(dir.join("old.meta"), "pinned:true\ntags:work, ideas\n").unwrap();
    let mut notes = vault.load().unwrap().notes;
    let n = notes.remove(0);
    assert_eq!(n.title(), "Existing");
    assert_eq!(n.field("tags"), "work, ideas");
    assert!(n.stored);
    let mut edited = n.content.clone();
    edited.text.push_str("A new line\n");
    edited.set_field("notebook", "Work");
    let saved = save(&vault, &n.id, Some(&n.content), &edited).unwrap();
    assert_eq!(saved.content, edited);
    assert_eq!(text(&dir.join(&n.id)), edited.text);
    assert!(!dir.join(".recovery-v2/pending.json").exists(), "the journal goes once the save lands");
    std::fs::write(dir.join(&n.id), "# External edit\n").unwrap();
    let mut draft = edited.clone();
    draft.text.push_str("Protected draft");
    let refused = save(&vault, &n.id, Some(&edited), &draft).unwrap_err();
    assert!(refused.error.sentence.contains("changed on disk"));
    assert!(refused.error.conflict);
    assert_eq!(refused.error.durability, Durability::NotWritten);
    let loaded = vault.load().unwrap();
    assert!(loaded.notice.contains("Recovered"));
    assert_eq!(loaded.notes.len(), 2);
    assert!(loaded.notes.iter().any(|n| n.content.text.ends_with("Protected draft")));
    assert_eq!(text(&dir.join("old.md")), "# External edit\n");
    let n = loaded.notes.iter().find(|n| n.id == "old.md").unwrap().clone();
    let mut more = n.content.clone();
    more.text.push_str("bad");
    std::fs::set_permissions(dir.join("old.md"), std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(save(&vault, "old.md", Some(&n.content), &more).is_err(), "a read-only note is not replaced");
    std::fs::set_permissions(dir.join("old.md"), std::fs::Permissions::from_mode(0o600)).unwrap();
    symlink(dir.join("old.md"), dir.join("link.md")).unwrap();
    assert!(read(&dir.join("link.md"), LIMIT).is_err());
    assert!(save(&vault, "../escape.md", Some(&n.content), &more).is_err());
}

#[test]
fn trash_restore_no_clobber_and_bounded_reads() {
    let dir = fixture("trash");
    let vault = Vault::real(dir.clone());
    vault.initialize().unwrap();
    let mut n = Note::blank("Trash me");
    n.content.set_field("notebook", "Research");
    let saved = save(&vault, &n.id, None, &n.content).unwrap();
    vault.trash(&n.id, &saved.content, saved.modified).unwrap();
    assert!(!dir.join(&n.id).exists());
    let notes = vault.load().unwrap().notes;
    assert_eq!(notes.len(), 1);
    assert!(notes[0].trash);
    std::fs::write(dir.join(&n.id), "collision").unwrap();
    assert!(vault.restore(&notes[0]).is_err());
    assert_eq!(text(&dir.join(&n.id)), "collision");
    std::fs::remove_file(dir.join(&n.id)).unwrap();
    let r = vault.restore(&notes[0]).unwrap();
    assert_eq!(r.content.text, n.content.text);
    assert_eq!(r.content.meta, n.content.meta);
    assert!(validate(&"x".repeat(LIMIT + 1)).is_err());
    assert!(validate("\0").is_err());
    let huge = dir.join("large.md");
    std::fs::File::create(&huge).unwrap().set_len(2 * 1024 * 1024 * 1024).unwrap();
    assert!(read(&huge, LIMIT).is_err());
    std::fs::remove_file(huge).unwrap();
    let fifo = dir.join("pipe.md");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(read(&fifo, LIMIT).is_err());
}

#[test]
fn an_export_and_a_new_note_never_replace_a_file_already_there() {
    let dir = fixture("no-clobber");
    let vault = Vault::real(dir.join("vault"));
    vault.initialize().unwrap();
    let out = dir.join("out.md");
    std::fs::write(&out, "mine").unwrap();
    assert!(vault.export(&out, "theirs").is_err());
    assert_eq!(text(&out), "mine");
    vault.export(&dir.join("fresh.md"), "exported").unwrap();
    assert_eq!(text(&dir.join("fresh.md")), "exported");
    // A file named like a new note before its first save: the save is refused, not a replace.
    let n = Note::blank("Fresh");
    std::fs::write(dir.join("vault").join(&n.id), "someone else's").unwrap();
    assert!(save(&vault, &n.id, None, &n.content).unwrap_err().error.conflict);
    assert_eq!(text(&dir.join("vault").join(&n.id)), "someone else's");
}

/// A vault over a disk that fails where the test says, with one saved note in it.
fn faulty(name: &str) -> (PathBuf, Arc<FaultyDisk>, Vault, Note) {
    let dir = fixture(name);
    let disk = Arc::new(FaultyDisk::new());
    let vault = Vault::new(dir.clone(), disk.clone());
    vault.initialize().unwrap();
    let mut n = Note::blank("Kept");
    n.content.set_field("tags", "one");
    let saved = save(&vault, &n.id, None, &n.content).unwrap();
    n.content = saved.content;
    (dir, disk, vault, n)
}

fn edited(n: &Note) -> NoteText {
    let mut c = n.content.clone();
    c.text.push_str("more\n");
    c.set_field("tags", "two");
    c
}

#[test]
fn a_save_that_fails_before_the_rename_leaves_the_note_as_it_was_and_wrote_nothing() {
    for step in [Step::CreateTemp, Step::Write, Step::SyncFile, Step::Rename] {
        let (dir, disk, vault, n) = faulty(&format!("before-{step:?}"));
        disk.fail(step, ".md", Some(1));
        let failed = save(&vault, &n.id, Some(&n.content), &edited(&n)).unwrap_err();
        assert_eq!(failed.error.durability, Durability::NotWritten, "{step:?}");
        assert!(failed.error.landed.is_none());
        assert_eq!(text(&dir.join(&n.id)), n.content.text, "{step:?}: the file is as it was");
        assert!(dir.join(".recovery-v2/pending.json").exists(), "the journal keeps the draft");
        // The next start offers the draft back as a note of its own.
        let loaded = vault.load().unwrap();
        assert!(loaded.notice.contains("Recovered"), "{step:?}");
        assert!(loaded.notes.iter().any(|r| r.id.starts_with("recovered-") && r.content.text.ends_with("more\n")));
    }
}

#[test]
fn a_journal_that_cannot_be_written_stops_the_save_before_anything_else() {
    let (dir, disk, vault, n) = faulty("journal");
    disk.fail(Step::Write, "pending.json", None);
    let failed = save(&vault, &n.id, Some(&n.content), &edited(&n)).unwrap_err();
    assert_eq!(failed.error.durability, Durability::NotWritten);
    assert_eq!(text(&dir.join(&n.id)), n.content.text);
    assert_eq!(text(&dir.join(&n.id).with_extension("meta")), n.content.meta);
    assert_eq!(*disk.hits.lock().unwrap(), vec![dir.join(".recovery-v2/pending.json")], "the journal was the write that failed");
}

#[test]
fn a_directory_sync_that_fails_after_the_rename_is_not_called_saved() {
    let (dir, disk, vault, n) = faulty("syncdir");
    disk.fail(Step::SyncDir, ".md", Some(1));
    let want = edited(&n);
    let failed = save(&vault, &n.id, Some(&n.content), &want).unwrap_err();
    assert_eq!(failed.error.durability, Durability::Unknown);
    assert_eq!(failed.error.landed, Some(Fingerprint::of_bytes(want.text.as_bytes())), "our bytes, if they are there");
    assert_eq!(text(&dir.join(&n.id)), want.text);
}

#[test]
fn a_sidecar_that_fails_after_the_text_is_content_only_and_the_journal_recovers_it() {
    let (dir, disk, vault, n) = faulty("meta");
    disk.fail(Step::Rename, ".meta", Some(1));
    let want = edited(&n);
    let failed = save(&vault, &n.id, Some(&n.content), &want).unwrap_err();
    assert_eq!(failed.error.durability, Durability::ContentOnly);
    assert!(failed.meta_unsure.is_none(), "the sidecar did not land");
    assert_eq!(text(&dir.join(&n.id)), want.text, "the text landed");
    assert_eq!(text(&dir.join(&n.id).with_extension("meta")), n.content.meta, "the sidecar did not");
    // The retry: the text on disk is ours (`unsure`), so it is not mistaken for an outside edit.
    let retry = vault.save(&SaveJob {
        id: n.id.clone(),
        ticket: SaveTicket {
            document: DocumentId(1),
            seq: 2,
            content: want.clone(),
            baseline: Some(Fingerprint::of_bytes(n.content.text.as_bytes())),
            unsure: failed.error.landed.clone(),
        },
        on_disk: Some(n.content.clone()),
        meta_unsure: None,
        modified: 0,
        alike: Vec::new(),
    });
    assert!(retry.is_ok(), "{:?}", retry.err());
    assert_eq!(text(&dir.join(&n.id).with_extension("meta")), want.meta);
}

#[test]
fn a_sidecar_left_by_a_failed_save_is_ours_at_the_next_start() {
    let (_dir, disk, vault, n) = faulty("meta-recovered");
    disk.fail(Step::Write, ".meta", Some(1));
    save(&vault, &n.id, Some(&n.content), &edited(&n)).unwrap_err();
    let loaded = vault.load().unwrap();
    assert!(loaded.notice.contains("Recovered"), "the journal holds the whole save");
    let recovered = loaded.notes.iter().find(|r| r.id.starts_with("recovered-")).expect("a recovered note");
    assert_eq!(recovered.field("tags"), "two");
}

/// The file must be byte for byte what the note was read from: a fingerprint that happens to
/// match (or is made to) does not let a save replace someone else's text.
#[test]
fn the_check_compares_the_text_itself_not_its_fingerprint() {
    let dir = fixture("exact");
    let vault = Vault::real(dir.clone());
    vault.initialize().unwrap();
    let n = Note::blank("Exact");
    save(&vault, &n.id, None, &n.content).unwrap();
    std::fs::write(dir.join(&n.id), "theirs").unwrap();
    let mut mine = n.content.clone();
    mine.text.push_str("mine");
    // A ticket whose fingerprint matches the file now, for a note read as something else.
    let refused = vault.save(&SaveJob {
        id: n.id.clone(),
        ticket: SaveTicket {
            document: DocumentId(1),
            seq: 1,
            content: mine,
            baseline: Some(Fingerprint::of_bytes(b"theirs")),
            unsure: None,
        },
        on_disk: Some(n.content.clone()),
        meta_unsure: None,
        modified: 0,
        alike: Vec::new(),
    });
    assert!(refused.unwrap_err().error.conflict);
    assert_eq!(text(&dir.join(&n.id)), "theirs");
}
