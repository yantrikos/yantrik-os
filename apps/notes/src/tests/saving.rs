//! Saves as jobs through the open note's document: one in flight, the edits made meanwhile as one
//! follow-up, no retry by the autosave after a failure, never over a file changed outside, the
//! file watched while the note is open. Closing is closing.rs's.

use serde_json::json;
use yantrik_app::documents::disk::Step;
use yantrik_app::documents::OutsideState;

use super::{
    act, autosave, bar, failing, file, not_saved_over, note, notice, outside, settle, state, unsaved, with_note,
};
use crate::command::{Command, WindowAction};
use crate::update::{said, AUTOSAVE_AFTER};

#[test]
fn two_quick_edits_during_a_save_produce_exactly_one_follow_up() {
    let (app, path) = with_note("Draft");
    app.run(Command::Edit("# Draft\n\none".into()));
    app.advance(AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1, "the autosave's save is in flight");
    app.run(Command::Edit("# Draft\n\none two".into()));
    app.advance(AUTOSAVE_AFTER);
    app.run(Command::Edit("# Draft\n\none two three".into()));
    app.advance(AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1, "one save in flight, never a pile");
    assert!(app.run_one_job());
    assert_eq!(file(&path), "# Draft\n\none", "the save in flight wrote what it was given");
    assert_eq!(app.jobs_waiting(), 1, "exactly one follow-up for both edits");
    assert!(app.run_one_job());
    assert_eq!(app.jobs_waiting(), 0, "and nothing after it");
    assert_eq!(file(&path), "# Draft\n\none two three", "the follow-up wrote the latest edit");
    assert!(!unsaved(&app));
    assert_eq!(app.model(|m| m.pane.status.clone()), "All changes saved");
}

#[test]
fn a_failed_save_stays_unsaved_and_the_autosave_does_not_retry_it_until_something_changes() {
    let (app, path) = with_note("Read only");
    let disk = failing(&app);
    disk.fail(Step::Write, ".md", None);
    app.run(Command::Edit("# Read only\n\nmine".into()));
    autosave(&app);
    assert!(unsaved(&app), "a failed save keeps the edits as edits");
    assert!(notice(&app).contains("injected failure at Write"), "{}", notice(&app));
    assert_eq!(app.model(|m| m.pane.status.clone()), "Needs attention · draft kept");
    let tried = disk.hits.lock().unwrap().len();
    // Ticks, and the autosave itself, find the same content failed: nothing is tried again.
    app.advance(AUTOSAVE_AFTER * 6);
    app.run(Command::Autosave);
    settle(&app);
    assert_eq!(disk.hits.lock().unwrap().len(), tried, "no retry, so no second notice");
    // An edit tries again.
    app.run(Command::Edit("# Read only\n\nmine, again".into()));
    autosave(&app);
    assert_eq!(disk.hits.lock().unwrap().len(), tried + 1);
    // So does the person's Save, once the disk is back, and that one lands.
    disk.heal();
    app.run(Command::Window(WindowAction::Save));
    settle(&app);
    assert!(!unsaved(&app));
    assert_eq!(file(&path), "# Read only\n\nmine, again");
    assert_eq!(notice(&app), "");
}

/// Gate row "Conflicting outside edit": with the watch disabled (it never looks here), the store's
/// own check still refuses, and the person is offered the same choices.
#[test]
fn the_store_refuses_to_save_over_a_file_changed_outside() {
    let (app, path) = with_note("Shared");
    std::fs::write(&path, "theirs").unwrap();
    app.run(Command::Edit("# Shared\n\nmine".into()));
    autosave(&app);
    assert_eq!(file(&path), "theirs", "never last-writer-wins");
    assert!(unsaved(&app), "my text is kept as edits");
    assert_eq!(notice(&app), not_saved_over());
    assert_eq!(bar(&app), 1, "and the person is offered the choices");
    // Once the store has refused, the document knows: every later save is refused at once, in
    // the same words, with no job.
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    app.run(Command::Window(WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 0);
    assert_eq!(file(&path), "theirs");
    let e = act(&app, "save", json!({})).unwrap_err();
    assert_eq!(e, not_saved_over());
    // The way out is still a copy.
    app.run(Command::Window(WindowAction::Copy));
    settle(&app);
    assert!(!unsaved(&app));
    assert_eq!(file(&path), "theirs");
}

/// Gate row "Conflicting outside edit": the change is detected before the next save.
#[test]
fn a_change_the_watch_sees_first_is_offered_at_once_and_no_save_is_tried_over_it() {
    let (app, path) = with_note("Watched");
    app.run(Command::Edit("# Watched\n\nmine".into()));
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    assert_eq!(bar(&app), 1, "the choices are offered as soon as the watch sees it");
    assert_eq!(state(&app)["outside"], "conflict");
    autosave(&app);
    assert_eq!(app.jobs_waiting(), 0, "a save over it is not even tried");
    assert_eq!(notice(&app), "", "the autosave adds nothing to what the bar says");
    assert_eq!(app.model(|m| m.pane.status.clone()), said::WAITING_ON_CHOICE);
    app.run(Command::Window(WindowAction::Save));
    assert_eq!(app.jobs_waiting(), 0);
    assert_eq!(notice(&app), not_saved_over(), "the person's own Save is refused in words");
    assert_eq!(file(&path), "theirs");
}

/// Gate row "Conflicting outside edit": our own save does not raise a change.
#[test]
fn our_own_save_is_not_an_outside_edit_and_the_watch_follows_the_open_note() {
    let (app, path) = with_note("Mine");
    assert_eq!(app.watched().iter().map(|(_, p)| p.clone()).collect::<Vec<_>>(), vec![path.clone()]);
    app.run(Command::Edit("# Mine\n\nsaved by us".into()));
    autosave(&app);
    app.look_at_files();
    app.look_at_files();
    assert!(matches!(outside(&app), OutsideState::Clean), "{:?}", outside(&app));
    assert_eq!((bar(&app), notice(&app)), (0, String::new()), "nothing is offered, nothing said");
    assert_eq!(state(&app)["outside"], "clean");
    app.run(Command::Edit("# Mine\n\nsaved by us, twice".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Mine\n\nsaved by us, twice");
    // Another note open: its file is watched, the first one's no longer.
    let other = note(&app, "Other", "");
    assert_eq!(app.watched().len(), 1);
    assert_eq!(app.watched()[0].1.display().to_string(), other["path"].as_str().unwrap());
    app.run(Command::Window(WindowAction::Trash));
    settle(&app);
    assert!(app.watched().is_empty(), "nothing open, nothing watched");
}
