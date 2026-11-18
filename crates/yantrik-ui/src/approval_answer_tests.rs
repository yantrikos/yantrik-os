//! `answered` says how an approval was reached, and only the shell sets it (#1311, round 2).

use super::*;

#[test]
fn a_request_that_needed_no_card_says_who_answered() {
    assert_eq!(not_needed_by(false, false), "mode");
    assert_eq!(not_needed_by(false, true), "session_rule");
    assert_eq!(not_needed_by(true, true), "standing_rule", "the administrator's rule is named before the person's");
}

#[test]
fn only_a_granted_or_spent_request_was_pressed() {
    for status in [Status::Granted, Status::Consumed] {
        assert_eq!(status_json("a-1", status)["answered"], "pressed", "{status:?}");
    }
    for status in [Status::Pending, Status::Denied, Status::Expired] {
        assert!(status_json("a-1", status).get("answered").is_none(), "{status:?} was not an Allow");
    }
}

#[test]
fn a_spent_grant_says_pressed_and_names_what_it_authorises() {
    let said = consumed_json("a-1", "shell", "delete_for_good");
    assert_eq!((said["answered"].as_str(), said["authorises"].as_str(), said["consumed"].as_bool()), (Some("pressed"), Some("shell.delete_for_good"), Some(true)));
}

/// Nothing a caller sends reaches these answers: the field is built from the store's state, the
/// mode and the rules, in this file, and the request handlers answer through it.
#[test]
fn the_field_is_set_in_this_file_and_read_from_no_argument() {
    let own = include_str!("approval_answer.rs").split("#[cfg(test)]").next().unwrap();
    assert!(!own.contains("args.get(") && !own.contains("args["), "an answer is built from no caller's argument");
    let control = include_str!("control_approvals.rs").split("#[cfg(test)]").next().unwrap();
    assert!(control.contains("approval_answer::not_needed("), "request_approval answers through it");
    assert!(control.contains("approval_answer::status_json("), "approval_status answers through it");
    assert!(control.contains("approval_answer::consumed_json("), "consume_approval answers through it");
    assert!(!control.contains("\"answered\""), "no other place writes the field");
}
