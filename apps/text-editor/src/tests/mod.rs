//! The Text Editor under test: headless through yantrik-testing, with its drafts under the test's
//! own root, jobs run when the test says, files in scratch folders outside the home, and the disk
//! made to fail where a test asks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use yantrik_app::documents::disk::FaultyDisk;
use yantrik_testing::{Call, TestApp};

use crate::command::{Command, WindowAction};
use crate::TextEditor;

mod agents;
mod big;
mod closing;
mod findings;
mod launch;
mod lingering;
mod mind_view;
mod outside;
mod parity;
mod reading;
mod recovery;
mod saving;
mod surface;
mod window;

/// An editor, started and settled.
pub fn started() -> TestApp<TextEditor> {
    let app = TestApp::<TextEditor>::start();
    settle(&app);
    app
}

/// Runs jobs until none is left: a landing may start its read-back, and so on.
pub fn settle(app: &TestApp<TextEditor>) {
    for _ in 0..32 {
        if app.jobs_waiting() == 0 {
            return;
        }
        app.run_jobs();
    }
    panic!("jobs kept starting jobs");
}

/// One call through the real dispatch, as the person, answered once its work has landed.
pub fn act(app: &TestApp<TextEditor>, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act(name, args)?;
    settle(app);
    pending.settle()
}

/// The same call, as an agent with this token.
pub fn act_as_agent(app: &TestApp<TextEditor>, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act_as(Call::new(name, args).as_agent(token).granted())?;
    settle(app);
    pending.settle()
}

/// What `describe` says now, its state object.
pub fn state(app: &TestApp<TextEditor>) -> Value {
    app.describe()["state"].clone()
}

pub fn summary(app: &TestApp<TextEditor>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

pub fn notice(app: &TestApp<TextEditor>) -> String {
    app.model(|m| m.notice.window().to_string())
}

/// The tab in front's whole text.
pub fn text(app: &TestApp<TextEditor>) -> String {
    app.model(|m| m.active_text().to_string())
}

pub fn tabs(app: &TestApp<TextEditor>) -> usize {
    app.model(|m| m.tabs.len())
}

pub fn modified(app: &TestApp<TextEditor>) -> bool {
    app.model(|m| m.modified(m.active_tab()))
}

/// The person pressed a window action.
pub fn press(app: &TestApp<TextEditor>, action: WindowAction) {
    app.run(Command::Window(action));
    settle(app);
}

/// The person typed: the editor field now holds `text`.
pub fn typed(app: &TestApp<TextEditor>, text: &str) {
    app.run(Command::Edit(text.into()));
}

/// A scratch folder of its own, outside the home, every link in its path resolved.
pub fn scratch(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("yantrik-editor-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

/// A file at `path` holding `text`, opened as the person through the Open dialog.
pub fn opened(app: &TestApp<TextEditor>, path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    press(app, WindowAction::Open);
    press(app, WindowAction::Confirm { path: path.display().to_string() });
    assert_eq!(app.model(|m| m.path(m.active_tab()).map(Path::to_path_buf)), Some(path.to_path_buf()), "{}", notice(app));
}

/// An editor with a saved file open, and the file.
pub fn with_file(name: &str, text: &str) -> (TestApp<TextEditor>, PathBuf) {
    let app = started();
    let path = scratch(name).join("notes.txt");
    opened(&app, &path, text);
    (app, path)
}

pub fn file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// Saves go through `disk` from now on.
pub fn failing(app: &TestApp<TextEditor>) -> Arc<FaultyDisk> {
    let disk = Arc::new(FaultyDisk::new());
    app.run(Command::UseDisk(disk.clone()));
    disk
}

/// Where the draft of the tab in front is kept.
pub fn draft_file(app: &TestApp<TextEditor>) -> PathBuf {
    let name = app.model(|m| m.active_tab().draft.clone());
    yantrik_app::drafts::path_in(app.paths().state_dir(), &name)
}

/// The same editor, started again at its files after it was dropped with no shutdown: a crash, a
/// `kill -9`. Nothing it had not handed the disk survives.
pub fn killed_and_started_again(app: TestApp<TextEditor>) -> TestApp<TextEditor> {
    let paths = app.paths();
    drop(app);
    let again = TestApp::<TextEditor>::start_at(paths, Vec::new());
    settle(&again);
    again
}

/// A file changed outside the editor, and the watch's next look.
pub fn outside_writes(app: &TestApp<TextEditor>, path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    app.look_at_files();
    settle(app);
}

/// `{}` as a call's arguments.
pub fn none() -> Value {
    json!({})
}
