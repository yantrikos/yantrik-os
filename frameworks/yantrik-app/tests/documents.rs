//! Documents through the real core (design/notes-gate-2026-10-21.md §2): a toy editor opens a
//! real file, saves it through a job over the disk seam, and hears its file change through the
//! watch. Our own save is not an outside edit; another program's write to a clean document is
//! offered; one to a dirty document is a conflict that blocks saving; closing unwatches.

use std::path::PathBuf;

use yantrik_app::app::{App, Boot, Changed, Event, Startup, WindowId};
use yantrik_app::cx::Cx;
use yantrik_app::documents::disk::{Disk, RealDisk};
use yantrik_app::documents::{
    BeginSave, DocumentId, Documents, Durability, Fingerprint, Observed, OutsideState, SaveError,
    SaveOutcome, SaveTicket, Source,
};
use yantrik_app::effects::Effects;
use yantrik_app::refused::Refused;
use yantrik_testing::TestApp;

slint::slint! {
    export component PadWin inherits Window {}
}

impl yantrik_app::Window for PadWin {
    fn make() -> Result<Self, slint::PlatformError> {
        PadWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

#[derive(Default)]
struct Model {
    docs: Documents<String>,
    open: Option<DocumentId>,
    log: Vec<String>,
}

#[derive(Debug, Clone)]
enum Command {
    Open(PathBuf),
    Type(String),
    Save,
    Landed { seq: u64, result: Result<Fingerprint, SaveError> },
    Seen { key: String, seen: Observed },
    Close,
}

struct Pad;

/// The save job: the file must still be what the document knows is there, then the bytes go
/// in whole or not at all.
fn write(path: PathBuf, ticket: SaveTicket<String>) -> Result<Fingerprint, SaveError> {
    if !ticket.may_replace(Fingerprint::of(&path).as_ref()) {
        return Err(SaveError::conflict("The file changed on disk."));
    }
    let written = Fingerprint::of_bytes(ticket.content.as_bytes());
    RealDisk
        .write_atomically(&path, ticket.content.as_bytes())
        .map_err(|e| SaveError::after_writing(e.step.durability(), written.clone(), e.to_string()))?;
    Ok(written)
}

/// The document's watch, armed again from its baseline.
fn rearm(fx: &mut Effects<Pad>, entry: Option<(String, PathBuf, Option<Observed>)>) {
    if let Some((key, path, seen)) = entry {
        fx.watch(&key, path, seen);
    }
}

impl App for Pad {
    type Ui = PadWin;
    type Model = Model;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "pad-test";

    fn model(_: &Boot) -> Result<Model, Startup> {
        Ok(Model::default())
    }

    fn wire(_: &Cx<Self>) {}

    fn update(m: &mut Model, command: Command, fx: &mut Effects<Self>) -> Result<(), Refused> {
        match command {
            Command::Open(path) => {
                let text = std::fs::read_to_string(&path).map_err(|_| Refused::new("unreadable", "Cannot read it."))?;
                let baseline = Fingerprint::of_bytes(text.as_bytes());
                let id = m.docs.open(Source::File(path), text, Some(baseline));
                m.docs.show(id, WindowId::PRIMARY);
                m.open = Some(id);
                let (key, path, seen) = m.docs.get(id).and_then(|d| d.watch_entry()).expect("a file");
                fx.watch(&key, path, seen);
            }
            Command::Type(text) => {
                let d = m.open.and_then(|id| m.docs.get_mut(id)).ok_or(Refused::new("none", "Nothing open."))?;
                d.edit(|c| c.push_str(&text));
            }
            Command::Save => {
                let d = m.open.and_then(|id| m.docs.get_mut(id)).ok_or(Refused::new("none", "Nothing open."))?;
                let path = d.source.path().expect("a file").to_path_buf();
                match d.begin_save() {
                    BeginSave::Ticket(t) => {
                        let seq = t.seq;
                        fx.spawn(move |_| Command::Landed { seq, result: write(path, t) });
                    }
                    BeginSave::Blocked => {
                        return Err(Refused::new("conflict", "The file changed; choose first."))
                    }
                    other => m.log.push(format!("save: {}", label(&other))),
                }
            }
            Command::Landed { seq, result } => {
                let d = m.open.and_then(|id| m.docs.get_mut(id)).expect("open");
                let outcome = d.save_landed(seq, result);
                m.log.push(format!("landed: {}", outcome_label(&outcome)));
                // From the new baseline: a look taken before the save is dropped, and our own
                // write is never reported.
                if matches!(outcome, SaveOutcome::Saved { .. }) {
                    rearm(fx, d.watch_entry());
                }
            }
            Command::Seen { key, seen } => {
                let Some(id) = m.docs.by_key(&key) else { return Ok(()) };
                let d = m.docs.get_mut(id).expect("open");
                let state = d.observe(seen).clone();
                m.log.push(format!("outside: {}", outside_label(&state)));
                rearm(fx, d.rewatch());
            }
            Command::Close => {
                if let Some(id) = m.open.take() {
                    let d = m.docs.close_document(id).expect("open");
                    fx.unwatch(&d.source.key());
                }
            }
        }
        Ok(())
    }

    fn render(_: &PadWin, _: WindowId, _: &Model, _: &Changed) {}

    fn event(event: Event<()>, _: &Model) -> Option<Command> {
        match event {
            Event::FileChanged { key, seen } => Some(Command::Seen { key, seen }),
            _ => None,
        }
    }
}

fn label(b: &BeginSave<String>) -> &'static str {
    match b {
        BeginSave::Ticket(_) => "ticket",
        BeginSave::Clean => "clean",
        BeginSave::InFlight => "in flight",
        BeginSave::Failed => "failed",
        BeginSave::Blocked => "blocked",
    }
}

fn outcome_label(o: &SaveOutcome<String>) -> &'static str {
    match o {
        SaveOutcome::Saved { replaced: Some(_), .. } => "replaced",
        SaveOutcome::Saved { follow_up: Some(_), .. } => "follow-up",
        SaveOutcome::Saved { .. } => "saved",
        SaveOutcome::Failed => "failed",
        SaveOutcome::Stale => "stale",
    }
}

fn outside_label(s: &OutsideState) -> &'static str {
    match s {
        OutsideState::Clean => "clean",
        OutsideState::Changed { .. } => "changed",
        OutsideState::Conflict { .. } => "conflict",
        OutsideState::Deleted => "deleted",
    }
}

/// A pad with `note.txt` holding `text`, opened.
fn pad_with(text: &str) -> (TestApp<Pad>, PathBuf) {
    let app = TestApp::<Pad>::start();
    let dir = app.paths().data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("note.txt");
    std::fs::write(&path, text).unwrap();
    app.run(Command::Open(path.clone()));
    (app, path)
}

fn log(app: &TestApp<Pad>) -> Vec<String> {
    app.model(|m| m.log.clone())
}

fn outside(app: &TestApp<Pad>) -> &'static str {
    app.model(|m| outside_label(&m.docs.get(m.open.unwrap()).unwrap().outside))
}

#[test]
fn opening_watches_the_file_and_the_file_it_was_read_from_is_not_a_change() {
    let (app, path) = pad_with("hello");
    let watched = app.watched();
    assert_eq!(watched.len(), 1);
    assert_eq!(watched[0].1, path);
    app.look_at_files();
    assert!(log(&app).is_empty(), "{:?}", log(&app));
}

#[test]
fn our_own_save_seen_by_the_watch_is_not_an_outside_edit() {
    let (app, path) = pad_with("hello");
    app.run(Command::Type(" world".into()));
    app.run(Command::Save);
    app.run_jobs();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    app.look_at_files();
    assert_eq!(log(&app), vec!["landed: saved"], "the watch, re-armed, does not report our write");
    assert!(!app.model(|m| m.docs.get(m.open.unwrap()).unwrap().dirty()));
}

#[test]
fn another_programs_write_to_a_clean_document_is_offered() {
    let (app, path) = pad_with("hello");
    std::fs::write(&path, "rewritten elsewhere").unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), "changed");
}

#[test]
fn another_programs_write_to_a_dirty_document_is_a_conflict_that_blocks_saving() {
    let (app, path) = pad_with("hello");
    app.run(Command::Type(" mine".into()));
    std::fs::write(&path, "theirs").unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), "conflict");
    app.run(Command::Save);
    assert_eq!(app.last_refusal().map(|r| r.kind), Some("conflict"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs", "never last-writer-wins");
}

#[test]
fn a_save_that_finds_the_file_changed_before_the_watch_did_is_a_conflict() {
    let (app, path) = pad_with("hello");
    app.run(Command::Type(" mine".into()));
    app.run(Command::Save);
    std::fs::write(&path, "theirs").unwrap();
    app.run_jobs();
    assert_eq!(log(&app), vec!["landed: failed"]);
    assert_eq!(outside(&app), "conflict");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs");
    let failed = app.model(|m| match &m.docs.get(m.open.unwrap()).unwrap().save {
        yantrik_app::documents::SaveState::Failed { error, .. } => error.durability,
        other => panic!("{other:?}"),
    });
    assert_eq!(failed, Durability::NotWritten);
}

#[test]
fn a_deleted_file_is_said() {
    let (app, path) = pad_with("hello");
    std::fs::remove_file(&path).unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), "deleted");
}

#[test]
fn closing_the_document_unwatches_its_file() {
    let (app, path) = pad_with("hello");
    app.run(Command::Close);
    assert!(app.watched().is_empty());
    std::fs::write(&path, "changed after close").unwrap();
    app.look_at_files();
    assert!(log(&app).is_empty());
}

#[test]
fn an_outside_revert_to_an_older_save_of_ours_is_caught_on_the_second_look() {
    let (app, path) = pad_with("v0");
    for more in [" a", " b"] {
        app.run(Command::Type(more.into()));
        app.run(Command::Save);
        app.run_jobs();
    }
    // A sync client puts our first save back.
    std::fs::write(&path, "v0 a").unwrap();
    app.look_at_files();
    assert_eq!(outside(&app), "clean", "a late look or a revert: not trusted yet");
    app.look_at_files();
    assert_eq!(outside(&app), "changed", "still there on the re-armed look: a revert");
}

#[test]
fn a_look_taken_before_the_watch_was_re_armed_is_dropped() {
    let (app, _path) = pad_with("hello");
    let (key, _) = app.watched().pop().expect("watched");
    // A sighting under an arming that is not the current one: queued behind a save's outcome.
    app.seen(yantrik_app::watch::Seen::File { key, seen: Observed::File(Fingerprint::of_bytes(b"x")), arming: 0 });
    assert!(log(&app).is_empty(), "{:?}", log(&app));
    assert_eq!(outside(&app), "clean");
}
