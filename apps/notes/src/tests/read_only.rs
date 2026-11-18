//! A library whose version file cannot be written opens read-only and says why: every change, from
//! the window or a call, is refused with that sentence and writes nothing; every read works; and
//! the library is written again from the first Refresh that can make the marker.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use yantrik_app::documents::disk::{FaultyDisk, Step};
use yantrik_testing::TestApp;

use super::{act, autosave, failing, note, settle, snapshot, started, state, vault};
use crate::command::{Command, WindowAction};
use crate::store::version::MARKER;
use crate::Notes;

const READ_ONLY: &str = "This library is open read-only: Notes could not write its version file (injected failure \
                         at CreateTemp). Make the folder writable, then Refresh.";

/// The library loses its marker and cannot make another: a reload, or a start, finds it so.
fn turn_read_only(app: &TestApp<Notes>) -> Arc<FaultyDisk> {
    std::fs::remove_file(vault(app).join(MARKER)).unwrap();
    let disk = failing(app);
    disk.fail(Step::CreateTemp, MARKER, None);
    app.run(Command::Window(WindowAction::Reload));
    disk
}

/// A library with "Binned" in the Trash and "Open" open, read-only.
fn read_only_library() -> (TestApp<Notes>, Arc<FaultyDisk>) {
    let app = started();
    note(&app, "Binned", "old");
    act(&app, "trash", json!({})).unwrap();
    note(&app, "Open", "body");
    let disk = turn_read_only(&app);
    settle(&app);
    assert_eq!(state(&app)["notice"], READ_ONLY, "the window says why");
    assert_eq!(state(&app)["library_ready"], true, "and the library is open to read");
    assert!(app.ui(|ui| ui.get_read_only()), "and the editor and fields take no keys");
    (app, disk)
}

/// A read-only library holding text typed during the reload that found it so, whose owed save
/// the vault has refused: the note's file, what it holds, and the text kept.
pub(super) fn holding_unsaved_text() -> (TestApp<Notes>, Arc<FaultyDisk>, PathBuf, String, String) {
    let app = started();
    let path = PathBuf::from(note(&app, "Draft", "")["path"].as_str().unwrap());
    let saved = std::fs::read_to_string(&path).unwrap();
    let disk = turn_read_only(&app);
    // Typed while the reload runs: kept when it lands, so a save of it is owed.
    let typed = format!("{saved}typed meanwhile\n");
    app.run(Command::Edit(typed.clone()));
    settle(&app);
    assert!(app.model(|m| m.unsaved()));
    autosave(&app);
    (app, disk, path, saved, typed)
}

/// How many times the marker has been tried since the library went read-only.
fn marker_tries(disk: &FaultyDisk) -> usize {
    disk.hits.lock().unwrap().iter().filter(|p| p.ends_with(MARKER)).count()
}

fn open_note(app: &TestApp<Notes>) -> (String, String) {
    app.model(|m| m.current().map(|c| (c.id().to_string(), c.text().to_string())).unwrap())
}

#[test]
fn every_change_is_refused_with_the_reason_and_nothing_is_written() {
    let (app, _disk) = read_only_library();
    let before = snapshot(&vault(&app));
    let (id, text) = open_note(&app);
    let notes = app.model(|m| m.library.len());
    let window = [
        ("typing", Command::Edit(format!("{text}typed"))),
        ("the fields", Command::Metadata { notebook: "Work".into(), tags: "x".into() }),
        ("save", Command::Window(WindowAction::Save)),
        ("pin", Command::Window(WindowAction::Pin)),
        ("new", Command::Window(WindowAction::New(None))),
        ("copy", Command::Window(WindowAction::Copy)),
        ("trash", Command::Window(WindowAction::Trash)),
        ("import", Command::Window(WindowAction::Import)),
        ("undo", Command::Window(WindowAction::Undo)),
        ("redo", Command::Window(WindowAction::Redo)),
    ];
    for (what, command) in window {
        // Opening the note again is a read, and it clears the notice, so each refusal is its own.
        app.run(Command::Window(WindowAction::Open { id: id.clone(), trash: false }));
        assert_eq!(state(&app)["notice"], "");
        app.run(command);
        settle(&app);
        assert_eq!(state(&app)["notice"], READ_ONLY, "{what}");
        assert_eq!(open_note(&app), (id.clone(), text.clone()), "{what} changed the open note");
        assert_eq!(app.model(|m| (m.library.len(), m.pane.dialog, m.unsaved())), (notes, 0, false), "{what}");
    }
    let calls = [
        ("new_note", json!({ "title": "New" })),
        ("set_title", json!({ "title": "Renamed" })),
        ("append", json!({ "text": "more" })),
        ("set_content", json!({ "text": "# Other" })),
        ("notebook", json!({ "text": "Work" })),
        ("tags", json!({ "text": "x" })),
        ("save", json!({})),
        ("trash", json!({})),
        ("copy", json!({})),
        ("undo", json!({})),
        ("redo", json!({})),
        ("import", json!({ "path": "/nonexistent/in.md" })),
    ];
    for (name, args) in calls {
        assert_eq!(act(&app, name, args), Err(READ_ONLY.to_string()), "{name}");
    }
    act(&app, "set_folder", json!({ "folder": "trash" })).unwrap();
    let binned = app.model(|m| m.library.iter().find(|n| n.trash).unwrap().id.clone());
    act(&app, "open_note", json!({ "title": format!("trash:{binned}") })).unwrap();
    assert_eq!(act(&app, "restore", json!({})), Err(READ_ONLY.to_string()), "restore");
    assert_eq!(snapshot(&vault(&app)), before, "a refused change wrote to the vault");
}

/// An outside change under text a read-only library holds unsaved: the two resolutions write a
/// note, so they are refused at the door with the reason; Compare only reads, so it works.
#[test]
fn a_conflict_in_a_read_only_library_is_compared_but_not_resolved() {
    let (app, _disk, path, _saved, typed) = holding_unsaved_text();
    std::fs::write(&path, "# Draft\n\ntheirs\n").unwrap();
    app.look_at_files();
    assert_eq!(state(&app)["outside"], "conflict");
    let before = snapshot(&vault(&app));
    for action in [WindowAction::KeepMine, WindowAction::TakeTheirs] {
        app.run(Command::Window(action.clone()));
        settle(&app);
        assert_eq!(state(&app)["notice"], READ_ONLY, "{action:?}");
    }
    for name in ["keep_mine_as_copy", "take_theirs"] {
        assert_eq!(act(&app, name, json!({})), Err(READ_ONLY.to_string()), "{name}");
    }
    assert_eq!(act(&app, "compare", json!({})).unwrap()["theirs"], "# Draft\n\ntheirs\n");
    assert_eq!(open_note(&app).1, typed, "the text is kept");
    assert_eq!(snapshot(&vault(&app)), before, "nothing was written");
}

#[test]
fn reading_and_exporting_go_on_in_a_read_only_library() {
    let (app, _disk) = read_only_library();
    let before = snapshot(&vault(&app));
    assert_eq!(act(&app, "read_notes", json!({})).unwrap()["matching"], 1);
    assert_eq!(act(&app, "search", json!({ "query": "Open" })).unwrap()["matches"], 1);
    assert_eq!(act(&app, "open_note", json!({ "title": "Open" })).unwrap()["title"], "Open");
    act(&app, "preview", json!({})).unwrap();
    act(&app, "focus", json!({})).unwrap();
    assert_eq!(act(&app, "reload", json!({})).unwrap()["notice"], READ_ONLY, "a reload says it again");
    // An export writes outside the vault, never into it: it is how a read-only library's notes
    // get out.
    let to: PathBuf = super::store::fixture("read-only-export").join("out.md");
    let exported = act(&app, "export", json!({ "path": to.display().to_string() })).unwrap();
    assert_eq!(exported["matches_note"], true);
    assert_eq!(state(&app)["note_count"], 1);
    assert_eq!(snapshot(&vault(&app)), before);
}

#[test]
fn the_first_refresh_that_can_make_the_marker_makes_the_library_writable_again() {
    let (app, disk) = read_only_library();
    disk.heal();
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!(state(&app)["notice"], "");
    assert_eq!(std::fs::read_to_string(vault(&app).join(MARKER)).unwrap(), "2\n");
    assert_eq!(note(&app, "Written again", "")["saved"], true);
}

#[test]
fn a_save_owed_when_the_library_turns_read_only_is_refused_and_the_text_is_kept() {
    let (app, _disk, path, saved, typed) = holding_unsaved_text();
    assert_eq!(state(&app)["notice"], READ_ONLY, "the vault refused the save");
    assert_eq!(app.model(|m| m.pane.status.clone()), "Needs attention · draft kept");
    assert_eq!(open_note(&app).1, typed, "the text is kept");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved, "and nothing was written");
    assert!(!vault(&app).join(".recovery-v2/pending.json").exists(), "not even the journal");
}

#[test]
fn a_refresh_once_the_folder_is_fixed_makes_the_library_writable_and_saves_the_kept_text() {
    let (app, disk, path, _, typed) = holding_unsaved_text();
    disk.heal();
    // No save first: the vault would refuse it, and this Refresh is what makes it writable.
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!(app.model(|m| m.vault.read_only().map(str::to_string)), None);
    assert_eq!(std::fs::read_to_string(vault(&app).join(MARKER)).unwrap(), "2\n");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), typed, "saved with no further key");
    assert!(!app.model(|m| m.unsaved()));
    assert_eq!(state(&app)["notice"], "");
    assert!(!app.ui(|ui| ui.get_read_only()), "the editor takes keys again");
}

#[test]
fn a_reload_call_on_a_read_only_library_with_unsaved_text_reloads_and_is_answered_truly() {
    let (app, disk, path, saved, typed) = holding_unsaved_text();
    let tries = marker_tries(&disk);
    let still = act(&app, "reload", json!({})).unwrap();
    assert_eq!(marker_tries(&disk), tries + 1, "the library was read again");
    assert_eq!(still["notice"], READ_ONLY, "and is still read-only, as the answer says");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
    assert_eq!(open_note(&app).1, typed, "the text is kept across it");

    disk.heal();
    let healed = act(&app, "reload", json!({})).unwrap();
    assert_eq!(healed["notice"], "", "answered once the library is writable and the text saved");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), typed);
}

#[test]
fn an_export_call_on_a_read_only_library_with_unsaved_text_exports_the_text_in_the_window() {
    let (app, _disk, _, _, typed) = holding_unsaved_text();
    let to = super::store::fixture("read-only-unsaved-export").join("out.md");
    let exported = act(&app, "export", json!({ "path": to.display().to_string() })).unwrap();
    assert_eq!(exported["matches_note"], true);
    assert_eq!(std::fs::read_to_string(&to).unwrap(), typed);
}

#[test]
fn an_export_into_a_read_only_vault_or_a_folder_in_it_is_refused() {
    let (app, _disk) = read_only_library();
    let before = snapshot(&vault(&app));
    // The folder itself is writable here, as a vault a newer Notes wrote may be.
    let link = super::store::fixture("into-vault").join("vault");
    std::os::unix::fs::symlink(vault(&app), &link).unwrap();
    for to in [vault(&app).join("out.md"), vault(&app).join(".trash-v2/out.md"), link.join("out.md")] {
        let refused = act(&app, "export", json!({ "path": to.display().to_string() }));
        assert_eq!(refused, Err(READ_ONLY.to_string()), "{}", to.display());
    }
    assert_eq!(snapshot(&vault(&app)), before, "an export wrote into the read-only vault");
}
