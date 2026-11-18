//! What `describe` carries beyond the shape `parity.rs` pins: the day the machine is living in, the
//! names behind the ids, and who the caller was taken to be. These were pinned against the old
//! `main.rs` text, because the closure needed a live window; they are read off the running app now.

use serde_json::json;
use yantrik_app::control::AgentTokenScope;
use yantrik_testing::Call;

use super::{a_week, open, open_at};
use crate::backend::fake::FakeCalendar;
use crate::views::NAMING_CAP;

/// "What is on my calendar today" is the question a calendar exists to answer, and the mind that
/// asked it used to run `date` through the shell to learn which day today even is (#207).
#[test]
fn the_describe_says_which_day_today_is() {
    assert_eq!(open(a_week()).state()["today"], json!("2026-09-23 Wednesday"));
    let f = open_at(FakeCalendar::default(), "2026-01-04 08:00:00");
    assert_eq!(f.state()["today"], json!("2026-01-04 Sunday"));
    f.act("show_month", json!({ "direction": "next" })).unwrap();
    assert_eq!(f.state()["today"], json!("2026-01-04 Sunday"), "today, not the month on screen");
}

/// The id→name index (#54): every event in hand named as a person would say it, never more than
/// the cap, the newest kept, and nothing from outside the range the window read.
#[test]
fn the_describe_publishes_what_its_ids_stand_for() {
    let store = FakeCalendar::default().holding("ev-december", "Far off", "2026-12-01T09:00:00", "2026-12-01T10:00:00");
    for i in 0..NAMING_CAP + 5 {
        let (day, minute) = (1 + i / 60, i % 60);
        let start = format!("2026-09-{day:02}T08:{minute:02}:00");
        let end = format!("2026-09-{day:02}T08:{minute:02}:30");
        store.put(&format!("ev-{i:03}"), &format!("Item {i}"), &start, &end, None);
    }
    let f = open(store);
    let naming = f.state()["naming"].as_object().cloned().unwrap();
    assert_eq!(naming.len(), NAMING_CAP);
    assert_eq!(naming["ev-204"], json!("Item 204, Fri 4 Sep 08:24"));
    assert!(!naming.contains_key("ev-000") && !naming.contains_key("ev-004"), "the oldest are dropped");
    assert!(naming.contains_key("ev-005"));
    assert!(!naming.contains_key("ev-december"), "only the range in hand is named");
}

/// Each event says whether this caller may take it off unasked, against the identity the machine
/// established for the caller, which `you` reports.
#[test]
fn the_describe_marks_the_callers_own_events() {
    let f = open(a_week());
    let made = f
        .app
        .act_as(Call::new("add_event", json!({ "title": "Mine", "date": "2026-09-23", "time": "11:00" })).as_agent("mind:7"))
        .unwrap();
    f.settled(made).unwrap();
    let state = f.app.model(|m| {
        let _agent = AgentTokenScope::enter(Some("mind:7".into()));
        crate::describe::view(m).state
    });
    assert_eq!(state["you"], json!("agent mind:7"));
    let marks: Vec<(String, bool)> = state["events_on_selected_day"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["title"].as_str().unwrap().to_string(), e["may_delete_unasked"].as_bool().unwrap()))
        .collect();
    assert_eq!(marks, [("Standup".into(), false), ("Mine".into(), true), ("Design review".into(), false)]);
    assert_eq!(f.state()["you"], json!("could not be identified"), "a describe with no caller");
}
