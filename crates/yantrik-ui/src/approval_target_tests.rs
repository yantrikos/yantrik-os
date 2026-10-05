//! What the app names a call's target as, and when a card may not be allowed without it.

use super::*;
use serde_json::json;

fn reply(target: serde_json::Value) -> serde_json::Value {
    json!({ "app": "calendar", "action": "delete_event", "target": target })
}

#[test]
fn the_apps_rows_are_read_as_it_wrote_them_and_bounded() {
    let named = from_reply(&reply(json!({
        "rows": [
            {"label": "Event", "value": "Dentist"},
            {"label": "When", "value": "Fri 25 Sep 2026, 13:00\u{2013}14:00 (UTC+01:00)"},
            {"label": "", "value": "a row with no label is not a row"},
            {"label": "Calendar", "value": 7},
        ],
        "series": true,
        "handles": ["id"],
    })));
    let Named::Resolved(t) = named else { panic!("named: {named:?}") };
    assert_eq!(t.rows.len(), 2, "unlabelled and non-string rows are dropped: {:?}", t.rows);
    assert_eq!(t.rows[0], ("Event".to_string(), "Dentist".to_string()));
    assert!(t.series);
    assert_eq!(t.handles, ["id"]);

    let many: Vec<_> = (0..20).map(|i| json!({"label": format!("L{i}"), "value": "x".repeat(1000)})).collect();
    let Named::Resolved(t) = from_reply(&reply(json!({ "rows": many }))) else { panic!() };
    assert_eq!(t.rows.len(), ROWS, "at most ROWS rows reach the record");
    assert!(t.rows.iter().all(|(_, v)| v.chars().count() <= 241), "values are cut on arrival");
}

#[test]
fn an_answer_that_names_nothing_is_unresolved() {
    for answer in [
        reply(serde_json::Value::Null),
        reply(json!({})),
        reply(json!({"rows": []})),
        reply(json!({"rows": [{"label": "Event"}]})),
        json!({}),
    ] {
        assert_eq!(from_reply(&answer), Named::Unresolved, "{answer}");
    }
}

#[test]
fn which_actions_take_a_named_thing_away() {
    for action in ["delete_event", "remove", "kill_process", "files_delete", "wifi_forget", "delete"] {
        assert!(acts_on_object(action), "{action}");
    }
    for action in ["update_event", "apply_update", "installer_reboot", "agent_run", "set_wifi", "power_off"] {
        assert!(!acts_on_object(action), "{action}");
    }
    assert_eq!(verb_of("files_delete"), "delete");
    assert_eq!(verb_of("kill_process"), "kill");
    assert!(needed("update_event", true), "a declared namer is asked whatever the verb");
    assert!(!needed("update_event", false));
}

/// The rule: Decline only, on a destructive card whose target was asked for and not named. A
/// card that is not destructive, or whose target was named, is what it was.
#[test]
fn only_a_destructive_card_with_an_unnamed_target_is_blocked() {
    let named = Named::Resolved(Target { rows: vec![("Event".into(), "Dentist".into())], ..Target::default() });
    assert!(blocked(&Named::Unresolved, true));
    assert!(!blocked(&Named::Unresolved, false), "non-destructive cards keep today's behaviour");
    assert!(!blocked(&named, true));
    assert!(!blocked(&Named::NotAsked, true));
    assert_eq!(unavailable("delete"), "Target details unavailable \u{00b7} the app could not say what this would delete");
}

#[test]
fn the_shells_own_namer_maps_like_the_socket_answer() {
    let t = Target { rows: vec![("Path".into(), "~/a.txt".into())], series: false, handles: vec!["name".into()] };
    assert!(matches!(from_local(Ok(Some(t)), "files_delete"), Named::Resolved(_)));
    assert_eq!(from_local(Ok(None), "files_delete"), Named::Unresolved);
    assert_eq!(from_local(Err("no namer".into()), "files_delete"), Named::Unresolved, "an object action must be named");
    assert_eq!(from_local(Err("no namer".into()), "apply_update"), Named::NotAsked);
}

/// The raw id leaves the card's face only when the app named what it points at, and only the
/// argument the app said its rows stand for, matched by key.
#[test]
fn the_face_leaves_out_only_the_handles_the_app_named() {
    let rows = |v: &serde_json::Value| crate::approvals::args_rows(v);
    let args = json!({"id": "01a0c718-3931", "title": "Dentist (moved)"});
    let named = Named::Resolved(Target {
        rows: vec![("Event".into(), "Dentist".into())],
        series: false,
        handles: vec!["id".into()],
    });
    assert_eq!(face_args(&named, &args, rows), ["title: Dentist (moved)"]);
    assert_eq!(face_args(&Named::Unresolved, &args, rows), rows(&args), "unnamed: every argument stays");
    assert!(face_args(&named, &json!({"id": "x"}), rows).is_empty(), "only the id: nothing left on the face");
}
