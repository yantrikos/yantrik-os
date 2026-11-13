//! Each promise keeps its own prepared draft (#1184, gate 8): a newer promise's draft does not
//! replace an earlier promise's while that card still says it is in Email.

use serde_json::json;
use yantrik_app::{App, Paths};

use yantrik_app::mind::fake::FakeMind;

use super::{act, quiet, start_at, state, with_inbox};
use crate::command::Command;
use crate::files;
use crate::Email;

fn started(paths: &Paths) -> yantrik_testing::TestApp<Email> {
    let (app, _) = start_at(paths.clone(), &with_inbox(), FakeMind::new());
    quiet(&app);
    app
}

fn home_file(name: &str, tag: &str) -> std::path::PathBuf {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let dir = home.join(format!(".yantrik-email-shelf-test-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, b"x").unwrap();
    file
}

fn compose(app: &yantrik_testing::TestApp<Email>, to: &str, file: &std::path::Path, commitment: &str) {
    compose_with(app, to, &file.to_string_lossy(), commitment);
}

fn compose_with(app: &yantrik_testing::TestApp<Email>, to: &str, attach: &str, commitment: &str) {
    let args = json!({ "to": to, "subject": "report", "body": "Here.", "attach": attach, "commitment": commitment });
    act(app, "compose", args).unwrap();
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(app);
}

fn shown_to(app: &yantrik_testing::TestApp<Email>) -> String {
    app.ui(|ui| ui.get_compose_to().to_string())
}

#[test]
fn two_promises_each_keep_their_draft_and_each_opens_its_own() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "two");
    let b = home_file("beta.ydeck", "two");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &b, "25");
    let slot = paths.data_dir().join(files::MIND_DRAFT);
    assert!(slot.exists(), "the newest is in the slot");
    assert!(paths.data_dir().join("mind_draft.24.json").exists(), "the earlier promise's draft is kept");

    app.run(Command::OpenPreparedFor("24".into()));
    quiet(&app);
    assert_eq!(shown_to(&app), "maya@example.com");
    assert_eq!(state(&app)["draft_attachments"], json!([a.canonicalize().unwrap().to_string_lossy()]));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(&app);
    assert_eq!(shown_to(&app), "ravi@example.com");
    assert_eq!(state(&app)["draft_attachments"], json!([b.canonicalize().unwrap().to_string_lossy()]));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn forgetting_one_promise_leaves_the_other_promises_draft() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "forget");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &a, "25");
    let shelf = paths.data_dir().join("mind_draft.24.json");
    act(&app, "forget_prepared", json!({ "commitment": "25" })).unwrap();
    assert!(shelf.exists() && !paths.data_dir().join(files::MIND_DRAFT).exists(), "only 25 went");
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    assert!(!shelf.exists(), "the waiting draft clears when its promise is forgotten");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn a_second_draft_for_the_same_promise_replaces_the_first() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "same");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "maya2@example.com", &a, "24");
    assert!(!paths.data_dir().join("mind_draft.24.json").exists());
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn an_id_that_could_name_a_path_is_not_shelved() {
    let slot = std::path::Path::new("/d/mind_draft.json");
    assert_eq!(crate::state::shelf_path(slot, "../x"), None);
    assert_eq!(crate::state::shelf_path(slot, ""), None);
    assert_eq!(crate::state::shelf_path(slot, "24").unwrap(), std::path::Path::new("/d/mind_draft.24.json"));
}

fn slot_file(paths: &Paths) -> std::path::PathBuf {
    paths.data_dir().join(files::MIND_DRAFT)
}

fn shelf_text(paths: &Paths, id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(paths.data_dir().join(format!("mind_draft.{id}.json"))).ok()?;
    Some(raw)
}

/// Two promises prepared, the person edits A's draft and then opens B's: A's edit waits, touched,
/// on A's shelf, and B holds the slot.
fn a_touched_and_b_in_the_slot(paths: &Paths, app: &yantrik_testing::TestApp<Email>, file: &std::path::Path) {
    touched_a(paths, app, &file.to_string_lossy());
}

/// The same, with `attach` on both drafts: empty means drafts that carry no file.
fn touched_a(paths: &Paths, app: &yantrik_testing::TestApp<Email>, attach: &str) {
    compose_with(app, "maya@example.com", attach, "24");
    compose_with(app, "ravi@example.com", attach, "25");
    app.run(Command::OpenPreparedFor("24".into()));
    quiet(app);
    super::compose::type_into_composer(app, &super::compose::draft("maya@example.com", "my own careful wording"));
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(app);
    assert!(shelf_text(paths, "24").unwrap().contains("my own careful wording"), "the edit is on A's shelf");
    assert_eq!(shown_to(app), "ravi@example.com");
}

#[test]
fn m1_a_compose_for_a_promise_whose_shelf_the_person_edited_is_refused_and_changes_nothing() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "m1");
    a_touched_and_b_in_the_slot(&paths, &app, &a);
    let before = (shelf_text(&paths, "24"), std::fs::read_to_string(slot_file(&paths)).unwrap());
    let args = json!({ "to": "other@example.com", "subject": "x", "body": "A Mind's words", "attach": a.to_string_lossy(), "commitment": "24" });
    let refused = act(&app, "compose", args).unwrap_err();
    assert!(refused.to_string().starts_with("You have an unsent message of your own"), "{refused}");
    assert_eq!((shelf_text(&paths, "24"), std::fs::read_to_string(slot_file(&paths)).unwrap()), before, "nothing was written or removed");
    // The edit is still reachable through A's own entry.
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPreparedFor("24".into()));
    quiet(&app);
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// The slot's file is made a folder, so the write that would replace it fails the way a full or
/// read-only disk does (a plain read-only folder does not stop root).
fn break_the_slot_write(paths: &Paths) {
    let slot = slot_file(paths);
    std::fs::remove_file(&slot).unwrap();
    std::fs::create_dir(&slot).unwrap();
}

#[test]
fn l1_a_compose_whose_write_fails_leaves_the_shelf_it_would_replace() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "l1c");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &a, "25");
    let shelf = shelf_text(&paths, "24").expect("24 waits");
    break_the_slot_write(&paths);
    let args = json!({ "to": "maya2@example.com", "subject": "x", "body": "new", "attach": a.to_string_lossy(), "commitment": "24" });
    let refused = act(&app, "compose", args).unwrap_err();
    assert!(refused.to_string().contains("could not be kept"), "the person is told: {refused}");
    assert_eq!(shelf_text(&paths, "24"), Some(shelf), "the older draft was not removed before the write");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn l1_opening_a_draft_whose_slot_write_fails_loses_neither_draft_and_says_so() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "l1o");
    compose(&app, "maya@example.com", &a, "24");
    compose(&app, "ravi@example.com", &a, "25");
    break_the_slot_write(&paths);
    app.run(Command::OpenPreparedFor("24".into()));
    quiet(&app);
    assert!(super::notice(&app).contains("could not be opened"), "{}", super::notice(&app));
    assert!(shelf_text(&paths, "24").unwrap().contains("maya@example.com"), "the draft being opened is still on its shelf");
    // The write failed, so the slot still holds the draft that was in it; no second copy is left on a shelf.
    assert!(shelf_text(&paths, "25").is_none(), "no second copy of the slot's draft is left");
    assert!(app.model(|m| matches!(&m.composer.mind, Ok(Some(d)) if d.text.to.contains("ravi@example.com"))), "the draft that was in the slot is still there");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn l2_a_touched_draft_with_no_file_becomes_the_persons_own_draft_when_its_promise_is_forgotten() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    touched_a(&paths, &app, "");
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    assert!(shelf_text(&paths, "24").is_none(), "the shelf is cleared: its text moved");
    app.ui(|ui| ui.invoke_compose_new());
    quiet(&app);
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording");
}

#[test]
fn n1_a_touched_draft_with_a_file_never_loses_it_when_its_promise_is_forgotten() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "n1");
    a_touched_and_b_in_the_slot(&paths, &app, &a);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    let own = std::fs::read_to_string(paths.data_dir().join("draft.json")).unwrap_or_default();
    assert!(!own.contains("my own careful wording"), "the text did not go to draft.json without its file: {own}");
    app.ui(|ui| ui.invoke_compose_new());
    quiet(&app);
    assert_ne!(app.model(|m| m.composer.text.body.clone()), "my own careful wording", "Compose does not reopen it");
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    // B goes; A, text and file, comes into the entry, where the attachment keeps it unsendable.
    app.ui(|ui| ui.invoke_discard_prepared());
    quiet(&app);
    app.ui(|ui| ui.invoke_open_prepared());
    quiet(&app);
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording");
    assert_eq!(state(&app)["draft_attachments"], json!([a.canonicalize().unwrap().to_string_lossy()]));
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn l2_with_their_own_draft_in_the_way_the_touched_one_waits_for_the_prepared_entry() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "l2b");
    a_touched_and_b_in_the_slot(&paths, &app, &a);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    // The person has a draft of their own already.
    app.ui(|ui| ui.invoke_compose_new());
    super::compose::type_into_composer(&app, &super::compose::draft("bob@example.com", "my own draft"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    assert!(shelf_text(&paths, "24").is_some(), "their draft is not overwritten; the shelf stays");
    // The slot's draft goes; the touched shelf comes into the entry.
    app.ui(|ui| ui.invoke_discard_prepared());
    quiet(&app);
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).contains("maya@example.com"));
    app.ui(|ui| ui.invoke_open_prepared());
    quiet(&app);
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// A touched, with their own draft in the way (the l2b situation), then B's slot emptied.
fn l2b(paths: &Paths, app: &yantrik_testing::TestApp<Email>, attach: &str) {
    touched_a(paths, app, attach);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(app);
    app.ui(|ui| ui.invoke_compose_new());
    super::compose::type_into_composer(app, &super::compose::draft("bob@example.com", "my own draft"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(app);
    act(app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    assert!(shelf_text(paths, "24").is_some(), "A waits on its shelf");
}

fn entry_shows_a(app: &yantrik_testing::TestApp<Email>) {
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).contains("maya@example.com"));
    app.ui(|ui| ui.invoke_open_prepared());
    quiet(app);
    assert_eq!(app.model(|m| m.composer.text.body.clone()), "my own careful wording");
}

#[test]
fn l3_removing_the_attachment_from_the_slot_brings_the_waiting_draft_into_the_entry() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "l3r");
    touched_a(&paths, &app, &a.to_string_lossy());
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPrepared);
    quiet(&app);
    app.run(Command::RemoveAttachment);
    quiet(&app);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    entry_shows_a(&app);
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn l3_sending_the_slot_draft_brings_the_waiting_draft_into_the_entry() {
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &with_inbox().answering("email.send_message", json!({})), FakeMind::new());
    quiet(&app);
    touched_a(&paths, &app, "");
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPrepared);
    quiet(&app);
    let body = app.model(|m| m.composer.text.body.clone());
    app.ui(|ui| ui.invoke_send_email("ravi@example.com".into(), "".into(), "".into(), "report".into(), body.into()));
    quiet(&app);
    entry_shows_a(&app);
}

#[test]
fn l3_a_restart_with_an_empty_slot_brings_the_waiting_draft_into_the_entry() {
    let paths = Paths::temporary(Email::ID);
    {
        let app = started(&paths);
        l2b(&paths, &app, "");
        // The slot went while nothing promoted (a process that ended first): only A's shelf is left.
        std::fs::remove_file(slot_file(&paths)).unwrap();
    }
    let app = started(&paths);
    entry_shows_a(&app);
}

#[test]
fn leftover_1_a_touched_slot_draft_whose_promise_is_forgotten_stays_in_the_slot() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "lo1");
    a_touched_and_b_in_the_slot(&paths, &app, &a);
    // Back to A, edit it, park it: A is touched and in the slot, B on its shelf.
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    app.run(Command::OpenPreparedFor("24".into()));
    quiet(&app);
    super::compose::type_into_composer(&app, &super::compose::draft("maya@example.com", "edited again"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(&app);
    assert!(!paths.data_dir().join("mind_draft.24.json").exists(), "it was not shelved under a dead id");
    assert!(std::fs::read_to_string(slot_file(&paths)).unwrap().contains("edited again"), "it stayed in the slot");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

/// 24 is edited and parked in the slot with 25 on its shelf; the slot write is broken and 25 opened,
/// which fails. Returns the slot file's text, to put back.
fn failed_open_with_a_touched_slot(paths: &Paths, app: &yantrik_testing::TestApp<Email>, attach: &str) -> String {
    compose_with(app, "maya@example.com", attach, "24");
    compose_with(app, "ravi@example.com", attach, "25");
    app.run(Command::OpenPreparedFor("24".into()));
    quiet(app);
    super::compose::type_into_composer(app, &super::compose::draft("maya@example.com", "edited in the slot"));
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(app);
    let held = std::fs::read_to_string(slot_file(paths)).unwrap();
    break_the_slot_write(paths);
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(app);
    assert!(super::notice(app).contains("could not be opened"), "{}", super::notice(app));
    std::fs::remove_dir(slot_file(paths)).unwrap();
    std::fs::write(slot_file(paths), &held).unwrap();
    held
}

fn nothing_of_24_comes_back(paths: &Paths, app: &yantrik_testing::TestApp<Email>) {
    assert!(!paths.data_dir().join("mind_draft.24.json").exists(), "no copy of the discarded draft is left on a shelf");
    assert!(app.model(|m| matches!(m.composer.mind, Ok(None))), "the entry is empty");
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).is_empty(), "the entry offers nothing");
}

#[test]
fn r1_a_failed_open_then_discard_prepared_does_not_bring_the_discarded_draft_back() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "r1d");
    failed_open_with_a_touched_slot(&paths, &app, &a.to_string_lossy());
    assert!(shelf_text(&paths, "24").is_none(), "the failed open left no copy of the held draft");
    app.ui(|ui| ui.invoke_discard_prepared());
    quiet(&app);
    nothing_of_24_comes_back(&paths, &app);
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn r1_a_failed_open_then_a_send_does_not_offer_the_sent_message_again() {
    let paths = Paths::temporary(Email::ID);
    let (app, _) = start_at(paths.clone(), &with_inbox().answering("email.send_message", json!({})), FakeMind::new());
    quiet(&app);
    let held = failed_open_with_a_touched_slot(&paths, &app, "");
    assert!(held.contains("edited in the slot"));
    app.run(Command::OpenPrepared);
    quiet(&app);
    let body = app.model(|m| m.composer.text.body.clone());
    app.ui(|ui| ui.invoke_send_email("maya@example.com".into(), "".into(), "".into(), "Plans".into(), body.into()));
    quiet(&app);
    nothing_of_24_comes_back(&paths, &app);
}

#[test]
fn r1_a_failed_open_then_remove_attachment_does_not_bring_the_draft_back_with_its_file() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "r1r");
    failed_open_with_a_touched_slot(&paths, &app, &a.to_string_lossy());
    app.run(Command::OpenPrepared);
    quiet(&app);
    app.run(Command::RemoveAttachment);
    quiet(&app);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    nothing_of_24_comes_back(&paths, &app);
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}

#[test]
fn r2_a_promoted_draft_whose_promise_was_forgotten_is_not_shelved_under_the_dead_id() {
    let paths = Paths::temporary(Email::ID);
    let app = started(&paths);
    let a = home_file("alpha.ydeck", "r2");
    a_touched_and_b_in_the_slot(&paths, &app, &a);
    app.ui(|ui| ui.invoke_cancel_compose());
    quiet(&app);
    compose(&app, "cara@example.com", &a, "26");
    act(&app, "forget_prepared", json!({ "commitment": "24" })).unwrap();
    app.ui(|ui| ui.invoke_discard_prepared());
    quiet(&app);
    assert!(app.ui(|ui| ui.get_prepared_label().to_string()).contains("maya@example.com"), "A is in the entry");
    app.run(Command::OpenPreparedFor("25".into()));
    quiet(&app);
    assert!(!paths.data_dir().join("mind_draft.24.json").exists(), "A was not shelved under the dead id");
    assert!(super::notice(&app).contains("was not moved"), "{}", super::notice(&app));
    assert!(std::fs::read_to_string(slot_file(&paths)).unwrap().contains("my own careful wording"), "A stayed in the slot");
    let _ = std::fs::remove_dir_all(a.parent().unwrap());
}
