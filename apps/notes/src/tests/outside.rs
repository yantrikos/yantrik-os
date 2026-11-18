//! Gate row "Conflicting outside edit" (design/notes-gate-2026-10-21.md §3): a change during an
//! in-flight save in each of the three orders it can meet the save in, before the store's check
//! (the save loses), between the check and the rename (the save wins, and says so), after the
//! rename (a change to what was saved), and a change to the sidecar alone. A note with nothing
//! unsaved, which follows its file, is follow.rs's; detection before the next save, the store's
//! check with the watch off, and our own save are in saving.rs.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yantrik_app::app::Event;
use yantrik_app::cx::Inbound;
use yantrik_app::documents::disk::{Disk, DiskError, RealDisk};
use yantrik_app::documents::{Fingerprint, Observed, OutsideState, Replaced};

use yantrik_testing::TestApp;

use super::{act, autosave, bar, file, not_saved_over, notice, outside, settle, unsaved, with_note};
use crate::command::{Command, WindowAction};
use crate::update::{said, AUTOSAVE_AFTER};
use crate::Notes;

#[test]
fn a_change_before_the_stores_check_wins_and_the_save_becomes_a_conflict() {
    let (app, path) = with_note("Race");
    app.run(Command::Edit("# Race\n\nmine".into()));
    app.advance(AUTOSAVE_AFTER);
    assert_eq!(app.jobs_waiting(), 1, "the save is in flight");
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), OutsideState::Clean, "seen during the save, judged when it lands");
    settle(&app);
    assert_eq!(outside(&app), OutsideState::Conflict { now: Some(Fingerprint::of_bytes(b"theirs")) });
    assert_eq!(file(&path), "theirs", "never written over");
    assert!(unsaved(&app), "my text is kept as edits");
    assert_eq!((notice(&app), bar(&app)), (not_saved_over(), 1));
}

/// A disk on which another program writes the note after the store's check and before our
/// rename, and the watch sees it in that moment: the one order the check cannot catch.
struct WrittenBeforeOurRename {
    note: PathBuf,
    theirs: &'static str,
    /// What the watch thread posts on seeing the file; the write and the sighting happen once.
    watch_sees: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Disk for WrittenBeforeOurRename {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        if path == self.note {
            if let Some(sighting) = self.watch_sees.lock().unwrap().take() {
                std::fs::write(path, self.theirs).unwrap();
                sighting();
            }
        }
        RealDisk.write_atomically(path, bytes)
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        RealDisk.create_new(path, bytes)
    }

    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        RealDisk.read(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        RealDisk.remove(path)
    }
}

/// A saved note whose next save replaced another program's write between the store's check and
/// its rename, with the watch seeing that write in that moment; the save has landed.
pub(super) fn replaced_once() -> (TestApp<Notes>, PathBuf) {
    let (app, path) = with_note("Race");
    let (key, _) = app.watched().remove(0);
    let theirs = "# Race\n\ntheirs";
    let post = app.post();
    // Posted from inside the save job, as the watch thread would: it reaches the app before the
    // save's own outcome does.
    let sighting = move || {
        let seen = Observed::File(Fingerprint::of_bytes(theirs.as_bytes()));
        post.post(Inbound::Event(Event::FileChanged { key, seen })).expect("the app is running");
    };
    app.run(Command::UseDisk(Arc::new(WrittenBeforeOurRename {
        note: path.clone(),
        theirs,
        watch_sees: Mutex::new(Some(Box::new(sighting))),
    })));
    app.run(Command::Edit("# Race\n\nmine".into()));
    autosave(&app);
    (app, path)
}

#[test]
fn a_change_between_the_check_and_the_rename_is_replaced_and_said() {
    let (app, path) = replaced_once();
    assert_eq!(file(&path), "# Race\n\nmine", "ours landed over theirs");
    assert_eq!(notice(&app), said::replaced(Replaced::OutsideEdit), "and the person is told");
    assert_eq!((outside(&app), bar(&app), unsaved(&app)), (OutsideState::Clean, 0, false));
    // Still said while the person types on and the next save lands.
    app.run(Command::Edit("# Race\n\nmine, more".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Race\n\nmine, more");
    assert_eq!(notice(&app), said::replaced(Replaced::OutsideEdit));
    // The person's next action takes it back.
    app.run(Command::Window(WindowAction::Save));
    assert_eq!(notice(&app), "");
}

#[test]
fn a_change_after_the_rename_is_a_change_to_what_was_saved() {
    let (app, path) = with_note("Race");
    app.run(Command::Edit("# Race\n\nv1".into()));
    app.advance(AUTOSAVE_AFTER);
    // Typed on during the save: one follow-up is owed.
    app.run(Command::Edit("# Race\n\nv2".into()));
    app.advance(AUTOSAVE_AFTER);
    assert!(app.run_one_job(), "the first save lands, and its follow-up starts");
    assert_eq!(file(&path), "# Race\n\nv1");
    assert_eq!(app.jobs_waiting(), 1);
    // The other program writes after our rename, and the watch sees it while the follow-up runs.
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    settle(&app);
    assert_eq!(file(&path), "theirs", "the follow-up does not save over it");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }));
    assert_eq!((notice(&app), bar(&app)), (not_saved_over(), 1), "a conflict, not a replaced write");
    assert_eq!(app.model(|m| m.current().unwrap().text().to_string()), "# Race\n\nv2", "v2 is kept");
}

/// The watch reads the text file only; the store's check reads the sidecar too.
#[test]
fn a_change_to_the_sidecar_alone_is_caught_by_the_stores_check_and_resolved_like_any_other() {
    let (app, path) = with_note("Tagged");
    app.run(Command::Metadata { notebook: String::new(), tags: "mine".into() });
    autosave(&app);
    let meta = path.with_extension("meta");
    std::fs::write(&meta, "tags:theirs\n").unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), OutsideState::Clean, "nothing the watch looks at changed");
    app.run(Command::Edit("# Tagged\n\nmine".into()));
    autosave(&app);
    assert_eq!(std::fs::read_to_string(&meta).unwrap(), "tags:theirs\n", "the sidecar is not written over");
    assert_eq!(file(&path), "# Tagged\n\n", "nor the text beside it");
    assert_eq!((notice(&app), bar(&app)), (not_saved_over(), 1));
    // Compare shows where the two differ: the sidecars as well as the texts.
    let compared = act(&app, "compare", serde_json::json!({})).expect("compare");
    assert_eq!(compared["theirs_metadata"], "tags:theirs\n");
    assert_eq!(compared["mine_metadata"], "notebook:\ntags:mine\n");
    let fields = app.ui(|ui| (ui.get_mine_fields().to_string(), ui.get_theirs_fields().to_string()));
    assert_eq!(fields, ("Tags: mine".to_string(), "Tags: theirs".to_string()));
    // Take theirs: the file's sidecar comes in; my text and tags are kept as a note of their own.
    app.run(Command::Window(WindowAction::TakeTheirs));
    settle(&app);
    let (text, tags) = app.model(|m| {
        let c = m.current().unwrap();
        (c.text().to_string(), c.content().field("tags"))
    });
    assert_eq!((text.as_str(), tags.as_str()), ("# Tagged\n\n", "theirs"));
    let kept = app.model(|m| m.library.iter().find(|n| n.id.starts_with("recovered-")).cloned());
    let kept = kept.expect("my version is kept");
    assert_eq!((kept.content.text.as_str(), kept.field("tags").as_str()), ("# Tagged\n\nmine", "mine"));
    assert!(!unsaved(&app));
}
