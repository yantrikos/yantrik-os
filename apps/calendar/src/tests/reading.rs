//! The intents that move the window and read the store, through the real dispatch: each answers
//! with what the window then shows, read after its work has landed, and refuses what it always
//! refused in the words it always used.

use serde_json::json;

use super::{a_week, open};
use crate::backend::fake::FakeCalendar;

#[test]
fn select_day_answers_with_what_is_on_that_day() {
    let f = open(a_week());
    assert_eq!(f.act("select_day", json!({ "day": 25 })).unwrap(), json!({ "day": 25, "events": ["Dentist"] }));
    assert_eq!(f.agenda(), ["Dentist"], "the window shows it too");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 25);
    assert_eq!(f.act("select_day", json!({ "day": 24 })).unwrap(), json!({ "day": 24, "events": [] }));
    let e = f.act("select_day", json!({ "day": 32 })).unwrap_err();
    assert!(e.contains("32 is not a day of the month"), "{e}");
}

#[test]
fn show_date_goes_to_any_day_in_one_call() {
    let f = open(a_week().holding("ev-oct", "Harvest fair", "2026-10-25T11:00:00", "2026-10-25T13:00:00"));
    assert_eq!(
        f.act("show_date", json!({ "date": "2026-10-25" })).unwrap(),
        json!({
            "showing": "October 2026",
            "date": "2026-10-25",
            "week_start": "2026-10-25",
            "week_end": "2026-10-31",
            "events": ["Harvest fair"],
        })
    );
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "October 2026");
    let e = f.act("show_date", json!({ "date": "25 October" })).unwrap_err();
    assert!(e.contains("`date` should look like 2026-10-25, not `25 October`"), "{e}");
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "October 2026", "the window stays where it was");
}

#[test]
fn events_between_reads_the_store_and_moves_nothing() {
    let f = open(a_week().holding("ev-oct", "Harvest fair", "2026-10-02T11:00:00", "2026-10-02T13:00:00"));
    let answer = f.act("events_between", json!({ "from": "2026-09-24", "to": "2026-10-04" })).unwrap();
    assert_eq!(answer["from"], json!("2026-09-24"));
    assert_eq!(answer["to"], json!("2026-10-04"));
    assert_eq!(answer["count"], json!(2));
    let titles: Vec<&str> = answer["events"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Dentist", "Harvest fair"]);
    assert_eq!(answer["events"][0]["location"], json!(null));
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "September 2026", "no view changes");
    let e = f.act("events_between", json!({ "from": "2026-09-01", "to": "2026-11-02" })).unwrap_err();
    assert!(e.contains("63 days is too many"), "{e}");
}

#[test]
fn events_between_says_why_when_the_store_cannot_answer() {
    let store = a_week();
    let f = open(store.clone());
    store.set_down(true);
    let e = f.act("events_between", json!({ "from": "2026-09-24", "to": "2026-09-30" })).unwrap_err();
    assert!(e.contains("the calendar service did not come up"), "{e}");
}

#[test]
fn show_month_steps_and_names_anything_else() {
    let f = open(a_week());
    assert_eq!(f.act("show_month", json!({ "direction": "next" })).unwrap(), json!({ "showing": "October 2026" }));
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 0, "nothing is picked in the month arrived at");
    assert_eq!(f.act("show_month", json!({ "direction": "Back" })).unwrap(), json!({ "showing": "September 2026" }));
    assert_eq!(f.act("show_month", json!({ "direction": "prev" })).unwrap(), json!({ "showing": "August 2026" }));
    let e = f.act("show_month", json!({ "direction": "sideways" })).unwrap_err();
    assert!(e.contains("`direction` is next or previous, not `sideways`"), "{e}");
}

#[test]
fn go_to_today_comes_back_to_the_day() {
    let f = open(a_week());
    f.act("show_month", json!({ "direction": "next" })).unwrap();
    assert_eq!(f.act("go_to_today", json!({})).unwrap(), json!({ "showing": "September 2026", "day": 23 }));
    assert_eq!(f.agenda(), ["Standup", "Design review"]);
}

#[test]
fn set_view_answers_with_what_is_on_screen() {
    let f = open(a_week());
    assert_eq!(
        f.act("set_view", json!({ "view": "week" })).unwrap(),
        json!({ "view": "week", "week_start": "2026-09-20", "week_end": "2026-09-26", "events": 3 })
    );
    assert_eq!(f.app.ui(|ui| ui.get_view_mode()), 1);
    assert_eq!(
        f.act("set_view", json!({ "view": "Day" })).unwrap(),
        json!({ "view": "day", "day": "Wednesday, 23 September 2026", "events": 2 })
    );
    assert_eq!(
        f.act("set_view", json!({ "view": "month" })).unwrap(),
        json!({ "view": "month", "showing": "September 2026", "events": 3 })
    );
    let e = f.act("set_view", json!({ "view": "year" })).unwrap_err();
    assert!(e.contains("unknown view `year`; use month, week or day"), "{e}");
}

/// The week view needs days the month fetch does not cover: the first week of October reaches
/// back into September, and those days are read and drawn.
#[test]
fn a_week_across_a_month_boundary_is_read_from_both_months() {
    let f = open(
        FakeCalendar::default()
            .holding("ev-sep", "September", "2026-09-29T09:00:00", "2026-09-29T10:00:00")
            .holding("ev-oct", "October", "2026-10-01T09:00:00", "2026-10-01T10:00:00"),
    );
    f.act("show_date", json!({ "date": "2026-10-01" })).unwrap();
    let answer = f.act("set_view", json!({ "view": "week" })).unwrap();
    assert_eq!(answer["events"], json!(2));
    use slint::Model as _;
    let columns: Vec<i32> = f.app.ui(|ui| ui.get_week_events().iter().map(|b| b.day_index).collect());
    assert_eq!(columns, [2, 4]);
}

/// `show_date` answers with the date it was asked for, whatever other call lands beside it
/// (review of #921, finding 1).
#[test]
fn show_date_answers_with_its_own_date_whatever_lands_beside_it() {
    let f = open(a_week());
    let shown = f.app.act("show_date", json!({ "date": "2026-10-25" })).unwrap();
    let read = f.app.act("events_between", json!({ "from": "2026-09-20", "to": "2026-09-26" })).unwrap();
    f.quiet();
    assert_eq!(shown.settle().unwrap()["date"], json!("2026-10-25"));
    assert_eq!(read.settle().unwrap()["count"], json!(3));
}

/// A reading intent answers about what it asked for, from its own read, whatever another call or
/// a press does before that read lands; it says the window has moved since (delta review of #921,
/// P8).
#[test]
fn select_day_answers_about_its_day_when_another_call_moves_the_window() {
    let f = open(a_week().holding("ev-early", "Early", "2026-09-07T08:00:00", "2026-09-07T09:00:00"));
    let picked = f.app.act("select_day", json!({ "day": 7 })).unwrap();
    f.app.ui(|ui| ui.invoke_delete_event(0));
    let shown = f.app.act("show_date", json!({ "date": "2026-11-05" })).unwrap();
    f.quiet();
    assert_eq!(
        picked.settle().unwrap(),
        json!({
            "day": 7,
            "events": ["Early"],
            "window_moved_since": "The window has since moved: it shows November 2026, day 5, in the month view.",
        })
    );
    assert_eq!(shown.settle().unwrap()["showing"], json!("November 2026"));
}

/// The person's own navigation, landing before the intent's read, does not become its answer
/// (delta review of #921, P13).
#[test]
fn select_day_answers_about_its_day_when_the_person_moves_on() {
    let f = open(a_week().holding("ev-harvest", "Harvest", "2026-10-09T10:00:00", "2026-10-09T12:00:00"));
    f.act("show_date", json!({ "date": "2026-10-01" })).unwrap();
    let picked = f.app.act("select_day", json!({ "day": 9 })).unwrap();
    f.app.ui(|ui| ui.invoke_next_month());
    f.quiet();
    assert_eq!(
        picked.settle().unwrap(),
        json!({
            "day": 9,
            "events": ["Harvest"],
            "window_moved_since": "The window has since moved: it shows November 2026, no day picked, in the month view.",
        })
    );
}
