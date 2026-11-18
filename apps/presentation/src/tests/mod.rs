//! yPresent under test: headless through yantrik-testing, with its drafts under the test's own
//! root, the folder a nameless deck is saved in outside the home, files in scratch folders, jobs
//! run when the test says, and the disk made to fail where a test asks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::documents::disk::FaultyDisk;
use yantrik_testing::{Call, TestApp};

use crate::command::{Command, WindowAction};
use crate::deck::{encode, parse, Content, Slide};
use crate::YPresent;

mod agents;
mod answers;
mod closing;
mod files;
mod launch;
mod lingering;
mod mind_view;
mod notes;
mod outside;
mod parity;
mod private_mode;
mod recovery;
mod rescue;
mod saving;
mod shots;
mod untrusted;
mod window;

/// yPresent, started and settled, saving nameless decks into a folder of the test's own.
pub fn started() -> TestApp<YPresent> {
    let app = TestApp::<YPresent>::start();
    deck_folder(&app);
    settle(&app);
    app
}

/// The folder a deck with no name is saved in, and the Open button looks in: a scratch folder,
/// never the person's `~/Documents/Presentations`.
pub fn deck_folder(app: &TestApp<YPresent>) -> PathBuf {
    let dir = scratch("decks");
    app.run(Command::UseDeckDir(dir.clone()));
    dir
}

/// Runs jobs until none is left: a landing may start its follow-up, and so on.
pub fn settle(app: &TestApp<YPresent>) {
    for _ in 0..32 {
        if app.jobs_waiting() == 0 {
            return;
        }
        app.run_jobs();
    }
    panic!("jobs kept starting jobs");
}

/// One call through the real dispatch, as the person, answered once its work has landed.
pub fn act(app: &TestApp<YPresent>, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act(name, args)?;
    settle(app);
    pending.settle()
}

/// The same call, as an agent with this token.
pub fn act_as_agent(app: &TestApp<YPresent>, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act_as(Call::new(name, args).as_agent(token).granted())?;
    settle(app);
    pending.settle()
}

/// What `describe` says now, its state object.
pub fn state(app: &TestApp<YPresent>) -> Value {
    app.describe()["state"].clone()
}

pub fn summary(app: &TestApp<YPresent>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

pub fn notice(app: &TestApp<YPresent>) -> String {
    app.model(|m| m.notice.window().to_string())
}

/// The slide titles, in order.
pub fn outline(app: &TestApp<YPresent>) -> Vec<String> {
    app.model(|m| m.deck().outline())
}

/// The selected slide.
pub fn current(app: &TestApp<YPresent>) -> usize {
    app.model(|m| m.deck().current())
}

pub fn modified(app: &TestApp<YPresent>) -> bool {
    app.model(|m| m.modified())
}

pub fn path_of(app: &TestApp<YPresent>) -> Option<PathBuf> {
    app.model(|m| m.path().map(Path::to_path_buf))
}

/// What the canvas holds now.
pub fn canvas(app: &TestApp<YPresent>) -> crate::deck::Edits {
    app.ui(crate::view::canvas)
}

/// The person pressed a window action, with the canvas as it was.
pub fn press(app: &TestApp<YPresent>, action: WindowAction) {
    let canvas = canvas(app);
    app.run(Command::Window { action, canvas });
    settle(app);
}

/// The person typed a title into the canvas: the field holds it, and says so.
pub fn type_title(app: &TestApp<YPresent>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.set_current_title(text.clone().into());
        ui.invoke_title_edited(text.into());
    });
}

pub fn type_body(app: &TestApp<YPresent>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.set_current_body(text.clone().into());
        ui.invoke_body_edited(text.into());
    });
}

pub fn type_notes(app: &TestApp<YPresent>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.set_current_notes(text.clone().into());
        ui.invoke_notes_edited(text.into());
    });
}

/// A scratch folder of its own, outside the home, every link in its path resolved.
pub fn scratch(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("yantrik-ypresent-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

/// A deck of these slide titles (each with a body "body of <title>"), as a `.ydeck` file holds it.
pub fn content_of(title: &str, slides: &[&str]) -> Content {
    Content {
        title: title.into(),
        theme: 0,
        slides: slides.iter().map(|t| Slide::new(*t, format!("body of {t}"), 1)).collect(),
    }
}

/// The text of a deck file.
pub fn deck_text(title: &str, slides: &[&str]) -> String {
    encode(&content_of(title, slides))
}

/// A deck file of these slides at `path`.
pub fn write_deck(path: &Path, title: &str, slides: &[&str]) {
    std::fs::write(path, deck_text(title, slides)).unwrap();
}

/// yPresent with a saved deck open, and the file.
pub fn with_file(name: &str, slides: &[&str]) -> (TestApp<YPresent>, PathBuf) {
    let app = started();
    let path = scratch(name).join("talk.ydeck");
    write_deck(&path, "Talk", slides);
    act(&app, "open", json!({ "path": path.display().to_string() })).expect("the deck opens");
    assert_eq!(path_of(&app), Some(path.clone()), "{}", notice(&app));
    (app, path)
}

pub fn file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The deck a file holds.
pub fn read_deck(path: &Path) -> Content {
    parse(&file(path)).expect("a deck")
}

/// The names in a folder, sorted.
pub fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Saves go through `disk` from now on.
pub fn failing(app: &TestApp<YPresent>) -> Arc<FaultyDisk> {
    let disk = Arc::new(FaultyDisk::new());
    app.run(Command::UseDisk(disk.clone()));
    disk
}

/// Where the deck's draft is kept.
pub fn draft_file(app: &TestApp<YPresent>) -> PathBuf {
    app.model(|m| m.draft_path())
}

/// The same yPresent, started again at its files after it was dropped with no shutdown: a crash,
/// a `kill -9`. Nothing it had not handed the disk survives.
pub fn killed_and_started_again(app: TestApp<YPresent>) -> TestApp<YPresent> {
    let paths = app.paths();
    drop(app);
    let again = TestApp::<YPresent>::start_at(paths, Vec::new());
    deck_folder(&again);
    settle(&again);
    again
}

/// A file changed outside yPresent, and the watch's next look.
pub fn outside_writes(app: &TestApp<YPresent>, path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    app.look_at_files();
    settle(app);
}

/// `{}` as a call's arguments.
pub fn none() -> Value {
    json!({})
}
