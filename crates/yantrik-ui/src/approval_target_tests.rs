//! What the app names a call's target as, and when a card may not be allowed without it.

use super::*;
use serde_json::json;

fn reply(target: serde_json::Value) -> serde_json::Value {
    json!({ "app": "calendar", "action": "delete_event", "target": target })
}

fn id_handle() -> Vec<String> {
    vec!["id".to_string()]
}

fn resolved(rows: &[(&str, &str)], handles: &[&str]) -> Named {
    Named::Resolved(Resolved {
        rows: rows.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect(),
        series: false,
        identity: "e1".into(),
        handles: handles.iter().map(|h| h.to_string()).collect(),
    })
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
        "identity": "01a0c718",
        "handles": ["title", "whatever-the-answer-says"],
    })), &id_handle());
    let Named::Resolved(t) = named else { panic!("named: {named:?}") };
    assert_eq!(t.rows.len(), 2, "unlabelled and non-string rows are dropped: {:?}", t.rows);
    assert_eq!(t.rows[0], ("Event".to_string(), "Dentist".to_string()));
    assert!(t.series);
    assert_eq!(t.identity, "01a0c718");
    assert_eq!(t.handles, ["id"], "the handles the action declared, never the ones an answer lists");

    let many: Vec<_> = (0..20).map(|i| json!({"label": format!("L{i}"), "value": "x".repeat(1000)})).collect();
    let Named::Resolved(t) = from_reply(&reply(json!({ "rows": many, "identity": "e1" })), &id_handle()) else { panic!() };
    assert_eq!(t.rows.len(), ROWS, "at most ROWS rows reach the record");
    assert!(t.rows.iter().all(|(_, v)| v.chars().count() <= 241), "values are cut on arrival");
}

#[test]
fn an_answer_that_names_nothing_is_unresolved() {
    for answer in [
        reply(serde_json::Value::Null),
        reply(json!({})),
        reply(json!({"rows": []})),
        reply(json!({"rows": [{"label": "Event"}], "identity": "e1"})),
        // Rows and no identity: the app cannot tell the thing apart, so the grant could not be
        // held to it.
        reply(json!({"rows": [{"label": "Event", "value": "Dentist"}]})),
        reply(json!({"rows": [{"label": "Event", "value": "Dentist"}], "identity": ""})),
        reply(json!({"rows": [{"label": "Event", "value": "Dentist"}], "identity": 7})),
        json!({}),
    ] {
        assert_eq!(from_reply(&answer, &id_handle()), Named::Unresolved, "{answer}");
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
    let named = resolved(&[("Event", "Dentist")], &["id"]);
    assert!(blocked(&Named::Unresolved, true));
    assert!(!blocked(&Named::Unresolved, false), "non-destructive cards keep today's behaviour");
    assert!(!blocked(&named, true));
    assert!(!blocked(&Named::NotAsked, true));
    assert_eq!(unavailable("delete"), "Target details unavailable \u{00b7} the app could not say what this would delete");
}

#[test]
fn the_shells_own_namer_maps_like_the_socket_answer() {
    let name = vec!["name".to_string()];
    let t = Target { rows: vec![("Path".into(), "~/a.txt".into())], series: false, identity: "dev 1 inode 2".into() };
    let Named::Resolved(r) = from_local(Ok(Some(t)), "files_delete", &name) else { panic!("named") };
    assert_eq!((r.identity.as_str(), r.handles.as_slice()), ("dev 1 inode 2", name.as_slice()));
    assert_eq!(identity(&Named::Resolved(r)).as_deref(), Some("dev 1 inode 2"), "what the grant hands back");
    assert_eq!(identity(&Named::NotAsked), None);
    let unidentified = Target { rows: vec![("Path".into(), "~/a.txt".into())], ..Target::default() };
    assert_eq!(from_local(Ok(Some(unidentified)), "files_delete", &name), Named::Unresolved, "no identity, no name");
    assert_eq!(from_local(Ok(None), "files_delete", &name), Named::Unresolved);
    assert_eq!(from_local(Err("no namer".into()), "files_delete", &name), Named::Unresolved, "an object action must be named");
    assert_eq!(from_local(Err("no namer".into()), "apply_update", &name), Named::NotAsked);
}

/// The raw id leaves the card's face only when the app named what it points at, and only the
/// argument the app said its rows stand for, matched by key.
#[test]
fn the_face_leaves_out_only_the_handles_the_app_named() {
    let rows = |v: &serde_json::Value| crate::approvals::args_rows(v);
    let args = json!({"id": "01a0c718-3931", "title": "Dentist (moved)"});
    let named = resolved(&[("Event", "Dentist")], &["id"]);
    assert_eq!(face_args(&named, &args, rows), ["title: Dentist (moved)"]);
    assert_eq!(face_args(&Named::Unresolved, &args, rows), rows(&args), "unnamed: every argument stays");
    assert!(face_args(&named, &json!({"id": "x"}), rows).is_empty(), "only the id: nothing left on the face");
    assert!(face_args(&named, &json!({"id": 4242}), rows).is_empty(), "a number is an identifier too");

    // A declared handle is hidden only while it is a scalar identifier: a flag, a list or an
    // object under that name says how, not which, and stays on the face.
    for (value, shown) in [(json!(true), "id: true"), (json!(["a"]), "id: [\"a\"]")] {
        let face = face_args(&named, &json!({ "id": value }), rows);
        assert_eq!(face.len(), 1, "{face:?}");
        assert!(face[0].starts_with(shown.split(':').next().unwrap()), "{face:?}");
    }
    let flag = resolved(&[("Process", "firefox, pid 4242")], &["pid", "force"]);
    assert_eq!(face_args(&flag, &json!({"pid": 4242, "force": true}), rows), ["force: true"], "never a flag");
}

/// M1: a name line longer than the card draws is cut — so the face keeps the whole bound
/// argument, the handle included, under "Exactly:" (#639's pinned rule).
#[test]
fn a_name_the_card_cuts_keeps_every_argument_on_the_face() {
    let rows = |v: &serde_json::Value| crate::approvals::args_rows(v);
    let long = format!("{} \u{00b7} ~/{}", "n".repeat(90), "n".repeat(90));
    let cut = resolved(&[("Path", long.as_str())], &["name"]);
    assert!(first_row_cut(&cut));
    let args = json!({ "name": "n".repeat(90) });
    assert_eq!(face_args(&cut, &args, rows), rows(&args));

    // Escaping counts: a short name whose control characters grow past the bound when escaped.
    let escaped = format!("{}{}", "a".repeat(100), "\u{202E}".repeat(10));
    assert!(escaped.chars().count() <= VALUE_CHARS);
    assert!(first_row_cut(&resolved(&[("Path", escaped.as_str())], &["name"])), "cut after escaping");
    assert!(!first_row_cut(&resolved(&[("Path", "thesis \u{00b7} ~/tmp/thesis")], &["name"])));
}
