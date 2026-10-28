//! ySheets under test: headless through yantrik-testing, with its drafts under the test's own root,
//! files in scratch folders, jobs run when the test says, and the disk made to fail where a test
//! asks. The window's own callbacks are invoked as the screen invokes them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use slint::Model as _;
use yantrik_app::documents::disk::FaultyDisk;
use yantrik_testing::{Call, TestApp};

use crate::address::{parse_cell, Cell};
use crate::command::Command;
use crate::files::picker::Choice;
use crate::YSheets;

mod agents;
mod answers;
mod closing;
mod dialogs;
mod files;
mod launch;
mod limits;
mod lingering;
mod mind_view;
mod outside;
mod parity;
mod private_mode;
mod recalc;
mod recovery;
mod saving;
mod shots;
mod special;
mod window;

/// ySheets, started and settled.
pub fn started() -> TestApp<YSheets> {
    let app = TestApp::<YSheets>::start();
    settle(&app);
    app
}

/// Runs jobs until none is left: a landing may start its follow-up, and so on.
pub fn settle(app: &TestApp<YSheets>) {
    for _ in 0..32 {
        if app.jobs_waiting() == 0 {
            return;
        }
        app.run_jobs();
    }
    panic!("jobs kept starting jobs");
}

/// One call through the real dispatch, as the person, answered once its work has landed.
pub fn act(app: &TestApp<YSheets>, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act(name, args)?;
    settle(app);
    pending.settle()
}

/// The same call, as an agent with this token.
pub fn act_as_agent(app: &TestApp<YSheets>, token: &str, name: &str, args: Value) -> Result<Value, String> {
    let pending = app.act_as(Call::new(name, args).as_agent(token).granted())?;
    settle(app);
    pending.settle()
}

/// What `describe` says now, its state object.
pub fn state(app: &TestApp<YSheets>) -> Value {
    app.describe()["state"].clone()
}

pub fn summary(app: &TestApp<YSheets>) -> String {
    app.describe()["summary"].as_str().unwrap_or_default().to_string()
}

/// The notice, as the window has it.
pub fn notice(app: &TestApp<YSheets>) -> String {
    app.model(|m| m.notice.window().to_string())
}

/// What the status line says.
pub fn status(app: &TestApp<YSheets>) -> String {
    app.ui(|ui| ui.get_status_text().to_string())
}

pub fn modified(app: &TestApp<YSheets>) -> bool {
    app.model(|m| m.modified())
}

pub fn path_of(app: &TestApp<YSheets>) -> Option<PathBuf> {
    app.model(|m| m.path().map(Path::to_path_buf))
}

pub fn cell(name: &str) -> Cell {
    parse_cell(name).expect("a cell name")
}

/// What was typed into a cell, and what it works out to as the model has it.
pub fn raw(app: &TestApp<YSheets>, name: &str) -> String {
    let at = cell(name);
    app.model(move |m| m.sheet().raw(at).to_string())
}

/// What a cell shows in the window's grid.
pub fn shown(app: &TestApp<YSheets>, name: &str) -> String {
    let at = cell(name);
    app.ui(move |ui| ui.get_rows().row_data(at.row).unwrap().cells.row_data(at.col).unwrap().text.to_string())
}

/// The selected cell.
pub fn active(app: &TestApp<YSheets>) -> String {
    app.model(|m| m.active.name())
}

/// What the formula bar holds.
pub fn bar(app: &TestApp<YSheets>) -> String {
    app.ui(|ui| ui.get_cell_data().to_string())
}

/// The person typed `text` into the formula bar and pressed Enter, as the screen sends it.
pub fn enter(app: &TestApp<YSheets>, text: &str) {
    submit(app, text, "down");
}

pub fn submit(app: &TestApp<YSheets>, text: &str, direction: &str) {
    let (text, direction) = (text.to_string(), direction.to_string());
    app.ui(move |ui| {
        ui.set_cell_data(text.clone().into());
        ui.invoke_formula_submitted(text.into(), direction.into());
    });
    settle(app);
}

/// The person typed `text` in the bar and has not pressed anything yet.
pub fn type_in_bar(app: &TestApp<YSheets>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| ui.set_cell_data(text.into()));
}

/// The person clicked a cell, with the bar as it was.
pub fn click(app: &TestApp<YSheets>, name: &str) {
    let at = cell(name);
    app.ui(move |ui| ui.invoke_cell_clicked(at.row as i32, at.col as i32));
    settle(app);
}

/// A header button or key.
pub fn header(app: &TestApp<YSheets>, id: &str) {
    let id = id.to_string();
    app.ui(move |ui| ui.invoke_command(id.into()));
    settle(app);
}

/// A scratch folder of its own, outside the home, every link in its path resolved.
pub fn scratch(name: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("yantrik-ysheets-{name}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

/// ySheets with a saved sheet open, and the file.
pub fn with_file(name: &str, csv: &str) -> (TestApp<YSheets>, PathBuf) {
    let app = started();
    let path = scratch(name).join("budget.csv");
    std::fs::write(&path, csv).unwrap();
    act(&app, "open", json!({ "path": path.display().to_string() })).expect("the sheet opens");
    assert_eq!(path_of(&app), Some(path.clone()), "{}", status(&app));
    (app, path)
}

pub fn file(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

/// The names in a folder, sorted.
pub fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Saves go through `disk` from now on.
pub fn failing(app: &TestApp<YSheets>) -> Arc<FaultyDisk> {
    let disk = Arc::new(FaultyDisk::new());
    app.run(Command::UseDisk(disk.clone()));
    disk
}

/// Where the sheet's draft is kept.
pub fn draft_file(app: &TestApp<YSheets>) -> PathBuf {
    app.model(|m| m.draft_path())
}

/// Lets the draft's pause pass, then the lane's writes.
pub fn flush_drafts(app: &TestApp<YSheets>) {
    app.advance(crate::update::drafts::PAUSE);
    settle(app);
    app.run_writes();
}

/// The same ySheets, started again at its files after it was dropped with no shutdown: a crash, a
/// `kill -9`. Nothing it had not handed the disk survives.
pub fn killed_and_started_again(app: TestApp<YSheets>) -> TestApp<YSheets> {
    let paths = app.paths();
    drop(app);
    let again = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&again);
    again
}

/// A file changed outside ySheets, and the watch's next look.
pub fn outside_writes(app: &TestApp<YSheets>, path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    app.look_at_files();
    settle(app);
}

/// `{}` as a call's arguments.
pub fn none() -> Value {
    json!({})
}

/// The desktop's picker answers with `answer` whatever it is asked, and asks of it what it was.
pub fn picker_says(app: &TestApp<YSheets>, answer: impl Fn(bool, Option<PathBuf>) -> Choice + Send + Sync + 'static) {
    app.run(Command::UseAsk(Arc::new(answer)));
}
