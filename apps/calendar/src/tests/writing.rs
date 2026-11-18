//! The intents that change what is stored, through the real dispatch: what each answers, read back
//! from the store; what each refuses, in the old words; and the own-creation rule of #201 and #332
//! with both sides established by the machine.

use serde_json::json;
use yantrik_testing::Call;

use super::{a_week, open};

#[test]
fn add_event_answers_with_the_id_the_store_gave_it_and_shows_its_day() {
    let f = open(a_week());
    let answer = f.act("add_event", json!({ "title": "  Call with Sam ", "date": "2026-09-30", "time": "15:00" })).unwrap();
    assert_eq!(
        answer,
        json!({ "added": "Call with Sam", "on": "2026-09-30 15:00", "id": "ev-1", "all_day": false, "reminder_minutes": 10 })
    );
    let stored = f.store.stored().into_iter().find(|e| e.id == "ev-1").unwrap();
    assert_eq!((stored.start.as_str(), stored.end.as_str()), ("2026-09-30T15:00:00", "2026-09-30T16:00:00"), "an hour");
    assert_eq!(stored.creator, None, "the person's own call: nobody's record");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 30, "the window is on the day it was added to");
    assert_eq!(f.agenda(), ["Call with Sam"]);
    assert_eq!(f.notice(), "");
}

#[test]
fn add_event_takes_a_length_a_reminder_or_a_whole_day() {
    let f = open(a_week());
    f.act("add_event", json!({ "title": "Quick", "date": "2026-09-24", "time": "23:30", "duration_min": 15, "reminder_minutes": 0 }))
        .unwrap();
    let quick = f.store.stored().into_iter().find(|e| e.title == "Quick").unwrap();
    assert_eq!(quick.end, "2026-09-24T23:45:00");
    assert_eq!(quick.reminder_minutes, 0);
    let answer = f.act("add_event", json!({ "title": "Holiday", "date": "2026-09-28", "all_day": true })).unwrap();
    assert_eq!(answer, json!({ "added": "Holiday", "on": "2026-09-28", "id": "ev-2", "all_day": true }));
    let holiday = f.store.stored().into_iter().find(|e| e.title == "Holiday").unwrap();
    assert!(holiday.is_all_day);
    assert_eq!((holiday.start.as_str(), holiday.end.as_str()), ("2026-09-28T00:00:00", "2026-09-28T23:59:00"));
}

#[test]
fn add_event_refuses_what_it_always_refused() {
    let f = open(a_week());
    for (args, said) in [
        (json!({ "title": " ", "date": "2026-09-24", "time": "10:00" }), "`title` is empty"),
        (json!({ "title": "A", "date": "24/09/2026", "time": "10:00" }), "`date` should look like 2026-09-06, not `24/09/2026`"),
        (json!({ "title": "A", "date": "2026-09-24" }), "`time` is needed for an event at a particular hour; set `all_day` for a whole day"),
        (json!({ "title": "A", "date": "2026-09-24", "time": "ten" }), "`time` should look like 14:30, not `ten`"),
        (
            json!({ "title": "A", "date": "2026-09-24", "all_day": true, "duration_min": 30 }),
            "an all-day event has no length to set; drop `duration_min` or drop `all_day`",
        ),
        (
            json!({ "title": "A", "date": "2026-09-24", "all_day": true, "reminder_minutes": 5 }),
            "an all-day event is never announced; drop `reminder_minutes` or drop `all_day`",
        ),
        (
            json!({ "title": "A", "date": "2026-09-24", "time": "10:00", "reminder_minutes": 99999 }),
            "`reminder_minutes` must be between 0 and 10080, and was 99999",
        ),
    ] {
        let e = f.act("add_event", args.clone()).unwrap_err();
        assert!(e.contains(said), "{args}: {e}");
    }
    assert!(f.store.asked_for("calendar.create_event").is_empty(), "nothing was sent to the store");
    assert_eq!(f.notice(), "", "a caller's mistake is not said in the window");
}

/// A failure the store or the times give is said in the window as well as answered: the person
/// watching is owed the reason too.
#[test]
fn a_save_that_fails_is_said_twice() {
    let f = open(a_week());
    let e = f.act("add_event", json!({ "title": "Late", "date": "2026-02-30", "time": "10:00" })).unwrap_err();
    assert_eq!(e, "`2026-02-30 10:00` is not a date and a time");
    assert_eq!(f.notice(), "Could not save “Late”: `2026-02-30 10:00` is not a date and a time");
    f.store.fail_once("calendar.create_event", -32001, "the disk is full");
    let e = f.act("add_event", json!({ "title": "Lunch", "date": "2026-09-24", "time": "12:00" })).unwrap_err();
    assert_eq!(e, "the disk is full");
    assert_eq!(f.notice(), "Could not save “Lunch”: the disk is full");
    assert_eq!(f.app.ui(|ui| ui.get_selected_day()), 23, "the window stays where it was");
}

#[test]
fn delete_event_by_id_or_by_title_and_date_and_never_a_guess() {
    let f = open(a_week().holding("ev-standup-2", "Standup", "2026-09-23T16:00:00", "2026-09-23T16:15:00"));
    assert_eq!(f.act("delete_event", json!({ "id": "ev-review" })).unwrap(), json!({ "deleted": "Design review", "id": "ev-review" }));
    assert_eq!(f.agenda(), ["Standup", "Standup"], "the window is drawn from the store again");
    assert_eq!(
        f.act("delete_event", json!({ "title": "dentist", "date": "2026-09-25" })).unwrap(),
        json!({ "deleted": "Dentist", "id": "ev-dentist" })
    );
    let e = f.act("delete_event", json!({ "title": "Standup", "date": "2026-09-23" })).unwrap_err();
    assert_eq!(e, "2 events on 2026-09-23 are called “Standup”; say which by id: ev-standup at 09:30:00, ev-standup-2 at 16:00:00");
    let e = f.act("delete_event", json!({ "title": "Gym", "date": "2026-09-23" })).unwrap_err();
    assert_eq!(e, "nothing called “Gym” on 2026-09-23; the day holds: “Standup”, “Standup”");
    let e = f.act("delete_event", json!({ "title": "Gym" })).unwrap_err();
    assert!(e.contains("name the event by `id`, or by `title` and `date` together"), "{e}");
    assert_eq!(f.store.stored().len(), 2);
}

#[test]
fn a_delete_the_store_did_not_make_is_said_twice() {
    let f = open(a_week());
    let e = f.act("delete_event", json!({ "id": "ev-nothing" })).unwrap_err();
    assert_eq!(e, "no event with id ev-nothing");
    assert_eq!(f.notice(), "Could not delete ev-nothing: no event with id ev-nothing");
}

/// #201: a caller may take off, unasked, the events it created itself, and nothing else; the
/// person is told which event was kept and who asked.
#[test]
fn delete_own_event_takes_only_the_callers_own() {
    let f = open(a_week());
    let made = f.app.act_as(Call::new("add_event", json!({ "title": "Arena run", "date": "2026-09-23", "time": "11:00" })).as_agent("mind:7")).unwrap();
    let id = f.settled(made).unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(f.store.stored().iter().find(|e| e.id == id).unwrap().creator.as_deref(), Some("agent mind:7"));

    let theirs = f.app.act_as(Call::new("delete_own_event", json!({ "id": "ev-standup" })).as_agent("mind:7")).unwrap();
    let e = f.settled(theirs).unwrap_err();
    assert_eq!(
        e,
        "ev-standup has no creator on record — it is older than the record, or a person made it in the window — \
         and this call is agent mind:7: only the caller that created an event may delete it without a person being \
         asked. Any event comes off through `delete_event`, which asks first"
    );
    assert_eq!(
        f.notice(),
        "Kept “Standup”. agent mind:7 asked to delete it, but on its own it may only delete events it made. \
         Delete it yourself if you want it gone."
    );
    assert_eq!(f.app.ui(|ui| ui.get_notice_event().to_string()), "ev-standup", "the way to it");
    assert!(f.store.asked_for("calendar.delete_event").is_empty());

    let other = f.app.act_as(Call::new("delete_own_event", json!({ "id": id })).as_agent("mind:8")).unwrap();
    let e = f.settled(other).unwrap_err();
    assert!(e.starts_with(&format!("{id} was created by agent mind:7 and this call is agent mind:8")), "{e}");

    let mine = f.app.act_as(Call::new("delete_own_event", json!({ "id": id })).as_agent("mind:7")).unwrap();
    assert_eq!(f.settled(mine).unwrap(), json!({ "deleted": "Arena run", "id": id }));
    assert_eq!(f.notice(), "");
}

#[test]
fn update_event_moves_an_event_and_answers_with_what_the_store_kept() {
    let f = open(a_week());
    let answer = f.act("update_event", json!({ "id": "ev-review", "date": "2026-10-02", "time": "10:00" })).unwrap();
    assert_eq!(
        answer,
        json!({
            "id": "ev-review", "title": "Design review", "start": "2026-10-02T10:00:00",
            "end": "2026-10-02T11:30:00", "all_day": false, "reminder_minutes": 10,
        })
    );
    assert_eq!(f.app.ui(|ui| ui.get_month_title().to_string()), "October 2026", "the window went with it");
    assert_eq!(f.agenda(), ["Design review"]);
    let e = f.act("update_event", json!({ "id": "ev-review" })).unwrap_err();
    assert!(e.contains("nothing to change"), "{e}");
    assert_eq!(f.notice(), format!("Could not change the event ev-review: {e}"));
    let e = f.act("update_event", json!({ "id": "ev-review", "time": "noon" })).unwrap_err();
    assert!(e.contains("`time` should look like 14:30, not `noon`"), "{e}");
    let e = f.act("update_event", json!({ "id": " " })).unwrap_err();
    assert!(e.contains("`id` is empty"), "{e}");
}

/// #332: editing anybody else's event is the sensitive door; the caller's own is standard.
#[test]
fn update_own_event_changes_only_the_callers_own() {
    let f = open(a_week());
    let theirs = f.app.act_as(Call::new("update_own_event", json!({ "id": "ev-dentist", "title": "Moved" })).as_agent("mind:7")).unwrap();
    let e = f.settled(theirs).unwrap_err();
    assert_eq!(
        e,
        "ev-dentist has no creator on record — it is older than the record, a person made it in the window, or a \
         sync stored it — and this call is agent mind:7: only the caller that created an event may change it \
         without a person being asked. Any event changes through `update_event`, which asks first"
    );
    assert!(f.notice().starts_with("Left “Dentist” as it was. agent mind:7 asked to change it"));
    assert_eq!(f.store.stored().iter().find(|e| e.id == "ev-dentist").unwrap().title, "Dentist");

    let made = f.app.act_as(Call::new("add_event", json!({ "title": "Mine", "date": "2026-09-24", "time": "09:00" })).as_agent("mind:7")).unwrap();
    let id = f.settled(made).unwrap()["id"].as_str().unwrap().to_string();
    let mine = f.app.act_as(Call::new("update_own_event", json!({ "id": id, "title": "Still mine" })).as_agent("mind:7")).unwrap();
    assert_eq!(f.settled(mine).unwrap()["title"], json!("Still mine"));
}

/// An agent in `ask` mode is refused the two sensitive doors before anything reaches the store;
/// the standard ones need no person.
#[test]
fn an_agent_in_ask_mode_is_refused_the_sensitive_doors() {
    let f = open(a_week());
    for (name, args) in [
        ("delete_event", json!({ "id": "ev-standup" })),
        ("update_event", json!({ "id": "ev-standup", "title": "X" })),
    ] {
        let refused = f.app.act_as(Call::new(name, args).as_agent("mind:7").in_mode("ask"));
        assert!(refused.is_err(), "{name} needs a person's Allow");
    }
    assert!(f.store.asked_for("calendar.delete_event").is_empty());
    assert!(f.store.asked_for("calendar.update_event").is_empty());
    let added = f.app.act_as(Call::new("add_event", json!({ "title": "Ok", "date": "2026-09-24", "time": "09:00" })).as_agent("mind:7").in_mode("ask"));
    assert!(added.is_ok(), "standard needs no card in ask mode");
}

// ── An answer owed to the Mind is its own (review of #921, finding 1) ──
//
// The person is on 7 October. A Mind's write is in flight; meanwhile the person presses the trash
// icon on a row and moves the window. Whatever lands in between, the Mind is answered about the
// event it named and nothing else.

/// Seven October holds one thing, which the person's trash icon takes off.
fn on_the_seventh() -> super::Fixture {
    let f = open(a_week().holding("ev-oct", "October thing", "2026-10-07T09:00:00", "2026-10-07T10:00:00"));
    f.act("show_date", json!({ "date": "2026-10-07" })).unwrap();
    assert_eq!(f.agenda(), ["October thing"]);
    f
}

#[test]
fn a_trash_press_never_becomes_the_answer_to_a_minds_delete() {
    let f = on_the_seventh();
    let mind = f.app.act("delete_event", json!({ "id": "ev-dentist" })).unwrap();
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.app.ui(|ui| ui.invoke_prev_month());
    assert_eq!(f.settled(mind).unwrap(), json!({ "deleted": "Dentist", "id": "ev-dentist" }));
    let left: Vec<String> = f.store.stored().into_iter().map(|e| e.id).collect();
    assert_eq!(left, ["ev-standup", "ev-review"], "both deletes happened, each answered as itself");
}

#[test]
fn a_trash_press_never_becomes_the_answer_to_a_minds_add() {
    let f = on_the_seventh();
    let mind = f.app.act("add_event", json!({ "title": "Planning", "date": "2026-10-01", "time": "09:00" })).unwrap();
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.app.ui(|ui| ui.invoke_switch_view(1));
    assert_eq!(
        f.settled(mind).unwrap(),
        json!({ "added": "Planning", "on": "2026-10-01 09:00", "id": "ev-1", "all_day": false, "reminder_minutes": 10 })
    );
}

#[test]
fn a_trash_press_never_becomes_the_answer_to_a_minds_update() {
    let f = on_the_seventh();
    let mind = f.app.act("update_event", json!({ "id": "ev-dentist", "date": "2026-10-01" })).unwrap();
    f.app.ui(|ui| ui.invoke_delete_event(0));
    f.app.ui(|ui| ui.invoke_switch_view(1));
    let answer = f.settled(mind).unwrap();
    assert_eq!((answer["id"].clone(), answer["title"].clone()), (json!("ev-dentist"), json!("Dentist")));
    assert_eq!(answer["start"], json!("2026-10-01T13:00:00"));
}
