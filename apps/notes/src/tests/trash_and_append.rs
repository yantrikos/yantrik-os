//! #968: `append` starts a new line, and a title never reaches a trashed note. Each goes through
//! the real dispatch, as the Mind calls it.

use serde_json::json;
use yantrik_testing::TestApp;

use super::{act, note, started, state};
use crate::Notes;

fn text_after_append(before: &str, addition: &str) -> String {
    let app = started();
    note(&app, "List", "");
    act(&app, "set_content", json!({ "text": before })).unwrap();
    act(&app, "append", json!({ "text": addition })).unwrap();
    app.model(|m| m.current().unwrap().text().to_string())
}

#[test]
fn append_to_a_note_with_no_final_newline_starts_a_new_line() {
    assert_eq!(text_after_append("# List\n\n- umbrella", "- socks"), "# List\n\n- umbrella\n- socks");
}

#[test]
fn append_to_an_empty_note_adds_no_leading_newline() {
    assert_eq!(text_after_append("", "- socks"), "- socks");
}

#[test]
fn append_to_a_note_ending_in_a_newline_adds_no_blank_line() {
    assert_eq!(text_after_append("# List\n\n- umbrella\n", "- socks"), "# List\n\n- umbrella\n- socks");
}

/// A model often sends the line break itself (a real call: "\n- Hotel confirmation AX-4471").
#[test]
fn an_addition_that_starts_with_a_newline_adds_no_blank_line() {
    assert_eq!(text_after_append("# List\n\n- umbrella", "\n- socks"), "# List\n\n- umbrella\n- socks");
}

#[test]
fn a_note_with_crlf_line_breaks_keeps_them() {
    assert_eq!(text_after_append("# List\r\n\r\n- umbrella", "- socks"), "# List\r\n\r\n- umbrella\r\n- socks");
}

/// A trashed "Socks" ("old") and, unless `live` is None, a live "Socks" ("new"); nothing open.
fn socks(live: Option<&str>) -> TestApp<Notes> {
    let app = started();
    note(&app, "Socks", "old");
    act(&app, "trash", json!({})).unwrap();
    if let Some(body) = live {
        note(&app, "Socks", body);
        act(&app, "save", json!({})).unwrap();
        note(&app, "Elsewhere", "");
    }
    app
}

fn open_text(app: &TestApp<Notes>) -> String {
    app.model(|m| m.current().unwrap().text().to_string())
}

#[test]
fn open_note_prefers_the_live_note_over_a_trashed_one_of_the_same_title() {
    let app = socks(Some("new"));
    act(&app, "open_note", json!({ "title": "Socks" })).unwrap();
    assert!(open_text(&app).contains("new"), "{}", open_text(&app));
    assert_eq!(state(&app)["trashed_note_open"], false);
}

#[test]
fn open_note_by_title_never_reaches_a_note_only_the_trash_holds() {
    let app = socks(None);
    let refused = act(&app, "open_note", json!({ "title": "Socks" })).unwrap_err();
    assert!(refused.starts_with("No note is called \"Socks\"") && refused.contains("trash:"), "{refused}");
    assert_eq!(state(&app)["trashed_note_open"], false);
    assert!(act(&app, "open_in_new_window", json!({ "title": "Socks" })).is_err());
}

#[test]
fn the_trash_is_opt_in_by_its_own_id() {
    let app = socks(None);
    let id = app.model(|m| m.library.iter().find(|n| n.trash).unwrap().id.clone());
    act(&app, "open_note", json!({ "title": format!("trash:{id}") })).unwrap();
    assert_eq!(state(&app)["trashed_note_open"], true);
    assert!(open_text(&app).contains("old"));
}

/// An edit of a trashed note leaves restoring to the person: a model told "restore it first"
/// restores and edits in one turn.
#[test]
fn editing_a_trashed_note_says_to_ask_the_person_before_restoring_it() {
    let app = socks(None);
    let id = app.model(|m| m.library.iter().find(|n| n.trash).unwrap().id.clone());
    act(&app, "open_note", json!({ "title": format!("trash:{id}") })).unwrap();
    let refused = act(&app, "append", json!({ "text": "- more" })).unwrap_err();
    assert_eq!(refused, "The open note is in the Trash and cannot be edited. Ask the person before restoring it.");
    assert!(open_text(&app).contains("old") && !open_text(&app).contains("more"));
}
