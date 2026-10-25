//! The window: what it opens on, its buttons, the form, the agenda's trash icon, the notice, the
//! rail and the card. Each button is pressed through the window's own callback.

use std::time::Duration;

use slint::Model as _;
use yantrik_app::app::Event;
use yantrik_app::mind::fake::FakeMind;
use yantrik_app::mind::Reach;

use super::{a_week, open, open_with};

#[test]
fn the_window_opens_on_today_with_its_events() {
    let f = open(a_week());
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "September 2026");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 23);
    assert_eq!(f.agenda(), ["Standup", "Design review"]);
    let times: Vec<String> = f.app.ui(|ui| ui.get_events_today().iter().map(|e| e.time_text.to_string()).collect());
    assert_eq!(times, ["09:30 – 09:45", "14:00 – 15:30"]);
    let ids: Vec<i32> = f.app.ui(|ui| ui.get_events_today().iter().map(|e| e.id).collect());
    assert_eq!(ids, [0, 1], "a row's id is its position in the day's list");
    let marked: Vec<i32> = f.app.ui(|ui| ui.get_days().iter().filter(|d| d.is_today).map(|d| d.day_number).collect());
    assert_eq!(marked, [23]);
    let busy: Vec<(i32, i32)> = f.app.ui(|ui| {
        ui.get_days().iter().filter(|d| d.has_events).map(|d| (d.day_number, d.event_count)).collect()
    });
    assert_eq!(busy, [(23, 2), (25, 1)]);
    assert_eq!(f.app.ui(|ui| (ui.get_current_hour(), ui.get_current_minute())), (10, 0), "the now line");
    assert_eq!(f.app.ui(|ui| ui.get_cal_timezone_display().to_string()), "Times are local, UTC+00:00");
}

#[test]
fn the_month_buttons_step_and_today_comes_back() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_next_month());
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "October 2026");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 0);
    f.app.ui(|ui| ui.invoke_prev_month());
    f.app.ui(|ui| ui.invoke_prev_month());
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "August 2026");
    f.app.ui(|ui| ui.invoke_today_pressed());
    f.quiet();
    assert_eq!((f.app.ui(|ui| ui.get_month_title().to_string()), f.app.ui(|ui| ui.get_selected_day())), ("September 2026".into(), 23));
    f.app.ui(|ui| ui.invoke_day_clicked(25));
    f.quiet();
    assert_eq!(f.agenda(), ["Dentist"]);
}

#[test]
fn the_week_and_day_views_are_drawn_from_the_same_events() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_switch_view(1));
    f.quiet();
    let labels: Vec<String> = f.app.ui(|ui| ui.get_week_day_labels().iter().map(|l| l.to_string()).collect());
    assert_eq!(labels, ["Sun 20", "Mon 21", "Tue 22", "Wed 23", "Thu 24", "Fri 25", "Sat 26"]);
    let blocks: Vec<(String, i32, i32)> = f.app.ui(|ui| {
        ui.get_week_events().iter().map(|b| (b.title.to_string(), b.day_index, b.duration_min)).collect()
    });
    assert_eq!(blocks, [("Standup".into(), 3, 15), ("Design review".into(), 3, 90), ("Dentist".into(), 5, 60)]);
    f.app.ui(|ui| ui.invoke_switch_view(2));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_day_view_title().to_string()), "Wednesday, 23 September 2026");
    assert_eq!(f.app.ui(|ui| ui.get_day_events().row_count()), 2);
    assert_eq!(f.state()["view"], "day");
}

#[test]
fn the_form_opens_on_the_selected_day_and_saves_through_the_store() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_add_event());
    assert!(f.app.ui(|ui| ui.get_show_event_form()));
    assert_eq!(f.app.ui(|ui| (ui.get_event_date().to_string(), ui.get_event_time().to_string())), ("2026-09-23".into(), "09:00".into()));
    f.app.ui(|ui| ui.invoke_save_event(" Lunch ".into(), "2026-09-23".into(), "12:00".into(), "with Sam".into()));
    f.quiet();
    assert!(!f.app.ui(|ui| ui.get_show_event_form()), "closed once the store kept it");
    let lunch = f.store.stored().into_iter().find(|e| e.title == "Lunch").expect("stored");
    assert_eq!((lunch.end.as_str(), lunch.description.as_str()), ("2026-09-23T13:00:00", "with Sam"));
    assert_eq!(lunch.creator, None, "the form records no creator");
    assert_eq!(f.agenda(), ["Standup", "Lunch", "Design review"]);
}

#[test]
fn a_template_gives_the_form_its_title_and_the_event_its_length_once() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_cal_use_template("Focus".into(), 120));
    assert_eq!(f.app.ui(|ui| ui.get_event_title().to_string()), "Focus");
    f.app.ui(|ui| ui.invoke_save_event("Focus".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    f.quiet();
    let focus = f.store.stored().into_iter().find(|e| e.title == "Focus").unwrap();
    assert_eq!(focus.end, "2026-09-24T11:00:00");
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.invoke_save_event("Plain".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    f.quiet();
    let plain = f.store.stored().into_iter().find(|e| e.title == "Plain").unwrap();
    assert_eq!(plain.end, "2026-09-24T10:00:00", "a blank form is an hour again");
}

/// The form stays open holding what was typed: closing it on a failed save threw the event away
/// twice.
#[test]
fn a_form_that_cannot_be_saved_stays_open_and_says_why() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.set_event_title("Typed".into()));
    f.app.ui(|ui| ui.invoke_save_event("  ".into(), "2026-09-23".into(), "12:00".into(), "".into()));
    f.quiet();
    assert_eq!(f.notice(), "Could not save “”: an event needs a title");
    assert!(f.app.ui(|ui| ui.get_show_event_form()));
    f.store.fail_once("calendar.create_event", -32001, "the disk is full");
    f.app.ui(|ui| ui.invoke_save_event("Typed".into(), "2026-09-23".into(), "12:00".into(), "".into()));
    f.quiet();
    assert_eq!(f.notice(), "Could not save “Typed”: the disk is full");
    assert!(f.app.ui(|ui| ui.get_show_event_form()));
    assert_eq!(f.app.ui(|ui| ui.get_event_title().to_string()), "Typed", "what was typed is still there");
    f.app.ui(|ui| ui.invoke_cancel_event_form());
    assert!(!f.app.ui(|ui| ui.get_show_event_form()));
}

/// The trash icon on a row takes off that row's event, through the same path `delete_event`
/// takes, and a second press while it is on its way does nothing more.
#[test]
fn the_trash_icon_takes_off_the_event_on_its_row() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_delete_event(1));
    f.app.ui(|ui| ui.invoke_delete_event(1));
    f.quiet();
    assert_eq!(f.agenda(), ["Standup"]);
    assert_eq!(f.store.asked_for("calendar.delete_event").len(), 1);
    f.app.ui(|ui| ui.invoke_delete_event(5));
    f.quiet();
    assert_eq!(f.agenda(), ["Standup"], "a row that is not there deletes nothing");
    f.store.fail_once("calendar.delete_event", -32001, "read-only file system");
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.quiet();
    assert_eq!(f.notice(), "Could not delete “Standup”: read-only file system");
    f.app.ui(|ui| ui.invoke_dismiss_notice());
    assert_eq!(f.notice(), "");
}

/// A refusal's "Open event" goes to the event's day, read from the store at the press.
#[test]
fn the_notice_opens_the_event_a_refusal_was_about() {
    let f = open(a_week());
    let refused = f.app.act_as(yantrik_testing::Call::new("delete_own_event", serde_json::json!({ "id": "ev-dentist" })).as_agent("mind:7")).unwrap();
    assert!(f.settled(refused).is_err());
    f.app.ui(|ui| ui.invoke_open_notice_event());
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 25);
    assert_eq!(f.agenda(), ["Dentist"]);
    assert!(f.notice().starts_with("Kept “Dentist”."), "the notice stays up");
}

#[test]
fn the_rail_lists_the_day_and_offers_the_way_back() {
    let f = open(a_week());
    f.app.advance(Duration::from_secs(5));
    let rows: Vec<(String, String, String)> = f.app.ui(|ui| {
        ui.get_agent_context().iter().map(|c| (c.id.to_string(), c.label.to_string(), c.source.to_string())).collect()
    });
    assert_eq!(
        rows,
        [("event:0".into(), "Standup".into(), "calendar".into()), ("event:1".into(), "Design review".into(), "calendar".into())]
    );
    let offered: Vec<String> = f.app.ui(|ui| ui.get_agent_suggestions().iter().map(|s| s.id.to_string()).collect());
    assert_eq!(offered, ["today"], "no question offered while no Mind can answer");
    f.app.ui(|ui| ui.invoke_next_month());
    f.quiet();
    f.app.ui(|ui| ui.invoke_agent_suggestion_activated("today".into()));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "September 2026");
}

/// The header's button and the rail's suggestion ask one question, about what the day holds, and
/// the answer is drawn on the card; the button asks even when the rail cannot.
#[test]
fn the_ai_button_asks_about_the_day_and_the_card_shows_the_answer() {
    let f = open_with(a_week(), FakeMind::new().answers([Ok("A light morning.".into()), Ok("Still light.".into())]));
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    assert!(f.app.ui(|ui| ui.get_ai_is_working()));
    assert_eq!(f.app.ui(|ui| ui.get_proposal().title.to_string()), "Your day");
    f.quiet();
    let card = f.app.ui(|ui| ui.get_proposal());
    assert_eq!((card.body.to_string(), card.source.to_string(), card.verb.to_string()),
        ("A light morning.".into(), "from September 2026 23".into(), "Close".into()));
    assert!(!f.app.ui(|ui| ui.get_ai_is_working()));
    let asked = f.mind.prompts();
    assert!(asked[0].starts_with("Here is my calendar for September 2026 23:\n- Standup (09:30 – 09:45)\n- Design review (14:00 – 15:30)"), "{asked:?}");
    f.app.ui(|ui| ui.invoke_ai_dismiss());
    assert_eq!(f.app.ui(|ui| ui.get_proposal().title.to_string()), "");

    // With the Mind reachable, the rail asks, and offers the same question.
    f.app.event(Event::ReachChanged(Reach::Ready));
    f.app.advance(Duration::from_secs(5));
    let offered: Vec<String> = f.app.ui(|ui| ui.get_agent_suggestions().iter().map(|s| s.id.to_string()).collect());
    assert_eq!(offered, ["explain", "today"]);
    f.app.ui(|ui| ui.invoke_agent_suggestion_activated("explain".into()));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_proposal().body.to_string()), "Still light.");
    f.app.ui(|ui| ui.invoke_proposal_dismissed());
    assert_eq!(f.app.ui(|ui| ui.get_proposal().title.to_string()), "");
}

/// A row in the rail is not a button here, so the window must not offer it as one.
#[test]
fn the_rail_rows_are_not_offered_as_buttons() {
    let sources = [
        include_str!("../view.rs"),
        include_str!("../lib.rs"),
        include_str!("../update/ai.rs"),
    ];
    assert!(sources.iter().all(|s| !s.contains("on_agent_context_activated")));
    assert!(!include_str!("../../../../crates/yantrik-ui-slint/ui/calendar.slint").contains("context-opens: true"));
}

/// Every callback the window declares is bound to something: a button that does nothing is how
/// this app came to have an AI button that logged a line and returned.
#[test]
fn every_callback_the_window_declares_is_wired() {
    let window = include_str!("../../ui/app.slint");
    let wired = [include_str!("../view.rs")].concat();
    // Bound by the framework's rail (`rail_window!`), or a row that is deliberately not a button.
    let by_the_rail = ["agent-suggestion-activated", "proposal-dismissed", "proposal-applied"];
    for line in window.lines().map(str::trim).filter(|l| l.starts_with("callback ")) {
        let name = line["callback ".len()..].split([' ', '(', ';']).next().unwrap_or_default();
        if by_the_rail.contains(&name) || name == "agent-context-activated" {
            continue;
        }
        let setter = format!("on_{}(", name.replace('-', "_"));
        assert!(wired.contains(&setter), "{name} is declared and nothing binds it");
    }
}

/// A Save that lands closes the form it saved, not one the person has opened since, and leaves the
/// new form's length alone (review of #921, finding 2).
#[test]
fn a_save_that_lands_leaves_a_form_the_person_reopened_alone() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.invoke_save_event("First".into(), "2026-09-24".into(), "08:00".into(), "".into()));
    f.app.ui(|ui| ui.invoke_cancel_event_form());
    f.app.ui(|ui| ui.invoke_cal_use_template("Deep work".into(), 120));
    f.app.ui(|ui| ui.set_event_title("Deep work, the draft".into()));
    f.quiet();
    assert!(f.app.ui(|ui| ui.get_show_event_form()), "the reopened form stays open");
    assert_eq!(f.app.ui(|ui| ui.get_event_title().to_string()), "Deep work, the draft");
    f.app.ui(|ui| ui.invoke_save_event("Deep work".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    f.quiet();
    let deep = f.store.stored().into_iter().find(|e| e.title == "Deep work").expect("stored");
    assert_eq!(deep.end, "2026-09-24T11:00:00", "the template's two hours");
    assert!(!f.app.ui(|ui| ui.get_show_event_form()));
}

/// A create the store kept is said as saved even when reading the calendar back afterwards
/// fails, so nobody is invited to save it twice (review of #921, finding 4).
#[test]
fn a_crash_reading_back_a_stored_event_still_says_it_was_saved() {
    let f = open(a_week());
    f.store.panicking("calendar.events");
    f.app.ui(|ui| ui.invoke_add_event());
    f.app.ui(|ui| ui.invoke_save_event("Once".into(), "2026-09-24".into(), "09:00".into(), "".into()));
    f.quiet();
    assert_eq!(f.notice(), "", "nothing says it did not finish");
    assert!(!f.app.ui(|ui| ui.get_show_event_form()), "closed: the store kept it");
    assert_eq!(f.store.stored().iter().filter(|e| e.title == "Once").count(), 1);
}

/// Putting the card away stops the AI button's spinner too.
#[test]
fn dismissing_the_card_stops_the_spinner() {
    let f = open(a_week());
    f.app.ui(|ui| ui.invoke_ai_explain_pressed());
    assert!(f.app.ui(|ui| ui.get_ai_is_working()));
    f.app.ui(|ui| ui.invoke_ai_dismiss());
    assert!(!f.app.ui(|ui| ui.get_ai_is_working()));
    f.quiet();
    assert_eq!(f.app.ui(|ui| ui.get_proposal().title.to_string()), "", "a late answer stays away");
}
