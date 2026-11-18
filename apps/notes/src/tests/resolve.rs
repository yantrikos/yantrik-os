//! Gate row "Conflicting outside edit" (design/notes-gate-2026-10-21.md §3), the choices: *Keep
//! mine as a copy*, *Take theirs*, *Compare*, *Not now*, and *Save as new* for a file that is
//! gone. Each is pressed as the bar's button sends it, and called as the action a mind uses,
//! which runs the same command. Whatever is chosen, the person's text is on disk first.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::documents::OutsideState;
use yantrik_app::intents::Intents;
use yantrik_testing::TestApp;

use super::{act, autosave, bar, file, not_saved_over, notice, outside, settle, state, unsaved, vault, with_note};
use crate::command::{Command, WindowAction};
use crate::note::Note;
use crate::update::said;
use crate::Notes;

const MINE: &str = "# Shared\n\nmine";
const THEIRS: &str = "# Shared\n\ntheirs\n";

/// A saved note, open, edited, and changed by another program meanwhile, as the watch saw it.
fn in_conflict() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Shared");
    app.run(Command::Edit(MINE.into()));
    std::fs::write(&path, THEIRS).unwrap();
    app.look_at_files();
    assert_eq!(bar(&app), 1, "the choices are offered");
    (app, path)
}

/// The window's action, by the id its button sends, and what it started, landed.
fn press(app: &TestApp<Notes>, id: &'static str) {
    app.ui(move |ui| ui.invoke_action(id.into()));
    settle(app);
}

fn current_id(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().id().to_string())
}

fn text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

/// The note in the library whose file name starts with `prefix`.
fn listed(app: &TestApp<Notes>, prefix: &'static str) -> Option<Note> {
    app.model(move |m| m.library.iter().find(|n| n.id.starts_with(prefix)).cloned())
}

#[test]
fn keep_mine_as_a_copy_makes_a_new_note_and_leaves_the_file_as_the_other_program_wrote_it() {
    let (app, path) = in_conflict();
    let original = current_id(&app);
    press(&app, "keep-mine");
    let copy = current_id(&app);
    assert!(copy.starts_with("copy-"), "{copy}");
    assert_eq!(file(&vault(&app).join(&copy)), MINE, "my text, as a note of its own");
    assert_eq!(file(&path), THEIRS, "the original untouched");
    assert!(!unsaved(&app));
    assert_eq!((bar(&app), notice(&app)), (0, said::KEPT_MINE.to_string()));
    assert_eq!(state(&app)["outside"], "clean");
    let original_listed = app.model(move |m| m.library.iter().find(|n| n.id == original).map(|n| n.content.text.clone()));
    assert_eq!(original_listed.as_deref(), Some(THEIRS), "listed as its file now holds it");
    assert_eq!(app.watched().into_iter().map(|(_, p)| p).collect::<Vec<_>>(), vec![vault(&app).join(&copy)]);
}

#[test]
fn take_theirs_keeps_my_text_as_a_recovery_note_first_and_starts_the_undo_history_again() {
    let (app, path) = with_note("Shared");
    for typed in ["# Shared\n\nm", "# Shared\n\nmi", MINE] {
        app.run(Command::Edit(typed.into()));
    }
    std::fs::write(&path, THEIRS).unwrap();
    app.look_at_files();
    let original = current_id(&app);
    press(&app, "take-theirs");
    assert_eq!(current_id(&app), original, "the same note, now with the file's text");
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), THEIRS);
    assert_eq!(file(&path), THEIRS, "the file is not written");
    assert!(!unsaved(&app));
    let kept = listed(&app, "recovered-").expect("my text is kept");
    assert_eq!(kept.content.text, MINE);
    assert_eq!(file(&vault(&app).join(&kept.id)), MINE, "on disk, before theirs was taken");
    assert_eq!((bar(&app), notice(&app)), (0, said::TOOK_THEIRS.to_string()));
    app.run(Command::Window(WindowAction::Undo));
    assert_eq!(text(&app), THEIRS, "no undo puts my old text back over theirs");
    // The watch starts again from the file as taken, and the note saves as any other.
    app.look_at_files();
    assert_eq!(state(&app)["outside"], "clean");
    app.run(Command::Edit(format!("{THEIRS}and more")));
    autosave(&app);
    assert_eq!(file(&path), format!("{THEIRS}and more"));
}

#[test]
fn compare_shows_both_side_by_side_read_only_and_changes_nothing() {
    let (app, path) = in_conflict();
    let answer = act(&app, "compare", json!({})).expect("compare");
    assert_eq!((answer["mine"].clone(), answer["theirs"].clone()), (json!(MINE), json!(THEIRS)));
    assert_eq!((answer["identical"].clone(), answer["first_different_line"].clone()), (json!(false), json!(3)));
    assert_eq!(answer["file_exists"], true);
    // The comparison is a dialog, so the editor under it is read-only.
    assert_eq!(app.ui(|ui| (ui.get_dialog(), ui.get_theirs().to_string())), (3, THEIRS.to_string()));
    assert_eq!((file(&path), text(&app)), (THEIRS.to_string(), MINE.to_string()), "nothing changed");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    assert!(listed(&app, "recovered-").is_none() && listed(&app, "copy-").is_none(), "nothing written");
    // A choice from inside the comparison closes it.
    press(&app, "take-theirs");
    assert_eq!(app.ui(|ui| ui.get_dialog()), 0);
    assert_eq!(text(&app), THEIRS);
}

#[test]
fn not_now_keeps_editing_and_the_next_save_is_refused_until_a_choice_is_made() {
    let (app, path) = in_conflict();
    press(&app, "not-now");
    assert_eq!(bar(&app), 0, "put away");
    app.run(Command::Edit(format!("{MINE}, more")));
    autosave(&app);
    assert_eq!((app.jobs_waiting(), notice(&app)), (0, String::new()), "the autosave is refused quietly");
    assert_eq!(app.model(|m| m.pane.status.clone()), said::WAITING_ON_CHOICE, "and the footer says why");
    assert_eq!(file(&path), THEIRS);
    app.run(Command::Window(WindowAction::Save));
    settle(&app);
    assert_eq!((notice(&app), bar(&app)), (not_saved_over(), 1), "the person's Save is refused; the choices are back");
    // Put away again, a new change from outside is news, and brings them back too.
    press(&app, "not-now");
    std::fs::write(&path, "theirs, again").unwrap();
    app.look_at_files();
    assert_eq!(bar(&app), 1);
    assert_eq!((file(&path), text(&app)), ("theirs, again".to_string(), format!("{MINE}, more")));
}

#[test]
fn a_deleted_file_offers_save_as_new_and_is_never_made_again_silently() {
    let (app, path) = with_note("Gone");
    let original = current_id(&app);
    app.run(Command::Edit("# Gone\n\nkeep me".into()));
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert_eq!((bar(&app), state(&app)["outside"].clone()), (2, json!("deleted")));
    let summary = app.describe()["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("(3 lines, unsaved, its file was moved or deleted)"), "{summary}");
    autosave(&app);
    app.run(Command::Window(WindowAction::Save));
    settle(&app);
    assert!(!path.exists(), "no save makes the file again");
    assert_eq!(notice(&app), said::blocked(&OutsideState::Deleted));
    let e = act(&app, "take_theirs", json!({})).unwrap_err();
    assert!(e.contains("no version on disk to take"), "{e}");
    press(&app, "keep-mine");
    let new = current_id(&app);
    assert!(new.starts_with("copy-"), "{new}");
    assert_eq!(file(&vault(&app).join(&new)), "# Gone\n\nkeep me");
    assert!(!path.exists(), "still not made again");
    assert_eq!(notice(&app), said::SAVED_AS_NEW);
    assert!(app.model(move |m| !m.library.iter().any(|n| n.id == original)), "the gone note is not listed");
}

/// With the watch off, a file deleted under a save is a conflict to the store's check; taking
/// theirs then finds nothing to take, and keeps my text as a note of its own.
#[test]
fn take_theirs_on_a_file_found_gone_keeps_my_text_as_a_new_note() {
    let (app, path) = with_note("Gone");
    app.run(Command::Edit("# Gone\n\nkeep me".into()));
    std::fs::remove_file(&path).unwrap();
    autosave(&app);
    assert_eq!(bar(&app), 1);
    press(&app, "take-theirs");
    let kept = current_id(&app);
    assert!(kept.starts_with("recovered-"), "{kept}");
    assert_eq!(file(&vault(&app).join(&kept)), "# Gone\n\nkeep me");
    assert!(!path.exists());
    assert_eq!(notice(&app), said::NOTHING_TO_TAKE);
}

/// Review finding 3: with the library at its limit, refusing the kept note would leave no way out
/// of the conflict (saving is refused, and so is everything that leaves the note) but to lose the
/// text. A resolution's kept note is a one-off recovery, so it passes the limit.
#[test]
fn a_full_library_still_lets_a_conflict_be_resolved() {
    for (choice, prefix, sentence) in
        [("keep-mine", "copy-", said::KEPT_MINE), ("take-theirs", "recovered-", said::TOOK_THEIRS)]
    {
        let paths = yantrik_app::paths::Paths::temporary(Notes::ID);
        std::fs::create_dir_all(paths.data_dir()).unwrap();
        for i in 0..crate::store::MAX_NOTES {
            std::fs::write(paths.data_dir().join(format!("n{i:04}.md")), format!("# Note {i}\n")).unwrap();
        }
        let app = TestApp::<Notes>::start_at(paths, Vec::new());
        settle(&app);
        app.run(Command::Window(WindowAction::Open { id: "n0007.md".into(), trash: false }));
        app.run(Command::Edit("# Note 7\n\nmine".into()));
        let path = vault(&app).join("n0007.md");
        std::fs::write(&path, "# Note 7\n\ntheirs\n").unwrap();
        app.look_at_files();
        assert_eq!(bar(&app), 1, "{choice}");
        press(&app, choice);
        assert_eq!(notice(&app), sentence, "{choice}");
        let kept = listed(&app, prefix).unwrap_or_else(|| panic!("{choice}: my text is kept"));
        assert_eq!(file(&vault(&app).join(&kept.id)), "# Note 7\n\nmine");
        assert_eq!(app.model(|m| m.library.len()), crate::store::MAX_NOTES + 1, "{choice}: past the limit, once");
        assert_eq!(file(&path), "# Note 7\n\ntheirs\n");
    }
}

/// Neither choice goes on unless the window's text is on disk first.
#[test]
fn a_choice_whose_kept_note_cannot_be_written_changes_nothing() {
    for choice in ["keep-mine", "take-theirs"] {
        let (app, path) = in_conflict();
        let original = current_id(&app);
        super::failing(&app).fail(yantrik_app::documents::disk::Step::Write, ".md", Some(1));
        press(&app, choice);
        assert_eq!(notice(&app), said::not_kept("injected failure at Write"), "{choice}");
        assert_eq!((current_id(&app), text(&app)), (original, MINE.to_string()), "{choice}: as it was");
        assert_eq!((bar(&app), file(&path)), (1, THEIRS.to_string()), "{choice}: still to choose");
        assert!(listed(&app, "copy-").is_none() && listed(&app, "recovered-").is_none());
    }
}

#[test]
fn typing_while_take_theirs_runs_replaces_nothing_and_keeps_both() {
    let (app, path) = in_conflict();
    app.ui(|ui| ui.invoke_action("take-theirs".into()));
    assert_eq!(app.jobs_waiting(), 1);
    app.run(Command::Edit(format!("{MINE}, typed on")));
    settle(&app);
    assert_eq!(text(&app), format!("{MINE}, typed on"), "the newer text is not replaced unasked");
    assert_eq!((notice(&app), bar(&app)), (said::TYPED_MEANWHILE.to_string(), 1));
    assert_eq!(listed(&app, "recovered-").unwrap().content.text, MINE, "the earlier text is kept too");
    assert_eq!(file(&path), THEIRS);
}

#[test]
fn typing_while_keep_mine_runs_goes_on_in_the_copy() {
    let (app, path) = in_conflict();
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    app.run(Command::Edit(format!("{MINE}, typed on")));
    settle(&app);
    let copy = current_id(&app);
    assert!(copy.starts_with("copy-"));
    assert_eq!(text(&app), format!("{MINE}, typed on"));
    assert!(unsaved(&app), "what was typed meanwhile is an edit of the copy");
    autosave(&app);
    assert_eq!(file(&vault(&app).join(&copy)), format!("{MINE}, typed on"));
    assert_eq!(file(&path), THEIRS);
}

#[test]
fn the_choices_are_actions_graded_honestly_that_run_the_buttons_commands() {
    let mut table = Intents::<Notes>::new();
    Notes::intents(&mut table);
    let grade = |name: &str| table.specs().into_iter().find(|s| s.name == name).unwrap().permission;
    assert_eq!(grade("keep_mine_as_copy"), "sensitive", "it writes a note and moves the window");
    assert_eq!(grade("take_theirs"), "sensitive", "it replaces the window's text");
    assert_eq!(grade("compare"), "safe", "it reads, and writes nothing");

    // With nothing to resolve, each is refused in words, and nothing is written.
    let (app, _) = with_note("Calm");
    for name in ["keep_mine_as_copy", "take_theirs", "compare"] {
        let e = act(&app, name, json!({})).unwrap_err();
        assert!(e.starts_with("Nothing to resolve"), "{name}: {e}");
        assert_eq!(notice(&app), e, "said in the window too");
    }
    assert_eq!(app.model(|m| m.library.len()), 1);

    let (app, path) = in_conflict();
    let original = current_id(&app);
    assert_eq!(state(&app)["outside"], "conflict");
    let summary = app.describe()["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("(3 lines, unsaved, changed outside Notes)"), "{summary}");
    let took = act(&app, "take_theirs", json!({})).expect("take_theirs");
    assert_eq!((took["filename"].clone(), took["saved"].clone()), (json!(original), json!(true)));
    assert!(took["kept_as"].as_str().unwrap().starts_with("recovered-"), "{took}");

    app.run(Command::Edit(MINE.into()));
    std::fs::write(&path, "theirs, again").unwrap();
    app.look_at_files();
    let kept = act(&app, "keep_mine_as_copy", json!({})).expect("keep_mine_as_copy");
    assert_eq!(kept["copied_from"], json!(original));
    assert!(kept["filename"].as_str().unwrap().starts_with("copy-"), "{kept}");
    assert_eq!(kept["saved"], true);
    assert_eq!(file(&path), "theirs, again");
}
