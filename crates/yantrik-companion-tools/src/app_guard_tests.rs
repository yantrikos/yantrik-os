//! `app_action` does not reach an effect that belongs to one of the four questions (#1313, round 5, H1).

use super::*;
use crate::fake_shell::{Answer, FakeShell, First};

fn nowhere() -> std::path::PathBuf {
    std::env::temp_dir().join("yantrik-app-guard-no-such-directory")
}

#[test]
fn every_refused_pair_is_refused_and_says_what_to_use_instead() {
    assert!(REFUSED.len() >= 20, "the list is not empty");
    for r in REFUSED {
        let said = refusal_in(&nowhere(), r.app, r.action).unwrap_or_else(|| panic!("{}.{} is not refused", r.app, r.action));
        assert!(said.starts_with("Refused:") && said.contains("Nothing was changed") && said.contains(r.question.action()), "{said}");
        match r.instead {
            Some(tool) => assert!(said.contains(&format!("Use `{tool}`")), "{said}"),
            None => assert!(said.contains("the person can do it in the"), "{said}"),
        }
    }
}

#[test]
fn the_calendar_names_from_the_review_are_refused_with_the_tool_that_asks() {
    let said = refusal_in(&nowhere(), "calendar", "delete_event").expect("refused");
    assert!(said.contains("`calendar_delete_event`") && said.contains("delete_for_good"), "{said}");
    let said = refusal_in(&nowhere(), "calendar", "update_event").expect("refused");
    assert!(said.contains("`calendar_update_event`") && said.contains("change_item"), "{said}");
}

#[test]
fn a_caller_s_spelling_of_the_app_is_not_a_way_round() {
    for spelling in ["Calendar", " calendar ", "CALENDAR"] {
        assert!(refusal_in(&nowhere(), spelling, "delete_event").is_some(), "{spelling}");
    }
    for spelling in ["text-editor", "text_editor", "Text Editor", "TEXT-EDITOR"] {
        assert!(refusal_in(&nowhere(), spelling, "save").is_some(), "{spelling}");
    }
}

/// An alias is a symlink to the app's own socket (`app-<alias>.sock` to `app-<id>.sock`): a call by it
/// is a call to the app, and is read as one.
#[cfg(unix)]
#[test]
fn an_alias_that_links_to_the_apps_socket_is_read_as_the_app() {
    let dir = std::env::temp_dir().join(format!("yantrik-app-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app-calendar.sock"), "").unwrap();
    std::os::unix::fs::symlink("app-calendar.sock", dir.join("app-agenda.sock")).unwrap();
    assert_eq!(linked_id_in(&dir, "agenda").as_deref(), Some("calendar"));
    assert!(refusal_in(&dir, "agenda", "delete_event").is_some(), "the alias reaches the same effect");
    assert!(refusal_in(&dir, "agenda", "add_event").is_none(), "and only the guarded actions are refused by it");
    assert!(refusal_in(&dir, "calendar", "events_between").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_action_of_the_same_name_in_another_app_is_not_refused() {
    for (app, action) in [("notes", "delete_event"), ("weather", "save"), ("calendar", "add_event"), ("calendar", "events_between"), ("spreadsheet", "save"), ("shell", "save")] {
        assert_eq!(refusal_in(&nowhere(), app, action), None, "{app}.{action}");
    }
}

#[test]
fn the_two_lists_hold_each_pair_once_and_share_none() {
    let mut seen = std::collections::HashSet::new();
    for r in REFUSED {
        assert!(seen.insert(format!("{}.{}", r.app, r.action)), "{}.{} is listed twice", r.app, r.action);
    }
    for group in REVIEWED {
        assert!(group.because.len() > 20, "a reason says something: {}", group.because);
        for pair in group.actions {
            assert!(seen.insert((*pair).to_string()), "{pair} is on a list twice, or on both");
        }
    }
}

/// The road itself: `app_action` on the calendar's own names changes nothing, starts nothing and
/// raises no card, whatever the mode would have said.
#[test]
fn the_app_action_road_refuses_before_it_starts_or_asks_anything() {
    use crate::{PermissionLevel, ToolContext};
    let db = yantrikdb_core::YantrikDB::new(":memory:", 384).expect("in-memory database");
    let ctx = ToolContext { db: &db, max_permission: PermissionLevel::Sensitive, registry_metadata: None, task_manager: None, incognito: false, agent_spawner: None };
    let shell = FakeShell::session(First::NotNeeded, Answer::Granted);
    for (app, action) in [("calendar", "delete_event"), ("calendar", "update_event"), ("Calendar", "delete_event"), ("text-editor", "set_content"), ("notes", "set_content")] {
        let said = crate::app_act::gated_act(&ctx, app, action, &serde_json::json!({ "id": "x" }), "").expect_err("refused");
        assert!(said.starts_with("Refused:") && said.contains("Nothing was changed"), "{app}.{action}: {said}");
    }
    assert!(shell.calls().is_empty(), "no shell call, no card: {:?}", shell.calls());
}
