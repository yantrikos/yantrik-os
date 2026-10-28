//! Working the sheet out is a job: typing never waits for it, a result for a sheet that has been
//! typed into since is dropped, and a call is answered from the result of the job made for it.

use std::time::{Duration, Instant};

use serde_json::json;
use slint::Model as _;
use yantrik_testing::TestApp;

use super::{act, active, raw, settle, shown, started};
use crate::command::Command;
use crate::sheet::Sheet;
use crate::YSheets;

/// The person typed `text` and pressed Enter; no job is run.
fn enter_no_wait(app: &TestApp<YSheets>, text: &str) {
    let text = text.to_string();
    app.ui(move |ui| {
        ui.set_cell_data(text.clone().into());
        ui.invoke_formula_submitted(text.into(), "down".into());
    });
}

fn solved(app: &TestApp<YSheets>) -> bool {
    app.model(|m| m.sheet().is_solved())
}

#[test]
fn typing_changes_the_text_at_once_and_the_values_come_from_a_job() {
    let app = started();
    enter_no_wait(&app, "4");
    assert_eq!(raw(&app, "A1"), "4", "the text is the sheet's already");
    assert_eq!(shown(&app, "A1"), "4", "and a value that is not a formula shows as itself");
    enter_no_wait(&app, "=A1*2");
    assert_eq!(raw(&app, "A2"), "=A1*2");
    assert!(!solved(&app), "nothing was worked out on the window's thread");
    assert_eq!(shown(&app, "A2"), "", "the formula shows nothing until its job lands");
    assert_eq!(app.jobs_waiting(), 1, "one job for the newest text; the first was replaced");
    settle(&app);
    assert!(solved(&app));
    assert_eq!(shown(&app, "A2"), "8");
}

#[test]
fn a_formula_that_takes_long_never_holds_up_the_command_that_typed_it() {
    // 26 columns of 1,000 running sums: slow to work out, and the command that types the first
    // of them returns before any of it is done.
    let rows: Vec<Vec<String>> = (1..=1000).map(|r| (0..26).map(|_| format!("=SUM(A{r}:A{r})+1")).collect()).collect();
    let started_at = Instant::now();
    let sheet = Sheet::from_rows(&rows).unwrap();
    let worked_out_in_place = started_at.elapsed();
    drop(sheet);

    let app = started();
    let typed = Instant::now();
    enter_no_wait(&app, "=SUM(A1:A1)");
    let returned = typed.elapsed();
    assert!(!solved(&app));
    assert!(returned < Duration::from_millis(250) || returned < worked_out_in_place, "{returned:?} vs {worked_out_in_place:?}");
    settle(&app);
    assert!(solved(&app));
}

#[test]
fn a_result_for_a_sheet_typed_into_since_is_dropped_and_the_call_is_still_answered_from_it() {
    let app = started();
    // A call's edit starts a job of its own, which a later edit does not replace.
    let call = app.act("set_cell", json!({ "cell": "A1", "text": "=B1+1" })).unwrap();
    // The person types into B1 before that job has landed.
    app.ui(|ui| ui.invoke_cell_clicked(0, 1));
    enter_no_wait(&app, "5");
    assert_eq!(app.jobs_waiting(), 2);
    settle(&app);
    let answer = call.settle().unwrap();
    assert_eq!(answer["value"], json!(1.0), "the call is answered with the sheet its own edit made: B1 was empty");
    assert_eq!(answer["typed"], "=B1+1");
    assert_eq!(shown(&app, "A1"), "6", "and the sheet in front is the one with the person's 5");
}

#[test]
fn two_calls_in_flight_are_each_answered_with_their_own_result() {
    let app = started();
    let first = app.act("set_cell", json!({ "cell": "A1", "text": "1" })).unwrap();
    let second = app.act("set_cell", json!({ "cell": "A1", "text": "2" })).unwrap();
    let third = app.act("get_range", json!({ "range": "A1" })).unwrap();
    settle(&app);
    let (first, second, third) = (first.settle().unwrap(), second.settle().unwrap(), third.settle().unwrap());
    assert_eq!((first["value"].clone(), first["before"].clone()), (json!(1.0), json!("")));
    assert_eq!((second["value"].clone(), second["before"].clone()), (json!(2.0), json!("1")));
    assert_eq!(third["typed"], json!([["2"]]), "a read is of the sheet when it was asked");
    assert_eq!(shown(&app, "A1"), "2");
}

#[test]
fn a_read_of_a_sheet_that_is_worked_out_is_answered_at_once() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "3" })).unwrap();
    let read = app.act("get_range", json!({ "range": "A1" })).unwrap();
    assert_eq!(app.jobs_waiting(), 0, "nothing to wait for");
    assert_eq!(read.settle().unwrap()["values"], json!([[3.0]]));
}

#[test]
fn a_read_of_a_sheet_typed_into_since_waits_for_its_own_job() {
    let app = started();
    enter_no_wait(&app, "=1+1");
    let read = app.act("get_range", json!({ "range": "A1" })).unwrap();
    assert!(app.jobs_waiting() >= 1);
    settle(&app);
    assert_eq!(read.settle().unwrap()["values"], json!([[2.0]]), "never the stand-in");
}

#[test]
fn only_the_cells_that_changed_are_drawn_again() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "C3", "text": "=1+1" })).unwrap();
    // Mark a cell nothing is about to touch, in the list the window holds.
    app.ui(|ui| {
        let rows = ui.get_rows();
        let line = rows.row_data(9).unwrap().cells;
        let mut cell = line.row_data(9).unwrap();
        cell.text = "NOT REDRAWN".into();
        line.set_row_data(9, cell);
    });
    act(&app, "set_cell", json!({ "cell": "C4", "text": "=C3*2" })).unwrap();
    assert_eq!(shown(&app, "C4"), "4");
    assert_eq!(shown(&app, "J10"), "NOT REDRAWN", "a result draws the cells that differ from before, not the grid");
}

#[test]
fn a_result_for_another_sheet_is_never_taken() {
    let mut mine = Sheet::default();
    let theirs = Sheet::default();
    mine.put(super::cell("A1"), "=1+1").unwrap();
    let solved = theirs.view();
    assert!(!mine.install(&solved), "the revision belongs to another sheet");
    assert!(!mine.is_solved());
    let stale = mine.snapshot().solve();
    mine.put(super::cell("B1"), "7").unwrap();
    assert!(!mine.install(&stale), "typed into since");
    mine.settle();
    assert!(mine.is_solved());
}

#[test]
fn a_job_that_ended_without_an_answer_is_said_and_not_started_again() {
    let app = started();
    enter_no_wait(&app, "=1+1");
    let rev = app.model(|m| m.sheet().rev());
    app.run(Command::SolveLost { rev, then: None });
    assert!(super::notice(&app).contains("could not be worked out"), "{}", super::notice(&app));
    assert_eq!(app.jobs_waiting(), 1, "no second job for the same revision");
    // An edit makes a new revision, and that one is worked out.
    enter_no_wait(&app, "=2+2");
    settle(&app);
    assert_eq!(shown(&app, "A2"), "4");
    assert_eq!(active(&app), "A3");
}

#[test]
fn opening_a_file_builds_and_works_out_its_sheet_in_the_job() {
    let (app, _) = super::with_file("open-solved", "1\r\n=A1+1\r\n=A2+1\r\n");
    assert!(solved(&app));
    assert_eq!(shown(&app, "A3"), "3");
    assert_eq!(app.jobs_waiting(), 0);
}
