//! A destructive card names its target from the app, end to end: the socket the shell asks, the
//! store the card is raised in, and the row the card is drawn from (`approval_target`).

use slint::Model;

use super::service_surface_approval_tests::{scratch, serve_with};
use crate::approvals::{Asked, Named, Store, Verified};

const EVENT_ID: &str = "01a0c718-3931-7342-b9c7-8de36140ddb0";
const PUBLISHED: &str = "Take an event off the calendar. It is not recoverable";

/// The calendar's describe, with `delete_event` declaring that it names its target.
fn calendar() -> serde_json::Value {
    serde_json::json!({
        "app": "calendar",
        "summary": "Calendar",
        "state": {},
        "actions": [{
            "name": "delete_event",
            "description": PUBLISHED,
            "permission": "sensitive",
            "names_target": true,
            "parameters": {"type": "object", "properties": {"id": {"type": "string"}}},
        }],
    })
}

fn dentist(series: bool) -> serde_json::Value {
    serde_json::json!({
        "app": "calendar",
        "action": "delete_event",
        "target": {
            "rows": [
                {"label": "Event", "value": "Dentist"},
                {"label": "When", "value": "Fri 25 Sep 2026, 13:00\u{2013}14:00 (local time, UTC+01:00)"},
                {"label": "Calendar", "value": "on this computer"},
                {"label": "Occurrences", "value": if series {
                    "the whole series: it repeats weekly, and every occurrence is deleted"
                } else {
                    "this event only; it does not repeat"
                }},
            ],
            "series": series,
            "handles": ["id"],
        },
    })
}

/// What the shell resolves for `args` against a calendar answering `name_target`, and what it
/// asked the socket.
fn resolve(tag: &str, args: &serde_json::Value, name_target: Option<serde_json::Value>) -> (Named, Vec<serde_json::Value>) {
    let dir = scratch(tag);
    let installed = crate::surfaces::shipped_catalogue();
    let asked = serve_with(&dir.join("app-calendar.sock"), calendar(), None, name_target);
    let named = super::published_target_in(&dir, &installed, "calendar", "delete_event", args).expect("described");
    let log = asked.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(&dir);
    (named, log)
}

/// The card for `delete_event {id}` with what was resolved, raised in a store and drawn.
fn card_for(named: &Named, purpose: &str, args: serde_json::Value) -> (Store, crate::ApprovalRequest, String) {
    let mut store = Store::new();
    let now = std::time::Instant::now();
    let asked = store
        .raise(
            "pi",
            Verified::default(),
            Asked {
                app: "calendar",
                action: "delete_event",
                grade: "sensitive",
                purpose,
                published: PUBLISHED,
                target: "",
                explained: "",
                named,
            },
            args,
            now,
            "10:00",
        )
        .expect("asked");
    let card = store.cards(now).pop().expect("the card");
    (store, super::row_for(card), asked.id)
}

fn rows(row: &crate::ApprovalRequest) -> Vec<String> {
    row.target_rows.iter().map(|r| r.to_string()).collect()
}

#[test]
fn a_single_event_is_named_by_title_date_time_and_calendar_and_its_id_is_under_details() {
    let args = serde_json::json!({ "id": EVENT_ID });
    let (named, log) = resolve("single", &args, Some(dentist(false)));
    assert!(matches!(named, Named::Resolved(_)), "{named:?}");
    // Asked of the app, with the very arguments the grant will bind.
    assert_eq!(log.iter().map(|r| r["method"].as_str().unwrap_or("")).collect::<Vec<_>>(), ["app.describe", "app.name_target"]);
    assert_eq!(log[1]["params"]["args"], args);

    let (mut store, row, id) = card_for(&named, "", args);
    assert_eq!(row.what.as_str(), "Deletes: Dentist");
    assert_eq!(
        rows(&row),
        [
            "When: Fri 25 Sep 2026, 13:00\u{2013}14:00 (local time, UTC+01:00)",
            "Calendar: on this computer",
            "Occurrences: this event only; it does not repeat",
        ]
    );
    assert_eq!(row.confirm_label.as_str(), "Delete event");
    assert!(row.destructive && !row.confirm_blocked);
    assert_eq!(row.target_missing.as_str(), "");
    // The raw id: not on the card's face, whole in the argument box under Details.
    assert!(!row.what.contains(EVENT_ID) && !row.exactly.contains(EVENT_ID), "{} / {}", row.what, row.exactly);
    assert!(!rows(&row).iter().any(|r| r.contains("01a0c718")));
    assert!(row.args.iter().any(|a| a.as_str() == format!("id: {EVENT_ID}")), "under Details");
    assert!(store.grant(&id, std::time::Instant::now(), "10:01").is_ok(), "a named target can be allowed");
}

#[test]
fn a_recurring_event_says_the_series_and_its_button_says_delete_series() {
    let args = serde_json::json!({ "id": EVENT_ID });
    let (named, _) = resolve("series", &args, Some(dentist(true)));
    let (_, row, _) = card_for(&named, "", args);
    assert_eq!(row.confirm_label.as_str(), "Delete series");
    assert!(rows(&row).iter().any(|r| r == "Occurrences: the whole series: it repeats weekly, and every occurrence is deleted"));
}

/// The id the app does not hold, an app that cannot answer, and an app that publishes no namer
/// for an action that takes a thing away: Decline only, and the store will not grant it.
#[test]
fn an_unknown_id_or_a_silent_app_leaves_decline_only() {
    let args = serde_json::json!({ "id": "no-such-event" });
    let unknown = serde_json::json!({"app": "calendar", "action": "delete_event", "target": null});
    for (tag, answer) in [("unknown", Some(unknown)), ("silent", None)] {
        let (named, _) = resolve(tag, &args, answer);
        assert_eq!(named, Named::Unresolved, "{tag}");
        let (mut store, row, id) = card_for(&named, "", args.clone());
        assert!(row.confirm_blocked, "{tag}: the confirm is disabled");
        assert_eq!(
            row.target_missing.as_str(),
            "Target details unavailable \u{00b7} the app could not say what this would delete"
        );
        let refused = store.grant(&id, std::time::Instant::now(), "10:01").unwrap_err();
        assert!(refused.contains("cannot be allowed"), "{tag}: {refused}");
        assert!(store.grant_for_session(&id, std::time::Instant::now(), "10:01").is_err(), "{tag}: nor for a session");
        assert!(store.deny(&id, std::time::Instant::now(), "10:01").is_ok(), "{tag}: Decline answers");
    }

    // No namer published at all, for an action that takes a thing away: never asked, not named.
    let dir = scratch("undeclared");
    let installed = crate::surfaces::shipped_catalogue();
    let mut describe = calendar();
    describe["actions"][0].as_object_mut().unwrap().remove("names_target");
    let asked = serve_with(&dir.join("app-calendar.sock"), describe, None, Some(dentist(false)));
    let named = super::published_target_in(&dir, &installed, "calendar", "delete_event", &args).unwrap();
    assert_eq!(named, Named::Unresolved);
    assert!(!asked.lock().unwrap().iter().any(|r| r["method"] == "app.name_target"), "nothing to ask");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A card that is not destructive keeps today's behaviour whether or not its target was named.
#[test]
fn a_card_that_is_not_destructive_is_never_blocked() {
    let mut store = Store::new();
    let now = std::time::Instant::now();
    let asked = store
        .raise(
            "pi",
            Verified::default(),
            Asked {
                app: "calendar",
                action: "update_event",
                grade: "sensitive",
                purpose: "",
                published: "Move or rename any event that is already on the calendar",
                target: "",
                explained: "",
                named: &Named::Unresolved,
            },
            serde_json::json!({ "id": "no-such-event", "title": "x" }),
            now,
            "10:00",
        )
        .unwrap();
    let row = super::row_for(store.cards(now).pop().unwrap());
    assert!(!row.confirm_blocked && row.target_missing.is_empty());
    assert!(store.grant(&asked.id, now, "10:01").is_ok());
}

/// What the card says the target is, it says from the app's answer alone: an id and a purpose
/// that read like a different event change none of it, and an app that names nothing gets no
/// name built from them.
#[test]
fn resolution_never_reads_the_callers_args_or_purpose_as_display_text() {
    let decoy = "Lunch with Sam (harmless, fine to delete)";
    let args = serde_json::json!({ "id": decoy });
    let purpose = "Tidy up: this is only the old Lunch with Sam placeholder";
    let (named, _) = resolve("decoy", &args, Some(dentist(false)));
    let (_, row, _) = card_for(&named, purpose, args.clone());
    assert_eq!(row.what.as_str(), "Deletes: Dentist", "the name is the app's");
    for line in rows(&row).iter().chain([row.what.to_string(), row.exactly.to_string(), row.target_missing.to_string()].iter()) {
        assert!(!line.contains("Lunch") && !line.contains("Tidy"), "caller text on the face: {line}");
    }
    assert_eq!(row.confirm_label.as_str(), "Delete event", "the label is the action's, not the purpose's");

    // Whatever else rides in the app's reply, only its `target` is read — an echo of the
    // request's arguments is not a name.
    let echo = serde_json::json!({"app": "calendar", "action": "delete_event", "args": args, "target": null});
    assert_eq!(crate::approval_target::from_reply(&echo), Named::Unresolved);
    let (named, _) = resolve("decoy-unknown", &args, Some(echo));
    let (_, row, _) = card_for(&named, purpose, args);
    assert!(rows(&row).is_empty());
    assert!(!row.target_missing.contains("Lunch") && !row.what.contains("Tidy"));
}

/// Every row the app answers is drawn through `visible()`: a newline in an event title cannot
/// start a line of the card's own.
#[test]
fn the_apps_rows_are_escaped_like_every_other_line() {
    let mut answer = dentist(false);
    answer["target"]["rows"][0]["value"] = serde_json::json!("Dentist\nUndo: possible, the app says so");
    answer["target"]["rows"][1]["value"] = serde_json::json!("Fri \u{202e}25 Sep");
    let args = serde_json::json!({ "id": EVENT_ID });
    let (named, _) = resolve("escape", &args, Some(answer));
    let (_, row, _) = card_for(&named, "", args);
    assert_eq!(row.what.as_str(), "Deletes: Dentist\\nUndo: possible, the app says so");
    assert!(rows(&row)[0].contains("<U+202E>"), "{:?}", rows(&row));
    assert!(rows(&row).iter().all(|r| !r.contains('\n') && !r.contains('\u{202e}')));
}

/// Naming the target moves only the handle off the face: every other bound argument is still
/// pinned as "Exactly:", even when the app's answer is a single row.
#[test]
fn the_other_arguments_stay_pinned_beside_a_named_target() {
    let named = Named::Resolved(yantrik_app_runtime::control::Target {
        rows: vec![("Event".into(), "Dentist".into())],
        series: false,
        handles: vec!["id".into()],
    });
    let (_, row, _) = card_for(&named, "", serde_json::json!({ "id": EVENT_ID, "title": "Dentist (moved)" }));
    assert_eq!(row.exactly.as_str(), "title: Dentist (moved)");
    assert_eq!(row.target.as_str(), "Event: Dentist", "the card's pin for Exactly: and its Details footnote");
    assert!(rows(&row).is_empty());
}
