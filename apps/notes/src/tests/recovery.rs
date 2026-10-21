//! Gate row "Recovery after a forced kill" (design/notes-gate-2026-10-21.md §3): a draft per note,
//! kept through the framework's drafts lane, and at the next start the newest of the file, the
//! journal and the draft, offered back only where it differs from the file, and never put back
//! until the person chooses. One note in two views giving one draft is 3b's.

use std::path::{Path, PathBuf};

use serde_json::json;
use yantrik_app::documents::disk::Step;
use yantrik_app::documents::{Envelope, OutsideState, Source};
use yantrik_testing::TestApp;

use super::{
    act, autosave, draft_file, failing, file, killed_and_started_again, note, notice, outside, settle, started,
    state, unsaved, with_note,
};
use crate::command::{Command, WindowAction};
use crate::note::NoteText;
use crate::update::said;
use crate::Notes;

fn envelope(path: &Path) -> Envelope<NoteText> {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// What the recovery bar offers, as the window says it.
fn bar(app: &TestApp<Notes>) -> String {
    app.ui(|ui| ui.get_recovery_said().to_string())
}

fn offered(app: &TestApp<Notes>) -> Vec<String> {
    state(app)["recovery"].as_array().unwrap().iter().map(|o| o["title"].as_str().unwrap().to_string()).collect()
}

fn open_text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

fn press(app: &TestApp<Notes>, id: &'static str) {
    app.ui(move |ui| ui.invoke_action(id.into()));
    settle(app);
}

/// A saved note titled `title`, with `typed` typed and its draft written, then the app killed.
fn typed_then_killed(title: &str, typed: &str) -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note(title);
    app.run(Command::Edit(typed.into()));
    app.run_writes();
    (killed_and_started_again(app), path)
}

#[test]
fn a_draft_is_kept_after_the_debounce_and_the_latest_edit_takes_its_place() {
    let (app, path) = with_note("Kept");
    let saved = file(&path);
    app.run(Command::Edit("# Kept\n\none".into()));
    assert!(!draft_file(&app, &path).exists(), "nothing is written per key");
    app.run(Command::Edit("# Kept\n\none two".into()));
    app.run_writes();
    let kept = envelope(&draft_file(&app, &path));
    assert_eq!(kept.content.text, "# Kept\n\none two", "the latest edit");
    assert_eq!(kept.seq, app.model(|m| m.current().unwrap().doc.seq));
    let files = NoteText { text: saved, meta: String::new() };
    assert_eq!(kept.baseline, crate::recovery::base_of(Some(&files)), "and the files it was written over");
    assert_eq!(state(&app)["draft"], "kept");
}

#[test]
fn a_note_typed_in_and_killed_is_offered_back_at_the_next_start_and_restored() {
    let typed = "# Lost\n\nwords typed before the crash";
    let (app, path) = typed_then_killed("Lost", typed);
    assert_eq!(file(&path), "# Lost\n\n", "nothing was put back unasked");
    assert_eq!(offered(&app), vec!["Lost".to_string()]);
    assert_eq!(notice(&app), "", "the bar says it; the notice is for when there are more");
    // Its note is opened, and the editor shows the text the choice is about, read-only.
    assert_eq!(bar(&app), said::offer_open("Lost"));
    assert_eq!(app.ui(|ui| (ui.get_recovery(), ui.get_content().to_string())), (1, typed.to_string()));
    assert_eq!(open_text(&app), "# Lost\n\n", "the note itself is still as its file holds it");
    // The note an offer is for takes no edits until the person chooses.
    app.run(Command::Edit("# Lost\n\nsomething else".into()));
    assert_eq!((notice(&app), open_text(&app)), (said::CHOOSE_FIRST.to_string(), "# Lost\n\n".to_string()));
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), typed, "the editor is put back to the offered text");
    assert!(app.ui(|ui| ui.get_recovering()), "and the editor says so by taking no keys");
    // The footer does not call it saved: it is on no disk but its recovery copy.
    let footer = app.ui(|ui| (ui.get_status().to_string(), ui.get_modified()));
    assert_eq!(footer, (said::WAITING_ON_RECOVERY.to_string(), true), "said, in amber");
    press(&app, "restore-draft");
    assert_eq!(open_text(&app), typed, "the text is back");
    assert_eq!(file(&path), typed, "and saved");
    assert!(offered(&app).is_empty() && bar(&app).is_empty());
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "the draft goes once its text is saved");
    let again = killed_and_started_again(app);
    assert!(offered(&again).is_empty(), "and nothing is offered twice");
}

#[test]
fn discard_keeps_the_note_as_its_file_holds_it_and_drops_the_draft() {
    let (app, path) = typed_then_killed("Dropped", "# Dropped\n\nnot wanted");
    let answer = act(&app, "discard_draft", json!({})).expect("discarded");
    assert_eq!(answer["still_offered"], 0);
    assert_eq!(file(&path), "# Dropped\n\n");
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), "# Dropped\n\n", "the editor shows the file again");
    let footer = app.ui(|ui| (ui.get_status().to_string(), ui.get_modified()));
    assert_eq!(footer, ("All changes saved".to_string(), false), "and the footer says it is saved, as it is");
    assert!(offered(&app).is_empty());
    app.run_writes();
    assert!(!draft_file(&app, &path).exists());
    assert_eq!(act(&app, "discard_draft", json!({})).unwrap_err(), said::NOTHING_OFFERED);
}

/// The text a draft holds goes back into its note only when the file is still the one it was
/// written over; otherwise it is a conflict the person resolves, and the file is left as it is.
#[test]
fn a_restore_over_a_file_changed_since_is_a_conflict_and_replaces_nothing() {
    let (app, path) = with_note("Moved on");
    app.run(Command::Edit("# Moved on\n\nmine".into()));
    app.run_writes();
    let paths = app.paths();
    drop(app);
    std::fs::write(&path, "# Moved on\n\ntheirs, written while Notes was not running").unwrap();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    let restored = act(&app, "restore_draft", json!({})).expect("restored");
    assert_eq!(restored["saved"], false, "not saved over theirs");
    assert_eq!(open_text(&app), "# Moved on\n\nmine");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }), "{:?}", outside(&app));
    assert_eq!(super::bar(&app), 1, "the outside choices are offered");
    assert_eq!(file(&path), "# Moved on\n\ntheirs, written while Notes was not running");
    assert_eq!(notice(&app), said::RESTORED_BESIDE);
}

/// An interrupted save (its journal) and a draft of the same note: the newer is offered, and the
/// older is never lost silently either.
#[test]
fn a_draft_newer_than_an_interrupted_save_is_the_one_offered() {
    let (app, path) = with_note("Race");
    let disk = failing(&app);
    disk.fail(Step::Write, ".md", None);
    app.run(Command::Edit("# Race\n\nsaved halfway".into()));
    autosave(&app);
    assert!(unsaved(&app), "the save stopped after its journal");
    app.run(Command::Edit("# Race\n\nsaved halfway, then typed on".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(offered(&app), vec!["Race".to_string()]);
    let offer = app.model(|m| m.recovery.offers[0].content.text.clone());
    assert_eq!(offer, "# Race\n\nsaved halfway, then typed on", "the draft, which is newer");
    let journal = app.model(|m| m.library.iter().find(|n| n.id.starts_with("recovered-")).map(|n| n.content.text.clone()));
    assert_eq!(journal.as_deref(), Some("# Race\n\nsaved halfway"), "the older text came back as a note of its own");
    assert_eq!(file(&path), "# Race\n\n");
}

#[test]
fn an_interrupted_save_newer_than_the_draft_wins_and_the_draft_is_dropped() {
    let (app, path) = with_note("Race");
    app.run(Command::Edit("# Race\n\nfirst".into()));
    app.run_writes();
    let disk = failing(&app);
    disk.fail(Step::Write, ".md", None);
    // Typed on, and saved halfway, before that edit's own draft reached the disk.
    app.run(Command::Edit("# Race\n\nfirst, second".into()));
    autosave(&app);
    let app = killed_and_started_again(app);
    assert!(offered(&app).is_empty(), "the older draft is not offered");
    let journal = app.model(|m| m.library.iter().find(|n| n.id.starts_with("recovered-")).map(|n| n.content.text.clone()));
    assert_eq!(journal.as_deref(), Some("# Race\n\nfirst, second"), "the newer text, the interrupted save, is kept");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "and the draft it superseded is removed");
}

#[test]
fn a_save_that_lands_removes_the_draft_and_a_stale_draft_is_not_offered() {
    let (app, path) = with_note("Saved");
    app.run(Command::Edit("# Saved\n\ndone".into()));
    app.run_writes();
    assert!(draft_file(&app, &path).exists());
    autosave(&app);
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "removed once the save of its edits landed");
    // A draft left by a crash between a save and its draft's removal says what the file says.
    let stale = Envelope {
        version: 1,
        key: Source::File(path.clone()).key(),
        path: Some(path.display().to_string()),
        seq: 9,
        at: 1,
        content: NoteText { text: file(&path), meta: String::new() },
        baseline: None,
    };
    std::fs::write(draft_file(&app, &path), serde_json::to_vec(&stale).unwrap()).unwrap();
    let app = killed_and_started_again(app);
    assert!(offered(&app).is_empty() && bar(&app).is_empty(), "nothing differs from the file");
    app.run_writes();
    assert!(!draft_file(&app, &path).exists(), "and the stale draft is removed");
}

#[test]
fn three_notes_with_drafts_are_each_offered_and_restored() {
    // Three runs, each killed while typing in a note of its own.
    let (app, first) = typed_then_killed("One", "# One\n\nfirst run");
    let second = std::path::PathBuf::from(note(&app, "Two", "")["path"].as_str().unwrap());
    app.run(Command::Edit("# Two\n\nsecond run".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    let third = std::path::PathBuf::from(note(&app, "Three", "")["path"].as_str().unwrap());
    app.run(Command::Edit("# Three\n\nthird run".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(offered(&app), vec!["Three", "Two", "One"], "every one, newest first");
    assert!(notice(&app).contains(&said::offered(3)));
    for _ in 0..3 {
        act(&app, "restore_draft", json!({})).expect("restored");
    }
    assert_eq!(
        [file(&first), file(&second), file(&third)],
        ["# One\n\nfirst run", "# Two\n\nsecond run", "# Three\n\nthird run"]
    );
    assert!(offered(&app).is_empty());
}

#[test]
fn a_draft_that_cannot_be_written_is_said() {
    let (app, _) = with_note("Unprotected");
    // The drafts folder cannot be made: a file has its name.
    let drafts = app.paths().state_dir().join(yantrik_app::drafts::DIR);
    let _ = std::fs::remove_dir_all(&drafts);
    std::fs::write(&drafts, "").unwrap();
    app.run(Command::Edit("# Unprotected\n\ntyped".into()));
    app.run_writes();
    assert!(notice(&app).starts_with(said::DRAFT_NOT_KEPT_START), "{}", notice(&app));
    assert_eq!(state(&app)["draft"], "failed");
    // Not tried again until the next edit: the failure is not a loop of writes.
    app.run(Command::Filter { folder: "all".into(), query: String::new() });
    assert_eq!(app.writes_waiting(), 0);
    // A save makes the text safe, and the notice goes with it.
    autosave(&app);
    assert_eq!((notice(&app), state(&app)["unsaved"].clone()), (String::new(), json!(false)));
}

/// A start with nothing kept offers nothing, and the bar is not drawn.
#[test]
fn nothing_kept_offers_nothing() {
    let app = started();
    assert!(offered(&app).is_empty() && bar(&app).is_empty());
    assert_eq!(act(&app, "restore_draft", json!({})).unwrap_err(), said::NOTHING_OFFERED);
}

/// #917 review finding 3: the draft's baseline is of the text alone, so a sidecar another program
/// changed since (tags, notebook, pin) is not put back over silently either.
#[test]
fn a_restore_over_a_sidecar_changed_since_is_a_conflict_too() {
    let (app, path) = with_note("Tagged");
    app.run(Command::Edit("# Tagged\n\nmine".into()));
    app.run_writes();
    let paths = app.paths();
    drop(app);
    std::fs::write(path.with_extension("meta"), "tags:theirs\n").unwrap();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    act(&app, "restore_draft", json!({})).expect("restored");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }), "{:?}", outside(&app));
    assert_eq!(std::fs::read_to_string(path.with_extension("meta")).unwrap(), "tags:theirs\n", "theirs kept");
    assert_eq!(file(&path), "# Tagged\n\n", "and nothing saved over the note");
}

/// #917 review finding 4: a recovery copy that cannot be read, or is of another library, is left
/// where it is, and its place is said.
#[test]
fn a_draft_that_cannot_be_read_or_is_of_another_library_is_said_and_left() {
    let app = started();
    let dir = app.paths().state_dir().join(yantrik_app::drafts::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let torn = dir.join("doc-torn.json");
    std::fs::write(&torn, br#"{"version":1,"key":"doc-torn","pa"#).unwrap();
    let elsewhere_path = PathBuf::from("/elsewhere/library/a.md");
    let key = Source::File(elsewhere_path.clone()).key();
    let other = Envelope {
        version: 1,
        key: key.clone(),
        path: Some(elsewhere_path.display().to_string()),
        seq: 1,
        at: 1,
        content: NoteText { text: "# Elsewhere\n".into(), meta: String::new() },
        baseline: None,
    };
    let other_file = dir.join(format!("{key}.json"));
    std::fs::write(&other_file, serde_json::to_vec(&other).unwrap()).unwrap();
    let app = killed_and_started_again(app);
    let said_now = notice(&app);
    assert!(said_now.contains(&format!("A recovery copy could not be read; it is kept at {}.", torn.display())), "{said_now}");
    assert!(said_now.contains(&format!("A recovery copy of a note in another library is kept at {}.", other_file.display())), "{said_now}");
    assert!(offered(&app).is_empty());
    app.run_writes();
    assert!(torn.exists() && other_file.exists(), "both are left where they are");
}

/// The bar's offer is the first in `describe.recovery`, and a note not open is shown before it can
/// be discarded from the window.
#[test]
fn the_bar_offers_the_first_listed_and_shows_a_note_before_discard() {
    let (app, first) = typed_then_killed("One", "# One\n\nfirst run");
    let second = std::path::PathBuf::from(note(&app, "Two", "")["path"].as_str().unwrap());
    app.run(Command::Edit("# Two\n\nsecond run".into()));
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(offered(&app), vec!["Two", "One"], "the open one, Two, first");
    // Another note open: the bar offers the first listed, which is not open, and shows it first.
    let id = first.file_name().unwrap().to_str().unwrap().to_string();
    app.run(Command::Window(WindowAction::New(Some("Elsewhere".into()))));
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_recovery()), 2, "Show, then the choice");
    press(&app, "show-draft");
    let open = app.model(|m| m.current().unwrap().id().to_string());
    assert!(open == id || second.ends_with(&open), "a note with an offer is open: {open}");
    assert_eq!(app.ui(|ui| ui.get_recovery()), 1);
    assert_eq!(offered(&app)[0], app.model(|m| m.current().unwrap().title()), "the open one is first");
}

/// The #917 delta review, B and C: while an offer waits, the window draws all of the offered
/// note (title, fields, list card) from it, and Copy and Export take what is shown. And a sidecar
/// changed by the person before Notes closed is theirs to restore, not a conflict: the draft's
/// baseline covers the note's text and sidecar together.
#[test]
fn an_offer_is_drawn_whole_copied_and_exported_as_shown_and_restores_its_own_sidecar() {
    let (app, path) = with_note("Tagged");
    app.run(Command::Edit("# Renamed\n\nmine".into()));
    app.run(Command::Metadata { notebook: String::new(), tags: "mine".into() });
    app.run_writes();
    let app = killed_and_started_again(app);
    assert_eq!(app.ui(|ui| ui.get_recovery()), 1);
    let drawn = app.ui(|ui| {
        use slint::Model as _;
        let row = ui.get_notes().row_data(0).unwrap();
        (ui.get_note_title().to_string(), ui.get_tags().to_string(), row.title.to_string())
    });
    assert_eq!(drawn, ("Renamed".to_string(), "mine".to_string(), "Renamed".to_string()), "title, fields and card");
    // Export and Copy take what the window shows.
    let out = path.parent().unwrap().parent().unwrap().join(format!("offer-export-{}.md", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let exported = act(&app, "export", json!({ "path": out.display().to_string() })).expect("exported");
    assert_eq!((file(&out), exported["matches_note"].clone()), ("# Renamed\n\nmine".to_string(), json!(true)));
    let copied = act(&app, "copy", json!({})).expect("copied");
    let copy = super::vault(&app).join(copied["filename"].as_str().unwrap());
    assert_eq!(file(&copy), "# Renamed\n\nmine", "the copy holds the offered text");
    assert_eq!(offered(&app), vec!["Renamed".to_string()], "and the offer still waits");
    // The person's own tag change restores cleanly: the note on disk is the one it was written over.
    let id = path.file_name().unwrap().to_str().unwrap().to_string();
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    act(&app, "restore_draft", json!({})).expect("restored");
    assert_eq!(outside(&app), OutsideState::Clean);
    assert_eq!(file(&path), "# Renamed\n\nmine");
    assert!(std::fs::read_to_string(path.with_extension("meta")).unwrap().contains("tags:mine"));
}


/// The 520 run of #925: while an offer is shown, `describe` says what the window shows: the
/// offered text, unsaved, kept as a draft; the same keys as always.
#[test]
fn describe_agrees_with_the_window_while_an_offer_is_shown() {
    let typed = "# Shown\n\nthe text from before the crash";
    let (app, _) = typed_then_killed("Shown", typed);
    let described = state(&app);
    assert_eq!(described["content"], typed, "the text the window shows");
    assert_eq!((described["unsaved"].clone(), described["draft"].clone()), (json!(true), json!("kept")));
    assert_eq!(described["windows"][0]["unsaved"], true);
    assert_eq!(described["status"], said::WAITING_ON_RECOVERY);
    let summary = app.describe()["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("unsaved"), "{summary}");
    act(&app, "discard_draft", json!({})).expect("discarded");
    let described = state(&app);
    assert_eq!((described["unsaved"].clone(), described["draft"].clone()), (json!(false), json!("none")));
    assert_eq!(described["content"], "# Shown\n\n", "and the file's text once the offer is gone");
}
