//! The write lane: a change is made in memory, written in a job, and taken back when the file
//! refuses it, so the window never shows what the file does not hold; writes landing out of
//! order never put an older library over a newer one; and each call is answered about its own
//! change.

use std::sync::Arc;

use serde_json::json;

use super::open;
use crate::disk::testing::{Faulty, REFUSED};
use crate::lane::{Landed, Lane};
use crate::store::{file, Library, ALL};
use crate::Command;

/// What the conformance probe forces with a read-only store directory: the app must refuse in
/// words, say the same thing in `describe.notice`, and leave the file exactly as it was. The
/// probe's own shape, through every change.
#[test]
fn a_change_the_file_refuses_is_refused_said_and_taken_back() {
    let f = open();
    let (first, second) = f.seeded();
    let bytes_before = f.file_bytes().unwrap();
    // What is kept, snippets and collections; the id counters only ever go up (#938 review, M3).
    let kept = |f: &super::Fixture| f.app.model(|m| (m.library.snippets().to_vec(), m.library.collections().to_vec()));
    let before = kept(&f);
    f.disk.refuse(true);

    let e = f.act("new", json!({ "title": "Should not be kept", "language": "Shell", "code": "echo no" })).unwrap_err();
    assert!(e.contains(REFUSED), "refused in words: {e}");
    assert!(f.state()["notice"].as_str().unwrap().contains(REFUSED), "the same failure is in describe.notice");
    assert_eq!(kept(&f), before, "the failed create is not in memory");
    assert!(f.act("save", json!({ "id": first.to_string(), "title": "Renamed" })).is_err());
    assert!(f.act("delete", json!({ "id": second.to_string() })).is_err());
    assert!(f.act("toggle_favorite", json!({ "id": first.to_string() })).is_err());
    f.app.ui(|ui| ui.invoke_collection_create("Infra".into()));
    f.quiet();
    assert_eq!(kept(&f), before, "each one taken back");
    assert_eq!(f.state()["total"], json!(2));
    assert_eq!(f.file_bytes().unwrap(), bytes_before, "and nothing was written while it refused");

    // With the way clear the same change works, which is what makes the failure above a property
    // of the write and not of the arguments.
    f.disk.refuse(false);
    let kept = f.act("save", json!({ "id": first.to_string(), "title": "Renamed" })).unwrap();
    assert_eq!(kept["title"], json!("Renamed"));
    assert_eq!(f.on_disk().get(first).unwrap().title, "Renamed");
    assert_eq!(f.notice(), "", "what worked clears the notice");
}

/// The typing a refused move was keeping stays where the person can retry it or copy it out, on
/// its own snippet, and is kept in the draft lane meanwhile.
#[test]
fn typing_the_file_refused_stays_in_the_editor() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Not yet kept", "journalctl --since today");
    f.disk.refuse(true);
    f.app.run(Command::Select(borrow));
    f.quiet();
    assert_eq!(f.app.model(|m| m.selected), journal, "back on the snippet whose typing was not kept");
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Not yet kept");
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal");
    assert!(f.app.model(move |m| m.editor_dirty() && m.drafted(journal).is_some()));
    assert!(f.notice().starts_with("Not saved: ") && f.notice().ends_with("Your text is still in the editor, and kept as a draft until it is saved."), "{}", f.notice());
    f.disk.refuse(false);
    f.app.ui(move |ui| ui.invoke_snip_select(borrow));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Not yet kept");
}

/// A new snippet whose write was refused is not left on screen: the editor goes back to where it
/// was.
#[test]
fn a_refused_new_leaves_the_editor_where_it_was() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.disk.refuse(true);
    f.app.ui(|ui| ui.invoke_snip_new());
    assert_eq!(f.app.model(|m| m.open().map(|s| s.title.clone())).as_deref(), Some("Untitled Snippet"), "drawn at once");
    f.quiet();
    assert_eq!(f.app.model(|m| m.selected), journal);
    assert_eq!(f.state()["total"], json!(2));
}

/// Two writes landing in the other order: the file never goes back to the older library, and the
/// older write says it is covered.
#[test]
fn a_write_landing_after_a_newer_one_is_covered_and_not_made() {
    let dir = std::env::temp_dir().join(format!("snippets-lane-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = file::state_path(&dir);
    let disk = Arc::new(Faulty::default());
    let mut lane = Lane::new(disk.clone(), path.clone());
    let mut library = Library::default();
    library.create("One", "Rust", "1", "", ALL).unwrap();
    let older = lane.ticket();
    let older_job = lane.job(older, file::encode(&library).unwrap());
    library.create("Two", "Rust", "2", "", ALL).unwrap();
    let newer = lane.ticket();
    let newer_job = lane.job(newer, file::encode(&library).unwrap());
    assert!(matches!(newer_job(), Landed::Saved));
    assert!(matches!(older_job(), Landed::Covered));
    assert_eq!(file::read_back(&path).unwrap().snippets().len(), 2, "the newer library stands");
    assert_eq!(disk.writes(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A failed write reads the file back under the lock, and every write of its epoch still waiting
/// is not made, because each carries the change that failed; the next epoch writes again.
#[test]
fn a_failed_write_spends_its_epoch_and_the_next_one_writes() {
    let dir = std::env::temp_dir().join(format!("snippets-epoch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = file::state_path(&dir);
    let disk = Arc::new(Faulty::default());
    let mut lane = Lane::new(disk.clone(), path.clone());
    let mut library = Library::default();
    library.create("Kept", "Rust", "1", "", ALL).unwrap();
    let first = lane.ticket();
    assert!(matches!(lane.job(first, file::encode(&library).unwrap())(), Landed::Saved));
    library.create("Refused", "Rust", "2", "", ALL).unwrap();
    let failing = lane.ticket();
    let after = lane.ticket();
    disk.refuse(true);
    match lane.job(failing, file::encode(&library).unwrap())() {
        Landed::Failed { on_disk, .. } => assert_eq!(on_disk.unwrap().snippets().len(), 1, "the file as last kept"),
        _ => panic!("the write failed"),
    }
    disk.refuse(false);
    assert!(matches!(lane.job(after, file::encode(&library).unwrap())(), Landed::Undone(_)), "built on the change that failed");
    assert!(lane.spend(failing));
    assert!(!lane.spend(after), "an epoch is spent once");
    let next = lane.ticket();
    assert!(matches!(lane.job(next, file::encode(&library).unwrap())(), Landed::Saved));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The person's delete pressed while the Mind's `new` is still being written does not become the
/// Mind's answer: each call is answered about its own change (review of #921, finding 1).
#[test]
fn a_press_meanwhile_never_becomes_a_minds_answer() {
    let f = open();
    let (journal, _) = f.seeded();
    let pending = f
        .app
        .act("new", json!({ "title": "The Mind's", "language": "Shell", "code": "echo mind" }))
        .unwrap();
    assert!(pending.is_owed());
    f.app.ui(move |ui| ui.invoke_snip_delete(journal));
    f.quiet();
    let answer = pending.settle().unwrap();
    assert_eq!(answer["title"], json!("The Mind's"));
    assert!(answer.get("deleted").is_none(), "{answer}");
    assert!(f.on_disk().get(journal).is_none());
}

/// A write job that ends without a result (a panic, a full pool) claims nothing: the call is
/// refused and the file is read again.
#[test]
fn a_write_that_did_not_finish_reads_the_file_again() {
    let f = open();
    let (journal, _) = f.seeded();
    f.disk.panic_next();
    let e = f.act("delete", json!({ "id": journal.to_string() })).unwrap_err();
    assert!(e.contains("did not finish") || e.contains("work failed"), "{e}");
    assert!(f.notice().starts_with("Saving did not finish"), "{}", f.notice());
    assert_eq!(f.app.model(|m| m.library.snippets().to_vec()), f.on_disk().snippets(), "memory is the file again");
    assert!(f.on_disk().get(journal).is_some(), "the write never happened, and the window says so");
    assert_eq!(f.app.model(|m| m.work), 0, "nothing is left counted in flight");
    // The writer is not left stuck by the panic: the next change writes.
    let kept = f.act("delete", json!({ "id": journal.to_string() })).unwrap();
    assert_eq!(kept["still_present"], json!(false));
    assert!(f.on_disk().get(journal).is_none());
}
