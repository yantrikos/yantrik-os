//! `calendar_delete_event` and `calendar_update_event` ask before the backend's delete and
//! update (#1311).

use yantrik_companion_tools::asks_scenarios::{check_all, Scenario};
use yantrik_companion_tools::fake_shell::{Answer, FakeShell, First};
use yantrik_ipc_contracts::calendar::UpdateEventParams;

use super::calendar;
use super::tests_support::{event, q, Calendar};

#[test]
fn deleting_an_event_asks_with_its_title_time_and_people() {
    check_all(&Scenario {
        action: "delete_for_good",
        grade: "sensitive",
        card: serde_json::json!({
            "tool": "calendar_delete_event", "title": q("Dentist"), "start": q("2026-11-18T09:00:00"),
            "end": q("2026-11-18T10:00:00"), "people": q("Dr Rao <rao@example.com>"), "location": q("Main Street 4"),
        }),
        call: &|variant, effect| {
            let title = match variant {
                0 => "Dentist".to_string(),
                1 => "Haircut".to_string(),
                _ => "d".repeat(300),
            };
            let existing = event(&title);
            let cal = Calendar::with(existing.clone());
            match calendar::delete(&cal, "ev1", &existing, false) {
                Ok(()) => {
                    assert_eq!((*cal.deletes.lock().unwrap(), cal.stored().is_none()), (1, true), "the event is gone, once");
                    effect()
                }
                Err(why) => {
                    assert_eq!((*cal.deletes.lock().unwrap(), cal.stored().is_some()), (0, true), "the event is kept");
                    Err(why)
                }
            }
        },
    });
}

#[test]
fn an_event_with_a_google_copy_says_it_goes_too() {
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let mut existing = event("Dentist");
    existing.remote_id = Some("g-1".into());
    let cal = Calendar::with(existing.clone());
    calendar::delete(&cal, "ev1", &existing, true).expect("allowed");
    assert_eq!(shell.cards()[0]["args_json"]["also"], "its copy on Google Calendar is deleted too");
    // With no Google account connected the copy is not touched, and the card does not claim it.
    let cal = Calendar::with(existing.clone());
    calendar::delete(&cal, "ev1", &existing, false).expect("allowed");
    assert!(shell.cards()[1]["args_json"].get("also").is_none());
}

#[test]
fn changing_an_event_shows_each_field_before_and_after() {
    let change = |before: &str, after: &str| format!("{} → {}", q(before).as_str().unwrap(), q(after).as_str().unwrap());
    check_all(&Scenario {
        action: "change_item",
        grade: "standard",
        card: serde_json::json!({
            "tool": "calendar_update_event", "event": q("Dentist"),
            "title": change("Dentist", "Dentist (moved)"), "start": change("2026-11-18T09:00:00", "2026-11-19T09:00:00"),
        }),
        call: &|variant, effect| {
            let title = match variant {
                0 => "Dentist (moved)".to_string(),
                1 => "Dentist, Tuesday".to_string(),
                _ => "d".repeat(300),
            };
            let cal = Calendar::with(event("Dentist"));
            let params = UpdateEventParams {
                id: "ev1".into(),
                title: Some(title.clone()),
                start: Some("2026-11-19T09:00:00".into()),
                ..Default::default()
            };
            match calendar::update(&cal, &params) {
                Ok(updated) => {
                    assert_eq!((updated.title, *cal.updates.lock().unwrap()), (title, 1));
                    effect()
                }
                Err(why) => {
                    assert_eq!(*cal.updates.lock().unwrap(), 0, "nothing was updated");
                    assert_eq!(cal.stored().unwrap().title, "Dentist");
                    Err(why)
                }
            }
        },
    });
}

#[test]
fn an_event_that_cannot_be_read_is_an_error_and_nobody_is_asked() {
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let cal = Calendar::with(event("Dentist"));
    let err = calendar::update(&cal, &UpdateEventParams { id: "gone".into(), title: Some("x".into()), ..Default::default() }).unwrap_err();
    assert!(err.contains("no event gone"), "{err}");
    assert!(shell.calls().is_empty());
}
