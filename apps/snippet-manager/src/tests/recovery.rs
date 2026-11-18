//! Unsaved text is never lost: it is kept in the draft lane, one draft per snippet, and offered
//! back on the recovery bar until the person chooses Restore (Restore as new) or Discard, as Notes
//! offers its drafts. Nothing is put back, and no snippet is made, without that press.

use serde_json::json;
use yantrik_app::app::WindowId;
use yantrik_app::mind::fake::FakeMind;

use super::{open, start_at, Fixture};
use crate::model::draft_name;
use crate::store::file;
use crate::Command;

fn bar(f: &Fixture) -> (String, i32) {
    f.app.ui(|ui| (ui.get_recovery_said().to_string(), ui.get_recovery()))
}

fn draft_on_disk(f: &Fixture, id: i32) -> bool {
    yantrik_app::drafts::path_in(f.paths.state_dir(), &draft_name(id)).exists()
}

/// Typing no button kept: the old app lost it when the window closed. The next start opens it in
/// the editor, unsaved, with the bar; Restore keeps it, and nothing is offered twice.
#[test]
fn typing_left_at_close_is_offered_back_and_restore_keeps_it() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Typed, never saved", "journalctl -fu yantrik-ui -p err");
    f.app.close_requested(WindowId::PRIMARY);
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal", "not saved behind the person's back");

    let f = f.restart();
    assert_eq!(f.app.model(|m| m.selected), journal);
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Typed, never saved");
    let (said, state) = bar(&f);
    assert_eq!(state, 1);
    assert!(said.contains("Snippets closed before your changes to \u{201c}Typed, never saved\u{201d} were kept"), "{said}");
    assert_eq!(f.state()["recovery"], json!([{ "id": journal, "title": "Typed, never saved", "gone": false }]));
    // Moving away puts nothing back: the text stays offered, and comes back with its snippet.
    f.app.run(Command::Select(borrow));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal");
    assert_eq!(bar(&f).1, 2, "Show, now that its snippet is not open");
    f.app.ui(|ui| ui.invoke_recovery_action("show".into()));
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "journalctl -fu yantrik-ui -p err");
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().code, "journalctl -fu yantrik-ui -p err");
    assert_eq!(bar(&f).0, "", "the bar goes once the person has chosen");
    f.app.run_writes();
    assert!(!draft_on_disk(&f, journal), "the draft goes once the text is on disk");
    let f = f.restart();
    assert_eq!(bar(&f).0, "", "nothing is offered twice");
    assert!(!draft_on_disk(&f, journal));
}

/// The coordinator's rule for a refused write: the view goes back to what is on disk, the save is
/// said to have failed, the typed text is kept, and a restart offers it back.
#[test]
fn a_refused_write_keeps_the_typed_text_and_offers_it_after_a_restart() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Typed into a full disk", "journalctl --disk-usage");
    f.disk.refuse(true);
    f.app.ui(move |ui| ui.invoke_snip_select(borrow));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal", "the file as it was");
    assert!(f.notice().starts_with("Not saved: "), "{}", f.notice());
    assert!(f.notice().contains("Your text is still in the editor"), "{}", f.notice());
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Typed into a full disk");
    f.app.close_requested(WindowId::PRIMARY);
    let f = f.restart();
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Typed into a full disk");
    assert_eq!(bar(&f).1, 1);
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().code, "journalctl --disk-usage");
}

/// Text whose save was refused after the editor had moved on is not lost with the read-back: it
/// is offered on the bar at once, and says the save failed.
#[test]
fn a_refused_save_of_text_the_editor_has_left_is_offered_at_once() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Left behind", "journalctl -n 5");
    // The move keeps it: the write is queued, not yet run.
    f.app.ui(move |ui| ui.invoke_snip_select(borrow));
    f.type_into("Typing on the next one", "let x = 1;");
    f.disk.refuse(true);
    f.quiet();
    assert_eq!(f.app.model(|m| m.selected), borrow, "the new typing stays where it is");
    let (said, state) = bar(&f);
    assert_eq!(state, 2);
    assert!(said.contains("could not be saved"), "{said}");
    assert!(f.notice().ends_with("Your text is kept as a draft; the bar offers it back."), "{}", f.notice());
    f.app.run_writes();
    assert!(draft_on_disk(&f, journal), "kept in its draft");
    f.disk.refuse(false);
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Left behind");
    assert_eq!(f.on_disk().get(borrow).unwrap().code, "let x = 1;", "the other typing went in first");
}

/// Typing for a snippet the file no longer holds is offered as Restore as new; no snippet is
/// made until the person presses it.
#[test]
fn typing_for_a_snippet_no_longer_kept_is_offered_and_made_only_on_restore() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Orphaned typing", "echo orphan");
    f.app.close_requested(WindowId::PRIMARY);
    let paths = f.paths.clone();
    let _ = f.app.shutdown();
    drop(f);
    // Between the runs the snippet goes from the file.
    let path = file::state_path(paths.data_dir());
    let mut library = file::read_back(&path).unwrap();
    library.delete(journal).unwrap();
    file::write(&path, &file::encode(&library).unwrap()).unwrap();

    let f = start_at(paths, FakeMind::new(), Vec::new());
    f.quiet();
    assert_eq!(f.on_disk().snippets().len(), 1, "nothing made without a press");
    let (said, state) = bar(&f);
    assert_eq!(state, 3);
    assert!(said.contains("that snippet is no longer kept"), "{said}");
    assert_eq!(f.state()["recovery"][0]["gone"], json!(true));
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    let kept: Vec<(String, String)> = f.on_disk().snippets().iter().map(|s| (s.title.clone(), s.code.clone())).collect();
    assert!(kept.contains(&("Orphaned typing".to_string(), "echo orphan".to_string())), "{kept:?}");
    assert_eq!(f.state()["open"], json!("Orphaned typing"));
    assert!(f.on_disk().get(borrow).is_some());
    let f = f.restart();
    assert_eq!(bar(&f).0, "");
}

/// Discard throws the offered text away: the snippet stays as kept, the draft goes, and a restart
/// offers nothing.
#[test]
fn discard_drops_the_offered_text_and_its_draft() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Thrown away", "rm -rf build");
    f.app.close_requested(WindowId::PRIMARY);
    let f = f.restart();
    assert_eq!(bar(&f).1, 1);
    f.app.ui(|ui| ui.invoke_recovery_action("discard".into()));
    f.quiet();
    assert_eq!(bar(&f).0, "");
    assert_eq!(f.app.ui(|ui| ui.get_detail_title().to_string()), "Tail the journal", "as it is kept");
    let f = f.restart();
    assert_eq!(bar(&f).0, "");
    assert!(!draft_on_disk(&f, journal));
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal");
}

/// A mind deleting the snippet the person is typing in does not take the typing with it: it is
/// offered as Restore as new. The person's own delete throws the snippet away with its typing.
#[test]
fn a_minds_delete_under_the_typing_offers_it_and_the_persons_own_delete_does_not() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Typed as a mind deleted it", "journalctl -b -1");
    f.app.run_writes();
    f.act("delete", json!({ "id": journal.to_string() })).unwrap();
    assert_eq!(bar(&f).1, 3, "offered as new");
    f.app.run(Command::Select(borrow));
    f.type_into("Typed then deleted by the person", "let y = 2;");
    f.app.ui(move |ui| ui.invoke_snip_delete(borrow));
    f.quiet();
    assert_eq!(f.app.model(|m| m.offers.len()), 1, "the person's delete offers nothing");
    f.app.run_writes();
    assert!(!draft_on_disk(&f, borrow));
    assert!(draft_on_disk(&f, journal));
}
