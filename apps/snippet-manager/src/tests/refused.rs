//! A refused or unfinished write hands the typing it was keeping to the draft lane, and nothing
//! lets that text go or offers it against the wrong snippet (the #938 review at 647b9161, M1–M3
//! and L1, each its probe):
//!
//! - the newest typing wins over an older refused commit of the same snippet, in the editor and
//!   in the draft;
//! - a write whose outcome is unknown (a panic, the pool refusing the job) keeps the draft;
//! - a refused create never makes its id be given again, so a draft is never offered against a
//!   different snippet.

use serde_json::json;
use yantrik_app::app::WindowId;

use super::{open, Fixture};
use crate::model::draft_name;
use crate::Command;

fn draft_file(f: &Fixture, id: i32) -> Option<String> {
    std::fs::read_to_string(yantrik_app::drafts::path_in(f.paths.state_dir(), &draft_name(id))).ok()
}

fn drafted(f: &Fixture, id: i32) -> Option<String> {
    f.app.model(move |m| m.drafted(id).map(|d| d.code.clone()))
}

/// M1: two commits of one snippet queued while the disk is full. The older one's refusal must not
/// put its text back over the newer typing, in the editor or in the draft.
#[test]
fn the_newest_typing_wins_over_an_older_refused_commit() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.disk.refuse(true);
    f.type_into("J", "codeA");
    f.app.run(Command::Select(borrow));
    f.app.run(Command::Select(journal));
    f.type_into("J", "codeAB");
    f.app.run(Command::Select(borrow));
    assert_eq!(f.app.jobs_waiting(), 2, "two commits of the snippet are queued");
    f.quiet();
    f.app.run_writes();
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "codeAB", "the newest typing is in the editor");
    assert_eq!(drafted(&f, journal).as_deref(), Some("codeAB"));
    assert!(draft_file(&f, journal).unwrap().contains("codeAB"), "and in the draft on disk");
    assert!(f.notice().ends_with("Your text is still in the editor, and kept as a draft until it is saved."), "{}", f.notice());
    f.disk.refuse(false);
    f.app.run(Command::Select(borrow));
    f.quiet();
    assert_eq!(f.on_disk().get(journal).unwrap().code, "codeAB");
}

/// M2: a write job that ends without a result says nothing about the file. The typing it was
/// keeping stays in its draft until a write has positively landed, and is offered back.
#[test]
fn a_write_that_did_not_finish_keeps_the_typing_it_was_keeping() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Typed before a panic", "journalctl -fu yantrik-ui --since now");
    f.app.run_writes();
    f.disk.panic_next();
    f.app.run(Command::Select(borrow));
    f.quiet();
    f.app.run_writes();
    assert_eq!(f.on_disk().get(journal).unwrap().title, "Tail the journal", "the file never took it");
    assert_eq!(drafted(&f, journal).as_deref(), Some("journalctl -fu yantrik-ui --since now"));
    assert!(draft_file(&f, journal).unwrap().contains("--since now"), "the draft on disk was kept");
    assert_eq!(f.app.model(|m| m.offers.iter().map(|o| o.id).collect::<Vec<_>>()), [journal], "offered back");
    // And across a restart.
    let f = f.restart();
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "journalctl -fu yantrik-ui --since now");
}

/// M3 and L1: a refused *Restore as new* must not let its id be given to the next snippet, in this
/// run or the next; the bar keeps saying the text's snippet is no longer kept, and *Restore* makes
/// a new snippet rather than overwriting someone else's.
#[test]
fn a_refused_create_never_lets_its_id_be_given_again() {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Orphan", "orphan-text");
    f.act("delete", json!({ "id": journal.to_string() })).unwrap();
    assert_eq!(f.app.ui(|ui| ui.get_recovery()), 3, "offered as new");
    f.disk.refuse(true);
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    assert!(f.notice().starts_with("Not saved: ") && f.notice().contains("kept as a draft"), "L1: {}", f.notice());
    let offered = f.app.model(|m| m.offers[0].id);
    f.disk.refuse(false);
    let made = f.act("new", json!({ "title": "Mind's", "language": "Shell", "code": "mind-code" })).unwrap();
    assert_ne!(made["id"].as_i64().unwrap() as i32, offered, "the refused create's id is not given again");
    assert_eq!(f.app.ui(|ui| ui.get_recovery()), 3, "still no longer kept");
    assert_eq!(f.state()["recovery"][0]["gone"], json!(true));

    // The same across a restart that never wrote the higher counter.
    f.app.close_requested(WindowId::PRIMARY);
    let f = f.restart();
    let made = f.act("new", json!({ "title": "After", "language": "Shell", "code": "after" })).unwrap();
    assert_ne!(made["id"].as_i64().unwrap() as i32, offered);
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    let codes: Vec<String> = f.on_disk().snippets().iter().map(|s| s.code.clone()).collect();
    for kept in ["mind-code", "after", "orphan-text", "let mut guard = state.borrow_mut();"] {
        assert!(codes.contains(&kept.to_string()), "{kept} in {codes:?}");
    }
    assert!(f.on_disk().get(borrow).is_some());
}
