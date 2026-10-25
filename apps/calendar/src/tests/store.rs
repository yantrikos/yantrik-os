//! Keeping what is in hand true: to the store, when somebody else writes to it, and to the clock,
//! when the date turns. The order reads land in, a read that fails, and jobs that end without a
//! result.

use std::time::Duration;

use serde_json::json;
use yantrik_app::app::WindowId;

use super::{a_week, open, open_at};
use crate::command::Command;
use crate::{CLOCK_EVERY, STORE_WATCH_EVERY};

/// A mind reads `describe` straight after writing through its own tools; the answer must already
/// hold what it wrote, and the window follows.
#[test]
fn describe_asks_the_store_before_it_answers() {
    let f = open(a_week());
    f.store.put("ev-outside", "Written elsewhere", "2026-09-23T17:00:00", "2026-09-23T18:00:00", Some("agent mind:7"));
    let described = f.app.describe();
    let titles: Vec<&str> = described["state"]["events_on_selected_day"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Standup", "Design review", "Written elsewhere"]);
    f.quiet();
    assert_eq!(f.agenda(), ["Standup", "Design review", "Written elsewhere"], "the window shows what the Mind was told");
    let listings = f.store.asked_for("calendar.events").len();
    f.describe();
    assert_eq!(f.store.asked_for("calendar.events").len(), listings, "nothing moved, so nothing is listed again");
}

/// The twenty-second watch: an outside write appears without anybody navigating.
#[test]
fn the_watch_follows_a_store_that_moved() {
    let f = open(a_week());
    f.store.put("ev-outside", "Synced", "2026-09-23T08:00:00", "2026-09-23T08:30:00", None);
    f.app.advance(STORE_WATCH_EVERY);
    f.quiet();
    assert_eq!(f.agenda(), ["Synced", "Standup", "Design review"]);
    let listings = f.store.asked_for("calendar.events").len();
    f.app.advance(STORE_WATCH_EVERY);
    f.quiet();
    assert_eq!(f.store.asked_for("calendar.events").len(), listings, "two numbers asked, nothing listed");
}

/// A window that never started the calendar service does not start it to ask whether it moved.
#[test]
fn a_service_that_is_down_is_not_started_to_be_watched() {
    let f = open(a_week());
    f.store.set_down(true);
    let revisions = f.store.asked_for("calendar.revision").len();
    f.app.advance(STORE_WATCH_EVERY);
    f.quiet();
    f.describe();
    assert_eq!(f.store.asked_for("calendar.revision").len(), revisions);
    assert_eq!(f.agenda(), ["Standup", "Design review"], "what is in hand is kept");
}

/// Two navigations, their reads landing in the other order: the window shows the month it is on.
#[test]
fn a_read_for_a_month_the_window_has_left_is_dropped() {
    let f = open(a_week().holding("ev-oct", "October thing", "2026-10-07T09:00:00", "2026-10-07T10:00:00"));
    f.app.ui(|ui| ui.invoke_next_month());
    f.app.ui(|ui| ui.invoke_prev_month());
    // October's read lands first, for a month the window has already left.
    assert_eq!(f.app.jobs_waiting(), 2);
    f.app.run_jobs();
    f.app.tick();
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "September 2026");
    assert_eq!(f.app.model(|m| m.range.map(|r| r.0.to_string())), Some("2026-09-01".to_string()));
    assert!(f.app.model(|m| m.events.iter().all(|e| e.id != "ev-oct")), "October's events are not September's");
}

/// Of two reads for the range on screen, the one taken later is shown, whatever order they land.
#[test]
fn an_older_read_never_replaces_a_newer_one() {
    let f = open(a_week());
    let (older, wanted) = f.app.model(|m| {
        let wanted = m.visible_range();
        (listed(m, wanted), wanted)
    });
    f.store.put("ev-new", "Newer", "2026-09-23T20:00:00", "2026-09-23T21:00:00", None);
    let newest = f.app.model(move |m| listed(m, wanted));
    f.app.run(Command::Listed(Box::new(newest)));
    f.app.run(Command::Listed(Box::new(older)));
    assert_eq!(f.agenda(), ["Standup", "Design review", "Newer"]);
}

/// A forced read of `wanted`, taken now, as a job would take it.
fn listed(m: &crate::Model, wanted: crate::work::Range) -> crate::work::Listing {
    let nothing = crate::work::Held { range: None, revision: None };
    crate::work::reread(m.machine.api.as_ref(), &m.reads.sequence, wanted, &nothing, true, crate::work::Start::MayStart)
}

/// A read that fails shows no events, says nothing false about them, and is asked again.
#[test]
fn a_failed_read_is_asked_again_rather_than_kept() {
    let f = open(a_week());
    f.store.fail_once("calendar.events", -32000, "the socket went away");
    f.app.ui(|ui| ui.invoke_next_month());
    f.quiet();
    assert_eq!(f.app.model(|m| m.range), None);
    f.app.ui(|ui| ui.invoke_prev_month());
    f.quiet();
    assert_eq!(f.app.model(|m| m.events.len()), 3, "the next move asked again");
    assert!(f.app.model(|m| m.range.is_some()));
}

/// The sign-off's case: a window left on today follows the date across midnight; the marker moves
/// whatever the window shows.
#[test]
fn a_window_left_on_today_follows_the_date_across_midnight() {
    let f = open_at(a_week(), "2026-09-23 23:59:30");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 23);
    f.clock.advance(31);
    f.app.advance(Duration::from_secs(31));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 24);
    assert_eq!(f.state()["today"], json!("2026-09-24 Thursday"));
    f.app.ui(|ui| ui.invoke_day_clicked(12));
    f.quiet();
    // The person goes elsewhere for a day and comes back: the window manager's focus, as the
    // framework hears it from winit, is `Event::Activated`.
    f.app.deactivate(WindowId::PRIMARY);
    f.clock.advance(86_400);
    f.app.activate(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 12, "a day the person picked stays picked");
    use slint::Model as _;
    let marked: Vec<i32> = f.app.ui(|ui| ui.get_days().iter().filter(|d| d.is_today).map(|d| d.day_number).collect());
    assert_eq!(marked, [25]);
}

/// Brought back to the front after midnight, with no timer fired since: the window follows the
/// date at once, through the framework's `Event::Activated` (it sets no winit hook of its own).
#[test]
fn coming_back_to_the_window_after_midnight_follows_the_date() {
    let f = open_at(a_week(), "2026-09-23 23:59:30");
    f.app.deactivate(WindowId::PRIMARY);
    f.clock.advance(60);
    f.app.activate(WindowId::PRIMARY);
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 24);
    assert_eq!(f.state()["today"], json!("2026-09-24 Thursday"));
}

#[test]
fn the_now_line_moves_each_minute() {
    let f = open(a_week());
    f.clock.advance(90);
    f.app.advance(CLOCK_EVERY);
    assert_eq!(f.app.ui(|ui| (ui.get_current_hour(), ui.get_current_minute())), (10, 1));
}

/// A job that ends without a result is said, and the window goes on: before, a panic took it down.
#[test]
fn a_write_whose_worker_panics_is_said_and_the_app_goes_on() {
    let f = open(a_week());
    f.store.panicking("calendar.delete_event");
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.quiet();
    assert_eq!(f.notice(), "The delete did not finish; try again.");
    let e = f.act("delete_event", json!({ "id": "ev-review" })).unwrap_err();
    assert!(e.contains("the work failed"), "{e}");
    f.app.ui(|ui| ui.invoke_day_clicked(25));
    f.quiet();
    assert_eq!(f.agenda(), ["Dentist"], "and reads go on");
}

/// The calendar's events are the service's: the app keeps no file of its own.
#[test]
fn the_app_keeps_no_files() {
    let f = open(a_week());
    f.act("add_event", json!({ "title": "A", "date": "2026-09-24", "time": "09:00" })).unwrap();
    f.app.run_writes();
    let paths = f.app.paths();
    assert!(!paths.config_file().exists(), "no settings file");
    assert!(!paths.data_dir().exists() || std::fs::read_dir(paths.data_dir()).map(|d| d.count()).unwrap_or(0) == 0);
}

/// A shutdown with a write in flight answers it and does not hang.
#[test]
fn a_shutdown_with_a_write_in_flight_answers_it() {
    let f = open(a_week());
    let pending = f.app.act("delete_event", json!({ "id": "ev-review" })).unwrap();
    assert!(pending.is_owed());
    let began = std::time::Instant::now();
    let _ = f.app.shutdown();
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(pending.settle().unwrap_err(), "the app is closing");
}

/// The watch never starts calendar-service, not even after a failed read left no range in hand
/// (review of #921, finding 3).
#[test]
fn the_watch_never_starts_a_service_that_is_down() {
    let f = open(a_week());
    f.store.fail_once("calendar.events", -32000, "the socket went away");
    f.app.ui(|ui| ui.invoke_next_month());
    f.quiet();
    assert_eq!(f.app.model(|m| m.range), None, "a failed read leaves no range held");
    f.store.set_down(true);
    f.app.advance(STORE_WATCH_EVERY);
    f.quiet();
    assert_eq!(f.store.start_attempts(), 0, "the watch asked nothing that starts the service");
    f.store.set_down(false);
    f.app.advance(STORE_WATCH_EVERY);
    f.quiet();
    assert!(f.app.model(|m| m.range.is_some()), "and reads again once it is up");
}

/// A reading intent keeps what is in hand when the store cannot be asked: with the service down
/// while the window holds the month, a day in it is answered from the month in hand, the agenda is
/// kept, and nothing tries to start the service (delta review of #921, P14).
#[test]
fn a_reading_intent_keeps_the_month_in_hand_while_the_service_is_down() {
    let f = open(a_week());
    f.store.set_down(true);
    assert_eq!(f.act("select_day", json!({ "day": 25 })).unwrap(), json!({ "day": 25, "events": ["Dentist"] }));
    assert_eq!(f.agenda(), ["Dentist"], "the window keeps the month");
    assert_eq!(f.act("set_view", json!({ "view": "month" })).unwrap()["events"], json!(3));
    f.app.ui(|ui| ui.invoke_day_clicked(23));
    f.quiet();
    assert_eq!(f.agenda(), ["Standup", "Design review"]);
    assert_eq!(f.store.start_attempts(), 0, "nothing asked the service to start");
}
