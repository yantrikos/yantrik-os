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

/// L8 (#1313): not only `control_approvals.rs`. No non-test file of the crate but
/// `approval_answer.rs` writes a field called `answered`, so a new `control_*.rs` that put one in
/// an approval answer fails here. The two that carry an unrelated key of the same name are named,
/// with what it is: any other file is read by a person before it is added to the list.
#[test]
fn no_other_file_of_the_crate_writes_the_field() {
    const UNRELATED: &[(&str, &str)] = &[
        ("agents/mod.rs", "the agents' progress counts, how many questions were answered"),
        ("control_chat.rs", "whether a chat question has an answer yet"),
    ];
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src.clone()];
    let mut looked_at = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
                continue;
            }
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "tests.rs" || name == "approval_answer.rs" {
                continue;
            }
            looked_at += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap();
            let relative = path.strip_prefix(&src).unwrap().to_string_lossy().replace('\', "/");
            if production.contains("\"answered\"") {
                assert!(UNRELATED.iter().any(|(file, _)| *file == relative), "{relative} writes a field called `answered`: only approval_answer.rs may, for an approval");
            }
        }
    }
    assert!(looked_at > 100, "the scan found {looked_at} files: it is looking at the wrong place");
    for (file, _) in UNRELATED {
        assert!(std::fs::read_to_string(src.join(file)).unwrap().contains("\"answered\""), "{file} no longer has the key: take it off the list");
    }
}
