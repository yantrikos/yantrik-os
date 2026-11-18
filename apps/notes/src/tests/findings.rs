//! What the reviews of #914 and #915 left for 8d-3: our last version held when it cannot be kept
//! (finding 5), our landed writes known after a Refresh or a reopening (6), the library's limit
//! leaving out the right note and saying the way out (7), no second copy of a version the
//! library already holds, text typed during a Trash job, a folder that refuses writes after it
//! was marked, and a write between the store's check and its replace kept (#916).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yantrik_app::app::App as _;
use yantrik_app::documents::disk::{Disk, DiskError, Displaced, Exchanged, RealDisk, Step};
use yantrik_app::documents::OutsideState;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{autosave, bar, failing, file, notice, outside, settle, state, unsaved, vault, with_note};
use crate::command::{Command, WindowAction};
use crate::update::said;
use crate::Notes;

const MINE: &str = "# Draft\n\nmy last version";

fn text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

fn kept_as(app: &TestApp<Notes>, prefix: &'static str) -> Vec<String> {
    app.model(move |m| m.library.iter().filter(|n| n.id.starts_with(prefix)).map(|n| n.content.text.clone()).collect())
}

/// A saved note whose last landed write is [`MINE`], and the older text of it another program
/// will write back.
fn saved_twice() -> (TestApp<Notes>, PathBuf, String) {
    let (app, path) = with_note("Draft");
    let stale = file(&path);
    app.run(Command::Edit(MINE.into()));
    autosave(&app);
    assert_eq!(file(&path), MINE);
    (app, path, stale)
}

/// Another program writes back a stale copy, and the watch looks (twice: an older write of ours
/// is made sure of before it is called a change).
fn written_back(app: &TestApp<Notes>, path: &Path, stale: &str) {
    std::fs::write(path, stale).unwrap();
    app.look_at_files();
    app.look_at_files();
    settle(app);
}

/// #915 finding 5: our last version could not be kept, so it is held as a conflict, and neither a
/// Refresh nor opening another note can drop it.
#[test]
fn a_last_version_that_cannot_be_kept_is_held_and_survives_a_refresh() {
    let (app, path, stale) = saved_twice();
    let disk = failing(&app);
    disk.fail(Step::Write, "pending.json", None);
    written_back(&app, &path, &stale);
    assert_eq!(text(&app), MINE, "not replaced");
    assert!(notice(&app).starts_with("This note changed on disk, but your last version could not be kept"));
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }), "{:?}", outside(&app));
    assert_eq!((bar(&app), unsaved(&app)), (1, true), "held as a conflict, the choices offered");
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!((text(&app), unsaved(&app)), (MINE.to_string(), true), "Refresh did not drop it");
    disk.heal();
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!(text(&app), MINE, "nor a Refresh once the disk is healed");
    app.run(Command::Window(WindowAction::New(None)));
    settle(&app);
    assert_eq!(text(&app), MINE, "nor opening another note");
    assert_eq!(file(&path), stale, "and nothing was saved over theirs");
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    settle(&app);
    assert_eq!(kept_as(&app, "copy-"), vec![MINE.to_string()], "the way out keeps it as a note");
}

/// #915 finding 6: a Refresh reopens the note as a fresh document; what it holds is still known
/// to be our last write, so a stale copy written back keeps it first.
#[test]
fn our_last_write_is_kept_from_a_stale_copy_after_a_refresh() {
    let (app, path, stale) = saved_twice();
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    written_back(&app, &path, &stale);
    assert_eq!(text(&app), stale, "the file is followed");
    assert_eq!(kept_as(&app, "recovered-"), vec![MINE.to_string()], "and my version kept first");
    assert_eq!(notice(&app), said::UPDATED_KEEPING_OURS);
}

#[test]
fn our_last_write_is_kept_from_a_stale_copy_after_reopening_the_note() {
    let (app, path, stale) = saved_twice();
    let id = app.model(|m| m.current().unwrap().id().to_string());
    app.run(Command::Window(WindowAction::New(Some("Elsewhere".into()))));
    settle(&app);
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    written_back(&app, &path, &stale);
    assert_eq!(text(&app), stale);
    assert_eq!(kept_as(&app, "recovered-"), vec![MINE.to_string()]);
}

/// The reviewer's guard on #915: a version a live note already holds is not kept again.
#[test]
fn a_version_the_library_already_holds_is_not_kept_twice() {
    let (app, path, stale) = saved_twice();
    let id = app.model(|m| m.current().unwrap().id().to_string());
    // A copy holds exactly my last version.
    app.run(Command::Window(WindowAction::Copy));
    settle(&app);
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    written_back(&app, &path, &stale);
    assert_eq!(text(&app), stale, "the file is followed");
    assert!(kept_as(&app, "recovered-").is_empty(), "no second copy");
    assert_eq!(kept_as(&app, "copy-"), vec![MINE.to_string()], "the one there is holds it");
    assert_eq!(notice(&app), said::UPDATED_FROM_DISK);
}

/// #917 review finding 1: the guard reads the copy's file, not the library as last loaded. A copy
/// another program deleted since holds nothing, so my last version is kept.
#[test]
fn a_copy_deleted_outside_notes_does_not_count_and_my_last_version_is_kept() {
    let (app, path, stale) = saved_twice();
    let id = app.model(|m| m.current().unwrap().id().to_string());
    app.run(Command::Window(WindowAction::Copy));
    settle(&app);
    let copy = app.model(|m| m.current().unwrap().id().to_string());
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    std::fs::remove_file(vault(&app).join(&copy)).unwrap();
    written_back(&app, &path, &stale);
    assert_eq!(text(&app), stale, "the file is followed");
    assert_eq!(kept_as(&app, "recovered-"), vec![MINE.to_string()], "and my version kept, the copy being gone");
    assert_eq!(notice(&app), said::UPDATED_KEEPING_OURS);
}

/// #915 finding 7: at the limit, a note a resolution or a recovery wrote is listed first, and the
/// sentence names the limit and the way out.
#[test]
fn at_the_limit_kept_notes_are_listed_first_and_the_way_out_is_said() {
    let paths = Paths::temporary(Notes::ID);
    let dir = paths.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..crate::store::MAX_NOTES {
        std::fs::write(dir.join(format!("note-{i:04}.md")), format!("# Note {i}\n")).unwrap();
    }
    std::fs::write(dir.join("recovered-0001.md"), "# Kept by a resolution\n").unwrap();
    let app = TestApp::<Notes>::start_at(paths, Vec::new());
    settle(&app);
    assert_eq!(
        notice(&app),
        "Library holds more than 2,000 notes; 1 not shown. Move some to the Trash, then Refresh."
    );
    let listed = app.model(|m| (m.library.len(), m.library.iter().any(|n| n.id == "recovered-0001.md")));
    assert_eq!(listed, (crate::store::MAX_NOTES, true), "the kept note is listed");
    let order = app.model(|m| m.library.windows(2).all(|w| w[0].id < w[1].id));
    assert!(order, "and the library is in file-name order, as it always was");
    assert!(dir.join("note-1999.md").exists(), "the one left out is untouched");
}

/// Deferred from #914: a key typed while the open note goes to the Trash is never closed away.
#[test]
fn text_typed_while_a_trash_job_runs_is_kept_as_a_new_note() {
    let (app, path) = with_note("Binned");
    app.run(Command::Window(WindowAction::Trash));
    assert_eq!(app.jobs_waiting(), 1, "the Trash job is in flight");
    app.run(Command::Edit("# Binned\n\ntyped while it went".into()));
    settle(&app);
    assert!(!path.exists(), "the note went to the Trash as its file held it");
    assert_eq!(notice(&app), said::TYPED_DURING_TRASH);
    let open = app.model(|m| m.current().map(|c| (c.id().to_string(), c.text().to_string())).unwrap());
    assert!(open.0.starts_with("recovered-"), "{}", open.0);
    assert_eq!(open.1, "# Binned\n\ntyped while it went");
    assert_eq!(file(&vault(&app).join(&open.0)), open.1, "and it is saved");
    assert_eq!(state(&app)["trash_count"], 1);
}

/// Deferred from #914: a library marked writable whose folder then refuses writes opens
/// read-only, with the reason, instead of failing as unavailable.
#[test]
fn a_folder_that_refuses_writes_after_it_was_marked_opens_read_only() {
    let (app, _) = with_note("Marked");
    let disk = failing(&app);
    disk.fail(Step::CreateTemp, ".write-check", None);
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!(
        notice(&app),
        "This library is open read-only: Notes cannot write in its folder (injected failure at CreateTemp). \
         Make the folder writable, then Refresh."
    );
    assert_eq!(state(&app)["library_ready"], true, "open to read, not unavailable");
    assert!(app.ui(|ui| ui.get_read_only()));
    disk.heal();
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    assert_eq!((notice(&app), app.ui(|ui| ui.get_read_only())), (String::new(), false), "writable again");
    let probes = std::fs::read_dir(vault(&app)).unwrap().flatten().filter(|e| {
        e.file_name().to_string_lossy().ends_with(".write-check")
    });
    assert_eq!(probes.count(), 0, "the check leaves nothing behind");
}

/// The same with a real folder made read-only. Root writes anywhere, so there it says so and
/// stops.
#[test]
fn a_real_read_only_folder_opens_read_only() {
    use std::os::unix::fs::PermissionsExt as _;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: running as root, which a read-only folder does not stop");
        return;
    }
    let (app, _) = with_note("Locked");
    let dir = vault(&app);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    app.run(Command::Window(WindowAction::Reload));
    settle(&app);
    let read_only = app.ui(|ui| ui.get_read_only());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(read_only, "{}", notice(&app));
    assert!(notice(&app).starts_with("This library is open read-only: Notes cannot write in its folder"));
}

/// A disk on which another program writes the note between the store's check and the replace.
struct WrittenBeforeTheExchange {
    note: PathBuf,
    theirs: &'static str,
}

impl Disk for WrittenBeforeTheExchange {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
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

    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        if path == self.note {
            std::fs::write(path, self.theirs).unwrap();
        }
        RealDisk.exchange(path, bytes)
    }
}

/// Whether the real disk can exchange in `dir` (Linux, on a filesystem with RENAME_EXCHANGE).
fn exchanges_in(dir: &Path) -> bool {
    let probe = dir.join(".exchange-probe");
    std::fs::write(&probe, b"old").unwrap();
    let done = RealDisk.exchange(&probe, b"new");
    let _ = std::fs::remove_file(&probe);
    matches!(done, Ok(Exchanged { displaced: Displaced::Bytes(_), .. }))
}

/// #915 finding 1, closed with #916: a write by another program between the store's check and
/// the replace is handed back by the exchange, kept as a recovered note, and said, whether or not
/// the watch happened to see it.
#[test]
fn a_write_between_the_check_and_the_replace_is_kept_and_said() {
    let (app, path) = with_note("Race");
    if !exchanges_in(&vault(&app)) {
        eprintln!("skipped: this filesystem has no atomic exchange, so what a replace displaces is not known");
        return;
    }
    let theirs = "# Race\n\ntheirs, written in the instant";
    app.run(Command::UseDisk(Arc::new(WrittenBeforeTheExchange { note: path.clone(), theirs })));
    app.run(Command::Edit("# Race\n\nmine".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Race\n\nmine", "ours is in place");
    assert_eq!(kept_as(&app, "recovered-"), vec![theirs.to_string()], "theirs is kept");
    let kept = app.model(|m| m.library.iter().find(|n| n.id.starts_with("recovered-")).unwrap().id.clone());
    assert_eq!(file(&vault(&app).join(kept)), theirs, "on disk");
    assert_eq!(notice(&app), said::theirs_kept("Race"));
    assert_eq!((outside(&app), unsaved(&app)), (OutsideState::Clean, false));
}

/// The guard on the exchange's side: another program's version that a live note already holds
/// byte for byte is not kept a second time, and the sentence names the note that holds it.
#[test]
fn a_replaced_version_the_library_already_holds_is_not_kept_twice() {
    let (app, path) = with_note("Race");
    if !exchanges_in(&vault(&app)) {
        eprintln!("skipped: this filesystem has no atomic exchange, so what a replace displaces is not known");
        return;
    }
    // As `new_note` writes it: the body ends with a line break.
    let theirs = "# Elsewhere\n\nthe same words\n";
    super::note(&app, "Elsewhere", "the same words");
    let id = path.file_name().unwrap().to_str().unwrap().to_string();
    app.run(Command::Window(WindowAction::Open { id, trash: false }));
    app.run(Command::UseDisk(Arc::new(WrittenBeforeTheExchange { note: path.clone(), theirs })));
    app.run(Command::Edit("# Race\n\nmine".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Race\n\nmine");
    assert!(kept_as(&app, "recovered-").is_empty(), "no second copy");
    assert_eq!(notice(&app), said::theirs_held("Elsewhere"));
}

/// A disk on which another program writes the note in the instant before the exchange, and on
/// which no new note can be made: the other program's version cannot be written anywhere.
struct WrittenAndFull {
    inner: WrittenBeforeTheExchange,
}

impl Disk for WrittenAndFull {
    fn write_atomically(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        self.inner.write_atomically(path, bytes)
    }

    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<(), DiskError> {
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("recovered-")) {
            return Err(DiskError { step: Step::CreateTemp, error: "No space left on device".into() });
        }
        self.inner.create_new(path, bytes)
    }

    fn read(&self, path: &Path) -> std::io::Result<Option<Vec<u8>>> {
        self.inner.read(path)
    }

    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.inner.remove(path)
    }

    fn exchange(&self, path: &Path, bytes: &[u8]) -> Result<Exchanged, DiskError> {
        self.inner.exchange(path, bytes)
    }
}

/// #917 review finding 2: a version that could not be written is kept as a draft of its own, so
/// the next start offers it, and the close prompt counts it while it is on no disk.
#[test]
fn a_replaced_version_that_cannot_be_written_is_kept_as_a_draft_and_counted_at_close() {
    let (app, path) = with_note("Race");
    if !exchanges_in(&vault(&app)) {
        eprintln!("skipped: this filesystem has no atomic exchange, so what a replace displaces is not known");
        return;
    }
    let theirs = "# Race\n\ntheirs, with nowhere to go";
    let inner = WrittenBeforeTheExchange { note: path.clone(), theirs };
    app.run(Command::UseDisk(Arc::new(WrittenAndFull { inner })));
    app.run(Command::Edit("# Race\n\nmine".into()));
    autosave(&app);
    assert_eq!(file(&path), "# Race\n\nmine");
    assert!(notice(&app).contains("its version could not be written (no space left on device)"), "{}", notice(&app));
    app.run_writes();
    let kept = app.model(|m| m.library.iter().find(|n| !n.stored).map(|n| n.id.clone())).expect("listed");
    assert!(super::draft_file(&app, &vault(&app).join(&kept)).exists(), "kept as a draft of its own");
    // Nothing unsaved in the open note, but the close prompt says what is on no disk.
    app.close_requested(yantrik_app::app::WindowId::PRIMARY);
    assert_eq!(app.ui(|ui| ui.get_dialog()), crate::update::closing::CLOSE_DIALOG);
    let said_now = app.ui(|ui| ui.get_close_said().to_string());
    assert!(said_now.starts_with("1 note in the list is not on disk yet."), "{said_now}");
    assert!(!app.ui(|ui| ui.get_close_saves()), "there is nothing for Save to save");
    app.ui(|ui| ui.invoke_action("close-discard".into()));
    let again = super::killed_and_started_again(app);
    // The store's journal of that write comes back as a recovered note at the start, so the text
    // is on disk once already. Its draft is not offered as well (#925 review, finding 3): Restore
    // would have written it a second time.
    let offered = again.model(|m| m.recovery.offers.iter().map(|o| o.content.text.clone()).collect::<Vec<_>>());
    let holding = again.model(move |m| m.library.iter().filter(|n| n.content.text == theirs && n.stored).count());
    assert_eq!((offered.len(), holding), (0, 1), "the next start has it once, on disk: {offered:?}");
    again.run_writes();
    assert!(!super::draft_file(&again, &vault(&again).join(&kept)).exists(), "and its draft is removed");
}

/// #917 review finding 6: a version a save kept is said even when the landing itself is one the
/// document no longer expects.
#[test]
fn a_kept_version_is_said_even_on_a_stale_landing() {
    use yantrik_app::documents::{Fingerprint, Witnessed};
    let (app, _) = with_note("Late");
    let doc = app.model(|m| m.current_id().unwrap());
    let kept = crate::note::Note {
        id: "recovered-late.md".into(),
        content: crate::note::NoteText { text: "# Late\n\ntheirs".into(), meta: String::new() },
        modified: 1,
        trash: false,
        stored: true,
    };
    let saved = crate::store::Saved {
        content: crate::note::NoteText { text: "# Late\n\n".into(), meta: String::new() },
        written: Fingerprint::of_bytes(b"# Late\n\n"),
        modified: 1,
        witnessed: Witnessed::Same,
        theirs: Some(Box::new(crate::store::Theirs::Kept(kept))),
    };
    app.run(Command::SaveLanded { doc, seq: 999, result: Ok(saved) });
    assert_eq!(notice(&app), said::theirs_kept("Late"));
    assert!(app.model(|m| m.library.iter().any(|n| n.id == "recovered-late.md")));
}

/// #917 delta review E: a conflict held because our last version could not be kept (finding 5)
/// survives a crash as a conflict. Its draft records what the edits were written over, not the
/// other program's version that `on_disk` holds while the conflict is held, so Restore after the
/// start never replaces theirs.
#[test]
fn a_held_conflict_comes_back_as_a_conflict_after_a_crash() {
    let (app, path, stale) = saved_twice();
    let disk = failing(&app);
    disk.fail(Step::Write, "pending.json", None);
    written_back(&app, &path, &stale);
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }), "held");
    disk.heal();
    app.run_writes();
    let app = super::killed_and_started_again(app);
    super::act(&app, "restore_draft", serde_json::json!({})).expect("restored");
    assert!(matches!(outside(&app), OutsideState::Conflict { .. }), "{:?}", outside(&app));
    assert_eq!(file(&path), stale, "the other program's version is untouched");
    assert_eq!(text(&app), MINE);
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    settle(&app);
    assert_eq!(kept_as(&app, "copy-"), vec![MINE.to_string()], "Keep mine keeps my version");
    assert_eq!(file(&path), stale);
}

/// The same when the file was gone by the time it was read: the draft comes back as the
/// deletion it was, offering Save as new, and never makes the file again unasked.
#[test]
fn a_held_deletion_comes_back_as_a_deletion_after_a_crash() {
    let (app, path, stale) = saved_twice();
    let disk = failing(&app);
    disk.fail(Step::Write, "pending.json", None);
    std::fs::write(&path, &stale).unwrap();
    app.look_at_files();
    app.look_at_files();
    assert_eq!(app.jobs_waiting(), 1, "the note is read again");
    std::fs::remove_file(&path).unwrap();
    settle(&app);
    assert_eq!(outside(&app), OutsideState::Deleted, "held as the deletion it is");
    disk.heal();
    app.run_writes();
    let app = super::killed_and_started_again(app);
    super::act(&app, "restore_draft", serde_json::json!({})).expect("restored");
    assert_eq!(outside(&app), OutsideState::Deleted, "{:?}", outside(&app));
    assert!(!path.exists(), "the file is not made again");
    assert_eq!(bar(&app), 2, "Save as new is offered");
    // #917's review, F: said as what it is, one version and its file gone, not two to choose between.
    assert_eq!(super::notice(&app), crate::update::said::RESTORED_GONE);
    app.ui(|ui| ui.invoke_action("keep-mine".into()));
    settle(&app);
    assert_eq!(kept_as(&app, "copy-"), vec![MINE.to_string()], "and keeps my version as a new note");
    assert!(!path.exists());
}

