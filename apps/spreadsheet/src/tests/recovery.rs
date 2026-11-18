//! The sheet a start opens again: the draft the last run left, and what a file named on the command
//! line does to it. Nothing here removes a draft, and nothing is offered twice.

use std::path::Path;

use serde_json::json;
use yantrik_app::paths::Paths;
use yantrik_testing::TestApp;

use super::{
    act, active, click, draft_file, enter, flush_drafts, killed_and_started_again, modified, notice, path_of, raw,
    scratch, settle, started, state, with_file,
};
use crate::files::drafts::{now_ms, SheetDraft, VERSION};
use crate::YSheets;

/// A draft in the lane, as this build writes one.
fn lane_draft(paths: &Paths, name: &str, path: Option<&Path>, rows: &[&[&str]], at: u64) {
    let draft = SheetDraft {
        version: VERSION,
        path: path.map(Path::to_path_buf),
        rows: rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect(),
        on_disk: None,
        active: "A1".into(),
        typing: None,
        at,
    };
    let file = yantrik_app::drafts::path_in(paths.state_dir(), name);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, draft.bytes()).unwrap();
}

#[test]
fn a_kill_after_the_draft_landed_loses_nothing() {
    let app = started();
    enter(&app, "typed");
    enter(&app, "=A1&\"!\"");
    click(&app, "C3");
    flush_drafts(&app);
    assert!(draft_file(&app).exists());
    let again = killed_and_started_again(app);
    assert_eq!((raw(&again, "A1"), raw(&again, "A2")), ("typed".into(), "=A1&\"!\"".into()));
    assert_eq!(super::shown(&again, "A2"), "typed!", "worked out again");
    assert_eq!(active(&again), "C3", "and the selection is where it was");
    assert!(modified(&again), "a recovered sheet is by construction not what is on disk");
    assert_eq!(notice(&again), "Recovered unsaved changes. Save keeps them.");
    assert_eq!(app_title(&again), "Untitled");
}

fn app_title(app: &TestApp<YSheets>) -> String {
    app.ui(|ui| ui.get_file_name().to_string())
}

#[test]
fn a_kill_before_the_pause_is_over_loses_only_the_last_moment() {
    let app = started();
    enter(&app, "typed");
    let again = killed_and_started_again(app);
    assert_eq!(raw(&again, "A1"), "", "the draft had not been made yet");
}

#[test]
fn a_sheet_with_a_file_comes_back_against_that_file_and_saves_into_it() {
    let (app, path) = with_file("recovered-named", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "Edited after saving" })).unwrap();
    flush_drafts(&app);
    let again = killed_and_started_again(app);
    assert_eq!(path_of(&again), Some(path.clone()));
    assert_eq!(notice(&again), format!("Recovered unsaved changes to {}. Save keeps them.", path.display()));
    act(&again, "save_over", super::none()).expect("the file is what the draft was made against, so Save is not a conflict");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "Edited after saving\r\n");
    assert!(!modified(&again));
}

#[test]
fn a_sheet_saved_or_undone_back_to_its_file_keeps_no_draft() {
    let (app, _) = with_file("no-draft", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    flush_drafts(&app);
    let draft = draft_file(&app);
    assert!(draft.exists());
    act(&app, "undo", json!({})).unwrap();
    flush_drafts(&app);
    assert!(!draft.exists(), "undone: nothing is owed");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "3" })).unwrap();
    flush_drafts(&app);
    act(&app, "save_over", json!({})).unwrap();
    app.run_writes();
    assert!(!draft.exists(), "saved: the draft is cleared, not offered again");
    app.shutdown();
    assert_eq!(notice(&killed_and_started_again(app)), "");
}

#[test]
fn a_draft_that_is_not_one_of_ours_is_left_alone_and_said() {
    let paths = Paths::temporary("spreadsheet");
    let file = yantrik_app::drafts::path_in(paths.state_dir(), "somebody-elses");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "{ not json").unwrap();
    let app = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&app);
    assert!(notice(&app).contains("1 more draft could not be opened at once and is kept"), "{}", notice(&app));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json", "nothing removes a draft");
    assert_eq!(raw(&app, "A1"), "");
}

#[test]
fn a_draft_past_the_sheets_limits_is_left_alone_and_said() {
    let paths = Paths::temporary("spreadsheet");
    let huge = vec![vec!["x".to_string(); 300]];
    let draft = SheetDraft { version: VERSION, path: None, rows: huge, on_disk: None, active: "A1".into(), typing: None, at: 1 };
    let file = yantrik_app::drafts::path_in(paths.state_dir(), "sheet-too-wide");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, draft.bytes()).unwrap();
    let app = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&app);
    assert!(notice(&app).contains("1 more draft"), "{}", notice(&app));
    assert!(file.exists());
}

#[test]
fn the_newest_of_several_drafts_opens_and_the_rest_are_counted() {
    let paths = Paths::temporary("spreadsheet");
    lane_draft(&paths, "sheet-1-1", None, &[&["older"]], 100);
    lane_draft(&paths, "sheet-2-1", None, &[&["newer"]], 200);
    let app = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&app);
    assert_eq!(raw(&app, "A1"), "newer");
    assert!(notice(&app).contains("1 more draft could not be opened at once and is kept"), "{}", notice(&app));
}

#[test]
fn a_file_named_on_the_command_line_wins_the_window_and_leaves_every_draft_where_it_is() {
    let paths = Paths::temporary("spreadsheet");
    lane_draft(&paths, "sheet-1-1", None, &[&["draft text"]], now_ms());
    let given = scratch("argv-draft").join("given.csv");
    std::fs::write(&given, "from the file\r\n").unwrap();
    let draft = yantrik_app::drafts::path_in(paths.state_dir(), "sheet-1-1");
    let app = TestApp::<YSheets>::start_at(paths.clone(), vec![given.clone().into()]);
    settle(&app);
    assert_eq!(raw(&app, "A1"), "from the file");
    assert!(draft.exists(), "the draft is left exactly where it is");
    let said = notice(&app);
    assert!(said.contains("An unsaved draft from a previous session is waiting in") && said.contains("drafts"), "{said}");
    app.shutdown();
    drop(app);
    let later = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&later);
    assert_eq!(raw(&later, "A1"), "draft text", "and the next start with no file offers it");
}

#[test]
fn a_recovered_sheet_whose_file_is_somewhere_an_agent_is_not_shown_stays_hidden_from_it() {
    let paths = Paths::temporary("spreadsheet");
    let outside = scratch("hidden-recovery").join("secret.csv");
    if outside.starts_with(std::env::var_os("HOME").unwrap_or_default()) {
        return; // Inside the home it would be shown, and prove nothing here.
    }
    lane_draft(&paths, "sheet-1-1", Some(&outside), &[&["topsecret"]], 1);
    let app = TestApp::<YSheets>::start_at(paths, Vec::new());
    settle(&app);
    assert!(app.model(|m| m.doc.hidden), "decided when the draft took its file");
    assert_eq!(state(&app)["selection"]["typed"], json!("topsecret"), "the person sees it");
}

#[test]
fn the_draft_is_encoded_in_a_job_and_handed_to_the_lane_when_it_lands() {
    let app = started();
    enter(&app, "typed");
    app.advance(crate::update::drafts::PAUSE);
    assert_eq!(app.jobs_waiting(), 1, "the encode is a job, not part of the timer's command");
    assert_eq!(app.writes_waiting(), 0, "and nothing is the lane's yet");
    settle(&app);
    assert_eq!(app.writes_waiting(), 1);
    app.run_writes();
    assert!(std::fs::read_to_string(draft_file(&app)).unwrap().contains("typed"));
}

#[test]
fn a_draft_encoded_for_a_sheet_typed_into_since_is_dropped_and_the_newer_one_is_made() {
    let app = started();
    enter(&app, "first");
    app.advance(crate::update::drafts::PAUSE);
    assert_eq!(app.jobs_waiting(), 1);
    // Typed into before the encode landed.
    enter_no_settle(&app, "second");
    app.run_jobs();
    assert_eq!(app.writes_waiting(), 0, "the encode of the older state is not kept");
    flush_drafts(&app);
    let kept = std::fs::read_to_string(draft_file(&app)).unwrap();
    assert!(kept.contains("second") && kept.contains("first"), "the newer state, with both: {kept}");
}

#[test]
fn a_draft_that_lands_after_the_sheet_was_saved_is_not_kept() {
    let (app, _) = with_file("draft-after-save", "1\r\n");
    act(&app, "set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    app.advance(crate::update::drafts::PAUSE);
    let saving = app.act("save_over", json!({})).unwrap();
    app.run_jobs();
    settle(&app);
    saving.settle().unwrap();
    app.run_writes();
    assert!(!draft_file(&app).exists(), "nothing is unsaved, so no draft");
}

fn enter_no_settle(app: &yantrik_testing::TestApp<crate::YSheets>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.set_cell_data(text.clone().into());
        ui.invoke_formula_submitted(text.into(), "down".into());
    });
}
