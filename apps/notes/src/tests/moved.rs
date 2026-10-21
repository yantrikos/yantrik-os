//! A note's text that moves to another note takes its draft with it (the 520 run of #918, defect
//! 1). *Keep mine as a copy*, *Save as new*, *Take theirs* (which keeps mine as a note), *Save a
//! copy* and text typed while a note goes to the Trash each leave the text in a note of its own;
//! the old note's draft must not outlive that, or the next start offers the same text again and
//! *Restore* makes it twice. Refresh, an import and a restore from the Trash move no text, and
//! leave nothing behind either. Each path ends in a crash here, and the next start offers nothing.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::documents::disk::Step;
use yantrik_testing::TestApp;

use super::{
    act, draft_file, failing, file, killed_and_started_again, notice, settle, state, unsaved, vault, with_note,
};
use crate::command::{Command, WindowAction};
use crate::update::said;
use crate::Notes;

const MINE: &str = "# Shared\n\nmine";
const THEIRS: &str = "# Shared\n\ntheirs\n";

fn press(app: &TestApp<Notes>, id: &'static str) {
    app.ui(move |ui| ui.invoke_action(id.into()));
    settle(app);
}

/// A saved note, open, edited with its draft written, and changed by another program meanwhile.
fn in_conflict() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Shared");
    app.run(Command::Edit(MINE.into()));
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "the draft is kept while the text is unsaved");
    std::fs::write(&path, THEIRS).unwrap();
    app.look_at_files();
    assert_eq!(super::bar(&app), 1);
    (app, path)
}

/// A saved note, open, edited with its draft written, and its file then deleted outside Notes.
fn file_gone() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Gone");
    app.run(Command::Edit("# Gone\n\nkeep me".into()));
    app.run_writes();
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert_eq!(super::bar(&app), 2);
    (app, path)
}

/// What the next start offers back, by title.
fn offered(app: &TestApp<Notes>) -> Vec<String> {
    state(app)["recovery"].as_array().unwrap().iter().map(|o| o["title"].as_str().unwrap().to_string()).collect()
}

/// The lane does what it was handed (it would, within moments), then Notes is killed and started
/// again: it must offer nothing, since every text is in a note of its own.
fn crashed_offers_nothing(app: TestApp<Notes>, path: &str) {
    app.run_writes();
    let again = killed_and_started_again(app);
    assert_eq!(offered(&again), Vec::<String>::new(), "{path}: nothing is offered twice");
    assert_eq!(notice(&again), "", "{path}");
}

/// The note in the library whose file name starts with `prefix`, and its text on disk.
fn kept(app: &TestApp<Notes>, prefix: &'static str) -> String {
    let id = app.model(move |m| m.library.iter().find(|n| n.id.starts_with(prefix)).map(|n| n.id.clone()));
    file(&vault(app).join(id.unwrap_or_else(|| panic!("a {prefix} note"))))
}

#[test]
fn keep_mine_as_a_copy_leaves_no_draft_of_the_original() {
    let (app, path) = in_conflict();
    press(&app, "keep-mine");
    assert_eq!(kept(&app, "copy-"), MINE);
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the original's draft went with its text");
    crashed_offers_nothing(app, "keep mine");
}

#[test]
fn save_as_new_leaves_no_draft_of_the_gone_note() {
    let (app, path) = file_gone();
    press(&app, "keep-mine");
    assert_eq!(kept(&app, "copy-"), "# Gone\n\nkeep me");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists());
    crashed_offers_nothing(app, "save as new");
}

#[test]
fn take_theirs_keeping_mine_as_a_note_leaves_no_draft() {
    let (app, path) = in_conflict();
    press(&app, "take-theirs");
    assert_eq!(kept(&app, "recovered-"), MINE);
    assert_eq!(file(&path), THEIRS);
    crashed_offers_nothing(app, "take theirs");
}

/// With the watch off, the store's check finds the file gone under a save: Take theirs has nothing
/// to take and moves the window to my text as a recovered note, as Keep mine would.
#[test]
fn take_theirs_on_a_file_found_gone_leaves_no_draft_of_the_gone_note() {
    let (app, path) = with_note("Gone");
    app.run(Command::Edit("# Gone\n\nkeep me".into()));
    app.run_writes();
    std::fs::remove_file(&path).unwrap();
    super::autosave(&app);
    press(&app, "take-theirs");
    assert_eq!(notice(&app), said::NOTHING_TO_TAKE);
    assert_eq!(kept(&app, "recovered-"), "# Gone\n\nkeep me");
    crashed_offers_nothing(app, "take theirs, nothing to take");
}

#[test]
fn save_a_copy_leaves_no_draft_of_the_original() {
    let (app, path) = in_conflict();
    press(&app, "copy");
    assert_eq!(kept(&app, "copy-"), MINE);
    assert!(!unsaved(&app), "the copy is saved");
    assert_eq!(file(&path), THEIRS);
    crashed_offers_nothing(app, "save a copy");
}

/// *Save a copy* of a note whose file is gone is *Save as new* by another button: the gone note
/// leaves the library with its draft.
#[test]
fn save_a_copy_of_a_gone_note_leaves_neither_its_draft_nor_its_card() {
    let (app, path) = file_gone();
    let original = path.file_name().unwrap().to_str().unwrap().to_string();
    press(&app, "copy");
    assert_eq!(kept(&app, "copy-"), "# Gone\n\nkeep me");
    assert!(app.model(move |m| !m.library.iter().any(|n| n.id == original)), "the gone note is not listed");
    crashed_offers_nothing(app, "save a copy of a gone note");
}

/// Never none either: a copy that cannot be saved keeps its own draft, written at once in place of
/// the original's, so the text is offered once, as the copy.
#[test]
fn a_copy_that_cannot_be_saved_is_offered_once_as_itself() {
    let (app, path) = in_conflict();
    failing(&app).fail(Step::Write, ".md", None);
    press(&app, "copy");
    assert!(unsaved(&app), "the copy could not be saved");
    let copy = app.model(|m| m.current().unwrap().id().to_string());
    app.run_writes();
    assert!(draft_file(&app, &vault(&app).join(&copy)).exists(), "the copy's draft is written");
    // Its landing (`DraftSaved`) releases the original's, removed by the lane's next write.
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "and then the original's is gone");
    let again = killed_and_started_again(app);
    // The copy's draft is offered, or the store's journal of its interrupted save came back as
    // a recovered note; either way the text is there once, and never under the original.
    let offers = state(&again)["recovery"].as_array().unwrap().clone();
    assert!(offers.iter().all(|o| o["filename"] == json!(copy)), "never as the original: {offers:?}");
    let listed = again.model(|m| m.library.iter().filter(|n| n.content.text == MINE).count());
    assert_eq!(offers.len() + listed, 1, "the text is there once: {offers:?}");
}

#[test]
fn text_typed_while_its_note_went_to_the_trash_is_offered_nothing_twice() {
    let (app, path) = with_note("Binned");
    app.run(Command::Window(WindowAction::Trash));
    assert_eq!(app.jobs_waiting(), 1, "the Trash job is in flight");
    app.run(Command::Edit("# Binned\n\ntyped while it went".into()));
    app.run_writes();
    assert!(draft_file(&app, &path).exists(), "the typing is kept as the note's draft meanwhile");
    settle(&app);
    assert_eq!(notice(&app), said::TYPED_DURING_TRASH);
    assert_eq!(kept(&app, "recovered-"), "# Binned\n\ntyped while it went");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the trashed note's draft went with the text");
    crashed_offers_nothing(app, "trash");
}

#[test]
fn refresh_an_import_and_a_restore_from_the_trash_leave_nothing_to_offer() {
    // Refresh with unsaved text: saved first, then read again.
    let (app, path) = with_note("Refreshed");
    app.run(Command::Edit("# Refreshed\n\nunsaved".into()));
    app.run_writes();
    press(&app, "reload");
    assert_eq!(file(&path), "# Refreshed\n\nunsaved");
    crashed_offers_nothing(app, "refresh");

    // An import with unsaved text: the open note is saved first, then the imported one opened.
    let (app, path) = with_note("Before import");
    app.run(Command::Edit("# Before import\n\nunsaved".into()));
    app.run_writes();
    let outside = super::store::fixture("moved-import").join("in.md");
    std::fs::write(&outside, "# Imported\n\nfrom outside\n").unwrap();
    act(&app, "import", json!({ "path": outside.display().to_string() })).expect("imported");
    assert_eq!(file(&path), "# Before import\n\nunsaved");
    crashed_offers_nothing(app, "import");

    // To the Trash and back.
    let (app, _) = with_note("Round trip");
    app.run(Command::Edit("# Round trip\n\nunsaved".into()));
    app.run_writes();
    let id = app.model(|m| m.current().unwrap().id().to_string());
    act(&app, "trash", json!({})).expect("trashed");
    app.run(Command::Window(WindowAction::Open { id, trash: true }));
    act(&app, "restore", json!({})).expect("restored");
    assert_eq!(state(&app)["title"], "Round trip");
    crashed_offers_nothing(app, "trash and restore");
}

/// Discarding an offer whose note's file is gone does not say the note is as it is on disk.
#[test]
fn discard_of_an_offer_whose_file_is_gone_says_nothing_is_kept() {
    let (app, path) = with_note("Vanished");
    app.run(Command::Edit("# Vanished\n\nonly here".into()));
    app.run_writes();
    let paths = app.paths();
    drop(app);
    std::fs::remove_file(&path).unwrap();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    assert_eq!(offered(&app), vec!["Vanished".to_string()]);
    act(&app, "discard_draft", json!({})).expect("discarded");
    assert_eq!(notice(&app), said::DISCARDED_GONE);
    crashed_offers_nothing(app, "discard");
}

/// The 925 review, finding 3 (P2): Notes stops after the kept note landed but before the lane
/// removed the old draft. Both are on disk; the start offers nothing, since a note already holds
/// the text, and removes the stale draft.
#[test]
fn a_kill_between_the_kept_note_and_the_old_drafts_removal_offers_nothing() {
    let (app, path) = in_conflict();
    press(&app, "keep-mine");
    assert_eq!(kept(&app, "copy-"), MINE, "the copy is on disk");
    assert!(draft_file(&app, &path).exists(), "and the old draft still is: the lane has not run");
    let again = killed_and_started_again(app);
    assert_eq!(offered(&again), Vec::<String>::new(), "the text is a note already: nothing to Restore twice");
    again.run_writes();
    assert!(!draft_file(&again, &path).exists(), "the stale draft is removed");
}

/// The same with two drafts of one text on disk and no note holding it: offered once.
#[test]
fn two_drafts_of_one_text_are_offered_once() {
    let (app, path) = in_conflict();
    failing(&app).fail(Step::Write, ".md", None);
    press(&app, "copy");
    let copy = app.model(|m| m.current().unwrap().id().to_string());
    app.run_writes();
    assert!(draft_file(&app, &path).exists() && draft_file(&app, &vault(&app).join(&copy)).exists(), "both drafts");
    let again = killed_and_started_again(app);
    let offers = state(&again)["recovery"].as_array().unwrap().clone();
    let listed = again.model(|m| m.library.iter().filter(|n| n.content.text == MINE).count());
    assert_eq!(offers.len() + listed, 1, "once: {offers:?}");
}

/// The 925 review, finding 4: the new draft's write fails, so the old draft is kept until the
/// copy is saved; then it goes.
#[test]
fn an_old_draft_stays_until_the_new_one_is_on_disk_or_the_copy_is_saved() {
    let (app, path) = in_conflict();
    let disk = failing(&app);
    disk.fail(Step::Write, ".md", None);
    press(&app, "copy");
    let copy = vault(&app).join(app.model(|m| m.current().unwrap().id().to_string()));
    // A folder where the copy's draft would go: the lane cannot write it.
    std::fs::create_dir_all(draft_file(&app, &copy)).unwrap();
    app.run_writes();
    app.run_writes();
    assert!(notice(&app).starts_with(said::DRAFT_NOT_KEPT_START), "{}", notice(&app));
    assert!(draft_file(&app, &path).exists(), "the old draft is the text's only copy for a crash, so it stays");
    disk.heal();
    app.run(Command::Window(WindowAction::Save));
    settle(&app);
    assert_eq!(file(&copy), MINE, "the copy is saved");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "and only then the old draft goes");
}

/// The 925 delta review, B (P11): one text typed into two notes. Left's file holds it, but Left
/// has its own offer too, of a later edit. Right's draft is the text's only copy that survives
/// Left's Restore, so it is offered, not dropped.
#[test]
fn a_draft_whose_twin_file_has_its_own_offer_is_still_offered() {
    let (app, first, left, second, right) = super::windows::two_notes_two_windows();
    let shared = "# Same\n\nshared text only typed twice";
    super::windows::type_in(&app, second, shared);
    app.run_writes();
    super::windows::type_in(&app, first, shared);
    app.run(Command::From { window: first, command: Box::new(Command::Window(WindowAction::Save)) });
    settle(&app);
    assert_eq!((file(&left), file(&right)), (shared.to_string(), "# Right\n\n".to_string()));
    super::windows::type_in(&app, first, "# Same\n\nshared text, then rewritten");
    app.run_writes();
    let again = killed_and_started_again(app);
    let offers: Vec<String> =
        state(&again)["recovery"].as_array().unwrap().iter().map(|o| o["filename"].as_str().unwrap().to_string()).collect();
    let name = |p: &PathBuf| p.file_name().unwrap().to_str().unwrap().to_string();
    assert!(offers.contains(&name(&right)), "Right's edit is offered: {offers:?}");
    assert!(offers.contains(&name(&left)), "and so is Left's: {offers:?}");
    again.run_writes();
    assert!(draft_file(&again, &right).exists(), "and its draft is kept");
}

/// Two notes' drafts of one text, not one moved text: both are offered, and discarding one keeps
/// the other's.
#[test]
fn two_notes_drafts_of_one_text_are_both_offered_and_discarded_one_at_a_time() {
    let (app, first, left, second, right) = super::windows::two_notes_two_windows();
    let shared = "# Same\n\nthe same words in two notes";
    super::windows::type_in(&app, first, shared);
    super::windows::type_in(&app, second, shared);
    app.run_writes();
    let again = killed_and_started_again(app);
    assert_eq!(state(&again)["recovery"].as_array().unwrap().len(), 2, "both offered");
    act(&again, "discard_draft", json!({})).expect("discarded");
    let left_over = state(&again)["recovery"].as_array().unwrap().clone();
    assert_eq!(left_over.len(), 1, "the other is still offered: {left_over:?}");
    again.run_writes();
    let drafts = [draft_file(&again, &left).exists(), draft_file(&again, &right).exists()];
    assert_eq!(drafts.iter().filter(|kept| **kept).count(), 1, "one draft discarded, one kept: {drafts:?}");
}

/// P12 and P14 of the same review: only a note in the Trash holding the text does not count, and
/// the comparison is exact (one newline more is another text). Either way the draft is offered.
#[test]
fn a_draft_matched_only_by_a_trashed_note_or_nearly_is_offered() {
    for (trashed, typed) in [(true, "# Same\n\nshared\n"), (false, "# Same\n\nshared\n\n")] {
        let (app, path) = with_note("Typed");
        let holder = super::note(&app, "Same", "shared");
        if trashed {
            act(&app, "trash", json!({})).expect("trashed");
        }
        let id = path.file_name().unwrap().to_str().unwrap().to_string();
        app.run(Command::Window(WindowAction::Open { id, trash: false }));
        app.run(Command::Edit(typed.into()));
        app.run_writes();
        let again = killed_and_started_again(app);
        let offers = state(&again)["recovery"].as_array().unwrap().len();
        assert_eq!(offers, 1, "trashed holder {trashed}, {holder}");
    }
}
