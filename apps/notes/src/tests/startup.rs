//! One copy per library: a second Notes on a library another copy holds hands its launch over and
//! does not start, and the lock in the vault is never a note. A library a newer Notes wrote is
//! refused at start, before anything in it is touched.

use std::sync::Arc;

use yantrik_app::app::{App as _, Boot, Startup};
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::paths::Paths;
use yantrik_app::Services;
use yantrik_testing::TestApp;

use super::{settle, snapshot, started, state, vault};
use crate::claim::{ALREADY_RUNNING, LEGACY_NAME, VAULT_NAME};
use crate::store::version::MARKER;
use crate::Notes;

fn boot(paths: &Paths) -> Boot {
    Boot::<()>::open(paths.clone(), Vec::new(), Arc::new(FakeMind::new()), Services::fakes())
        .expect("a launch with no arguments")
        .0
}

/// What a second start at `paths` says, or `None` when it started.
fn second_start(paths: &Paths) -> Option<String> {
    declined(paths).map(|e| e.sentence)
}

/// Why a start at `paths` did not go on, or `None` when it started.
fn declined(paths: &Paths) -> Option<Startup> {
    Notes::model(&boot(paths)).err()
}

#[test]
fn a_second_copy_on_a_held_library_does_not_start() {
    let paths = Paths::temporary(Notes::ID);
    let _first = TestApp::<Notes>::start_at(paths.clone(), Vec::new());
    assert_eq!(second_start(&paths).as_deref(), Some(ALREADY_RUNNING));
}

#[test]
fn a_copy_the_library_claim_turns_away_has_handed_over_and_has_not_failed() {
    // A hand-over ends the process with 0 and files no problem (the runner's `model_declined`).
    let paths = Paths::temporary(Notes::ID);
    let _first = TestApp::<Notes>::start_at(paths.clone(), Vec::new());
    assert!(declined(&paths).expect("held").is_handed_over());
}

#[test]
fn a_library_a_newer_notes_wrote_is_refused_at_start_and_left_as_it_was() {
    let paths = Paths::temporary(Notes::ID);
    let dir = paths.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MARKER), "3\n").unwrap();
    std::fs::write(dir.join("a.md"), "# Written by format 3\n").unwrap();
    let before = snapshot(&dir);
    let refused = declined(&paths).expect("refused");
    assert_eq!(
        refused.sentence,
        "This library was written by a newer Notes (format 3); this Notes reads format 2. It was not \
         opened, and nothing in it was changed."
    );
    assert!(!refused.is_handed_over(), "a refusal is a failed start, said and filed");
    assert_eq!(snapshot(&dir), before, "not even the library's lock was made in it");
}

#[test]
fn the_old_builds_lock_alone_keeps_this_build_out() {
    let paths = Paths::temporary(Notes::ID);
    std::fs::create_dir_all(paths.state_dir()).unwrap();
    // As the build before the framework held it: the same file, flocked.
    let old_build = yantrik_app::lock::hold(&paths.state_dir().join(LEGACY_NAME)).unwrap();
    assert_eq!(second_start(&paths).as_deref(), Some(ALREADY_RUNNING));
    drop(old_build);
    assert_eq!(second_start(&paths), None, "free once the old build is gone");
}

#[test]
fn the_vault_lock_alone_keeps_a_second_profile_on_the_same_vault_out() {
    let paths = Paths::temporary(Notes::ID);
    std::fs::create_dir_all(paths.data_dir()).unwrap();
    let other_profile = yantrik_app::lock::hold(&paths.data_dir().join(VAULT_NAME)).unwrap();
    assert_eq!(second_start(&paths).as_deref(), Some(ALREADY_RUNNING));
    drop(other_profile);
    assert_eq!(second_start(&paths), None);
}

#[test]
fn the_vault_lock_is_never_a_note() {
    let app = started();
    assert!(vault(&app).join(VAULT_NAME).exists(), "the lock lives in the vault");
    assert_eq!(state(&app)["note_count"], 0);
    app.run(crate::command::Command::Window(crate::command::WindowAction::Reload));
    settle(&app);
    assert_eq!(state(&app)["note_count"], 0, "nor after a reload");
    assert_eq!(state(&app)["notice"], "", "and not a file that could not be loaded either");
}

/// #978 L5: a launch handed over from a Mind's view is refused in the name the person knows the app
/// by, with a file or without, and nothing is applied.
#[test]
fn a_launch_from_a_minds_view_is_refused_as_notes_and_applies_nothing() {
    let app = TestApp::<Notes>::start();
    let profile = app.paths().profile().to_string();
    let said = "Notes cannot open in your view; use its actions and os_describe notes instead.";
    let empty = yantrik_app::Launch::empty(&profile);
    assert_eq!(app.launch_from_a_mind(empty).unwrap_err(), said);
    let with_file = yantrik_app::Launch::of_files(&profile, std::path::Path::new("/"), [std::path::PathBuf::from("/tmp/a.md")]);
    assert_eq!(app.launch_from_a_mind(with_file).unwrap_err(), said);
}
