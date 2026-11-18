//! A note with nothing unsaved follows its file (decided in review of #915): another program's
//! change is read again and shown, said in one quiet line, and the undo history starts again. A
//! write of ours it replaces is kept as a recovered note first; a key typed before the read lands
//! makes it a conflict; a change seen while the store works is read once it is free.

use std::path::PathBuf;

use serde_json::json;
use yantrik_app::app::App as _;
use yantrik_app::documents::OutsideState;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{autosave, bar, file, notice, outside, settle, state, unsaved, vault, with_note};
use crate::command::{Command, WindowAction};
use crate::update::said;
use crate::Notes;

fn text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

fn recovered(app: &TestApp<Notes>) -> Vec<String> {
    app.model(|m| {
        m.library.iter().filter(|n| n.id.starts_with("recovered-")).map(|n| n.content.text.clone()).collect()
    })
}

/// A note that was on disk before Notes started, open, with nothing of ours written to it.
fn opened_from_disk(text: &str) -> (TestApp<Notes>, PathBuf) {
    let paths = Paths::temporary(Notes::ID);
    std::fs::create_dir_all(paths.data_dir()).unwrap();
    let path = paths.data_dir().join("from-disk.md");
    std::fs::write(&path, text).unwrap();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    app.run(Command::Window(WindowAction::Open { id: "from-disk.md".into(), trash: false }));
    (app, path)
}

#[test]
fn a_saved_note_changed_outside_is_read_again_and_shown() {
    let (app, path) = opened_from_disk("# Clean\n\nas found\n");
    std::fs::write(&path, "# Clean\n\nfrom elsewhere\n").unwrap();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the note is read again");
    settle(&app);
    assert_eq!(app.ui(|ui| ui.get_content().to_string()), "# Clean\n\nfrom elsewhere\n", "and shown");
    assert_eq!(notice(&app), said::UPDATED_FROM_DISK);
    assert_eq!((outside(&app), bar(&app), unsaved(&app)), (OutsideState::Clean, 0, false));
    assert_eq!(state(&app)["outside"], "clean");
    assert!(recovered(&app).is_empty(), "nothing of ours was replaced, so nothing is kept");
    // The watch goes on from the file as read: another look is quiet, and the next save lands.
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 0);
    app.run(Command::Edit("# Clean\n\nfrom elsewhere\nand here".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Clean\n\nfrom elsewhere\nand here");
}

/// Review finding 2: another program writing back the copy it opened before our save would leave
/// the person's last version nowhere, undo included. It is kept as a recovered note first.
#[test]
fn a_saved_note_overwritten_from_a_stale_copy_keeps_my_last_version() {
    let (app, path) = with_note("Draft");
    let stale = file(&path);
    app.run(Command::Edit("# Draft\n\nmy last version".into()));
    autosave(&app);
    std::fs::write(&path, &stale).unwrap();
    // That is an older write of ours, so the watch makes sure before calling it a change.
    app.look_at_files();
    app.look_at_files();
    settle(&app);
    assert_eq!(text(&app), stale, "the file is followed");
    assert_eq!(notice(&app), said::UPDATED_KEEPING_OURS);
    assert_eq!(recovered(&app), vec!["# Draft\n\nmy last version".to_string()], "and my version kept");
    let kept = app.model(|m| m.library.iter().find(|n| n.id.starts_with("recovered-")).unwrap().id.clone());
    assert_eq!(file(&vault(&app).join(kept)), "# Draft\n\nmy last version", "on disk");
    app.run(Command::Window(WindowAction::Undo));
    assert_eq!(text(&app), stale, "the undo history starts again, as for Take theirs");
}

/// Never replaced once something is typed: a key between the change and the read landing makes
/// it the conflict it is, and the person chooses.
#[test]
fn a_key_typed_before_the_reread_lands_makes_it_a_conflict_and_nothing_is_lost() {
    let (app, path) = with_note("Clean");
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the read is in flight");
    app.run(Command::Edit("# Clean\n\ntyped".into()));
    settle(&app);
    assert_eq!(text(&app), "# Clean\n\ntyped", "what was typed is not replaced");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    assert_eq!(bar(&app), 1, "the choices are offered");
    assert!(notice(&app).is_empty(), "nothing was updated: {}", notice(&app));
    autosave(&app);
    assert_eq!(file(&path), "theirs", "and nothing is saved over theirs");
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    settle(&app);
    let copy = app.model(|m| m.current().unwrap().id().to_string());
    assert_eq!(file(&vault(&app).join(copy)), "# Clean\n\ntyped");
    assert_eq!(file(&path), "theirs");
}

/// A change seen while the store works on something else is read once the store is free.
#[test]
fn a_change_seen_while_the_store_works_is_read_again_once_it_is_done() {
    let (app, path) = opened_from_disk("# Clean\n\n");
    let out = vault(&app).parent().unwrap().join(format!("exported-{}.md", std::process::id()));
    let export = app.act("export", json!({ "path": out.display().to_string() })).unwrap();
    assert_eq!(app.jobs_waiting(), 1, "the export is in flight");
    std::fs::write(&path, "# Clean\n\nfrom elsewhere\n").unwrap();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the read waits for the store");
    settle(&app);
    export.settle().expect("the export is answered");
    assert_eq!(file(&out), "# Clean\n\n");
    assert_eq!(text(&app), "# Clean\n\nfrom elsewhere\n");
    assert_eq!(notice(&app), said::UPDATED_FROM_DISK);
    assert_eq!(outside(&app), OutsideState::Clean);
}

/// Review finding 4: a lasting notice replaced by an update is the update's, and goes with the
/// next save as any notice does.
#[test]
fn an_update_takes_the_place_of_a_lasting_notice() {
    let (app, path) = super::outside::replaced_once();
    assert!(app.model(|m| m.lasting_notice), "the replaced write is said, lastingly");
    std::fs::write(&path, "# Race\n\nfrom elsewhere\n").unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(notice(&app), said::UPDATED_KEEPING_OURS);
    assert!(!app.model(|m| m.lasting_notice));
    app.run(Command::Edit("# Race\n\nfrom elsewhere\nmore".into()));
    autosave(&app);
    assert_eq!(notice(&app), "", "the next save takes it back");
}
